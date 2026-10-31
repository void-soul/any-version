//! planning responsibility.
use super::*;

pub fn parse_time_to_minutes(time_str: &str) -> i32 {
    let parts: Vec<&str> = time_str.split(':').collect();
    let h = parts
        .first()
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or(0);
    let m = parts
        .get(1)
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or(0);
    h * 60 + m
}

pub fn get_today_date_string() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

pub fn random_u32_below(max: u32) -> u32 {
    if max == 0 {
        return 0;
    }
    let mut buf = [0u8; 4];
    let _ = getrandom::getrandom(&mut buf);
    u32::from_le_bytes(buf) % max
}

/// 当前本地时间的「当天分钟数」（0~1439）。
pub fn current_minute_of_day() -> i32 {
    let now = Local::now();
    (now.hour() * 60 + now.minute()) as i32
}

/// 计划窗口 `[startTime, endTime]`（`end < start` 时收口到 start，与生成逻辑一致）。
pub fn plan_window(start_time: &str, end_time: &str) -> (i32, i32) {
    let start = parse_time_to_minutes(start_time);
    let end = parse_time_to_minutes(end_time).max(start);
    (start, end)
}

/// 计划是否已经「错过」：超过 `MISSED_PLAN_GRACE_MIN` 仍未执行。
pub fn is_plan_missed(scheduled_minute: i32, current_minute: i32) -> bool {
    current_minute - scheduled_minute > MISSED_PLAN_GRACE_MIN
}

/// 今天窗口内还剩下的可用区间 `[max(start, now + 1), end]`；`None` = 窗口已全部过去。
///
/// 「打开应用时发现计划时间已过」的处理都以它为准：还有剩余窗口就重排到里面，
/// 全过去了就今天放弃（等明天跨天重规划）。
pub fn remaining_window(start_min: i32, end_min: i32, current_minute: i32) -> Option<(i32, i32)> {
    let from = start_min.max(current_minute + 1);
    if from > end_min {
        None
    } else {
        Some((from, end_min))
    }
}

/// 第 `attempt` 次失败（从 1 起）之后应等待的秒数。
pub fn checkin_retry_delay_secs(attempt: u32) -> u64 {
    let idx = (attempt.max(1) - 1) as usize;
    CHECKIN_RETRY_DELAYS_SECS[idx.min(CHECKIN_RETRY_DELAYS_SECS.len() - 1)]
}

/// Retry only a transient HTTP/transport error or the preserved 10001 refusal.
/// Authentication always wins; affirmative completion is handled by the decoder.
pub fn checkin_error_is_retryable(err: &str) -> bool {
    api::is_retryable_checkin_error(err)
}

/// 计划是否到了该执行的时候：到点、错过不超过宽限、且仍在今天的窗口内。
///
/// 三个条件缺一不可：
/// - 还没到点不动；
/// - 错过太久的计划交给重排，绝不「打开应用就立刻补执行」；
/// - 窗口已全部过去 → 今天放弃（用户确认的语义：不补执行，等明天）。
pub fn is_plan_due(scheduled_minute: i32, current_minute: i32, end_min: i32) -> bool {
    current_minute >= scheduled_minute
        && !is_plan_missed(scheduled_minute, current_minute)
        && current_minute <= end_min
}

/// 抽一个计划分钟：优先落在「窗口内还没过去的时间」上，窗口已全部过去时退回整窗随机。
///
/// 生成计划与重排计划共用：晚开应用（例如 09:00 打开、窗口 06:00~12:00）不应该
/// 一生成就抽到一个已经过去的点——那等于「立刻执行」。
pub fn draw_plan_minute(start_min: i32, end_min: i32, current_minute: i32) -> i32 {
    match remaining_window(start_min, end_min, current_minute) {
        Some((from, to)) => random_minute_in(from, to),
        None => {
            let span = (end_min - start_min).max(0) as u32;
            start_min + random_u32_below(span + 1) as i32
        }
    }
}

/// 在 `[from, to]` 内随机取一分钟（含端点）。
pub(super) fn random_minute_in(from: i32, to: i32) -> i32 {
    let span = (to - from).max(0) as u32;
    from + random_u32_below(span + 1) as i32
}

/// 展示用邮箱（缺失时退回账号 id）。
pub(super) fn account_email(account: &BuddyAccount) -> String {
    if account.email.trim().is_empty() {
        account.id.clone()
    } else {
        account.email.clone()
    }
}

/// 今天是否还有账号没有生成计划（跨天 / 首次运行 / 新增账号）。
///
/// 这是「每天打开时若发现过了一天就重新计划」的判定：只要有一个账号的计划
/// 不是今天的，就允许立即重规划（不必等 `startTime`），从根上消除
/// 前端"一直显示待生成计划"的现象。
pub fn needs_replan(config: &BuddyAutoCheckinConfig, accounts: &[BuddyAccount]) -> bool {
    let today = get_today_date_string();
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

/// 为每个账号生成「当天」的计划签到分钟（随机落在 [startTime, endTime] 内）。
///
/// `allow_generate`：是否允许生成**今天**的计划。调用方传入「已到达当天开始时间」
/// （或用户强制立即执行），以符合「每天在开始时间生成计划」的语义——未到开始时间
/// 只保留既有计划不动，前端据此显示「今日计划待生成」。
pub fn ensure_account_schedules(
    config: &mut BuddyAutoCheckinConfig,
    accounts: &[BuddyAccount],
    allow_generate: bool,
) -> bool {
    let today_str = get_today_date_string();
    let (start_min, end_min) = plan_window(&config.start_time, &config.end_time);
    let current_minute = current_minute_of_day();

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

        // 抽「窗口内还没过去」的点：晚开应用时若从整窗随机，容易抽到一个已经
        // 过去的时刻而被当成「立刻执行」。窗口已全过去则退回整窗随机（当天不再
        // 执行，前端显示「未完成」）。
        let scheduled_minute = draw_plan_minute(start_min, end_min, current_minute);

        let last_checked = existing.and_then(|e| {
            if e.last_checked_date.as_deref() == Some(&today_str) {
                Some(today_str.clone())
            } else {
                None
            }
        });

        schedules.insert(
            account.id.clone(),
            BuddyAccountScheduleState {
                scheduled_date: today_str.clone(),
                scheduled_minute,
                last_checked_date: last_checked,
                last_checked_time: None,
                // 跨天/首次生成 = 全新的一轮，失败退避状态一并清掉
                retry_count: 0,
                exhausted: false,
                retry_at_epoch: None,
            },
        );
        changed = true;
    }

    if changed {
        config.account_schedules = Some(schedules);
    }
    changed
}

/// 把「计划时间已过、今天却还没完成」的账号重新安排到窗口内的新随机时间。
///
/// 规则（与用户确认）：
/// - 窗口内还有剩余时间 → 重新随机一个**未来**时间点，并记一条 `rescheduled` 日志；
/// - 窗口已全部过去 → 今天放弃：计划保持原样、不执行，等明天的跨天重规划
///   （`is_plan_due` 的「仍在窗口内」条件保证它不会被立刻执行，前端显示「未完成」）；
/// - 已进入重试或当天终止的账号不参与随机重排，避免覆盖退避或耗尽状态。
///
/// 返回 `(计划是否有改动, 新增的行为日志)`。
pub(super) fn reschedule_missed_plans(
    config: &mut BuddyAutoCheckinConfig,
    accounts: &[BuddyAccount],
    current_minute: i32,
) -> (bool, Vec<action_log::BuddyActionLogEntry>) {
    let today = get_today_date_string();
    let (start_min, end_min) = plan_window(&config.start_time, &config.end_time);
    let Some((from, to)) = remaining_window(start_min, end_min, current_minute) else {
        return (false, Vec::new());
    };

    let mut schedules = config.account_schedules.clone().unwrap_or_default();
    let mut entries = Vec::new();

    for account in accounts {
        let Some(schedule) = schedules.get(&account.id) else {
            continue;
        };
        if schedule.scheduled_date != today
            || schedule.last_checked_date.as_deref() == Some(today.as_str())
            || !is_plan_missed(schedule.scheduled_minute, current_minute)
            // 失败退避中的账号由重试闸接管：这里再挪计划会把它推得更远
            || schedule.retry_at_epoch.is_some()
            || schedule.exhausted
        {
            continue;
        }

        let old_minute = schedule.scheduled_minute;
        let new_minute = draw_plan_minute(from, to, current_minute);
        if let Some(state) = schedules.get_mut(&account.id) {
            state.scheduled_minute = new_minute;
        }
        entries.push(action_log::make_entry(
            "checkin",
            &account.id,
            &account_email(account),
            "rescheduled",
            Some(format!(
                "原计划 {} 已过且未完成，重新安排到 {}",
                format_minutes(old_minute),
                format_minutes(new_minute)
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

/// 把本轮「尝试过但没成功」的账号标记成「过一会儿再试」。
///
/// **退避是秒级的**（5s / 15s / 60s，最多三次重试），落在 `retry_at_epoch` 上，
/// `scheduled_minute` 保持不动。原实现是把计划重抽到「窗口内随机分钟」（`from = now+1`），
/// 效果是**一分钟内无限重打**同一个账号 —— 上游对此的回敬是
/// `500 {"code":10001,"msg":"签到失败，请稍后重试"}`（参考实现的事故记录）。
///
/// - 不可重试的失败：清空退避并标记当天终止；
/// - 退避点落在当天窗口之外：今天放弃，等跨天重规划。
///
/// `attempted` 是 `(账号 id, 是否值得重试)`。
/// 返回是否有账号的计划状态被改动（需要落盘）。
pub(super) fn reschedule_failed_plans(
    schedules: &mut HashMap<String, BuddyAccountScheduleState>,
    attempted: &[(String, bool)],
    today: &str,
    current_minute: i32,
    start_min: i32,
    end_min: i32,
    now_epoch: i64,
) -> bool {
    if current_minute < start_min || current_minute > end_min {
        return false;
    }
    let mut changed = false;
    for (account_id, retryable) in attempted {
        let Some(state) = schedules.get_mut(account_id) else {
            continue;
        };
        // 成功签到会把 last_checked_date 置为今天：没置上的才是真失败
        if state.scheduled_date != today || state.last_checked_date.as_deref() == Some(today) {
            continue;
        }
        if !*retryable {
            state.retry_count = 0;
            state.retry_at_epoch = None;
            state.exhausted = true;
            changed = true;
            continue;
        }
        state.retry_count = state.retry_count.saturating_add(1);
        let window_end =
            now_epoch - now_epoch.rem_euclid(60) + i64::from(end_min - current_minute + 1) * 60;
        let delay = checkin_retry_delay_secs(state.retry_count) as i64;
        state.exhausted = state.retry_count as usize > CHECKIN_RETRY_DELAYS_SECS.len()
            || now_epoch + delay >= window_end;
        state.retry_at_epoch = (!state.exhausted).then_some(now_epoch + delay);
        changed = true;
    }
    changed
}

pub(super) fn mark_schedule_checked(
    schedules: &mut HashMap<String, BuddyAccountScheduleState>,
    account_id: &str,
    today: &str,
    current_minute: i32,
) {
    let schedule =
        schedules
            .entry(account_id.to_string())
            .or_insert_with(|| BuddyAccountScheduleState {
                scheduled_date: today.to_string(),
                scheduled_minute: current_minute,
                last_checked_date: None,
                last_checked_time: None,
                retry_count: 0,
                exhausted: false,
                retry_at_epoch: None,
            });
    schedule.last_checked_date = Some(today.to_string());
    schedule.last_checked_time = Some(Local::now().format("%H:%M:%S").to_string());
    // 成功/已签到：退避状态清零，明天从「第一次」重新算
    schedule.exhausted = false;
    schedule.retry_count = 0;
    schedule.retry_at_epoch = None;
}

/// 把一条签到结果写进每日归档（失败只打日志，绝不影响签到主流程）。
pub(super) fn archive_checkin(account: &BuddyAccount, patch: CheckinPatch) {
    let email = if account.email.trim().is_empty() {
        account.id.clone()
    } else {
        account.email.clone()
    };
    let today = get_today_date_string();
    if let Err(err) = daily_history::upsert_checkin(&today, &account.id, &email, patch) {
        eprintln!(
            "[BuddyAutoCheckin] 写入签到归档失败({}): {}",
            account.id, err
        );
    }
}

/// 计划发生变化（跨天重规划 / 首次生成）时同步归档：
/// - 旧日期的记录收尾为 `unfinished`（日历上区分「没做」与「没做完」）；
/// - 新日期的计划时间写入归档（状态仅在为空时置 `pending`，不降级已成功的记录）。
pub(super) fn archive_schedule_changes(
    before: &HashMap<String, BuddyAccountScheduleState>,
    after: &BuddyAutoCheckinConfig,
    accounts: &[BuddyAccount],
) {
    let today = get_today_date_string();
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
                        "[BuddyAutoCheckin] 收尾旧日期归档失败({}): {}",
                        o.scheduled_date, err
                    );
                }
            }
        }
        if let Some(n) = new {
            if n.scheduled_date == today {
                let plan_time = format_minutes(n.scheduled_minute);
                if let Err(err) = daily_history::upsert_checkin_plan(
                    &today,
                    &account.id,
                    &account.email,
                    &plan_time,
                ) {
                    eprintln!(
                        "[BuddyAutoCheckin] 写入计划归档失败({}): {}",
                        account.id, err
                    );
                }
            }
        }
    }
}

/// Retry eligibility must not depend on the original plan's five-minute grace.
pub fn checkin_schedule_due(
    state: &BuddyAccountScheduleState,
    today: &str,
    minute: i32,
    end_min: i32,
    now_epoch: i64,
) -> bool {
    if state.scheduled_date != today
        || state.last_checked_date.as_deref() == Some(today)
        || state.exhausted
        || minute > end_min
    {
        return false;
    }
    match state.retry_at_epoch {
        Some(at) => at <= now_epoch,
        None => is_plan_due(state.scheduled_minute, minute, end_min),
    }
}

pub fn next_checkin_delay(config: &BuddyAutoCheckinConfig, now_epoch: i64) -> Duration {
    let today = get_today_date_string();
    let next = config
        .account_schedules
        .as_ref()
        .into_iter()
        .flat_map(|all| all.values())
        .filter(|state| {
            state.scheduled_date == today
                && !state.exhausted
                && state.last_checked_date.as_deref() != Some(today.as_str())
        })
        .filter_map(|state| state.retry_at_epoch)
        .filter(|at| *at > now_epoch)
        .min();
    next.map(|at| Duration::from_secs((at - now_epoch) as u64).min(SCHEDULER_POLL_DELAY))
        .unwrap_or(SCHEDULER_POLL_DELAY)
}
