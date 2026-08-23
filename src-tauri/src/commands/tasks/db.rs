use std::sync::Mutex;

use crate::commands::config::get_data_dir;

/// 全局数据库连接（Mutex 保护，WAL 模式）。
/// 与 ai_usage.db 保持一致的连接管理风格。
static DB_CONN: Mutex<Option<rusqlite::Connection>> = Mutex::new(None);

/// 任务数据库文件路径：~/.any-version/tasks.db
fn db_path() -> std::path::PathBuf {
    get_data_dir().join("tasks.db")
}

/// 初始化数据库（幂等）。
pub fn init_db() -> Result<(), String> {
    let path = db_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let conn = rusqlite::Connection::open(&path).map_err(|e| format!("打开任务数据库失败: {}", e))?;

    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| format!("设置 WAL 模式失败: {}", e))?;
    conn.pragma_update(None, "foreign_keys", "ON").ok();

    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS tasks (
            id               TEXT    PRIMARY KEY,
            title            TEXT    NOT NULL,
            description      TEXT    NOT NULL DEFAULT '',
            scheduled_date   TEXT,
            priority         TEXT    NOT NULL DEFAULT 'medium',
            progress         INTEGER NOT NULL DEFAULT 0,
            sort_order       INTEGER NOT NULL DEFAULT 0,
            estimate_minutes INTEGER NOT NULL DEFAULT 0,
            tags             TEXT    NOT NULL DEFAULT '',
            archived         INTEGER NOT NULL DEFAULT 0,
            created_at       TEXT    NOT NULL,
            updated_at       TEXT    NOT NULL,
            completed_at     TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_tasks_date     ON tasks(scheduled_date);
        CREATE INDEX IF NOT EXISTS idx_tasks_archived ON tasks(archived);
        CREATE INDEX IF NOT EXISTS idx_tasks_progress ON tasks(progress);

        CREATE TABLE IF NOT EXISTS task_logs (
            id              TEXT    PRIMARY KEY,
            task_id         TEXT    NOT NULL,
            log_date        TEXT    NOT NULL,
            content         TEXT    NOT NULL DEFAULT '',
            progress_before INTEGER NOT NULL DEFAULT 0,
            progress_after  INTEGER NOT NULL DEFAULT 0,
            minutes_spent   INTEGER NOT NULL DEFAULT 0,
            created_at      TEXT    NOT NULL,
            FOREIGN KEY(task_id) REFERENCES tasks(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_logs_task ON task_logs(task_id);
        CREATE INDEX IF NOT EXISTS idx_logs_date ON task_logs(log_date);

        CREATE TABLE IF NOT EXISTS task_moves (
            id            TEXT    PRIMARY KEY,
            task_id       TEXT    NOT NULL,
            from_status   TEXT    NOT NULL DEFAULT '',
            from_progress INTEGER NOT NULL DEFAULT 0,
            from_date     TEXT,
            to_status     TEXT    NOT NULL DEFAULT '',
            to_progress   INTEGER NOT NULL DEFAULT 0,
            to_date       TEXT,
            reason        TEXT    NOT NULL DEFAULT '',
            moved_at      TEXT    NOT NULL,
            FOREIGN KEY(task_id) REFERENCES tasks(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_moves_task ON task_moves(task_id);
        CREATE INDEX IF NOT EXISTS idx_moves_at   ON task_moves(moved_at);
        "#,
    )
    .map_err(|e| format!("初始化任务表失败: {}", e))?;

    // 旧版本数据库没有 parent_id，使用幂等迁移补列。
    let has_parent_id: bool = conn
        .prepare("PRAGMA table_info(tasks)")
        .and_then(|mut stmt| {
            stmt.query_map([], |row| row.get::<_, String>(1))?.collect::<rusqlite::Result<Vec<_>>>()
        })
        .map(|columns| columns.iter().any(|column| column == "parent_id"))
        .unwrap_or(false);
    if !has_parent_id {
        conn.execute("ALTER TABLE tasks ADD COLUMN parent_id TEXT", [])
            .map_err(|e| format!("迁移任务父子关系字段失败: {}", e))?;
    }
    conn.execute("CREATE INDEX IF NOT EXISTS idx_tasks_parent ON tasks(parent_id)", [])
        .map_err(|e| format!("创建父任务索引失败: {}", e))?;

    let mut guard = DB_CONN.lock().map_err(|e| format!("DB锁错误: {}", e))?;
    *guard = Some(conn);
    Ok(())
}

/// 确保数据库已初始化。
fn ensure_db() -> Result<(), String> {
    {
        let guard = DB_CONN.lock().map_err(|e| format!("DB锁错误: {}", e))?;
        if guard.is_some() {
            return Ok(());
        }
    }
    init_db()
}

/// 以闭包方式借用连接，避免各处重复解锁样板代码。
pub fn with_conn<T, F>(f: F) -> Result<T, String>
where
    F: FnOnce(&mut rusqlite::Connection) -> Result<T, String>,
{
    ensure_db()?;
    let mut guard = DB_CONN.lock().map_err(|e| format!("DB锁错误: {}", e))?;
    let conn = guard.as_mut().ok_or("任务数据库未初始化")?;
    f(conn)
}

/// 当前时间戳（本地时间，ISO 秒级）。
pub fn now_ts() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

/// 今天日期 YYYY-MM-DD。
pub fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// 生成唯一 id。
pub fn new_id(prefix: &str) -> String {
    format!(
        "{}_{}_{}",
        prefix,
        chrono::Local::now().format("%Y%m%d%H%M%S%3f"),
        std::process::id()
    )
}
