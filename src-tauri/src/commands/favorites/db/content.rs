use super::*;

/// 写一条正文缓存（同一个条目重复写就覆盖：内容变了要跟新）。
pub fn put_content(
    conn: &Connection,
    favorite_id: i64,
    text: &str,
    html: Option<&str>,
    label: Option<&str>,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO favorite_content (favorite_id, text, html, label, fetched_at) \
         VALUES (?1, ?2, ?3, ?4, ?5) \
         ON CONFLICT(favorite_id) DO UPDATE SET text = ?2, html = ?3, label = ?4, fetched_at = ?5",
        rusqlite::params![favorite_id, text, html, label, now_str()],
    )
    .map_err(|e| format!("缓存正文失败: {}", e))?;
    Ok(())
}

/// 读正文缓存。
pub fn get_content(conn: &Connection, favorite_id: i64) -> Result<Option<CachedContent>, String> {
    let mut statement = conn
        .prepare("SELECT text, label, fetched_at FROM favorite_content WHERE favorite_id = ?1")
        .map_err(|e| format!("读取正文缓存失败: {}", e))?;
    match statement.query_row([favorite_id], |row| {
        Ok(CachedContent {
            text: row.get(0)?,
            label: row.get(1)?,
            fetched_at: row.get(2)?,
        })
    }) {
        Ok(row) => Ok(Some(row)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(format!("读取正文缓存失败: {}", e)),
    }
}

/// 读取某键的断点游标（键形如 `zhihu:<收藏夹id>`）。
///
/// 为什么需要：知乎用户数据接口按自然日配额（默认 100 次/天、未实名 10 次/天），
/// 每次翻页消耗一次。如果每次导入都从第 0 页重来，已导入的页照样烧额度，
/// 长收藏夹永远导不完——断点让每天的配额全部花在新内容上。
pub fn get_import_cursor(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    let mut statement = conn
        .prepare("SELECT cursor FROM favorite_import_state WHERE source = ?1")
        .map_err(|e| format!("读取导入断点失败: {}", e))?;
    match statement.query_row([key], |row| row.get::<_, Option<String>>(0)) {
        Ok(cursor) => Ok(cursor.filter(|c| !c.is_empty())),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(format!("读取导入断点失败: {}", e)),
    }
}

/// 写入 / 清除断点游标（`None` = 该键已导完）。
pub fn set_import_cursor(conn: &Connection, key: &str, cursor: Option<&str>) -> Result<(), String> {
    conn.execute(
        "INSERT INTO favorite_import_state (source, cursor, last_run_at, total) \
         VALUES (?1, ?2, ?3, 0) \
         ON CONFLICT(source) DO UPDATE SET cursor = ?2, last_run_at = ?3",
        rusqlite::params![key, cursor, now_str()],
    )
    .map_err(|e| format!("保存导入断点失败: {}", e))?;
    Ok(())
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

/// 写入并返回条目 id（正文缓存要按 id 挂，而 upsert 本身不返回 id）。
pub fn upsert_with_id(
    conn: &Connection,
    item: &NewFavorite,
) -> Result<(UpsertOutcome, i64), String> {
    let outcome = upsert(conn, item)?;
    // 命中墓碑：条目没进库，也就没有 id 可查（更不该去缓存正文）
    if outcome == UpsertOutcome::Deleted {
        return Ok((outcome, 0));
    }
    let id = conn
        .query_row(
            "SELECT id FROM favorite WHERE source = ?1 AND external_id = ?2",
            rusqlite::params![item.source, item.external_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|e| format!("读取条目 id 失败: {}", e))?;
    Ok((outcome, id))
}
