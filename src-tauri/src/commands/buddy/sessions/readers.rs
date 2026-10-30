use super::*;

/// 会话活动时间：取 `last_activity_at` / `updated_at` / `created_at` 中**最新**者。
///
/// 真机数据（WorkBuddy 5.x）显示 `updated_at` 恒 ≥ `last_activity_at`
/// （客户端刷新会 bump updated_at），取最新者才符合「最近活跃」的直觉；
/// 缺失列（老库）自动退化为 updated_at / created_at。
pub(super) fn activity_at(raw: &RawSession) -> Option<i64> {
    [raw.last_activity_at, raw.updated_at, raw.created_at]
        .into_iter()
        .flatten()
        .max()
}

/// 表是否含某列（老版本 `workbuddy.db` 没有 `last_activity_at`，直接写进 SQL 会报错）。
pub(super) fn table_has_column(conn: &Connection, table: &str, column: &str) -> bool {
    let sql = format!("PRAGMA table_info({})", table);
    let Ok(mut stmt) = conn.prepare(&sql) else {
        return false;
    };
    let Ok(mut rows) = stmt.query([]) else {
        return false;
    };
    while let Ok(Some(row)) = rows.next() {
        if row
            .get::<_, String>(1)
            .map(|name| name == column)
            .unwrap_or(false)
        {
            return true;
        }
    }
    false
}

/// WorkBuddy 会话库路径：`~/.workbuddy/workbuddy.db`
pub(super) fn workbuddy_sessions_db_path() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
    Ok(home.join(".workbuddy").join("workbuddy.db"))
}

/// CodeBuddy CN 会话库路径：`{data_dir}/codebuddy-sessions.vscdb`
pub(super) fn codebuddy_cn_sessions_db_path() -> Result<PathBuf, String> {
    let data_dir = crate::commands::buddy::codebuddy_cn::default_data_dir()
        .ok_or_else(|| "无法定位 CodeBuddy CN 数据目录".to_string())?;
    Ok(data_dir.join("codebuddy-sessions.vscdb"))
}

/// 读取 WorkBuddy `workbuddy.db` 的 sessions 表
pub(super) fn read_sessions_from_workbuddy_db(db_path: &Path) -> Vec<RawSession> {
    if !db_path.exists() {
        return Vec::new();
    }

    let conn =
        match Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) {
            Ok(c) => c,
            Err(e) => {
                eprintln!(
                    "[BuddySession] 打开 WorkBuddy db 失败 {}: {}",
                    db_path.display(),
                    e
                );
                return Vec::new();
            }
        };

    // 老版本库没有 last_activity_at（5.x 才有），缺列时用 NULL 占位，排序退化为 updated_at/created_at
    let has_last_activity = table_has_column(&conn, "sessions", "last_activity_at");
    let activity_column = if has_last_activity {
        "last_activity_at"
    } else {
        "NULL"
    };
    let sql = format!(
        "SELECT id, cwd, user_id, title, status, created_at, updated_at, deleted_at, is_playground, {} \
         FROM sessions",
        activity_column
    );

    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "[BuddySession] 准备 WorkBuddy 查询失败 {}: {}",
                db_path.display(),
                e
            );
            return Vec::new();
        }
    };

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<i64>>(5)?,
            row.get::<_, Option<i64>>(6)?,
            row.get::<_, Option<i64>>(7)?,
            row.get::<_, i64>(8).unwrap_or(0),
            row.get::<_, Option<i64>>(9)?,
        ))
    });

    let mut sessions = Vec::new();
    if let Ok(rows) = rows {
        for row in rows.flatten() {
            let (
                id,
                cwd,
                user_id,
                title,
                status,
                created_at,
                updated_at,
                deleted_at,
                is_play,
                last_activity_at,
            ) = row;
            sessions.push(RawSession {
                conversation_id: id,
                title: title.unwrap_or_default(),
                cwd: cwd.unwrap_or_default(),
                user_id: user_id.unwrap_or_default(),
                status: status.unwrap_or_default(),
                created_at,
                updated_at,
                last_activity_at,
                is_deleted: deleted_at.is_some(),
                is_playground: is_play != 0,
            });
        }
    }

    sessions
}

/// 收集扩展数据目录中「本机仍有正文」的会话 id。
///
/// 判定与参考实现一致：工作区 `index.json` 登记了该会话，**且** `<workspace>/<conversationId>/` 目录存在。
/// 返回 `None` 表示本机没有可识别的旧版目录布局（5.x 会话只落 DB、或全新装机），
/// 此时调用方跳过「正文缺失」标记，避免把正常会话全标成缺失。
pub(crate) fn collect_local_conversation_ids_from_roots(
    roots: &[PathBuf],
    uid: &str,
) -> Option<HashSet<String>> {
    if crate::commands::buddy::session_transfer::codebuddy::validate_uid(uid).is_err() {
        return None;
    }
    let mut ids = HashSet::new();
    let mut layout_found = false;
    for root in roots {
        let account_outer = root.join(uid);
        let Ok(ide_entries) = std::fs::read_dir(&account_outer) else {
            continue;
        };
        for ide_entry in ide_entries.flatten() {
            let Ok(metadata) = std::fs::symlink_metadata(ide_entry.path()) else {
                continue;
            };
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                continue;
            }
            let history_root = ide_entry.path().join(uid).join("history");
            if !history_root.is_dir() {
                continue;
            }
            layout_found = true;
            let Ok(workspace_entries) = std::fs::read_dir(&history_root) else {
                continue;
            };
            for workspace_entry in workspace_entries.flatten() {
                let Ok(metadata) = std::fs::symlink_metadata(workspace_entry.path()) else {
                    continue;
                };
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    continue;
                }
                let workspace_path = workspace_entry.path();
                let Ok(index) =
                    crate::commands::buddy::session_transfer::codebuddy::read_workspace_index(
                        &workspace_path.join("index.json"),
                    )
                else {
                    continue;
                };
                let Some(conversations) = index.get("conversations").and_then(Value::as_array)
                else {
                    continue;
                };
                for conversation in conversations {
                    let Some(id) = conversation.get("id").and_then(Value::as_str) else {
                        continue;
                    };
                    if workspace_path.join(id).is_dir() {
                        ids.insert(id.to_string());
                    }
                }
            }
        }
    }
    layout_found.then_some(ids)
}

/// 标注哪些会话在本机已经没有正文（`contentMissing`）。`local` 为 `None` 时不动任何记录。
pub(super) fn apply_content_probe(
    records: &mut [BuddySessionRecord],
    local: Option<&HashSet<String>>,
) {
    let Some(local) = local else {
        return;
    };
    for record in records.iter_mut() {
        record.content_missing = !local.contains(&record.conversation_id);
    }
}

/// 读取 vscdb 中的会话 JSON（ItemTable key 以 `session:` 开头）
pub(super) fn read_sessions_from_db(db_path: &Path) -> Vec<RawSession> {
    if !db_path.exists() {
        return Vec::new();
    }

    let conn =
        match Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[BuddySession] 打开 {} 失败: {}", db_path.display(), e);
                return Vec::new();
            }
        };

    let mut stmt = match conn.prepare("SELECT value FROM ItemTable WHERE key LIKE 'session:%'") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[BuddySession] 准备查询失败 {}: {}", db_path.display(), e);
            return Vec::new();
        }
    };

    let rows = stmt.query_map([], |row| {
        let value: String = row.get(0)?;
        Ok(value)
    });

    let mut sessions = Vec::new();
    if let Ok(rows) = rows {
        for row in rows.flatten() {
            if let Some(s) = parse_raw_session(&row) {
                sessions.push(s);
            }
        }
    }

    sessions
}

pub(super) fn parse_raw_session(json_str: &str) -> Option<RawSession> {
    let v: Value = serde_json::from_str(json_str).ok()?;

    let conversation_id = v.get("conversationId")?.as_str()?.to_string();
    let title = v
        .get("title")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();
    let cwd = v
        .get("cwd")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();
    let user_id = v
        .get("userId")
        .and_then(|u| u.as_str())
        .unwrap_or("")
        .to_string();
    let status = v
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("Unknown")
        .to_string();
    let created_at = v.get("createdAt").and_then(|t| t.as_i64());
    let updated_at = v.get("updatedAt").and_then(|t| t.as_i64());
    let last_activity_at = v.get("lastActivityAt").and_then(|t| t.as_i64());
    let is_deleted = v.get("deletedAt").and_then(|t| t.as_i64()).is_some();
    let is_playground = v
        .get("isPlayground")
        .and_then(|p| p.as_bool())
        .unwrap_or(false);

    Some(RawSession {
        conversation_id,
        title,
        cwd,
        user_id,
        status,
        created_at,
        updated_at,
        last_activity_at,
        is_deleted,
        is_playground,
    })
}
