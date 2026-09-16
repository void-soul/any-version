//! 音乐播放器模块（纯本地）。
//!
//! - `library`：曲库（目录扫描 + lofty 标签/时长 + `data_dir/music/library.json` 持久化，
//!   **启动不自动重扫**，只有手动刷新才读盘）
//! - `dsp`：10 段图形均衡器 + 总增益 + 声道平衡（包装成 rodio `Source`，实时生效）
//! - `player`：rodio 播放引擎与状态机（idle/playing/paused/ended）
//! - `settings`：音量 / 播放模式 / 音效持久化（`data_dir/music/settings.json`）
//!
//! 支持格式（symphonia）：mp3 / flac / wav / m4a(mp4/aac) / ogg(vorbis)。
//! 不含 ape / wma（symphonia 不支持，需外部 ffmpeg 转码）。

mod commands;
mod dsp;
mod library;
mod player;
mod queue;
mod settings;

pub use commands::*;
pub use player::{start_queue_watcher, MusicPlayerState};
