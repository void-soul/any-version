use super::*;

/// `codebuddy-sessions.vscdb` 的 `session:%` 记录 userId 重映射。
/// WorkBuddy 旧版 vscdb 合并也复用本函数。
pub(in crate::commands::buddy::session_transfer) fn remap_session_vscdb_user_id(
    db_path: &Path,
    source_uid: &str,
    target_uid: &str,
    backup_root: &Path,
) -> Result<usize, String> {
    if !db_path.is_file() {
        return Ok(0);
    }
    reject_symlink_if_exists(db_path)?;
    let mut connection = rusqlite::Connection::open(db_path).map_err(|e| {
        format!(
            "打开 CodeBuddy CN 会话数据库失败: path={}, error={}",
            db_path.display(),
            e
        )
    })?;
    connection
        .busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| format!("设置 CodeBuddy CN 会话数据库超时失败: {}", e))?;

    let mut updates = Vec::new();
    {
        let mut statement = connection
            .prepare("SELECT key, value FROM ItemTable WHERE key LIKE 'session:%'")
            .map_err(|e| format!("读取 CodeBuddy CN 会话数据库失败: {}", e))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("查询 CodeBuddy CN 会话数据库失败: {}", e))?;
        for row in rows {
            let (key, raw_value) =
                row.map_err(|e| format!("读取 CodeBuddy CN 会话记录失败: {}", e))?;
            let Ok(mut value) = serde_json::from_str::<Value>(&raw_value) else {
                continue;
            };
            let Some(object) = value.as_object_mut() else {
                continue;
            };
            if object.get("userId").and_then(Value::as_str) != Some(source_uid) {
                continue;
            }
            object.insert("userId".to_string(), Value::String(target_uid.to_string()));
            let serialized = serde_json::to_string(&value)
                .map_err(|e| format!("序列化 CodeBuddy CN 会话记录失败: {}", e))?;
            updates.push((key, serialized));
        }
    }
    if updates.is_empty() {
        return Ok(0);
    }

    std::fs::create_dir_all(backup_root).map_err(|e| {
        format!(
            "创建 CodeBuddy CN 会话数据库备份目录失败: path={}, error={}",
            backup_root.display(),
            e
        )
    })?;
    std::fs::copy(db_path, backup_root.join("codebuddy-sessions.vscdb")).map_err(|e| {
        format!(
            "备份 CodeBuddy CN 会话数据库失败: path={}, error={}",
            db_path.display(),
            e
        )
    })?;

    let transaction = connection
        .transaction()
        .map_err(|e| format!("开启 CodeBuddy CN 会话数据库事务失败: {}", e))?;
    for (key, value) in &updates {
        transaction
            .execute(
                "UPDATE ItemTable SET value = ?1 WHERE key = ?2",
                rusqlite::params![value, key],
            )
            .map_err(|e| format!("更新 CodeBuddy CN 会话 userId 失败: {}", e))?;
    }
    transaction
        .commit()
        .map_err(|e| format!("提交 CodeBuddy CN 会话数据库事务失败: {}", e))?;
    Ok(updates.len())
}
