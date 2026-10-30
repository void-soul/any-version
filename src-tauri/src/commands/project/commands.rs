//! 项目管理模块 — Tauri 命令层。
//!
//! 暴露给前端的所有 project_* 命令，内部调用 scanner 模块执行实际逻辑。

use super::scanner;
use super::types::{ManagePreview, ProjectDetail, ProjectStatus};
use crate::commands::utils::resolve_detected_path;
use tauri::Emitter;

mod management;
pub use management::*;
mod runtime_commands;
pub use runtime_commands::*;
mod cache_queries;
pub use cache_queries::*;
mod data_commands;
pub use data_commands::*;
mod git;
pub use git::*;

#[cfg(test)]
#[path = "commands/tests.rs"]
mod tests;
