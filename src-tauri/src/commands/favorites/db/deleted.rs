use super::*;

/// 墓碑键：默认就是 `external_id`；书签来源的 external_id 是 **URL**，
/// 多一个 `#片段`、末尾斜杠或 http/https 之差就会漏碑，因此统一归一化。
pub(super) fn tombstone_key(source: &str, external_id: &str) -> String {
    if source != "bookmark" {
        return external_id.to_string();
    }
    let trimmed = external_id.trim();
    // 去 fragment
    let no_frag = trimmed.split('#').next().unwrap_or(trimmed);
    // scheme/host 统一小写，末尾斜杠去掉（查询串保留：它通常是 URL 的一部分）
    match no_frag.split_once("://") {
        Some((scheme, rest)) => {
            let rest = rest.trim_end_matches('/');
            format!("{}://{}", scheme.to_lowercase(), rest.to_lowercase())
        }
        None => no_frag.trim_end_matches('/').to_lowercase(),
    }
}

/// 记一条墓碑（同一条重复删除时刷新时间与标题）。
pub fn mark_deleted(
    conn: &Connection,
    source: &str,
    external_id: &str,
    title: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO favorite_deleted (source, external_id, title, deleted_at) \
         VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(source, external_id) DO UPDATE SET title = ?3, deleted_at = ?4",
        rusqlite::params![source, tombstone_key(source, external_id), title, now_str()],
    )
    .map_err(|e| format!("记录删除墓碑失败: {}", e))?;
    Ok(())
}

/// 是否被用户删过（导入前查，命中就跳过）。
pub fn is_deleted(conn: &Connection, source: &str, external_id: &str) -> Result<bool, String> {
    let hit: i64 = conn
        .query_row(
            "SELECT 1 FROM favorite_deleted WHERE source = ?1 AND external_id = ?2",
            rusqlite::params![source, tombstone_key(source, external_id)],
            |_| Ok(1),
        )
        .unwrap_or(0);
    Ok(hit == 1)
}

pub fn list_deleted(conn: &Connection) -> Result<Vec<FavoriteDeletedRow>, String> {
    let mut stmt = conn
        .prepare("SELECT source, external_id, title, deleted_at FROM favorite_deleted ORDER BY deleted_at DESC")
        .map_err(|e| format!("读取删除记录失败: {}", e))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(FavoriteDeletedRow {
                source: row.get(0)?,
                external_id: row.get(1)?,
                title: row.get(2)?,
                deleted_at: row.get(3)?,
            })
        })
        .map_err(|e| format!("读取删除记录失败: {}", e))?;
    let mut out = Vec::new();
    for r in rows.flatten() {
        out.push(r);
    }
    Ok(out)
}

/// 清墓碑 → 这些条目下次导入会重新进来。
/// `source` / `external_id` 都给 = 只恢复一条；只给 source = 恢复该来源全部；
/// 都不给 = 全部恢复。返回清除的条数。
pub fn clear_deleted(
    conn: &Connection,
    source: Option<&str>,
    external_id: Option<&str>,
) -> Result<usize, String> {
    let cleared = match (source, external_id) {
        (Some(s), Some(e)) => conn
            .execute(
                "DELETE FROM favorite_deleted WHERE source = ?1 AND external_id = ?2",
                rusqlite::params![s, tombstone_key(s, e)],
            )
            .map_err(|e| format!("清除删除记录失败: {}", e))?,
        (Some(s), None) => conn
            .execute(
                "DELETE FROM favorite_deleted WHERE source = ?1",
                rusqlite::params![s],
            )
            .map_err(|e| format!("清除删除记录失败: {}", e))?,
        (None, _) => conn
            .execute("DELETE FROM favorite_deleted", [])
            .map_err(|e| format!("清除删除记录失败: {}", e))?,
    };
    Ok(cleared)
}
