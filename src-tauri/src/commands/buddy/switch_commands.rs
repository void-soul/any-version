//! switch commands responsibility.
use super::*;

// ─── 本地导入（导入后经官方 API 补全资料/用量，并清理同 token 占位账号） ───

#[tauri::command]
pub async fn buddy_import_from_local(platform: String) -> Result<Option<BuddyAccount>, String> {
    let platform = platform_from_str(&platform)?;
    let local = match platform {
        BuddyPlatform::Workbuddy => workbuddy::import_payload_from_local(platform)?,
        BuddyPlatform::CodebuddyCn => codebuddy_cn::import_payload_from_local()?,
    };
    let Some(mut account) = local else {
        return Ok(None);
    };

    // 用官方 API 补全账号资料与用量
    match api::build_payload_from_token(platform, &account.access_token).await {
        Ok(enriched) => {
            if enriched.uid.is_some() {
                account.uid = enriched.uid;
            }
            if enriched.nickname.is_some() {
                account.nickname = enriched.nickname;
            }
            if enriched.refresh_token.is_some() {
                account.refresh_token = enriched.refresh_token;
            }
            if enriched.domain.is_some() {
                account.domain = enriched.domain;
            }
            if enriched.expires_at.is_some() {
                account.expires_at = enriched.expires_at;
            }
            if enriched.profile_raw.is_some() {
                account.profile_raw = enriched.profile_raw;
            }
            if enriched.quota_raw.is_some() {
                account.quota_raw = enriched.quota_raw;
            }
            if enriched.usage_raw.is_some() {
                account.usage_raw = enriched.usage_raw;
            }
            if enriched.dosage_notify_code.is_some() {
                account.dosage_notify_code = enriched.dosage_notify_code;
            }
            if enriched.dosage_notify_zh.is_some() {
                account.dosage_notify_zh = enriched.dosage_notify_zh;
            }
            if enriched.dosage_notify_en.is_some() {
                account.dosage_notify_en = enriched.dosage_notify_en;
            }
            if enriched.payment_type.is_some() {
                account.payment_type = enriched.payment_type;
            }
            if account.email.trim().is_empty() || account.email == "unknown" {
                account.email = enriched.email;
            }
        }
        Err(err) => {
            eprintln!(
                "[Buddy Import Local] 拉取账号资料失败，将保留本地导入结果: {}",
                err
            );
        }
    }

    let mut saved = store::upsert_account(platform, account)?;

    // 清理同 token 的历史占位账号（email unknown / 无 uid）
    for existing in store::list_accounts(platform) {
        if existing.id == saved.id {
            continue;
        }
        if existing.access_token != saved.access_token {
            continue;
        }
        let is_placeholder = existing.email.trim().eq_ignore_ascii_case("unknown")
            || existing.email.trim().is_empty()
            || existing
                .uid
                .as_deref()
                .map(|s| s.trim().is_empty())
                .unwrap_or(true);
        if is_placeholder {
            if let Err(err) = store::delete_account(platform, &existing.id) {
                eprintln!(
                    "[Buddy Import Local] 清理占位账号失败: id={}, error={}",
                    existing.id, err
                );
            }
        }
    }

    // 导入后刷新一次 token（失败则保留原账号）
    match api::refresh_payload_for_account(platform, &saved).await {
        Ok((refreshed, _quota_err)) => {
            saved = store::upsert_account(platform, refreshed)?;
        }
        Err(e) => {
            eprintln!("[Buddy Import Local] 登录后刷新失败，保留原账号信息: {}", e);
        }
    }
    Ok(Some(saved))
}

// ─── 当前账号 ───

#[tauri::command]
pub fn buddy_get_current_account_id(platform: String) -> Result<Option<String>, String> {
    let platform = platform_from_str(&platform)?;
    let accounts = store::list_accounts(platform);

    // 优先用持久化的当前账号映射（切换时写入，稳定可靠）
    if let Some(stored) = store::get_current_account_id(platform) {
        if accounts.iter().any(|a| a.id == stored) {
            return Ok(Some(stored));
        }
        // 持久化 id 已不存在（账号被删除）→ 清理
        let _ = store::set_current_account_id(platform, None);
    }

    // 回退：读取本机客户端当前登录账号
    let current = match platform {
        BuddyPlatform::Workbuddy => workbuddy::resolve_current_account_id(platform, &accounts),
        BuddyPlatform::CodebuddyCn => codebuddy_cn::resolve_current_account_id(&accounts),
    };
    Ok(current)
}

// ─── 切换账号 ───

#[tauri::command]
pub async fn buddy_switch_account(
    app: tauri::AppHandle,
    platform: String,
    account_id: String,
) -> Result<(String, Option<session_transfer::SessionTransferReport>), String> {
    let platform = platform_from_str(&platform)?;
    let account = store::load_account(platform, &account_id)
        .ok_or_else(|| format!("账号不存在: {}", account_id))?;

    // 切换时序：检查客户端已退出 → 合并来源会话 → 写入登录态 → 重新启动。
    // 各阶段通过 buddy-switch-progress 事件上报前端；合并统计随报告一并返回（前端渲染为文本）。
    tauri::async_runtime::spawn_blocking(move || {
        // 0) 占用切号互斥闸（连点两次 → 第二次立刻报错，不并行写登录态）
        let _guard = SwitchGuard::acquire(platform)?;
        // 1) **只检查**客户端是否已退出：约定由用户自己关闭，程序不代关（更不会强杀），
        //    进程占用 db/会话目录，强关可能丢未落盘的会话。仍在运行 → 中止并提示手动退出。
        emit_switch_progress(Some(&app), platform, &account_id, "checking", 0, None);
        client_process::ensure_not_running(platform)?;

        // 2) 切换之前合并来源账号本地会话（合并进度在 session_transfer 内部上报）
        let transfer_report = session_transfer::transfer_on_switch(platform, &account, Some(&app))?;
        if let Some(report) = &transfer_report {
            eprintln!(
                "[Buddy Switch] 会话合并报告: 新增会话={}, 替换会话={}, 重映射记录={}, 扫描工作区={}, 会话总数={}, 已同步={}, 跳过={}, 冲突={}, 部分失败={}, 失败={}, 全部无变化={}",
                report.added_conversations,
                report.replaced_conversations,
                report.updated_session_rows,
                report.scanned_workspaces,
                report.sync.total,
                report.sync.copied,
                report.sync.skipped,
                report.sync.conflict,
                report.sync.partial,
                report.sync.failed,
                report.sync.unchanged
            );
        }

        // 3) 写入目标账号登录态
        emit_switch_progress(Some(&app), platform, &account_id, "writing", 0, None);
        let message = match platform {
            BuddyPlatform::Workbuddy => {
                workbuddy::switch_account(platform, &account_id)?
            }
            BuddyPlatform::CodebuddyCn => codebuddy_cn::switch_account(&account_id)?,
        };
        // 3.5) **延时复查**：等客户端落定后再确认登录态确实是我们写的。
        //     客户端处在退出/启动窗口时会用缓存会话回写覆盖（真机 2026-10-17 踩过），
        //     那种情况必须报错而不是返回成功 —— 静默成功比失败更糟。
        // CN confirms while its committed snapshot is still available for guarded recovery.
        if let Some(problem) = (platform != BuddyPlatform::CodebuddyCn)
            .then(|| confirm_switch_landed(platform, &account.access_token)).flatten() {
            eprintln!("[Buddy Switch] 切号未生效：{problem}");
            return Err(problem);
        }

        // 复查通过才算切号成功，此时才持久化当前账号（复刻 provider_current_state）。
        // 先写的话，复查失败返回 Err 时「当前账号」已经是新账号了 ——
        // 刷新后面板会显示一个并未生效的当前账号。
        let _ = store::set_current_account_id(platform, Some(&account_id));

        // 4) 切换后**不自动启动客户端**（两个平台一致）。
        //    切换时客户端本来就必须是「已退出」状态（见上面 ensure_not_running），
        //    自动拉起会打断用户手头的事：他可能正准备自己开窗口、或者正在处理别的窗口。
        //    两个平台都需要手动启动，统一提示即可。
        // 认证域体检：官方迁过域（codebuddy.cn → workbuddy.cn），两个域的 token 不通用。
        // 不阻断切换（老域账号的签到/用量仍能跑），但必须说清楚，否则用户看到的是
        // "切成功了、客户端却提示登录失效"这种哑故障。
        let mut message = format!(
            "{}，请手动启动 {}",
            message,
            client_process::app_display_name(platform)
        );
        if let Some(warning) = api::auth_origin_warning(platform, &account.access_token) {
            eprintln!("[Buddy Switch] 认证域不匹配：{warning}");
            message = format!("{}\n注意：{}", message, warning);
        }
        emit_switch_progress(Some(&app), platform, &account_id, "done", 0, None);
        Ok((message, transfer_report))
    })
    .await
    .map_err(|e| format!("切换账号后台任务失败: {}", e))?
}
