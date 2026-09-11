//! Buddy 模块会话合并共享引擎 + CodeBuddy CN 入口。
//!
//! 移植自 cockpit-tools `modules/codebuddy_session_transfer.rs`：
//! - `sync_history_between_accounts`：按 `<uid>/<IDE>/<uid>/history/<workspace>` 布局逐工作区合并
//!   会话目录与辅助目录（check-point / file-tree / plan-task / genie-cache / connectors），
//!   并合并 `index.json`（按 conversationId 去重、按 lastMessageAt 取新、保留/更新 current）。
//! - `remap_session_vscdb_user_id`：`codebuddy-sessions.vscdb` 中 `session:%` 记录的 userId 重映射。
//! WorkBuddy（`workbuddy.rs`）同样复用这两个函数（与 cockpit-tools 的依赖关系一致）。

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Component, Path};

use serde_json::Value;

use super::super::models::{BuddyAccount, BuddyPlatform};
use super::super::store;
use super::SessionTransferReport;
use super::TransferProgress;
use crate::commands::buddy::emit_switch_progress;

/// 进程级互斥：同一时刻只允许一次 CodeBuddy CN 会话合并
static TRANSFER_LOCK: std::sync::LazyLock<std::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(()));

/// 辅助数据目录类型（复刻 cockpit-tools 的 kinds 清单）
const AUXILIARY_KINDS: [&str; 5] = [
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

    emit_switch_progress(progress.app, progress.platform, progress.account_id, "merging", 0, None);
    let user_data_dir = codebuddy_cn_user_data_dir()?;
    let report = transfer_local_sessions(&user_data_dir, source_uid, uid, progress)?;
    Ok(Some(report))
}

fn buddy_current_account(platform: BuddyPlatform, except_ids: &[String]) -> Option<BuddyAccount> {
    let accounts = store::list_accounts(platform);
    let current_id = store::get_current_account_id(platform);
    for account in &accounts {
        if current_id.as_deref() == Some(account.id.as_str()) {
            return Some(account.clone());
        }
    }
    let output = match platform {
        BuddyPlatform::Workbuddy => super::super::workbuddy::resolve_current_account_id(&accounts),
        BuddyPlatform::CodebuddyCn => super::super::codebuddy_cn::resolve_current_account_id(&accounts),
    };
    output.filter(|id| !except_ids.iter().any(|except| except == id))
        .and_then(|id| accounts.iter().find(|account| account.id == id).cloned())
}

fn codebuddy_cn_user_data_dir() -> Result<std::path::PathBuf, String> {
    super::super::codebuddy_cn::default_data_dir()
        .ok_or_else(|| "无法定位 CodeBuddy CN 数据目录".to_string())
}

fn transfer_local_sessions(
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
    let backup_root = user_data_dir
        .parent()
        .map(|p| p.join(".kira-codebuddy-session-backup").join(target_uid))
        .unwrap_or_else(|| {
            std::path::PathBuf::from(".")
                .join(".kira-codebuddy-session-backup")
                .join(target_uid)
        });
    std::fs::create_dir_all(&backup_root).map_err(|e| {
        format!(
            "创建 CodeBuddy CN 会话备份目录失败: path={}, error={}",
            backup_root.display(),
            e
        )
    })?;

    let mut report =
        sync_history_between_accounts(&extension_data_dir, source_uid, target_uid, &backup_root, progress)?;
    report.updated_session_rows = remap_session_vscdb_user_id(
        &user_data_dir.join("codebuddy-sessions.vscdb"),
        source_uid,
        target_uid,
        &backup_root,
    )?;

    eprintln!(
        "[Buddy CN Transfer] 合并完成: source_uid={}, target_uid={}, workspaces={}, added={}, replaced={}, db_rows={}",
        source_uid,
        target_uid,
        report.scanned_workspaces,
        report.added_conversations,
        report.replaced_conversations,
        report.updated_session_rows
    );

    Ok(report)
}

fn codebuddy_extension_data_dir() -> Result<std::path::PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "无法获取用户主目录".to_string())?;
    #[cfg(target_os = "windows")]
    let root = home.join("AppData").join("Local").join("CodeBuddyExtension").join("Data");
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

pub(super) fn validate_uid(uid: &str) -> Result<(), String> {
    let trimmed = uid.trim();
    if trimmed.is_empty() || trimmed != uid {
        return Err("CodeBuddy CN UID 为空或包含首尾空白".to_string());
    }
    if uid.contains('/') || uid.contains('\\') || uid.contains("..") || uid.contains('\0') {
        return Err("CodeBuddy CN UID 包含不安全的路径字符".to_string());
    }
    let mut components = Path::new(uid).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Ok(()),
        _ => Err("CodeBuddy CN UID 不是安全的单级路径".to_string()),
    }
}

/// 合并来源账号 history 目录到目标账号（逐 IDE、逐工作区）。
/// 返回完整报告（新增/替换会话数 + 扫描工作区数），WorkBuddy 也复用本函数。
pub(super) fn sync_history_between_accounts(
    extension_data_dir: &Path,
    source_uid: &str,
    target_uid: &str,
    backup_root: &Path,
    progress: TransferProgress,
) -> Result<SessionTransferReport, String> {
    let source_outer = extension_data_dir.join(source_uid);
    if !source_outer.is_dir() {
        return Ok(SessionTransferReport::default());
    }
    reject_symlink_if_exists(&source_outer)?;

    let target_outer = extension_data_dir.join(target_uid);
    if target_outer.exists() {
        reject_symlink_if_exists(&target_outer)?;
    }

    let mut report = SessionTransferReport::default();
    let ide_entries = std::fs::read_dir(&source_outer).map_err(|e| {
        format!(
            "读取 CodeBuddy CN 会话根目录失败: path={}, error={}",
            source_outer.display(),
            e
        )
    })?;

    for ide_entry in ide_entries {
        let ide_entry =
            ide_entry.map_err(|e| format!("读取 CodeBuddy CN IDE 目录失败: {}", e))?;
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
            let delta = merge_workspace_history(
                &source_workspace,
                &target_workspace,
                &source_account_root,
                &target_account_root,
                &workspace_name,
                &workspace_backup,
            )?;
            report.scanned_workspaces += 1;
            report.added_conversations += delta.added_conversations;
            report.replaced_conversations += delta.replaced_conversations;
            emit_switch_progress(
                progress.app,
                progress.platform,
                progress.account_id,
                "merging",
                report.scanned_workspaces,
                None,
            );
        }
    }
    Ok(report)
}

/// 合并单个工作区：会话目录 + index.json（去重、取新、保留 current）+ 辅助目录。
fn merge_workspace_history(
    source_workspace: &Path,
    target_workspace: &Path,
    source_account_root: &Path,
    target_account_root: &Path,
    workspace_name: &std::ffi::OsStr,
    backup_root: &Path,
) -> Result<SessionTransferReport, String> {
    let source_index_path = source_workspace.join("index.json");
    if !source_index_path.is_file() {
        return Ok(SessionTransferReport::default());
    }
    let source_index = read_workspace_index(&source_index_path)?;

    // 目标工作区不存在：整目录原子复制（含全部会话子目录）+ 辅助目录
    if !target_workspace.exists() {
        copy_dir_atomic(source_workspace, target_workspace)?;
        copy_auxiliary_workspace_roots(source_account_root, target_account_root, workspace_name)?;
        return Ok(SessionTransferReport {
            added_conversations: conversations(&source_index).len(),
            ..SessionTransferReport::default()
        });
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

    let mut report = SessionTransferReport::default();
    let mut index_backup_created = false;
    for source_conversation in source_conversations {
        let Some(id) = conversation_id(&source_conversation) else {
            continue;
        };
        validate_conversation_id(id)?;
        let source_conversation_dir = source_workspace.join(id);
        if !source_conversation_dir.is_dir() {
            eprintln!(
                "[Buddy CodeBuddy CN Transfer] 来源会话目录不存在，已跳过: {}",
                source_conversation_dir.display()
            );
            continue;
        }

        match target_positions.get(id).copied() {
            None => {
                ensure_workspace_index_backup(&target_index_path, backup_root, &mut index_backup_created)?;
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
                target_positions.insert(id.to_string(), merged.len());
                merged.push(source_conversation);
                report.added_conversations += 1;
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
                // 目标会话目录缺失，或来源更新：整体替换（复刻 cockpit-tools 判定）
                if !target_conversation_dir.is_dir()
                    || conversation_is_newer(&source_conversation, &merged[target_index_pos])
                {
                    ensure_workspace_index_backup(&target_index_path, backup_root, &mut index_backup_created)?;
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
                    merged[target_index_pos] = source_conversation;
                    report.replaced_conversations += 1;
                    index_changed = true;
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
                object.insert("current".to_string(), Value::String(source_current.to_string()));
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

    Ok(report)
}

fn ensure_workspace_index_backup(
    target_index_path: &Path,
    backup_root: &Path,
    backup_created: &mut bool,
) -> Result<(), String> {
    if *backup_created {
        return Ok(());
    }
    std::fs::create_dir_all(backup_root).map_err(|e| {
        format!(
            "创建 CodeBuddy CN 会话备份目录失败: path={}, error={}",
            backup_root.display(),
            e
        )
    })?;
    std::fs::copy(target_index_path, backup_root.join("index.json")).map_err(|e| {
        format!(
            "备份 CodeBuddy CN 工作区索引失败: path={}, error={}",
            target_index_path.display(),
            e
        )
    })?;
    *backup_created = true;
    Ok(())
}

fn read_workspace_index(path: &Path) -> Result<Value, String> {
    let content = std::fs::read_to_string(path).map_err(|e| {
        format!(
            "读取 CodeBuddy CN 工作区索引失败: path={}, error={}",
            path.display(),
            e
        )
    })?;
    let value: Value = serde_json::from_str(&content).map_err(|e| {
        format!(
            "解析 CodeBuddy CN 工作区索引失败: path={}, error={}",
            path.display(),
            e
        )
    })?;
    if !value.is_object()
        || value
            .get("conversations")
            .map(|v| !v.is_array())
            .unwrap_or(false)
    {
        return Err(format!("CodeBuddy CN 工作区索引结构无效: {}", path.display()));
    }
    Ok(value)
}

fn conversations(index: &Value) -> Vec<Value> {
    index
        .get("conversations")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn conversation_id(conversation: &Value) -> Option<&str> {
    conversation.get("id").and_then(Value::as_str)
}

fn validate_conversation_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
        return Err("CodeBuddy CN conversationId 包含不安全的路径字符".to_string());
    }
    Ok(())
}

fn conversation_is_newer(source: &Value, target: &Value) -> bool {
    compare_conversation_recency(source, target) == Ordering::Greater
}

fn compare_conversation_recency(left: &Value, right: &Value) -> Ordering {
    conversation_timestamp(left).cmp(&conversation_timestamp(right))
}

fn conversation_timestamp(conversation: &Value) -> i64 {
    let value = conversation.get("lastMessageAt");
    if let Some(timestamp) = value.and_then(Value::as_i64) {
        return timestamp;
    }
    value
        .and_then(Value::as_str)
        .and_then(|ts| chrono::DateTime::parse_from_rfc3339(ts).ok())
        .map(|ts| ts.timestamp_millis())
        .unwrap_or_default()
}

fn reject_symlink_if_exists(path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "拒绝通过符号链接读写 CodeBuddy CN 会话: {}",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!(
            "读取 CodeBuddy CN 会话路径属性失败: path={}, error={}",
            path.display(),
            e
        )),
    }
}

/// 工作区级辅助目录复制（check-point/file-tree/plan-task/genie-cache/connectors）
fn copy_auxiliary_workspace_roots(
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
fn copy_auxiliary_conversation(
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

fn temp_name(prefix: &str) -> String {
    format!(
        ".kira-session-{}-{}-{}",
        prefix,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}

fn copy_dir_atomic(source: &Path, target: &Path) -> Result<(), String> {
    if target.exists() {
        return Ok(());
    }
    let parent = target
        .parent()
        .ok_or_else(|| format!("无法定位目标目录: {}", target.display()))?;
    std::fs::create_dir_all(parent).map_err(|e| {
        format!(
            "创建 CodeBuddy CN 会话目标目录失败: path={}, error={}",
            parent.display(),
            e
        )
    })?;
    let tmp = parent.join(temp_name("tmp"));
    let result = copy_dir_recursive(source, &tmp).and_then(|_| {
        std::fs::rename(&tmp, target).map_err(|e| {
            format!(
                "提交 CodeBuddy CN 会话目录失败: from={}, to={}, error={}",
                tmp.display(),
                target.display(),
                e
            )
        })
    });
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    result
}

fn replace_dir_atomic(source: &Path, target: &Path) -> Result<(), String> {
    let parent = target
        .parent()
        .ok_or_else(|| format!("无法定位目标目录: {}", target.display()))?;
    std::fs::create_dir_all(parent).map_err(|e| format!("创建目标目录失败: {}", e))?;
    let tmp = parent.join(temp_name("replace"));
    copy_dir_recursive(source, &tmp)?;
    let old = parent.join(temp_name("old"));
    if target.exists() {
        std::fs::rename(target, &old).map_err(|e| {
            format!(
                "暂存旧 CodeBuddy CN 会话目录失败: from={}, to={}, error={}",
                target.display(),
                old.display(),
                e
            )
        })?;
    }
    if let Err(e) = std::fs::rename(&tmp, target) {
        let _ = std::fs::remove_dir_all(&tmp);
        if old.exists() {
            let _ = std::fs::rename(&old, target);
        }
        return Err(format!(
            "替换 CodeBuddy CN 会话目录失败: path={}, error={}",
            target.display(),
            e
        ));
    }
    if old.exists() {
        let _ = std::fs::remove_dir_all(&old);
    }
    Ok(())
}

fn copy_dir_recursive(source: &Path, target: &Path) -> Result<(), String> {
    let source_metadata = std::fs::symlink_metadata(source)
        .map_err(|e| format!("读取源文件属性失败: {}", e))?;
    if source_metadata.file_type().is_symlink() {
        return Err(format!("拒绝复制符号链接: {}", source.display()));
    }
    if source_metadata.is_file() {
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("创建目标目录失败: {}", e))?;
        }
        std::fs::copy(source, target).map_err(|e| {
            format!(
                "复制 CodeBuddy CN 会话文件失败: from={}, to={}, error={}",
                source.display(),
                target.display(),
                e
            )
        })?;
        return Ok(());
    }
    if !source_metadata.is_dir() {
        return Err(format!(
            "不支持的 CodeBuddy CN 会话文件类型: {}",
            source.display()
        ));
    }
    std::fs::create_dir_all(target)
        .map_err(|e| format!("创建 CodeBuddy CN 会话目录失败: {}", e))?;
    for entry in std::fs::read_dir(source)
        .map_err(|e| format!("读取 CodeBuddy CN 会话目录失败: {}", e))?
    {
        let entry = entry.map_err(|e| format!("读取 CodeBuddy CN 会话条目失败: {}", e))?;
        copy_dir_recursive(&entry.path(), &target.join(entry.file_name()))?;
    }
    Ok(())
}

/// `codebuddy-sessions.vscdb` 的 `session:%` 记录 userId 重映射。
/// WorkBuddy 旧版 vscdb 合并也复用本函数。
pub(super) fn remap_session_vscdb_user_id(
    db_path: &Path,
    source_uid: &str,
    target_uid: &str,
    backup_root: &Path,
) -> Result<usize, String> {
    if !db_path.is_file() {
        return Ok(0);
    }
    reject_symlink_if_exists(db_path)?;
    let mut connection = rusqlite::Connection::open(db_path).map_err(|e| {
        format!(
            "打开 CodeBuddy CN 会话数据库失败: path={}, error={}",
            db_path.display(),
            e
        )
    })?;
    connection
        .busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| format!("设置 CodeBuddy CN 会话数据库超时失败: {}", e))?;

    let mut updates = Vec::new();
    {
        let mut statement = connection
            .prepare("SELECT key, value FROM ItemTable WHERE key LIKE 'session:%'")
            .map_err(|e| format!("读取 CodeBuddy CN 会话数据库失败: {}", e))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("查询 CodeBuddy CN 会话数据库失败: {}", e))?;
        for row in rows {
            let (key, raw_value) =
                row.map_err(|e| format!("读取 CodeBuddy CN 会话记录失败: {}", e))?;
            let Ok(mut value) = serde_json::from_str::<Value>(&raw_value) else {
                continue;
            };
            let Some(object) = value.as_object_mut() else {
                continue;
            };
            if object.get("userId").and_then(Value::as_str) != Some(source_uid) {
                continue;
            }
            object.insert("userId".to_string(), Value::String(target_uid.to_string()));
            let serialized = serde_json::to_string(&value)
                .map_err(|e| format!("序列化 CodeBuddy CN 会话记录失败: {}", e))?;
            updates.push((key, serialized));
        }
    }
    if updates.is_empty() {
        return Ok(0);
    }

    std::fs::create_dir_all(backup_root).map_err(|e| {
        format!(
            "创建 CodeBuddy CN 会话数据库备份目录失败: path={}, error={}",
            backup_root.display(),
            e
        )
    })?;
    std::fs::copy(db_path, backup_root.join("codebuddy-sessions.vscdb")).map_err(|e| {
        format!(
            "备份 CodeBuddy CN 会话数据库失败: path={}, error={}",
            db_path.display(),
            e
        )
    })?;

    let transaction = connection
        .transaction()
        .map_err(|e| format!("开启 CodeBuddy CN 会话数据库事务失败: {}", e))?;
    for (key, value) in &updates {
        transaction
            .execute(
                "UPDATE ItemTable SET value = ?1 WHERE key = ?2",
                rusqlite::params![value, key],
            )
            .map_err(|e| format!("更新 CodeBuddy CN 会话 userId 失败: {}", e))?;
    }
    transaction
        .commit()
        .map_err(|e| format!("提交 CodeBuddy CN 会话数据库事务失败: {}", e))?;
    Ok(updates.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_temp() -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "kira-codebuddy-transfer-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            id
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn no_progress() -> TransferProgress<'static> {
        TransferProgress {
            app: None,
            account_id: "",
            platform: BuddyPlatform::CodebuddyCn,
        }
    }

    #[test]
    fn rejects_unsafe_uids() {
        for uid in ["", "../a", "a/b", "a\\b", "a..b", " a"] {
            assert!(validate_uid(uid).is_err(), "uid should be rejected: {uid}");
        }
        assert!(validate_uid("384c6dd0-c1bc-4ae2-a0d0-f70350c62f7b").is_ok());
    }

    #[test]
    fn remaps_vscdb_user_id() {
        let dest = make_temp();
        let db_path = dest.join("codebuddy-sessions.vscdb");
        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                INSERT INTO ItemTable VALUES ('session:1', '{\"userId\":\"source\",\"messages\":[]}');
                INSERT INTO ItemTable VALUES ('session:2', '{\"userId\":\"source\",\"messages\":[]}');
                INSERT INTO ItemTable VALUES ('session:3', '{\"userId\":\"target\",\"messages\":[]}');
                INSERT INTO ItemTable VALUES ('session:4', '{\"userId\":\"other\",\"messages\":[]}');",
            )
            .unwrap();
        }

        let backup_root = dest.join("backup");
        let changed =
            remap_session_vscdb_user_id(&db_path, "source", "target", &backup_root).unwrap();
        assert_eq!(changed, 2);

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let remaining: String = conn
            .query_row(
                "SELECT value FROM ItemTable WHERE key = 'session:1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&remaining).unwrap();
        assert_eq!(value["userId"].as_str(), Some("target"));
        assert!(backup_root.join("codebuddy-sessions.vscdb").is_file());
    }

    #[test]
    fn missing_database_is_a_noop() {
        let dest = make_temp();
        assert_eq!(
            remap_session_vscdb_user_id(
                &dest.join("missing.vscdb"),
                "source",
                "target",
                &dest.join("backup")
            )
            .unwrap(),
            0
        );
    }

    #[test]
    fn merges_workspace_history_without_dropping_target_conversations() {
        let dest = make_temp();
        let data_root = dest.join("CodeBuddyExtension").join("Data");
        let source_account = data_root.join("source").join("VSCode").join("source");
        let target_account = data_root.join("target").join("VSCode").join("target");
        let source_history = source_account.join("history").join("workspace");
        let target_history = target_account.join("history").join("workspace");

        for (history, id, ts) in [
            (&source_history, "source-only", "2026-07-02T00:00:00Z"),
            (&target_history, "target-only", "2026-07-01T00:00:00Z"),
        ] {
            std::fs::create_dir_all(history.join(id).join("messages")).unwrap();
            std::fs::write(history.join(id).join("index.json"), br#"{"messages":[]}"#).unwrap();
            std::fs::write(
                history.join("index.json"),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "conversations": [{
                        "id": id,
                        "createdAt": ts,
                        "lastMessageAt": ts
                    }],
                    "current": id
                }))
                .unwrap(),
            )
            .unwrap();
        }
        std::fs::create_dir_all(source_account.join("check-point").join("workspace").join("source-only")).unwrap();

        let backup_root = dest.join("backup");
        let report = sync_history_between_accounts(
            &data_root,
            "source",
            "target",
            &backup_root,
            no_progress(),
        )
        .unwrap();
        assert_eq!(report.scanned_workspaces, 1);
        // 来源工作区在目标不存在 → 整体复制：conversation 目录本体必须存在
        assert!(target_history.join("source-only").is_dir());
        assert!(target_history.join("source-only").join("index.json").is_file());
        assert!(target_history.join("target-only").is_dir());
        // 辅助目录（check-point）也应被复制
        assert!(target_account.join("check-point").join("workspace").join("source-only").is_dir());

        let merged: serde_json::Value = serde_json::from_slice(
            &std::fs::read(target_history.join("index.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            merged.get("conversations").and_then(|v| v.as_array()).map(Vec::len),
            Some(2)
        );
    }

    #[test]
    fn copies_entire_workspace_dir_when_target_missing() {
        // 回归：目标工作区不存在时，必须复制会话目录本体（此前只复制了 index.json 导致合并失败）
        let dest = make_temp();
        let data_root = dest.join("CodeBuddyExtension").join("Data");
        let source_account = data_root.join("source").join("VSCode").join("source");
        let source_history = source_account.join("history").join("workspace");
        std::fs::create_dir_all(source_history.join("conv1").join("messages")).unwrap();
        std::fs::write(source_history.join("conv1").join("messages").join("0.json"), "[]").unwrap();
        std::fs::write(
            source_history.join("index.json"),
            serde_json::to_vec(&serde_json::json!({
                "conversations": [{"id": "conv1", "lastMessageAt": 123}],
                "current": "conv1"
            }))
            .unwrap(),
        )
        .unwrap();

        let report = sync_history_between_accounts(
            &data_root,
            "source",
            "target",
            &dest.join("backup"),
            no_progress(),
        )
        .unwrap();
        assert_eq!(report.added_conversations, 1);
        let target_conv = data_root
            .join("target")
            .join("VSCode")
            .join("target")
            .join("history")
            .join("workspace")
            .join("conv1");
        assert!(target_conv.is_dir());
        assert!(target_conv.join("messages").join("0.json").is_file());
    }
}
