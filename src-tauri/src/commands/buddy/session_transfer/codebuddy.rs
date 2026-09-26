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
use super::super::session_sync::{PendingConflict, SessionSyncStatus, SyncTracker};
use super::super::store;
use super::SessionTransferReport;
use super::TransferProgress;
use crate::commands::buddy::emit_switch_progress;

/// 进程级互斥：同一时刻只允许一次 CodeBuddy CN 会话合并
static TRANSFER_LOCK: std::sync::LazyLock<std::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(()));

/// 会话备份目录的分类名（`{data_dir}/buddy/session-backup/<本值>/<uid>`）
const BACKUP_PLATFORM_LABEL: &str = "codebuddy-cn";

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
        BuddyPlatform::Workbuddy => {
            super::super::workbuddy::resolve_current_account_id(BuddyPlatform::Workbuddy, &accounts)
        }
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
    let backup_root = super::prepare_backup_root(BACKUP_PLATFORM_LABEL, target_uid)?;

    let mut report = SessionTransferReport::default();
    // 上次同步基线（目标账号维度）：用于"只处理有变化的会话"与"两侧都改过即冲突"的判定。
    let baseline = super::super::session_sync::load_baseline(BACKUP_PLATFORM_LABEL, target_uid);
    let mut tracker = SyncTracker::new(baseline);
    let mut pending_conflicts: Vec<PendingConflict> = Vec::new();

    report.scanned_workspaces = sync_history_between_accounts(
        &extension_data_dir,
        source_uid,
        target_uid,
        &backup_root,
        progress,
        &mut tracker,
        &mut pending_conflicts,
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
    super::super::session_sync::save_baseline(BACKUP_PLATFORM_LABEL, target_uid, &next_baseline);
    // 冲突落盘（空列表 = 清掉旧文件）：切换结束后用户仍可逐条裁决
    super::super::session_sync::save_pending_conflicts(
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
}

impl ConflictAction {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "merge" => Ok(Self::Merge),
            "overwrite" => Ok(Self::Overwrite),
            "keep" => Ok(Self::Keep),
            other => Err(format!("未知的冲突处理方式: {}", other)),
        }
    }

    /// 落盘与前端展示用的稳定名字（与 parse 成反函数）。
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Overwrite => "overwrite",
            Self::Keep => "keep",
        }
    }
}

/// 待处理冲突文件所在目录（`{data_dir}/buddy/session-sync/codebuddy-cn/`）。
fn pending_conflicts_dir() -> Result<std::path::PathBuf, String> {
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
fn conflicts_stem(file_name: &str) -> Option<&str> {
    file_name.strip_suffix(".conflicts.json")
}

/// 当前登录账号（CodeBuddy CN）的 uid；未登录/无 uid/uid 非法时返回 None。
fn current_account_uid() -> Option<String> {
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
        super::super::session_sync::load_pending_conflicts(BACKUP_PLATFORM_LABEL, &uid);
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
fn find_pending_conflict(conversation_id: &str) -> Result<Option<(PendingConflict, String)>, String> {
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
        let conflicts =
            super::super::session_sync::load_pending_conflicts(BACKUP_PLATFORM_LABEL, stem);
        if let Some(conflict) = conflicts.iter().find(|c| c.id == conversation_id) {
            return Ok(Some((conflict.clone(), stem.to_string())));
        }
    }
    Ok(None)
}

/// 按会话 id 执行用户裁决，返回**当前账号**剩余的待处理冲突（前端直接整体替换）。
pub(crate) fn resolve_conflict_command(
    conversation_id: &str,
    action: ConflictAction,
) -> Result<Vec<PendingConflict>, String> {
    resolve_conflicts_command(&[conversation_id.to_string()], action)
}

/// **批量**裁决：一次对多条执行同一 action，返回当前账号剩余的待处理冲突。
///
/// 为什么必须走批量命令而不是前端串行调单条：`prepare_backup_root` 每次都会
/// `remove_dir_all` 清掉该 uid 的备份目录（见 `session_transfer.rs`），串行 N 次
/// 的结果就是**只剩最后一条的备份**，前面几条想回滚已经没东西可回。批量在这里
/// 只加一次锁、每个目标 uid 只准备一次备份目录、共用一份基线，最后统一落盘。
pub(crate) fn resolve_conflicts_command(
    conversation_ids: &[String],
    action: ConflictAction,
) -> Result<Vec<PendingConflict>, String> {
    if conversation_ids.is_empty() {
        return Err("未选择要处理的冲突".to_string());
    }

    // 1) 定位：逐条找记录，连同它所属的冲突文件 stem（一个 uid 一个文件）
    let mut found: Vec<(PendingConflict, String)> = Vec::new();
    for id in conversation_ids {
        validate_conversation_id(id)?;
        let Some(item) = find_pending_conflict(id)? else {
            return Err(format!(
                "找不到会话 {} 的待处理冲突（可能已经处理过）",
                id
            ));
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
            match grouped.iter_mut().find(|(uid, _)| uid == &conflict.target_uid) {
                Some(entry) => entry.1.push(conflict.clone()),
                None => grouped.push((conflict.target_uid.clone(), vec![conflict.clone()])),
            }
        }

        let mut resolved: Vec<(String, PendingConflict, String)> = Vec::new();
        for (uid, conflicts) in &grouped {
            let backup_root = super::prepare_backup_root(BACKUP_PLATFORM_LABEL, uid)?;
            let mut baseline =
                super::super::session_sync::load_baseline(BACKUP_PLATFORM_LABEL, uid);
            for conflict in conflicts {
                let message = resolve_pending_conflict_at(
                    &extension_data_dir,
                    conflict,
                    action,
                    &backup_root,
                    &mut baseline,
                )?;
                resolved.push((uid.clone(), conflict.clone(), message));
            }
            super::super::session_sync::save_baseline(BACKUP_PLATFORM_LABEL, uid, &baseline);
        }
        resolved
    };

    // 3) 记录处理结果（会话明细要显示「处理过了、怎么处理的」）
    let now = super::super::session_sync::now_millis();
    for (uid, conflict, message) in &resolved {
        super::super::session_sync::append_conflict_resolutions(
            BACKUP_PLATFORM_LABEL,
            uid,
            &[super::super::session_sync::ConflictResolution {
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
        let conflicts =
            super::super::session_sync::load_pending_conflicts(BACKUP_PLATFORM_LABEL, &stem);
        let remaining: Vec<PendingConflict> = conflicts
            .into_iter()
            .filter(|c| !done_ids.contains(&c.id))
            .collect();
        super::super::session_sync::save_pending_conflicts(BACKUP_PLATFORM_LABEL, &stem, &remaining);
    }

    Ok(list_pending_conflicts_for_current_account())
}

/// 列出**当前登录账号**的冲突处理结果（会话明细展示用）。
pub(crate) fn list_conflict_resolutions_for_current_account(
) -> Vec<super::super::session_sync::ConflictResolution> {
    let Some(uid) = current_account_uid() else {
        return Vec::new();
    };
    super::super::session_sync::load_conflict_resolutions(BACKUP_PLATFORM_LABEL, &uid)
}

/// 执行单条冲突裁决（命令入口）。
///
/// - `Overwrite` / `Merge`（来源较新）：备份目标 → 整目录替换 → 同步辅助目录 →
///   更新目标 index.json 条目 → 基线写来源时间戳；
/// - `Keep` / `Merge`（目标不旧）：什么都不动，基线写目标时间戳（冲突就此了结）。
fn resolve_pending_conflict(
    extension_data_dir: &Path,
    conflict: &PendingConflict,
    action: ConflictAction,
) -> Result<String, String> {
    // 与切换时的合并互斥：同一时刻只允许一边在动会话目录
    let _guard = TRANSFER_LOCK
        .lock()
        .map_err(|_| "CodeBuddy CN 会话合并正在进行，请稍后重试".to_string())?;

    // 备份目录沿用切换时的约定（每次裁决清掉旧的，只留最近一次）
    let backup_root = super::prepare_backup_root(BACKUP_PLATFORM_LABEL, &conflict.target_uid)?;
    let mut baseline =
        super::super::session_sync::load_baseline(BACKUP_PLATFORM_LABEL, &conflict.target_uid);
    let message = resolve_pending_conflict_at(
        extension_data_dir,
        conflict,
        action,
        &backup_root,
        &mut baseline,
    )?;
    super::super::session_sync::save_baseline(BACKUP_PLATFORM_LABEL, &conflict.target_uid, &baseline);
    Ok(message)
}

/// 冲突裁决的文件系统核心（基线以参数传入，便于测试注入临时目录）。
fn resolve_pending_conflict_at(
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
    let source_workspace = source_account_root.join("history").join(&conflict.workspace);
    let target_account_root = extension_data_dir
        .join(&conflict.target_uid)
        .join(&conflict.ide)
        .join(&conflict.target_uid);
    let target_workspace = target_account_root.join("history").join(&conflict.workspace);
    if !source_workspace.is_dir() {
        return Err(format!(
            "来源会话目录不存在（可能账号数据已被清理）: {}",
            source_workspace.display()
        ));
    }
    if !target_workspace.is_dir() {
        return Err(format!(
            "目标会话目录不存在: {}",
            target_workspace.display()
        ));
    }
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
        ConflictAction::Merge => {
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
        // 基线推进到来源时间戳：这轮来源改动已确认落地
        baseline.insert(conflict.id.clone(), conflict.source_stamp.to_string());
        Ok(format!(
            "已用来源会话（{}）覆盖目标（{}）",
            fmt_stamp(conflict.source_stamp),
            fmt_stamp(conflict.target_stamp)
        ))
    } else {
        // 保留目标：基线推进到目标时间戳，冲突就此了结（来源侧改动视为放弃）
        baseline.insert(conflict.id.clone(), conflict.target_stamp.to_string());
        Ok(format!(
            "已保留目标会话（{}），来源侧改动（{}）已放弃",
            fmt_stamp(conflict.target_stamp),
            fmt_stamp(conflict.source_stamp)
        ))
    }
}

/// epoch 毫秒 → 可读时间（报错信息与返回文案用）。
fn fmt_stamp(stamp_ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(stamp_ms)
        .map(|dt| dt.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| stamp_ms.to_string())
}

// ─── 冲突明细：对话内容预览 ───

/// 对话预览里的一条消息（已从双层 JSON 里抽出正文）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictMessage {
    /// user / assistant / tool
    pub role: String,
    pub time: String,
    pub text: String,
}

/// 预览取**最后**多少条（裁决关心的是两边最近聊到哪了）
const CONVERSATION_PREVIEW_MAX_MESSAGES: usize = 40;
/// 单条正文截断长度（tool 输出可能巨大）
const CONVERSATION_PREVIEW_MAX_CHARS: usize = 1200;

/// 读取冲突会话某一侧（source / target）的对话内容。
///
/// 会话正文布局：`<uid>/<IDE>/<uid>/history/<workspace>/<convId>/index.json`
/// 的 `messages` 数组按序存消息元数据，每条正文在 `messages/<messageId>.json`。
pub(crate) fn read_conflict_messages(
    conversation_id: &str,
    side: &str,
) -> Result<Vec<ConflictMessage>, String> {
    let Some((conflict, _)) = find_pending_conflict(conversation_id)? else {
        return Err(format!("找不到会话 {} 的待处理冲突", conversation_id));
    };
    let uid = match side {
        "source" => &conflict.source_uid,
        "target" => &conflict.target_uid,
        other => return Err(format!("未知的会话侧: {}", other)),
    };
    validate_uid(uid)?;
    let extension_data_dir = codebuddy_extension_data_dir()?;
    let conv_dir = extension_data_dir
        .join(uid)
        .join(&conflict.ide)
        .join(uid)
        .join("history")
        .join(&conflict.workspace)
        .join(&conflict.id);
    if !conv_dir.is_dir() {
        return Err(format!("会话目录不存在: {}", conv_dir.display()));
    }
    reject_symlink_if_exists(&conv_dir)?;
    let index = read_workspace_index(&conv_dir.join("index.json"))?;
    let metas = index
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let start = metas.len().saturating_sub(CONVERSATION_PREVIEW_MAX_MESSAGES);
    let mut out = Vec::new();
    for meta in &metas[start..] {
        let Some(id) = meta.get("id").and_then(Value::as_str) else {
            continue;
        };
        if id.contains('/') || id.contains('\\') || id.contains("..") {
            continue;
        }
        let path = conv_dir.join("messages").join(format!("{}.json", id));
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        extract_message_text(&raw, &mut out);
    }
    Ok(out)
}

/// 从消息文件原文抽出 `{role, time, text}`。
///
/// `message` 字段是**内嵌 JSON 字符串**，其 `content` 数组里 `type=text` 的才是
/// 用户可读正文；tool-call 折叠成一行标记，避免工具输出把预览撑爆。
fn extract_message_text(raw: &str, out: &mut Vec<ConflictMessage>) {
    let Ok(value) = serde_json::from_str::<Value>(raw) else {
        return;
    };
    let role = value
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("?")
        .to_string();
    let time = value
        .get("createdAt")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let message = match value.get("message") {
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    };
    let mut text = String::new();
    if let Ok(inner) = serde_json::from_str::<Value>(&message) {
        if let Some(content) = inner.get("content").and_then(Value::as_array) {
            for part in content {
                match part.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(t) = part.get("text").and_then(Value::as_str) {
                            if !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str(t);
                        }
                    }
                    Some("tool-call") => {
                        let name = part
                            .get("toolName")
                            .and_then(Value::as_str)
                            .unwrap_or("tool");
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(&format!("[调用工具: {}]", name));
                    }
                    _ => {}
                }
            }
        }
    }
    // 双层解析失败就用原文兜底，别让一条消息悄悄消失
    if text.is_empty() {
        text = message;
    }
    // tool 输出（role=tool 的原始 JSON）对裁决没帮助，更紧地截断
    let max_chars = if role == "tool" { 300 } else { CONVERSATION_PREVIEW_MAX_CHARS };
    if text.chars().count() > max_chars {
        text = text.chars().take(max_chars).collect::<String>() + "…";
    }
    out.push(ConflictMessage { role, time, text });
}

pub(crate) fn validate_uid(uid: &str) -> Result<(), String> {
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
/// 返回扫描到的工作区数；逐会话结局写入 `tracker`，冲突详情追加进 `pending_conflicts`
/// （WorkBuddy 也复用本函数，但它不会产生冲突分支）。
pub(super) fn sync_history_between_accounts(
    extension_data_dir: &Path,
    source_uid: &str,
    target_uid: &str,
    backup_root: &Path,
    progress: TransferProgress,
    tracker: &mut SyncTracker,
    pending_conflicts: &mut Vec<PendingConflict>,
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
            let scan = ConflictScanContext {
                ide: ide_name.to_string_lossy().to_string(),
                source_uid: source_uid.to_string(),
                target_uid: target_uid.to_string(),
            };
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
    let source_changed = source_stamp != baseline;
    let target_changed = target_stamp != baseline;
    match (source_changed, target_changed) {
        (false, false) => SyncDecision::Unchanged,
        (true, true) => SyncDecision::Conflict,
        _ => SyncDecision::Apply,
    }
}

/// 会话展示名：标题优先，其次**目录名**，最后才回落到 id。
///
/// 此前无标题的会话直接显示裸 id（一串哈希），用户根本认不出是哪个会话；
/// 所属工作区目录对应真实项目，是比 id 有意义得多的 fallback。
fn conversation_label(conversation: &Value, id: &str, workspace_fallback: &str) -> String {
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

// ─── 工作区哈希 → 真实项目路径 ───

/// 进程级缓存：工作区哈希 → 解析出的真实路径（None = 解析失败也缓存，避免反复开库）。
static WORKSPACE_PATH_CACHE: std::sync::LazyLock<
    std::sync::Mutex<HashMap<String, Option<String>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// 把 history 下的工作区哈希目录名还原成真实项目路径。
///
/// 哈希规则（实测验证）：**MD5(项目路径)**，其中 Windows 路径盘符为小写、
/// 分隔符为反斜杠（如 `md5("e:\pro\my\any-version") = 48399de9…`）。
/// 候选路径取 CodeBuddy CN `state.vscdb` 的最近打开列表
/// （`history.recentlyOpenedPathsList`：folderUri 与 workspace.configPath）。
///
/// 还原失败（最近列表被清理/从未在本机打开过）返回 None ——
/// 显示层回退到原始哈希，不影响按哈希定位文件。
pub(crate) fn resolve_workspace_display(workspace_hash: &str) -> Option<String> {
    if let Some(cached) = WORKSPACE_PATH_CACHE
        .lock()
        .ok()
        .and_then(|cache| cache.get(workspace_hash).cloned())
    {
        return cached;
    }
    let resolved = resolve_workspace_display_uncached(workspace_hash);
    if let Ok(mut cache) = WORKSPACE_PATH_CACHE.lock() {
        cache.insert(workspace_hash.to_string(), resolved.clone());
    }
    resolved
}

fn resolve_workspace_display_uncached(workspace_hash: &str) -> Option<String> {
    let db = super::super::codebuddy_cn::default_state_db_path()?;
    let conn = rusqlite::Connection::open(&db).ok()?;
    let raw: String = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = 'history.recentlyOpenedPathsList'",
            [],
            |row| row.get(0),
        )
        .ok()?;
    let list: Value = serde_json::from_str(&raw).ok()?;
    for entry in list.get("entries")?.as_array()? {
        // 两种形态：folderUri（文件夹）与 workspace.configPath（.code-workspace 文件）
        for key in ["folderUri", "configPath"] {
            let Some(uri) = entry.get(key).and_then(Value::as_str) else {
                continue;
            };
            let Some(path) = uri_to_os_path(uri) else {
                continue;
            };
            let digest = format!("{:x}", md5::compute(path.as_bytes()));
            if digest == workspace_hash {
                return Some(path);
            }
        }
    }
    None
}

/// `file:///e%3A/pro/my/any-version` → `e:\pro\my\any-version`（盘符小写，与哈希口径一致）。
/// 非 file 协议（remote 等）返回 None。
fn uri_to_os_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let decoded = percent_decode(rest);
    let path = decoded.trim_start_matches('/');
    if path.is_empty() {
        return None;
    }
    if cfg!(target_os = "windows") {
        // `e:/pro/x` → `e:\pro\x`，盘符转小写（实测哈希用小写盘符）
        let mut chars = path.chars();
        let drive = chars.next()?;
        let tail: String = chars.collect();
        let tail = tail.replace('/', "\\");
        if !tail.starts_with(':') {
            return None;
        }
        Some(format!("{}{}", drive.to_ascii_lowercase(), tail))
    } else {
        Some(format!("/{}", path))
    }
}

/// 手写 percent-decode（不引入额外依赖；最近打开列表只含 `%3A` 这类盘符转义）。
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok();
            if let Some(value) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(value);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// 待处理冲突文件的定位上下文：冲突落盘后，解析命令要靠这三个字段找回两侧会话。
#[derive(Debug, Clone)]
pub(crate) struct ConflictScanContext {
    pub ide: String,
    pub source_uid: String,
    pub target_uid: String,
}

/// 合并单个工作区：会话目录 + index.json（去重、取新、保留 current）+ 辅助目录。
fn merge_workspace_history(
    source_workspace: &Path,
    target_workspace: &Path,
    source_account_root: &Path,
    target_account_root: &Path,
    workspace_name: &std::ffi::OsStr,
    backup_root: &Path,
    tracker: &mut SyncTracker,
    scan: &ConflictScanContext,
    pending_conflicts: &mut Vec<PendingConflict>,
) -> Result<(), String> {
    let workspace_label = workspace_name.to_string_lossy().to_string();
    // 哈希目录名对用户毫无意义（一串 32 位 MD5），能还原就还原成真实项目路径；
    // 还原不了再用哈希。明细「目录」列、无标题会话的展示名都用它。
    let workspace_display = resolve_workspace_display(&workspace_label)
        .unwrap_or_else(|| workspace_label.clone());
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
            let id = conversation_id(&conversation).unwrap_or_default().to_string();
            tracker.record(
                &id,
                &conversation_label(&conversation, &id, &workspace_display),
                SessionSyncStatus::Copied,
                "firstSync",
                Some(conversation_timestamp(&conversation).to_string()),
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
        let source_stamp = conversation_timestamp(&source_conversation).to_string();
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
                let target_stamp = conversation_timestamp(&merged[target_index_pos]).to_string();
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
                            source_stamp: conversation_timestamp(&source_conversation),
                            target_stamp: conversation_timestamp(&merged[target_index_pos]),
                        });
                        tracker.record(id, &label, SessionSyncStatus::Conflict, "bothChanged", None);
                        continue;
                    }
                    SyncDecision::Apply => {}
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

    Ok(())
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

pub(crate) fn read_workspace_index(path: &Path) -> Result<Value, String> {
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

/// 原子写入工作区索引（冲突裁决更新目标 index.json 用，与读取同一格式）。
fn write_workspace_index(path: &Path, index: &Value) -> Result<(), String> {
    let serialized = serde_json::to_string_pretty(index)
        .map_err(|e| format!("序列化 CodeBuddy CN 工作区索引失败: {}", e))?;
    store::write_atomic(&path.to_path_buf(), &serialized)
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

pub(crate) fn validate_conversation_id(id: &str) -> Result<(), String> {
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

pub(crate) fn reject_symlink_if_exists(path: &Path) -> Result<(), String> {
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
    fn conflicts_stem_strips_full_double_extension() {
        // 回归：Path::file_stem 只剥最后一个扩展名，"x.conflicts.json" 的 stem 是
        // "x.conflicts" 而不是 "x" —— 按它拼回文件名永远读不到，列表恒为空
        // （Q-0202 返工时真实踩坑：后端落盘 6 条、前端列表 0 条）。
        assert_eq!(
            conflicts_stem("4dc9bdfa-7cb0-4961-9b2f-bfcbde7e78b2.conflicts.json"),
            Some("4dc9bdfa-7cb0-4961-9b2f-bfcbde7e78b2")
        );
        assert_eq!(conflicts_stem("whatever.json"), None);
        assert_eq!(conflicts_stem("conflicts.json"), None);
    }

    #[test]
    fn conversation_label_falls_back_to_workspace_not_id() {
        // 无标题的会话以前显示裸 id（一串哈希），用户认不出是哪个会话；
        // 现在回落到所属工作区目录，只有目录也没有时才用 id
        let untitled = serde_json::json!({"id": "abc123"});
        assert_eq!(conversation_label(&untitled, "abc123", "my-project"), "my-project");
        let titled = serde_json::json!({"id": "abc123", "title": "重构登录"});
        assert_eq!(conversation_label(&titled, "abc123", "my-project"), "重构登录");
        let blank_workspace = serde_json::json!({"id": "abc123"});
        assert_eq!(conversation_label(&blank_workspace, "abc123", "  "), "abc123");
    }

    /// 构造一个「同 id 会话在来源/目标两侧都有」的冲突现场。
    /// 返回 (扩展数据根目录, 冲突记录)。
    fn conflict_fixture(
        dest: &Path,
        source_ts: i64,
        target_ts: i64,
    ) -> (std::path::PathBuf, PendingConflict) {
        let data_root = dest.join("CodeBuddyExtension").join("Data");
        let source_history = data_root
            .join("src-uid").join("VSCode").join("src-uid").join("history").join("ws");
        let target_history = data_root
            .join("dst-uid").join("VSCode").join("dst-uid").join("history").join("ws");
        for (history, body, ts) in [
            (&source_history, "source-body", source_ts),
            (&target_history, "target-body", target_ts),
        ] {
            std::fs::create_dir_all(history.join("conv-1")).unwrap();
            std::fs::write(history.join("conv-1").join("messages.jsonl"), body).unwrap();
            std::fs::write(
                history.join("index.json"),
                serde_json::to_vec_pretty(&serde_json::json!({
                    "conversations": [{ "id": "conv-1", "lastMessageAt": ts }]
                }))
                .unwrap(),
            )
            .unwrap();
        }
        let conflict = PendingConflict {
            id: "conv-1".to_string(),
            label: "ws".to_string(),
            workspace: "ws".to_string(),
            workspace_path: None,
            ide: "VSCode".to_string(),
            source_uid: "src-uid".to_string(),
            target_uid: "dst-uid".to_string(),
            source_stamp: source_ts,
            target_stamp: target_ts,
        };
        (data_root, conflict)
    }

    #[test]
    fn uri_to_os_path_matches_workspace_hash_convention() {
        // 哈希口径（实测）：MD5(小写盘符 + 反斜杠路径)，如
        // md5("e:\pro\my\any-version") = 48399de9e5eb08e5fe07dfb180314c02
        let path = uri_to_os_path("file:///e%3A/pro/my/any-version").unwrap();
        assert_eq!(path, "e:\\pro\\my\\any-version");
        let digest = format!("{:x}", md5::compute(path.as_bytes()));
        assert_eq!(digest, "48399de9e5eb08e5fe07dfb180314c02");

        // workspace 配置文件（.code-workspace）路径同样参与匹配
        let ws = uri_to_os_path("file:///e%3A/pro/gld/gld-web/wechat.code-workspace").unwrap();
        assert_eq!(ws, "e:\\pro\\gld\\gld-web\\wechat.code-workspace");

        // 非 file 协议不认；路径为空不认
        assert!(uri_to_os_path("vscode-remote://file%2Be%3A/pro/x").is_none());
        assert!(uri_to_os_path("file:///").is_none());
    }

    #[test]
    fn resolve_conflict_overwrite_replaces_target_and_advances_baseline() {
        let dest = make_temp();
        let (data_root, conflict) = conflict_fixture(&dest, 1_700_000_000_000, 1_699_000_000_000);
        let mut baseline = std::collections::BTreeMap::new();
        let message = resolve_pending_conflict_at(
            &data_root,
            &conflict,
            ConflictAction::Overwrite,
            &dest.join("backup"),
            &mut baseline,
        )
        .unwrap();
        assert!(message.contains("覆盖"), "返回文案应说明做了覆盖: {message}");

        // 目标会话正文被来源替换
        let target_conv = data_root
            .join("dst-uid").join("VSCode").join("dst-uid").join("history").join("ws").join("conv-1");
        let body = std::fs::read_to_string(target_conv.join("messages.jsonl")).unwrap();
        assert_eq!(body, "source-body");
        // 目标 index.json 条目同步为来源的（时间戳变成来源侧）
        let index: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(target_conv.parent().unwrap().join("index.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(index["conversations"][0]["lastMessageAt"], 1_700_000_000_000i64);
        // 原目标正文有备份可回滚
        let backup_body = std::fs::read_to_string(
            dest.join("backup").join("conversations").join("conv-1").join("messages.jsonl"),
        )
        .unwrap();
        assert_eq!(backup_body, "target-body");
        // 基线推进到来源时间戳：这轮来源改动视为已落地
        assert_eq!(baseline.get("conv-1").map(String::as_str), Some("1700000000000"));
    }

    #[test]
    fn resolve_conflict_keep_discards_source_and_resolves() {
        let dest = make_temp();
        let (data_root, conflict) = conflict_fixture(&dest, 1_700_000_000_000, 1_699_000_000_000);
        let mut baseline = std::collections::BTreeMap::new();
        let message = resolve_pending_conflict_at(
            &data_root,
            &conflict,
            ConflictAction::Keep,
            &dest.join("backup"),
            &mut baseline,
        )
        .unwrap();
        assert!(message.contains("保留"), "返回文案应说明保留了目标: {message}");

        // 目标一字未动
        let body = std::fs::read_to_string(
            data_root.join("dst-uid").join("VSCode").join("dst-uid")
                .join("history").join("ws").join("conv-1").join("messages.jsonl"),
        )
        .unwrap();
        assert_eq!(body, "target-body");
        // 基线推进到目标时间戳：冲突了结，下次切换不再提醒
        assert_eq!(baseline.get("conv-1").map(String::as_str), Some("1699000000000"));
    }

    #[test]
    fn resolve_conflict_merge_picks_the_newer_side() {
        // 来源较新 → 合并等价于覆盖
        let dest = make_temp();
        let (data_root, conflict) = conflict_fixture(&dest, 1_700_000_000_000, 1_699_000_000_000);
        let mut baseline = std::collections::BTreeMap::new();
        resolve_pending_conflict_at(
            &data_root, &conflict, ConflictAction::Merge, &dest.join("backup"), &mut baseline,
        )
        .unwrap();
        let body = std::fs::read_to_string(
            data_root.join("dst-uid").join("VSCode").join("dst-uid")
                .join("history").join("ws").join("conv-1").join("messages.jsonl"),
        )
        .unwrap();
        assert_eq!(body, "source-body");

        // 目标较新 → 合并等价于保留
        let dest2 = make_temp();
        let (data_root2, conflict2) = conflict_fixture(&dest2, 1_690_000_000_000, 1_699_000_000_000);
        let mut baseline2 = std::collections::BTreeMap::new();
        resolve_pending_conflict_at(
            &data_root2, &conflict2, ConflictAction::Merge, &dest2.join("backup"), &mut baseline2,
        )
        .unwrap();
        let body2 = std::fs::read_to_string(
            data_root2.join("dst-uid").join("VSCode").join("dst-uid")
                .join("history").join("ws").join("conv-1").join("messages.jsonl"),
        )
        .unwrap();
        assert_eq!(body2, "target-body");
        assert_eq!(baseline2.get("conv-1").map(String::as_str), Some("1699000000000"));
    }

    #[test]
    fn rejects_unsafe_uids() {
        for uid in ["", "../a", "a/b", "a\\b", "a..b", " a"] {
            assert!(validate_uid(uid).is_err(), "uid should be rejected: {uid}");
        }
        assert!(validate_uid("384c6dd0-c1bc-4ae2-a0d0-f70350c62f7b").is_ok());
    }

    /// 批量裁决落盘用的 action 名必须与 parse 成反函数（前端按它做 i18n）。
    #[test]
    fn conflict_action_roundtrips_through_as_str() {
        for action in [ConflictAction::Merge, ConflictAction::Overwrite, ConflictAction::Keep] {
            assert_eq!(ConflictAction::parse(action.as_str()).unwrap(), action);
        }
        assert!(ConflictAction::parse("nope").is_err());
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
        let mut tracker = SyncTracker::new(std::collections::BTreeMap::new());
        let scanned = sync_history_between_accounts(
            &data_root,
            "source",
            "target",
            &backup_root,
            no_progress(),
            &mut tracker,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(scanned, 1);
        let (summary, _) = tracker.finish();
        assert_eq!(summary.total, 1);
        assert_eq!(summary.copied, 1);
        assert_eq!(summary.details[0].status, SessionSyncStatus::Copied);
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

        let mut tracker = SyncTracker::new(std::collections::BTreeMap::new());
        let scanned = sync_history_between_accounts(
            &data_root,
            "source",
            "target",
            &dest.join("backup"),
            no_progress(),
            &mut tracker,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(scanned, 1);
        let (summary, _) = tracker.finish();
        assert_eq!(summary.copied, 1);
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

    /// 参考 WorkDaddy §17 的三分支判定：无变化跳过、两侧都改则冲突。
    #[test]
    fn decide_sync_skips_unchanged_and_flags_conflicts() {
        // 首次同步（无基线）→ 正常合并
        assert_eq!(decide_sync(None, "5", "3", true), SyncDecision::Apply);
        // 两侧都与基线一致 → 无变化
        assert_eq!(decide_sync(Some("3"), "3", "3", true), SyncDecision::Unchanged);
        // 只有一侧变化 → 正常合并
        assert_eq!(decide_sync(Some("3"), "5", "3", true), SyncDecision::Apply);
        assert_eq!(decide_sync(Some("3"), "3", "5", true), SyncDecision::Apply);
        // 两侧都比基线新 → 冲突，拒绝覆盖
        assert_eq!(decide_sync(Some("3"), "5", "6", true), SyncDecision::Conflict);
        // 目标会话文件缺失时必须补齐，不参与"无变化"判定
        assert_eq!(decide_sync(Some("3"), "3", "3", false), SyncDecision::Apply);
    }

    /// 第二次合并同一批会话：内容没变 → 全部跳过，不再重复复制。
    #[test]
    fn second_merge_skips_unchanged_conversations() {
        let dest = make_temp();
        let data_root = dest.join("CodeBuddyExtension").join("Data");
        let source_account = data_root.join("source").join("VSCode").join("source");
        let target_account = data_root.join("target").join("VSCode").join("target");
        let source_history = source_account.join("history").join("workspace");
        let target_history = target_account.join("history").join("workspace");
        std::fs::create_dir_all(source_history.join("conv1").join("messages")).unwrap();
        std::fs::write(source_history.join("conv1").join("messages").join("0.json"), "[]").unwrap();
        let index = serde_json::to_vec(&serde_json::json!({
            "conversations": [{"id": "conv1", "title": "会话一", "lastMessageAt": 123}],
            "current": "conv1"
        }))
        .unwrap();
        std::fs::write(source_history.join("index.json"), &index).unwrap();

        let backup_root = dest.join("backup");
        let mut first = SyncTracker::new(std::collections::BTreeMap::new());
        sync_history_between_accounts(
            &data_root, "source", "target", &backup_root, no_progress(), &mut first,
            &mut Vec::new(),
        )
        .unwrap();
        let (first_summary, baseline) = first.finish();
        assert_eq!(first_summary.copied, 1);

        // 目标工作区已经存在，来源内容未变：第二次应当什么都不做
        let mut second = SyncTracker::new(baseline);
        sync_history_between_accounts(
            &data_root, "source", "target", &backup_root, no_progress(), &mut second,
            &mut Vec::new(),
        )
        .unwrap();
        let (second_summary, _) = second.finish();
        assert_eq!(second_summary.total, 1);
        assert_eq!(second_summary.skipped, 1);
        assert_eq!(second_summary.copied, 0);
        assert!(second_summary.unchanged);
    }
}
