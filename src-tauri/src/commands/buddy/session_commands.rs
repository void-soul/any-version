//! session commands responsibility.
use super::*;

// ─── 会话管理 ───

/// 删除会话（数据库记录 + 本地会话文件，含正文与辅助目录）。
///
/// 走 `spawn_blocking`：一次可能删几百 MB 正文，同步命令会把界面卡住。
/// 进入时占**切号闸**（与 `buddy_switch_account` 同一把）—— 会话同步 worker 与切号并行时，
/// 它可能已读到源行、在删除返回后又把会话写回来（WorkDaddy 1.2.64 的同类问题）。
#[tauri::command]
pub async fn buddy_delete_sessions(
    platform: String,
    conversation_ids: Vec<String>,
) -> Result<sessions::BuddySessionDeleteReport, String> {
    let platform_enum = platform_from_str(&platform)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = SwitchGuard::acquire(platform_enum)?;
        sessions::delete_sessions(&platform, &conversation_ids)
    })
    .await
    .map_err(|e| format!("删除会话后台任务失败: {}", e))?
}

#[tauri::command]
pub fn buddy_list_sessions(
    platform: String,
    keyword: Option<String>,
    status: Option<String>,
) -> Result<Vec<sessions::BuddySessionRecord>, String> {
    let filter = sessions::BuddySessionFilter { keyword, status };
    sessions::list_sessions(&platform, &filter)
}

/// 分叉会话：复制正文并用新 id 注册一条会话记录，原会话保持不动。
#[tauri::command]
pub fn buddy_fork_session(
    platform: String,
    conversation_id: String,
    title: Option<String>,
) -> Result<sessions::BuddySessionForkReport, String> {
    sessions::fork_session(&platform, &conversation_id, title)
}

/// 改会话的所属目录（**连正文一起搬**到目标目录对应的工作区）。
///
/// 只改索引里的 `cwd` 不改正文会让会话变成「内容缺失」，所以两处一起改，
/// 并在搬之前整批备份一次（备份目录在返回值的 `backup_dir`，要还原就整棵放回去）。
#[tauri::command]
pub fn buddy_move_sessions(
    platform: String,
    conversation_ids: Vec<String>,
    target_dir: String,
) -> Result<sessions::BuddySessionMoveReport, String> {
    // WorkBuddy 的会话正文不在 md5(cwd) 目录里（5.x 是共享库），搬法完全不同，先不支持
    if platform_from_str(&platform)? != models::BuddyPlatform::CodebuddyCn {
        return Err("目前只支持 CodeBuddy CN 会话改目录".to_string());
    }
    if conversation_ids.is_empty() {
        return Err("请先选中要改目录的会话".to_string());
    }
    sessions::move_sessions_to_dir(&conversation_ids, &target_dir)
}

/// 列出**当前登录账号**（作为合并目标）的待处理会话冲突。
///
/// 冲突文件按目标账号分文件落盘，来回切换会留下多份旧快照；只按当前账号
/// 过滤，避免「横幅 7 个、弹窗 13 条」的错位（多出的是过时方向的旧记录）。
/// 冲突目前只会产生于 CodeBuddy CN（WorkBuddy 的合并是共享库重映射，无基线差异）。
#[tauri::command]
pub fn buddy_list_session_conflicts(
    platform: String,
) -> Result<Vec<session_sync::PendingConflict>, String> {
    match platform_from_str(&platform)? {
        BuddyPlatform::CodebuddyCn => {
            Ok(session_transfer::codebuddy::list_pending_conflicts_for_current_account())
        }
        BuddyPlatform::Workbuddy => Ok(Vec::new()),
    }
}

/// 裁决一条会话冲突，返回当前账号剩余的待处理冲突。
///
/// `action`：`merge`（取较新）/ `overwrite`（用来源覆盖目标）/ `keep`（保留目标）/ `ignore`。
///
/// 走 `spawn_blocking`：裁决要**递归复制整个会话目录**（几十~几百 MB）并重写 index，
/// 同步命令会把这些全塞进主线程 —— 表现为点完按钮界面直接卡死、无任何反馈。
/// 切换账号（`buddy_switch_account`）早就是这么做的，这里补齐同一处理。
#[tauri::command]
pub async fn buddy_resolve_session_conflict(
    app: tauri::AppHandle,
    platform: String,
    conversation_id: String,
    action: String,
) -> Result<Vec<session_sync::PendingConflict>, String> {
    let platform = platform_from_str(&platform)?;
    let action = session_transfer::codebuddy::ConflictAction::parse(&action)?;
    match platform {
        BuddyPlatform::CodebuddyCn => {
            // 单条也走批量入口：批量版本一次加锁、共用备份目录与基线，
            // 而单条版本每次都会清空备份目录（串行调用会丢掉前面的回滚点）。
            let ids = vec![conversation_id];
            let total = ids.len();
            tauri::async_runtime::spawn_blocking(move || {
                let ctx = ConflictProgressCtx {
                    app: Some(&app),
                    platform,
                    total,
                };
                let result = session_transfer::codebuddy::resolve_conflicts_command(
                    &ids,
                    action,
                    Some(&ctx),
                );
                emit_conflict_progress(Some(&app), platform, "done", total, total, None, None);
                result
            })
            .await
            .map_err(|e| format!("冲突处理任务异常退出: {}", e))?
        }
        BuddyPlatform::Workbuddy => Err("WorkBuddy 的会话合并不产生冲突，无需处理".to_string()),
    }
}

/// **批量**裁决会话冲突：对选中的多条执行同一 action，返回当前账号剩余的待处理冲突。
///
/// 走批量命令而不是前端串行调单条：备份目录在每次 `prepare_backup_root` 时会被清空，
/// 串行调用只会留下最后一条的备份（前面的无法回滚）。
///
/// 同样是 `spawn_blocking` + 逐条进度事件，避免大批量处理时界面卡死。
#[tauri::command]
pub async fn buddy_resolve_session_conflicts(
    app: tauri::AppHandle,
    platform: String,
    conversation_ids: Vec<String>,
    action: String,
) -> Result<Vec<session_sync::PendingConflict>, String> {
    let platform = platform_from_str(&platform)?;
    let action = session_transfer::codebuddy::ConflictAction::parse(&action)?;
    match platform {
        BuddyPlatform::CodebuddyCn => {
            let total = conversation_ids.len();
            tauri::async_runtime::spawn_blocking(move || {
                let ctx = ConflictProgressCtx {
                    app: Some(&app),
                    platform,
                    total,
                };
                let result = session_transfer::codebuddy::resolve_conflicts_command(
                    &conversation_ids,
                    action,
                    Some(&ctx),
                );
                emit_conflict_progress(Some(&app), platform, "done", total, total, None, None);
                result
            })
            .await
            .map_err(|e| format!("冲突处理任务异常退出: {}", e))?
        }
        BuddyPlatform::Workbuddy => Err("WorkBuddy 的会话合并不产生冲突，无需处理".to_string()),
    }
}

/// 列出当前账号的**冲突处理结果**（会话明细据此显示「已合并 / 已覆盖 / 已保留」，
/// 而不是永远停在切换那一刻的初始状态）。
#[tauri::command]
pub fn buddy_list_conflict_resolutions(
    platform: String,
) -> Result<Vec<session_sync::ConflictResolution>, String> {
    match platform_from_str(&platform)? {
        BuddyPlatform::CodebuddyCn => {
            Ok(session_transfer::codebuddy::list_conflict_resolutions_for_current_account())
        }
        BuddyPlatform::Workbuddy => Ok(Vec::new()),
    }
}

/// 读取冲突会话某一侧（source / target）的对话内容预览。
#[tauri::command]
pub fn buddy_read_conflict_messages(
    platform: String,
    conversation_id: String,
    side: String,
) -> Result<Vec<session_transfer::codebuddy::ConflictMessage>, String> {
    match platform_from_str(&platform)? {
        BuddyPlatform::CodebuddyCn => {
            session_transfer::codebuddy::read_conflict_messages(&conversation_id, &side)
        }
        BuddyPlatform::Workbuddy => Err("WorkBuddy 的会话合并不产生冲突，无需处理".to_string()),
    }
}
