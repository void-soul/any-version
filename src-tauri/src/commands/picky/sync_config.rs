use super::*;

// ─── 敏感字段加密（复用 commands::secrets，与 cert 模块同一主密钥） ───
use crate::commands::secrets::{decrypt_secret, encrypt_secret};

// ─── 同步配置读写 ───

pub(super) fn config_from_row(row: &rusqlite::Row) -> rusqlite::Result<PickySyncConfig> {
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

pub(super) fn load_sync_config(conn: &rusqlite::Connection) -> PickySyncConfig {
    conn.query_row("SELECT * FROM picky_sync_config WHERE id=1", [], |row| {
        config_from_row(row)
    })
    .unwrap_or_default()
}

pub(super) fn save_sync_config_db(
    conn: &rusqlite::Connection,
    cfg: &PickySyncConfig,
) -> Result<(), String> {
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

pub(super) fn state_to_json(conn: &rusqlite::Connection) -> Result<PickyState, String> {
    let mut bookmarks = Vec::new();
    {
        let mut stmt = conn
            .prepare("SELECT id,title,description,url,image_url,favicon_url,created_at,updated_at,refined,meta_fetched,extra,content FROM picky_bookmarks")
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
            .prepare(
                "SELECT id,bookmark_id,content,created_at,updated_at,parent_id FROM picky_comments",
            )
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
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        for r in rows {
            let (bid, tid) = r.map_err(|e| e.to_string())?;
            bookmark_tags.entry(bid).or_default().push(tid);
        }
    }
    Ok(PickyState {
        bookmarks,
        comments,
        tags,
        bookmark_tags,
    })
}
