//! 音乐播放器模块：**本地曲库** + **MusicFree 在线音源插件**。
//!
//! - `library`：曲库（目录扫描 + lofty 标签/时长 + `data_dir/music/library.json` 持久化，
//!   **启动不自动重扫**，只有手动刷新才读盘）
//! - `dsp`：10 段图形均衡器 + 总增益 + 声道平衡（包装成 rodio `Source`，实时生效）
//! - `player`：rodio 播放引擎与状态机（idle/playing/paused/ended）
//! - `settings`：音量 / 播放模式 / 音效 / 下载目录持久化（`data_dir/music/settings.json`）
//! - `plugin_host`：MusicFree 插件宿主（常驻 Node 子进程桥，见该模块文档）
//!
//! 播放路径**只有一条**：本地文件。在线音源的产物是「远程 URL + 请求头」，
//! 因此先把远程内容落到磁盘（播放走缓存文件、下载走下载目录），再交给同一个播放引擎 ——
//! 这样队列 / 均衡器 / 切歌推进全部复用，不需要为在线源再造一套播放器。
//!
//! 支持格式（symphonia）：mp3 / flac / wav / m4a(mp4/aac) / ogg(vorbis)。
//! 不含 ape / wma（symphonia 不支持，需外部 ffmpeg 转码）。

mod commands;
mod dsp;
mod library;
mod player;
mod plugin_commands;
mod plugin_host;
mod plugin_playback;
mod plugin_registry;
mod queue;
mod settings;
mod transcode;

pub use commands::*;
pub use plugin_commands::*;
pub use plugin_playback::*;
pub use player::{start_queue_watcher, AdvanceOutcome, MusicPlayerState, spawn_online_resolve};
