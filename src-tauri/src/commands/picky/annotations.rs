use super::*;

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
        rusqlite::params![
            c.id,
            c.bookmark_id,
            c.content,
            c.created_at,
            c.updated_at,
            c.parent_id
        ],
    )
    .map_err(|e| format!("写入评论失败: {}", e))?;
    schedule_auto_sync();
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
    schedule_auto_sync();
    Ok(())
}

/// 删除评论（连同其子回复）。
#[tauri::command]
pub fn picky_delete_comment(id: String) -> Result<(), String> {
    let conn = open_db()?;
    // 子回复一并删，墓碑也要一并记（漏记就会在下一次同步被补回来）
    let ids: Vec<String> = conn
        .prepare("SELECT id FROM picky_comments WHERE id=?1 OR parent_id=?1")
        .and_then(|mut s| {
            s.query_map([&id], |r| r.get(0))
                .and_then(|rows| rows.collect::<Result<Vec<String>, _>>())
        })
        .map_err(|e| format!("读取待删评论失败: {}", e))?;
    record_tombstones(&conn, ENT_COMMENT, &ids)?;
    conn.execute(
        "DELETE FROM picky_comments WHERE id=?1 OR parent_id=?1",
        [&id],
    )
    .map_err(|e| format!("删除评论失败: {}", e))?;
    schedule_auto_sync();
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
        .query_row("SELECT id FROM picky_tags WHERE name=?1", [&name], |r| {
            r.get(0)
        })
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
    schedule_auto_sync();
    Ok(t)
}

#[tauri::command]
pub fn picky_delete_tag(id: String) -> Result<(), String> {
    let conn = open_db()?;
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    record_tombstones(&tx, ENT_TAG, &[id.clone()])?;
    tx.execute("DELETE FROM picky_tags WHERE id=?1", [&id])
        .map_err(|e| format!("删除标签失败: {}", e))?;
    tx.execute("DELETE FROM picky_bookmark_tags WHERE tag_id=?1", [&id])
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    schedule_auto_sync();
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
        schedule_auto_sync();
        Ok(false)
    } else {
        add_binding_if_absent(&conn, &bookmark_id, &tag_id);
        schedule_auto_sync();
        Ok(true)
    }
}
