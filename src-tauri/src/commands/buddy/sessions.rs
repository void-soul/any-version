//! Buddy 会话管理：列出本机 WorkBuddy / CodeBuddy CN 的 AI 会话。
//!
//! 复刻自 cockpit-tools `codebuddy_session.rs`：
//! - WorkBuddy：会话存在 `~/.workbuddy/workbuddy.db`（单库，按 user_id 分区）
//! - CodeBuddy CN：会话存在 `%APPDATA%/CodeBuddy CN/codebuddy-sessions.vscdb`
//!   （ItemTable 中 key 以 `session:` 开头的 JSON）

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::models::BuddyPlatform;

mod models;
pub use models::*;
mod readers;
pub(crate) use readers::*;
mod listing;
pub use listing::*;
mod deletion;
pub use deletion::*;
mod movement;
pub use movement::*;
mod workbuddy_deletion;
use workbuddy_deletion::*;
mod extension_files;
use extension_files::*;
mod forking;
pub use forking::*;
mod fork_files;
use fork_files::*;

#[cfg(test)]
#[path = "sessions/tests.rs"]
mod tests;
