//! Picky 模块：收藏 / 归档页面（与 Flutter 端 picky 保持同一数据接口）。
//!
//! 数据字段（camelCase，与 Flutter 模型 toJson 一致）：
//! - 页面信息：id / title / description / url / imageUrl / faviconUrl / createdAt / updatedAt / metaFetched
//! - 状态：refined（false=收藏中，true=已归档/炼化）
//! - 评论：id / bookmarkId / content / createdAt / updatedAt / parentId（树形，2 级）
//! - 标签：id / name / color / createdAt，书签-标签关联 bookmarkTags: { bookmarkId: [tagId...] }
//!
//! 云同步：S3 兼容存储（SigV4），state.json 结构与 Flutter 端完全一致，可跨端互通。
//! 收藏中的 lingzuCode / aiCopy 等个性化字段不做处理，但会原样保留（extra），
//! 避免与 Flutter 端来回同步时丢失其数据。

mod s3;

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};

use crate::commands::config::get_data_dir;
pub use s3::PickySyncConfig;

// ─── 模型（camelCase，与 Flutter 端一致） ───

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bookmark {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub image_url: Option<String>,
    #[serde(default)]
    pub favicon_url: Option<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub refined: bool,
    #[serde(default)]
    pub meta_fetched: bool,
    /// 保留 Flutter 端的个性化等未知字段（lingzuCode / aiCopy 等），同步时原样带回。
    #[serde(flatten)]
    pub extra: JsonMap<String, JsonValue>,
}

impl Default for Bookmark {
    fn default() -> Self {
        let now = now_iso();
        Self {
            id: uuid_v4(),
            title: String::new(),
            description: None,
            url: None,
            image_url: None,
            favicon_url: None,
            created_at: now.clone(),
            updated_at: now,
            refined: false,
            meta_fetched: false,
            extra: JsonMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub id: String,
    #[serde(default)]
    pub bookmark_id: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_tag_color")]
    pub color: String,
    #[serde(default)]
    pub created_at: String,
}

fn default_tag_color() -> String {
    "#4FC3F7".to_string()
}

/// 全量状态（前端一次性加载 + 同步用）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PickyState {
    pub bookmarks: Vec<Bookmark>,
    pub comments: Vec<Comment>,
    pub tags: Vec<Tag>,
    pub bookmark_tags: HashMap<String, Vec<String>>,
}

// ─── 工具 ───

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// 生成 UUID v4（与 Flutter uuid 包格式一致）。
fn uuid_v4() -> String {
    let mut bytes = [0u8; 16];
    let _ = getrandom::getrandom(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{}-{}-{}-{}-{}",
        hex(&bytes[0..4]),
        hex(&bytes[4..6]),
        hex(&bytes[6..8]),
        hex(&bytes[8..10]),
        hex(&bytes[10..16])
    )
}

// ─── SQLite 存储 ───

fn db_path() -> std::path::PathBuf {
    get_data_dir().join("picky").join("picky.db")
}

fn open_db() -> Result<rusqlite::Connection, String> {
    let dir = get_data_dir().join("picky");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建 Picky 数据目录失败: {}", e))?;
    let conn = rusqlite::Connection::open(db_path())
        .map_err(|e| format!("打开 Picky 数据库失败: {}", e))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS picky_bookmarks (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL DEFAULT '',
            description TEXT,
            url TEXT,
            image_url TEXT,
            favicon_url TEXT,
            created_at TEXT NOT NULL DEFAULT '',
            updated_at TEXT NOT NULL DEFAULT '',
            refined INTEGER NOT NULL DEFAULT 0,
            meta_fetched INTEGER NOT NULL DEFAULT 0,
            extra TEXT NOT NULL DEFAULT '{}'
        );
        CREATE TABLE IF NOT EXISTS picky_comments (
            id TEXT PRIMARY KEY,
            bookmark_id TEXT NOT NULL DEFAULT '',
            content TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL DEFAULT '',
            updated_at TEXT NOT NULL DEFAULT '',
            parent_id TEXT
        );
        CREATE TABLE IF NOT EXISTS picky_tags (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL DEFAULT '',
            color TEXT NOT NULL DEFAULT '#4FC3F7',
            created_at TEXT NOT NULL DEFAULT ''
        );
        CREATE TABLE IF NOT EXISTS picky_bookmark_tags (
            bookmark_id TEXT NOT NULL,
            tag_id TEXT NOT NULL,
            PRIMARY KEY (bookmark_id, tag_id)
        );
        CREATE TABLE IF NOT EXISTS picky_sync_config (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            endpoint TEXT,
            region TEXT NOT NULL DEFAULT 'us-east-1',
            access_key_id TEXT NOT NULL DEFAULT '',
            secret_enc TEXT NOT NULL DEFAULT '',
            bucket_name TEXT NOT NULL DEFAULT '',
            prefix TEXT,
            enabled INTEGER NOT NULL DEFAULT 0,
            last_sync_at TEXT,
            addressing_style TEXT NOT NULL DEFAULT 'auto',
            tls_verify INTEGER NOT NULL DEFAULT 1,
            timeout_seconds INTEGER NOT NULL DEFAULT 30,
            concurrent_reqs INTEGER NOT NULL DEFAULT 1
        );",
    )
    .map_err(|e| format!("初始化 Picky 表失败: {}", e))?;
    Ok(conn)
}

// ─── Bookmark 读写 ───

fn bookmark_from_row(row: &rusqlite::Row) -> rusqlite::Result<Bookmark> {
    let extra_raw: String = row.get(10)?;
    let extra: JsonMap<String, JsonValue> = serde_json::from_str(&extra_raw).unwrap_or_default();
    Ok(Bookmark {
        id: row.get(0)?,
        title: row.get(1)?,
        description: row.get(2)?,
        url: row.get(3)?,
        image_url: row.get(4)?,
        favicon_url: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        refined: row.get::<_, i64>(8)? != 0,
        meta_fetched: row.get::<_, i64>(9)? != 0,
        extra,
    })
}

fn insert_bookmark(conn: &rusqlite::Connection, bm: &Bookmark) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO picky_bookmarks
         (id, title, description, url, image_url, favicon_url, created_at, updated_at, refined, meta_fetched, extra)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        rusqlite::params![
            bm.id,
            bm.title,
            bm.description,
            bm.url,
            bm.image_url,
            bm.favicon_url,
            bm.created_at,
            bm.updated_at,
            bm.refined as i64,
            bm.meta_fetched as i64,
            serde_json::to_string(&bm.extra).unwrap_or_else(|_| "{}".to_string()),
        ],
    )
    .map_err(|e| format!("写入收藏失败: {}", e))?;
    Ok(())
}

fn bookmark_exists(conn: &rusqlite::Connection, id: &str) -> bool {
    conn.query_row("SELECT 1 FROM picky_bookmarks WHERE id=?1", [id], |_| Ok(()))
        .is_ok()
}

fn comment_exists(conn: &rusqlite::Connection, id: &str) -> bool {
    conn.query_row("SELECT 1 FROM picky_comments WHERE id=?1", [id], |_| Ok(()))
        .is_ok()
}

fn tag_exists(conn: &rusqlite::Connection, id: &str) -> bool {
    conn.query_row("SELECT 1 FROM picky_tags WHERE id=?1", [id], |_| Ok(()))
        .is_ok()
}

fn add_binding_if_absent(conn: &rusqlite::Connection, bookmark_id: &str, tag_id: &str) {
    let _ = conn.execute(
        "INSERT OR IGNORE INTO picky_bookmark_tags (bookmark_id, tag_id) VALUES (?1,?2)",
        rusqlite::params![bookmark_id, tag_id],
    );
}

// ─── 敏感字段加密（复用 commands::secrets，与 cert 模块同一主密钥） ───
use crate::commands::secrets::{decrypt_secret, encrypt_secret};

// ─── 同步配置读写 ───

fn config_from_row(row: &rusqlite::Row) -> rusqlite::Result<PickySyncConfig> {
    let secret_enc: String = row.get(4)?;
    let secret = decrypt_secret(&secret_enc).unwrap_or_default();
    Ok(PickySyncConfig {
        endpoint: row.get(1)?,
        region: row.get(2)?,
        access_key_id: row.get(3)?,
        secret_access_key: secret,
        bucket_name: row.get(5)?,
        prefix: row.get(6)?,
        enabled: row.get::<_, i64>(7)? != 0,
        last_sync_at: row.get(8)?,
        addressing_style: row.get(9)?,
        tls_verify: row.get::<_, i64>(10)? != 0,
        timeout_seconds: row.get(11)?,
        concurrent_reqs: row.get(12)?,
    })
}

fn load_sync_config(conn: &rusqlite::Connection) -> PickySyncConfig {
    conn.query_row("SELECT * FROM picky_sync_config WHERE id=1", [], |row| config_from_row(row))
        .unwrap_or_default()
}

fn save_sync_config_db(conn: &rusqlite::Connection, cfg: &PickySyncConfig) -> Result<(), String> {
    let secret_enc = encrypt_secret(&cfg.secret_access_key)?;
    conn.execute(
        "INSERT OR REPLACE INTO picky_sync_config
         (id, endpoint, region, access_key_id, secret_enc, bucket_name, prefix, enabled, last_sync_at, addressing_style, tls_verify, timeout_seconds, concurrent_reqs)
         VALUES (1,?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        rusqlite::params![
            cfg.endpoint,
            cfg.region,
            cfg.access_key_id,
            secret_enc,
            cfg.bucket_name,
            cfg.prefix,
            cfg.enabled as i64,
            cfg.last_sync_at,
            cfg.addressing_style,
            cfg.tls_verify as i64,
            cfg.timeout_seconds as i64,
            cfg.concurrent_reqs as i64,
        ],
    )
    .map_err(|e| format!("保存云同步配置失败: {}", e))?;
    Ok(())
}

// ─── 状态序列化（供同步） ───

fn state_to_json(conn: &rusqlite::Connection) -> Result<PickyState, String> {
    let mut bookmarks = Vec::new();
    {
        let mut stmt = conn
            .prepare("SELECT id,title,description,url,image_url,favicon_url,created_at,updated_at,refined,meta_fetched,extra FROM picky_bookmarks")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| bookmark_from_row(row))
            .map_err(|e| e.to_string())?;
        for r in rows {
            bookmarks.push(r.map_err(|e| e.to_string())?);
        }
    }
    let mut comments = Vec::new();
    {
        let mut stmt = conn
            .prepare("SELECT id,bookmark_id,content,created_at,updated_at,parent_id FROM picky_comments")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok(Comment {
                    id: row.get(0)?,
                    bookmark_id: row.get(1)?,
                    content: row.get(2)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                    parent_id: row.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?;
        for r in rows {
            comments.push(r.map_err(|e| e.to_string())?);
        }
    }
    let mut tags = Vec::new();
    {
        let mut stmt = conn
            .prepare("SELECT id,name,color,created_at FROM picky_tags")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok(Tag {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    color: row.get(2)?,
                    created_at: row.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?;
        for r in rows {
            tags.push(r.map_err(|e| e.to_string())?);
        }
    }
    let mut bookmark_tags: HashMap<String, Vec<String>> = HashMap::new();
    {
        let mut stmt = conn
            .prepare("SELECT bookmark_id, tag_id FROM picky_bookmark_tags")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?;
        for r in rows {
            let (bid, tid) = r.map_err(|e| e.to_string())?;
            bookmark_tags.entry(bid).or_default().push(tid);
        }
    }
    Ok(PickyState { bookmarks, comments, tags, bookmark_tags })
}

// ─── 云同步合并（合并语义：LWW 按 updatedAt 后写覆盖，绝不丢数据） ───
// 收藏/评论：本地缺失 → 插入；已存在且云端 updatedAt 更新 → 覆盖本地（后写优先）。
// 标签：按 id 只补缺失（避免覆盖本地改名）。标签关联：幂等补充。

#[derive(Debug, Clone, Default)]
struct MergeCounts {
    added_bookmarks: usize,
    updated_bookmarks: usize,
    added_comments: usize,
    updated_comments: usize,
    added_tags: usize,
}

/// 宽松解析 ISO 时间（rfc3339 / 无时区微秒两种格式，兼容 Flutter toIso8601String）。
fn parse_iso_utc(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&chrono::Utc));
    }
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f")
        .ok()
        .map(|n| n.and_utc())
}

/// 判断 incoming 是否比本地更新（按 updatedAt；解析失败时退化为字符串比较）。
fn incoming_is_newer(incoming: &str, local: &str) -> bool {
    match (parse_iso_utc(incoming), parse_iso_utc(local)) {
        (Some(a), Some(b)) => a > b,
        _ => incoming > local,
    }
}

fn insert_comment(conn: &rusqlite::Connection, c: &Comment) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO picky_comments (id, bookmark_id, content, created_at, updated_at, parent_id)
         VALUES (?1,?2,?3,?4,?5,?6)",
        rusqlite::params![c.id, c.bookmark_id, c.content, c.created_at, c.updated_at, c.parent_id],
    )
    .map_err(|e| format!("写入评论失败: {}", e))?;
    Ok(())
}

fn merge_cloud_state(conn: &rusqlite::Connection, state: &JsonValue) -> Result<MergeCounts, String> {
    let mut counts = MergeCounts::default();

    if let Some(arr) = state.get("bookmarks").and_then(|v| v.as_array()) {
        for item in arr {
            if let Ok(bm) = serde_json::from_value::<Bookmark>(item.clone()) {
                let local_updated: Option<String> = conn
                    .query_row("SELECT updated_at FROM picky_bookmarks WHERE id=?1", [&bm.id], |r| r.get(0))
                    .ok();
                match local_updated {
                    None => {
                        insert_bookmark(conn, &bm)?;
                        counts.added_bookmarks += 1;
                    }
                    Some(local) if incoming_is_newer(&bm.updated_at, &local) => {
                        insert_bookmark(conn, &bm)?; // INSERT OR REPLACE 全字段覆盖
                        counts.updated_bookmarks += 1;
                    }
                    _ => {} // 本地更新，保留
                }
            }
        }
    }
    if let Some(arr) = state.get("comments").and_then(|v| v.as_array()) {
        for item in arr {
            if let Ok(c) = serde_json::from_value::<Comment>(item.clone()) {
                let local_updated: Option<String> = conn
                    .query_row("SELECT updated_at FROM picky_comments WHERE id=?1", [&c.id], |r| r.get(0))
                    .ok();
                match local_updated {
                    None => {
                        insert_comment(conn, &c)?;
                        counts.added_comments += 1;
                    }
                    Some(local) if incoming_is_newer(&c.updated_at, &local) => {
                        insert_comment(conn, &c)?;
                        counts.updated_comments += 1;
                    }
                    _ => {}
                }
            }
        }
    }
    if let Some(arr) = state.get("tags").and_then(|v| v.as_array()) {
        for item in arr {
            if let Ok(t) = serde_json::from_value::<Tag>(item.clone()) {
                if tag_exists(conn, &t.id) {
                    continue;
                }
                conn.execute(
                    "INSERT OR REPLACE INTO picky_tags (id, name, color, created_at) VALUES (?1,?2,?3,?4)",
                    rusqlite::params![t.id, t.name, t.color, t.created_at],
                )
                .map_err(|e| format!("写入标签失败: {}", e))?;
                counts.added_tags += 1;
            }
        }
    }
    if let Some(map) = state.get("bookmarkTags").and_then(|v| v.as_object()) {
        for (bid, val) in map {
            if let Some(ids) = val.as_array() {
                for tid in ids {
                    if let Some(t) = tid.as_str() {
                        add_binding_if_absent(conn, bid, t);
                    }
                }
            }
        }
    }
    Ok(counts)
}

fn format_merge_summary(counts: &MergeCounts, prefix: &str) -> String {
    let mut parts = Vec::new();
    if counts.added_bookmarks > 0 {
        parts.push(format!("新增收藏 {}", counts.added_bookmarks));
    }
    if counts.updated_bookmarks > 0 {
        parts.push(format!("更新收藏 {}", counts.updated_bookmarks));
    }
    if counts.added_comments > 0 {
        parts.push(format!("新增评论 {}", counts.added_comments));
    }
    if counts.updated_comments > 0 {
        parts.push(format!("更新评论 {}", counts.updated_comments));
    }
    if counts.added_tags > 0 {
        parts.push(format!("新增标签 {}", counts.added_tags));
    }
    if parts.is_empty() {
        format!("{}（无变化）", prefix)
    } else {
        format!("{}：{}", prefix, parts.join("，"))
    }
}

// ─── 命令：收藏 ───

/// 一次性加载全量状态（收藏/评论/标签/关联）。
#[tauri::command]
pub fn picky_get_state() -> Result<PickyState, String> {
    let conn = open_db()?;
    state_to_json(&conn)
}

/// 新增收藏。title 为空时用 url 兜底；可传入从网页抓取的元数据（imageUrl/faviconUrl）。
#[tauri::command]
pub fn picky_add_bookmark(
    title: Option<String>,
    url: Option<String>,
    description: Option<String>,
    image_url: Option<String>,
    favicon_url: Option<String>,
) -> Result<Bookmark, String> {
    let conn = open_db()?;
    let mut bm = Bookmark {
        title: title.unwrap_or_default().trim().to_string(),
        url: url.map(|u| u.trim().to_string()).filter(|u| !u.is_empty()),
        description: description.map(|d| d.trim().to_string()).filter(|d| !d.is_empty()),
        image_url: image_url.map(|u| u.trim().to_string()).filter(|u| !u.is_empty()),
        favicon_url: favicon_url.map(|u| u.trim().to_string()).filter(|u| !u.is_empty()),
        ..Default::default()
    };
    if bm.title.is_empty() {
        bm.title = bm.url.clone().unwrap_or_else(|| "未命名收藏".to_string());
    }
    insert_bookmark(&conn, &bm)?;
    Ok(bm)
}

#[tauri::command]
pub fn picky_update_bookmark(bookmark: Bookmark) -> Result<(), String> {
    let conn = open_db()?;
    if !bookmark_exists(&conn, &bookmark.id) {
        return Err("收藏不存在".to_string());
    }
    let mut bm = bookmark;
    bm.updated_at = now_iso();
    insert_bookmark(&conn, &bm)
}

/// 切换收藏状态（refined：false=收藏中，true=已归档）。
#[tauri::command]
pub fn picky_set_refined(id: String, refined: bool) -> Result<(), String> {
    let conn = open_db()?;
    conn.execute(
        "UPDATE picky_bookmarks SET refined=?1, updated_at=?2 WHERE id=?3",
        rusqlite::params![refined as i64, now_iso(), id],
    )
    .map_err(|e| format!("更新收藏状态失败: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn picky_delete_bookmark(id: String) -> Result<(), String> {
    let conn = open_db()?;
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM picky_bookmarks WHERE id=?1", [&id])
        .map_err(|e| format!("删除收藏失败: {}", e))?;
    tx.execute("DELETE FROM picky_comments WHERE bookmark_id=?1", [&id])
        .map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM picky_bookmark_tags WHERE bookmark_id=?1", [&id])
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

// ─── 命令：评论 ───

#[tauri::command]
pub fn picky_add_comment(
    bookmark_id: String,
    content: String,
    parent_id: Option<String>,
) -> Result<Comment, String> {
    let conn = open_db()?;
    let content = content.trim().to_string();
    if content.is_empty() {
        return Err("评论内容不能为空".to_string());
    }
    if !bookmark_exists(&conn, &bookmark_id) {
        return Err("收藏不存在".to_string());
    }
    let now = now_iso();
    let c = Comment {
        id: uuid_v4(),
        bookmark_id,
        content,
        created_at: now.clone(),
        updated_at: now,
        parent_id: parent_id.filter(|p| !p.is_empty()),
    };
    conn.execute(
        "INSERT INTO picky_comments (id, bookmark_id, content, created_at, updated_at, parent_id)
         VALUES (?1,?2,?3,?4,?5,?6)",
        rusqlite::params![c.id, c.bookmark_id, c.content, c.created_at, c.updated_at, c.parent_id],
    )
    .map_err(|e| format!("写入评论失败: {}", e))?;
    Ok(c)
}

#[tauri::command]
pub fn picky_update_comment(comment: Comment) -> Result<(), String> {
    let conn = open_db()?;
    if !comment_exists(&conn, &comment.id) {
        return Err("评论不存在".to_string());
    }
    conn.execute(
        "UPDATE picky_comments SET content=?1, updated_at=?2 WHERE id=?3",
        rusqlite::params![comment.content, now_iso(), comment.id],
    )
    .map_err(|e| format!("更新评论失败: {}", e))?;
    Ok(())
}

/// 删除评论（连同其子回复）。
#[tauri::command]
pub fn picky_delete_comment(id: String) -> Result<(), String> {
    let conn = open_db()?;
    conn.execute("DELETE FROM picky_comments WHERE id=?1 OR parent_id=?1", [&id])
        .map_err(|e| format!("删除评论失败: {}", e))?;
    Ok(())
}

// ─── 命令：标签 ───

#[tauri::command]
pub fn picky_add_tag(name: String, color: Option<String>) -> Result<Tag, String> {
    let conn = open_db()?;
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("标签名不能为空".to_string());
    }
    // 同名标签去重
    let existing: Option<String> = conn
        .query_row("SELECT id FROM picky_tags WHERE name=?1", [&name], |r| r.get(0))
        .ok();
    if let Some(id) = existing {
        return Ok(Tag {
            id,
            name,
            color: color.unwrap_or_else(default_tag_color),
            created_at: now_iso(),
        });
    }
    let t = Tag {
        id: uuid_v4(),
        name,
        color: color.unwrap_or_else(default_tag_color),
        created_at: now_iso(),
    };
    conn.execute(
        "INSERT INTO picky_tags (id, name, color, created_at) VALUES (?1,?2,?3,?4)",
        rusqlite::params![t.id, t.name, t.color, t.created_at],
    )
    .map_err(|e| format!("写入标签失败: {}", e))?;
    Ok(t)
}

#[tauri::command]
pub fn picky_delete_tag(id: String) -> Result<(), String> {
    let conn = open_db()?;
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM picky_tags WHERE id=?1", [&id])
        .map_err(|e| format!("删除标签失败: {}", e))?;
    tx.execute("DELETE FROM picky_bookmark_tags WHERE tag_id=?1", [&id])
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

/// 切换书签-标签关联，返回切换后的状态（true=已关联）。
#[tauri::command]
pub fn picky_toggle_bookmark_tag(bookmark_id: String, tag_id: String) -> Result<bool, String> {
    let conn = open_db()?;
    let exists: bool = conn
        .query_row(
            "SELECT 1 FROM picky_bookmark_tags WHERE bookmark_id=?1 AND tag_id=?2",
            rusqlite::params![bookmark_id, tag_id],
            |_| Ok(()),
        )
        .is_ok();
    if exists {
        conn.execute(
            "DELETE FROM picky_bookmark_tags WHERE bookmark_id=?1 AND tag_id=?2",
            rusqlite::params![bookmark_id, tag_id],
        )
        .map_err(|e| format!("移除标签失败: {}", e))?;
        Ok(false)
    } else {
        add_binding_if_absent(&conn, &bookmark_id, &tag_id);
        Ok(true)
    }
}

// ─── 命令：云同步配置 ───

#[tauri::command]
pub fn picky_get_sync_config() -> Result<PickySyncConfig, String> {
    let conn = open_db()?;
    Ok(load_sync_config(&conn))
}

#[tauri::command]
pub fn picky_save_sync_config(config: PickySyncConfig) -> Result<(), String> {
    let conn = open_db()?;
    save_sync_config_db(&conn, &config)
}

// ─── 命令：云同步 ───

/// 双向同步（一个按钮内部自动合并）：
/// 1) 先下载云端状态并合并到本地（LWW：本地缺失补入，云端 updatedAt 更新则覆盖）；
/// 2) 再上传合并后的全量状态。
/// 这样无论哪端先/后同步，云端始终是两端数据的并集，绝不互相覆盖丢数据。
/// 返回结果文本。
#[tauri::command]
pub async fn picky_sync_now() -> Result<String, String> {
    let conn = open_db()?;
    let cfg = load_sync_config(&conn);
    if !cfg.is_configured() {
        return Err("云同步未配置（需启用并填写 endpoint / AccessKey / SecretKey / Bucket）".to_string());
    }

    // 1) 先合并云端到本地（不存在则下载并合并，404 视为首次同步）
    let client = s3::PickyS3Client::new(cfg.clone());
    let merged = match client.download_full_state().await? {
        Some(cloud) => {
            let counts = merge_cloud_state(&conn, &cloud)?;
            format_merge_summary(&counts, "已合并云端")
        }
        None => "云端暂无数据（首次同步）".to_string(),
    };

    // 2) 上传合并后的全量状态
    let state = state_to_json(&conn)?;
    let bookmarks: Vec<JsonValue> = state.bookmarks.iter().map(|b| serde_json::to_value(b).unwrap_or_default()).collect();
    let comments: Vec<JsonValue> = state.comments.iter().map(|c| serde_json::to_value(c).unwrap_or_default()).collect();
    let tags: Vec<JsonValue> = state.tags.iter().map(|t| serde_json::to_value(t).unwrap_or_default()).collect();
    let bm_tags: JsonMap<String, JsonValue> = state
        .bookmark_tags
        .iter()
        .map(|(k, v)| (k.clone(), serde_json::json!(v)))
        .collect();

    let synced_at = client.upload_full_state(&bookmarks, &comments, &tags, &bm_tags).await?;

    // 记录上次同步时间
    let mut updated = cfg.clone();
    updated.last_sync_at = Some(synced_at.clone());
    let conn2 = open_db()?;
    save_sync_config_db(&conn2, &updated)?;

    Ok(format!("同步完成。{} · 同步时间 {}", merged, synced_at))
}
