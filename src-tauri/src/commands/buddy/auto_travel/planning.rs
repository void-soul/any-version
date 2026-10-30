//! planning responsibility.
use super::*;

/// 今天是否还有账号没有生成派出计划（跨天 / 首次运行 / 新增账号）。
///
/// 与自动签到同构：让「每天打开应用」就把今天的计划生成出来，
/// 不再等到 `startTime`，从根上消除前端"一直显示未安排"的现象。
pub fn needs_replan(config: &BuddyAutoTravelConfig, accounts: &[BuddyAccount]) -> bool {
    let today = super::super::auto_checkin::get_today_date_string();
    let Some(schedules) = config.account_schedules.as_ref() else {
        return !accounts.is_empty();
    };
    accounts.iter().any(|account| {
        schedules
            .get(&account.id)
            .map(|schedule| schedule.scheduled_date != today)
            .unwrap_or(true)
    })
}

/// 把一条派旅行结果写进每日归档（失败只打日志，不影响状态机）。
///
/// `date` 传「这笔旅行归属的日期」：派出/归来/领取都归到出发那天
/// （跨零点的情况，比如 23:50 出发、次日 01:00 归来，仍然记在出发日）。
pub(super) fn archive_travel(account: &BuddyAccount, date: &str, patch: TravelPatch) {
    let email = if account.email.trim().is_empty() {
        account.id.clone()
    } else {
        account.email.clone()
    };
    if let Err(err) = daily_history::upsert_travel(date, &account.id, &email, patch) {
        eprintln!(
            "[BuddyAutoTravel] 写入派旅行归档失败({}): {}",
            account.id, err
        );
    }
}

/// 当前本地时间 `HH:MM:SS`。
pub(super) fn now_time_string() -> String {
    daily_history::now_time_string()
}

/// 计划发生变化（跨天重规划 / 首次生成）时同步归档（与签到侧同构）。
pub(super) fn archive_schedule_changes(
    before: &HashMap<String, BuddyAccountTravelState>,
    after: &BuddyAutoTravelConfig,
    accounts: &[BuddyAccount],
) {
    let today = super::super::auto_checkin::get_today_date_string();
    let schedules = after.account_schedules.clone().unwrap_or_default();
    for account in accounts {
        let old = before.get(&account.id);
        let new = schedules.get(&account.id);
        let plan_changed = match (old, new) {
            (Some(o), Some(n)) => {
                o.scheduled_date != n.scheduled_date || o.scheduled_minute != n.scheduled_minute
            }
            (None, Some(_)) => true,
            _ => false,
        };
        if !plan_changed {
            continue;
        }
        if let Some(o) = old {
            if o.scheduled_date != today {
                if let Err(err) = daily_history::finalize_stale_day(&o.scheduled_date) {
                    eprintln!(
                        "[BuddyAutoTravel] 收尾旧日期归档失败({}): {}",
                        o.scheduled_date, err
                    );
                }
            }
        }
        if let Some(n) = new {
            if n.scheduled_date == today {
                let plan_time = super::super::auto_checkin::format_minutes(n.scheduled_minute);
                if let Err(err) = daily_history::upsert_travel_plan(
                    &today,
                    &account.id,
                    &account.email,
                    &plan_time,
                ) {
                    eprintln!(
                        "[BuddyAutoTravel] 写入计划归档失败({}): {}",
                        account.id, err
                    );
                }
            }
        }
    }
}

/// 为每个账号生成「当天」的派出计划（随机落在 [startTime, endTime] 内）。
///
/// `allow_generate`：仅当到达当天开始时间（或用户强制立即执行）才生成，
/// 与自动签到的语义一致。同一天因修改时间窗重新生成时，保留当日已发生的
/// 派出/完成状态，避免把进度抹掉。
pub fn ensure_travel_schedules(
    config: &mut BuddyAutoTravelConfig,
    accounts: &[BuddyAccount],
    allow_generate: bool,
) -> bool {
    let today_str = get_today_date_string();
    let (start_min, end_min) =
        super::super::auto_checkin::plan_window(&config.start_time, &config.end_time);
    let current_minute = super::super::auto_checkin::current_minute_of_day();

    let mut schedules = config.account_schedules.clone().unwrap_or_default();
    let mut changed = false;

    for account in accounts {
        let existing = schedules.get(&account.id);
        if let Some(sch) = existing {
            if sch.scheduled_date == today_str
                && sch.scheduled_minute >= start_min
                && sch.scheduled_minute <= end_min
            {
                continue;
            }
        }
        if !allow_generate {
            continue;
        }

        let (depart, depart_time, done, reward) = existing
            .filter(|e| e.scheduled_date == today_str)
            .map(|e| {
                (
                    e.last_depart_date.clone(),
                    e.last_depart_time.clone(),
                    e.last_done_date.clone(),
                    e.last_reward_credit,
                )
            })
            .unwrap_or((None, None, None, None));

        // 抽「窗口内还没过去」的点（与签到同源），窗口已全过去则退回整窗随机
        // ——那天不会再派出，前端显示「未完成」。
        let scheduled_minute =
            super::super::auto_checkin::draw_plan_minute(start_min, end_min, current_minute);

        schedules.insert(
            account.id.clone(),
            BuddyAccountTravelState {
                scheduled_date: today_str.clone(),
                scheduled_minute,
                last_depart_date: depart,
                last_depart_time: depart_time,
                last_done_date: done,
                last_reward_credit: reward,
            },
        );
        changed = true;
    }

    if changed {
        config.account_schedules = Some(schedules);
    }
    changed
}

/// 把「计划时间已过、今天却还没派出/完成」的账号重新安排到窗口内的新随机时间。
///
/// 规则与自动签到完全一致：
/// - 窗口内还有剩余时间 → 重新随机一个**未来**时间点，并记一条 `rescheduled` 日志；
/// - 窗口已全部过去 → 今天放弃（不改计划、不执行，等明天跨天重规划）；
/// - 已经派出过（旅行中/已归来待领取）或当日已完成的账号不参与重排；
/// - 派出失败（网络错误等）也重排，不再按固定轮询原地重试。
///
/// 返回 `(计划是否有改动, 新增的行为日志)`。
pub(super) fn reschedule_missed_plans(
    config: &mut BuddyAutoTravelConfig,
    accounts: &[BuddyAccount],
    current_minute: i32,
) -> (bool, Vec<action_log::BuddyActionLogEntry>) {
    let today = get_today_date_string();
    let (start_min, end_min) =
        super::super::auto_checkin::plan_window(&config.start_time, &config.end_time);
    let Some((from, to)) =
        super::super::auto_checkin::remaining_window(start_min, end_min, current_minute)
    else {
        return (false, Vec::new());
    };

    let mut schedules = config.account_schedules.clone().unwrap_or_default();
    let mut entries = Vec::new();

    for account in accounts {
        let Some(schedule) = schedules.get(&account.id) else {
            continue;
        };
        if schedule.scheduled_date != today
            || schedule.last_depart_date.as_deref() == Some(today.as_str())
            || schedule.last_done_date.as_deref() == Some(today.as_str())
            || !super::super::auto_checkin::is_plan_missed(
                schedule.scheduled_minute,
                current_minute,
            )
        {
            continue;
        }

        let old_minute = schedule.scheduled_minute;
        let new_minute = super::super::auto_checkin::draw_plan_minute(from, to, current_minute);
        if let Some(state) = schedules.get_mut(&account.id) {
            state.scheduled_minute = new_minute;
        }
        entries.push(action_log::make_entry(
            "travel",
            &account.id,
            &account_email(account),
            "rescheduled",
            Some(format!(
                "原计划 {} 已过且未完成，重新安排到 {}",
                super::super::auto_checkin::format_minutes(old_minute),
                super::super::auto_checkin::format_minutes(new_minute)
            )),
            None,
        ));
    }

    let changed = !entries.is_empty();
    if changed {
        config.account_schedules = Some(schedules);
    }
    (changed, entries)
}

/// 把本轮「尝试派出但没成功」的账号的计划挪到窗口内的新随机时间（失败的重试时机）。
///
/// 与签到侧同构：光靠 `reschedule_missed_plans` 要等「错过超过宽限」才动手，而计划在
/// 宽限内仍算「到点」，下一个轮询就会把同一个失败账号再打一遍。
///
/// 已派出过（旅行中 / 已归来待领取）与当日已完成的账号不挪：它们的重试走正常轮询。
/// 窗口已全部过去时不改（今天放弃）。返回是否有账号被挪动。
pub(super) fn reschedule_failed_plans(
    schedules: &mut HashMap<String, BuddyAccountTravelState>,
    attempted: &[String],
    today: &str,
    current_minute: i32,
    start_min: i32,
    end_min: i32,
) -> bool {
    let Some((from, to)) =
        super::super::auto_checkin::remaining_window(start_min, end_min, current_minute)
    else {
        return false;
    };
    let mut changed = false;
    for account_id in attempted {
        let Some(state) = schedules.get_mut(account_id) else {
            continue;
        };
        if state.scheduled_date != today
            || state.last_depart_date.as_deref() == Some(today)
            || state.last_done_date.as_deref() == Some(today)
        {
            continue;
        }
        state.scheduled_minute =
            super::super::auto_checkin::draw_plan_minute(from, to, current_minute);
        changed = true;
    }
    changed
}

/// 展示用邮箱（缺失时退回账号 id）。
pub(super) fn account_email(account: &BuddyAccount) -> String {
    if account.email.trim().is_empty() {
        account.id.clone()
    } else {
        account.email.clone()
    }
}
