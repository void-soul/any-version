//! task view responsibility.
use super::*;

/// 单个账号的「今日派旅行」视图（前端账号状态卡的一行）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddyTravelTask {
    pub account_id: String,
    pub email: String,
    /// pending | traveling | arrived | claimed | limit_reached | rejected | failed
    /// | unfinished | none（既无计划也无记录 → 前端显示「未安排」）
    pub status: String,
    /// 计划派出时间（HH:MM）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_time: Option<String>,
    /// 实际派出时间（HH:MM:SS）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depart_time: Option<String>,
    /// 归来时间（HH:MM:SS，轮询发现的时刻，误差 ≤ 轮询间隔）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub back_time: Option<String>,
    /// 领取奖励时间（HH:MM:SS）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim_time: Option<String>,
    /// 今日派出次数（每天从 0 开始）
    pub depart_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// 今日派旅行任务列表视图。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddyTravelTasksView {
    pub enabled: bool,
    pub start_time: String,
    pub end_time: String,
    pub location_id: i64,
    /// 今日计划是否已生成
    pub generated: bool,
    pub tasks: Vec<BuddyTravelTask>,
}

/// 组装「今日派旅行任务」列表（供前端账号状态卡与日历使用）。
///
/// 状态与时间优先取今日归档（跨天后仍可追溯、字段齐全），
/// 归档缺失时回退到调度器里的今日计划（`account_schedules`）。
///
/// 归档里还是 `pending` 但计划已错过、且今天的窗口已全过去时，对外显示 `unfinished`
/// （「今天已放弃」，等明天跨天重规划）——与签到侧同一套判据（见 `is_plan_due`）。
pub fn build_travel_tasks_view() -> Result<BuddyTravelTasksView, String> {
    let config = get_config_checked()?;
    // 旅行是 WorkBuddy 专属活动
    let accounts = store::list_accounts(BuddyPlatform::Workbuddy);
    let today_str = super::super::auto_checkin::get_today_date_string();
    let schedules = config.account_schedules.clone().unwrap_or_default();
    let archive = daily_history::get_day(&today_str).unwrap_or_default();

    let mut generated = false;
    let mut tasks = Vec::with_capacity(accounts.len());
    let current_minute = super::super::auto_checkin::current_minute_of_day();
    let (start_min, end_min) =
        super::super::auto_checkin::plan_window(&config.start_time, &config.end_time);
    let window_closed =
        super::super::auto_checkin::remaining_window(start_min, end_min, current_minute).is_none();

    for account in &accounts {
        let schedule = schedules.get(&account.id);
        let scheduled_today = schedule
            .map(|s| s.scheduled_date == today_str)
            .unwrap_or(false);
        if scheduled_today {
            generated = true;
        }
        let archived = archive
            .get(&account.id)
            .and_then(|record| record.travel.as_ref());
        let email = if account.email.trim().is_empty() {
            account.id.clone()
        } else {
            account.email.clone()
        };
        let plan_time = archived.and_then(|t| t.plan_time.clone()).or_else(|| {
            if scheduled_today {
                schedule.map(|s| super::super::auto_checkin::format_minutes(s.scheduled_minute))
            } else {
                None
            }
        });
        let status = archived
            .map(|t| t.status.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                if plan_time.is_some() {
                    travel_status::PENDING.to_string()
                } else {
                    travel_status::NONE.to_string()
                }
            });
        // 「今天已放弃」：计划错过 + 窗口已全过去 + 今天既没派出过也没结束
        let give_up_today = window_closed
            && scheduled_today
            && schedule
                .map(|s| {
                    super::super::auto_checkin::is_plan_missed(s.scheduled_minute, current_minute)
                        && s.last_depart_date.as_deref() != Some(today_str.as_str())
                        && s.last_done_date.as_deref() != Some(today_str.as_str())
                })
                .unwrap_or(false);
        let status = if give_up_today && status == travel_status::PENDING {
            travel_status::UNFINISHED.to_string()
        } else {
            status
        };

        tasks.push(BuddyTravelTask {
            account_id: account.id.clone(),
            email,
            status,
            plan_time,
            depart_time: archived.and_then(|t| t.depart_time.clone()),
            back_time: archived.and_then(|t| t.back_time.clone()),
            claim_time: archived.and_then(|t| t.claim_time.clone()),
            depart_count: archived.map(|t| t.depart_count).unwrap_or(0),
            credit: archived.and_then(|t| t.credit),
            message: archived.and_then(|t| t.message.clone()),
        });
    }

    Ok(BuddyTravelTasksView {
        enabled: config.enabled,
        start_time: config.start_time.clone(),
        end_time: config.end_time.clone(),
        location_id: config.location_id,
        generated,
        tasks,
    })
}
