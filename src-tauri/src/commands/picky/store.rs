use super::*;

pub(super) fn insert_bookmark(conn: &rusqlite::Connection, bm: &Bookmark) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO picky_bookmarks
         (id, title, description, url, image_url, favicon_url, created_at, updated_at, refined, meta_fetched, extra, content)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
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
            bm.content,
        ],
    )
    .map_err(|e| format!("写入收藏失败: {}", e))?;
    Ok(())
}

pub(super) fn bookmark_exists(conn: &rusqlite::Connection, id: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM picky_bookmarks WHERE id=?1",
        [id],
        |_| Ok(()),
    )
    .is_ok()
}

pub(super) fn comment_exists(conn: &rusqlite::Connection, id: &str) -> bool {
    conn.query_row("SELECT 1 FROM picky_comments WHERE id=?1", [id], |_| Ok(()))
        .is_ok()
}

pub(super) fn tag_exists(conn: &rusqlite::Connection, id: &str) -> bool {
    conn.query_row("SELECT 1 FROM picky_tags WHERE id=?1", [id], |_| Ok(()))
        .is_ok()
}

pub(super) fn add_binding_if_absent(conn: &rusqlite::Connection, bookmark_id: &str, tag_id: &str) {
    let _ = conn.execute(
        "INSERT OR IGNORE INTO picky_bookmark_tags (bookmark_id, tag_id) VALUES (?1,?2)",
        rusqlite::params![bookmark_id, tag_id],
    );
}
