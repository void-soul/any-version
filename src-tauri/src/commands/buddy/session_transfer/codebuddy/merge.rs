use super::*;

/// 合并单个工作区：会话目录 + index.json（去重、取新、保留 current）+ 辅助目录。
pub(super) fn merge_workspace_history(
    source_workspace: &Path,
    target_workspace: &Path,
    source_account_root: &Path,
    target_account_root: &Path,
    workspace_name: &std::ffi::OsStr,
    backup_root: &Path,
    tracker: &mut SyncTracker,
    scan: &ConflictScanContext,
    pending_conflicts: &mut Vec<PendingConflict>,
    conflict_policy: ConflictPolicy,
) -> Result<(), String> {
    let workspace_label = workspace_name.to_string_lossy().to_string();
    // 哈希目录名对用户毫无意义（一串 32 位 MD5），能还原就还原成真实项目路径；
    // 还原不了再用哈希。明细「目录」列、无标题会话的展示名都用它。
    let workspace_display =
        resolve_workspace_display(&workspace_label).unwrap_or_else(|| workspace_label.clone());
    // 本次调用处理的就是 workspace_name 这一个工作区目录：设一次上下文，
    // 随后本函数内所有 record 都会带上它（明细里的「目录」列）。
    tracker.set_workspace(Some(workspace_display.clone()));

    let source_index_path = source_workspace.join("index.json");
    if !source_index_path.is_file() {
        return Ok(());
    }
    let source_index = read_workspace_index(&source_index_path)?;

    // 目标工作区不存在：整目录原子复制（含全部会话子目录）+ 辅助目录
    if !target_workspace.exists() {
        copy_dir_atomic(source_workspace, target_workspace)?;
        copy_auxiliary_workspace_roots(source_account_root, target_account_root, workspace_name)?;
        for conversation in conversations(&source_index) {
            let id = conversation_id(&conversation)
                .unwrap_or_default()
                .to_string();
            tracker.record(
                &id,
                &conversation_label(&conversation, &id, &workspace_display),
                SessionSyncStatus::Copied,
                "firstSync",
                Some(conversation_fingerprint(
                    &conversation,
                    &source_workspace.join(&id),
                )),
            );
        }
        return Ok(());
    }

    let target_index_path = target_workspace.join("index.json");
    let mut target_index = read_workspace_index(&target_index_path)?;
    let source_conversations = conversations(&source_index);
    let target_conversations = conversations(&target_index);
    let mut target_positions: HashMap<String, usize> = HashMap::new();
    let mut merged: Vec<Value> = Vec::with_capacity(target_conversations.len());
    let mut index_changed = false;
    for conversation in target_conversations {
        if let Some(id) = conversation_id(&conversation) {
            if let Some(existing_index) = target_positions.get(id).copied() {
                if conversation_is_newer(&conversation, &merged[existing_index]) {
                    merged[existing_index] = conversation;
                }
                index_changed = true;
                continue;
            }
            target_positions.insert(id.to_string(), merged.len());
        }
        merged.push(conversation);
    }

    let mut index_backup_created = false;
    for source_conversation in source_conversations {
        let Some(id) = conversation_id(&source_conversation) else {
            continue;
        };
        validate_conversation_id(id)?;
        let source_conversation_dir = source_workspace.join(id);
        let label = conversation_label(&source_conversation, id, &workspace_display);
        let source_stamp = conversation_fingerprint(&source_conversation, &source_conversation_dir);
        if !source_conversation_dir.is_dir() {
            eprintln!(
                "[Buddy SessionTransfer] 来源会话目录不存在，已跳过: {}",
                source_conversation_dir.display()
            );
            tracker.record(id, &label, SessionSyncStatus::Failed, "sourceMissing", None);
            continue;
        }

        match target_positions.get(id).copied() {
            None => {
                ensure_workspace_index_backup(
                    &target_index_path,
                    backup_root,
                    &mut index_backup_created,
                )?;
                let target_conversation_dir = target_workspace.join(id);
                reject_symlink_if_exists(&target_conversation_dir)?;
                if target_conversation_dir.exists() && !target_conversation_dir.is_dir() {
                    return Err(format!(
                        "CodeBuddy CN 目标会话路径不是目录: {}",
                        target_conversation_dir.display()
                    ));
                }
                let replace_orphaned = target_conversation_dir.exists();
                if target_conversation_dir.exists() {
                    let backup = backup_root.join("orphaned-conversations").join(id);
                    if !backup.exists() {
                        copy_dir_recursive(&target_conversation_dir, &backup)?;
                    }
                    replace_dir_atomic(&source_conversation_dir, &target_conversation_dir)?;
                } else {
                    copy_dir_atomic(&source_conversation_dir, &target_conversation_dir)?;
                }
                copy_auxiliary_conversation(
                    source_account_root,
                    target_account_root,
                    workspace_name,
                    id,
                    replace_orphaned,
                    &backup_root.join("auxiliary"),
                )?;
                let reason = if tracker.baseline_of(id).is_none() {
                    "firstSync"
                } else {
                    "restored"
                };
                let owned_id = id.to_string();
                target_positions.insert(owned_id.clone(), merged.len());
                merged.push(source_conversation);
                tracker.record(
                    &owned_id,
                    &label,
                    SessionSyncStatus::Copied,
                    reason,
                    Some(source_stamp.clone()),
                );
                index_changed = true;
            }
            Some(target_index_pos) => {
                let target_conversation_dir = target_workspace.join(id);
                reject_symlink_if_exists(&target_conversation_dir)?;
                if target_conversation_dir.exists() && !target_conversation_dir.is_dir() {
                    return Err(format!(
                        "CodeBuddy CN 目标会话路径不是目录: {}",
                        target_conversation_dir.display()
                    ));
                }
                // 「只处理有变化的会话」+「两侧都改过即冲突」：参考 WorkDaddy §17 的三分支。
                let target_stamp =
                    conversation_fingerprint(&merged[target_index_pos], &target_conversation_dir);
                match decide_sync(
                    tracker.baseline_of(id),
                    &source_stamp,
                    &target_stamp,
                    target_conversation_dir.is_dir(),
                ) {
                    SyncDecision::Unchanged => {
                        tracker.record(
                            id,
                            &label,
                            SessionSyncStatus::Skipped,
                            "unchanged",
                            Some(source_stamp.clone()),
                        );
                        continue;
                    }
                    SyncDecision::Conflict => {
                        // WorkBuddy（共享库重映射语义）：不产生可裁决冲突。
                        // 数据行为与冲突分支完全一致 —— 保留目标版本、不覆盖、不写基线
                        // （下次切换若仍两侧偏离会再判一次）；只是不报「处理不了的冲突」。
                        if conflict_policy == ConflictPolicy::Suppressed {
                            tracker.record(
                                id,
                                &label,
                                SessionSyncStatus::Skipped,
                                "bothChangedKept",
                                None,
                            );
                            continue;
                        }
                        eprintln!(
                            "[Buddy SessionTransfer] 会话两侧都有更新，保留目标版本并记为冲突: id={}, source={}, target={}",
                            id, source_stamp, target_stamp
                        );
                        // 不写基线：冲突未解决前每次切换都会再次提醒。
                        // 同时把定位信息落盘，用户在切换结束后仍可逐条裁决
                        // （合并 / 覆盖目标 / 保留目标），不必赶在切换流程里做决定。
                        pending_conflicts.push(PendingConflict {
                            id: id.to_string(),
                            label: label.clone(),
                            // 文件系统定位用原始哈希名；真实路径另放一列给前端展示
                            workspace: workspace_label.clone(),
                            workspace_path: if workspace_display == workspace_label {
                                None
                            } else {
                                Some(workspace_display.clone())
                            },
                            ide: scan.ide.clone(),
                            source_uid: scan.source_uid.clone(),
                            target_uid: scan.target_uid.clone(),
                            source_stamp: conversation_timestamp(&source_conversation)
                                .unwrap_or_default(),
                            target_stamp: conversation_timestamp(&merged[target_index_pos])
                                .unwrap_or_default(),
                            // 裁决后写入基线用的必须是判定指纹（与合并流程同格式），
                            // 否则下一次切换会因格式不一致再次报同一条冲突
                            source_fingerprint: source_stamp.clone(),
                            target_fingerprint: target_stamp.clone(),
                        });
                        tracker.record(
                            id,
                            &label,
                            SessionSyncStatus::Conflict,
                            "bothChanged",
                            None,
                        );
                        continue;
                    }
                    SyncDecision::Apply => {}
                }
                // 目标会话目录缺失，或来源更新：整体替换（复刻 cockpit-tools 判定）
                if !target_conversation_dir.is_dir()
                    || conversation_is_newer(&source_conversation, &merged[target_index_pos])
                {
                    ensure_workspace_index_backup(
                        &target_index_path,
                        backup_root,
                        &mut index_backup_created,
                    )?;
                    if target_conversation_dir.exists() {
                        let backup = backup_root.join("conversations").join(id);
                        if !backup.exists() {
                            copy_dir_recursive(&target_conversation_dir, &backup)?;
                        }
                    }
                    replace_dir_atomic(&source_conversation_dir, &target_conversation_dir)?;
                    copy_auxiliary_conversation(
                        source_account_root,
                        target_account_root,
                        workspace_name,
                        id,
                        true,
                        &backup_root.join("auxiliary"),
                    )?;
                    let reason = if tracker.baseline_of(id).is_none() {
                        "firstSync"
                    } else {
                        "updated"
                    };
                    let owned_id = id.to_string();
                    merged[target_index_pos] = source_conversation;
                    tracker.record(
                        &owned_id,
                        &label,
                        SessionSyncStatus::Copied,
                        reason,
                        Some(source_stamp.clone()),
                    );
                    index_changed = true;
                } else {
                    // 目标更新、无需替换：仍然刷新基线，避免下次重复比较
                    tracker.record(
                        id,
                        &label,
                        SessionSyncStatus::Skipped,
                        "targetNewer",
                        Some(target_stamp),
                    );
                }
            }
        }
    }

    merged.sort_by(|left, right| compare_conversation_recency(right, left));
    if let Some(object) = target_index.as_object_mut() {
        object.insert("conversations".to_string(), Value::Array(merged.clone()));
    }

    // 仅当来源 current 会话在合并结果中存在时才转移 current 指针
    let source_current = source_index
        .get("current")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty());
    if let Some(source_current) = source_current {
        let current_exists = merged
            .iter()
            .any(|conversation| conversation_id(conversation) == Some(source_current));
        if current_exists
            && target_index.get("current").and_then(Value::as_str) != Some(source_current)
        {
            if let Some(object) = target_index.as_object_mut() {
                object.insert(
                    "current".to_string(),
                    Value::String(source_current.to_string()),
                );
            }
            index_changed = true;
        }
    }

    if index_changed {
        ensure_workspace_index_backup(&target_index_path, backup_root, &mut index_backup_created)?;
        let serialized = serde_json::to_string_pretty(&target_index)
            .map_err(|e| format!("序列化 CodeBuddy CN 工作区索引失败: {}", e))?;
        store::write_atomic(&target_index_path, &serialized)?;
    }

    Ok(())
}
