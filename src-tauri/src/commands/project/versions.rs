//! 项目版本管理模块 -- 远程版本列表、安装、卸载、切换。
//!
//! 从已删除的 sdk.rs 迁移而来，适配新的项目托管架构。
//! 使用 project_id（原 sdk_name）标识项目，通过 load_config() 获取 versions_dir/links_dir，
//! 通过 junction 实现版本切换。
//!
//! 注意：本地版本注册/导入功能已移除。托管时自动将旧版数据（版本、来源、路径、备份环境变量）
//! 保存到 backup 目录供"旧版数据"选项卡展示。

use crate::commands::config::{get_data_dir, load_config};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};

mod state;
pub use state::*;
mod cache;
pub use cache::*;
mod remote;
pub use remote::*;
mod installation;
pub use installation::*;
mod msix;
pub use msix::*;
mod lifecycle;
pub use lifecycle::*;
mod archives;
pub use archives::*;
mod downloads;
pub use downloads::*;

#[cfg(test)]
#[path = "versions/tests.rs"]
mod tests;
