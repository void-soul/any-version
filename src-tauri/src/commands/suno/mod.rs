//! Suno 音频下载：解析用户主页（匿名，无需 Cookie）→ 提取歌曲 → 下载 m4a → ffmpeg 转 mp3。
//!
//! 技术要点（已实测）：
//! - Suno 主页是 Next.js SSR，歌曲数据内嵌在 HTML 的 RSC payload 里（JSON 转义形式）。
//! - 真实音频 = `media_urls[0].url`（CloudFront 的 `https://d2lwuy8qc234o3.cloudfront.net/1/clip/<id>.m4a`，
//!   是 m4a 容器 + opus 编码）；匿名时 `audio_url` 是 `.../api/forbidden` 占位，不可用。
//! - 转 mp3 复用 kira 内置 ffmpeg（`utils::bin_tool_path("ffmpeg")`），固定 320k。

use std::path::{Path, PathBuf};
use std::process::Command;

use base64::Engine;
use serde::{Deserialize, Serialize};
use tauri::Emitter;
use winreg::enums::*;
use winreg::RegKey;

use crate::commands::utils;

mod models;
pub use models::*;
mod storage;
pub use storage::*;
mod http;
pub use http::*;
mod profiles;
pub use profiles::*;
mod download;
pub use download::*;
mod parsing;
pub use parsing::*;
mod feed;
pub use feed::*;

#[cfg(test)]
mod tests;
