use super::*;

/// 进程级互斥：同一时刻只允许一次 CodeBuddy CN 会话合并
pub(super) static TRANSFER_LOCK: std::sync::LazyLock<std::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(()));

/// 会话备份目录的分类名（`{data_dir}/buddy/session-backup/<本值>/<uid>`）。
/// 同时是同步基线 / 待处理冲突文件的分类名（三处共用同一个 label）。
pub(crate) const BACKUP_PLATFORM_LABEL: &str = "codebuddy-cn";

/// 与「切号合并」共用同一把进程级锁：两边动的是同一批会话目录，必须互斥。
pub(crate) fn transfer_lock() -> &'static std::sync::Mutex<()> {
    &TRANSFER_LOCK
}

/// 工作区目录名 = **MD5(项目路径)**，口径与客户端一致（Windows：小写盘符 + 反斜杠）。
///
/// 反向解析（哈希 → 路径）见 [`resolve_workspace_display`]；这里只需要正向。
pub(crate) fn workspace_hash_for_path(path: &str) -> String {
    format!(
        "{:x}",
        md5::compute(normalize_workspace_path(path).as_bytes())
    )
}

/// 规范成哈希口径：去掉首尾空白与尾部分隔符；Windows 上盘符转小写、分隔符统一为反斜杠。
///
/// 用户从目录选择器里拿到的是 `E:\pro\x`（大写盘符），客户端落盘用的是 `e:\pro\x`
/// —— 不规范化就会算出另一个哈希，搬过去客户端照样找不到。
pub(crate) fn normalize_workspace_path(path: &str) -> String {
    let trimmed = path.trim().trim_end_matches(['/', '\\']).to_string();
    if !cfg!(target_os = "windows") {
        return trimmed;
    }
    let mut out = trimmed.replace('/', "\\");
    let mut chars = out.chars();
    if let (Some(drive), Some(colon)) = (chars.next(), chars.next()) {
        if colon == ':' && drive.is_ascii_alphabetic() {
            out.replace_range(0..1, &drive.to_ascii_lowercase().to_string());
        }
    }
    out
}

/// 把「哈希 → 真实路径」立刻填进缓存。
///
/// 不改的话，目标目录若从没在 IDE 里打开过，`resolve_workspace_display` 还原不出路径，
/// 界面上这个会话的目录会退化成一串 32 位哈希。
pub(crate) fn cache_workspace_path(workspace_hash: &str, path: &str) {
    if let Ok(mut cache) = WORKSPACE_PATH_CACHE.lock() {
        cache.insert(workspace_hash.to_string(), Some(path.to_string()));
    }
}

/// 辅助数据目录类型（复刻 cockpit-tools 的 kinds 清单）
pub(crate) const AUXILIARY_KINDS: [&str; 5] = [
    "check-point",
    "file-tree",
    "plan-task",
    "genie-cache",
    "connectors",
];

/// 在写入目标账号 state.vscdb 之前，合并来源账号的本地会话到目标账号。
pub fn transfer_on_switch(
    target: &BuddyAccount,
    progress: TransferProgress,
) -> Result<Option<SessionTransferReport>, String> {
    let uid = target
        .uid
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "目标 CodeBuddy CN 账号缺少 uid，无法执行会话合并".to_string())?;

    let current = match buddy_current_account(BuddyPlatform::CodebuddyCn, &[target.id.clone()]) {
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
    let user_data_dir = codebuddy_cn_user_data_dir()?;
    let report = transfer_local_sessions(&user_data_dir, source_uid, uid, progress)?;
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
        BuddyPlatform::CodebuddyCn => {
            crate::commands::buddy::codebuddy_cn::resolve_current_account_id(&accounts)
        }
    };
    output
        .filter(|id| !except_ids.iter().any(|except| except == id))
        .and_then(|id| accounts.iter().find(|account| account.id == id).cloned())
}

pub(super) fn codebuddy_cn_user_data_dir() -> Result<std::path::PathBuf, String> {
    crate::commands::buddy::codebuddy_cn::default_data_dir()
        .ok_or_else(|| "无法定位 CodeBuddy CN 数据目录".to_string())
}

/// 路径是否为重解析点（junction / 符号链接 / 挂载点）。
///
/// 不能只看 `file_type().is_symlink()`：Windows junction 的 `is_symlink()` 不可靠，
/// 必须读 `FILE_ATTRIBUTE_REPARSE_POINT` 属性位（2026-10-20 实测 Python/Rust 两边都踩过）。
pub(crate) fn is_reparse_point(path: &Path) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return false;
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink()
}

/// 目标账号的会话目录是不是指向共享目录的链接。
///
/// 共享模式下所有账号读写同一份会话数据，切号**不需要复制**（复制只会把链接物化成真实目录）。
/// 目录名不硬编码：`<Data>/<uid>/<IDE>/<uid>/history` 的 IDE 层靠扫描发现。
///
/// WorkBuddy 也复用这个判定：两个平台共用同一批 uid，WorkBuddy 切号的「旧版扩展目录」探测
/// 会命中 CN 的共享 junction，不短路就会拿 CN 会话去合并（2026-10-20 实测）。
pub(crate) fn target_sessions_shared(extension_data_dir: &Path, target_uid: &str) -> bool {
    let account_outer = extension_data_dir.join(target_uid);
    let Ok(entries) = std::fs::read_dir(&account_outer) else {
        return false;
    };
    for entry in entries.flatten() {
        let history = entry.path().join(target_uid).join("history");
        if is_reparse_point(&history) {
            return true;
        }
    }
    false
}
