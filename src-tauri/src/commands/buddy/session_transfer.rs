//! Buddy 模块会话迁移：在切换（写入新登录态）之前，把前一账号的所有本地会话合并到目标账号。

pub(crate) mod workbuddy;
pub(crate) mod codebuddy;

use std::path::PathBuf;

use super::models::BuddyAccount;
use super::models::BuddyPlatform;
use super::session_sync::SessionSyncSummary;
use super::emit_switch_sync_progress;

/// 合并进度上报上下文（切换事件按目标账号标识，stage=merging）
#[derive(Clone, Copy)]
pub(crate) struct TransferProgress<'a> {
    pub app: Option<&'a tauri::AppHandle>,
    pub account_id: &'a str,
    pub platform: BuddyPlatform,
}

/// 切换账号之前，先合并来源账号的会话到目标账号。
/// 返回 `(switch_result, transfer_report)`。
pub fn transfer_on_switch(
    platform: BuddyPlatform,
    target: &BuddyAccount,
    app: Option<&tauri::AppHandle>,
) -> Result<Option<SessionTransferReport>, String> {
    let progress = TransferProgress {
        app,
        account_id: &target.id,
        platform,
    };
    let report = match platform {
        BuddyPlatform::Workbuddy => workbuddy::transfer_on_switch(target, progress)?,
        BuddyPlatform::CodebuddyCn => codebuddy::transfer_on_switch(target, progress)?,
    };
    // 明细在合并结束时一次性上报（逐会话明细最多 500 条，不适合跟每个工作区一起推）。
    if let Some(report) = &report {
        emit_switch_sync_progress(app, platform, &target.id, &report.sync);
    }
    Ok(report)
}

/// 会话合并备份根目录（`{data_dir}/buddy/session-backup/{platform}/{uid}`）。
///
/// 备份落在程序自己的数据目录下（可通过数据目录设置迁移到其他盘），
/// 而不是系统数据目录（Windows 的 `%APPDATA%`），避免在 C 盘堆积大文件。
///
/// 每次合并前先清空该账号的旧备份，只保留**最近一次**切换的备份：
/// 备份的用途是「本次合并可回滚」，按账号无限累积会让目录持续膨胀。
pub(crate) fn prepare_backup_root(
    platform_label: &str,
    target_uid: &str,
) -> Result<PathBuf, String> {
    let root = crate::commands::config::get_data_dir()
        .join("buddy")
        .join("session-backup")
        .join(platform_label)
        .join(target_uid);
    match std::fs::symlink_metadata(&root) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(format!(
                "拒绝清理符号链接形式的会话备份目录: {}",
                root.display()
            ));
        }
        Ok(metadata) if metadata.is_dir() => {
            std::fs::remove_dir_all(&root).map_err(|e| {
                format!(
                    "清理旧会话备份目录失败: path={}, error={}",
                    root.display(),
                    e
                )
            })?;
        }
        Ok(_) => {
            std::fs::remove_file(&root).map_err(|e| {
                format!(
                    "清理旧会话备份文件失败: path={}, error={}",
                    root.display(),
                    e
                )
            })?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(format!(
                "读取会话备份目录属性失败: path={}, error={}",
                root.display(),
                e
            ));
        }
    }
    std::fs::create_dir_all(&root).map_err(|e| {
        format!(
            "创建会话备份目录失败: path={}, error={}",
            root.display(),
            e
        )
    })?;
    Ok(root)
}

/// 总结一次会话迁移的结果。
///
/// `sync` 是「只处理有变化的会话」的台账：逐会话结局（复制/跳过/部分失败/冲突/失败）
/// 与计数。`added_*` / `replaced_*` / `updated_session_rows` 保留为兼容字段，
/// 与 `sync.copied` 口径不同（前者是文件/数据库行数，后者是会话数）。
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionTransferReport {
    pub added_conversations: usize,
    pub replaced_conversations: usize,
    pub updated_session_rows: usize,
    pub scanned_workspaces: usize,
    #[serde(default)]
    pub sync: SessionSyncSummary,
}
