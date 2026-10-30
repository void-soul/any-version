use super::*;

/// 工作区级辅助目录复制（check-point/file-tree/plan-task/genie-cache/connectors）
pub(super) fn copy_auxiliary_workspace_roots(
    source_account_root: &Path,
    target_account_root: &Path,
    workspace_name: &std::ffi::OsStr,
) -> Result<(), String> {
    for kind in AUXILIARY_KINDS {
        let source = source_account_root.join(kind).join(workspace_name);
        if source.is_dir() {
            copy_dir_atomic(
                &source,
                &target_account_root.join(kind).join(workspace_name),
            )?;
        }
    }
    Ok(())
}

/// 会话级辅助目录复制；replace=true 时旧目录先备份再替换。
pub(super) fn copy_auxiliary_conversation(
    source_account_root: &Path,
    target_account_root: &Path,
    workspace_name: &std::ffi::OsStr,
    conversation_id: &str,
    replace: bool,
    backup_root: &Path,
) -> Result<(), String> {
    for kind in AUXILIARY_KINDS {
        let source = source_account_root
            .join(kind)
            .join(workspace_name)
            .join(conversation_id);
        if !source.is_dir() {
            continue;
        }
        let target = target_account_root
            .join(kind)
            .join(workspace_name)
            .join(conversation_id);
        if replace && target.exists() {
            let backup = backup_root.join(kind).join(conversation_id);
            if !backup.exists() {
                copy_dir_recursive(&target, &backup)?;
            }
            replace_dir_atomic(&source, &target)?;
        } else if !target.exists() {
            copy_dir_atomic(&source, &target)?;
        }
    }
    Ok(())
}

/// 搬一个会话到另一个工作区的产物。
#[derive(Debug, Default)]
pub(crate) struct ConversationMoveOutcome {
    /// 搬动过的目录数（正文 + 各类辅助目录）
    pub dirs_moved: usize,
    /// 实际发生搬运的 IDE 目录名（基线 key 带 IDE 维度，要跟着一起改）
    pub ides: Vec<String>,
}

/// 把一个会话从旧工作区搬到新工作区（同账号下**每个 IDE 目录**都搬）。
///
/// 每个 IDE 目录内依次处理：正文 `history/<ws>/<id>` → 5 类辅助目录 → 两侧 index.json。
/// 搬运顺序是「先备份、再复制到目标、最后才删源」：中途失败时源还在，不至于两头空。
///
/// `backup_root` 由调用方**整批准备一次**（`prepare_backup_root` 会清空该 uid 的旧备份，
/// 一条会话调一次会把前一条的备份冲掉）。
pub(crate) fn move_conversation_to_workspace(
    extension_data_dir: &Path,
    uid: &str,
    conversation_id: &str,
    old_workspace: &str,
    new_workspace: &str,
    backup_root: &Path,
) -> Result<ConversationMoveOutcome, String> {
    validate_uid(uid)?;
    validate_conversation_id(conversation_id)?;
    let mut outcome = ConversationMoveOutcome::default();
    let account_outer = extension_data_dir.join(uid);
    if !account_outer.is_dir() {
        return Ok(outcome);
    }
    let ide_entries = std::fs::read_dir(&account_outer).map_err(|e| {
        format!(
            "读取 CodeBuddy CN 账号目录失败: path={}, error={}",
            account_outer.display(),
            e
        )
    })?;
    let mut errors: Vec<String> = Vec::new();

    for ide_entry in ide_entries.flatten() {
        let ide_name = ide_entry.file_name().to_string_lossy().to_string();
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
        let source = history_root.join(old_workspace).join(conversation_id);
        if !source.is_dir() {
            continue;
        }

        // ① 备份：按 ide/workspace/id 分桶，只备一次
        let backup = backup_root
            .join(&ide_name)
            .join("history")
            .join(old_workspace)
            .join(conversation_id);
        if !backup.exists() {
            copy_dir_recursive(&source, &backup).map_err(|e| format!("备份会话正文失败: {}", e))?;
        }

        // ② 正文：复制到目标工作区（已存在则不覆盖 —— 宁可报错也不动用户的现有会话）
        let target = history_root.join(new_workspace).join(conversation_id);
        reject_symlink_if_exists(&target)?;
        if target.exists() {
            errors.push(format!(
                "目标工作区已存在同名会话目录: {}",
                target.display()
            ));
            continue;
        }
        copy_dir_atomic(&source, &target)?;

        // ③ 辅助目录（check-point / file-tree / plan-task / genie-cache / connectors）
        for kind in AUXILIARY_KINDS {
            let aux_source = account_root
                .join(kind)
                .join(old_workspace)
                .join(conversation_id);
            if !aux_source.is_dir() {
                continue;
            }
            let aux_backup = backup_root
                .join(&ide_name)
                .join(kind)
                .join(old_workspace)
                .join(conversation_id);
            if !aux_backup.exists() {
                let _ = copy_dir_recursive(&aux_source, &aux_backup);
            }
            let aux_target = account_root
                .join(kind)
                .join(new_workspace)
                .join(conversation_id);
            if aux_target.exists() {
                continue;
            }
            if let Err(e) = copy_dir_atomic(&aux_source, &aux_target) {
                errors.push(format!("搬辅助目录 {} 失败: {}", kind, e));
                continue;
            }
            let _ = std::fs::remove_dir_all(&aux_source);
            outcome.dirs_moved += 1;
        }

        // ④ 两侧 index.json：旧工作区摘掉条目、目标工作区登记
        if let Err(e) = reindex_after_move(
            &history_root,
            old_workspace,
            new_workspace,
            conversation_id,
            backup_root,
        ) {
            errors.push(e);
            continue;
        }

        // ⑤ 目标都就位了才删源
        let _ = std::fs::remove_dir_all(&source);
        outcome.dirs_moved += 1;
        outcome.ides.push(ide_name);
    }

    if !errors.is_empty() {
        return Err(errors.join("；"));
    }
    Ok(outcome)
}

/// 搬完后维护两侧 index.json（**改前都先备份**）。
///
/// 旧工作区：移除该会话条目，`current` 指向它就清空（与删除会话同一约定）。
/// 目标工作区：没有索引文件时新建一份再登记；`current` 空着则指向刚搬来的会话。
/// 参数名不能用 `conversation_id`：会和取条目 id 的 `conversation_id()` 函数重名（遮蔽）。
pub(super) fn reindex_after_move(
    history_root: &Path,
    old_workspace: &str,
    new_workspace: &str,
    conv_id: &str,
    backup_root: &Path,
) -> Result<(), String> {
    let mut moved_entry: Option<Value> = None;

    let old_index_path = history_root.join(old_workspace).join("index.json");
    if old_index_path.is_file() {
        let mut index = read_workspace_index(&old_index_path)?;
        let _ = std::fs::copy(
            &old_index_path,
            backup_root.join(format!("index-{}.json", old_workspace)),
        );
        let mut remaining: Vec<Value> = Vec::new();
        for conversation in conversations(&index) {
            if conversation_id(&conversation) == Some(conv_id) {
                moved_entry = Some(conversation);
            } else {
                remaining.push(conversation);
            }
        }
        if let Some(object) = index.as_object_mut() {
            object.insert("conversations".to_string(), Value::Array(remaining));
            let current = object
                .get("current")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if current == conv_id {
                object.insert("current".to_string(), Value::String(String::new()));
            }
        }
        let serialized = serde_json::to_string_pretty(&index)
            .map_err(|e| format!("序列化工作区索引失败: {}", e))?;
        store::write_atomic(&old_index_path, &serialized)?;
    }

    let Some(entry) = moved_entry else {
        return Ok(());
    };

    let new_dir = history_root.join(new_workspace);
    let new_index_path = new_dir.join("index.json");
    let mut new_index = if new_index_path.is_file() {
        read_workspace_index(&new_index_path)?
    } else {
        std::fs::create_dir_all(&new_dir).map_err(|e| format!("创建目标工作区目录失败: {}", e))?;
        serde_json::json!({ "version": 1, "conversations": [] })
    };
    let mut list = conversations(&new_index);
    if !list.iter().any(|c| conversation_id(c) == Some(conv_id)) {
        list.push(entry);
        list.sort_by(|left, right| compare_conversation_recency(right, left));
    }
    if let Some(object) = new_index.as_object_mut() {
        object.insert("conversations".to_string(), Value::Array(list));
        let current = object
            .get("current")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if current.is_empty() {
            object.insert("current".to_string(), Value::String(conv_id.to_string()));
        }
    }
    let serialized = serde_json::to_string_pretty(&new_index)
        .map_err(|e| format!("序列化目标工作区索引失败: {}", e))?;
    store::write_atomic(&new_index_path, &serialized)?;
    Ok(())
}
