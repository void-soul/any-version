use super::*;

pub(super) fn db_path() -> std::path::PathBuf {
    get_data_dir().join("file_index.db")
}

pub(super) fn table_exists(conn: &Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
        params![table],
        |_| Ok(()),
    )
    .is_ok()
}

pub(super) fn column_exists(conn: &Connection, table: &str, column: &str) -> bool {
    let sql = format!("PRAGMA table_info({})", table);
    let Ok(mut stmt) = conn.prepare(&sql) else {
        return false;
    };
    let names: Vec<String> = match stmt.query_map([], |row| row.get::<_, String>(1)) {
        Ok(rows) => rows.flatten().collect(),
        Err(_) => return false,
    };
    names.iter().any(|name| name == column)
}

/// 建表 / 升级结构。
///
/// v1 的 `fs_entry` 是 `WITHOUT ROWID`（无 `id` 列），无法与 FTS5 的 rowid 对齐，
/// 直接丢弃旧索引并把盘符状态重置为 idle，用户重新建立索引即可（索引本身可重建）。
pub(super) fn init_schema(conn: &Connection) -> Result<(), String> {
    let legacy = table_exists(conn, "fs_entry") && !column_exists(conn, "fs_entry", "id");
    if legacy {
        conn.execute_batch(
            "DROP TABLE IF EXISTS fs_entry_fts;
             DROP TABLE IF EXISTS fs_entry_tmp;
             DROP TABLE IF EXISTS fs_entry;",
        )
        .map_err(|e| format!("清理旧版索引表失败: {}", e))?;
    }
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS fs_entry (
            id         INTEGER PRIMARY KEY,
            drive      TEXT    NOT NULL,
            path       TEXT    NOT NULL,
            name       TEXT    NOT NULL,
            name_lower TEXT    NOT NULL,
            is_dir     INTEGER NOT NULL DEFAULT 0,
            size       INTEGER NOT NULL DEFAULT 0,
            modified   INTEGER NOT NULL DEFAULT 0
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_fs_path ON fs_entry(drive, path);
        CREATE INDEX IF NOT EXISTS idx_fs_name ON fs_entry(name_lower);

        CREATE TABLE IF NOT EXISTS fs_entry_tmp (
            drive      TEXT    NOT NULL,
            path       TEXT    NOT NULL,
            name       TEXT    NOT NULL,
            name_lower TEXT    NOT NULL,
            is_dir     INTEGER NOT NULL DEFAULT 0,
            size       INTEGER NOT NULL DEFAULT 0,
            modified   INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (drive, path)
        ) WITHOUT ROWID;

        -- 文件名全文索引。trigram 分词支持任意位置的中缀匹配（搜索词需 ≥3 字符）。
        CREATE VIRTUAL TABLE IF NOT EXISTS fs_entry_fts USING fts5(
            name,
            tokenize = 'trigram'
        );

        CREATE TABLE IF NOT EXISTS fs_meta (
            drive TEXT PRIMARY KEY,
            status TEXT    NOT NULL DEFAULT 'idle',
            item_count INTEGER NOT NULL DEFAULT 0,
            indexed_at INTEGER NOT NULL DEFAULT 0,
            elapsed_ms INTEGER NOT NULL DEFAULT 0,
            respect_gitignore INTEGER NOT NULL DEFAULT 0
        );
        "#,
    )
    .map_err(|e| format!("初始化索引表失败: {}", e))?;
    // v2 之前的库缺这一列；幂等补齐，不强制重建索引。
    if !column_exists(conn, "fs_meta", "respect_gitignore") {
        conn.execute(
            "ALTER TABLE fs_meta ADD COLUMN respect_gitignore INTEGER NOT NULL DEFAULT 0",
            [],
        )
        .map_err(|e| format!("补充索引状态字段失败: {}", e))?;
    }
    if legacy {
        conn.execute("UPDATE fs_meta SET status = 'idle', item_count = 0", [])
            .map_err(|e| format!("重置索引状态失败: {}", e))?;
    }
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)
        .map_err(|e| format!("写入索引库版本失败: {}", e))?;
    Ok(())
}

pub(super) fn build_connection() -> Result<Connection, String> {
    let path = db_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建索引目录失败: {}", e))?;
    }
    let conn = Connection::open(&path).map_err(|e| format!("打开索引数据库失败: {}", e))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| format!("设置索引库 WAL 模式失败: {}", e))?;
    conn.pragma_update(None, "synchronous", "NORMAL").ok();
    // 扫描线程与查询线程各持一个连接；写事务（尤其收尾的全文索引填充）期间
    // 给读写留出等待窗口，避免直接抛 database is locked。
    conn.pragma_update(None, "busy_timeout", 5000i64).ok();
    // 排序 / 临时结构放内存，避免大结果集排序落盘。
    conn.pragma_update(None, "temp_store", "MEMORY").ok();
    // 64 MiB 页缓存 + 256 MiB 只读映射：千万级索引的随机页读取尽量不触发系统调用。
    conn.pragma_update(None, "cache_size", -65536i64).ok();
    conn.pragma_update(None, "mmap_size", 268435456i64).ok();
    init_schema(&conn)?;
    Ok(conn)
}

pub(super) fn with_conn<T, F>(f: F) -> Result<T, String>
where
    F: FnOnce(&mut Connection) -> Result<T, String>,
{
    let mut guard = INDEX_DB
        .lock()
        .map_err(|e| format!("索引库锁错误: {}", e))?;
    if guard.is_none() {
        *guard = Some(build_connection()?);
    }
    match guard.as_mut() {
        Some(conn) => f(conn),
        None => Err("索引数据库未连接".to_string()),
    }
}

pub(super) fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub(super) fn set_meta(
    conn: &Connection,
    drive: &str,
    status: &str,
    item_count: i64,
    indexed_at: i64,
    elapsed_ms: i64,
    respect_gitignore: bool,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO fs_meta (drive, status, item_count, indexed_at, elapsed_ms, respect_gitignore)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(drive) DO UPDATE SET
            status = excluded.status,
            item_count = excluded.item_count,
            indexed_at = excluded.indexed_at,
            elapsed_ms = excluded.elapsed_ms,
            respect_gitignore = excluded.respect_gitignore",
        params![
            drive,
            status,
            item_count,
            indexed_at,
            elapsed_ms,
            respect_gitignore as i64
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub(super) fn set_status(drive: &str, status: &str) {
    let _ = with_conn(|conn| {
        conn.execute(
            "INSERT INTO fs_meta (drive, status) VALUES (?1, ?2)
             ON CONFLICT(drive) DO UPDATE SET status = excluded.status",
            params![drive, status],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    });
}

/// 取消/失败后恢复状态：按实际剩余数据判定 ready / idle，避免「重建索引被取消」
/// 之后仍显示旧的条目数（此时旧数据已被清空）。
pub(super) fn restore_status(drive: &str) {
    let _ = with_conn(|conn| {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM fs_entry WHERE drive = ?1",
                params![drive],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let status = if count > 0 { "ready" } else { "idle" };
        conn.execute(
            "INSERT INTO fs_meta (drive, status, item_count) VALUES (?1, ?2, ?3)
             ON CONFLICT(drive) DO UPDATE SET
                status = excluded.status,
                item_count = excluded.item_count",
            params![drive, status, count],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    });
}

/// 清理扫描中途留下的临时行（失败/异常退出时调用）。
pub(super) fn cleanup_tmp(drive: &str) {
    let _ = with_conn(|conn| {
        conn.execute("DELETE FROM fs_entry_tmp WHERE drive = ?1", params![drive])
            .map_err(|e| e.to_string())?;
        Ok(())
    });
}
