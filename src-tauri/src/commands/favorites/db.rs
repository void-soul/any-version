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

        -- 各平台 Cookie（B站 SESSDATA / 知乎 z_c0）。**只存不打印**，
        -- 日志与错误信息里一律不出现它的内容。
        CREATE TABLE IF NOT EXISTS favorite_credential (
            source      TEXT PRIMARY KEY,
            cookie      TEXT NOT NULL,
            updated_at  TEXT NOT NULL
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
    /// 导入时就能确定失效的（B站内容被 UP 主删除）直接落这个状态，省一次探测
    pub initial_status: Option<String>,
}

/// 送进 AI 归类的最小信息（只带判断分类需要的字段）。
#[derive(Debug, Clone)]
pub struct ClassifyItem {
    pub id: i64,
    pub title: String,
    pub description: String,
    pub language: String,
    pub topics: Vec<String>,
    /// GitHub star 数（热度参考）；视频/文章条目为 None
    pub stars: Option<i64>,
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
             (source, external_id, url, title, subtitle, description, extra_json, status, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, COALESCE(?8, 'ok'), ?9, ?9)",
            rusqlite::params![
                item.source,
                item.external_id,
                item.url,
                item.title,
                item.subtitle,
                item.description,
                item.extra_json,
                item.initial_status,
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

/// 取一批「待归类」条目：**没有标签** 且 **未被人工锁定** 且 **未失效**。
///
/// - 人工改过标签的条目（`ai_locked = 1`）永不再进入归类批次——用户的选择优先于模型；
/// - 已失效（`gone`）的条目同样跳过：给一个打不开的仓库归类没有意义，还白花 token。
pub fn select_unclassified(conn: &Connection, limit: usize) -> Result<Vec<ClassifyItem>, String> {
    let mut statement = conn
        .prepare(
            "SELECT f.id, f.title, COALESCE(f.description, ''), COALESCE(f.extra_json, '') \
             FROM favorite f \
             WHERE f.ai_locked = 0 \
               AND f.status != 'gone' \
               AND NOT EXISTS (SELECT 1 FROM favorite_tag t WHERE t.favorite_id = f.id) \
             ORDER BY f.id LIMIT ?1",
        )
        .map_err(|e| format!("查询待归类条目失败: {}", e))?;
    let rows = statement
        .query_map([limit as i64], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|e| format!("查询待归类条目失败: {}", e))?;

    let mut items = Vec::new();
    for row in rows {
        let (id, title, description, extra_json) =
            row.map_err(|e| format!("读取待归类条目失败: {}", e))?;
        let extra: serde_json::Value =
            serde_json::from_str(&extra_json).unwrap_or(serde_json::Value::Null);
        let language = extra
            .get("language")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let topics = extra
            .get("topics")
            .and_then(|v| v.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|t| t.as_str())
                    .map(|t| t.to_string())
                    .collect()
            })
            .unwrap_or_default();
        let stars = extra
            .get("stars")
            .and_then(|v| v.as_i64())
            .filter(|count| *count > 0);
        items.push(ClassifyItem {
            id,
            title,
            description,
            language,
            topics,
            stars,
        });
    }
    Ok(items)
}

/// 写入一批归类结果（多标签：一个条目可落多个分类），并记录所用模型。
pub fn apply_tags(
    conn: &Connection,
    items: &[ClassifyItem],
    groups: &[(String, Vec<usize>)],
    model: &str,
) -> Result<usize, String> {
    let written = now_str();
    let mut count = 0usize;
    for (name, indices) in groups {
        for index in indices {
            let Some(item) = items.get(*index) else {
                continue;
            };
            conn.execute(
                "INSERT OR IGNORE INTO favorite_tag (favorite_id, tag) VALUES (?1, ?2)",
                rusqlite::params![item.id, name],
            )
            .map_err(|e| format!("写入分类标签失败: {}", e))?;
            count += 1;
        }
    }
    for item in items {
        conn.execute(
            "UPDATE favorite SET ai_model = ?1, ai_at = ?2 WHERE id = ?3",
            rusqlite::params![model, written, item.id],
        )
        .map_err(|e| format!("记录归类模型失败: {}", e))?;
    }
    Ok(count)
}

/// 读取某平台的 Cookie（没配过返回 None）。
pub fn get_credential(conn: &Connection, source: &str) -> Result<Option<String>, String> {
    let mut statement = conn
        .prepare("SELECT cookie FROM favorite_credential WHERE source = ?1")
        .map_err(|e| format!("读取凭证失败: {}", e))?;
    match statement.query_row([source], |row| row.get::<_, String>(0)) {
        Ok(cookie) => Ok(Some(cookie)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(format!("读取凭证失败: {}", e)),
    }
}

/// 写入 / 清除某平台的 Cookie（传空串即清除）。
pub fn set_credential(conn: &Connection, source: &str, cookie: &str) -> Result<(), String> {
    let trimmed = cookie.trim();
    if trimmed.is_empty() {
        conn.execute("DELETE FROM favorite_credential WHERE source = ?1", [source])
            .map_err(|e| format!("清除凭证失败: {}", e))?;
        return Ok(());
    }
    conn.execute(
        "INSERT INTO favorite_credential (source, cookie, updated_at) VALUES (?1, ?2, ?3) \
         ON CONFLICT(source) DO UPDATE SET cookie = ?2, updated_at = ?3",
        rusqlite::params![source, trimmed, now_str()],
    )
    .map_err(|e| format!("保存凭证失败: {}", e))?;
    Ok(())
}

/// 待探测条目：`(id, full_name, url)`。`all = false` 时跳过已探测过的。
pub fn select_for_check(
    conn: &Connection,
    source: &str,
    all: bool,
    limit: usize,
) -> Result<Vec<(i64, String, String)>, String> {
    let sql = if all {
        "SELECT id, title, url FROM favorite WHERE source = ?1 ORDER BY id LIMIT ?2"
    } else {
        "SELECT id, title, url FROM favorite WHERE source = ?1 AND checked_at IS NULL \
         ORDER BY id LIMIT ?2"
    };
    let mut statement = conn
        .prepare(sql)
        .map_err(|e| format!("查询待探测条目失败: {}", e))?;
    let rows = statement
        .query_map(rusqlite::params![source, limit as i64], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })
        .map_err(|e| format!("查询待探测条目失败: {}", e))?;
    let mut items = Vec::new();
    for row in rows {
        items.push(row.map_err(|e| format!("读取待探测条目失败: {}", e))?);
    }
    Ok(items)
}

/// 落探测结论；改名的顺手把 URL 指到新地址。
pub fn apply_status(
    conn: &Connection,
    id: i64,
    status: &str,
    new_url: Option<&str>,
) -> Result<(), String> {
    match new_url {
        Some(url) => conn.execute(
            "UPDATE favorite SET status = ?1, url = ?2, checked_at = ?3 WHERE id = ?4",
            rusqlite::params![status, url, now_str(), id],
        ),
        None => conn.execute(
            "UPDATE favorite SET status = ?1, checked_at = ?2 WHERE id = ?3",
            rusqlite::params![status, now_str(), id],
        ),
    }
    .map_err(|e| format!("更新探测状态失败: {}", e))?;
    Ok(())
}

/// 列表用的一条收藏（含标签）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FavoriteRow {
    pub id: i64,
    pub source: String,
    pub external_id: String,
    pub url: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    pub status: String,
    pub checked_at: Option<String>,
    pub ai_locked: bool,
    pub ai_model: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub tags: Vec<String>,
}

/// 列表筛选条件（全部可选，不传就是不过滤）。
#[derive(Debug, Clone, Default)]
pub struct ListFilter {
    pub source: Option<String>,
    pub tag: Option<String>,
    pub status: Option<String>,
    pub keyword: Option<String>,
    pub limit: usize,
}

/// 按条件列出条目（默认按最近更新排序，最多 1000 条）。
pub fn list(conn: &Connection, filter: &ListFilter) -> Result<Vec<FavoriteRow>, String> {
    let limit = if filter.limit == 0 { 1000 } else { filter.limit };
    let keyword = filter
        .keyword
        .as_deref()
        .map(|k| format!("%{}%", k.trim()))
        .filter(|k| k != "%%");

    let sql = format!(
        "SELECT f.id, f.source, f.external_id, f.url, f.title, f.subtitle, f.description, \
                f.status, f.checked_at, f.ai_locked, f.ai_model, f.created_at, f.updated_at \
         FROM favorite f \
         WHERE (?1 IS NULL OR f.source = ?1) \
           AND (?2 IS NULL OR f.status = ?2) \
           AND (?3 IS NULL OR f.title LIKE ?3 OR COALESCE(f.description, '') LIKE ?3) \
           AND (?4 IS NULL OR EXISTS (SELECT 1 FROM favorite_tag t WHERE t.favorite_id = f.id AND t.tag = ?4)) \
         ORDER BY f.updated_at DESC, f.id DESC LIMIT {}",
        limit
    );
    let mut statement = conn
        .prepare(&sql)
        .map_err(|e| format!("查询收藏列表失败: {}", e))?;
    let rows = statement
        .query_map(
            rusqlite::params![filter.source, filter.status, keyword, filter.tag],
            |row| {
                Ok(FavoriteRow {
                    id: row.get(0)?,
                    source: row.get(1)?,
                    external_id: row.get(2)?,
                    url: row.get(3)?,
                    title: row.get(4)?,
                    subtitle: row.get(5)?,
                    description: row.get(6)?,
                    status: row.get(7)?,
                    checked_at: row.get(8)?,
                    ai_locked: row.get::<_, i64>(9)? != 0,
                    ai_model: row.get(10)?,
                    created_at: row.get(11)?,
                    updated_at: row.get(12)?,
                    tags: Vec::new(),
                })
            },
        )
        .map_err(|e| format!("查询收藏列表失败: {}", e))?;

    let mut items = Vec::new();
    for row in rows {
        items.push(row.map_err(|e| format!("读取收藏条目失败: {}", e))?);
    }
    // 标签单独查一次再挂回去：条目量上千时比每行一个子查询便宜得多
    let mut tag_map: std::collections::HashMap<i64, Vec<String>> = std::collections::HashMap::new();
    {
        let mut tag_statement = conn
            .prepare("SELECT favorite_id, tag FROM favorite_tag ORDER BY tag")
            .map_err(|e| format!("查询分类标签失败: {}", e))?;
        let tag_rows = tag_statement
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
            .map_err(|e| format!("查询分类标签失败: {}", e))?;
        for row in tag_rows {
            let (id, tag) = row.map_err(|e| format!("读取分类标签失败: {}", e))?;
            tag_map.entry(id).or_default().push(tag);
        }
    }
    for item in items.iter_mut() {
        item.tags = tag_map.remove(&item.id).unwrap_or_default();
    }
    Ok(items)
}

/// 人工设置标签（全量替换），并锁定：后续 AI 归类不再碰它。
pub fn set_tags(conn: &Connection, id: i64, tags: &[String]) -> Result<(), String> {
    conn.execute("DELETE FROM favorite_tag WHERE favorite_id = ?1", [id])
        .map_err(|e| format!("清除旧分类失败: {}", e))?;
    for tag in tags {
        let trimmed = tag.trim();
        if trimmed.is_empty() {
            continue;
        }
        conn.execute(
            "INSERT OR IGNORE INTO favorite_tag (favorite_id, tag) VALUES (?1, ?2)",
            rusqlite::params![id, trimmed],
        )
        .map_err(|e| format!("写入分类标签失败: {}", e))?;
    }
    conn.execute(
        "UPDATE favorite SET ai_locked = 1, updated_at = ?1 WHERE id = ?2",
        rusqlite::params![now_str(), id],
    )
    .map_err(|e| format!("锁定条目失败: {}", e))?;
    Ok(())
}

/// 删除本地条目（只删本地，不动平台）。
pub fn delete(conn: &Connection, id: i64) -> Result<bool, String> {
    conn.execute("DELETE FROM favorite_tag WHERE favorite_id = ?1", [id])
        .map_err(|e| format!("删除分类标签失败: {}", e))?;
    let removed = conn
        .execute("DELETE FROM favorite WHERE id = ?1", [id])
        .map_err(|e| format!("删除收藏条目失败: {}", e))?;
    Ok(removed > 0)
}

#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FavoriteStats {
    pub total: usize,
    pub unclassified: usize,
    pub gone: usize,
    pub by_source: Vec<(String, usize)>,
    pub tags: Vec<(String, usize)>,
}

/// 概览：总数 / 未归类 / 已失效 / 各源条数 / 各分类条数。
pub fn stats(conn: &Connection) -> Result<FavoriteStats, String> {
    let total: usize = conn
        .query_row("SELECT COUNT(*) FROM favorite", [], |r| r.get(0))
        .map_err(|e| format!("统计总数失败: {}", e))?;
    let gone: usize = conn
        .query_row("SELECT COUNT(*) FROM favorite WHERE status = 'gone'", [], |r| r.get(0))
        .map_err(|e| format!("统计失效数失败: {}", e))?;
    let unclassified = select_unclassified(conn, 1_000_000)?.len();

    let mut by_source: Vec<(String, usize)> = Vec::new();
    {
        let mut statement = conn
            .prepare("SELECT source, COUNT(*) FROM favorite GROUP BY source ORDER BY COUNT(*) DESC")
            .map_err(|e| format!("统计来源失败: {}", e))?;
        let rows = statement
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, usize>(1)?)))
            .map_err(|e| format!("统计来源失败: {}", e))?;
        for row in rows {
            by_source.push(row.map_err(|e| format!("读取来源统计失败: {}", e))?);
        }
    }
    let mut tags: Vec<(String, usize)> = Vec::new();
    {
        let mut statement = conn
            .prepare("SELECT tag, COUNT(*) FROM favorite_tag GROUP BY tag ORDER BY COUNT(*) DESC, tag")
            .map_err(|e| format!("统计分类失败: {}", e))?;
        let rows = statement
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, usize>(1)?)))
            .map_err(|e| format!("统计分类失败: {}", e))?;
        for row in rows {
            tags.push(row.map_err(|e| format!("读取分类统计失败: {}", e))?);
        }
    }

    Ok(FavoriteStats {
        total,
        unclassified,
        gone,
        by_source,
        tags,
    })
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
    use super::{
        apply_status, apply_tags, count_all, delete, get_credential, list, migrate,
        select_unclassified, set_credential, set_tags, stats, upsert, ListFilter, NewFavorite,
        UpsertOutcome,
    };

    fn sample(external_id: &str, title: &str) -> NewFavorite {
        NewFavorite {
            source: "github".to_string(),
            external_id: external_id.to_string(),
            url: format!("https://github.com/{}", title),
            title: title.to_string(),
            subtitle: Some(title.to_string()),
            description: Some("desc".to_string()),
            extra_json: Some("{}".to_string()),
            initial_status: None,
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

    fn insert_raw(conn: &rusqlite::Connection, id: i64, external_id: &str, extra: &str) {
        conn.execute(
            "INSERT INTO favorite (id, source, external_id, url, title, extra_json, created_at, updated_at) \
             VALUES (?1, 'github', ?2, 'https://x', ?2, ?3, 'now', 'now')",
            rusqlite::params![id, external_id, extra],
        )
        .unwrap();
    }

    /// 凭证按 source 隔离：收藏模块的 GitHub Token 与 B站 Cookie 互不影响，
    /// 也就不会出现「换了 B站 Cookie 把 GitHub Token 顶掉」这种事。
    #[test]
    fn credentials_are_stored_per_source() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        assert_eq!(get_credential(&conn, "github").unwrap(), None);

        set_credential(&conn, "github", "ghp_xxx").unwrap();
        set_credential(&conn, "bilibili", "SESSDATA=yyy").unwrap();
        assert_eq!(get_credential(&conn, "github").unwrap().as_deref(), Some("ghp_xxx"));
        assert_eq!(
            get_credential(&conn, "bilibili").unwrap().as_deref(),
            Some("SESSDATA=yyy")
        );

        // 覆盖同一个 source 不影响另一个；空串即清除
        set_credential(&conn, "github", "ghp_new").unwrap();
        assert_eq!(get_credential(&conn, "bilibili").unwrap().as_deref(), Some("SESSDATA=yyy"));
        set_credential(&conn, "github", "").unwrap();
        assert_eq!(get_credential(&conn, "github").unwrap(), None);
        assert_eq!(get_credential(&conn, "bilibili").unwrap().is_some(), true);
    }

    /// 已失效的条目不再送归类（给它分类没意义，还白花 token）。
    #[test]
    fn select_unclassified_skips_gone_items() {
        let conn = seeded();
        apply_status(&conn, 2, "gone", None).unwrap();
        let pending = select_unclassified(&conn, 10).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, 1);
    }

    /// 归类只处理「没标签且没被人工改过」的条目。
    #[test]
    fn select_unclassified_skips_tagged_and_locked() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        insert_raw(&conn, 1, "1", "{}");
        insert_raw(&conn, 2, "2", "{}");
        insert_raw(&conn, 3, "3", "{}");

        let pending = select_unclassified(&conn, 10).unwrap();
        assert_eq!(pending.len(), 3);

        // 已有标签 → 不再归类
        conn.execute("INSERT INTO favorite_tag VALUES (1, 'CLI')", []).unwrap();
        // 人工锁定 → 永远不再归类
        conn.execute("UPDATE favorite SET ai_locked = 1 WHERE id = 2", []).unwrap();

        let pending = select_unclassified(&conn, 10).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, 3);
    }

    /// 多标签：一个条目落两个分类要写两行；写完之后它就不再是「待归类」。
    #[test]
    fn apply_tags_writes_every_label_and_clears_pending() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        insert_raw(&conn, 1, "1", "{}");
        let items = select_unclassified(&conn, 10).unwrap();
        let groups = vec![
            ("LLM".to_string(), vec![0usize]),
            ("运维".to_string(), vec![0usize]),
        ];
        let written = apply_tags(&conn, &items, &groups, "gpt-x").unwrap();
        assert_eq!(written, 2, "多标签每条都要写");

        let tags: Vec<String> = conn
            .prepare("SELECT tag FROM favorite_tag WHERE favorite_id = 1 ORDER BY tag")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(tags, vec!["LLM".to_string(), "运维".to_string()]);
        assert!(select_unclassified(&conn, 10).unwrap().is_empty());

        let model: String = conn
            .query_row("SELECT ai_model FROM favorite WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(model, "gpt-x");
    }

    /// 归类要能读到语言与 topics（这是判断分类最有用的两个信号）。
    #[test]
    fn select_unclassified_reads_language_and_topics() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        insert_raw(
            &conn,
            1,
            "o/r",
            r#"{"language":"Rust","topics":["cli","rust"]}"#,
        );
        let items = select_unclassified(&conn, 10).unwrap();
        assert_eq!(items[0].language, "Rust");
        assert_eq!(items[0].topics, vec!["cli".to_string(), "rust".to_string()]);
    }

    fn seeded() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        insert_raw(&conn, 1, "1", r#"{"language":"Rust"}"#);
        insert_raw(&conn, 2, "2", r#"{"language":"Go"}"#);
        conn.execute("UPDATE favorite SET title = 'o/cli', description = 'a fast cli' WHERE id = 1", [])
            .unwrap();
        conn.execute("UPDATE favorite SET title = 'o/web', description = 'web framework' WHERE id = 2", [])
            .unwrap();
        conn
    }

    /// 人工改标签 = 全量替换 + 锁定（用户的选择优先于模型）。
    #[test]
    fn set_tags_replaces_and_locks() {
        let conn = seeded();
        set_tags(&conn, 1, &["CLI".to_string(), "工具".to_string()]).unwrap();
        let row = &list(&conn, &ListFilter::default()).unwrap()
            .into_iter()
            .find(|r| r.id == 1)
            .unwrap();
        assert_eq!(row.tags, vec!["CLI".to_string(), "工具".to_string()]);
        assert!(row.ai_locked);
        assert!(select_unclassified(&conn, 10).unwrap().iter().all(|i| i.id != 1));

        // 再设一次：旧标签被替换而不是叠加
        set_tags(&conn, 1, &["CLI".to_string()]).unwrap();
        let row = &list(&conn, &ListFilter::default()).unwrap()
            .into_iter()
            .find(|r| r.id == 1)
            .unwrap();
        assert_eq!(row.tags, vec!["CLI".to_string()]);
    }

    /// 删本地条目要连标签一起删（不留孤儿行）。
    #[test]
    fn delete_removes_tags_too() {
        let conn = seeded();
        set_tags(&conn, 1, &["CLI".to_string()]).unwrap();
        assert!(delete(&conn, 1).unwrap());
        assert!(!delete(&conn, 1).unwrap(), "删第二次应返回 false");
        let left: i64 = conn
            .query_row("SELECT COUNT(*) FROM favorite_tag", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0);
        assert_eq!(count_all(&conn).unwrap(), 1);
    }

    #[test]
    fn list_filters_by_source_tag_and_keyword() {
        let conn = seeded();
        set_tags(&conn, 1, &["CLI".to_string()]).unwrap();
        set_tags(&conn, 2, &["Web".to_string()]).unwrap();

        assert_eq!(list(&conn, &ListFilter::default()).unwrap().len(), 2);
        assert_eq!(
            list(&conn, &ListFilter { tag: Some("CLI".into()), ..Default::default() })
                .unwrap()
                .len(),
            1
        );
        // 关键字同时匹配标题与描述
        assert_eq!(
            list(&conn, &ListFilter { keyword: Some("cli".into()), ..Default::default() })
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            list(&conn, &ListFilter { keyword: Some("framework".into()), ..Default::default() })
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            list(&conn, &ListFilter { source: Some("zhihu".into()), ..Default::default() })
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn stats_counts_unclassified_and_gone() {
        let conn = seeded();
        let before = stats(&conn).unwrap();
        assert_eq!(before.total, 2);
        assert_eq!(before.unclassified, 2);
        assert_eq!(before.gone, 0);

        set_tags(&conn, 1, &["CLI".to_string()]).unwrap();
        apply_status(&conn, 2, "gone", None).unwrap();
        let after = stats(&conn).unwrap();
        assert_eq!(after.unclassified, 0, "已归类的 + 已失效的都不再送归类");
        assert_eq!(after.gone, 1);
        assert_eq!(after.tags, vec![("CLI".to_string(), 1)]);
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
