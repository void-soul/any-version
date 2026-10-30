//! Buddy 模块会话合并共享引擎 + CodeBuddy CN 入口。
//!
//! 移植自 cockpit-tools `modules/codebuddy_session_transfer.rs`：
//! - `sync_history_between_accounts`：按 `<uid>/<IDE>/<uid>/history/<workspace>` 布局逐工作区合并
//!   会话目录与辅助目录（check-point / file-tree / plan-task / genie-cache / connectors），
//!   并合并 `index.json`（按 conversationId 去重、按 lastMessageAt 取新、保留/更新 current）。
//! - `remap_session_vscdb_user_id`：`codebuddy-sessions.vscdb` 中 `session:%` 记录的 userId 重映射。
//! WorkBuddy（`workbuddy.rs`）同样复用这两个函数（与 cockpit-tools 的依赖关系一致）。

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Component, Path};

use serde_json::Value;

use super::super::models::{BuddyAccount, BuddyPlatform};
use super::super::session_sync::{PendingConflict, SessionSyncStatus, SyncTracker};
use super::super::store;
use super::SessionTransferReport;
use super::TransferProgress;
use crate::commands::buddy::emit_switch_progress;

mod context;
pub use context::*;
mod shared_storage;
use shared_storage::*;
mod transfer;
pub(crate) use transfer::*;
mod conflicts;
pub(crate) use conflicts::*;
mod preview;
pub use preview::*;
mod synchronization;
pub(crate) use synchronization::*;
mod workspaces;
pub(crate) use workspaces::*;
mod merge;
use merge::*;
mod index;
pub(crate) use index::*;
mod movement;
pub(crate) use movement::*;
mod filesystem;
use filesystem::*;
mod database;
pub(in crate::commands::buddy::session_transfer) use database::*;

#[cfg(test)]
mod tests;
