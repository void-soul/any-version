//! Buddy 模块会话迁移：在切换（写入新登录态）之前，把前一账号的所有本地会话合并到目标账号。

mod workbuddy;
mod codebuddy;

use super::models::BuddyAccount;
use super::models::BuddyPlatform;

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
    match platform {
        BuddyPlatform::Workbuddy => workbuddy::transfer_on_switch(target, progress),
        BuddyPlatform::CodebuddyCn => codebuddy::transfer_on_switch(target, progress),
    }
}

/// 总结一次会话迁移的结果。
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionTransferReport {
    pub added_conversations: usize,
    pub replaced_conversations: usize,
    pub updated_session_rows: usize,
    pub scanned_workspaces: usize,
}
