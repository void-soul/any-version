use super::*;

// ─── 冲突裁决（切换结束后由用户逐条决定） ───

/// 用户对冲突会话的裁决方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConflictAction {
    /// 按标准合并规则取**较新**的一侧（另一侧先备份）
    Merge,
    /// 无条件用来源会话覆盖目标
    Overwrite,
    /// 保留目标会话，丢弃来源侧的修改
    Keep,
    /// 什么都不动，只结束这条冲突（会话可能已被清理/不重要，别让横幅一直亮着）。
    ///
    /// 以前只有三选一，而 `Merge` / `Overwrite` / `Keep` 都需要来源或目标目录还在；
    /// 一旦账号数据被清理，裁决直接报错，这条冲突就**永远留在列表里**。
    Ignore,
}

impl ConflictAction {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "merge" => Ok(Self::Merge),
            "overwrite" => Ok(Self::Overwrite),
            "keep" => Ok(Self::Keep),
            "ignore" => Ok(Self::Ignore),
            other => Err(format!("未知的冲突处理方式: {}", other)),
        }
    }

    /// 落盘与前端展示用的稳定名字（与 parse 成反函数）。
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Overwrite => "overwrite",
            Self::Keep => "keep",
            Self::Ignore => "ignore",
        }
    }
}

/// 待处理冲突文件所在目录（`{data_dir}/buddy/session-sync/codebuddy-cn/`）。
pub(super) fn pending_conflicts_dir() -> Result<std::path::PathBuf, String> {
    Ok(crate::commands::config::get_data_dir()
        .join("buddy")
        .join("session-sync")
        .join(BACKUP_PLATFORM_LABEL))
}

/// 从冲突文件名提取目标账号 uid（文件名 = `<uid>.conflicts.json`）。
///
/// 必须用 strip_suffix 而不是 `file_stem()`：`Path::file_stem` 只剥掉**最后一个**
/// 扩展名，`"x.conflicts.json".file_stem()` 返回 `"x.conflicts"` —— 拿它拼回
/// 完整文件名会多一段 `.conflicts`，读不到文件，列表永远是空（Q-0202 踩过的坑）。
pub(super) fn conflicts_stem(file_name: &str) -> Option<&str> {
    file_name.strip_suffix(".conflicts.json")
}

/// 当前登录账号（CodeBuddy CN）的 uid；未登录/无 uid/uid 非法时返回 None。
pub(super) fn current_account_uid() -> Option<String> {
    let platform = BuddyPlatform::CodebuddyCn;
    let current_id = store::get_current_account_id(platform)?;
    let accounts = store::list_accounts(platform);
    let account = accounts.iter().find(|a| a.id == current_id)?;
    let uid = account.uid.as_deref()?.trim().to_string();
    if uid.is_empty() || validate_uid(&uid).is_err() {
        return None;
    }
    Some(uid)
}

/// 列出**当前登录账号**（作为合并目标）的待处理冲突。
///
/// 只按当前账号过滤是有意的：冲突文件按目标账号分文件落盘，来回切换会留下
/// 多个账号各自的旧快照——把所有文件混在一起展示，「明明只见 7 个冲突、
/// 弹窗里却冒出 13 条」，多出来的还是过时方向（当前账号是来源而非目标）的记录。
/// 用户要裁决的永远是「我现在登录的账号里哪些会话等着处理」。
pub(crate) fn list_pending_conflicts_for_current_account() -> Vec<PendingConflict> {
    let Some(uid) = current_account_uid() else {
        return Vec::new();
    };
    let mut conflicts =
        crate::commands::buddy::session_sync::load_pending_conflicts(BACKUP_PLATFORM_LABEL, &uid);
    // 旧版本落盘的记录没有 workspace_path（或当时还原失败）：这里补解析，
    // 不用等下一次切换才拿到真实路径
    for conflict in &mut conflicts {
        if conflict.workspace_path.is_none() {
            conflict.workspace_path = resolve_workspace_display(&conflict.workspace);
        }
    }
    conflicts.sort_by(|left, right| right.source_stamp.cmp(&left.source_stamp));
    conflicts
}

/// 在全部冲突文件里按会话 id 查找记录（裁决入口用；展示层不走这里）。
pub(super) fn find_pending_conflict(
    conversation_id: &str,
) -> Result<Option<(PendingConflict, String)>, String> {
    let dir = pending_conflicts_dir()?;
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(None);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(stem) = conflicts_stem(file_name) else {
            continue;
        };
        let conflicts = crate::commands::buddy::session_sync::load_pending_conflicts(
            BACKUP_PLATFORM_LABEL,
            stem,
        );
        if let Some(conflict) = conflicts.iter().find(|c| c.id == conversation_id) {
            return Ok(Some((conflict.clone(), stem.to_string())));
        }
    }
    Ok(None)
}

/// **批量**裁决：一次对多条执行同一 action，返回当前账号剩余的待处理冲突。
///
/// 为什么必须走批量入口而不是前端串行调单条：`prepare_backup_root` 每次都会
/// `remove_dir_all` 清掉该 uid 的备份目录（见 `session_transfer.rs`），串行 N 次
/// 的结果就是**只剩最后一条的备份**，前面几条想回滚已经没东西可回。批量在这里
/// 只加一次锁、每个目标 uid 只准备一次备份目录、共用一份基线，最后统一落盘。
/// 单条命令也因此统一走这里（ids 只有一个元素）。
///
/// `progress`：非 None 时逐条上报进度。裁决要递归复制整个会话目录，整批可能跑很久，
/// 命令层已把它放进 `spawn_blocking`，进度事件是前端唯一的反馈来源。
pub(crate) fn resolve_conflicts_command(
    conversation_ids: &[String],
    action: ConflictAction,
    progress: Option<&crate::commands::buddy::ConflictProgressCtx>,
) -> Result<Vec<PendingConflict>, String> {
    if conversation_ids.is_empty() {
        return Err("未选择要处理的冲突".to_string());
    }

    // 1) 定位：逐条找记录，连同它所属的冲突文件 stem（一个 uid 一个文件）
    let mut found: Vec<(PendingConflict, String)> = Vec::new();
    for id in conversation_ids {
        validate_conversation_id(id)?;
        let Some(item) = find_pending_conflict(id)? else {
            return Err(format!("找不到会话 {} 的待处理冲突（可能已经处理过）", id));
        };
        found.push(item);
    }
    let extension_data_dir = codebuddy_extension_data_dir()?;

    // 2) 一次加锁，按目标 uid 分组执行
    let resolved = {
        let _guard = TRANSFER_LOCK
            .lock()
            .map_err(|_| "CodeBuddy CN 会话合并正在进行，请稍后重试".to_string())?;

        let mut grouped: Vec<(String, Vec<PendingConflict>)> = Vec::new();
        for (conflict, _) in &found {
            match grouped
                .iter_mut()
                .find(|(uid, _)| uid == &conflict.target_uid)
            {
                Some(entry) => entry.1.push(conflict.clone()),
                None => grouped.push((conflict.target_uid.clone(), vec![conflict.clone()])),
            }
        }

        let mut resolved: Vec<(String, PendingConflict, String)> = Vec::new();
        let mut done = 0usize;
        let total = progress.map(|p| p.total).unwrap_or(conversation_ids.len());
        for (uid, conflicts) in &grouped {
            let backup_root = crate::commands::buddy::session_transfer::prepare_backup_root(
                BACKUP_PLATFORM_LABEL,
                uid,
            )?;
            let mut baseline =
                crate::commands::buddy::session_sync::load_baseline(BACKUP_PLATFORM_LABEL, uid);
            for conflict in conflicts {
                let message = resolve_pending_conflict_at(
                    &extension_data_dir,
                    conflict,
                    action,
                    &backup_root,
                    &mut baseline,
                )?;
                done += 1;
                if let Some(ctx) = progress {
                    crate::commands::buddy::emit_conflict_progress(
                        ctx.app,
                        ctx.platform,
                        "resolving",
                        total,
                        done,
                        Some(conflict.id.clone()),
                        Some(message.clone()),
                    );
                }
                resolved.push((uid.clone(), conflict.clone(), message));
            }
            crate::commands::buddy::session_sync::save_baseline(
                BACKUP_PLATFORM_LABEL,
                uid,
                &baseline,
            );
        }
        resolved
    };

    // 3) 记录处理结果（会话明细要显示「处理过了、怎么处理的」）
    let now = crate::commands::buddy::session_sync::now_millis();
    for (uid, conflict, message) in &resolved {
        crate::commands::buddy::session_sync::append_conflict_resolutions(
            BACKUP_PLATFORM_LABEL,
            uid,
            &[crate::commands::buddy::session_sync::ConflictResolution {
                id: conflict.id.clone(),
                action: action.as_str().to_string(),
                at_ms: now,
                message: message.clone(),
            }],
        );
    }

    // 4) 从各自文件里移除已处理的条目（按 stem 分组，清空则删文件）
    let mut done_ids: Vec<String> = resolved.iter().map(|(_, c, _)| c.id.clone()).collect();
    done_ids.sort();
    done_ids.dedup();
    let mut stems: Vec<String> = found.iter().map(|(_, stem)| stem.clone()).collect();
    stems.sort();
    stems.dedup();
    for stem in stems {
        let conflicts = crate::commands::buddy::session_sync::load_pending_conflicts(
            BACKUP_PLATFORM_LABEL,
            &stem,
        );
        let remaining: Vec<PendingConflict> = conflicts
            .into_iter()
            .filter(|c| !done_ids.contains(&c.id))
            .collect();
        crate::commands::buddy::session_sync::save_pending_conflicts(
            BACKUP_PLATFORM_LABEL,
            &stem,
            &remaining,
        );
    }

    Ok(list_pending_conflicts_for_current_account())
}

/// 列出**当前登录账号**的冲突处理结果（会话明细展示用）。
pub(crate) fn list_conflict_resolutions_for_current_account(
) -> Vec<crate::commands::buddy::session_sync::ConflictResolution> {
    let Some(uid) = current_account_uid() else {
        return Vec::new();
    };
    crate::commands::buddy::session_sync::load_conflict_resolutions(BACKUP_PLATFORM_LABEL, &uid)
}

/// 执行单条冲突裁决（命令入口）。
///
/// - `Overwrite` / `Merge`（来源较新）：备份目标 → 整目录替换 → 同步辅助目录 →
///   更新目标 index.json 条目 → 基线写来源时间戳；
/// - `Keep` / `Merge`（目标不旧）：什么都不动，基线写目标时间戳（冲突就此了结）。
pub(super) fn resolve_pending_conflict(
    extension_data_dir: &Path,
    conflict: &PendingConflict,
    action: ConflictAction,
) -> Result<String, String> {
    // 与切换时的合并互斥：同一时刻只允许一边在动会话目录
    let _guard = TRANSFER_LOCK
        .lock()
        .map_err(|_| "CodeBuddy CN 会话合并正在进行，请稍后重试".to_string())?;

    // 备份目录沿用切换时的约定（每次裁决清掉旧的，只留最近一次）
    let backup_root = crate::commands::buddy::session_transfer::prepare_backup_root(
        BACKUP_PLATFORM_LABEL,
        &conflict.target_uid,
    )?;
    let mut baseline = crate::commands::buddy::session_sync::load_baseline(
        BACKUP_PLATFORM_LABEL,
        &conflict.target_uid,
    );
    let message = resolve_pending_conflict_at(
        extension_data_dir,
        conflict,
        action,
        &backup_root,
        &mut baseline,
    )?;
    crate::commands::buddy::session_sync::save_baseline(
        BACKUP_PLATFORM_LABEL,
        &conflict.target_uid,
        &baseline,
    );
    Ok(message)
}

/// 冲突裁决的文件系统核心（基线以参数传入，便于测试注入临时目录）。
pub(super) fn resolve_pending_conflict_at(
    extension_data_dir: &Path,
    conflict: &PendingConflict,
    action: ConflictAction,
    backup_root: &Path,
    baseline: &mut std::collections::BTreeMap<String, String>,
) -> Result<String, String> {
    validate_uid(&conflict.source_uid)?;
    validate_uid(&conflict.target_uid)?;
    validate_conversation_id(&conflict.id)?;
    if conflict.ide.is_empty()
        || conflict.ide.contains('/')
        || conflict.ide.contains('\\')
        || conflict.ide.contains("..")
    {
        return Err("待处理冲突的 IDE 目录名不安全".to_string());
    }

    // `<uid>/<IDE>/<uid>/history/<workspace>` 布局（与扫描时一致）
    let source_account_root = extension_data_dir
        .join(&conflict.source_uid)
        .join(&conflict.ide)
        .join(&conflict.source_uid);
    let source_workspace = source_account_root
        .join("history")
        .join(&conflict.workspace);
    let target_account_root = extension_data_dir
        .join(&conflict.target_uid)
        .join(&conflict.ide)
        .join(&conflict.target_uid);
    let target_workspace = target_account_root
        .join("history")
        .join(&conflict.workspace);
    let baseline_key = conflict_baseline_key(conflict);
    let source_dir = source_workspace.join(&conflict.id);
    let target_dir = target_workspace.join(&conflict.id);

    // ── 「忽略」：不动任何文件，只结束这条冲突 ──
    // 以前只有三选一，而三个动作都要求来源/目标目录还在；账号数据一旦被清理，
    // 裁决直接报错，这条冲突就永远留在列表里、横幅一直亮。
    if action == ConflictAction::Ignore {
        baseline.insert(
            baseline_key,
            conflict_fingerprint(&conflict.target_fingerprint, conflict.target_stamp),
        );
        return Ok("已忽略此冲突：未改动任何会话，本次不再提醒".to_string());
    }

    // ── 两侧目录都没了：没有可裁决的对象，直接清除 ──
    let source_present = source_workspace.is_dir() && source_dir.is_dir();
    let target_present = target_workspace.is_dir() && target_dir.is_dir();
    if !source_present && !target_present {
        eprintln!(
            "[Buddy SessionTransfer] 冲突两侧目录均已不存在，清除该条: id={}",
            conflict.id
        );
        baseline.remove(&baseline_key);
        return Ok("两侧会话目录均已不存在，该冲突已清除".to_string());
    }

    // ── 目录缺失时降级，而不是报错：──
    // 没有来源可覆盖 / 合并 → 保留目标；目标没了 → 只能把来源拷过去。
    let action = match action {
        ConflictAction::Overwrite | ConflictAction::Merge if !source_present => {
            ConflictAction::Keep
        }
        ConflictAction::Keep if !target_present => ConflictAction::Overwrite,
        other => other,
    };
    reject_symlink_if_exists(&source_workspace)?;
    reject_symlink_if_exists(&target_workspace)?;

    let source_index = read_workspace_index(&source_workspace.join("index.json"))?;
    let target_index_path = target_workspace.join("index.json");
    let mut target_index = read_workspace_index(&target_index_path)?;
    let source_entry = conversations(&source_index)
        .into_iter()
        .find(|c| conversation_id(c) == Some(conflict.id.as_str()))
        .ok_or_else(|| format!("来源索引里找不到会话 {}", conflict.id))?;
    let target_entry = conversations(&target_index)
        .into_iter()
        .find(|c| conversation_id(c) == Some(conflict.id.as_str()))
        .ok_or_else(|| format!("目标索引里找不到会话 {}", conflict.id))?;

    // 裁决：overwrite 必换；keep 必留；merge 交给时间戳（与正常合并同一判定）
    let overwrite_source = match action {
        ConflictAction::Overwrite => true,
        ConflictAction::Keep => false,
        // Ignore 已在前面返回；这里只为穷尽匹配
        ConflictAction::Merge | ConflictAction::Ignore => {
            let target_dir = target_workspace.join(&conflict.id);
            if !target_dir.is_dir() {
                return Err(format!(
                    "目标会话目录缺失，无法按「合并」判定，请改用覆盖或保留: {}",
                    target_dir.display()
                ));
            }
            conversation_is_newer(&source_entry, &target_entry)
        }
    };

    if overwrite_source {
        let source_dir = source_workspace.join(&conflict.id);
        let target_dir = target_workspace.join(&conflict.id);
        if !source_dir.is_dir() {
            return Err(format!("来源会话目录不存在: {}", source_dir.display()));
        }
        reject_symlink_if_exists(&target_dir)?;
        if target_dir.exists() {
            let backup = backup_root.join("conversations").join(&conflict.id);
            if !backup.exists() {
                copy_dir_recursive(&target_dir, &backup)?;
            }
        }
        replace_dir_atomic(&source_dir, &target_dir)?;
        copy_auxiliary_conversation(
            &source_account_root,
            &target_account_root,
            std::ffi::OsStr::new(&conflict.workspace),
            &conflict.id,
            true,
            &backup_root.join("auxiliary"),
        )?;
        // 目标 index.json：用来源条目替换（找不到就追加，保证会话在列表里可见）
        let entry_list = target_index
            .get_mut("conversations")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| "目标索引缺少 conversations 数组".to_string())?;
        match entry_list
            .iter_mut()
            .find(|c| conversation_id(c) == Some(conflict.id.as_str()))
        {
            Some(slot) => *slot = source_entry.clone(),
            None => entry_list.push(source_entry.clone()),
        }
        write_workspace_index(&target_index_path, &target_index)?;
        // 基线推进到来源指纹：这轮来源改动已确认落地
        // （key 与指纹格式都要和合并流程一致，否则下次切换会重复报同一条冲突）
        baseline.insert(
            baseline_key,
            conflict_fingerprint(&conflict.source_fingerprint, conflict.source_stamp),
        );
        Ok(format!(
            "已用来源会话（{}）覆盖目标（{}）",
            fmt_stamp(conflict.source_stamp),
            fmt_stamp(conflict.target_stamp)
        ))
    } else {
        // 保留目标：基线推进到目标指纹，冲突就此了结（来源侧改动视为放弃）
        baseline.insert(
            baseline_key,
            conflict_fingerprint(&conflict.target_fingerprint, conflict.target_stamp),
        );
        Ok(format!(
            "已保留目标会话（{}），来源侧改动（{}）已放弃",
            fmt_stamp(conflict.target_stamp),
            fmt_stamp(conflict.source_stamp)
        ))
    }
}

/// epoch 毫秒 → 可读时间（报错信息与返回文案用）。
pub(super) fn fmt_stamp(stamp_ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(stamp_ms)
        .map(|dt| {
            dt.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| stamp_ms.to_string())
}
