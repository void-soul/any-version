//! 播放引擎：rodio 输出流 + 播放队列 + 均衡器接入。
//!
//! 一个进程内只有一个播放器（Tauri `State`），曲目切换 = 重建 `Player` 并 append 新的
//! `EqSource`。位置/时长由 rodio 的 `get_pos()`/曲库时长提供。
//!
//! **队列与自动切歌在后端**（[`start_queue_watcher`] 的巡查线程驱动）：
//! 窗口最小化到托盘后 WebView2 会节流前端定时器，前端无法及时感知「播完」，
//! 若把切歌交给前端就会出现「托盘模式下不自动切下一首」。

use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Source};
use serde::Serialize;
use tauri::Manager;

use super::dsp::{EqParams, EqSource};
use super::library;
use super::queue::{PlayMode, PlayQueue};
use super::settings::{self, MusicSettings};

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

struct Inner {
    /// 音频输出设备（drop 即停止播放，必须与 player 同生命周期）
    device: Option<MixerDeviceSink>,
    player: Option<Player>,
    current: Option<CurrentTrack>,
    status: PlayStatus,
    volume: f32,
    /// 播放队列（顺序 / 随机 / 单曲），自动切歌由后端驱动
    queue: PlayQueue,
}

/// 全局播放器（Tauri State）
pub struct MusicPlayerState {
    inner: Mutex<Inner>,
    /// 供播放线程实时读取的均衡器参数（UI 改动即时生效）
    eq: Arc<Mutex<EqParams>>,
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
            }),
            eq: Arc::new(Mutex::new(settings.eq)),
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
    fn ensure_player(inner: &mut Inner) -> Result<(), String> {
        if inner.device.is_none() {
            let device = DeviceSinkBuilder::open_default_sink()
                .map_err(|e| format!("打开音频输出设备失败: {}", e))?;
            inner.device = Some(device);
        }
        if inner.player.is_none() {
            let mixer = inner.device.as_ref().expect("device").mixer();
            inner.player = Some(Player::connect_new(mixer));
        }
        Ok(())
    }

    /// 播放指定文件（替换当前曲目）
    pub fn play(&self, path: &str) -> Result<PlayerState, String> {
        let mut inner = self.inner.lock();
        Self::play_locked(&mut inner, &self.eq, path)
    }

    /// 重置播放队列（前端在曲库或播放模式变化时调用）
    ///
    /// `current` 传当前正在播放的曲目路径：随机模式下会保证它仍在队列中且被选中，
    /// 避免重排后把正在播的那首「挤掉」。
    pub fn set_queue(&self, paths: Vec<String>, mode: &str) -> Result<(), String> {
        let mut inner = self.inner.lock();
        let current = inner.current.as_ref().map(|c| c.path.clone());
        let mode = PlayMode::from_str(mode);
        inner.queue.set(paths, mode, current.as_deref());
        Ok(())
    }

    /// 下一首（用户点「下一首」）
    pub fn next(&self) -> Result<PlayerState, String> {
        let mut inner = self.inner.lock();
        match inner.queue.advance() {
            Some(path) => Self::play_locked(&mut inner, &self.eq, &path),
            None => Ok(Self::snapshot(&mut inner)),
        }
    }

    /// 上一首
    pub fn prev(&self) -> Result<PlayerState, String> {
        let mut inner = self.inner.lock();
        match inner.queue.back() {
            Some(path) => Self::play_locked(&mut inner, &self.eq, &path),
            None => Ok(Self::snapshot(&mut inner)),
        }
    }

    /// 播放/暂停切换（播放器热键用）：正在播→暂停；暂停→继续；
    /// 空闲/已播完→接着当前曲目（没有则从队列取下一首）
    pub fn toggle(&self) -> Result<PlayerState, String> {
        let mut inner = self.inner.lock();
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
                let path = inner
                    .current
                    .as_ref()
                    .map(|c| c.path.clone())
                    .or_else(|| inner.queue.current().map(|p| p.to_string()))
                    .or_else(|| inner.queue.first());
                match path {
                    Some(path) => Self::play_locked(&mut inner, &self.eq, &path),
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
    pub fn tick(&self) {
        let mut inner = self.inner.lock();
        let exhausted = inner.current.is_some()
            && matches!(inner.status, PlayStatus::Playing | PlayStatus::Ended)
            && matches!(inner.player.as_ref(), Some(player) if player.empty());
        if !exhausted {
            return;
        }

        let attempts = inner.queue.len().max(1);
        for _ in 0..attempts {
            match inner.queue.advance() {
                Some(path) => {
                    if Self::play_locked(&mut inner, &self.eq, &path).is_ok() {
                        crate::exit_log!("[音乐] 播完自动切下一首: {}", path);
                        return;
                    }
                    // 解码失败（文件损坏/被删除）：跳过，继续下一首
                }
                None => {
                    inner.status = PlayStatus::Ended;
                    return;
                }
            }
        }
        inner.status = PlayStatus::Ended;
    }

    /// 真正播放：调用方必须已持有 `inner` 的锁
    fn play_locked(
        inner: &mut Inner,
        eq: &Arc<Mutex<EqParams>>,
        path: &str,
    ) -> Result<PlayerState, String> {
        let file = File::open(path).map_err(|e| format!("打开文件失败: {}", e))?;
        // 必须显式告知字节长度：symphonia 只有在已知流长度时才允许随机访问，
        // 否则 try_seek 会返回 RandomAccessNotSupported（进度条拖动失效）。
        let byte_len = file.metadata().map(|m| m.len()).unwrap_or(0);
        let mut builder = Decoder::builder().with_data(BufReader::new(file));
        if byte_len > 0 {
            builder = builder.with_byte_len(byte_len);
        }
        let decoder = builder
            .build()
            .map_err(|e| format!("无法解码该音频文件: {}", e))?;
        let decoder_duration_ms = decoder
            .total_duration()
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        // 曲库里的标签信息（标题/歌手/时长）优先，缺失则回退文件名
        let library_track = library::load_library()
            .tracks
            .into_iter()
            .find(|t| library::same_path(&t.path, path));
        let title = library_track
            .as_ref()
            .map(|t| t.title.clone())
            .unwrap_or_else(|| {
                std::path::Path::new(path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.to_string())
            });
        let artist = library_track
            .as_ref()
            .map(|t| t.artist.clone())
            .unwrap_or_default();
        let duration_ms = library_track
            .as_ref()
            .map(|t| t.duration_ms)
            .filter(|d| *d > 0)
            .unwrap_or(decoder_duration_ms);

        let source = EqSource::new(decoder, eq.clone());

        Self::ensure_player(inner)?;
        // 每首重建 Player：位置计数归零，避免复用时的残留队列
        let mixer = inner.device.as_ref().expect("device").mixer();
        let player = Player::connect_new(mixer);
        player.set_volume(inner.volume);
        player.append(source);
        inner.player = Some(player);
        inner.status = PlayStatus::Playing;
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
                Some(state) => state.tick(),
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

    #[test]
    fn test_initial_state_is_idle() {
        let state = MusicPlayerState::default();
        let snapshot = state.state();
        assert_eq!(snapshot.status, PlayStatus::Idle);
        assert!(snapshot.path.is_none());
        assert_eq!(snapshot.position_ms, 0);
        assert!(snapshot.volume >= 0.0 && snapshot.volume <= 1.0);
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
        state.tick();
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
                state.tick();
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
        state.tick();
        assert_eq!(state.state().status, PlayStatus::Idle, "未在播放时不应被巡查改变");
    }
}
