//! 播放队列：顺序 / 随机（洗牌）/ 单曲，自动推进与回退。
//!
//! 为什么放在后端：窗口最小化到托盘后 WebView2 会节流前端定时器，
//! 前端无法及时发现「播完」并切歌；因此队列与推进必须由 Rust 侧持有
//! （由 `player::start_queue_watcher` 的巡查线程驱动）。
//!
//! 随机模式：整条队列视为一个洗牌袋，播到末尾时重新洗牌（并把刚播过的
//! 那首挪出首位，避免「随机到同一首」的观感）。

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// 播放模式（持久化在 settings.json 的 `play_mode`）
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum PlayMode {
    Sequence,
    Shuffle,
    Single,
}

impl Default for PlayMode {
    fn default() -> Self {
        Self::Sequence
    }
}

impl PlayMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sequence => "sequence",
            Self::Shuffle => "shuffle",
            Self::Single => "single",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "shuffle" => Self::Shuffle,
            "single" => Self::Single,
            _ => Self::Sequence,
        }
    }
}

/// 队列条目。
///
/// 绝大多数是本地文件。在线曲目只有「插件 + 曲目对象」，**要轮到它时才取流落盘**
/// （取流要跑插件、下载要几秒，还可能失败），所以放进队列时它**还没有路径** ——
/// 早先队列只存路径，于是「用整份搜索结果替换播放列表」只能先把几十首全下载一遍。
/// 落盘后由 [`PlayQueue::materialize_current`] 换成 `Path`。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueItem {
    /// 本地文件（曲库 / 下载目录 / 在线缓存）
    Path(String),
    /// 在线曲目：轮到时才取流落缓存
    Online {
        /// 来源插件的脚本文件名
        file: String,
        /// 插件返回的曲目对象（原样保存，`getMediaSource` 需要它）
        item: serde_json::Value,
        quality: String,
    },
}

impl QueueItem {
    /// 本地路径（在线曲目在落盘前没有路径）
    pub fn path(&self) -> Option<&str> {
        match self {
            Self::Path(path) => Some(path.as_str()),
            Self::Online { .. } => None,
        }
    }

    /// 稳定标识：本地=路径；在线=插件 + 音质 + 曲目 id。
    ///
    /// 队列里的比对（定位当前曲、去重、剔除）都用它 —— 在线曲目没有路径可比。
    pub fn key(&self) -> String {
        match self {
            Self::Path(path) => path.clone(),
            Self::Online {
                file, item, quality, ..
            } => {
                let id = item
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| item.get("title").and_then(serde_json::Value::as_str))
                    .unwrap_or("");
                format!("online:{file}:{quality}:{id}")
            }
        }
    }
}

/// 极简 xorshift64 —— 只为洗牌，不额外引入 rand 依赖（可注入种子便于单测）
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn from_time() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        Self::new(nanos)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }
}

/// 播放队列（曲库顺序进、按模式出）
pub struct PlayQueue {
    /// 曲库顺序（原始顺序）：从随机模式切回顺序模式时用它恢复播放顺序
    source: Vec<QueueItem>,
    /// 实际播放顺序：顺序/单曲 = 曲库顺序；随机 = 洗牌后的顺序
    order: Vec<QueueItem>,
    /// 当前曲目在 order 中的下标
    index: Option<usize>,
    mode: PlayMode,
    rng: Rng,
}

impl Default for PlayQueue {
    fn default() -> Self {
        Self {
            source: Vec::new(),
            order: Vec::new(),
            index: None,
            mode: PlayMode::Sequence,
            rng: Rng::from_time(),
        }
    }
}

impl PlayQueue {
    /// 用固定种子构造（仅测试用）
    #[cfg(test)]
    fn with_seed(seed: u64) -> Self {
        Self {
            rng: Rng::new(seed),
            ..Default::default()
        }
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn mode(&self) -> PlayMode {
        self.mode
    }

    pub fn current(&self) -> Option<&QueueItem> {
        self.index.and_then(|i| self.order.get(i))
    }

    /// 当前曲目的本地路径（在线曲目落盘前为 None）
    pub fn current_path(&self) -> Option<&str> {
        self.current().and_then(QueueItem::path)
    }

    /// 重置队列：`items` 为曲库顺序，`current` 为当前正在播放的曲目
    /// （随机模式下会被放到首位，保证正在播的那首不被打乱）
    pub fn set(&mut self, items: Vec<QueueItem>, mode: PlayMode, current: Option<&str>) {
        self.source = items;
        self.rebuild_order(mode, current);
    }

    /// 当前条目（在线曲目）已落盘：换成它的本地路径。
    ///
    /// `source` 里同一条也要一起换 —— 只换 `order` 的话，切回顺序模式时它会变回
    /// 「未落盘」，于是同一首歌又会被重新下载一遍。
    pub fn materialize_current(&mut self, path: &str) {
        let Some(index) = self.index else {
            return;
        };
        let Some(old) = self.order.get(index).cloned() else {
            return;
        };
        self.order[index] = QueueItem::Path(path.to_string());
        if let Some(pos) = self.source.iter().position(|item| *item == old) {
            self.source[pos] = QueueItem::Path(path.to_string());
        }
    }

    /// 仅切换模式（保持当前曲目继续播放）：
    /// 切到随机 → 重新洗牌；切回顺序/单曲 → 恢复曲库顺序。
    pub fn set_mode(&mut self, mode: PlayMode, current: Option<&str>) {
        self.rebuild_order(mode, current);
    }

    /// 按模式重建播放顺序，并把下标对准到 `current`
    fn rebuild_order(&mut self, mode: PlayMode, current: Option<&str>) {
        self.mode = mode;
        self.order = self.source.clone();
        if mode == PlayMode::Shuffle {
            self.shuffle();
        }
        // 用 key 比对：在线条目的 key 不是路径，走进来的 `current` 也可能是 key
        self.index = current.and_then(|key| self.order.iter().position(|item| item.key() == key));
    }

    /// 把当前曲目对准到指定条目（用户双击某首时调用）；找到返回 true
    pub fn focus(&mut self, key: &str) -> bool {
        if let Some(pos) = self.order.iter().position(|item| item.key() == key) {
            self.index = Some(pos);
            true
        } else {
            false
        }
    }

    /// 下一首；队列为空返回 None
    pub fn advance(&mut self) -> Option<QueueItem> {
        let total = self.order.len();
        if total == 0 {
            return None;
        }
        let next = match (self.mode, self.index) {
            // 单曲循环：始终返回当前曲（没有当前曲则从第一首开始）
            (PlayMode::Single, Some(idx)) => idx,
            (PlayMode::Single, None) => 0,
            (PlayMode::Sequence, Some(idx)) => (idx + 1) % total,
            (PlayMode::Sequence, None) => 0,
            (PlayMode::Shuffle, Some(idx)) if idx + 1 < total => idx + 1,
            // 随机模式播到队尾：重新洗牌，并把刚播过的曲目挪出首位
            (PlayMode::Shuffle, Some(idx)) => {
                let last = self.order.get(idx).cloned();
                self.order = self.source.clone();
                self.shuffle();
                if let Some(last) = last {
                    if self.order.first() == Some(&last) && self.order.len() > 1 {
                        self.order.swap(0, 1);
                    }
                }
                0
            }
            (PlayMode::Shuffle, None) => 0,
        };
        self.index = Some(next);
        self.order.get(next).cloned()
    }

    /// 上一首（随机模式按实际播放顺序回退）
    pub fn back(&mut self) -> Option<QueueItem> {
        let total = self.order.len();
        if total == 0 {
            return None;
        }
        let prev = match (self.mode, self.index) {
            (PlayMode::Single, Some(idx)) => idx,
            (PlayMode::Single, None) => 0,
            (_, Some(0)) | (_, None) => total - 1,
            (_, Some(idx)) => idx - 1,
        };
        self.index = Some(prev);
        self.order.get(prev).cloned()
    }

    /// 首曲（前端「开始播放」时用）
    pub fn first(&mut self) -> Option<QueueItem> {
        if self.order.is_empty() {
            return None;
        }
        self.index = Some(0);
        self.order.first().cloned()
    }

    /// 曲目被重命名（磁盘文件改名）后同步队列里的路径。
    pub fn rename_path(&mut self, old: &str, new: &str) {
        for item in self.source.iter_mut().chain(self.order.iter_mut()) {
            if let QueueItem::Path(path) = item {
                if path == old {
                    *path = new.to_string();
                }
            }
        }
    }

    /// 曲目被删除后从队列移除，返回**当前曲目是否被删除**。
    ///
    /// 下标修正是这段的关键：删完之后 `advance()` 必须落到「被删那首的后一首」，
    /// 而不是跳过一首或倒回前一首 —— 三种模式对 `index` 的解释不同，因此分开处理：
    /// - 顺序 / 随机：`advance()` 取 `index + 1`，所以把 `index` 指向被删位置的前一个
    ///   （被删的是首曲时置 `None`，`advance()` 会取新的第一首）；
    /// - 单曲循环：`advance()` 取 `index` 本身，所以把 `index` 对准被删位置（原位置的新占用者）。
    pub fn remove_paths(&mut self, paths: &[String]) -> bool {
        if paths.is_empty() {
            return false;
        }
        // 只按路径剔除：在线条目没有路径，不会被误删
        let hit = |item: &QueueItem| match item {
            QueueItem::Path(p) => paths.iter().any(|x| x.as_str() == p),
            QueueItem::Online { .. } => false,
        };
        let current_removed = self.current().map(hit).unwrap_or(false);
        if self.order.is_empty() {
            return current_removed;
        }
        // 被删项里位于当前下标之前的数量（用于把下标平移回同一首歌）
        let removed_before_index = match self.index {
            Some(i) => self.order.iter().take(i).filter(|p| hit(p)).count(),
            None => 0,
        };

        self.source.retain(|p| !hit(p));
        self.order.retain(|p| !hit(p));

        self.index = match self.index {
            None => None,
            Some(_) if self.order.is_empty() => None,
            Some(i) => {
                let new_pos = i.saturating_sub(removed_before_index);
                if current_removed {
                    match self.mode {
                        PlayMode::Single => Some(new_pos.min(self.order.len() - 1)),
                        _ if new_pos == 0 => None,
                        _ => Some(new_pos - 1),
                    }
                } else {
                    Some(new_pos.min(self.order.len() - 1))
                }
            }
        };
        current_removed
    }

    /// Fisher-Yates 洗牌（in place）
    fn shuffle(&mut self) {
        let len = self.order.len();
        for i in (1..len).rev() {
            let j = self.rng.below(i + 1);
            self.order.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue_of(n: usize) -> Vec<QueueItem> {
        (0..n)
            .map(|i| QueueItem::Path(format!("track-{}.mp3", i)))
            .collect()
    }

    /// 断言里要的是字符串：本地条目取路径，在线条目取 key。
    fn path(item: Option<QueueItem>) -> Option<String> {
        item.map(|item| item.path().map(str::to_string).unwrap_or_else(|| item.key()))
    }

    #[test]
    fn test_sequence_wraps_around() {
        let mut q = PlayQueue::default();
        q.set(queue_of(3), PlayMode::Sequence, Some("track-0.mp3"));
        assert_eq!(path(q.advance()).as_deref(), Some("track-1.mp3"));
        assert_eq!(path(q.advance()).as_deref(), Some("track-2.mp3"));
        // 末尾回到开头
        assert_eq!(path(q.advance()).as_deref(), Some("track-0.mp3"));
    }

    #[test]
    fn test_single_repeats_current() {
        let mut q = PlayQueue::default();
        q.set(queue_of(4), PlayMode::Single, Some("track-2.mp3"));
        assert_eq!(path(q.advance()).as_deref(), Some("track-2.mp3"));
        assert_eq!(path(q.advance()).as_deref(), Some("track-2.mp3"));
        assert_eq!(path(q.back()).as_deref(), Some("track-2.mp3"));
    }

    #[test]
    fn test_empty_queue_returns_none() {
        let mut q = PlayQueue::default();
        q.set(Vec::new(), PlayMode::Shuffle, None);
        assert_eq!(q.advance(), None);
        assert_eq!(q.back(), None);
        assert_eq!(q.first(), None);
        assert_eq!(q.len(), 0);
    }

    #[test]
    fn test_shuffle_covers_every_track_once_per_bag() {
        let mut q = PlayQueue::with_seed(42);
        q.set(queue_of(6), PlayMode::Shuffle, None);
        let mut played: Vec<String> = Vec::new();
        for _ in 0..6 {
            played.push(path(q.advance()).expect("有曲目"));
        }
        played.sort();
        played.dedup();
        assert_eq!(played.len(), 6, "一袋内应覆盖全部曲目且不重复");
    }

    #[test]
    fn test_shuffle_reshuffles_at_wrap_without_immediate_repeat() {
        let mut q = PlayQueue::with_seed(7);
        q.set(queue_of(4), PlayMode::Shuffle, None);
        let mut last = String::new();
        for _ in 0..4 {
            last = path(q.advance()).expect("有曲目");
        }
        // 袋已用尽：下一首触发重洗，且不应紧接着重复刚播过的那首
        let next = path(q.advance()).expect("有曲目");
        assert_ne!(next, last, "重洗后不应立刻重复上一首");
    }

    #[test]
    fn test_shuffle_set_puts_current_first() {
        let mut q = PlayQueue::with_seed(99);
        q.set(queue_of(5), PlayMode::Shuffle, Some("track-3.mp3"));
        // 当前曲目必须仍在队列中（且能被 focus 定位）
        assert!(q.focus("track-3.mp3"));
        assert_eq!(q.current_path(), Some("track-3.mp3"));
    }

    #[test]
    fn test_back_wraps_to_last() {
        let mut q = PlayQueue::default();
        q.set(queue_of(3), PlayMode::Sequence, Some("track-0.mp3"));
        assert_eq!(path(q.back()).as_deref(), Some("track-2.mp3"));
        assert_eq!(path(q.back()).as_deref(), Some("track-1.mp3"));
    }

    #[test]
    fn test_focus_unknown_path_returns_false() {
        let mut q = PlayQueue::default();
        q.set(queue_of(2), PlayMode::Sequence, None);
        assert!(!q.focus("missing.mp3"));
        assert!(q.focus("track-1.mp3"));
        assert_eq!(q.current_path(), Some("track-1.mp3"));
    }

    #[test]
    fn test_sequence_advance_without_current_starts_from_first() {
        let mut q = PlayQueue::default();
        q.set(queue_of(3), PlayMode::Sequence, None);
        assert_eq!(path(q.advance()).as_deref(), Some("track-0.mp3"));
        assert_eq!(path(q.first()).as_deref(), Some("track-0.mp3"));
    }

    #[test]
    fn test_mode_switch_keeps_current_track() {
        let mut q = PlayQueue::default();
        q.set(queue_of(5), PlayMode::Sequence, Some("track-2.mp3"));
        q.set_mode(PlayMode::Shuffle, Some("track-2.mp3"));
        assert_eq!(q.current_path(), Some("track-2.mp3"));
        assert_eq!(q.mode(), PlayMode::Shuffle);
        // 切回顺序模式：顺序恢复为曲库顺序（而不是保留洗牌后的顺序），
        // 因此下一首应恰好是曲库里的后一首
        q.set_mode(PlayMode::Sequence, Some("track-2.mp3"));
        assert_eq!(q.current_path(), Some("track-2.mp3"));
        assert_eq!(path(q.advance()).as_deref(), Some("track-3.mp3"));
    }

    #[test]
    fn test_sequence_mode_restores_library_order_after_shuffle() {
        // 回归：曾出现过「随机切回顺序后仍按洗牌顺序播放」
        let mut q = PlayQueue::with_seed(1234);
        q.set(queue_of(6), PlayMode::Shuffle, None);
        q.advance();
        q.set_mode(PlayMode::Sequence, Some("track-0.mp3"));
        let mut played = vec![path(q.advance()).expect("有曲目")];
        for _ in 0..4 {
            played.push(path(q.advance()).expect("有曲目"));
        }
        assert_eq!(
            played,
            vec![
                "track-1.mp3",
                "track-2.mp3",
                "track-3.mp3",
                "track-4.mp3",
                "track-5.mp3"
            ],
            "顺序模式必须按曲库顺序推进"
        );
    }

    #[test]
    fn test_play_mode_str_round_trip() {
        for mode in [PlayMode::Sequence, PlayMode::Shuffle, PlayMode::Single] {
            assert_eq!(PlayMode::from_str(mode.as_str()), mode);
        }
        assert_eq!(PlayMode::from_str("unknown"), PlayMode::Sequence);
    }

    /// 重命名后队列里的路径要跟着换（否则下一首会指向已不存在的旧路径）。
    #[test]
    fn test_rename_path_follows_file_rename() {
        let mut q = PlayQueue::default();
        q.set(queue_of(3), PlayMode::Sequence, Some("track-1.mp3"));
        q.rename_path("track-1.mp3", "renamed.mp3");
        assert_eq!(q.current_path(), Some("renamed.mp3"));
        assert_eq!(path(q.advance()).as_deref(), Some("track-2.mp3"));
    }

    /// 删除当前曲目后，`advance()` 必须落到「被删那首的后一首」而不是跳曲。
    #[test]
    fn test_remove_current_advances_to_the_following_track() {
        let mut q = PlayQueue::default();
        q.set(queue_of(4), PlayMode::Sequence, Some("track-1.mp3"));
        assert!(q.remove_paths(&["track-1.mp3".to_string()]));
        assert_eq!(q.len(), 3);
        assert_eq!(path(q.advance()).as_deref(), Some("track-2.mp3"), "顺序模式不能跳曲");
    }

    /// 删除的是首曲时，下一次从「新的第一首」开始（不能回到被删位置）。
    #[test]
    fn test_remove_first_track_starts_from_new_head() {
        let mut q = PlayQueue::default();
        q.set(queue_of(3), PlayMode::Sequence, Some("track-0.mp3"));
        assert!(q.remove_paths(&["track-0.mp3".to_string()]));
        assert_eq!(path(q.advance()).as_deref(), Some("track-1.mp3"));
    }

    /// 单曲循环：`advance()` 取 `index` 本身，删除当前曲目后应落到原位置的新占用者。
    #[test]
    fn test_remove_current_in_single_mode_keeps_the_slot() {
        let mut q = PlayQueue::default();
        q.set(queue_of(3), PlayMode::Single, Some("track-1.mp3"));
        assert!(q.remove_paths(&["track-1.mp3".to_string()]));
        assert_eq!(path(q.advance()).as_deref(), Some("track-2.mp3"));
    }

    /// 删除非当前曲目：当前曲目不变，下标平移不能导致错位。
    #[test]
    fn test_remove_other_tracks_keeps_current() {
        // 删掉当前曲目**之前**的一首
        let mut q = PlayQueue::default();
        q.set(queue_of(4), PlayMode::Sequence, Some("track-2.mp3"));
        assert!(!q.remove_paths(&["track-0.mp3".to_string()]));
        assert_eq!(q.current_path(), Some("track-2.mp3"));
        assert_eq!(path(q.advance()).as_deref(), Some("track-3.mp3"));

        // 删掉当前曲目**之后**的一首
        let mut q2 = PlayQueue::default();
        q2.set(queue_of(4), PlayMode::Sequence, Some("track-1.mp3"));
        assert!(!q2.remove_paths(&["track-3.mp3".to_string()]));
        assert_eq!(path(q2.advance()).as_deref(), Some("track-2.mp3"));
    }

    /// 批量删除（含当前曲目及其前后项）后队列不越界，且能继续推进。
    #[test]
    fn test_bulk_remove_keeps_queue_consistent() {
        let mut q = PlayQueue::default();
        q.set(queue_of(5), PlayMode::Sequence, Some("track-2.mp3"));
        let removed = vec![
            "track-0.mp3".to_string(),
            "track-2.mp3".to_string(),
            "track-4.mp3".to_string(),
        ];
        assert!(q.remove_paths(&removed));
        assert_eq!(q.len(), 2);
        assert_eq!(path(q.advance()).as_deref(), Some("track-3.mp3"));

        // 全部删空后不再返回曲目
        assert!(q.remove_paths(&["track-1.mp3".to_string(), "track-3.mp3".to_string()]));
        assert!(q.len() == 0);
        assert_eq!(q.advance(), None);
        assert_eq!(q.current(), None);
    }

    fn online(id: &str) -> QueueItem {
        QueueItem::Online {
            file: "demo.js".to_string(),
            item: serde_json::json!({ "id": id, "title": id }),
            quality: "standard".to_string(),
        }
    }

    /// 在线曲目落盘前**没有路径**；落盘后 `order` 与 `source` 都要换成路径 ——
    /// 只换 order 的话，切回顺序模式它会变回「未落盘」，同一首歌又被下载一遍。
    #[test]
    fn online_item_becomes_a_path_once_materialized() {
        let mut q = PlayQueue::default();
        q.set(
            vec![
                QueueItem::Path("a.mp3".to_string()),
                online("s1"),
                QueueItem::Path("b.mp3".to_string()),
            ],
            PlayMode::Sequence,
            Some("a.mp3"),
        );

        let next = q.advance().expect("有曲目");
        assert!(next.path().is_none(), "在线曲目不该有路径");
        assert!(next.key().starts_with("online:"), "key 要能标出它是在线的");

        q.materialize_current("cache/s1.m4a");
        assert_eq!(q.current_path(), Some("cache/s1.m4a"));

        // 切回顺序模式后仍是路径（说明 source 里那条也换了）
        q.set_mode(PlayMode::Sequence, Some("cache/s1.m4a"));
        assert_eq!(q.current_path(), Some("cache/s1.m4a"));

        let mut rest: Vec<String> = Vec::new();
        for _ in 0..3 {
            if let Some(item) = q.advance() {
                rest.push(item.path().map(str::to_string).unwrap_or_else(|| item.key()));
            }
        }
        assert!(
            !rest.iter().any(|item| item.starts_with("online:")),
            "落盘后队列里不该再有未落盘条目: {rest:?}"
        );
    }

    /// 删除本地文件不能顺手把在线曲目也删掉（它们没有路径可比）。
    #[test]
    fn removing_paths_leaves_online_items_alone() {
        let mut q = PlayQueue::default();
        q.set(
            vec![
                QueueItem::Path("a.mp3".to_string()),
                online("s1"),
                QueueItem::Path("b.mp3".to_string()),
            ],
            PlayMode::Sequence,
            None,
        );
        assert!(!q.remove_paths(&["a.mp3".to_string(), "b.mp3".to_string()]));
        assert_eq!(q.len(), 1);
        assert!(
            q.current().map(|item| item.path().is_none()).unwrap_or(false),
            "剩下的应是在线曲目"
        );
    }
}
