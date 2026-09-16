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
    source: Vec<String>,
    /// 实际播放顺序：顺序/单曲 = 曲库顺序；随机 = 洗牌后的顺序
    order: Vec<String>,
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

    pub fn current(&self) -> Option<&str> {
        self.index
            .and_then(|i| self.order.get(i))
            .map(|s| s.as_str())
    }

    /// 重置队列：`paths` 为曲库顺序，`current` 为当前正在播放的曲目
    /// （随机模式下会被放到首位，保证正在播的那首不被打乱）
    pub fn set(&mut self, paths: Vec<String>, mode: PlayMode, current: Option<&str>) {
        self.source = paths;
        self.rebuild_order(mode, current);
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
        self.index = current.and_then(|path| self.order.iter().position(|p| p == path));
    }

    /// 把当前曲目对准到指定路径（用户双击某首时调用）；找到返回 true
    pub fn focus(&mut self, path: &str) -> bool {
        if let Some(pos) = self.order.iter().position(|p| p == path) {
            self.index = Some(pos);
            true
        } else {
            false
        }
    }

    /// 下一首；队列为空返回 None
    pub fn advance(&mut self) -> Option<String> {
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
    pub fn back(&mut self) -> Option<String> {
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
    pub fn first(&mut self) -> Option<String> {
        if self.order.is_empty() {
            return None;
        }
        self.index = Some(0);
        self.order.first().cloned()
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

    fn queue_of(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("track-{}.mp3", i)).collect()
    }

    #[test]
    fn test_sequence_wraps_around() {
        let mut q = PlayQueue::default();
        q.set(queue_of(3), PlayMode::Sequence, Some("track-0.mp3"));
        assert_eq!(q.advance().as_deref(), Some("track-1.mp3"));
        assert_eq!(q.advance().as_deref(), Some("track-2.mp3"));
        // 末尾回到开头
        assert_eq!(q.advance().as_deref(), Some("track-0.mp3"));
    }

    #[test]
    fn test_single_repeats_current() {
        let mut q = PlayQueue::default();
        q.set(queue_of(4), PlayMode::Single, Some("track-2.mp3"));
        assert_eq!(q.advance().as_deref(), Some("track-2.mp3"));
        assert_eq!(q.advance().as_deref(), Some("track-2.mp3"));
        assert_eq!(q.back().as_deref(), Some("track-2.mp3"));
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
            played.push(q.advance().expect("有曲目"));
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
            last = q.advance().expect("有曲目");
        }
        // 袋已用尽：下一首触发重洗，且不应紧接着重复刚播过的那首
        let next = q.advance().expect("有曲目");
        assert_ne!(next, last, "重洗后不应立刻重复上一首");
    }

    #[test]
    fn test_shuffle_set_puts_current_first() {
        let mut q = PlayQueue::with_seed(99);
        q.set(queue_of(5), PlayMode::Shuffle, Some("track-3.mp3"));
        // 当前曲目必须仍在队列中（且能被 focus 定位）
        assert!(q.focus("track-3.mp3"));
        assert_eq!(q.current().as_deref(), Some("track-3.mp3"));
    }

    #[test]
    fn test_back_wraps_to_last() {
        let mut q = PlayQueue::default();
        q.set(queue_of(3), PlayMode::Sequence, Some("track-0.mp3"));
        assert_eq!(q.back().as_deref(), Some("track-2.mp3"));
        assert_eq!(q.back().as_deref(), Some("track-1.mp3"));
    }

    #[test]
    fn test_focus_unknown_path_returns_false() {
        let mut q = PlayQueue::default();
        q.set(queue_of(2), PlayMode::Sequence, None);
        assert!(!q.focus("missing.mp3"));
        assert!(q.focus("track-1.mp3"));
        assert_eq!(q.current().as_deref(), Some("track-1.mp3"));
    }

    #[test]
    fn test_sequence_advance_without_current_starts_from_first() {
        let mut q = PlayQueue::default();
        q.set(queue_of(3), PlayMode::Sequence, None);
        assert_eq!(q.advance().as_deref(), Some("track-0.mp3"));
        assert_eq!(q.first().as_deref(), Some("track-0.mp3"));
    }

    #[test]
    fn test_mode_switch_keeps_current_track() {
        let mut q = PlayQueue::default();
        q.set(queue_of(5), PlayMode::Sequence, Some("track-2.mp3"));
        q.set_mode(PlayMode::Shuffle, Some("track-2.mp3"));
        assert_eq!(q.current().as_deref(), Some("track-2.mp3"));
        assert_eq!(q.mode(), PlayMode::Shuffle);
        // 切回顺序模式：顺序恢复为曲库顺序（而不是保留洗牌后的顺序），
        // 因此下一首应恰好是曲库里的后一首
        q.set_mode(PlayMode::Sequence, Some("track-2.mp3"));
        assert_eq!(q.current().as_deref(), Some("track-2.mp3"));
        assert_eq!(q.advance().as_deref(), Some("track-3.mp3"));
    }

    #[test]
    fn test_sequence_mode_restores_library_order_after_shuffle() {
        // 回归：曾出现过「随机切回顺序后仍按洗牌顺序播放」
        let mut q = PlayQueue::with_seed(1234);
        q.set(queue_of(6), PlayMode::Shuffle, None);
        q.advance();
        q.set_mode(PlayMode::Sequence, Some("track-0.mp3"));
        let mut played = vec![q.advance().expect("有曲目")];
        for _ in 0..4 {
            played.push(q.advance().expect("有曲目"));
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
}
