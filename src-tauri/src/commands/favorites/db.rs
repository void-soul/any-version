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

/// 一条待落库的收藏条目（各源统一形态）。
#[derive(Debug, Clone)]
pub struct NewFavorite {
    pub source: String,
    /// 平台**原生 id**：GitHub 用仓库 id（不用 full_name，改名会变），B站用 bvid，知乎用内容 id
    pub external_id: String,
    pub url: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    /// 各源自定的附加字段（语言 / topics / 分区…），存 JSON 字符串
    pub extra_json: Option<String>,
}

/// 送进 AI 归类的最小信息（只带判断分类需要的字段）。
#[derive(Debug, Clone)]
pub struct ClassifyItem {
    pub id: i64,
    pub title: String,
    pub description: String,
    pub language: String,
    pub topics: Vec<String>,
}

/// upsert 的结果：新增 / 有变化已更新 / 完全没变（跨次导入去重的正常结局）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpsertOutcome {
    Added,
    Updated,
    Skipped,
}

fn now_str() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

/// 写入一条条目：已存在则按可变字段更新，完全没变则跳过。
///
/// 这是「重复导入不进重复记录」的唯一保证——依赖 `(source, external_id)` 唯一约束，
/// 而不是先查后写（避免并发下两边都查不到）。
pub fn upsert(conn: &Connection, item: &NewFavorite) -> Result<UpsertOutcome, String> {
    let inserted = conn
        .execute(
            "INSERT OR IGNORE INTO favorite \
             (source, external_id, url, title, subtitle, description, extra_json, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            rusqlite::params![
                item.source,
                item.external_id,
                item.url,
                item.title,
                item.subtitle,
                item.description,
                item.extra_json,
                now_str(),
            ],
        )
        .map_err(|e| format!("写入收藏条目失败: {}", e))?;
    if inserted > 0 {
        return Ok(UpsertOutcome::Added);
    }

    let current: (String, String, String, String, String) = conn
        .query_row(
            "SELECT url, title, COALESCE(subtitle, ''), COALESCE(description, ''), COALESCE(extra_json, '') \
             FROM favorite WHERE source = ?1 AND external_id = ?2",
            rusqlite::params![item.source, item.external_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .map_err(|e| format!("读取已存在收藏条目失败: {}", e))?;

    let incoming = (
        item.url.clone(),
        item.title.clone(),
        item.subtitle.clone().unwrap_or_default(),
        item.description.clone().unwrap_or_default(),
        item.extra_json.clone().unwrap_or_default(),
    );
    if current == incoming {
        return Ok(UpsertOutcome::Skipped);
    }

    conn.execute(
        "UPDATE favorite SET url = ?1, title = ?2, subtitle = ?3, description = ?4, \
         extra_json = ?5, updated_at = ?6 WHERE source = ?7 AND external_id = ?8",
        rusqlite::params![
            incoming.0,
            incoming.1,
            item.subtitle,
            item.description,
            item.extra_json,
            now_str(),
            item.source,
            item.external_id,
        ],
    )
    .map_err(|e| format!("更新收藏条目失败: {}", e))?;
    Ok(UpsertOutcome::Updated)
}

/// 记录一次导入的收尾状态（供 UI 展示"上次导入"与续跑判断）。
pub fn mark_imported(conn: &Connection, source: &str, total: usize) -> Result<(), String> {
    conn.execute(
        "INSERT INTO favorite_import_state (source, cursor, last_run_at, total) \
         VALUES (?1, NULL, ?2, ?3) \
         ON CONFLICT(source) DO UPDATE SET last_run_at = ?2, total = ?3",
        rusqlite::params![source, now_str(), total as i64],
    )
    .map_err(|e| format!("记录导入状态失败: {}", e))?;
    Ok(())
}

/// 条目总数（测试与概览用）。
pub fn count_all(conn: &Connection) -> Result<usize, String> {
    conn.query_row("SELECT COUNT(*) FROM favorite", [], |row| row.get(0))
        .map_err(|e| format!("统计收藏条目失败: {}", e))
}

#[cfg(test)]
mod tests {
    use super::{count_all, migrate, upsert, NewFavorite, UpsertOutcome};

    fn sample(external_id: &str, title: &str) -> NewFavorite {
        NewFavorite {
            source: "github".to_string(),
            external_id: external_id.to_string(),
            url: format!("https://github.com/{}", title),
            title: title.to_string(),
            subtitle: Some(title.to_string()),
            description: Some("desc".to_string()),
            extra_json: Some("{}".to_string()),
        }
    }

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

    /// 第二次导入同一批数据必须「一个都不新增」——这是用户对重复导入的核心要求。
    #[test]
    fn second_import_is_idempotent() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let item = sample("1", "o/r");
        assert_eq!(upsert(&conn, &item).unwrap(), UpsertOutcome::Added);
        assert_eq!(upsert(&conn, &item).unwrap(), UpsertOutcome::Skipped);
        assert_eq!(count_all(&conn).unwrap(), 1);
    }

    /// 内容变了（仓库改名 / 描述更新）要更新，而不是无视。
    #[test]
    fn changed_fields_are_updated_in_place() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        upsert(&conn, &sample("1", "o/r")).unwrap();
        let mut renamed = sample("1", "o/r");
        renamed.title = "o/renamed".to_string();
        assert_eq!(upsert(&conn, &renamed).unwrap(), UpsertOutcome::Updated);
        assert_eq!(count_all(&conn).unwrap(), 1, "更新不能变成新增一行");
        let title: String = conn
            .query_row("SELECT title FROM favorite WHERE external_id = '1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(title, "o/renamed");
    }

    /// GitHub 用仓库 id 做 external_id：仓库改名（full_name 变了）仍是同一条记录。
    #[test]
    fn renaming_full_name_does_not_duplicate() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let first = sample("42", "old/name");
        let mut renamed = sample("42", "old/name");
        renamed.title = "new/name".to_string();
        renamed.url = "https://github.com/new/name".to_string();
        upsert(&conn, &first).unwrap();
        upsert(&conn, &renamed).unwrap();
        assert_eq!(count_all(&conn).unwrap(), 1, "改名不能导进重复条目");
    }

    /// 不同平台的 id 撞车不算同一条（外部 id 只在同 source 内唯一）。
    #[test]
    fn same_external_id_across_sources_stays_separate() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let mut zhihu = sample("42", "x/y");
        zhihu.source = "zhihu".to_string();
        upsert(&conn, &sample("42", "x/y")).unwrap();
        upsert(&conn, &zhihu).unwrap();
        assert_eq!(count_all(&conn).unwrap(), 2);
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
