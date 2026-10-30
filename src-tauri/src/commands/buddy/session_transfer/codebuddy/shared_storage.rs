use super::*;

/// 共享模式下的会话分类目录（与 `scripts/migrate-shared-sessions.py` 的 KINDS 一致）。
///
/// **只含 `<uid>/CodeBuddyIDE/<uid>/` 这一层的 4 类**。不能照搬 [`AUXILIARY_KINDS`]：
/// 2026-10-20 实测，`genie-cache` 的真实位置是 `<uid>/CodeBuddyIDE/genie-cache`（**少一层 uid**，
/// 20 个账号下各有 10~34 项真实数据），`connectors` 在 `CodeBuddyIDE` 层根本不存在
/// （ConnectorService 读 `<uid>/CodeBuddyIDE/connectors/connector-servers.json`，日志是 ENOENT）。
/// 把这两类建在 uid 层只会得到客户端永不读取的空链接。
pub(super) const SHARED_KINDS: [&str; 4] = ["history", "check-point", "file-tree", "plan-task"];

/// 接管时原目录的改名后缀（`history.kira-shared-bak`）。
pub(super) const TAKEOVER_BAK_SUFFIX: &str = ".kira-shared-bak";

/// 共享根：所有账号共用同一份会话数据，放在本软件数据目录下。
///
/// 布局与 `scripts/migrate-shared-sessions.py` 完全一致：
/// `<data_dir>/buddy/shared-sessions/codebuddy-cn/<kind>`，每个账号的
/// `<...>/<uid>/CodeBuddyIDE/<uid>/<kind>` 是指向它的 junction。
pub(super) fn shared_sessions_root() -> std::path::PathBuf {
    crate::commands::config::get_data_dir()
        .join("buddy")
        .join("shared-sessions")
        .join(BACKUP_PLATFORM_LABEL)
}

/// 共享根里是否已经有会话数据（空目录 = 用户还没迁移，绝不能接管）。
pub(super) fn shared_root_ready(shared_root: &Path) -> bool {
    std::fs::read_dir(shared_root.join("history"))
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
}

/// 该账号的 `kind` 目录是否已经是指向共享根的链接。
pub(super) fn kind_links_to(account_root: &Path, shared_root: &Path, kind: &str) -> bool {
    let path = account_root.join(kind);
    if !is_reparse_point(&path) {
        return false;
    }
    match (
        std::fs::canonicalize(&path),
        std::fs::canonicalize(shared_root.join(kind)),
    ) {
        (Ok(actual), Ok(expected)) => actual == expected,
        _ => false,
    }
}

/// 该账号的 history 是否已经是指向共享根的链接。
pub(super) fn history_links_to(account_root: &Path, shared_root: &Path) -> bool {
    kind_links_to(account_root, shared_root, "history")
}

/// 4 类目录是否**全部**已接入共享根。
///
/// 只看 history 会把「只链了 history、其余 3 类是真实目录」的半接管账号判为已就绪：
/// 切号短路随即跳过合并，那 3 类就永久留在账号目录里各存一份（客户端会照常写入）。
pub(super) fn all_shared_kinds_linked(account_root: &Path, shared_root: &Path) -> bool {
    SHARED_KINDS
        .iter()
        .all(|kind| kind_links_to(account_root, shared_root, kind))
}

/// 从现有账号目录里探测 IDE 层目录名（客户端固定叫 `CodeBuddyIDE`，探测优先）。
pub(super) fn discover_ide_names(extension_data_dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    if let Ok(accounts) = std::fs::read_dir(extension_data_dir) {
        for account in accounts.flatten() {
            let Ok(ides) = std::fs::read_dir(account.path()) else {
                continue;
            };
            for ide in ides.flatten() {
                if !ide.path().is_dir() || is_reparse_point(&ide.path()) {
                    continue;
                }
                let name = ide.file_name().to_string_lossy().to_string();
                if !names
                    .iter()
                    .any(|existing| existing.eq_ignore_ascii_case(&name))
                {
                    names.push(name);
                }
            }
        }
    }
    if names.is_empty() {
        names.push("CodeBuddyIDE".to_string());
    }
    names
}

/// 把来源工作区 index.json 里共享根缺少的会话条目并进去（共享根为准，不覆盖已有条目）。
pub(super) fn merge_index_into_shared(
    shared_workspace: &Path,
    source_workspace: &Path,
) -> Result<(), String> {
    let source_index_path = source_workspace.join("index.json");
    if !source_index_path.is_file() {
        return Ok(());
    }
    let source_index = read_workspace_index(&source_index_path)?;
    let shared_index_path = shared_workspace.join("index.json");
    let mut shared_index = if shared_index_path.is_file() {
        read_workspace_index(&shared_index_path)?
    } else {
        serde_json::json!({ "conversations": [], "current": "" })
    };
    let mut merged = conversations(&shared_index);
    let mut known: std::collections::HashSet<String> = merged
        .iter()
        .filter_map(conversation_id)
        .map(str::to_string)
        .collect();
    let mut added = 0usize;
    for conversation in conversations(&source_index) {
        let Some(id) = conversation_id(&conversation) else {
            continue;
        };
        if known.insert(id.to_string()) {
            merged.push(conversation);
            added += 1;
        }
    }
    if added == 0 {
        return Ok(());
    }
    shared_index["conversations"] = Value::Array(merged);
    write_workspace_index(&shared_index_path, &shared_index)
}

/// 把某个账号目录里**共享根没有的**会话并入共享根，然后整体换成 junction。
///
/// 只搬共享根缺少的会话：共享根是权威副本，已存在的同名会话一律以共享根为准，
/// 因此这个操作是幂等的，重复执行不会产生任何复制。
pub(super) fn adopt_account_into_shared(
    account_root: &Path,
    shared_root: &Path,
) -> Result<usize, String> {
    let mut adopted = 0usize;
    for kind in SHARED_KINDS {
        let source = account_root.join(kind);
        if !source.is_dir() || is_reparse_point(&source) {
            continue;
        }
        let destination = shared_root.join(kind);
        std::fs::create_dir_all(&destination).map_err(|e| {
            format!(
                "创建共享会话目录失败: path={}, error={}",
                destination.display(),
                e
            )
        })?;
        for workspace in std::fs::read_dir(&source).map_err(|e| {
            format!(
                "读取账号会话目录失败: path={}, error={}",
                source.display(),
                e
            )
        })? {
            let workspace = workspace.map_err(|e| format!("读取账号会话子目录失败: {}", e))?;
            let workspace_source = workspace.path();
            if !workspace_source.is_dir() {
                continue;
            }
            let workspace_destination = destination.join(workspace.file_name());
            std::fs::create_dir_all(&workspace_destination)
                .map_err(|e| format!("创建共享工作区目录失败: {}", e))?;
            for conversation in std::fs::read_dir(&workspace_source)
                .map_err(|e| format!("读取账号会话条目失败: {}", e))?
            {
                let conversation =
                    conversation.map_err(|e| format!("读取账号会话条目失败: {}", e))?;
                let conversation_path = conversation.path();
                if !conversation_path.is_dir() {
                    continue;
                }
                let target = workspace_destination.join(conversation.file_name());
                if target.exists() {
                    continue;
                }
                copy_dir_atomic(&conversation_path, &target)?;
                adopted += 1;
            }
            merge_index_into_shared(&workspace_destination, &workspace_source)?;
        }
    }

    // 数据已经并进共享根，再换链接：先改名（同盘瞬间），建好 junction 才删备份。
    for kind in SHARED_KINDS {
        let source = account_root.join(kind);
        if is_reparse_point(&source) {
            continue;
        }
        let target = shared_root.join(kind);
        std::fs::create_dir_all(&target).map_err(|e| {
            format!(
                "创建共享会话目录失败: path={}, error={}",
                target.display(),
                e
            )
        })?;
        let mut backup = None;
        if source.exists() {
            let path = account_root.join(format!("{}{}", kind, TAKEOVER_BAK_SUFFIX));
            if path.exists() {
                std::fs::remove_dir_all(&path)
                    .map_err(|e| format!("清理上次接管的备份失败: {}: {}", path.display(), e))?;
            }
            std::fs::rename(&source, &path).map_err(|e| {
                format!(
                    "暂存账号会话目录失败: from={}, to={}, error={}",
                    source.display(),
                    path.display(),
                    e
                )
            })?;
            backup = Some(path);
        }
        if let Err(error) = crate::commands::cache::create_junction(&source, &target) {
            if let Some(backup) = &backup {
                let _ = std::fs::rename(backup, &source);
            }
            return Err(format!(
                "建立共享会话链接失败: link={}, target={}, error={}",
                source.display(),
                target.display(),
                error
            ));
        }
        if let Some(backup) = backup {
            if let Err(error) = std::fs::remove_dir_all(&backup) {
                // 删不掉不影响正确性（链接已就位，数据已在共享根）：留给下次接管清理
                eprintln!(
                    "[Buddy CN Transfer] 接管备份清理失败（下次切号重试）: {}: {}",
                    backup.display(),
                    error
                );
            }
        }
    }
    Ok(adopted)
}

/// 确保目标账号读写的会话目录就是共享根，返回 `true` 表示「已处于共享模式」。
///
/// 客户端给新账号首次登录时会建一套真实目录；不接管的话，第一次切号就会把
/// 整份共享会话复制给这个账号，机制当场退化。所以这里在切号前（客户端已退出）
/// 把独有会话并进共享根、再把目录换成 junction。
///
/// 返回 `false` 只有一种情况：共享根还没建立/为空 —— 即用户没启用共享，
/// 调用方按旧逻辑逐会话合并。
pub(super) fn ensure_shared_target(
    extension_data_dir: &Path,
    target_uid: &str,
) -> Result<bool, String> {
    ensure_shared_target_at(extension_data_dir, target_uid, &shared_sessions_root())
}

pub(super) fn ensure_shared_target_at(
    extension_data_dir: &Path,
    target_uid: &str,
    shared_root: &Path,
) -> Result<bool, String> {
    if !shared_root_ready(shared_root) {
        return Ok(false);
    }
    validate_uid(target_uid)?;
    let account_outer = extension_data_dir.join(target_uid);
    let mut ready = false;
    for ide in discover_ide_names(extension_data_dir) {
        let account_root = account_outer.join(&ide).join(target_uid);
        if !account_root.is_dir() {
            // 账号还没被客户端建目录：直接铺一套 junction，一条会话都不用复制
            std::fs::create_dir_all(&account_root)
                .map_err(|e| format!("创建账号会话根目录失败: {}", e))?;
        } else if all_shared_kinds_linked(&account_root, shared_root) {
            ready = true;
            continue;
        } else if !is_reparse_point(&account_root.join("history"))
            && !account_root.join("history").exists()
            && std::fs::read_dir(&account_root)
                .map(|mut entries| entries.next().is_none())
                .unwrap_or(false)
        {
            // 空账号目录：没什么可搬的，直接铺链接
        }
        let adopted = adopt_account_into_shared(&account_root, shared_root)?;
        eprintln!(
            "[Buddy CN Transfer] 账号 {} 接入共享会话目录: 并入独有会话 {} 条，其余只建链接（不复制）",
            target_uid, adopted
        );
        ready = true;
    }
    Ok(ready)
}
