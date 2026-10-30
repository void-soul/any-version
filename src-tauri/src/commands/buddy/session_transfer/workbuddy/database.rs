use super::*;

/// WorkBuddy 5.x 主存储重映射：**只重映射有变化的会话**。
///
/// 语义参考 WorkDaddy `daemon.js` 的 auto-copy job（`references/workdaddy.md` §17）：
/// - 会话指纹 = DB `updated_at` + 会话正文本地落盘指纹（`projects/<hash>/<id>.jsonl` 等）；
/// - 与上次同步基线一致 → **跳过**（不写库、进 `skipped`）；
/// - 有变化 → 单独 `UPDATE` 该行（不再整库 UPDATE），进 `copied`；
/// - DB 有记录但磁盘正文缺失 → 仍然重映射，但记为 `partial` / `contentMissing` 提醒用户。
///
/// 无基线（首次同步或基线丢失）时全部按"有变化"处理，行为退化为原全量重映射。
pub(super) fn remap_workbuddy_database_user_id(
    db_path: &Path,
    target_uid: &str,
    backup_root: &Path,
    data_root: &Path,
    tracker: &mut SyncTracker,
) -> Result<usize, String> {
    if !db_path.is_file() {
        return Ok(0);
    }
    reject_symlink_if_exists(db_path)?;
    let mut connection = rusqlite::Connection::open(db_path).map_err(|e| {
        format!(
            "打开 WorkBuddy 会话数据库失败: path={}, error={}",
            db_path.display(),
            e
        )
    })?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|e| format!("设置 WorkBuddy 会话数据库超时失败: {}", e))?;

    let has_sessions_table: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='sessions')",
            [],
            |row| row.get(0),
        )
        .map_err(|e| format!("检查 WorkBuddy 会话数据库结构失败: {}", e))?;
    if !has_sessions_table {
        return Ok(0);
    }

    struct Candidate {
        id: String,
        label: String,
        /// 会话所属目录（`sessions.cwd`）；旧版表没有该列时为 None
        workspace: Option<String>,
        fingerprint: String,
        content_missing: bool,
    }

    // 不同 WorkBuddy 版本的 sessions 表列不完全一致（旧版可能没有 title / custom_title /
    // updated_at）。动态探测可用列后再拼 SQL，避免我们引入新的硬依赖。
    let columns: std::collections::HashSet<String> = {
        let mut statement = connection
            .prepare("PRAGMA table_info(sessions)")
            .map_err(|e| format!("读取 WorkBuddy 会话表结构失败: {}", e))?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| format!("读取 WorkBuddy 会话表结构失败: {}", e))?;
        let mut names = std::collections::HashSet::new();
        for row in rows {
            if let Ok(name) = row {
                names.insert(name.to_ascii_lowercase());
            }
        }
        names
    };
    let label_parts: Vec<&str> = ["custom_title", "title", "cwd"]
        .into_iter()
        .filter(|column| columns.contains(*column))
        .map(|column| match column {
            "cwd" => "cwd",
            other => match other {
                "custom_title" => "NULLIF(TRIM(custom_title), '')",
                _ => "NULLIF(TRIM(title), '')",
            },
        })
        .collect();
    let label_expression = if label_parts.is_empty() {
        "id".to_string()
    } else {
        format!("COALESCE({}, id)", label_parts.join(", "))
    };
    let updated_expression = if columns.contains("updated_at") {
        "updated_at"
    } else {
        "0"
    };
    // `cwd` 就是「会话所属目录」，旧版只把它拼进 label 的 COALESCE 里当兜底，
    // 明细上完全看不到；这里单独取出来作为独立的 workspace 字段。
    let workspace_expression = if columns.contains("cwd") {
        "cwd"
    } else {
        "NULL"
    };

    let candidates: Vec<Candidate> = {
        let sql = format!(
            "SELECT id, {}, {}, {} FROM sessions WHERE user_id != ?1 AND deleted_at IS NULL",
            label_expression, updated_expression, workspace_expression
        );
        let mut statement = connection
            .prepare(&sql)
            .map_err(|e| format!("读取 WorkBuddy 会话索引失败: {}", e))?;
        let rows = statement
            .query_map([target_uid], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(|e| format!("读取 WorkBuddy 会话索引失败: {}", e))?;
        let mut collected = Vec::new();
        for row in rows {
            let (id, label, updated_at, workspace) =
                row.map_err(|e| format!("读取 WorkBuddy 会话记录失败: {}", e))?;
            let content = session_sync::workbuddy_session_fingerprint(data_root, &id);
            collected.push(Candidate {
                fingerprint: session_sync::combined_fingerprint(updated_at, content.as_deref()),
                content_missing: content.is_none(),
                label: label.unwrap_or_else(|| id.clone()),
                // 空串 / 全空白与「表里没这一列」等价，统一收敛成 None
                workspace: workspace.filter(|w| !w.trim().is_empty()),
                id,
            });
        }
        collected
    };
    if candidates.is_empty() {
        return Ok(0);
    }

    let mut pending: Vec<&Candidate> = Vec::new();
    for candidate in &candidates {
        if tracker.is_unchanged(&candidate.id, &candidate.fingerprint) {
            tracker.record_in(
                &candidate.id,
                &candidate.label,
                SessionSyncStatus::Skipped,
                "unchanged",
                Some(candidate.fingerprint.clone()),
                candidate.workspace.clone(),
            );
        } else {
            pending.push(candidate);
        }
    }
    if pending.is_empty() {
        return Ok(0);
    }

    std::fs::create_dir_all(backup_root).map_err(|e| {
        format!(
            "创建 WorkBuddy 会话数据库备份目录失败: path={}, error={}",
            backup_root.display(),
            e
        )
    })?;
    backup_sqlite_database(db_path, backup_root)?;

    let transaction = connection
        .transaction()
        .map_err(|e| format!("开启 WorkBuddy 会话数据库事务失败: {}", e))?;
    let mut changed = 0usize;
    for candidate in pending {
        match transaction.execute(
            "UPDATE sessions SET user_id = ?1 WHERE id = ?2 AND deleted_at IS NULL",
            rusqlite::params![target_uid, candidate.id],
        ) {
            Ok(rows) => {
                changed += rows;
                let reason = if candidate.content_missing {
                    "contentMissing"
                } else if tracker.baseline_of(&candidate.id).is_none() {
                    "firstSync"
                } else {
                    "updated"
                };
                let status = if candidate.content_missing {
                    SessionSyncStatus::Partial
                } else {
                    SessionSyncStatus::Copied
                };
                tracker.record_in(
                    &candidate.id,
                    &candidate.label,
                    status,
                    reason,
                    Some(candidate.fingerprint.clone()),
                    candidate.workspace.clone(),
                );
            }
            Err(error) => {
                eprintln!(
                    "[Buddy WB Transfer] 会话 user_id 重映射失败: id={}, error={}",
                    candidate.id, error
                );
                tracker.record_in(
                    &candidate.id,
                    &candidate.label,
                    SessionSyncStatus::Failed,
                    "updateFailed",
                    None,
                    candidate.workspace.clone(),
                );
            }
        }
    }
    transaction
        .commit()
        .map_err(|e| format!("提交 WorkBuddy 会话数据库事务失败: {}", e))?;
    Ok(changed)
}

/// 备份 sqlite 主库 + -wal + -shm（WAL 模式下未 checkpoint 的数据在 -wal 里）。
pub(super) fn backup_sqlite_database(db_path: &Path, backup_root: &Path) -> Result<(), String> {
    for suffix in ["", "-wal", "-shm"] {
        let source = if suffix.is_empty() {
            db_path.to_path_buf()
        } else {
            let mut path = db_path.as_os_str().to_os_string();
            path.push(suffix);
            std::path::PathBuf::from(path)
        };
        if !source.is_file() {
            continue;
        }
        reject_symlink_if_exists(&source)?;
        let file_name = source
            .file_name()
            .ok_or_else(|| format!("无法解析 WorkBuddy 数据库文件名: {}", source.display()))?;
        std::fs::copy(&source, backup_root.join(file_name)).map_err(|e| {
            format!(
                "备份 WorkBuddy 会话数据库失败: path={}, error={}",
                source.display(),
                e
            )
        })?;
    }
    Ok(())
}
