//! 播放引擎：rodio 输出流 + 单曲播放状态机 + 均衡器接入。
//!
//! 一个进程内只有一个播放器（Tauri `State`），曲目切换 = 重建 `Player` 并 append 新的
//! `EqSource`。位置/时长由 rodio 的 `get_pos()`/曲库时长提供，前端轮询取状态；
//! 「播完」由本模块判定为 `Ended`，下一首由前端按播放模式决定。

use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player, Source};
use serde::Serialize;

use super::dsp::{EqParams, EqSource};
use super::library;
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

        let source = EqSource::new(decoder, self.eq_handle());

        let mut inner = self.inner.lock();
        Self::ensure_player(&mut inner)?;
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
        Ok(Self::snapshot(&mut inner))
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

    /// 组装状态快照；顺带把「队列已空」判为播完
    fn snapshot(inner: &mut Inner) -> PlayerState {
        let duration_ms = inner.current.as_ref().map(|c| c.duration_ms).unwrap_or(0);
        let mut position_ms = 0u64;
        // 先只读地取值，再改状态，避免同时持有 inner 的可变与不可变借用
        let mut ended = false;
        if inner.current.is_some() {
            if let Some(player) = inner.player.as_ref() {
                position_ms = player.get_pos().as_millis() as u64;
                if inner.status == PlayStatus::Playing && player.empty() {
                    ended = true;
                }
            }
        }
        if ended {
            inner.status = PlayStatus::Ended;
            position_ms = duration_ms.max(position_ms);
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

#[cfg(test)]
mod tests {
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
}
