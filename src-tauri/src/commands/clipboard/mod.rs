//! 剪贴板管理器模块（复刻 CopyQ 核心能力）
//!
//! - 后台监控系统剪贴板（文本/图片），自动保存历史
//! - 历史列表：搜索 / 类型过滤 / 置顶 / 删除 / 清空
//! - 复制回剪贴板 / 一键粘贴到之前的前台窗口（SendInput Ctrl+V）
//! - 忽略规则（按来源程序）
//!
//! 数据存储：`{data_dir}/clipboard/clipboard.db`（SQLite）
//! 图片：`{data_dir}/clipboard/images/`（PNG + 缩略图）

mod commands;

// windows-sys 0.59 的 DataExchange 模块不含剪贴板格式常量，这里自行定义。
// 标准值：CF_UNICODETEXT=13, CF_DIB=8, CF_DIBV5=17
pub(crate) const CF_UNICODETEXT: u32 = 13;
pub(crate) const CF_DIB: u32 = 8;
pub(crate) const CF_DIBV5: u32 = 17;
mod db;
mod formats;
mod images;
mod monitor;
mod paste;

pub use commands::*;

// monitor 内部函数对 commands 层可见
pub(crate) use monitor::remember_previous_window as monitor_remember_window;
pub(crate) use monitor::take_previous_window as monitor_take_previous_window;

use std::sync::Mutex;
use tauri::Manager;

use crate::commands::config::get_data_dir;

/// 剪贴板历史项
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardItem {
    pub id: i64,
    pub kind: String, // "text" | "image"
    pub content: Option<String>,
    pub image_path: Option<String>,
    pub thumb_path: Option<String>,
    pub width: i64,
    pub height: i64,
    pub source_app: String,
    pub pinned: bool,
    pub created_at: i64,
    /// 复制时剪贴板中包含的格式（MIME 风格名称，CopyQ 式）
    pub formats: Vec<String>,
}

/// 剪贴板模块状态
#[derive(Clone)]
pub struct ClipboardState {
    pub db: std::sync::Arc<Mutex<rusqlite::Connection>>,
    pub data_dir: std::path::PathBuf,
    /// 剪贴板「系统写锁」：复制/粘贴（写系统剪贴板）时持有；
    /// monitor 读取系统剪贴板前先 try_lock，拿不到则跳过本次更新。
    /// 避免同进程内「写」与「读」两个线程同时 OpenClipboard 互相抢锁
    /// （管理员高完整性会话下更易触发，表现为复制静默失败）。
    pub clipboard_write_lock: std::sync::Arc<Mutex<()>>,
}

/// 剪贴板配置
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardSettings {
    /// 是否启用后台监控
    pub enabled: bool,
    /// 历史保留上限
    pub max_items: i64,
    /// 是否保存图片
    pub store_images: bool,
    /// 是否忽略纯空白文本
    pub ignore_blank: bool,
    /// 是否忽略长度过短（<=2）的文本
    pub ignore_short: bool,
}

impl Default for ClipboardSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            max_items: 1000,
            store_images: true,
            ignore_blank: true,
            ignore_short: false,
        }
    }
}

/// 剪贴板数据目录
pub fn clipboard_dir() -> std::path::PathBuf {
    get_data_dir().join("clipboard")
}

/// 剪贴板图片目录
pub fn images_dir() -> std::path::PathBuf {
    clipboard_dir().join("images")
}

/// 初始化剪贴板模块（应用启动时调用一次）
pub fn init_clipboard_state(app: &tauri::AppHandle) -> Result<(), String> {
    let dir = clipboard_dir();
    let img_dir = images_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建剪贴板目录失败: {}", e))?;
    std::fs::create_dir_all(&img_dir).map_err(|e| format!("创建剪贴板图片目录失败: {}", e))?;

    let db_path = dir.join("clipboard.db");
    let conn = db::open_db(&db_path).map_err(|e| e.to_string())?;
    db::init_schema(&conn).map_err(|e| e.to_string())?;
    db::cleanup_orphan_images(&conn, &img_dir).map_err(|e| e.to_string())?;

    let state = ClipboardState {
        db: std::sync::Arc::new(Mutex::new(conn)),
        data_dir: dir,
        clipboard_write_lock: std::sync::Arc::new(Mutex::new(())),
    };
    app.manage(state);

    // 启动后台监控线程
    monitor::spawn_monitor(app.clone());

    Ok(())
}
