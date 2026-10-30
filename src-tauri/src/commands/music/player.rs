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

mod models;
pub use models::*;
mod diagnostics;
use diagnostics::*;
mod output;
pub use output::*;
mod queue;
pub use queue::*;
mod playback;
pub use playback::*;
mod snapshot;
pub use snapshot::*;
mod tasks;
pub use tasks::*;

#[cfg(test)]
#[path = "player/decode_probe_tests.rs"]
mod decode_probe_tests;

#[cfg(test)]
#[path = "player/tests.rs"]
mod tests;
