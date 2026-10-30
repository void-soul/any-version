use super::*;

/// 推进结果：本地条目已经播起来了；在线条目要调用方**在 async 上下文里**先取流落盘。
///
/// 为什么把在线曲目抛回调用方：取流要跑插件（Node 桥）+ 下载整个文件，是秒级 async 操作，
/// 而播放器这把锁不能横跨 await 持有（会把所有状态查询卡死）。
#[derive(Debug)]
pub enum AdvanceOutcome {
    Playing(PlayerState),
    /// 轮到的是在线曲目：先取流落盘，再用拿到的路径调 [`MusicPlayerState::play_online_resolved`]
    Online(PendingOnline),
}

/// 待落盘的在线曲目。
#[derive(Clone, Debug)]
pub struct PendingOnline {
    /// 来源插件的脚本文件名
    pub file: String,
    /// 插件返回的曲目对象（`getMediaSource` 需要它，原样带回）
    pub item: serde_json::Value,
    pub quality: String,
    /// 本次请求的序号：回来时对不上说明用户已经切走，结果作废
    pub seq: u64,
}

/// 让「无法解码该音频文件」这句话变得**能定位问题**。
///
/// 真机案例：`D:\files\Music\Yungblud - Abyss (from Kaiju No. 8).flac` 其实是
/// **MP4 容器 + E-AC-3（杜比数字+）音轨**（`stsd` 里的 fourcc 是 `ec-3`），只是扩展名
/// 被写成了 `.flac`。symphonia 没有 AC-3 系解码器，于是只抛一句
/// `An IO error occurred while reading, writing, or seeking the stream.` ——
/// 用户既看不出「文件不对劲」也看不出「播放器不支持」，只能来问。
///
/// 所以这里补四件事：真实容器、是否命中所知不支持的编码、扩展名有没有在骗人、
/// 以及完整错误链（含底层 `io::Error`）。
/// 播放状态
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PlayStatus {
    Idle,
    Playing,
    Paused,
    /// 当前曲目已播完（等待前端决定下一首）
    Ended,
}

/// 暴露给前端的播放状态快照
#[derive(Serialize, Clone, Debug)]
pub struct PlayerState {
    pub status: PlayStatus,
    pub path: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub volume: f32,
    /// 当前曲目来自在线音源时的来源摘要（界面据此显示「下载」）
    pub online: Option<OnlineMeta>,
}

/// 当前曲目信息
pub(super) struct CurrentTrack {
    pub(super) path: String,
    pub(super) title: String,
    pub(super) artist: String,
    pub(super) duration_ms: u64,
    /// 来自在线音源时的来源（本地曲目为 None）
    pub(super) online: Option<OnlineOrigin>,
}

/// 非曲库文件（在线缓存文件）的显示名。
///
/// 为什么需要它：在线播放会把音频落到缓存目录，文件名是内容哈希（`a3f9…m4a`），
/// 曲库里查不到，`play_locked` 就退化成「显示文件名」—— 底部播放条于是显示一串哈希。
/// 这类文件的真实名字只能由调用方（`music_plugin_play`）显式登记。
#[derive(Clone)]
pub(super) struct TrackName {
    pub(super) title: String,
    pub(super) artist: String,
}

/// 当前曲目来自**在线音源**时的来源信息。
///
/// 只记住缓存文件路径是不够的：下载要重新取流，而取流需要「插件 + 原始曲目对象」
/// （`getMediaSource` 的入参）—— 搜索结果早就被清掉了，界面上只剩一个路径。
/// 所以来源必须跟着当前曲目一起保存。
#[derive(Clone, Debug)]
pub struct OnlineOrigin {
    /// 来源插件的脚本文件名
    pub file: String,
    /// 插件返回的曲目对象（原样保存；取流 / 下载都要原样回传）
    pub item: serde_json::Value,
    /// 当前缓存所用的音质（下载按钮默认选它）
    pub quality: String,
    /// 来源插件的展示名（播放时查一次，避免状态轮询每次读盘）
    pub platform: String,
}

/// 暴露给前端的在线来源摘要（不含曲目对象，只够显示与选音质）。
#[derive(Serialize, Clone, Debug)]
pub struct OnlineMeta {
    /// 来源插件的展示名
    pub platform: String,
    /// 当前缓存所用的音质
    pub quality: String,
}

/// 曲目的显示名（标题 / 歌手）。
///
/// 优先级：**登记名 → 曲库标签 → 文件名**。
/// 登记名优先是因为缓存文件既不在曲库里，文件名也没有可读性。
pub(super) fn track_labels(
    path: &str,
    named: Option<&TrackName>,
    library_track: Option<&library::MusicTrack>,
) -> (String, String) {
    let fallback_title = std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string());
    let title = named
        .map(|n| n.title.clone())
        // 插件偶尔不给标题：登记了空串也等于没名字，继续往下回退
        .filter(|s| !s.trim().is_empty())
        .or_else(|| library_track.map(|t| t.title.clone()))
        .unwrap_or(fallback_title);
    let artist = named
        .map(|n| n.artist.clone())
        .or_else(|| library_track.map(|t| t.artist.clone()))
        .unwrap_or_default();
    (title, artist)
}

pub(super) struct Inner {
    /// 音频输出设备（drop 即停止播放，必须与 player 同生命周期）
    pub(super) device: Option<MixerDeviceSink>,
    pub(super) player: Option<Player>,
    pub(super) current: Option<CurrentTrack>,
    pub(super) status: PlayStatus,
    pub(super) volume: f32,
    /// 播放队列（顺序 / 随机 / 单曲），自动切歌由后端驱动
    pub(super) queue: PlayQueue,
    /// 不在曲库里的文件（在线缓存）的显示名：路径 → 标题/歌手
    pub(super) names: HashMap<String, TrackName>,
    /// 正在后台取流的在线曲目（非空 = 有曲目在下载，巡查线程不再推进）
    pub(super) pending: Option<PendingOnline>,
    /// 取流请求序号：每次发起自增，回来时对不上说明用户已经切走
    pub(super) seq: u64,
    /// 已经预转码过的「下一首」（避免巡查每 250ms 重复检查同一个文件）
    pub(super) prefetched: Option<String>,
    /// 看门狗：上次观测到的播放位置（毫秒）
    pub(super) last_pos_ms: u64,
    /// 看门狗：上次观测到位置发生变化的时间
    pub(super) last_progress_at: Instant,
}

/// 播放位置多久不前进就判定「输出已死」（毫秒）。
///
/// 取 2s：正常音频回调延迟远小于它，不会误判；又短到用户几乎察觉不到卡顿。
/// 设这个兜底是因为**有些驱动在设备消失时只会静默停摆、不报任何流错误**。
pub(super) const STALL_RECOVER_MS: u64 = 2000;

/// 是否需要重建音频输出。
///
/// 两个触发条件（互为补充）：
/// 1. 音频流报错 —— 拔耳机 / 切换默认输出设备时 cpal 会把该流置为不可用；
/// 2. 仍在 `Playing` 但播放位置长时间不前进 —— 驱动不报错时的兜底。
///
/// 只有 `Playing` 才看停摆：暂停本来就不前进，`Ended` 由播完推进逻辑负责。
pub(super) fn should_recover(
    output_lost: bool,
    status: PlayStatus,
    stalled_for: Option<Duration>,
) -> bool {
    if output_lost {
        return true;
    }
    if status != PlayStatus::Playing {
        return false;
    }
    stalled_for
        .map(|d| d.as_millis() as u64 >= STALL_RECOVER_MS)
        .unwrap_or(false)
}

/// 输出重建后是否需要**续播**：只有「正在播 / 已暂停」才续播。
///
/// `Ended` 若续播会把已经放完的歌重新拉起来（位置在末尾，等于又震一下）；
/// `Idle` 只需丢掉坏流，等下次播放时自然重开。
pub(super) fn should_resume_after_recover(status: PlayStatus) -> bool {
    matches!(status, PlayStatus::Playing | PlayStatus::Paused)
}

/// 全局播放器（Tauri State）
pub struct MusicPlayerState {
    pub(super) inner: Mutex<Inner>,
    /// 供播放线程实时读取的均衡器参数（UI 改动即时生效）
    pub(super) eq: Arc<Mutex<EqParams>>,
    /// 音频输出流是否已失效（由 cpal 流错误回调置位；见 [`MusicPlayerState::recover_output`]）
    pub(super) output_lost: Arc<AtomicBool>,
}

impl Default for MusicPlayerState {
    fn default() -> Self {
        // 启动时恢复上次的音量与音效配置
        let settings = settings::load_settings();
        Self {
            inner: Mutex::new(Inner {
                device: None,
                player: None,
                current: None,
                status: PlayStatus::Idle,
                volume: settings.volume,
                queue: PlayQueue::default(),
                names: HashMap::new(),
                pending: None,
                seq: 0,
                prefetched: None,
                last_pos_ms: 0,
                last_progress_at: Instant::now(),
            }),
            eq: Arc::new(Mutex::new(settings.eq)),
            output_lost: Arc::new(AtomicBool::new(false)),
        }
    }
}
