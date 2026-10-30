use super::models::{
    Classification, ClassificationData, DeleteClassificationResult, Item, ItemData, LauncherSetting,
};
use crate::commands::config::get_data_dir;
use chrono::Local;
use rusqlite::{params, Connection};
use std::sync::Mutex;

static DB_CONN: Mutex<Option<Connection>> = Mutex::new(None);

pub(crate) fn migrate_item_usage_columns(conn: &Connection) -> Result<(), String> {
    // 兼容早期已经创建的启动器数据库：为旧表补齐统计列。
    let _ = conn.execute(
        "ALTER TABLE launcher_item ADD COLUMN open_number INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE launcher_item ADD COLUMN last_open INTEGER NOT NULL DEFAULT 0",
        [],
    );
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_item_open ON launcher_item(open_number)",
        [],
    )
    .map_err(|e| format!("初始化启动统计索引失败: {}", e))?;
    Ok(())
}

pub(crate) fn increment_item_open_count_on(conn: &Connection, id: i64) -> Result<(), String> {
    let now_ts = Local::now().timestamp_millis();
    conn.execute(
        "UPDATE launcher_item SET open_number = open_number + 1, last_open = ?1 WHERE id = ?2",
        params![now_ts, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn db_path() -> std::path::PathBuf {
    get_data_dir().join("launcher.db")
}

fn build_connection() -> Result<Connection, String> {
    let path = db_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let conn = Connection::open(&path).map_err(|e| format!("打开启动器数据库失败: {}", e))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| format!("设置 WAL 模式失败: {}", e))?;
    conn.pragma_update(None, "foreign_keys", "ON").ok();

    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS launcher_classification (
            id                    INTEGER PRIMARY KEY AUTOINCREMENT,
            parent_id             INTEGER,
            name                  TEXT    NOT NULL,
            classification_type   INTEGER NOT NULL DEFAULT 0,
            data                  TEXT    NOT NULL DEFAULT '{}',
            shortcut_key          TEXT,
            global_shortcut_key   INTEGER NOT NULL DEFAULT 0,
            sort_order            INTEGER NOT NULL DEFAULT 0,
            created_at            TEXT    NOT NULL,
            updated_at            TEXT    NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_cls_parent ON launcher_classification(parent_id);
        CREATE INDEX IF NOT EXISTS idx_cls_order  ON launcher_classification(sort_order);

        CREATE TABLE IF NOT EXISTS launcher_item (
            id                    INTEGER PRIMARY KEY AUTOINCREMENT,
            classification_id     INTEGER NOT NULL,
            name                  TEXT    NOT NULL,
            item_type             INTEGER NOT NULL DEFAULT 0,
            data                  TEXT    NOT NULL DEFAULT '{}',
            shortcut_key          TEXT,
            global_shortcut_key   INTEGER NOT NULL DEFAULT 0,
            sort_order            INTEGER NOT NULL DEFAULT 0,
            open_number           INTEGER NOT NULL DEFAULT 0,
            last_open             INTEGER NOT NULL DEFAULT 0,
            created_at            TEXT    NOT NULL,
            updated_at            TEXT    NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_item_cls   ON launcher_item(classification_id);
        CREATE INDEX IF NOT EXISTS idx_item_order ON launcher_item(sort_order);

        CREATE TABLE IF NOT EXISTS launcher_setting (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        "#,
    )
    .map_err(|e| format!("初始化启动器表失败: {}", e))?;

    migrate_item_usage_columns(&conn)?;

    // 检查是否需要播种初始默认数据
    let count: i64 = conn
        .query_row("SELECT COUNT(1) FROM launcher_classification", [], |row| {
            row.get(0)
        })
        .unwrap_or(0);

    if count == 0 {
        seed_default_data(&conn)?;
    }

    Ok(conn)
}

pub fn init_db() -> Result<(), String> {
    let conn = build_connection()?;
    *DB_CONN.lock().map_err(|e| format!("DB锁错误: {}", e))? = Some(conn);
    Ok(())
}

pub(super) fn with_conn<T, F>(f: F) -> Result<T, String>
where
    F: FnOnce(&mut Connection) -> Result<T, String>,
{
    // 检查 + 初始化 + 使用放在同一锁临界区，避免并发重复初始化覆盖连接。
    let mut guard = DB_CONN.lock().map_err(|e| format!("DB锁错误: {}", e))?;
    if guard.is_none() {
        *guard = Some(build_connection()?);
    }
    match guard.as_mut() {
        Some(conn) => f(conn),
        None => Err("启动器数据库未连接".to_string()),
    }
}

/// 默认数据播种
fn seed_default_data(conn: &Connection) -> Result<(), String> {
    let now = Local::now().to_rfc3339();

    // 默认分类 1: 常用推荐
    conn.execute(
        "INSERT INTO launcher_classification (id, parent_id, name, classification_type, data, sort_order, created_at, updated_at) VALUES (1, NULL, '常用推荐', 0, '{\"icon\":\"🚀\"}', 1, ?1, ?1)",
        params![now],
    ).ok();

    // 默认分类 2: 系统工具
    conn.execute(
        "INSERT INTO launcher_classification (id, parent_id, name, classification_type, data, sort_order, created_at, updated_at) VALUES (2, NULL, '系统工具', 0, '{\"icon\":\"⚙️\"}', 2, ?1, ?1)",
        params![now],
    ).ok();

    // 默认分类 3: 聚合统计 (聚合分类)
    conn.execute(
        "INSERT INTO launcher_classification (id, parent_id, name, classification_type, data, sort_order, created_at, updated_at) VALUES (3, NULL, '最常打开', 2, '{\"icon\":\"🔥\",\"aggregateItemCount\":20,\"aggregateSort\":\"openNumber\"}', 3, ?1, ?1)",
        params![now],
    ).ok();

    // 默认系统工具预置条目
    let sys_items = vec![
        ("任务管理器", "taskmgr.exe", "", "⚡", 2),
        ("控制面板", "control.exe", "", "🎛️", 2),
        ("系统属性与环境变量", "sysdm.cpl", "", "🌿", 2),
        ("设备管理器", "devmgmt.msc", "", "🖥️", 2),
        ("计算机管理", "compmgmt.msc", "", "🏢", 2),
        ("服务列表", "services.msc", "", "🛠️", 2),
        ("注册表编辑器", "regedit.exe", "", "📝", 2),
        ("锁定计算机", "static:LockWorkstation", "", "🔒", 2),
        ("清空回收站", "static:EmptyRecycleBin", "", "🗑️", 2),
        ("关闭显示器", "static:TurnOffMonitor", "", "🌙", 2),
        ("重启资源管理器", "static:RestartExplorer", "", "🔄", 2),
        ("命令提示符 (CMD)", "cmd.exe", "", "💻", 1),
        ("Windows PowerShell", "powershell.exe", "", "🔷", 1),
        ("Windows 终端", "wt.exe", "", "⬛", 1),
    ];

    for (idx, (name, target, params_str, icon_emoji, cls_id)) in sys_items.into_iter().enumerate() {
        let item_data = serde_json::json!({
            "target": target,
            "params": params_str,
            "htmlIcon": icon_emoji,
            "runAsAdmin": false,
            "openNumber": 0,
            "lastOpen": 0,
        });
        conn.execute(
            "INSERT INTO launcher_item (classification_id, name, item_type, data, sort_order, created_at, updated_at) VALUES (?1, ?2, 3, ?3, ?4, ?5, ?5)",
            params![cls_id, name, item_data.to_string(), (idx + 1) as i32, now],
        ).ok();
    }

    // 默认设置
    let default_setting = LauncherSetting::default();
    let val_json = serde_json::to_string(&default_setting).unwrap_or_default();
    conn.execute(
        "INSERT OR REPLACE INTO launcher_setting (key, value) VALUES ('global', ?1)",
        params![val_json],
    )
    .ok();

    Ok(())
}

mod classifications;
pub use classifications::*;
mod items;
pub use items::*;
mod backup;
pub use backup::*;
#[cfg(test)]
mod tests;
