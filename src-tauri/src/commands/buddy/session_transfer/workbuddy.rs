//! Buddy 模块 WorkBuddy 会话跨账号合并入口。
//!
//! 移植自 cockpit-tools `modules/workbuddy_session_transfer.rs`：
//! - WorkBuddy 5.x 的会话主存储是 `{config_dir}/workbuddy.db`（按 `user_id` 区分），
//!   因此**无论是否存在旧版扩展会话目录**，都必须执行数据库层 user_id 重映射
//!   （这才是 5.x 真正生效的会话合并）。
//! - 旧版 per-uid 目录（WorkBuddyExtension / CodeBuddyExtension）的 history 合并
//!   复用 codebuddy.rs 的共享引擎；旧版 `codebuddy-sessions.vscdb` 重映射同样复用。

use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

use super::super::models::{BuddyAccount, BuddyPlatform};
use super::super::session_sync::{self, PendingConflict, SessionSyncStatus, SyncTracker};
use super::super::store;
use super::codebuddy;
use super::SessionTransferReport;
use super::TransferProgress;
use crate::commands::buddy::emit_switch_progress;

mod context;
pub use context::*;
mod transfer;
use transfer::*;
mod roots;
pub(crate) use roots::*;
mod database;
use database::*;

#[cfg(test)]
#[path = "workbuddy/tests.rs"]
mod tests;
