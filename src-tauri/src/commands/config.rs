pub(crate) use super::foundation::atomic_write_file;
pub use super::foundation::{
    get_base_dir, get_cache_root, get_data_dir, get_node_projects_dir, get_sdk_dir,
    get_sdk_link_dir, get_sdk_versions_dir,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 串行化 config.json 的写入，避免并发命令交错写坏文件。
static CONFIG_SAVE_LOCK: Mutex<()> = Mutex::new(());

mod models;
pub use models::*;
mod persistence;
pub use persistence::*;
mod tray_settings;
pub use tray_settings::*;
mod backups;
pub use backups::*;
mod queries;
pub use queries::*;
mod rss_settings;
pub use rss_settings::*;
mod appearance;
pub use appearance::*;

#[cfg(test)]
mod tests;
