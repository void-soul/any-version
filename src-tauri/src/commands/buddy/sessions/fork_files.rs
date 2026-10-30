use super::*;

/// CodeBuddy CN 分叉：复制 vscdb 的会话索引 + 扩展目录里的会话正文。
pub(super) fn fork_codebuddy_cn_session(
    src_id: &str,
    new_id: &str,
    title: Option<&str>,
) -> Result<BuddySessionForkReport, String> {
    let data_dir = crate::commands::buddy::codebuddy_cn::default_data_dir()
        .ok_or_else(|| "无法定位 CodeBuddy CN 数据目录".to_string())?;
    let db_path = data_dir.join("codebuddy-sessions.vscdb");
    if !db_path.is_file() {
        return Err("未找到 CodeBuddy CN 会话库（codebuddy-sessions.vscdb）".to_string());
    }
    crate::commands::buddy::session_transfer::codebuddy::reject_symlink_if_exists(&db_path)?;
    let mut conn =
        Connection::open(&db_path).map_err(|e| format!("打开 CodeBuddy CN 会话库失败: {}", e))?;
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(|e| format!("设置数据库超时失败: {}", e))?;

    let key = format!("session:{}", src_id);
    let raw: String = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = ?1",
            rusqlite::params![key],
            |row| row.get(0),
        )
        .map_err(|_| format!("会话不存在: {}", src_id))?;
    let mut session: Value =
        serde_json::from_str(&raw).map_err(|e| format!("解析会话失败: {}", e))?;
    let obj = session
        .as_object_mut()
        .ok_or_else(|| "会话格式异常（不是对象）".to_string())?;
    let src_title = obj
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let new_title = fork_title(&src_title, title);
    let now = now_ms();
    obj.insert(
        "conversationId".to_string(),
        Value::String(new_id.to_string()),
    );
    if obj.contains_key("id") {
        obj.insert("id".to_string(), Value::String(new_id.to_string()));
    }
    obj.insert("title".to_string(), Value::String(new_title.clone()));
    obj.remove("deletedAt");
    for k in ["createdAt", "updatedAt", "lastActivityAt"] {
        if obj.contains_key(k) {
            obj.insert(k.to_string(), Value::from(now));
        }
    }
    let serialized =
        serde_json::to_string(&session).map_err(|e| format!("序列化会话失败: {}", e))?;
    {
        let tx = conn
            .transaction()
            .map_err(|e| format!("开启数据库事务失败: {}", e))?;
        tx.execute(
            "INSERT OR REPLACE INTO ItemTable(key, value) VALUES(?1, ?2)",
            rusqlite::params![format!("session:{}", new_id), serialized],
        )
        .map_err(|e| format!("写入新会话记录失败: {}", e))?;
        tx.commit()
            .map_err(|e| format!("提交新会话记录失败: {}", e))?;
    }

    // 正文：扩展目录 `<uid>/<IDE>/<uid>/history/<workspace>/<id>/`
    let mut files_copied = 0usize;
    let mut errors: Vec<String> = Vec::new();
    let uid = current_platform_uid(crate::commands::buddy::models::BuddyPlatform::CodebuddyCn);
    let extension_dir =
        crate::commands::buddy::session_transfer::codebuddy::codebuddy_extension_data_dir().ok();
    if let (Some(uid), Some(extension_dir)) = (uid.as_deref(), extension_dir.as_deref()) {
        copy_session_files_in_extension(
            extension_dir,
            uid,
            src_id,
            new_id,
            &mut files_copied,
            &mut errors,
        );
    } else {
        errors.push("未定位到扩展数据目录，只复制了会话索引".to_string());
    }

    Ok(BuddySessionForkReport {
        conversation_id: new_id.to_string(),
        title: new_title,
        files_copied,
        content_missing: files_copied == 0,
        errors,
    })
}

/// 在扩展目录里复制会话正文：会话目录 + 5 类辅助目录，并向工作区 index.json 登记新会话。
/// 结构与 `delete_session_files_in_extension` 对称（同一套遍历规则）。
pub(super) fn copy_session_files_in_extension(
    extension_data_dir: &Path,
    uid: &str,
    src_id: &str,
    new_id: &str,
    files_copied: &mut usize,
    errors: &mut Vec<String>,
) {
    use crate::commands::buddy::session_transfer::codebuddy as transfer;
    if transfer::validate_uid(uid).is_err() {
        errors.push(format!("uid 不安全，跳过正文复制: {}", uid));
        return;
    }
    let account_outer = extension_data_dir.join(uid);
    if !account_outer.is_dir() {
        return;
    }
    let Ok(ide_entries) = std::fs::read_dir(&account_outer) else {
        return;
    };
    for ide_entry in ide_entries.flatten() {
        let Ok(metadata) = std::fs::symlink_metadata(ide_entry.path()) else {
            continue;
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            continue;
        }
        let account_root = ide_entry.path().join(uid);
        let history_root = account_root.join("history");
        if !history_root.is_dir() {
            continue;
        }
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
            let index_path = workspace_path.join("index.json");
            let Ok(index) = transfer::read_workspace_index(&index_path) else {
                continue;
            };
            let Some(conversations) = index.get("conversations").and_then(Value::as_array) else {
                continue;
            };
            let Some(src_entry) = conversations
                .iter()
                .find(|c| c.get("id").and_then(Value::as_str) == Some(src_id))
            else {
                continue;
            };

            // 目录本体 + 辅助目录
            let conv_dir = workspace_path.join(src_id);
            if conv_dir.is_dir() {
                match copy_dir_recursive(&conv_dir, &workspace_path.join(new_id)) {
                    Ok(n) => *files_copied += n,
                    Err(e) => errors.push(format!("复制会话目录失败: {}", e)),
                }
            }
            for kind in transfer::AUXILIARY_KINDS {
                let aux_src = account_root
                    .join(kind)
                    .join(workspace_entry.file_name())
                    .join(src_id);
                if aux_src.is_dir() {
                    let aux_dst = account_root
                        .join(kind)
                        .join(workspace_entry.file_name())
                        .join(new_id);
                    if let Err(e) = copy_dir_recursive(&aux_src, &aux_dst) {
                        errors.push(format!("复制辅助目录失败 {}: {}", kind, e));
                    }
                }
            }

            // index.json：登记新会话（沿用源条目属性，只换 id 与标题），current 不动
            let mut new_entry = src_entry.clone();
            if let Some(obj) = new_entry.as_object_mut() {
                obj.insert("id".to_string(), Value::String(new_id.to_string()));
                if obj.contains_key("title") {
                    obj.insert(
                        "title".to_string(),
                        Value::String(fork_title(
                            obj.get("title").and_then(Value::as_str).unwrap_or_default(),
                            None,
                        )),
                    );
                }
            }
            let mut new_index = index.clone();
            if let Some(array) = new_index
                .get_mut("conversations")
                .and_then(Value::as_array_mut)
            {
                array.push(new_entry);
            }
            match serde_json::to_string_pretty(&new_index)
                .map_err(|e| format!("序列化工作区索引失败: {}", e))
                .and_then(|s| crate::commands::buddy::store::write_atomic(&index_path, &s))
            {
                Ok(()) => {}
                Err(e) => errors.push(e),
            }
        }
    }
}

/// 递归复制目录（跳过符号链接，返回复制的文件数）。
pub(super) fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<usize, String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("创建目录失败 {}: {}", dst.display(), e))?;
    let mut count = 0usize;
    for entry in std::fs::read_dir(src)
        .map_err(|e| format!("读取目录失败 {}: {}", src.display(), e))?
        .flatten()
    {
        let Ok(metadata) = std::fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        let target = dst.join(entry.file_name());
        if metadata.is_dir() {
            count += copy_dir_recursive(&entry.path(), &target)?;
        } else {
            std::fs::copy(&entry.path(), &target)
                .map_err(|e| format!("复制文件失败 {}: {}", entry.path().display(), e))?;
            count += 1;
        }
    }
    Ok(count)
}
