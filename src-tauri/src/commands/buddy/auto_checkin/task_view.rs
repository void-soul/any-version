//! task view responsibility.
use super::*;

/// 由「今日是否已签到」+「今日日志中最近一条结果」推导任务状态。
pub(super) fn derive_task_status(
    checked_today: bool,
    detail: Option<&action_log::BuddyActionLogEntry>,
) -> &'static str {
    if checked_today
        || detail
            .map(|d| d.status == "success" || d.status == "already_checked")
            .unwrap_or(false)
    {
        "success"
    } else if detail
        .map(|d| d.status == "failed" || d.status == "inactive")
        .unwrap_or(false)
    {
        "failed"
    } else {
        "pending"
    }
}

/// 组装「今日签到任务」列表（供前端展示）。
///
/// 状态推导（只关心今天）：
/// - `success`：今日计划已标记签到完成，或今日日志中该账号为成功/已签到
/// - `failed`：今日日志中该账号最近一次为失败（会按调度继续重试）
/// - `unfinished`：计划时间已错过、今天从未尝试过、且今天的窗口已全过去
///   （调度器已放弃今天，等明天跨天重规划）
/// - `pending`：其余情况（含「计划时间未到」与「今日计划尚未生成」）
pub fn build_tasks_view(platform: BuddyPlatform) -> Result<BuddyCheckinTasksView, String> {
    let config = get_config_checked()?;
    let accounts = store::list_accounts(platform);
    let today_str = get_today_date_string();

    // 今日行为日志里每个账号的最新签到结果（日志新的在前，首个即最新）
    let mut today_results: HashMap<String, action_log::BuddyActionLogEntry> = HashMap::new();
    for entry in action_log::get_action_logs()? {
        if entry.date != today_str || entry.kind != "checkin" {
            continue;
        }
        today_results
            .entry(entry.account_id.clone())
            .or_insert(entry);
    }

    let schedules = config.account_schedules.clone().unwrap_or_default();
    // 今日归档：实际签到时间与来源优先从这里取（跨天后仍可追溯）
    let archive = daily_history::get_day(&today_str).unwrap_or_default();
    let mut tasks = Vec::with_capacity(accounts.len());
    let mut generated = false;
    // 「今天已放弃」的判据（与调度器同一套，见 `is_plan_due`）：计划已错过、且今天
    // 的窗口已经全过去 → 显示「未完成」，而不是整天挂着「待签到」。
    let current_minute = current_minute_of_day();
    let (start_min, end_min) = plan_window(&config.start_time, &config.end_time);
    let window_closed = remaining_window(start_min, end_min, current_minute).is_none();

    for account in &accounts {
        let schedule = schedules.get(&account.id);
        let scheduled_today = schedule
            .map(|s| s.scheduled_date == today_str)
            .unwrap_or(false);
        if scheduled_today {
            generated = true;
        }
        let checked_today = schedule
            .and_then(|s| s.last_checked_date.as_deref())
            .map(|d| d == today_str)
            .unwrap_or(false);
        let detail = today_results.get(&account.id);
        // 「尝试过」= 有过真实调用结果的日志（重排日志不算），失败过的账号保持「失败」
        let attempted = detail
            .map(|d| {
                matches!(
                    d.status.as_str(),
                    "success" | "already_checked" | "failed" | "inactive"
                )
            })
            .unwrap_or(false);
        let give_up_today = !checked_today
            && !attempted
            && scheduled_today
            && window_closed
            && schedule
                .map(|s| is_plan_missed(s.scheduled_minute, current_minute))
                .unwrap_or(false);
        let status = if give_up_today {
            checkin_status::UNFINISHED
        } else {
            derive_task_status(checked_today, detail)
        };
        let archived_checkin = archive.get(&account.id).and_then(|r| r.checkin.as_ref());

        let email = if account.email.trim().is_empty() {
            account.id.clone()
        } else {
            account.email.clone()
        };
        tasks.push(BuddyCheckinTask {
            account_id: account.id.clone(),
            email,
            status: status.to_string(),
            scheduled_time: if scheduled_today {
                schedule.map(|s| format_minutes(s.scheduled_minute))
            } else {
                None
            },
            last_attempt_time: detail
                .map(|d| d.timestamp.get(11..).unwrap_or_default().to_string()),
            last_checkin_time: archived_checkin
                .and_then(|c| c.actual_time.clone())
                .or_else(|| {
                    schedule
                        .filter(|_| checked_today)
                        .and_then(|s| s.last_checked_time.clone())
                }),
            source: archived_checkin.and_then(|c| c.source.clone()),
            message: detail.and_then(|d| d.message.clone()),
        });
    }

    Ok(BuddyCheckinTasksView {
        enabled: config.enabled,
        start_time: config.start_time.clone(),
        end_time: config.end_time.clone(),
        generated,
        tasks,
    })
}
