//! Everything 式文件搜索。
//!
//! - 按盘符建立 / 更新文件索引（独立 SQLite：`file_index.db`，与启动器数据分离）
//! - 全局快捷键呼出的独立无边框悬浮窗（`index.html?popup=file-search`）
//! - 索引扫描在后台线程执行，进度通过事件 `file-search-index-progress` 实时推送
//!
//! 更新策略：扫描先写入 `fs_entry_tmp`，完成后一次性替换该盘符的旧数据。
//! 因此「更新索引」期间旧索引仍然可搜；「重建索引」会先清空旧数据。
//!
//! 检索策略（v2）：
//! - `fs_entry` 存条目（rowid 表，`UNIQUE(drive, path)` 去重）；
//! - `fs_entry_fts` 是 FTS5 trigram 全文索引，让 `%中缀%` 也能走索引；
//! - 词长全部 ≥3 时走 FTS；否则退回「前缀走 `idx_fs_name` + 中缀补全」两段式查询。

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::types::Value;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde::Serialize;
use tauri::{Emitter, Manager};

use crate::commands::config::get_data_dir;

use super::gitignore::IgnoreStack;

const POPUP_LABEL: &str = "file-search-popup";
const POPUP_W: f64 = 780.0;
const POPUP_H: f64 = 560.0;
const PROGRESS_EVENT: &str = "file-search-index-progress";

/// 索引库连接（与启动器主库分开，避免全盘索引把 launcher.db 撑大）。
static INDEX_DB: Mutex<Option<Connection>> = Mutex::new(None);
/// 当前正在索引的盘符（None = 空闲）。同一时间只允许一个索引任务。
static BUILDING_DRIVE: Mutex<Option<String>> = Mutex::new(None);
/// 取消标志：扫描循环每轮检查一次。
static CANCEL_INDEX: AtomicBool = AtomicBool::new(false);

// ============================ 数据模型 ============================

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveInfo {
    /// "C:"
    pub letter: String,
    /// "C:\\"
    pub root: String,
    /// fixed / removable / network / cdrom / ramdisk / unknown
    pub drive_type: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    /// idle / building / ready
    pub status: String,
    pub item_count: i64,
    /// 最近一次索引完成时间（unix 秒）
    pub indexed_at: i64,
    pub elapsed_ms: i64,
    /// 建立索引时是否遵循 .gitignore
    pub respect_gitignore: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileHit {
    pub drive: String,
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size: i64,
    /// 修改时间（unix 秒）
    pub modified: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct IndexProgress {
    drive: String,
    scanned: usize,
    inserted: usize,
    current: String,
    elapsed_ms: i64,
    done: bool,
    canceled: bool,
    error: Option<String>,
}

enum ScanOutcome {
    Done { scanned: usize, count: i64 },
    Canceled { scanned: usize },
}

// ============================ 索引库 ============================

/// 索引库结构版本。v2：`fs_entry` 改为 rowid 表（与 FTS5 的 rowid 对齐）并新增 trigram 全文索引。
const SCHEMA_VERSION: i64 = 2;

// ============================ 盘符枚举 ============================

// ============================ 索引扫描 ============================

/// 任意层级都跳过的目录名（小写比较）：系统目录、版本库内部与常见依赖/构建缓存。
/// 这些内容没有检索价值，索引进来只会让索引膨胀、搜索变慢。
const SKIP_DIR_NAMES: &[&str] = &[
    "$recycle.bin",
    "system volume information",
    ".git",
    "node_modules",
    ".pnpm-store",
    ".yarn",
    ".npm",
    ".gradle",
    ".m2",
    ".nuget",
    ".next",
    ".nuxt",
    ".turbo",
    ".venv",
    // Python
    "__pycache__",
    ".tox",
    ".nox",
    // 前端框架与打包产物
    ".webpack",
    ".nyc_output",
    ".angular",
    ".svelte-kit",
    ".astro",
    ".docusaurus",
    // 语言工具链
    ".dart_tool",
    ".pub-cache",
    ".stack-work",
    // 编辑器 / 笔记本的会话与索引
    ".vs",
    ".vscode-server",
    ".vscode-test",
    ".ipynb_checkpoints",
    ".history",
    "ipch",
    "workspacestorage",
    "blob_storage",
    "service worker",
    "crashdumps",
];

/// 只在特定位置跳过的缓存目录（相对盘根的小写路径后缀，避免误伤同名目录）。
const SKIP_REL_SUFFIXES: &[&str] = &[
    "appdata/local/temp",
    "appdata/local/yarn/cache",
    "appdata/local/npm-cache",
    "appdata/roaming/npm-cache",
    "appdata/local/pnpm/store",
    "appdata/local/pip/cache",
    "appdata/local/nuget/v3-cache",
    "appdata/local/go-build",
    "appdata/local/microsoft/windows/inetcache",
    // IDE：索引 / 日志 / 临时文件集中在这里（配置在 Roaming，不受影响）
    "appdata/local/jetbrains",
    // 浏览器：User Data 下是各 profile 的缓存、代码缓存与 GPU 缓存
    "appdata/local/google/chrome/user data",
    "appdata/local/microsoft/edge/user data",
    // 其他应用缓存
    "appdata/local/unity/cache",
    "appdata/local/steam/htmlcache",
    "anaconda3/pkgs",
    "miniconda3/pkgs",
    "cargo/registry",
    "windows/temp",
    "windows/softwaredistribution",
    "windows/installer",
    "windows/winsxs",
    "windows/servicing",
];

type BatchRow = (String, String, String, i64, i64, i64);

// ============================ 命令：索引进度 / 管理 ============================

// ============================ 命令：搜索 ============================

/// 一条命中及其 rowid（rowid 用于两段式查询去重）。
type HitRow = (i64, FileHit);

/// 输入解析结果：盘符 / 目录前缀 / 关键词。
///
/// 支持 `e pro gld 1234` 这类写法：首词是盘符，随后连续的词只要能在索引里
/// 逐级命中目录，就当作目录前缀，剩下的词才是关键词。
#[derive(Debug, Default, PartialEq)]
struct QueryScope {
    drive: Option<String>,
    /// 已解析出的目录，如 `E:\pro\gld`（不含结尾反斜杠）
    path_prefix: Option<String>,
    tokens: Vec<String>,
}

// ============================ 悬浮窗 ============================

mod database;
pub use database::*;
mod drives;
pub use drives::*;
mod indexing;
pub use indexing::*;
mod query;
pub use query::*;
mod popup;
pub use popup::*;
#[cfg(test)]
mod tests;
