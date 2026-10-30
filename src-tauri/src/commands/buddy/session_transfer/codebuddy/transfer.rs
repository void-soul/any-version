use super::*;

pub(super) fn transfer_local_sessions(
    user_data_dir: &Path,
    source_uid: &str,
    target_uid: &str,
    progress: TransferProgress,
) -> Result<SessionTransferReport, String> {
    validate_uid(source_uid)?;
    validate_uid(target_uid)?;
    if source_uid == target_uid {
        return Ok(SessionTransferReport::default());
    }

    let _guard = TRANSFER_LOCK
        .lock()
        .map_err(|_| "CodeBuddy CN 本地会话合并正在进行，请稍后重试".to_string())?;

    let extension_data_dir = codebuddy_extension_data_dir()?;
    let backup_root = crate::commands::buddy::session_transfer::prepare_backup_root(
        BACKUP_PLATFORM_LABEL,
        target_uid,
    )?;

    let mut report = SessionTransferReport::default();
    // 上次同步基线（目标账号维度）：用于"只处理有变化的会话"与"两侧都改过即冲突"的判定。
    let baseline =
        crate::commands::buddy::session_sync::load_baseline(BACKUP_PLATFORM_LABEL, target_uid);
    let mut tracker = SyncTracker::new(baseline);
    let mut pending_conflicts: Vec<PendingConflict> = Vec::new();

    // ── 接管目标账号的会话目录（共享根已建立时）──
    // 客户端给新账号首次登录时会建一套真实目录。不在这里先换成链接，下面的复制分支
    // 就会把整份共享会话搬给这个账号，共享模式当场退化成「每个账号一份副本」。
    // 接管只把共享根缺少的会话并进去，幂等；共享根为空（用户没启用共享）时返回 false，
    // 走原来的逐会话合并逻辑。
    if ensure_shared_target(&extension_data_dir, target_uid)? {
        eprintln!(
            "[Buddy CN Transfer] 账号 {} 接入共享会话目录（只并独有会话，其余只建链接）",
            target_uid
        );
    }

    // ── 共享目录模式（junction / symlink）──
    // 所有账号读写同一份会话数据，切号不需要复制；而且 `replace_dir_atomic` 的
    // 「临时目录 + rename」会把链接物化成真实目录（2026-10-20 真机实证）。
    // 这里只改 vscdb 归属：客户端按 `userId` 过滤列表。
    if target_sessions_shared(&extension_data_dir, target_uid) {
        report.updated_session_rows = remap_session_vscdb_user_id(
            &user_data_dir.join("codebuddy-sessions.vscdb"),
            source_uid,
            target_uid,
            &backup_root,
        )?;
        let (summary, next_baseline) = tracker.finish();
        crate::commands::buddy::session_sync::save_baseline(
            BACKUP_PLATFORM_LABEL,
            target_uid,
            &next_baseline,
        );
        crate::commands::buddy::session_sync::save_pending_conflicts(
            BACKUP_PLATFORM_LABEL,
            target_uid,
            &pending_conflicts,
        );
        report.sync = summary;
        eprintln!(
            "[Buddy CN Transfer] 目标账号 {} 的会话目录是共享链接，跳过复制（只 remap userId={}）",
            target_uid, report.updated_session_rows
        );
        return Ok(report);
    }

    report.scanned_workspaces = sync_history_between_accounts(
        &extension_data_dir,
        source_uid,
        target_uid,
        &backup_root,
        progress,
        &mut tracker,
        &mut pending_conflicts,
        ConflictPolicy::Resolvable,
    )?;
    report.updated_session_rows = remap_session_vscdb_user_id(
        &user_data_dir.join("codebuddy-sessions.vscdb"),
        source_uid,
        target_uid,
        &backup_root,
    )?;
    if report.updated_session_rows > 0 {
        // 这是整库汇总行（id 为空），不归属任何工作区，显式给 None，
        // 免得继承到别处设定的「当前工作区」上下文
        tracker.record_in(
            "",
            "codebuddy-sessions.vscdb",
            SessionSyncStatus::Copied,
            "remappedRows",
            None,
            None,
        );
    }

    let (summary, next_baseline) = tracker.finish();
    crate::commands::buddy::session_sync::save_baseline(
        BACKUP_PLATFORM_LABEL,
        target_uid,
        &next_baseline,
    );
    // 冲突落盘（空列表 = 清掉旧文件）：切换结束后用户仍可逐条裁决
    crate::commands::buddy::session_sync::save_pending_conflicts(
        BACKUP_PLATFORM_LABEL,
        target_uid,
        &pending_conflicts,
    );
    report.sync = summary;

    eprintln!(
        "[Buddy CN Transfer] 合并完成: source_uid={}, target_uid={}, workspaces={}, sessions={}, copied={}, skipped={}, conflict={}, partial={}, failed={}, db_rows={}",
        source_uid,
        target_uid,
        report.scanned_workspaces,
        report.sync.total,
        report.sync.copied,
        report.sync.skipped,
        report.sync.conflict,
        report.sync.partial,
        report.sync.failed,
        report.updated_session_rows
    );

    Ok(report)
}

pub(crate) fn codebuddy_extension_data_dir() -> Result<std::path::PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "无法获取用户主目录".to_string())?;
    #[cfg(target_os = "windows")]
    let root = home
        .join("AppData")
        .join("Local")
        .join("CodeBuddyExtension")
        .join("Data");
    #[cfg(target_os = "macos")]
    let root = home
        .join("Library")
        .join("Application Support")
        .join("CodeBuddyExtension")
        .join("Data");
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    let root = home
        .join(".local")
        .join("share")
        .join("CodeBuddyExtension")
        .join("Data");
    Ok(root)
}
