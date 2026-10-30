use super::*;

/// 合并来源账号 history 目录到目标账号（逐 IDE、逐工作区）。
/// 返回扫描到的工作区数；逐会话结局写入 `tracker`，冲突详情追加进 `pending_conflicts`。
///
/// `conflict_policy` 见 [`ConflictPolicy`]：WorkBuddy 复用本函数但不产生冲突分支，
/// 两侧都改过时保留目标版本并记 skipped，而不是报一个「处理不了」的冲突。
pub(in crate::commands::buddy::session_transfer) fn sync_history_between_accounts(
    extension_data_dir: &Path,
    source_uid: &str,
    target_uid: &str,
    backup_root: &Path,
    progress: TransferProgress,
    tracker: &mut SyncTracker,
    pending_conflicts: &mut Vec<PendingConflict>,
    conflict_policy: ConflictPolicy,
) -> Result<usize, String> {
    let source_outer = extension_data_dir.join(source_uid);
    if !source_outer.is_dir() {
        return Ok(0);
    }
    reject_symlink_if_exists(&source_outer)?;

    let target_outer = extension_data_dir.join(target_uid);
    if target_outer.exists() {
        reject_symlink_if_exists(&target_outer)?;
    }

    let mut scanned_workspaces = 0usize;
    let ide_entries = std::fs::read_dir(&source_outer).map_err(|e| {
        format!(
            "读取 CodeBuddy CN 会话根目录失败: path={}, error={}",
            source_outer.display(),
            e
        )
    })?;

    for ide_entry in ide_entries {
        let ide_entry = ide_entry.map_err(|e| format!("读取 CodeBuddy CN IDE 目录失败: {}", e))?;
        let metadata = std::fs::symlink_metadata(ide_entry.path())
            .map_err(|e| format!("读取 CodeBuddy CN IDE 目录属性失败: {}", e))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            continue;
        }
        let ide_name = ide_entry.file_name();
        let source_account_root = ide_entry.path().join(source_uid);
        let source_history_root = source_account_root.join("history");
        if !source_history_root.is_dir() {
            continue;
        }
        let target_account_root = extension_data_dir
            .join(target_uid)
            .join(&ide_name)
            .join(target_uid);
        reject_symlink_if_exists(&extension_data_dir.join(target_uid).join(&ide_name))?;
        reject_symlink_if_exists(&target_account_root)?;
        let target_history_root = target_account_root.join("history");

        let workspaces = std::fs::read_dir(&source_history_root).map_err(|e| {
            format!(
                "读取 CodeBuddy CN 工作区会话目录失败: path={}, error={}",
                source_history_root.display(),
                e
            )
        })?;
        for workspace_entry in workspaces {
            let workspace_entry =
                workspace_entry.map_err(|e| format!("读取 CodeBuddy CN 工作区目录失败: {}", e))?;
            let metadata = std::fs::symlink_metadata(workspace_entry.path())
                .map_err(|e| format!("读取 CodeBuddy CN 工作区目录属性失败: {}", e))?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                continue;
            }
            let workspace_name = workspace_entry.file_name();
            let source_workspace = workspace_entry.path();
            let target_workspace = target_history_root.join(&workspace_name);
            reject_symlink_if_exists(&target_workspace)?;
            let workspace_backup = backup_root
                .join("history")
                .join(&ide_name)
                .join(&workspace_name);
            let scan = ConflictScanContext {
                ide: ide_name.to_string_lossy().to_string(),
                source_uid: source_uid.to_string(),
                target_uid: target_uid.to_string(),
            };
            // 基线 key 带上 IDE + 工作区维度：同一个会话 id 可能同时存在于多个
            // IDE / 工作区目录，共用一个 key 会让跨工作区的改动互相干扰。
            tracker.set_namespace(Some(workspace_namespace(
                &scan.ide,
                &workspace_name.to_string_lossy(),
            )));
            merge_workspace_history(
                &source_workspace,
                &target_workspace,
                &source_account_root,
                &target_account_root,
                &workspace_name,
                &workspace_backup,
                tracker,
                &scan,
                pending_conflicts,
                conflict_policy,
            )?;
            scanned_workspaces += 1;
            emit_switch_progress(
                progress.app,
                progress.platform,
                progress.account_id,
                "merging",
                scanned_workspaces,
                None,
            );
        }
    }
    Ok(scanned_workspaces)
}

/// 会话同步判定（对应 WorkDaddy auto-copy job 的三分支，见 `references/workdaddy.md` §17）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyncDecision {
    /// 两侧都与基线一致：什么都不做
    Unchanged,
    /// 两侧都比基线新：拒绝猜测、拒绝覆盖
    Conflict,
    /// 只有一侧有变化（或首次同步）：正常合并
    Apply,
}

/// 依据上次同步基线判定该会话是否要处理。
///
/// - 无基线（首次同步）→ `Apply`（行为与改造前一致）；
/// - 目标会话文件缺失 → `Apply`（必须补齐，不参与"无变化"判定）；
/// - **任一侧时间戳未知** → `Apply`：未知的一侧没法自证"没变过"，
///   宁可多比较一次，也不能像以前那样因"两边都是 0"被判成无变化而静默跳过；
/// - 两侧都与基线一致 → `Unchanged`；
/// - 两侧都偏离基线 → `Conflict`。
pub(crate) fn decide_sync(
    baseline: Option<&str>,
    source_stamp: &str,
    target_stamp: &str,
    target_present: bool,
) -> SyncDecision {
    let Some(baseline) = baseline else {
        return SyncDecision::Apply;
    };
    if !target_present {
        return SyncDecision::Apply;
    }
    let known = |stamp: &str| !stamp.starts_with(UNKNOWN_STAMP);
    if !(known(source_stamp) && known(target_stamp) && known(baseline)) {
        return SyncDecision::Apply;
    }
    let source_changed = source_stamp != baseline;
    let target_changed = target_stamp != baseline;
    match (source_changed, target_changed) {
        (false, false) => SyncDecision::Unchanged,
        (true, true) => SyncDecision::Conflict,
        _ => SyncDecision::Apply,
    }
}

/// 基线 key 的命名空间：`IDE|工作区`（与 `merge_workspace_history` 里写入的维度一致）。
pub(crate) fn workspace_namespace(ide: &str, workspace: &str) -> String {
    format!("{}|{}", ide, workspace)
}

/// 这条冲突对应的基线 key（带 IDE + 工作区维度，见 [`workspace_namespace`]）。
pub(super) fn conflict_baseline_key(conflict: &PendingConflict) -> String {
    crate::commands::buddy::session_sync::baseline_key(
        &workspace_namespace(&conflict.ide, &conflict.workspace),
        &conflict.id,
    )
}

/// 裁决后要写入基线的指纹：优先用落盘时的判定指纹；
/// 老记录（本次改动之前写的）没有这个字段，回落到纯时间戳，保证格式不混。
pub(super) fn conflict_fingerprint(stored: &str, stamp_ms: i64) -> String {
    if stored.is_empty() {
        stamp_ms.to_string()
    } else {
        stored.to_string()
    }
}

/// 会话展示名：标题优先，其次**目录名**，最后才回落到 id。
///
/// 此前无标题的会话直接显示裸 id（一串哈希），用户根本认不出是哪个会话；
/// 所属工作区目录对应真实项目，是比 id 有意义得多的 fallback。
pub(super) fn conversation_label(
    conversation: &Value,
    id: &str,
    workspace_fallback: &str,
) -> String {
    for key in ["title", "name", "label"] {
        if let Some(text) = conversation.get(key).and_then(Value::as_str) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    if !workspace_fallback.trim().is_empty() {
        return workspace_fallback.trim().to_string();
    }
    id.to_string()
}
