//! 通用「Node 项目管理器」
//!
//! 以配置驱动的方式管理类似 deepseek-harness（dsh）的 Node 项目：
//! 安装、升级、启动、停止、打开主页面。新增项目 = 在 node-projects/
//! 目录添加一个 JSON 定义，零代码改动。
//!
//! 两种安装方式（由配置决定）：
//! - git 模式（默认）：git clone → 包管理器 install → build
//! - npx 模式（配置 npxPackage）：`npm install --prefix <托管目录> <包名>` 一步到位，
//!   免 git clone / 依赖安装 / 编译；启动用 `npx --prefix <托管目录> <bin> <args...>`。
//!
//! 配置目录查找逻辑复用 ai-tools 的候选目录扫描机制（资源目录 / exe
//! 同级 / cwd / ~/.any-version）。

use crate::commands::config::get_base_dir;
use crate::commands::utils::find_in_path;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::Emitter;

mod models;
pub use models::*;
mod registry;
pub use registry::*;
mod versions;
use versions::*;
mod process_io;
pub use process_io::*;
mod package_tools;
pub(crate) use package_tools::*;
mod python_install;
use python_install::*;
mod build_tools;
use build_tools::*;
mod docker;
pub use docker::*;
mod install_commands;
pub use install_commands::*;
mod process_lifecycle;
pub use process_lifecycle::*;
mod browser;
pub use browser::*;
mod status_commands;
pub use status_commands::*;

#[cfg(test)]
#[path = "node_manager/tests.rs"]
mod tests;
