use super::*;

// ─── 云同步合并（合并语义：LWW 按 updatedAt 后写覆盖，绝不丢数据） ───
// 收藏/评论：本地缺失 → 插入；已存在且云端 updatedAt 更新 → 覆盖本地（后写优先）。
// 标签：按 id 只补缺失（避免覆盖本地改名）。标签关联：幂等补充。

// ─── 删除墓碑（tombstone）───
//
// 云同步的合并语义是「并集 + LWW」：本地没有的条目就从云端补进来。
// 这条规则本身没错，但它天生压过删除 —— 删掉 A 之后下一次同步，云端（还没收到删除）
// 会把 A 原样补回来，于是出现「删 A，再删 B，A 又回来了；再删 C，A、B 都在」。
// 删除必须留下痕迹，云端数据只有在**比删除更晚被改过**时才准它复活。

pub(super) const ENT_BOOKMARK: &str = "bookmark";
pub(super) const ENT_COMMENT: &str = "comment";
pub(super) const ENT_TAG: &str = "tag";
/// 墓碑保留时长：留够久，避免另一端（Flutter 端 / 另一台机器）隔很久才同步、
/// 又把旧数据带回来。到期清理只是不想让这张表无限膨胀。
pub(super) const TOMBSTONE_RETENTION_DAYS: i64 = 90;

/// 记下「这些 id 是被我们删掉的」。删除命令里调用，必须在写库同一趟里完成。
pub(super) fn record_tombstones(
    conn: &rusqlite::Connection,
    entity: &str,
    ids: &[String],
) -> Result<(), String> {
    if ids.is_empty() {
        return Ok(());
    }
    let now = now_iso();
    for id in ids {
        conn.execute(
            "INSERT OR REPLACE INTO picky_tombstones (entity, id, deleted_at) VALUES (?1,?2,?3)",
            rusqlite::params![entity, id, now],
        )
        .map_err(|e| format!("记录删除墓碑失败: {}", e))?;
    }
    Ok(())
}

/// 该 id 是否曾被我们删掉；返回删除时间。
pub(super) fn tombstone_time(
    conn: &rusqlite::Connection,
    entity: &str,
    id: &str,
) -> Option<String> {
    conn.query_row(
        "SELECT deleted_at FROM picky_tombstones WHERE entity=?1 AND id=?2",
        rusqlite::params![entity, id],
        |r| r.get(0),
    )
    .ok()
}

pub(super) fn clear_tombstone(
    conn: &rusqlite::Connection,
    entity: &str,
    id: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM picky_tombstones WHERE entity=?1 AND id=?2",
        rusqlite::params![entity, id],
    )
    .map_err(|e| format!("清理删除墓碑失败: {}", e))?;
    Ok(())
}

/// 清理过期墓碑（超过保留期的老删除记录，留着只会让表变大）。
pub(super) fn prune_tombstones(conn: &rusqlite::Connection) -> usize {
    let cutoff =
        (chrono::Utc::now() - chrono::Duration::days(TOMBSTONE_RETENTION_DAYS)).to_rfc3339();
    conn.execute(
        "DELETE FROM picky_tombstones WHERE deleted_at < ?1",
        [&cutoff],
    )
    .unwrap_or(0)
}

#[derive(Debug, Clone, Default)]
pub(super) struct MergeCounts {
    pub(super) added_bookmarks: usize,
    pub(super) updated_bookmarks: usize,
    pub(super) added_comments: usize,
    pub(super) updated_comments: usize,
    pub(super) added_tags: usize,
    /// 因本地已删除而被跳过的云端条目数（没有墓碑机制时它们会被「复活」）
    pub(super) skipped_deleted: usize,
}

/// 宽松解析 ISO 时间（rfc3339 / 无时区微秒两种格式，兼容 Flutter toIso8601String）。
pub(super) fn parse_iso_utc(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&chrono::Utc));
    }
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f")
        .ok()
        .map(|n| n.and_utc())
}

/// 判断 incoming 是否比本地更新（按 updatedAt；解析失败时退化为字符串比较）。
pub(super) fn incoming_is_newer(incoming: &str, local: &str) -> bool {
    match (parse_iso_utc(incoming), parse_iso_utc(local)) {
        (Some(a), Some(b)) => a > b,
        _ => incoming > local,
    }
}

pub(super) fn insert_comment(conn: &rusqlite::Connection, c: &Comment) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO picky_comments (id, bookmark_id, content, created_at, updated_at, parent_id)
         VALUES (?1,?2,?3,?4,?5,?6)",
        rusqlite::params![c.id, c.bookmark_id, c.content, c.created_at, c.updated_at, c.parent_id],
    )
    .map_err(|e| format!("写入评论失败: {}", e))?;
    Ok(())
}

pub(super) fn merge_cloud_state(
    conn: &rusqlite::Connection,
    state: &JsonValue,
) -> Result<MergeCounts, String> {
    let mut counts = MergeCounts::default();
    let mut parse_failures = 0usize;

    if let Some(arr) = state.get("bookmarks").and_then(|v| v.as_array()) {
        for item in arr {
            match serde_json::from_value::<Bookmark>(item.clone()) {
                Ok(bm) => {
                    // 墓碑优先：本地删过 → 只有云端在这之后又改过才算「另一端重新加回来」。
                    if let Some(deleted_at) = tombstone_time(conn, ENT_BOOKMARK, &bm.id) {
                        if !incoming_is_newer(&bm.updated_at, &deleted_at) {
                            counts.skipped_deleted += 1;
                            continue;
                        }
                        clear_tombstone(conn, ENT_BOOKMARK, &bm.id)?;
                    }
                    let local_updated: Option<String> = conn
                        .query_row(
                            "SELECT updated_at FROM picky_bookmarks WHERE id=?1",
                            [&bm.id],
                            |r| r.get(0),
                        )
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
                Err(e) => {
                    // 解析失败必须上报并中止同步，绝不允许"云端有数据但本地解析成 0 条"
                    // 再上传空数据把云端覆盖清空（曾经因此丢过数据）。
                    parse_failures += 1;
                    crate::exit_log::exit_log(&format!("[picky-sync] 解析云端书签失败: {}", e));
                }
            }
        }
    }
    if parse_failures > 0 {
        return Err(format!(
            "云端快照中有 {} 条书签解析失败，已中止同步以防覆盖云端数据。请检查云端 state.json 格式。",
            parse_failures
        ));
    }
    if let Some(arr) = state.get("comments").and_then(|v| v.as_array()) {
        for item in arr {
            if let Ok(c) = serde_json::from_value::<Comment>(item.clone()) {
                // 评论自身被删过 → 同上；所属收藏被删过 → 也不要把孤儿评论补回来。
                if let Some(deleted_at) = tombstone_time(conn, ENT_COMMENT, &c.id) {
                    if !incoming_is_newer(&c.updated_at, &deleted_at) {
                        counts.skipped_deleted += 1;
                        continue;
                    }
                    clear_tombstone(conn, ENT_COMMENT, &c.id)?;
                }
                if tombstone_time(conn, ENT_BOOKMARK, &c.bookmark_id).is_some() {
                    counts.skipped_deleted += 1;
                    continue;
                }
                let local_updated: Option<String> = conn
                    .query_row(
                        "SELECT updated_at FROM picky_comments WHERE id=?1",
                        [&c.id],
                        |r| r.get(0),
                    )
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
                // 标签没有 updatedAt：本地删过就永远是删除优先，不复活。
                if tombstone_time(conn, ENT_TAG, &t.id).is_some() {
                    counts.skipped_deleted += 1;
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
                        // 收藏或标签任一方已被本地删除 → 这条关联也不该补回来
                        if tombstone_time(conn, ENT_BOOKMARK, bid).is_some()
                            || tombstone_time(conn, ENT_TAG, t).is_some()
                        {
                            continue;
                        }
                        add_binding_if_absent(conn, bid, t);
                    }
                }
            }
        }
    }
    Ok(counts)
}

pub(super) fn format_merge_summary(counts: &MergeCounts, prefix: &str) -> String {
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
    if counts.skipped_deleted > 0 {
        parts.push(format!("忽略已删除 {}", counts.skipped_deleted));
    }
    if parts.is_empty() {
        format!("{}（无变化）", prefix)
    } else {
        format!("{}：{}", prefix, parts.join("，"))
    }
}
