//! 剪贴板历史 SQLite 存储层

use rusqlite::{params, Connection};
use std::path::PathBuf;

use super::{ClipboardItem, ClipboardSettings};

/// 剪贴板内容为敏感数据（常含密码/密钥），落盘必须加密。
/// 与 secrets.rs 的 CRED_ENCRYPTION_MARKER 保持一致。
const ENC_MARKER: &str = "ENC_V2:";

/// 加密剪贴板文本内容（AES-256-GCM；空值与已加密值原样保留）
fn encrypt_content(value: Option<&str>) -> Option<String> {
    let value = value?;
    if value.is_empty() || value.starts_with(ENC_MARKER) {
        return Some(value.to_string());
    }
    Some(crate::commands::secrets::encrypt_secret(value).unwrap_or_else(|e| {
        eprintln!("[clipboard] content 加密失败: {}", e);
        value.to_string()
    }))
}

/// 解密剪贴板文本内容（兼容旧版明文）
fn decrypt_content(value: String) -> String {
    if value.starts_with(ENC_MARKER) {
        crate::commands::secrets::decrypt_secret(&value).unwrap_or_else(|e| {
            eprintln!("[clipboard] content 解密失败（密钥不匹配或数据损坏）: {}", e);
            value
        })
    } else {
        value
    }
}

/// 打开（或创建）剪贴板数据库；并对历史明文 content 做就地加密迁移（幂等）。
pub fn open_db(path: &std::path::Path) -> Result<Connection, String> {
    let conn = Connection::open(path).map_err(|e| format!("打开剪贴板数据库失败: {}", e))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| format!("设置 WAL 失败: {}", e))?;
    // 安全迁移：历史明文文本内容 → AES 加密（仅处理未加密行）
    let plain_rows: Vec<(i64, String)> = {
        let mut stmt = conn
            .prepare(
                "SELECT id, content FROM clipboard_items
                 WHERE kind='text' AND content != '' AND content NOT LIKE 'ENC_V2:%'",
            )
            .map_err(|e| format!("查询剪贴板明文行失败: {}", e))?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map(|iter| iter.filter_map(|r| r.ok()).collect())
            .unwrap_or_default()
    };
    for (id, content) in plain_rows {
        if let Some(enc) = encrypt_content(Some(&content)) {
            let _ = conn.execute(
                "UPDATE clipboard_items SET content=?1 WHERE id=?2",
                params![enc, id],
            );
        }
    }
    Ok(conn)
}

/// 初始化表结构
pub fn init_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS clipboard_items (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            kind       TEXT    NOT NULL,              -- 'text' | 'image'
            content    TEXT,                          -- 文本内容（kind=text）
            image_path TEXT,                          -- 图片相对 data 目录路径（kind=image）
            thumb_path TEXT,                          -- 缩略图相对路径
            width      INTEGER NOT NULL DEFAULT 0,
            height     INTEGER NOT NULL DEFAULT 0,
            source_app TEXT    NOT NULL DEFAULT '',
            hash       TEXT    NOT NULL,
            pinned     INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,              -- unix 秒
            formats    TEXT    NOT NULL DEFAULT ''    -- 复制时的剪贴板格式（JSON 数组）
        );
        CREATE INDEX IF NOT EXISTS idx_clip_created ON clipboard_items(created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_clip_hash    ON clipboard_items(hash);
        CREATE INDEX IF NOT EXISTS idx_clip_pinned  ON clipboard_items(pinned);

        CREATE TABLE IF NOT EXISTS clipboard_settings (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS clipboard_ignored_apps (
            app TEXT PRIMARY KEY
        );

        -- 附加格式数据（CopyQ 式）：复制时的 HTML/RTF 等原始字节，粘贴时原样写回
        CREATE TABLE IF NOT EXISTS clipboard_item_formats (
            id      INTEGER PRIMARY KEY AUTOINCREMENT,
            item_id INTEGER NOT NULL,
            mime    TEXT    NOT NULL,      -- 'text/html' | 'application/rtf'
            data    BLOB    NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_item_formats_item ON clipboard_item_formats(item_id);
        "#,
    )
    .map_err(|e| format!("初始化剪贴板表失败: {}", e))?;

    // 迁移：为旧库补充 formats 列（新库已在 CREATE TABLE 中包含）
    let _ = conn.execute(
        "ALTER TABLE clipboard_items ADD COLUMN formats TEXT NOT NULL DEFAULT ''",
        [],
    );

    Ok(())
}

/// 保存条目的附加格式数据（写回剪贴板时使用）
pub fn insert_item_formats(
    conn: &Connection,
    item_id: i64,
    formats: &[(String, Vec<u8>)],
) -> Result<(), String> {
    delete_item_formats(conn, item_id)?;
    for (mime, data) in formats {
        conn.execute(
            "INSERT INTO clipboard_item_formats (item_id, mime, data) VALUES (?1, ?2, ?3)",
            params![item_id, mime, data],
        )
        .map_err(|e| format!("保存剪贴板格式数据失败: {}", e))?;
    }
    Ok(())
}

/// 读取条目的附加格式数据
pub fn get_item_formats(conn: &Connection, item_id: i64) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut stmt = conn
        .prepare("SELECT mime, data FROM clipboard_item_formats WHERE item_id=?1 ORDER BY id")
        .map_err(|e| format!("读取剪贴板格式数据失败: {}", e))?;
    let rows = stmt
        .query_map(params![item_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })
        .map_err(|e| format!("读取剪贴板格式数据失败: {}", e))?;
    let mut v = Vec::new();
    for r in rows {
        if let Ok(x) = r {
            v.push(x);
        }
    }
    Ok(v)
}

/// 删除条目的附加格式数据
pub fn delete_item_formats(conn: &Connection, item_id: i64) -> Result<(), String> {
    conn.execute(
        "DELETE FROM clipboard_item_formats WHERE item_id=?1",
        params![item_id],
    )
    .map_err(|e| format!("删除剪贴板格式数据失败: {}", e))?;
    Ok(())
}

/// 清理孤儿格式数据（条目已被删除的残留行）
pub fn cleanup_orphan_formats(conn: &Connection) -> Result<(), String> {
    conn.execute(
        "DELETE FROM clipboard_item_formats WHERE item_id NOT IN (SELECT id FROM clipboard_items)",
        [],
    )
    .map_err(|e| format!("清理剪贴板格式数据失败: {}", e))?;
    Ok(())
}

/// 解析 DB 中的 formats 字段（JSON 数组字符串）；空串或解析失败返回空数组
fn parse_formats(raw: &str) -> Vec<String> {
    if raw.trim().is_empty() {
        return Vec::new();
    }
    serde_json::from_str(raw).unwrap_or_default()
}

/// 清理孤儿图片文件（DB 中已不存在的图片），防止磁盘无限膨胀
pub fn cleanup_orphan_images(conn: &Connection, img_dir: &std::path::Path) -> Result<(), String> {
    let existing: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT image_path, thumb_path FROM clipboard_items WHERE kind='image'")
            .map_err(|e| format!("查询图片路径失败: {}", e))?;
        let rows = stmt
            .query_map([], |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?)))
            .map_err(|e| format!("读取图片路径失败: {}", e))?;
        let mut v = Vec::new();
        for r in rows {
            if let Ok((a, b)) = r {
                if let Some(x) = a {
                    v.push(x);
                }
                if let Some(x) = b {
                    v.push(x);
                }
            }
        }
        v
    };

    let mut keep = std::collections::HashSet::new();
    for p in &existing {
        keep.insert(p.clone());
    }

    if let Ok(entries) = std::fs::read_dir(img_dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                let rel = format!("images/{}", name);
                if !keep.contains(&rel) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }
    Ok(())
}

/// 读取设置；无记录时返回默认值
pub fn load_settings(conn: &Connection) -> Result<ClipboardSettings, String> {
    let mut s = ClipboardSettings::default();
    let mut stmt = conn
        .prepare("SELECT key, value FROM clipboard_settings")
        .map_err(|e| format!("读取剪贴板设置失败: {}", e))?;
    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(|e| format!("读取剪贴板设置失败: {}", e))?;
    for r in rows {
        if let Ok((k, v)) = r {
            match k.as_str() {
                "enabled" => s.enabled = v == "1" || v == "true",
                "max_items" => {
                    if let Ok(n) = v.parse::<i64>() {
                        s.max_items = n;
                    }
                }
                "store_images" => s.store_images = v == "1" || v == "true",
                "ignore_blank" => s.ignore_blank = v == "1" || v == "true",
                "ignore_short" => s.ignore_short = v == "1" || v == "true",
                _ => {}
            }
        }
    }
    Ok(s)
}

/// 保存设置
pub fn save_settings(conn: &Connection, s: &ClipboardSettings) -> Result<(), String> {
    let pairs = [
        ("enabled", if s.enabled { "1" } else { "0" }),
        ("max_items", &s.max_items.to_string()),
        ("store_images", if s.store_images { "1" } else { "0" }),
        ("ignore_blank", if s.ignore_blank { "1" } else { "0" }),
        ("ignore_short", if s.ignore_short { "1" } else { "0" }),
    ];
    for (k, v) in pairs {
        conn.execute(
            "INSERT INTO clipboard_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![k, v],
        )
        .map_err(|e| format!("保存剪贴板设置失败: {}", e))?;
    }
    Ok(())
}

/// 查询历史列表（支持关键词/类型/置顶过滤）
#[allow(clippy::too_many_arguments)]
pub fn query_items(
    conn: &Connection,
    keyword: &str,
    kind: &str,
    pinned_only: bool,
    limit: i64,
    offset: i64,
) -> Result<Vec<ClipboardItem>, String> {
    let mut sql = String::from(
        "SELECT id, kind, content, image_path, thumb_path, width, height,
                COALESCE(source_app,''), pinned, created_at, COALESCE(formats,'')
         FROM clipboard_items WHERE 1=1",
    );
    match kind {
        "text" => sql.push_str(" AND kind='text'"),
        "image" => sql.push_str(" AND kind='image'"),
        _ => {}
    }
    if pinned_only {
        sql.push_str(" AND pinned=1");
    }
    // 置顶在前，其余按时间倒序。关键词在内存中过滤（content 已加密，不能走 SQL LIKE）。
    sql.push_str(" ORDER BY pinned DESC, created_at DESC");

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("查询剪贴板历史失败: {}", e))?;
    let rows = stmt
        .query_map([], |row| {
            let formats_raw: String = row.get(10)?;
            Ok(ClipboardItem {
                id: row.get(0)?,
                kind: row.get(1)?,
                content: row.get::<_, Option<String>>(2)?.map(decrypt_content),
                image_path: row.get(3)?,
                thumb_path: row.get(4)?,
                width: row.get(5)?,
                height: row.get(6)?,
                source_app: row.get(7)?,
                pinned: row.get::<_, i64>(8)? != 0,
                created_at: row.get(9)?,
                formats: parse_formats(&formats_raw),
            })
        })
        .map_err(|e| format!("读取剪贴板历史失败: {}", e))?;

    let kw = keyword.trim().to_lowercase();
    let mut items: Vec<ClipboardItem> = rows
        .filter_map(|r| r.ok())
        .filter(|it| {
            if kw.is_empty() {
                return true;
            }
            it.content
                .as_deref()
                .map(|c| c.to_lowercase().contains(&kw))
                .unwrap_or(false)
        })
        .collect();
    // 内存分页（上限由 max_items 设置约束，历史量有界）
    let start = offset.max(0) as usize;
    if start >= items.len() {
        items.clear();
    } else {
        let end = (start + limit.max(0) as usize).min(items.len());
        items = items[start..end].to_vec();
    }
    Ok(items)
}

/// 查询总数
pub fn count_items(conn: &Connection, keyword: &str, kind: &str, pinned_only: bool) -> Result<i64, String> {
    let mut sql = String::from("SELECT id, kind, content FROM clipboard_items WHERE 1=1");
    match kind {
        "text" => sql.push_str(" AND kind='text'"),
        "image" => sql.push_str(" AND kind='image'"),
        _ => {}
    }
    if pinned_only {
        sql.push_str(" AND pinned=1");
    }

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("统计剪贴板历史失败: {}", e))?;
    let kw = keyword.trim().to_lowercase();
    let count = stmt
        .query_map([], |row| Ok((row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?)))
        .map_err(|e| format!("统计剪贴板历史失败: {}", e))?
        .filter_map(|r| r.ok())
        .filter(|(kind, content)| {
            if kw.is_empty() {
                return true;
            }
            if kind != "text" {
                return false;
            }
            decrypt_content(content.clone().unwrap_or_default())
                .to_lowercase()
                .contains(&kw)
        })
        .count();
    Ok(count as i64)
}

/// 按 id 查询单条
pub fn get_item(conn: &Connection, id: i64) -> Result<Option<ClipboardItem>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, kind, content, image_path, thumb_path, width, height,
                    COALESCE(source_app,''), pinned, created_at, COALESCE(formats,'')
             FROM clipboard_items WHERE id=?1",
        )
        .map_err(|e| format!("查询剪贴板条目失败: {}", e))?;
    let mut rows = stmt
        .query_map(params![id], |row| {
            let formats_raw: String = row.get(10)?;
            Ok(ClipboardItem {
                id: row.get(0)?,
                kind: row.get(1)?,
                content: row.get::<_, Option<String>>(2)?.map(decrypt_content),
                image_path: row.get(3)?,
                thumb_path: row.get(4)?,
                width: row.get(5)?,
                height: row.get(6)?,
                source_app: row.get(7)?,
                pinned: row.get::<_, i64>(8)? != 0,
                created_at: row.get(9)?,
                formats: parse_formats(&formats_raw),
            })
        })
        .map_err(|e| format!("查询剪贴板条目失败: {}", e))?;
    if let Some(row) = rows.next() {
        Ok(Some(row.map_err(|e| format!("读取剪贴板条目失败: {}", e))?))
    } else {
        Ok(None)
    }
}

/// 按 hash 查询是否已存在，返回 (id, pinned)
pub fn find_by_hash(conn: &Connection, hash: &str) -> Result<Option<(i64, bool)>, String> {
    let mut stmt = conn
        .prepare("SELECT id, pinned FROM clipboard_items WHERE hash=?1 LIMIT 1")
        .map_err(|e| format!("查询剪贴板 hash 失败: {}", e))?;
    let mut rows = stmt
        .query_map(params![hash], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)? != 0)))
        .map_err(|e| format!("查询剪贴板 hash 失败: {}", e))?;
    if let Some(r) = rows.next() {
        Ok(Some(r.map_err(|e| format!("读取剪贴板 hash 失败: {}", e))?))
    } else {
        Ok(None)
    }
}

/// 插入新条目
pub fn insert_item(
    conn: &Connection,
    kind: &str,
    content: Option<&str>,
    image_path: Option<&str>,
    thumb_path: Option<&str>,
    width: i64,
    height: i64,
    source_app: &str,
    hash: &str,
    created_at: i64,
    formats: &[String],
) -> Result<i64, String> {
    let formats_json = serde_json::to_string(formats).unwrap_or_else(|_| "[]".to_string());
    let content = encrypt_content(content);
    conn.execute(
        "INSERT INTO clipboard_items (kind, content, image_path, thumb_path, width, height, source_app, hash, pinned, created_at, formats)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, ?10)",
        params![kind, content, image_path, thumb_path, width, height, source_app, hash, created_at, formats_json],
    )
    .map_err(|e| format!("插入剪贴板条目失败: {}", e))?;
    Ok(conn.last_insert_rowid())
}

/// 删除条目
pub fn delete_item(conn: &Connection, id: i64) -> Result<(), String> {
    conn.execute("DELETE FROM clipboard_items WHERE id=?1", params![id])
        .map_err(|e| format!("删除剪贴板条目失败: {}", e))?;
    Ok(())
}

/// 置顶/取消置顶
pub fn pin_item(conn: &Connection, id: i64, pinned: bool) -> Result<(), String> {
    conn.execute(
        "UPDATE clipboard_items SET pinned=?1 WHERE id=?2",
        params![if pinned { 1 } else { 0 }, id],
    )
    .map_err(|e| format!("置顶剪贴板条目失败: {}", e))?;
    Ok(())
}

/// 清空历史（可指定是否保留置顶）
pub fn clear_items(conn: &Connection, keep_pinned: bool) -> Result<(), String> {
    if keep_pinned {
        conn.execute("DELETE FROM clipboard_items WHERE pinned=0", [])
            .map_err(|e| format!("清空剪贴板历史失败: {}", e))?;
    } else {
        conn.execute("DELETE FROM clipboard_items", [])
            .map_err(|e| format!("清空剪贴板历史失败: {}", e))?;
    }
    Ok(())
}

/// 按上限裁剪历史（保留置顶）
pub fn trim_history(conn: &Connection, max_items: i64) -> Result<(), String> {
    if max_items <= 0 {
        return Ok(());
    }
    // 保留前 max_items 条（置顶优先 + 时间新优先），删除更旧的
    conn.execute(
        "DELETE FROM clipboard_items WHERE pinned=0 AND id NOT IN (
            SELECT id FROM clipboard_items ORDER BY pinned DESC, created_at DESC LIMIT ?1
        )",
        params![max_items],
    )
    .map_err(|e| format!("裁剪剪贴板历史失败: {}", e))?;
    Ok(())
}

/// 忽略规则：全部 app
pub fn list_ignored_apps(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT app FROM clipboard_ignored_apps ORDER BY app")
        .map_err(|e| format!("读取忽略规则失败: {}", e))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| format!("读取忽略规则失败: {}", e))?;
    let mut v = Vec::new();
    for r in rows {
        if let Ok(x) = r {
            v.push(x);
        }
    }
    Ok(v)
}

/// 忽略规则：添加
pub fn add_ignored_app(conn: &Connection, app: &str) -> Result<(), String> {
    conn.execute(
        "INSERT OR IGNORE INTO clipboard_ignored_apps (app) VALUES (?1)",
        params![app],
    )
    .map_err(|e| format!("添加忽略规则失败: {}", e))?;
    Ok(())
}

/// 忽略规则：移除
pub fn remove_ignored_app(conn: &Connection, app: &str) -> Result<(), String> {
    conn.execute("DELETE FROM clipboard_ignored_apps WHERE app=?1", params![app])
        .map_err(|e| format!("移除忽略规则失败: {}", e))?;
    Ok(())
}

/// 忽略规则：判断某 app 是否被忽略
pub fn is_app_ignored(conn: &Connection, app: &str) -> Result<bool, String> {
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(1) FROM clipboard_ignored_apps WHERE lower(app)=lower(?1)",
            params![app],
            |row| row.get(0),
        )
        .map_err(|e| format!("查询忽略规则失败: {}", e))?;
    Ok(n > 0)
}

/// 置顶图片条目引用的图片文件完整路径（清空历史时用于保留）
pub fn pinned_image_paths(conn: &Connection, data_dir: &std::path::Path) -> Result<Vec<std::path::PathBuf>, String> {
    let mut stmt = conn
        .prepare("SELECT image_path, thumb_path FROM clipboard_items WHERE pinned=1 AND kind='image'")
        .map_err(|e| format!("查询置顶图片失败: {}", e))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(|e| format!("查询置顶图片失败: {}", e))?;
    let mut paths = Vec::new();
    for r in rows {
        let (img, thumb) = r.map_err(|e| format!("查询置顶图片失败: {}", e))?;
        for rel in [img, thumb].into_iter().flatten() {
            paths.push(data_dir.join(&rel));
        }
    }
    Ok(paths)
}

/// 删除条目并移除关联图片文件与格式数据
pub fn delete_item_with_files(conn: &Connection, id: i64, data_dir: &std::path::Path) -> Result<(), String> {
    if let Some(item) = get_item(conn, id)? {
        for p in [item.image_path, item.thumb_path].into_iter().flatten() {
            let full = data_dir.join(&p);
            let _ = std::fs::remove_file(full);
        }
    }
    delete_item_formats(conn, id)?;
    delete_item(conn, id)
}

/// 获取图片文件的绝对路径
pub fn image_file_path(data_dir: &std::path::Path, rel: &str) -> PathBuf {
    data_dir.join(rel)
}
