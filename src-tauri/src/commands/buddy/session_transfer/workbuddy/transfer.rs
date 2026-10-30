use super::*;

pub(super) fn transfer_local_sessions(
    runtime_dir: &Path,
    source_uid: &str,
    target_uid: &str,
    progress: TransferProgress,
) -> Result<SessionTransferReport, String> {
    codebuddy::validate_uid(source_uid).map_err(|e| e.replace("CodeBuddy CN", "WorkBuddy"))?;
    codebuddy::validate_uid(target_uid).map_err(|e| e.replace("CodeBuddy CN", "WorkBuddy"))?;
    if source_uid == target_uid {
        return Ok(SessionTransferReport::default());
    }

    let _guard = TRANSFER_LOCK
        .lock()
        .map_err(|_| "WorkBuddy 本地会话合并正在进行，请稍后重试".to_string())?;

    let (config_dir, electron_data_dir) = resolve_runtime_dirs(runtime_dir)?;
    let backup_root = crate::commands::buddy::session_transfer::prepare_backup_root(
        BACKUP_PLATFORM_LABEL,
        target_uid,
    )?;

    let mut report = SessionTransferReport::default();
    // 上次同步基线（目标账号维度）：用来判断"哪些会话真的有变化"，无基线时退化为全量。
    let baseline = session_sync::load_baseline(BACKUP_PLATFORM_LABEL, target_uid);
    let mut tracker = SyncTracker::new(baseline);
    let data_root = data_dir_for_workbuddy()?;

    // 1) 旧版 per-uid 扩展目录合并（5.x 通常不存在，仅兼容旧版落盘结构）
    //
    // 共享模式短路：WorkBuddy 5.x 的会话本来就只有一份（`~/.workbuddy/workbuddy.db` 的
    // sessions 行 + `~/.workbuddy/projects/<hash>/<id>.jsonl` 正文），没有 per-uid 副本，
    // 所以目录级合并对 5.x 是纯冗余。更危险的是两个平台**共用同一批 uid**：CN 的
    // `<Data>/<uid>/CodeBuddyIDE/<uid>/history` 现在是指向共享根的 junction，
    // 不短路就会拿 CN 的共享会话去和 WorkBuddy 目标账号「合并」（源和目标其实是同一份物理数据）。
    // 判定为共享时只做第 2/3 步的 user_id 重映射 —— 这正是 CN 共享模式的等价语义。
    let extension_roots = select_extension_data_roots(source_uid)?;
    if extension_roots.is_empty() {
        eprintln!(
            "[Buddy WB Transfer] 未找到来源账号本地会话目录（uid={}），跳过目录级合并，继续数据库层合并",
            source_uid
        );
    } else {
        // WorkBuddy 的会话合并不产生可裁决冲突（共享库重映射语义）：
        // 共用的合并函数遇到「两侧都改过」时照旧保留目标版本、不覆盖、不写基线，
        // 但不记成 conflict —— 否则会出现「切换说有冲突 → 列表里是 codebuddy 会话 →
        // 点处理冲突说没有冲突」的三段自相矛盾（处理端对 WorkBuddy 恒返回空）。
        let mut pending_conflicts: Vec<PendingConflict> = Vec::new();
        for (label, extension_data_dir) in extension_roots {
            let shared = codebuddy::target_sessions_shared(&extension_data_dir, target_uid)
                || codebuddy::target_sessions_shared(&extension_data_dir, source_uid);
            if shared {
                eprintln!(
                    "[Buddy WB Transfer] {} 下 uid={} 的会话目录是共享链接，跳过目录级合并（只重映射 user_id）",
                    label,
                    target_uid
                );
                tracker.record_in(
                    "",
                    &format!("shared:{}", label),
                    SessionSyncStatus::Skipped,
                    "sharedDirectory",
                    None,
                    None,
                );
                continue;
            }
            report.scanned_workspaces += codebuddy::sync_history_between_accounts(
                &extension_data_dir,
                source_uid,
                target_uid,
                &backup_root.join(label),
                progress,
                &mut tracker,
                &mut pending_conflicts,
                codebuddy::ConflictPolicy::Suppressed,
            )?;
        }
    }

    // 2) WorkBuddy 5.x 主存储：workbuddy.db 逐会话判定后按需重映射 —— 无条件执行
    report.updated_session_rows += remap_workbuddy_database_user_id(
        &config_dir.join("workbuddy.db"),
        target_uid,
        &backup_root.join("database"),
        &data_root,
        &mut tracker,
    )?;

    // 3) 旧版 vscdb（若存在）—— 无条件执行，取第一个命中的（vscdb 里没有逐会话时间戳，
    //    按目标账号整库重映射，明细以"已处理"一条记录体现）。
    for legacy_db in [
        electron_data_dir.join("codebuddy-sessions.vscdb"),
        config_dir.join("codebuddy-sessions.vscdb"),
    ] {
        if !legacy_db.is_file() {
            continue;
        }
        let remapped = codebuddy::remap_session_vscdb_user_id(
            &legacy_db,
            source_uid,
            target_uid,
            &backup_root.join("legacy-database"),
        )?;
        report.updated_session_rows += remapped;
        // 整库汇总行（id 为空），不归属任何工作区，显式给 None，
        // 免得继承到 CodeBuddy 侧设定的「当前工作区」上下文
        tracker.record_in(
            "",
            "legacy:codebuddy-sessions.vscdb",
            if remapped > 0 {
                SessionSyncStatus::Copied
            } else {
                SessionSyncStatus::Skipped
            },
            if remapped > 0 {
                "remappedRows"
            } else {
                "unchanged"
            },
            None,
            None,
        );
        break;
    }

    let (summary, next_baseline) = tracker.finish();
    session_sync::save_baseline(BACKUP_PLATFORM_LABEL, target_uid, &next_baseline);
    report.sync = summary;

    eprintln!(
        "[Buddy WB Transfer] 合并完成: source_uid={}, target_uid={}, workspaces={}, added={}, replaced={}, db_rows={}, sessions={}, copied={}, skipped={}, conflict={}, partial={}, failed={}",
        source_uid,
        target_uid,
        report.scanned_workspaces,
        report.added_conversations,
        report.replaced_conversations,
        report.updated_session_rows,
        report.sync.total,
        report.sync.copied,
        report.sync.skipped,
        report.sync.conflict,
        report.sync.partial,
        report.sync.failed
    );

    Ok(report)
}
