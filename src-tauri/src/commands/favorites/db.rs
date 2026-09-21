//! 收藏模块本地库：`get_data_dir()/favorites.db`。
//!
//! 所有函数都接受 `&Connection`，便于用内存库做单测；命令层通过 [`with_conn`] 拿全局连接。

use std::path::PathBuf;
use std::sync::Mutex;

use rusqlite::Connection;

use crate::commands::config::get_data_dir;

/// 全局连接（Mutex 保护）。WAL 模式，读写不互相阻塞。
static DB_CONN: Mutex<Option<Connection>> = Mutex::new(None);

fn db_path() -> PathBuf {
    get_data_dir().join("favorites.db")
}

/// 建表（幂等）。三张表：条目、标签（多标签）、导入进度。
pub fn migrate(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS favorite (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            source       TEXT    NOT NULL,
            external_id  TEXT    NOT NULL,
            url          TEXT    NOT NULL,
            title        TEXT    NOT NULL,
            subtitle     TEXT,
            description  TEXT,
            extra_json   TEXT,
            status       TEXT    NOT NULL DEFAULT 'ok',
            checked_at   TEXT,
            ai_locked    INTEGER NOT NULL DEFAULT 0,
            ai_model     TEXT,
            ai_at        TEXT,
            created_at   TEXT    NOT NULL,
            updated_at   TEXT    NOT NULL,
            UNIQUE(source, external_id)
        );
        CREATE INDEX IF NOT EXISTS idx_favorite_source       ON favorite(source);
        CREATE INDEX IF NOT EXISTS idx_favorite_status       ON favorite(status);
        CREATE INDEX IF NOT EXISTS idx_favorite_ai_locked    ON favorite(ai_locked);

        -- 多标签：一个条目可以同时属于多个分类，不设「主分类」
        CREATE TABLE IF NOT EXISTS favorite_tag (
            favorite_id INTEGER NOT NULL,
            tag         TEXT    NOT NULL,
            PRIMARY KEY (favorite_id, tag)
        );
        CREATE INDEX IF NOT EXISTS idx_favorite_tag_tag ON favorite_tag(tag);

        CREATE TABLE IF NOT EXISTS favorite_import_state (
            source      TEXT PRIMARY KEY,
            cursor      TEXT,
            last_run_at TEXT,
            total       INTEGER NOT NULL DEFAULT 0
        );
        "#,
    )
    .map_err(|e| format!("初始化收藏库失败: {}", e))
}

/// 打开/初始化全局连接（不持有锁；由调用方在锁内调用）。
fn init_connection() -> Result<Connection, String> {
    let path = db_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建数据目录失败: {}", e))?;
    }
    let conn = Connection::open(&path).map_err(|e| format!("打开收藏库失败: {}", e))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| format!("设置 WAL 模式失败: {}", e))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| format!("设置收藏库超时失败: {}", e))?;
    migrate(&conn)?;
    Ok(conn)
}

/// 在全局连接上执行一段逻辑（首次调用自动初始化）。
pub fn with_conn<T>(f: impl FnOnce(&Connection) -> Result<T, String>) -> Result<T, String> {
    let mut guard = DB_CONN.lock().map_err(|e| format!("DB锁错误: {}", e))?;
    if guard.is_none() {
        *guard = Some(init_connection()?);
    }
    let conn = guard.as_ref().expect("连接已初始化");
    f(conn)
}

#[cfg(test)]
mod tests {
    use super::migrate;

    #[test]
    fn schema_creates_all_three_tables() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' \
                 AND name IN ('favorite','favorite_tag','favorite_import_state')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 3);
    }

    /// 建表必须幂等：命令层每次调用都会走到这里。
    #[test]
    fn migrate_is_idempotent() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
    }

    /// 去重靠的是 (source, external_id) 唯一约束，缺了它整个模块的前提就没了。
    #[test]
    fn duplicate_source_and_external_id_is_rejected() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let insert = |external_id: &str| -> rusqlite::Result<usize> {
            conn.execute(
                "INSERT INTO favorite (source, external_id, url, title, created_at, updated_at) \
                 VALUES ('github', ?1, 'https://x', 't', 'now', 'now')",
                [external_id],
            )
        };
        assert!(insert("42").is_ok());
        let err = insert("42").unwrap_err();
        assert!(
            matches!(err, rusqlite::Error::SqliteFailure(_, _)),
            "重复 (source, external_id) 必须被唯一约束拦下，实际: {:?}",
            err
        );
        // 不同 source 的同 id 不算冲突
        conn.execute(
            "INSERT INTO favorite (source, external_id, url, title, created_at, updated_at) \
             VALUES ('zhihu', '42', 'https://x', 't', 'now', 'now')",
            [],
        )
        .unwrap();
    }

    /// 多标签：同一条目可挂多个标签，但完全相同的 (条目, 标签) 只能有一条。
    #[test]
    fn tags_allow_multiple_labels_per_item() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn.execute(
            "INSERT INTO favorite (id, source, external_id, url, title, created_at, updated_at) \
             VALUES (1, 'github', '42', 'https://x', 't', 'now', 'now')",
            [],
        )
        .unwrap();
        let tag = |t: &str| {
            conn.execute("INSERT INTO favorite_tag (favorite_id, tag) VALUES (1, ?1)", [t])
        };
        assert!(tag("LLM").is_ok());
        assert!(tag("运维").is_ok(), "多标签必须允许同一条目挂第二个分类");
        assert!(tag("LLM").is_err(), "同一标签不能重复挂");
    }
}
