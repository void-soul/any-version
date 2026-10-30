use super::*;

/// 会话备份目录的分类名（`{data_dir}/buddy/session-backup/<本值>/<uid>`）
pub(super) const BACKUP_PLATFORM_LABEL: &str = "workbuddy";

/// 进程级互斥：同一时刻只允许一次 WorkBuddy 会话合并
pub(super) static TRANSFER_LOCK: LazyLock<std::sync::Mutex<()>> =
    LazyLock::new(|| std::sync::Mutex::new(()));

/// 在写入目标账号 auth 文件之前，合并来源账号的本地会话到目标账号。
pub fn transfer_on_switch(
    target: &BuddyAccount,
    progress: TransferProgress,
) -> Result<Option<SessionTransferReport>, String> {
    let uid = target
        .uid
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "目标 WorkBuddy 账号缺少 uid，无法执行会话合并".to_string())?;

    let current = match buddy_current_account(BuddyPlatform::Workbuddy, &[target.id.clone()]) {
        Some(current) => current,
        None => return Ok(None),
    };
    let source_uid = current
        .uid
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let Some(source_uid) = source_uid else {
        return Ok(None);
    };
    if source_uid == uid {
        return Ok(None);
    }

    emit_switch_progress(
        progress.app,
        progress.platform,
        progress.account_id,
        "merging",
        0,
        None,
    );
    let runtime_dir = runtime_dir_for_workbuddy()?;
    let report = transfer_local_sessions(&runtime_dir, source_uid, uid, progress)?;
    Ok(Some(report))
}

pub(super) fn buddy_current_account(
    platform: BuddyPlatform,
    except_ids: &[String],
) -> Option<BuddyAccount> {
    let accounts = store::list_accounts(platform);
    let current_id = store::get_current_account_id(platform);
    for account in &accounts {
        if current_id.as_deref() == Some(account.id.as_str()) {
            return Some(account.clone());
        }
    }
    let output = match platform {
        BuddyPlatform::Workbuddy => crate::commands::buddy::workbuddy::resolve_current_account_id(
            BuddyPlatform::Workbuddy,
            &accounts,
        ),
        // 该模块只服务 CN WorkBuddy 的会话迁移，其余平台不参与
        BuddyPlatform::CodebuddyCn => None,
    };
    output
        .filter(|id| !except_ids.iter().any(|except| except == id))
        .and_then(|id| accounts.iter().find(|account| account.id == id).cloned())
}

pub(super) fn runtime_dir_for_workbuddy() -> Result<std::path::PathBuf, String> {
    let data_dir = data_dir_for_workbuddy()?;
    let app_dir = data_dir.join("app");
    Ok(if app_dir.is_dir() { app_dir } else { data_dir })
}

pub(super) fn data_dir_for_workbuddy() -> Result<std::path::PathBuf, String> {
    dirs::home_dir()
        .map(|h| h.join(".workbuddy"))
        .ok_or_else(|| "无法确定 WorkBuddy 数据目录".to_string())
}

/// 解析 WorkBuddy 运行时目录：路径以 app 结尾 → (父目录=配置目录, 该目录=Electron 数据目录)；
/// 否则 → (该目录=配置目录, 子目录 app=Electron 数据目录)。复刻 resolve_workbuddy_runtime_dirs。
pub(super) fn resolve_runtime_dirs(
    runtime_dir: &Path,
) -> Result<(std::path::PathBuf, std::path::PathBuf), String> {
    let raw = runtime_dir.to_string_lossy().trim().to_string();
    if raw.is_empty() {
        return Err("WorkBuddy 实例目录为空".to_string());
    }
    let path = std::path::PathBuf::from(raw);
    if path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.eq_ignore_ascii_case("app"))
    {
        let config_dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(std::path::PathBuf::from)
            .ok_or_else(|| "无法从 Electron 目录推导 WorkBuddy 配置目录".to_string())?;
        return Ok((config_dir, path));
    }
    Ok((path.clone(), path.join("app")))
}
