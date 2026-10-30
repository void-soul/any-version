use super::*;

// ─── 工具 ───

pub(super) fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// 生成 UUID v4（与 Flutter uuid 包格式一致）。
pub(super) fn uuid_v4() -> String {
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

pub(super) fn db_path() -> std::path::PathBuf {
    get_data_dir().join("picky").join("picky.db")
}

pub(super) fn open_db() -> Result<rusqlite::Connection, String> {
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
            extra TEXT NOT NULL DEFAULT '{}',
            content TEXT
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
        CREATE TABLE IF NOT EXISTS picky_tombstones (
            entity TEXT NOT NULL,
            id TEXT NOT NULL,
            deleted_at TEXT NOT NULL,
            PRIMARY KEY (entity, id)
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
    // 迁移：旧库无 content 列（正文）时补上，保证正文随书签 S3 同步互通。
    let has_content: bool = conn
        .prepare("SELECT 1 FROM pragma_table_info('picky_bookmarks') WHERE name = 'content'")
        .and_then(|mut s| s.query_row([], |r| r.get::<_, i64>(0)))
        .map(|v| v != 0)
        .unwrap_or(false);
    if !has_content {
        conn.execute("ALTER TABLE picky_bookmarks ADD COLUMN content TEXT", [])
            .map_err(|e| format!("迁移 picky_bookmarks.content 列失败: {}", e))?;
    }
    Ok(conn)
}

// ─── Bookmark 读写 ───

pub(super) fn bookmark_from_row(row: &rusqlite::Row) -> rusqlite::Result<Bookmark> {
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
        // content 列由 ALTER TABLE 迁移添加、无默认值：旧行该列为 NULL，必须按
        // Option<String> 读取（按 String 读 NULL 会报 "Invalid column type Null"）。
        content: row
            .get::<_, Option<String>>(11)?
            .filter(|s| !s.trim().is_empty()),
    })
}

/// 按 id 读取单条收藏（用于 refetch 等场景）。
pub(super) fn get_bookmark(conn: &rusqlite::Connection, id: &str) -> Result<Bookmark, String> {
    conn.query_row(
        "SELECT id,title,description,url,image_url,favicon_url,created_at,updated_at,refined,meta_fetched,extra,content FROM picky_bookmarks WHERE id=?1",
        [id],
        |row| bookmark_from_row(row),
    )
    .map_err(|e| format!("收藏不存在: {}", e))
}
