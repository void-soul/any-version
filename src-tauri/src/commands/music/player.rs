//! 播放引擎：rodio 输出流 + 播放队列 + 均衡器接入。
//!
//! 一个进程内只有一个播放器（Tauri `State`），曲目切换 = 重建 `Player` 并 append 新的
//! `EqSource`。位置/时长由 rodio 的 `get_pos()`/曲库时长提供。
//!
//! **队列与自动切歌在后端**（[`start_queue_watcher`] 的巡查线程驱动）：
//! 窗口最小化到托盘后 WebView2 会节流前端定时器，前端无法及时感知「播完」，
//! 若把切歌交给前端就会出现「托盘模式下不自动切下一首」。

use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Source};
use serde::Serialize;
use tauri::Manager;

use super::dsp::{EqParams, EqSource};
use super::library;
use super::queue::{PlayMode, PlayQueue, QueueItem};
use super::settings::{self, MusicSettings};

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
fn decode_failure_message(path: &str, err: &rodio::decoder::DecoderError) -> String {
    let path_ref = std::path::Path::new(path);
    let (container, codec) = sniff_audio_header(path_ref);

    let mut raw = format!("{err}");
    let mut source = std::error::Error::source(err);
    while let Some(e) = source {
        raw.push_str(&format!("（{e}）"));
        source = e.source();
    }

    compose_decode_message(
        &container,
        codec,
        path_ref.extension().and_then(|e| e.to_str()),
        Some(&raw),
    )
}

/// 组装「无法解码」的说明（从错误里拆出来以便单测）。
///
/// **已给出具体诊断时故意不附底层错误**：真机案例里那句
/// `An IO error occurred while reading, writing, or seeking the stream.`
/// 既说不出「文件不对劲」也说不出「播放器不支持」，挂在已经点明原因的结论后面
/// 只会把结论冲淡（用户要的是「为什么播不了」，不是 symphonia 的内部错误）。
/// 反过来，**没有诊断出原因时**底层错误是唯一线索，必须保留。
fn compose_decode_message(
    container: &str,
    codec: Option<&str>,
    ext: Option<&str>,
    raw: Option<&str>,
) -> String {
    let mut msg = format!("无法解码该音频文件（实际容器：{container}");
    let mut diagnosed = false;
    if let Some(codec) = codec {
        msg.push_str(&format!("，音轨编码：{codec} —— 本播放器不支持这种编码"));
        diagnosed = true;
    }
    msg.push(')');

    if let Some(ext) = ext {
        let declared = declared_container(ext);
        if !declared.is_empty() && declared != container {
            msg.push_str(&format!(
                "；扩展名是 .{ext}，与真实容器不符（多半是下载/转换时改错了名）"
            ));
            diagnosed = true;
        }
    }

    if !diagnosed {
        if let Some(raw) = raw {
            msg.push_str(&format!("：{raw}"));
        }
    }
    msg
}

/// 从**文件头**判断真实容器，并在有限窗口里找已知**不被支持**的音频编码 fourcc。
///
/// 只看开头 64KB（`moov`/`stsd` 通常紧随 `ftyp`）。找不到就当「不认识」——
/// 绝不因为没找到就断言文件没问题。**按内容判断，不看扩展名**（扩展名是本案的误导源）。
fn sniff_audio_header(path: &std::path::Path) -> (String, Option<&'static str>) {
    use std::io::Read;
    const WINDOW: usize = 64 * 1024;

    let Ok(mut file) = File::open(path) else {
        return ("无法读取".to_string(), None);
    };
    let mut head = vec![0u8; WINDOW];
    let Ok(read) = file.read(&mut head) else {
        return ("无法读取".to_string(), None);
    };
    head.truncate(read);
    if head.len() < 12 {
        return ("文件过短（可能没下载完）".to_string(), None);
    }

    let container = if &head[0..4] == b"fLaC" {
        "FLAC"
    } else if &head[4..8] == b"ftyp" {
        "MP4/M4A"
    } else if &head[0..3] == b"ID3" || (head[0] == 0xFF && (head[1] & 0xE0) == 0xE0) {
        "MP3"
    } else if &head[0..4] == b"RIFF" {
        "WAV"
    } else if &head[0..4] == b"OggS" {
        "Ogg"
    } else {
        "未知"
    };

    // symphonia 不提供这些解码器；命中就直接点名，别让用户对着 IoError 猜
    let codec = [
        ("ec-3", "E-AC-3（杜比数字+）"),
        ("ac-3", "AC-3（杜比数字）"),
        ("dtsc", "DTS"),
    ]
    .into_iter()
    .find(|(fourcc, _)| head.windows(4).any(|w| w == fourcc.as_bytes()))
    .map(|(_, name)| name);

    (container.to_string(), codec)
}

/// 扩展名**声称**的容器名（用于判断文件是不是被改了名）。返回空串 = 不判断。
fn declared_container(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "flac" => "FLAC",
        "mp3" => "MP3",
        "wav" => "WAV",
        "ogg" | "oga" => "Ogg",
        "m4a" | "mp4" => "MP4/M4A",
        // aac 既可能是裸流也可能装在 MP4 里，光看扩展名无法判定，不参与比对
        _ => "",
    }
}

#[cfg(test)]
mod decode_probe_tests {
    use super::*;
    use std::io::Write;

    fn probe_file(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("anyver-decode-{name}-{}", std::process::id()));
        let mut f = File::create(&p).unwrap();
        f.write_all(bytes).unwrap();
        p
    }

    /// 容器按**内容**判断，不看扩展名 —— 扩展名正是本案的误导源。
    #[test]
    fn sniffs_container_by_content() {
        let mut mp4 = vec![0u8, 0, 0, 0x18];
        mp4.extend_from_slice(b"ftypmp42");
        mp4.extend_from_slice(&[0u8; 64]);
        let p = probe_file("mp4", &mp4);
        assert_eq!(sniff_audio_header(&p).0, "MP4/M4A");
        let _ = std::fs::remove_file(&p);

        let mut flac = b"fLaC".to_vec();
        flac.extend_from_slice(&[0u8, 0, 0, 0x22, 0, 0, 0, 0]);
        let p = probe_file("flac", &flac);
        assert_eq!(sniff_audio_header(&p).0, "FLAC");
        let _ = std::fs::remove_file(&p);

        let p = probe_file("riff", b"RIFF\x00\x00\x00\x00WAVEfmt ");
        assert_eq!(sniff_audio_header(&p).0, "WAV");
        let _ = std::fs::remove_file(&p);
    }

    /// 命中所知不支持的编码要**点名**，而不是让用户对着 IoError 发呆。
    #[test]
    fn flags_known_unsupported_codecs() {
        let mut mp4 = vec![0u8, 0, 0, 0x18];
        mp4.extend_from_slice(b"ftypmp42");
        mp4.extend_from_slice(b"....stsd....ec-3....dec3");
        let p = probe_file("ec3", &mp4);
        let (container, codec) = sniff_audio_header(&p);
        assert_eq!(container, "MP4/M4A");
        assert_eq!(codec, Some("E-AC-3（杜比数字+）"));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn declared_container_skips_ambiguous_extension() {
        assert_eq!(declared_container("flac"), "FLAC");
        assert_eq!(declared_container("M4A"), "MP4/M4A");
        // aac 可能是裸流也可能是 MP4，光看扩展名判不了 → 不参与比对
        assert_eq!(declared_container("aac"), "");
        assert_eq!(declared_container("xyz"), "");
    }

    /// 原因已点名（不支持的编码 / 扩展名在骗人）→ 不再附底层 IoError，那句只会冲淡结论。
    #[test]
    fn drops_raw_error_once_the_cause_is_named() {
        let msg = compose_decode_message(
            "MP4/M4A",
            Some("E-AC-3（杜比数字+）"),
            Some("flac"),
            Some("An IO error occurred while reading"),
        );
        assert!(msg.contains("E-AC-3"), "得点名编码: {msg}");
        assert!(msg.contains("与真实容器不符"), "得点破扩展名在骗人: {msg}");
        assert!(!msg.contains("IO error"), "已诊断时不该再挂原始错误: {msg}");
    }

    /// 没诊断出原因时，底层错误是唯一线索，必须保留。
    #[test]
    fn keeps_raw_error_when_there_is_no_diagnosis() {
        let msg = compose_decode_message("MP4/M4A", None, Some("m4a"), Some("Unsupported codec"));
        assert!(msg.contains("Unsupported codec"), "原因不明时不能丢线索: {msg}");
    }

    #[test]
    fn truncated_file_is_called_out() {
        let p = probe_file("short", b"\x00\x01");
        assert!(sniff_audio_header(&p).0.contains("过短"));
        let _ = std::fs::remove_file(&p);
    }
}

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
}

/// 当前曲目信息
struct CurrentTrack {
    path: String,
    title: String,
    artist: String,
    duration_ms: u64,
}

/// 非曲库文件（在线缓存文件）的显示名。
///
/// 为什么需要它：在线播放会把音频落到缓存目录，文件名是内容哈希（`a3f9…m4a`），
/// 曲库里查不到，`play_locked` 就退化成「显示文件名」—— 底部播放条于是显示一串哈希。
/// 这类文件的真实名字只能由调用方（`music_plugin_play`）显式登记。
#[derive(Clone)]
struct TrackName {
    title: String,
    artist: String,
}

/// 曲目的显示名（标题 / 歌手）。
///
/// 优先级：**登记名 → 曲库标签 → 文件名**。
/// 登记名优先是因为缓存文件既不在曲库里，文件名也没有可读性。
fn track_labels(
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

struct Inner {
    /// 音频输出设备（drop 即停止播放，必须与 player 同生命周期）
    device: Option<MixerDeviceSink>,
    player: Option<Player>,
    current: Option<CurrentTrack>,
    status: PlayStatus,
    volume: f32,
    /// 播放队列（顺序 / 随机 / 单曲），自动切歌由后端驱动
    queue: PlayQueue,
    /// 不在曲库里的文件（在线缓存）的显示名：路径 → 标题/歌手
    names: HashMap<String, TrackName>,
    /// 正在后台取流的在线曲目（非空 = 有曲目在下载，巡查线程不再推进）
    pending: Option<PendingOnline>,
    /// 取流请求序号：每次发起自增，回来时对不上说明用户已经切走
    seq: u64,
    /// 看门狗：上次观测到的播放位置（毫秒）
    last_pos_ms: u64,
    /// 看门狗：上次观测到位置发生变化的时间
    last_progress_at: Instant,
}

/// 播放位置多久不前进就判定「输出已死」（毫秒）。
///
/// 取 2s：正常音频回调延迟远小于它，不会误判；又短到用户几乎察觉不到卡顿。
/// 设这个兜底是因为**有些驱动在设备消失时只会静默停摆、不报任何流错误**。
const STALL_RECOVER_MS: u64 = 2000;

/// 是否需要重建音频输出。
///
/// 两个触发条件（互为补充）：
/// 1. 音频流报错 —— 拔耳机 / 切换默认输出设备时 cpal 会把该流置为不可用；
/// 2. 仍在 `Playing` 但播放位置长时间不前进 —— 驱动不报错时的兜底。
///
/// 只有 `Playing` 才看停摆：暂停本来就不前进，`Ended` 由播完推进逻辑负责。
fn should_recover(output_lost: bool, status: PlayStatus, stalled_for: Option<Duration>) -> bool {
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
fn should_resume_after_recover(status: PlayStatus) -> bool {
    matches!(status, PlayStatus::Playing | PlayStatus::Paused)
}

/// 全局播放器（Tauri State）
pub struct MusicPlayerState {
    inner: Mutex<Inner>,
    /// 供播放线程实时读取的均衡器参数（UI 改动即时生效）
    eq: Arc<Mutex<EqParams>>,
    /// 音频输出流是否已失效（由 cpal 流错误回调置位；见 [`MusicPlayerState::recover_output`]）
    output_lost: Arc<AtomicBool>,
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
                last_pos_ms: 0,
                last_progress_at: Instant::now(),
            }),
            eq: Arc::new(Mutex::new(settings.eq)),
            output_lost: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl MusicPlayerState {
    pub fn eq_handle(&self) -> Arc<Mutex<EqParams>> {
        self.eq.clone()
    }

    /// 设置均衡器参数（播放线程会在下一批同步时读取）
    pub fn set_eq_params(&self, params: EqParams) {
        *self.eq.lock() = params.sanitized();
    }

    /// 懒加载音频设备与播放器
    ///
    /// 这里必须自己装 cpal 流错误回调：rodio 的默认回调只把错误打进日志，
    /// 而「设备被拔掉 / 默认输出被切换」正是靠这个信号才能知道该重建输出流。
    fn ensure_player(inner: &mut Inner, output_lost: &Arc<AtomicBool>) -> Result<(), String> {
        if inner.device.is_none() {
            let flag = output_lost.clone();
            let device = DeviceSinkBuilder::from_default_device()
                .map_err(|e| format!("打开音频输出设备失败: {}", e))?
                .with_error_callback(move |err| {
                    crate::exit_log!("[音乐] 音频输出流错误（设备被移除或切换？）: {}", err);
                    flag.store(true, Ordering::SeqCst);
                })
                .open_stream()
                .map_err(|e| format!("打开音频输出设备失败: {}", e))?;
            inner.device = Some(device);
            // 新流已经建立：清掉失效标记
            output_lost.store(false, Ordering::SeqCst);
        }
        if inner.player.is_none() {
            let mixer = inner.device.as_ref().expect("device").mixer();
            inner.player = Some(Player::connect_new(mixer));
        }
        Ok(())
    }

    /// 丢弃当前（已失效的）输出流，让下一次 [`Self::ensure_player`] 重开默认设备。
    fn discard_output(&self, inner: &mut Inner) {
        inner.player = None;
        inner.device = None;
        self.output_lost.store(false, Ordering::SeqCst);
    }

    /// 音频输出失效后重建：丢弃旧流 → 重开默认设备 → 从原位置续播。
    ///
    /// 为什么必须重建：`MixerDeviceSink` 绑定的是**创建那一刻**那个设备的流，
    /// 设备消失后它既不拉样本也不报错（`Player::play()` 只是翻个标志就返回，
    /// 「恢复」看起来是成功的），于是表现为「拔耳机后再也放不出声，只能退出重进」。
    fn recover_output(&self, inner: &mut Inner) {
        // 只有「正在播 / 已暂停」才需要续播。`Ended` 曲目若在这里重播，
        // 会把已经放完的歌重新拉起来（位置在末尾，等于又从头震一下）；
        // `Idle` 同理只丢坏流。
        let resumable = should_resume_after_recover(inner.status);
        let Some(path) = inner
            .current
            .as_ref()
            .filter(|_| resumable)
            .map(|c| c.path.clone())
        else {
            // 没有当前曲目（或已经播完）：丢掉坏流即可，下次播放时自然重开
            self.discard_output(inner);
            inner.last_pos_ms = 0;
            inner.last_progress_at = Instant::now();
            return;
        };
        let resume_ms = inner
            .player
            .as_ref()
            .map(|p| p.get_pos().as_millis() as u64)
            .unwrap_or(inner.last_pos_ms);
        let was_paused = inner.status == PlayStatus::Paused;
        self.discard_output(inner);

        match Self::play_locked(inner, &self.eq, &self.output_lost, &path) {
            Ok(_) => {
                if let Some(player) = inner.player.as_ref() {
                    let _ = player.try_seek(Duration::from_millis(resume_ms));
                    if was_paused {
                        player.pause();
                    }
                }
                if was_paused {
                    inner.status = PlayStatus::Paused;
                }
                inner.last_pos_ms = resume_ms;
                inner.last_progress_at = Instant::now();
                crate::exit_log!("[音乐] 音频输出已重建，从 {} ms 续播: {}", resume_ms, path);
            }
            Err(e) => {
                inner.status = PlayStatus::Idle;
                inner.current = None;
                crate::exit_log!("[音乐] 音频输出重建失败: {}", e);
            }
        }
    }

    /// 更新停摆看门狗并返回「已停摆多久」；未在播放时返回 None（并重置计时）。
    fn stalled_for(&self, inner: &mut Inner) -> Option<Duration> {
        let pos = inner
            .player
            .as_ref()
            .map(|p| p.get_pos().as_millis() as u64);
        if inner.status != PlayStatus::Playing {
            inner.last_pos_ms = pos.unwrap_or(0);
            inner.last_progress_at = Instant::now();
            return None;
        }
        let pos = pos?;
        if pos != inner.last_pos_ms {
            inner.last_pos_ms = pos;
            inner.last_progress_at = Instant::now();
            return None;
        }
        Some(inner.last_progress_at.elapsed())
    }

    /// 播放指定文件（替换当前曲目）。
    ///
    /// symphonia 解不了的编码（E-AC-3 / AC-3 / DTS，常见于「扩展名被改错」的音源）
    /// 会**退回 ffmpeg 转码**：转成 AAC 落在转码缓存里，之后的播放直接命中缓存。
    pub fn play(&self, path: &str) -> Result<PlayerState, String> {
        let mut inner = self.inner.lock();
        // 输出已失效：本调用马上就会 append 新曲目，直接丢坏流即可（不必先续播旧曲）
        if self.output_lost.load(Ordering::SeqCst) {
            self.discard_output(&mut inner);
        }
        match Self::play_locked(&mut inner, &self.eq, &self.output_lost, path) {
            Ok(state) => Ok(state),
            Err(err) => {
                if !super::transcode::worth_retrying(&err) {
                    return Err(err);
                }
                // 转码要跑外部进程（秒级），**必须先把锁放掉**：
                // 占着锁的话前端每 500ms 的状态轮询会被一起卡住，界面像死了一样。
                drop(inner);
                let cached = super::transcode::ensure(path)?;
                crate::exit_log!("[音乐] 原编码无法解码，已转码后播放: {cached}");
                // 传**原路径**：play_locked 会自己找到转码副本，
                // 于是曲库标签、队列对齐、改名/删除都仍以用户的原文件为准。
                let (title, artist) = super::transcode::labels_for(path);
                self.play_named(path, &title, &artist)
            }
        }
    }

    /// 播放并**指定显示名**（在线曲目专用）。
    ///
    /// 在线播放落在缓存目录里的文件名是内容哈希，既不在曲库里也没有可读性，
    /// 名字只能由调用方给；登记后连播下一首（走 `play_locked`）也能取到正确名字。
    pub fn play_named(
        &self,
        path: &str,
        title: &str,
        artist: &str,
    ) -> Result<PlayerState, String> {
        let mut inner = self.inner.lock();
        // 输出已失效：本调用马上就会 append 新曲目，直接丢坏流即可
        if self.output_lost.load(Ordering::SeqCst) {
            self.discard_output(&mut inner);
        }
        inner.names.insert(
            path.to_string(),
            TrackName {
                title: title.to_string(),
                artist: artist.to_string(),
            },
        );
        Self::play_locked(&mut inner, &self.eq, &self.output_lost, path)
    }

    /// 重置播放队列（前端在曲库或播放模式变化时调用）
    ///
    /// `current` 传当前正在播放的曲目路径：随机模式下会保证它仍在队列中且被选中，
    /// 避免重排后把正在播的那首「挤掉」。
    pub fn set_queue(&self, paths: Vec<String>, mode: &str) -> Result<(), String> {
        let items = paths.into_iter().map(QueueItem::Path).collect();
        let mut inner = self.inner.lock();
        let current = inner.current.as_ref().map(|c| c.path.clone());
        let mode = PlayMode::from_str(mode);
        inner.queue.set(items, mode, current.as_deref());
        Ok(())
    }

    /// 用**队列条目**重置队列（在线搜索结果替换播放列表时用）；播放模式沿用当前设置。
    ///
    /// 与 [`Self::set_queue`] 的区别是条目可以是**还没落盘的在线曲目**。
    pub fn set_queue_items(&self, items: Vec<QueueItem>, current: Option<&str>) -> Result<(), String> {
        let mode = PlayMode::from_str(&settings::load_settings().play_mode);
        let mut inner = self.inner.lock();
        // 没指定就以「正在播的那首」为当前曲：随机模式下它必须留在队列里
        let current = current
            .map(str::to_string)
            .or_else(|| inner.current.as_ref().map(|c| c.path.clone()));
        inner.queue.set(items, mode, current.as_deref());
        Ok(())
    }

    /// 曲目在磁盘上改名后同步当前曲目与队列里的路径（播放不中断）。
    pub fn rename_path(&self, old: &str, new: &str) {
        let mut inner = self.inner.lock();
        if let Some(current) = inner.current.as_mut() {
            if current.path == old {
                current.path = new.to_string();
            }
        }
        // 登记名跟着换 key，否则改名后在线曲目的名字会丢失
        if let Some(name) = inner.names.remove(old) {
            inner.names.insert(new.to_string(), name);
        }
        inner.queue.rename_path(old, new);
    }

    /// 曲目被删除后同步播放器：从队列移除；返回「被删的是不是当前曲目」。
    ///
    /// 调用方据此决定是否切下一首 —— 这里**不能**自己调 `next()`：
    /// `parking_lot::Mutex` 不可重入，在持锁状态下再取一次锁会直接死锁。
    pub fn forget_paths(&self, paths: &[String]) -> bool {
        let mut inner = self.inner.lock();
        let playing_removed = inner
            .current
            .as_ref()
            .map(|c| paths.iter().any(|p| p == &c.path))
            .unwrap_or(false);
        let current_removed = inner.queue.remove_paths(paths) || playing_removed;
        if current_removed {
            // 立刻停掉输出，避免继续播放一个已被删除的文件
            if let Some(player) = inner.player.as_ref() {
                player.clear();
                player.stop();
            }
            inner.current = None;
            inner.status = PlayStatus::Idle;
        }
        current_removed
    }

    /// 下一首（用户点「下一首」）
    pub fn next(&self) -> Result<AdvanceOutcome, String> {
        let mut inner = self.inner.lock();
        let item = inner.queue.advance();
        self.step(&mut inner, item)
    }

    /// 上一首
    pub fn prev(&self) -> Result<AdvanceOutcome, String> {
        let mut inner = self.inner.lock();
        let item = inner.queue.back();
        self.step(&mut inner, item)
    }

    /// 推进到指定条目并播放；条是在线曲目时把取流任务交回调用方。
    fn step(&self, inner: &mut Inner, item: Option<QueueItem>) -> Result<AdvanceOutcome, String> {
        let Some(item) = item else {
            return Ok(AdvanceOutcome::Playing(Self::snapshot(inner)));
        };
        match item {
            QueueItem::Path(path) => {
                Self::play_locked(inner, &self.eq, &self.output_lost, &path).map(AdvanceOutcome::Playing)
            }
            QueueItem::Online {
                file,
                item,
                quality,
            } => {
                // 记下「正在取流」：巡查线程据此停止推进，否则每 250ms 推进一次会连跳好几首
                inner.seq += 1;
                let pending = PendingOnline {
                    file,
                    item,
                    quality,
                    seq: inner.seq,
                };
                inner.pending = Some(pending.clone());
                // 挪出 Playing：输出队列已经空了，留着会让巡查判定「又播完一首」而立即再推进
                inner.status = PlayStatus::Idle;
                Ok(AdvanceOutcome::Online(pending))
            }
        }
    }

    /// 在线曲目已落盘：接上播放（由取流任务在拿到本地文件后调用）。
    ///
    /// `seq` 对不上说明用户已经切到别的曲目 —— 这次结果作废，不打断当前播放。
    pub fn play_online_resolved(
        &self,
        path: &str,
        title: &str,
        artist: &str,
        seq: u64,
    ) -> Result<PlayerState, String> {
        let mut inner = self.inner.lock();
        let matches = inner.pending.as_ref().map(|p| p.seq == seq).unwrap_or(false);
        inner.pending = None;
        if !matches {
            return Ok(Self::snapshot(&mut inner));
        }
        // 落盘后换成本地路径：回退 / 切模式时不必再取一次流
        inner.queue.materialize_current(path);
        if self.output_lost.load(Ordering::SeqCst) {
            self.discard_output(&mut inner);
        }
        inner.names.insert(
            path.to_string(),
            TrackName {
                title: title.to_string(),
                artist: artist.to_string(),
            },
        );
        Self::play_locked(&mut inner, &self.eq, &self.output_lost, path)
    }

    /// 在线曲目取流失败：清掉「正在取流」标记（不清的话巡查线程会一直等下去）。
    pub fn abandon_online(&self, seq: u64) {
        let mut inner = self.inner.lock();
        if inner.pending.as_ref().map(|p| p.seq == seq).unwrap_or(false) {
            inner.pending = None;
        }
    }

    /// 播放/暂停切换（播放器热键用）：正在播→暂停；暂停→继续；
    /// 空闲/已播完→接着当前曲目（没有则从队列取下一首）
    pub fn toggle(&self) -> Result<PlayerState, String> {
        let mut inner = self.inner.lock();
        // 输出已失效：先重建，否则下面的 play()/pause() 只是空操作（用户看到的「点了没反应」）
        if self.output_lost.load(Ordering::SeqCst) {
            self.recover_output(&mut inner);
        }
        match inner.status {
            PlayStatus::Playing => {
                if let Some(player) = inner.player.as_ref() {
                    player.pause();
                }
                inner.status = PlayStatus::Paused;
                Ok(Self::snapshot(&mut inner))
            }
            PlayStatus::Paused => {
                if let Some(player) = inner.player.as_ref() {
                    player.play();
                }
                inner.status = PlayStatus::Playing;
                Ok(Self::snapshot(&mut inner))
            }
            _ => {
                // 队列里可能是还没落盘的在线曲目：那种情况这里起不来（没有本地文件），
                // 交给巡查线程或用户的下一次操作去取流
                let path = inner
                    .current
                    .as_ref()
                    .map(|c| c.path.clone())
                    .or_else(|| {
                        inner
                            .queue
                            .current()
                            .and_then(QueueItem::path)
                            .map(str::to_string)
                    })
                    .or_else(|| {
                        inner
                            .queue
                            .first()
                            .and_then(|item| item.path().map(str::to_string))
                    });
                match path {
                    Some(path) => {
                        Self::play_locked(&mut inner, &self.eq, &self.output_lost, &path)
                    }
                    None => Ok(Self::snapshot(&mut inner)),
                }
            }
        }
    }

    /// 后台巡查（由 [`start_queue_watcher`] 每 250ms 调用）：
    /// 当前曲目已放完（输出队列为空）→ 自动切下一首。
    ///
    /// 判定**不限于 `Playing`**：`Ended` 同样继续推进（自愈）—— 无论谁读过状态、
    /// 或历史状态被置为 Ended，续播都不会被卡死。
    /// 失败（如文件已被删除）时继续尝试后续曲目，最多绕队列一圈。
    /// `app` 为 None 时（单测）在线曲目直接放弃推进 —— 没有句柄就无法取流，
    /// 卡在「等待取流」上更糟。
    pub fn tick(&self, app: Option<&tauri::AppHandle>) {
        let mut inner = self.inner.lock();

        // ⓪ 有在线曲目正在取流：**不推进**。等它落盘后由 `play_online_resolved` 接上；
        //    否则巡查每 250ms 推进一次，一首没下载完就跳过了好几首。
        if inner.pending.is_some() {
            return;
        }

        // ① 输出自愈：设备报错（拔耳机/换默认设备）或位置停摆 → 重建并从原位置续播。
        //    这一段必须在「播完判定」之前：输出死了以后位置不会再前进，
        //    不先自愈的话队列永远等不到 empty()，表现为「彻底卡住」。
        let stalled = self.stalled_for(&mut inner);
        if should_recover(
            self.output_lost.load(Ordering::SeqCst),
            inner.status,
            stalled,
        ) {
            self.recover_output(&mut inner);
        }

        // ② 播完自动切下一首
        let exhausted = inner.current.is_some()
            && matches!(inner.status, PlayStatus::Playing | PlayStatus::Ended)
            && matches!(inner.player.as_ref(), Some(player) if player.empty());
        if !exhausted {
            return;
        }

        let attempts = inner.queue.len().max(1);
        for _ in 0..attempts {
            let item = inner.queue.advance();
            match self.step(&mut inner, item) {
                Ok(AdvanceOutcome::Playing(_)) => {
                    if let Some(current) = inner.current.as_ref() {
                        crate::exit_log!("[音乐] 播完自动切下一首: {}", current.path);
                    }
                    return;
                }
                // 在线曲目：交给异步任务取流落盘（巡查线程是普通线程，不能 await）
                Ok(AdvanceOutcome::Online(pending)) => {
                    match app {
                        Some(app) => {
                            crate::exit_log!("[音乐] 播完自动切下一首（在线曲目，取流中）");
                            spawn_online_resolve(app.clone(), pending);
                        }
                        None => inner.pending = None,
                    }
                    return;
                }
                // 解码失败（文件损坏/被删除）：跳过，继续下一首
                Err(_) => {}
            }
        }
        inner.status = PlayStatus::Ended;
    }

    /// 真正播放：调用方必须已持有 `inner` 的锁
    fn play_locked(
        inner: &mut Inner,
        eq: &Arc<Mutex<EqParams>>,
        output_lost: &Arc<AtomicBool>,
        path: &str,
    ) -> Result<PlayerState, String> {
        // 之前转过码的文件（symphonia 解不了的编码）：播转码后的副本。
        // 报错信息仍指向**原文件** —— 容器、扩展名那些线索都在原文件上。
        let transcoded = super::transcode::cached(path);
        let target = transcoded.as_deref().unwrap_or(path);
        let file = File::open(target).map_err(|e| format!("打开文件失败: {}", e))?;
        // 必须显式告知字节长度：symphonia 只有在已知流长度时才允许随机访问，
        // 否则 try_seek 会返回 RandomAccessNotSupported（进度条拖动失效）。
        let byte_len = file.metadata().map(|m| m.len()).unwrap_or(0);
        let mut builder = Decoder::builder().with_data(BufReader::new(file));
        if byte_len > 0 {
            builder = builder.with_byte_len(byte_len);
        }
        let decoder = builder
            .build()
            .map_err(|e| decode_failure_message(path, &e))?;
        let decoder_duration_ms = decoder
            .total_duration()
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        // 显示名：登记名（在线缓存）→ 曲库标签 → 文件名
        let named = inner.names.get(path).cloned();
        // 已有登记名时不必再读曲库：缓存文件本来就不在曲库里
        let library_track = if named.is_some() {
            None
        } else {
            library::load_library()
                .tracks
                .into_iter()
                .find(|t| library::same_path(&t.path, path))
        };
        let (title, artist) = track_labels(path, named.as_ref(), library_track.as_ref());
        let duration_ms = library_track
            .as_ref()
            .map(|t| t.duration_ms)
            .filter(|d| *d > 0)
            .unwrap_or(decoder_duration_ms);

        let source = EqSource::new(decoder, eq.clone());

        Self::ensure_player(inner, output_lost)?;
        // 每首重建 Player：位置计数归零，避免复用时的残留队列
        let mixer = inner.device.as_ref().expect("device").mixer();
        let player = Player::connect_new(mixer);
        player.set_volume(inner.volume);
        player.append(source);
        inner.player = Some(player);
        inner.status = PlayStatus::Playing;
        // 新曲目从 0 开始：重置看门狗，给输出流留出启动窗口
        inner.last_pos_ms = 0;
        inner.last_progress_at = Instant::now();
        inner.current = Some(CurrentTrack {
            path: path.to_string(),
            title,
            artist,
            duration_ms,
        });
        // 让队列下标对齐到本曲，之后的「下一首 / 上一首」都从这里继续
        inner.queue.focus(path);
        Ok(Self::snapshot(inner))
    }

    pub fn pause(&self) -> PlayerState {
        let mut inner = self.inner.lock();
        if let Some(player) = inner.player.as_ref() {
            player.pause();
            if inner.status == PlayStatus::Playing {
                inner.status = PlayStatus::Paused;
            }
        }
        Self::snapshot(&mut inner)
    }

    pub fn resume(&self) -> PlayerState {
        let mut inner = self.inner.lock();
        // 输出已失效：先重建（会从原位置续播）；否则这里的 play() 点下去不会有声
        if self.output_lost.load(Ordering::SeqCst) {
            self.recover_output(&mut inner);
        }
        if let Some(player) = inner.player.as_ref() {
            // 已播完的曲目不允许“继续”
            if inner.status == PlayStatus::Paused {
                player.play();
                inner.status = PlayStatus::Playing;
            }
        }
        Self::snapshot(&mut inner)
    }

    pub fn stop(&self) -> PlayerState {
        let mut inner = self.inner.lock();
        if let Some(player) = inner.player.as_ref() {
            player.clear();
            player.stop();
        }
        inner.status = PlayStatus::Idle;
        inner.current = None;
        Self::snapshot(&mut inner)
    }

    pub fn seek(&self, position_ms: u64) -> Result<PlayerState, String> {
        let mut inner = self.inner.lock();
        // 输出已失效：先重建再跳转，否则 try_seek 会作用在一个已经死掉的流上
        if self.output_lost.load(Ordering::SeqCst) {
            self.recover_output(&mut inner);
        }
        let was_ended = inner.status == PlayStatus::Ended;
        {
            let player = inner
                .player
                .as_ref()
                .ok_or_else(|| "当前没有正在播放的曲目".to_string())?;
            player
                .try_seek(Duration::from_millis(position_ms))
                .map_err(|e| format!("跳转失败: {}", e))?;
            // 播完后再拖动进度条 → 视为继续播放
            if was_ended {
                player.play();
            }
        }
        if was_ended {
            inner.status = PlayStatus::Playing;
        }
        Ok(Self::snapshot(&mut inner))
    }

    pub fn set_volume(&self, volume: f32) -> PlayerState {
        let mut inner = self.inner.lock();
        let volume = if volume.is_finite() {
            volume.clamp(0.0, 1.0)
        } else {
            inner.volume
        };
        inner.volume = volume;
        if let Some(player) = inner.player.as_ref() {
            player.set_volume(volume);
        }
        Self::snapshot(&mut inner)
    }

    pub fn state(&self) -> PlayerState {
        let mut inner = self.inner.lock();
        Self::snapshot(&mut inner)
    }

    /// 当前曲目的本地路径（在线曲目落盘后才有）
    pub fn current_path(&self) -> Option<String> {
        let inner = self.inner.lock();
        inner.current.as_ref().map(|c| c.path.clone())
    }

    /// 组装状态快照。
    ///
    /// **这是只读路径**（前端每 500ms 轮询 `music_get_state` 都会走到这里），
    /// 绝不能修改播放状态：旧实现在这里把「输出已空」判为 `Ended`，而 `tick()` 只
    /// 在 `Playing` 时推进 —— 于是任何一次轮询抢先置为 `Ended` 都会让自动切歌
    /// **永久失效**（表现就是托盘模式下「播完不切下一首」）。
    /// 播完的判定与推进统一由 [`MusicPlayerState::tick`] 负责。
    fn snapshot(inner: &mut Inner) -> PlayerState {
        let duration_ms = inner.current.as_ref().map(|c| c.duration_ms).unwrap_or(0);
        let mut position_ms = 0u64;
        if inner.current.is_some() {
            if let Some(player) = inner.player.as_ref() {
                position_ms = player.get_pos().as_millis() as u64;
            }
        }
        if duration_ms > 0 {
            position_ms = position_ms.min(duration_ms);
        }

        PlayerState {
            status: inner.status,
            path: inner.current.as_ref().map(|c| c.path.clone()),
            title: inner.current.as_ref().map(|c| c.title.clone()),
            artist: inner.current.as_ref().map(|c| c.artist.clone()),
            position_ms,
            duration_ms,
            volume: inner.volume,
        }
    }
}

/// 保存设置（音量 + 模式 + 音效）并从播放器侧收下音效参数
pub fn persist_settings(state: &MusicPlayerState, settings: &MusicSettings) -> Result<MusicSettings, String> {
    let settings = settings.clone().sanitized();
    state.set_volume(settings.volume);
    state.set_eq_params(settings.eq.clone());
    settings::save_settings(&settings)?;
    Ok(settings)
}

/// 把在线曲目交给异步运行时：取流 → 落缓存 → 接上播放。
///
/// 为什么不在巡查线程里等：巡查线程是**普通线程**（不能 await），而取流是秒级操作；
/// 真要等就得阻塞 250ms 的巡检循环。等待期间由 `Inner.pending` 挡住推进，
/// 任务完成后自己调 `play_online_resolved` 接上 —— 巡查线程不需要知道结果。
pub fn spawn_online_resolve(app: tauri::AppHandle, pending: PendingOnline) {
    tauri::async_runtime::spawn(async move {
        let seq = pending.seq;
        let outcome =
            super::plugin_playback::materialize(&app, &pending.file, &pending.item, &pending.quality)
                .await;
        let Some(state) = app.try_state::<MusicPlayerState>() else {
            return;
        };
        match outcome {
            Ok(materialized) => {
                let path = materialized.path.to_string_lossy().to_string();
                if let Err(err) =
                    state.play_online_resolved(&path, &materialized.title, &materialized.artist, seq)
                {
                    crate::exit_log!("[音乐] 在线曲目落盘后播放失败: {err}");
                    state.abandon_online(seq);
                }
            }
            Err(err) => {
                crate::exit_log!("[音乐] 在线曲目取流失败: {err}");
                state.abandon_online(seq);
            }
        }
    });
}

/// 启动后台巡查线程：每 250ms 检查一次「本曲是否播完」，播完则由后端自动切下一首。
///
/// 必须由后端驱动（而不是前端轮询后切换）：窗口隐藏 / 最小化到托盘后，
/// WebView2 会节流甚至暂停前端定时器，只有 Rust 侧才能可靠地续播。
pub fn start_queue_watcher(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        crate::exit_log!("[音乐] 后台巡查线程已启动（播完自动切下一首）");
        loop {
            std::thread::sleep(Duration::from_millis(250));
            match app.try_state::<MusicPlayerState>() {
                Some(state) => state.tick(Some(&app)),
                // 应用已退出（State 已回收）：结束线程
                None => break,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    /// 设备报错一定触发重建（不管播放状态）。
    #[test]
    fn test_should_recover_on_output_lost() {
        assert!(should_recover(true, PlayStatus::Playing, None));
        assert!(should_recover(true, PlayStatus::Paused, None));
        assert!(should_recover(true, PlayStatus::Idle, None));
    }

    /// 停摆只有 Playing 才算：暂停本来就不前进，播完交给推进逻辑。
    #[test]
    fn test_should_recover_on_stall_only_while_playing() {
        let stalled = Some(Duration::from_millis(STALL_RECOVER_MS));
        assert!(should_recover(false, PlayStatus::Playing, stalled));
        assert!(!should_recover(false, PlayStatus::Paused, stalled));
        assert!(!should_recover(false, PlayStatus::Ended, stalled));
        assert!(!should_recover(false, PlayStatus::Idle, stalled));
    }

    /// 只有正在播/已暂停才在重建后续播：已播完的曲目不能被「拉起来」。
    #[test]
    fn test_recover_resumes_only_for_active_playback() {
        assert!(should_resume_after_recover(PlayStatus::Playing));
        assert!(should_resume_after_recover(PlayStatus::Paused));
        assert!(!should_resume_after_recover(PlayStatus::Ended));
        assert!(!should_resume_after_recover(PlayStatus::Idle));
    }

    /// 未达到阈值不重建（正常播放中位置每 250ms 都会前进，这里模拟「刚查过一次」）。
    #[test]
    fn test_should_not_recover_before_stall_threshold() {
        assert!(!should_recover(
            false,
            PlayStatus::Playing,
            Some(Duration::from_millis(STALL_RECOVER_MS - 1))
        ));
        // 位置在前进（stalled_for 返回 None）→ 不重建
        assert!(!should_recover(false, PlayStatus::Playing, None));
    }

    #[test]
    fn test_initial_state_is_idle() {
        let state = MusicPlayerState::default();
        let snapshot = state.state();
        assert_eq!(snapshot.status, PlayStatus::Idle);
        assert!(snapshot.path.is_none());
        assert_eq!(snapshot.position_ms, 0);
        assert!(snapshot.volume >= 0.0 && snapshot.volume <= 1.0);
    }

    /// 在线缓存文件的名字是哈希，必须显示登记的歌名；没有登记时才回退文件名。
    #[test]
    fn registered_name_beats_the_cache_file_name() {
        let named = TrackName {
            title: "Abyss".to_string(),
            artist: "Yungblud".to_string(),
        };
        let (title, artist) = track_labels("D:/cache/a3f9e1c0.m4a", Some(&named), None);
        assert_eq!(title, "Abyss");
        assert_eq!(artist, "Yungblud");

        // 没登记 → 回退文件名（曲库外的本地文件）
        let (title, _) = track_labels("D:/music/周杰伦 - 稻香.mp3", None, None);
        assert_eq!(title, "周杰伦 - 稻香");

        // 插件给了空标题 → 视同没登记，继续回退
        let (title, _) = track_labels(
            "D:/cache/x.m4a",
            Some(&TrackName {
                title: "  ".to_string(),
                artist: String::new(),
            }),
            None,
        );
        assert_eq!(title, "x");
    }

    #[test]
    fn test_play_missing_file_reports_error() {
        let state = MusicPlayerState::default();
        let err = state.play("D:/definitely/missing/song.mp3").unwrap_err();
        assert!(err.contains("打开文件失败"), "实际错误: {err}");
    }

    #[test]
    fn test_set_volume_clamps() {
        let state = MusicPlayerState::default();
        assert_eq!(state.set_volume(5.0).volume, 1.0);
        assert_eq!(state.set_volume(-1.0).volume, 0.0);
        assert_eq!(state.set_volume(f32::NAN).volume, 0.0);
    }

    #[test]
    fn test_stop_clears_current_track() {
        let state = MusicPlayerState::default();
        let snapshot = state.stop();
        assert_eq!(snapshot.status, PlayStatus::Idle);
        assert!(snapshot.title.is_none());
    }

    #[test]
    fn test_resume_without_pause_does_nothing() {
        let state = MusicPlayerState::default();
        assert_eq!(state.resume().status, PlayStatus::Idle);
        assert_eq!(state.pause().status, PlayStatus::Idle);
    }

    #[test]
    fn test_toggle_with_empty_queue_is_noop() {
        // 空队列下按播放键不应崩溃，也不应尝试打开音频设备
        let state = MusicPlayerState::default();
        assert_eq!(state.toggle().unwrap().status, PlayStatus::Idle);
    }

    #[test]
    fn test_set_queue_then_next_reports_missing_file() {
        let state = MusicPlayerState::default();
        state
            .set_queue(vec!["D:/definitely/missing/song.mp3".to_string()], "sequence")
            .unwrap();
        let err = state.next().unwrap_err();
        assert!(err.contains("打开文件失败"), "实际错误: {err}");
    }

    #[test]
    fn test_tick_without_playing_does_nothing() {
        // 未在播放时巡查不应改变状态（也不会误触发切歌）
        let state = MusicPlayerState::default();
        state.set_queue(vec!["D:/missing/a.mp3".to_string()], "sequence").unwrap();
        state.tick(None);
        assert_eq!(state.state().status, PlayStatus::Idle);
    }

    #[test]
    fn test_snapshot_is_read_only() {
        // 回归（Q-0078 第二轮）：快照/读状态曾经会把 Playing 判成 Ended，
        // 而 tick() 只在 Playing 时推进 —— 一次轮询就能让自动切歌永久失效。
        // 这里退化到无音频设备也能断言的部分：未播放时读状态不改任何状态。
        let state = MusicPlayerState::default();
        let before = state.state();
        let after = state.state();
        assert_eq!(before.status, after.status);
        assert_eq!(after.status, PlayStatus::Idle);
    }

    /// 写一个最小 16-bit 单声道 PCM WAV（测试用，避免依赖 ffmpeg）
    #[cfg(test)]
    fn write_sine_wav(path: &std::path::Path, seconds: f32) {
        use std::io::Write;
        const SAMPLE_RATE: u32 = 44_100;
        let total = (SAMPLE_RATE as f32 * seconds) as u32;
        let data_len = total * 2;
        let mut buf = Vec::with_capacity(44 + data_len as usize);
        buf.extend_from_slice(b"RIFF");
        buf.extend_from_slice(&(36 + data_len).to_le_bytes());
        buf.extend_from_slice(b"WAVEfmt ");
        buf.extend_from_slice(&16u32.to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
        buf.extend_from_slice(&1u16.to_le_bytes()); // 单声道
        buf.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
        buf.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
        buf.extend_from_slice(&2u16.to_le_bytes());
        buf.extend_from_slice(&16u16.to_le_bytes());
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&data_len.to_le_bytes());
        for i in 0..total {
            let t = i as f32 / SAMPLE_RATE as f32;
            let s = (t * 440.0 * std::f32::consts::TAU).sin() * 0.2;
            buf.extend_from_slice(&((s * i16::MAX as f32) as i16).to_le_bytes());
        }
        std::fs::File::create(path)
            .and_then(|mut f| f.write_all(&buf))
            .expect("写测试音频");
    }

    /// 端到端回归：**同时模拟前端轮询与后台巡查**，播完必须自动切下一首。
    ///
    /// 需要音频输出设备，因此默认忽略；在本机手动验证：
    /// `cargo test --lib -- --ignored test_auto_advance_survives_state_polling`
    #[test]
    #[ignore = "需要音频输出设备（真实播放），用 `cargo test --lib -- --ignored` 手动跑"]
    fn test_auto_advance_survives_state_polling() {
        let dir = std::env::temp_dir().join(format!("kira-music-race-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let a_path = dir.join("a.wav");
        let b_path = dir.join("b.wav");
        write_sine_wav(&a_path, 1.0);
        write_sine_wav(&b_path, 1.0);
        let a = a_path.to_string_lossy().to_string();
        let b = b_path.to_string_lossy().to_string();

        let state = MusicPlayerState::default();
        state.set_queue(vec![a.clone(), b.clone()], "sequence").unwrap();
        state.play(&a).expect("应能开始播放（需要音频输出设备）");

        let start = Instant::now();
        let mut last_tick = Instant::now();
        let mut last_poll = Instant::now();
        let mut advanced = false;
        while start.elapsed() < Duration::from_secs(15) {
            std::thread::sleep(Duration::from_millis(25));
            // 后台巡查线程（真实实现 250ms 一次）
            if last_tick.elapsed() >= Duration::from_millis(250) {
                last_tick = Instant::now();
                state.tick(None);
            }
            // 前端轮询 music_get_state（隐藏窗口下会被节流，可见时 500ms 一次）
            if last_poll.elapsed() >= Duration::from_millis(25) {
                last_poll = Instant::now();
                assert_eq!(
                    state.state().status,
                    PlayStatus::Playing,
                    "读状态不得改变播放状态（旧 bug：轮询把它置成 Ended 后永久不切歌）"
                );
            }
            if state.state().path.as_deref() == Some(b.as_str()) {
                advanced = true;
                break;
            }
        }
        state.stop();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(advanced, "有前端轮询时也必须能自动切歌");
    }

    #[test]
    fn test_tick_skips_broken_tracks_and_ends_on_empty() {
        // 队列里全是坏文件：巡查切换时应逐个跳过，最终置为 Ended 而不是卡在 Playing
        let state = MusicPlayerState::default();
        state
            .set_queue(
                vec![
                    "D:/missing/a.mp3".to_string(),
                    "D:/missing/b.mp3".to_string(),
                ],
                "sequence",
            )
            .unwrap();
        state.tick(None);
        assert_eq!(state.state().status, PlayStatus::Idle, "未在播放时不应被巡查改变");
    }
}
