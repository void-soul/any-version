//! checkin commands responsibility.
use super::*;

// ─── 签到 ───

#[tauri::command]
pub async fn buddy_checkin(
    app: tauri::AppHandle,
    platform: String,
    account_id: String,
) -> Result<api::CheckinResponse, String> {
    let platform = platform_from_str(&platform)?;
    let account = store::load_account(platform, &account_id)
        .ok_or_else(|| format!("账号不存在: {}", account_id))?;

    let mut response = api::perform_checkin(
        &account.access_token,
        account.uid.as_deref(),
        account.enterprise_id.as_deref(),
        account.domain.as_deref(),
    )
    .await
    .map_err(|err| {
        // AUTH_EXPIRED 前缀是给调度器用的内部约定，手动签到要展示给人看的文案
        match err.strip_prefix(api::AUTH_EXPIRED_PREFIX) {
            Some(text) => text.to_string(),
            None => err,
        }
    })?;

    // 失败但非「活动未开启」时回查一次状态：官方在部分通道会用业务码返回「今日已签到」，
    // 不回查就会把「已签到」显示成失败。自动签到早已这么做，手动路径此前漏了这一步。
    if !response.success && response.inactive != Some(true) {
        if let Ok(status) = api::get_checkin_status(
            &account.access_token,
            account.uid.as_deref(),
            account.enterprise_id.as_deref(),
            account.domain.as_deref(),
        )
        .await
        {
            if status.today_checked_in {
                response.success = true;
                response.already = Some(true);
                response.message = Some("今日已完成签到".to_string());
            }
        }
    }

    let already = response.already == Some(true);
    let inactive = response.inactive == Some(true);

    if response.success || inactive {
        let now = chrono::Utc::now().timestamp();
        let streak = response
            .streak_days
            .unwrap_or_else(|| account.checkin_streak.saturating_add(1));
        let reward = response.reward.clone().or_else(|| {
            response
                .credit
                .map(|credit| serde_json::json!({ "credit": credit }))
        });
        if response.success {
            api::update_checkin_info(platform, &account_id, Some(now), streak, reward)
                .map_err(|e| format!("签到成功但更新状态失败: {}", e))?;
        }

        // 手动签到同样要"落账"：写每日归档 + 行为日志。
        // 之前这里只更新了账号自身的 last_checkin_time，于是手动签到成功后
        // 账号状态卡与日历仍显示「待生成」，属于本模块的历史缺陷。
        let email = if account.email.trim().is_empty() {
            account.id.clone()
        } else {
            account.email.clone()
        };
        let today = daily_history::today_string();
        let credit = response.credit;
        // 三种「无需重试」的终态：已签到 / 活动未开启 / 本次成功
        let (archive_status, log_status, fallback_message) = if inactive {
            (
                daily_history::checkin_status::INACTIVE,
                "inactive",
                "签到活动未开启",
            )
        } else if already {
            (
                daily_history::checkin_status::ALREADY_CHECKED,
                "already_checked",
                "今日已完成签到",
            )
        } else {
            (
                daily_history::checkin_status::SUCCESS,
                "success",
                "签到成功（手动）",
            )
        };
        let message = response
            .message
            .clone()
            .filter(|text| !text.trim().is_empty())
            .unwrap_or_else(|| fallback_message.to_string());
        let patch = daily_history::CheckinPatch::new()
            .actual_time(daily_history::now_time_string())
            .status(archive_status)
            .source("kira")
            .credit(credit)
            .streak(Some(streak))
            .message(Some(message.clone()));
        if let Err(err) = daily_history::upsert_checkin(&today, &account_id, &email, patch) {
            eprintln!("[Buddy] 手动签到写入归档失败({}): {}", account_id, err);
        }
        if let Err(err) = action_log::append_action_logs(&[action_log::make_entry(
            "checkin",
            &account_id,
            &email,
            log_status,
            Some(message),
            credit,
        )]) {
            eprintln!("[Buddy] 手动签到写入行为日志失败({}): {}", account_id, err);
        }
        use tauri::Emitter;
        let _ = app.emit("buddy-action-logs-changed", ());
    }

    Ok(response)
}

#[tauri::command]
pub async fn buddy_checkin_status(
    platform: String,
    account_id: String,
) -> Result<api::CheckinStatusResponse, String> {
    let platform = platform_from_str(&platform)?;
    let account = store::load_account(platform, &account_id)
        .ok_or_else(|| format!("账号不存在: {}", account_id))?;
    api::get_checkin_status(
        &account.access_token,
        account.uid.as_deref(),
        account.enterprise_id.as_deref(),
        account.domain.as_deref(),
    )
    .await
}

// ─── 自动签到 ───

#[tauri::command]
pub fn buddy_auto_checkin_get_config() -> Result<auto_checkin::BuddyAutoCheckinConfig, String> {
    auto_checkin::get_config_checked()
}

#[tauri::command]
pub fn buddy_auto_checkin_save_config(
    config: auto_checkin::BuddyAutoCheckinConfig,
) -> Result<(), String> {
    auto_checkin::save_config(&config)
}

#[tauri::command]
pub fn buddy_get_action_logs() -> Result<Vec<action_log::BuddyActionLogEntry>, String> {
    action_log::get_action_logs()
}

#[tauri::command]
pub fn buddy_clear_action_logs() -> Result<(), String> {
    action_log::clear_action_logs()
}

/// 今日签到任务列表（账号 + 计划时间 + 状态）。签到仅限 WorkBuddy，固定按 WB 账号返回。
#[tauri::command]
pub fn buddy_auto_checkin_tasks() -> Result<auto_checkin::BuddyCheckinTasksView, String> {
    auto_checkin::build_tasks_view(BuddyPlatform::Workbuddy)
}

/// 手动立即执行一轮自动签到（仅 WorkBuddy）。
#[tauri::command]
pub async fn buddy_auto_checkin_run(
    app: tauri::AppHandle,
    force: Option<bool>,
) -> Result<String, String> {
    auto_checkin::run_auto_checkin_cycle_if_needed(
        BuddyPlatform::Workbuddy,
        &app,
        force.unwrap_or(false),
    )
    .await
}
