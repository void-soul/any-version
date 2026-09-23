//! Buddy 自动派 Buddy 旅行调度器（WorkBuddy 专属活动）。
//!
//! 参考 workbuddy-auto-credits 技能的旅行 API 与状态机：
//! - arrived（已回来）→ claim 领取奖励
//! - idle 且未达每日上限 → depart 派出（location_id 1~4）
//! - traveling（旅行中）→ 等待下一轮
//! - idle 且已达上限 → 当日完成
//!
//! 与自动签到一致：每个账号在 [startTime, endTime] 窗口内随机分配**派出时间**，
//! 避免所有账号同时派出；领取检测全天进行（旅行时长 1~4 小时随机，返回后下一轮领取）。
//! 计划时间已过而当天没派出时（应用没开着 / 上一轮失败）重排到窗口内的新随机时间，
//! 不「打开应用就立刻补派」；窗口已全过去则今天放弃，等明天跨天重规划。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex, MutexGuard, OnceLock,
};
use std::time::Duration;

use chrono::{Local, Timelike};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::sync::Notify;

use super::action_log;
use super::daily_history::{self, travel_status, TravelPatch};
use super::models::{BuddyAccount, BuddyPlatform};
use super::{api, store};

static IS_TRAVEL_RUNNING: AtomicBool = AtomicBool::new(false);
static STORAGE_LOCK: Mutex<()> = Mutex::new(());
static SCHEDULER_WAKE: OnceLock<Notify> = OnceLock::new();

const SCHEDULER_POLL_DELAY: Duration = Duration::from_secs(60);
const INITIAL_RETRY_DELAY: Duration = Duration::from_secs(5 * 60);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(60 * 60);

/// 派遣地点 id 上限（1 咖啡馆 / 2 商场店铺 / 3 健身房 / 4 古镇客栈）
const LOCATION_ID_MAX: i64 = 4;

struct TravelGuard;
impl Drop for TravelGuard {
    fn drop(&mut self) {
        IS_TRAVEL_RUNNING.store(false, Ordering::SeqCst);
    }
}

/// 单个账号的当日旅行计划与状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddyAccountTravelState {
    pub scheduled_date: String,
    pub scheduled_minute: i32,
    /// 当日派出完成的日期（depart 成功后写入）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_depart_date: Option<String>,
    /// 实际派出时刻（HH:MM:SS）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_depart_time: Option<String>,
    /// 当日流程完成的日期（领取成功或已达派出上限）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_done_date: Option<String>,
    /// 最近一次领取的积分
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_reward_credit: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddyAutoTravelConfig {
    pub enabled: bool,
    pub start_time: String,
    pub end_time: String,
    /// 派遣地点 id（1 咖啡馆 / 2 商场店铺 / 3 健身房 / 4 古镇客栈）
    pub location_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_schedules: Option<HashMap<String, BuddyAccountTravelState>>,
}

impl Default for BuddyAutoTravelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            start_time: "08:00".to_string(),
            end_time: "12:00".to_string(),
            location_id: 1,
            account_schedules: None,
        }
    }
}

fn get_config_file_path() -> PathBuf {
    crate::commands::config::get_data_dir()
        .join("buddy")
        .join("auto_travel_config.json")
}

fn scheduler_wake() -> &'static Notify {
    SCHEDULER_WAKE.get_or_init(Notify::new)
}

fn wake_scheduler() {
    scheduler_wake().notify_one();
}

fn lock_storage() -> Result<MutexGuard<'static, ()>, String> {
    STORAGE_LOCK
        .lock()
        .map_err(|_| "Buddy 自动旅行存储锁已损坏".to_string())
}

fn validate_time(value: &str) -> Option<i32> {
    if value.len() != 5 || value.as_bytes().get(2) != Some(&b':') {
        return None;
    }
    let hour = value.get(0..2)?.parse::<i32>().ok()?;
    let minute = value.get(3..5)?.parse::<i32>().ok()?;
    if (0..=23).contains(&hour) && (0..=59).contains(&minute) {
        Some(hour * 60 + minute)
    } else {
        None
    }
}

fn validate_config(config: &BuddyAutoTravelConfig) -> Result<(), String> {
    let start = validate_time(&config.start_time)
        .ok_or_else(|| format!("自动旅行开始时间无效: {}", config.start_time))?;
    let end = validate_time(&config.end_time)
        .ok_or_else(|| format!("自动旅行结束时间无效: {}", config.end_time))?;
    if start > end {
        return Err("自动旅行开始时间不能晚于结束时间".to_string());
    }
    if !(1..=LOCATION_ID_MAX).contains(&config.location_id) {
        return Err(format!("派遣地点无效: {}", config.location_id));
    }
    if let Some(schedules) = &config.account_schedules {
        for (account_id, schedule) in schedules {
            if !(0..=1439).contains(&schedule.scheduled_minute) {
                return Err(format!(
                    "账号 {} 的自动旅行分钟无效: {}",
                    account_id, schedule.scheduled_minute
                ));
            }
        }
    }
    Ok(())
}

fn write_atomic(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {}", e))?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, content).map_err(|e| format!("写入临时文件失败: {}", e))?;
    fs::rename(&tmp, path).map_err(|e| format!("原子替换文件失败: {}", e))
}

fn read_config_from_path(path: &Path) -> Result<Option<BuddyAutoTravelConfig>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(path).map_err(|e| format!("读取自动旅行配置失败: {}", e))?;
    let config: BuddyAutoTravelConfig =
        serde_json::from_str(&content).map_err(|e| format!("解析自动旅行配置失败: {}", e))?;
    validate_config(&config)?;
    Ok(Some(config))
}

fn get_today_date_string() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

pub fn get_config_checked() -> Result<BuddyAutoTravelConfig, String> {
    let _guard = lock_storage()?;
    Ok(read_config_from_path(&get_config_file_path())?.unwrap_or_default())
}

fn save_config_without_wake(config: &BuddyAutoTravelConfig) -> Result<(), String> {
    validate_config(config)?;
    let _guard = lock_storage()?;
    let content = serde_json::to_string_pretty(config)
        .map_err(|e| format!("序列化自动旅行配置失败: {}", e))?;
    write_atomic(&get_config_file_path(), &content)
}

pub fn save_config(config: &BuddyAutoTravelConfig) -> Result<(), String> {
    let result = save_config_without_wake(config);
    if result.is_ok() {
        wake_scheduler();
    }
    result
}

/// 今天是否还有账号没有生成派出计划（跨天 / 首次运行 / 新增账号）。
///
/// 与自动签到同构：让「每天打开应用」就把今天的计划生成出来，
/// 不再等到 `startTime`，从根上消除前端"一直显示未安排"的现象。
pub fn needs_replan(config: &BuddyAutoTravelConfig, accounts: &[BuddyAccount]) -> bool {
    let today = super::auto_checkin::get_today_date_string();
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
fn archive_travel(account: &BuddyAccount, date: &str, patch: TravelPatch) {
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
fn now_time_string() -> String {
    daily_history::now_time_string()
}

/// 计划发生变化（跨天重规划 / 首次生成）时同步归档（与签到侧同构）。
fn archive_schedule_changes(
    before: &HashMap<String, BuddyAccountTravelState>,
    after: &BuddyAutoTravelConfig,
    accounts: &[BuddyAccount],
) {
    let today = super::auto_checkin::get_today_date_string();
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
                let plan_time = super::auto_checkin::format_minutes(n.scheduled_minute);
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
    let (start_min, end_min) = super::auto_checkin::plan_window(&config.start_time, &config.end_time);
    let current_minute = super::auto_checkin::current_minute_of_day();

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
            super::auto_checkin::draw_plan_minute(start_min, end_min, current_minute);

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
fn reschedule_missed_plans(
    config: &mut BuddyAutoTravelConfig,
    accounts: &[BuddyAccount],
    current_minute: i32,
) -> (bool, Vec<action_log::BuddyActionLogEntry>) {
    let today = get_today_date_string();
    let (start_min, end_min) = super::auto_checkin::plan_window(&config.start_time, &config.end_time);
    let Some((from, to)) = super::auto_checkin::remaining_window(start_min, end_min, current_minute)
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
            || !super::auto_checkin::is_plan_missed(schedule.scheduled_minute, current_minute)
        {
            continue;
        }

        let old_minute = schedule.scheduled_minute;
        let new_minute = super::auto_checkin::draw_plan_minute(from, to, current_minute);
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
                super::auto_checkin::format_minutes(old_minute),
                super::auto_checkin::format_minutes(new_minute)
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
fn reschedule_failed_plans(
    schedules: &mut HashMap<String, BuddyAccountTravelState>,
    attempted: &[String],
    today: &str,
    current_minute: i32,
    start_min: i32,
    end_min: i32,
) -> bool {
    let Some((from, to)) = super::auto_checkin::remaining_window(start_min, end_min, current_minute)
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
        state.scheduled_minute = super::auto_checkin::draw_plan_minute(from, to, current_minute);
        changed = true;
    }
    changed
}

/// 展示用邮箱（缺失时退回账号 id）。
fn account_email(account: &BuddyAccount) -> String {
    if account.email.trim().is_empty() {
        account.id.clone()
    } else {
        account.email.clone()
    }
}

/// 执行一轮自动旅行状态机（每轮「查状态 → 至多一个动作」）。
///
/// - 派出：仅当到达当日计划时间（或 force）且当日未派出过；计划时间已过（应用没开着 /
///   上一轮失败）时由 `reschedule_missed_plans` 重排到窗口内的新随机时间，不「立刻补派」；
/// - 领取：全天检测 arrived（旅行时长 1~4 小时随机，返回后下一轮领取）；
/// - 已达派出上限：当日完成；
/// - 窗口已全部过去：今天放弃（等明天跨天重规划）。
pub async fn run_auto_travel_cycle_if_needed(
    app: &AppHandle,
    force: bool,
) -> Result<String, String> {
    if IS_TRAVEL_RUNNING.swap(true, Ordering::SeqCst) {
        return Ok("already_running".to_string());
    }
    let _guard = TravelGuard;

    let mut config = get_config_checked()?;
    if !config.enabled && !force {
        return Ok("disabled".to_string());
    }

    // 旅行是 WorkBuddy 专属活动
    let accounts = store::list_accounts(BuddyPlatform::Workbuddy);
    if accounts.is_empty() {
        return Ok("no_accounts".to_string());
    }

    let now = Local::now();
    let current_minute = (now.hour() * 60 + now.minute()) as i32;
    let today_str = get_today_date_string();
    let (start_min, end_min) = super::auto_checkin::plan_window(&config.start_time, &config.end_time);
    let cross_day = needs_replan(&config, &accounts);
    let allow_generate = force
        || cross_day
        || current_minute >= super::auto_checkin::parse_time_to_minutes(&config.start_time);
    let before_schedules = config.account_schedules.clone().unwrap_or_default();
    let mut schedule_changed = ensure_travel_schedules(&mut config, &accounts, allow_generate);
    // 计划时间已过但今天还没派出（应用没开着 / 上一轮失败）→ 重排到窗口内的新随机时间，
    // 而不是「打开应用就立刻补派」。手动立即巡检（force）不参与重排。
    let (rescheduled, reschedule_entries) = if force {
        (false, Vec::new())
    } else {
        reschedule_missed_plans(&mut config, &accounts, current_minute)
    };
    schedule_changed |= rescheduled;
    if schedule_changed {
        save_config_without_wake(&config)?;
        // 计划变更同步归档：旧日期收尾 + 今日计划时间
        archive_schedule_changes(&before_schedules, &config, &accounts);
        let _ = app.emit("buddy-auto-travel-config-changed", ());
    }

    let schedules = config.account_schedules.clone().unwrap_or_default();
    let mut new_schedules = schedules.clone();
    let mut acted = false;
    let mut auth_expired = false;
    let mut entries: Vec<action_log::BuddyActionLogEntry> = reschedule_entries;
    // 本轮到点、真正跑过状态机的账号（派出失败者要把重试时机挪到新的随机时刻）
    let mut attempted_ids: Vec<String> = Vec::new();

    for account in &accounts {
        let Some(sch) = schedules.get(&account.id).cloned() else {
            continue;
        };
        let email_display = if !account.email.trim().is_empty() {
            account.email.clone()
        } else {
            account.id.clone()
        };
        if sch.scheduled_date != today_str {
            continue;
        }
        if sch.last_done_date.as_deref() == Some(&today_str) {
            continue; // 当日流程已完成
        }
        // 已派出过 → 继续跟领取（不受窗口限制：旅行可能在窗口结束后才回来）；
        // 否则只有「到点且仍在窗口内、没错过太久」才派出。
        let due = sch.last_depart_date.as_deref() == Some(&today_str)
            || force
            || super::auto_checkin::is_plan_due(sch.scheduled_minute, current_minute, end_min);
        if !due {
            continue;
        }
        attempted_ids.push(account.id.clone());

        // 这笔旅行归属的日期：归来/领取都归档到出发那天
        // （跨零点时，比如 23:50 出发、次日 01:00 归来，仍记在出发日）
        let travel_day = sch
            .last_depart_date
            .clone()
            .unwrap_or_else(|| today_str.clone());

        let status = match api::travel_status(
            &account.access_token,
            account.uid.as_deref(),
            account.enterprise_id.as_deref(),
            account.domain.as_deref(),
        )
        .await
        {
            Ok(s) => s,
            Err(e) => {
                if e.starts_with(api::TRAVEL_AUTH_EXPIRED_PREFIX) {
                    auth_expired = true;
                    eprintln!("[BuddyAutoTravel] 账号 {} 令牌失效: {}", account.id, e);
                    entries.push(action_log::make_entry(
                        "travel",
                        &account.id,
                        &email_display,
                        "failed",
                        Some("令牌已失效，请打开 WorkBuddy 刷新登录态".to_string()),
                        None,
                    ));
                    archive_travel(
                        account,
                        &travel_day,
                        TravelPatch::new()
                            .status(travel_status::FAILED)
                            .message(Some(
                                "令牌已失效，请打开 WorkBuddy 刷新登录态".to_string(),
                            )),
                    );
                    continue;
                }
                eprintln!("[BuddyAutoTravel] 账号 {} 查询旅行状态失败: {}", account.id, e);
                entries.push(action_log::make_entry(
                    "travel",
                    &account.id,
                    &email_display,
                    "failed",
                    Some(e.clone()),
                    None,
                ));
                archive_travel(
                    account,
                    &travel_day,
                    TravelPatch::new()
                        .status(travel_status::FAILED)
                        .message(Some(e)),
                );
                continue;
            }
        };

        let Some(state) = new_schedules.get_mut(&account.id) else {
            continue;
        };

        if status.state == "arrived" {
            // 我们是"这一轮才发现它回来"，所以归来时间取当下，误差 ≤ 轮询间隔
            let back_time = now_time_string();
            let Some(record_id) = status.record_id.clone() else {
                eprintln!("[BuddyAutoTravel] 账号 {} arrived 但缺少 record_id", account.id);
                entries.push(action_log::make_entry(
                    "travel",
                    &account.id,
                    &email_display,
                    "failed",
                    Some("旅行已归来但缺少 record_id，无法领取".to_string()),
                    None,
                ));
                // 它的确已经回来了，只是拿不到 record_id 领不了：按「已归来待领取」归档
                archive_travel(
                    account,
                    &travel_day,
                    TravelPatch::new()
                        .back_time(back_time)
                        .status(travel_status::ARRIVED)
                        .message(Some("旅行已归来但缺少 record_id，无法领取".to_string())),
                );
                continue;
            };
            match api::travel_claim(
                record_id,
                &account.access_token,
                account.uid.as_deref(),
                account.enterprise_id.as_deref(),
                account.domain.as_deref(),
            )
            .await
            {
                Ok(reward) => {
                    state.last_done_date = Some(today_str.clone());
                    state.last_reward_credit = reward;
                    acted = true;
                    entries.push(action_log::make_entry(
                        "travel",
                        &account.id,
                        &email_display,
                        "claimed",
                        Some("领取旅行奖励".to_string()),
                        reward,
                    ));
                    archive_travel(
                        account,
                        &travel_day,
                        TravelPatch::new()
                            .back_time(back_time.clone())
                            .claim_time(now_time_string())
                            .status(travel_status::CLAIMED)
                            .credit(reward),
                    );
                    eprintln!(
                        "[BuddyAutoTravel] 账号 {} 领取旅行奖励: {:?} 积分",
                        account.id, reward
                    );
                }
                Err(e) => {
                    if e.starts_with(super::api::TRAVEL_REJECTED_PREFIX)
                        && (e.contains("已领取") || e.to_lowercase().contains("already"))
                    {
                        // 奖励已被领取过（可能在客户端手动领了）→ 当日完成
                        state.last_done_date = Some(today_str.clone());
                        acted = true;
                        entries.push(action_log::make_entry(
                            "travel",
                            &account.id,
                            &email_display,
                            "claimed",
                            Some("奖励已在客户端领取".to_string()),
                            None,
                        ));
                        archive_travel(
                            account,
                            &travel_day,
                            TravelPatch::new()
                                .back_time(back_time)
                                .claim_time(now_time_string())
                                .status(travel_status::CLAIMED)
                                .message(Some("奖励已在客户端领取".to_string())),
                        );
                        eprintln!("[BuddyAutoTravel] 账号 {} 奖励已领取过，当日完成", account.id);
                    } else {
                        eprintln!(
                            "[BuddyAutoTravel] 账号 {} 领取失败（下轮重试）: {}",
                            account.id, e
                        );
                        entries.push(action_log::make_entry(
                            "travel",
                            &account.id,
                            &email_display,
                            "failed",
                            Some(e.clone()),
                            None,
                        ));
                        archive_travel(
                            account,
                            &travel_day,
                            TravelPatch::new()
                                .back_time(back_time)
                                .status(travel_status::FAILED)
                                .message(Some(e)),
                        );
                    }
                }
            }
        } else if status.state == "idle" && status.daily_limit_reached {
            // 今日已派出过（可能是在客户端手动派的）→ 当日完成
            state.last_done_date = Some(today_str.clone());
            acted = true;
            entries.push(action_log::make_entry(
                "travel",
                &account.id,
                &email_display,
                "limit_reached",
                Some("今日已达派出上限".to_string()),
                None,
            ));
            // 从没由我们派出过 → 说明名额被外部用掉了，次数至少记 1 次
            let patch = TravelPatch::new().status(travel_status::LIMIT_REACHED);
            let patch = if sch.last_depart_date.as_deref() == Some(&today_str) {
                patch
            } else {
                patch.add_depart()
            };
            archive_travel(account, &travel_day, patch);
        } else if status.state == "idle" {
            // 空闲且未达上限：到达计划时间后派出（当日只派一次）
            if sch.last_depart_date.as_deref() != Some(&today_str) {
                match api::travel_depart(
                    config.location_id,
                    &account.access_token,
                    account.uid.as_deref(),
                    account.enterprise_id.as_deref(),
                    account.domain.as_deref(),
                )
                .await
                {
                    Ok(()) => {
                        let depart_time = now_time_string();
                        state.last_depart_date = Some(today_str.clone());
                        state.last_depart_time = Some(depart_time.clone());
                        acted = true;
                        entries.push(action_log::make_entry(
                            "travel",
                            &account.id,
                            &email_display,
                            "departed",
                            Some(format!("已派出旅行（地点 {}）", config.location_id)),
                            None,
                        ));
                        archive_travel(
                            account,
                            &today_str,
                            TravelPatch::new()
                                .depart_time(depart_time)
                                .status(travel_status::TRAVELING)
                                .add_depart(),
                        );
                        eprintln!(
                            "[BuddyAutoTravel] 账号 {} 已派出旅行（地点 {}）",
                            account.id, config.location_id
                        );
                    }
                    Err(e) => {
                        if e.starts_with(super::api::TRAVEL_REJECTED_PREFIX) {
                            // 业务拒绝（如「今日太累了」等提示）：当日不再派出，
                            // 否则会以调度间隔无限重试打扰官方接口
                            state.last_done_date = Some(today_str.clone());
                            acted = true;
                            eprintln!(
                                "[BuddyAutoTravel] 账号 {} 派出被拒绝，当日放弃: {}",
                                account.id, e
                            );
                            entries.push(action_log::make_entry(
                                "travel",
                                &account.id,
                                &email_display,
                                "depart_rejected",
                                Some(e.clone()),
                                None,
                            ));
                            archive_travel(
                                account,
                                &travel_day,
                                TravelPatch::new()
                                    .status(travel_status::REJECTED)
                                    .message(Some(e)),
                            );
                        } else {
                            // 网络/服务端临时错误：下一轮重试
                            eprintln!(
                                "[BuddyAutoTravel] 账号 {} 派出失败（下轮重试）: {}",
                                account.id, e
                            );
                            entries.push(action_log::make_entry(
                                "travel",
                                &account.id,
                                &email_display,
                                "failed",
                                Some(e.clone()),
                                None,
                            ));
                            archive_travel(
                                account,
                                &travel_day,
                                TravelPatch::new()
                                    .status(travel_status::FAILED)
                                    .message(Some(e)),
                            );
                        }
                    }
                }
            }
        }
        // traveling：旅行中，等下一轮领取
    }

    // 派出失败（网络错误/令牌失效等）的重试时机也改成「窗口内新随机时间」：否则下一个
    // 轮询就会以「已到点」立刻重打同一个失败账号。
    let retry_moved = reschedule_failed_plans(
        &mut new_schedules,
        &attempted_ids,
        &today_str,
        current_minute,
        start_min,
        end_min,
    );

    config.account_schedules = Some(new_schedules);
    if retry_moved {
        // 计划时间被挪动过 → 同步归档（日历与卡片都以归档为准）
        archive_schedule_changes(&schedules, &config, &accounts);
    }
    save_config_without_wake(&config)?;
    action_log::append_action_logs(&entries)?;
    let _ = app.emit("buddy-action-logs-changed", ());
    let _ = app.emit("buddy-auto-travel-config-changed", ());

    // 失败不再让调度器拉长间隔：重试时机已改成「重排到窗口内的新随机时间」，
    // 轮询若退避到分钟以上就会错过这些随机时刻。失败信息已写进行为日志。
    if auth_expired {
        eprintln!(
            "[BuddyAutoTravel] 本轮存在令牌失效账号：已过计划时间的账号已重排到窗口内新随机时间（窗口已结束的今天放弃）"
        );
    }
    Ok("completed".to_string())
}

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
    let today_str = super::auto_checkin::get_today_date_string();
    let schedules = config.account_schedules.clone().unwrap_or_default();
    let archive = daily_history::get_day(&today_str).unwrap_or_default();

    let mut generated = false;
    let mut tasks = Vec::with_capacity(accounts.len());
    let current_minute = super::auto_checkin::current_minute_of_day();
    let (start_min, end_min) = super::auto_checkin::plan_window(&config.start_time, &config.end_time);
    let window_closed =
        super::auto_checkin::remaining_window(start_min, end_min, current_minute).is_none();

    for account in &accounts {
        let schedule = schedules.get(&account.id);
        let scheduled_today = schedule
            .map(|s| s.scheduled_date == today_str)
            .unwrap_or(false);
        if scheduled_today {
            generated = true;
        }
        let archived = archive.get(&account.id).and_then(|record| record.travel.as_ref());
        let email = if account.email.trim().is_empty() {
            account.id.clone()
        } else {
            account.email.clone()
        };
        let plan_time = archived.and_then(|t| t.plan_time.clone()).or_else(|| {
            if scheduled_today {
                schedule.map(|s| super::auto_checkin::format_minutes(s.scheduled_minute))
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
                    super::auto_checkin::is_plan_missed(s.scheduled_minute, current_minute)
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

fn next_retry_delay(current: Duration) -> Duration {
    current.saturating_mul(2).min(MAX_RETRY_DELAY)
}

/// 启动后台自动旅行调度（仅 WorkBuddy：旅行是 WorkBuddy 专属活动）。
pub fn start_auto_travel_scheduler(app: AppHandle) {
    let wake = scheduler_wake();
    tauri::async_runtime::spawn(async move {
        eprintln!("[BuddyAutoTravel] 后台自动旅行调度服务已启动");
        let mut next_delay = Duration::ZERO;
        let mut retry_delay = INITIAL_RETRY_DELAY;
        loop {
            if !next_delay.is_zero() {
                tokio::select! {
                    _ = tokio::time::sleep(next_delay) => {}
                    _ = wake.notified() => {
                        next_delay = Duration::ZERO;
                        retry_delay = INITIAL_RETRY_DELAY;
                        continue;
                    }
                }
            }
            match run_auto_travel_cycle_if_needed(&app, false).await {
                // 固定短间隔轮询：账号级失败已改成「重排到窗口内新随机时间」，
                // 轮询若退避到分钟级就会错过这些随机时刻。
                Ok(_) => {
                    next_delay = SCHEDULER_POLL_DELAY;
                    retry_delay = INITIAL_RETRY_DELAY;
                }
                Err(err) => {
                    next_delay = retry_delay;
                    retry_delay = next_retry_delay(retry_delay);
                    eprintln!(
                        "[BuddyAutoTravel] 调度异常: {}，{} 秒后重试",
                        err,
                        next_delay.as_secs()
                    );
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_account(id: &str, email: &str) -> BuddyAccount {
        BuddyAccount {
            id: id.to_string(),
            platform: "workbuddy".to_string(),
            email: email.to_string(),
            uid: Some(format!("uid-{}", id)),
            nickname: None,
            enterprise_id: None,
            enterprise_name: None,
            tags: None,
            access_token: format!("token-{}", id),
            refresh_token: None,
            token_type: Some("Bearer".to_string()),
            expires_at: None,
            domain: None,
            plan_type: None,
            dosage_notify_code: None,
            dosage_notify_zh: None,
            dosage_notify_en: None,
            payment_type: None,
            quota_raw: None,
            usage_raw: None,
            profile_raw: None,
            status: None,
            status_reason: None,
            quota_query_last_error: None,
            quota_query_last_error_at: None,
            usage_updated_at: None,
            last_checkin_time: None,
            checkin_streak: 0,
            checkin_rewards: None,
            auth_raw: None,
            expiry_times: Default::default(),
            created_at: 0,
            last_used: 0,
        }
    }

    #[test]
    fn test_validate_config_rejects_bad_time_and_location() {
        let mut config = BuddyAutoTravelConfig::default();
        assert!(validate_config(&config).is_ok());

        config.end_time = "06:00".to_string(); // start 08:00 > end
        assert!(validate_config(&config).is_err());

        config.start_time = "08:00".to_string();
        config.end_time = "12:00".to_string();
        config.location_id = 5;
        assert!(validate_config(&config).is_err());
        config.location_id = 1;
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn test_ensure_travel_schedules_respects_allow_generate() {
        let mut config = BuddyAutoTravelConfig::default();
        let accounts = vec![sample_account("acc_1", "a@x.com")];

        // 未到开始时间：不生成
        assert!(!ensure_travel_schedules(&mut config, &accounts, false));
        assert!(config.account_schedules.is_none());

        // 到达开始时间：生成，分钟落在窗口内
        assert!(ensure_travel_schedules(&mut config, &accounts, true));
        let schedules = config.account_schedules.clone().unwrap();
        let sch = schedules.get("acc_1").unwrap();
        assert_eq!(sch.scheduled_date, get_today_date_string());
        assert!((480..=720).contains(&sch.scheduled_minute));

        // 同日重新生成保留当日进度
        if let Some(map) = config.account_schedules.as_mut() {
            let sch = map.get_mut("acc_1").unwrap();
            sch.last_depart_date = Some(get_today_date_string());
        }
        // 把 scheduled_minute 移出窗口强制重新生成
        if let Some(map) = config.account_schedules.as_mut() {
            map.get_mut("acc_1").unwrap().scheduled_minute = 0;
        }
        assert!(ensure_travel_schedules(&mut config, &accounts, true));
        let sch = config.account_schedules.unwrap().remove("acc_1").unwrap();
        assert_eq!(sch.last_depart_date.as_deref(), Some(get_today_date_string().as_str()));
    }

    #[test]
    fn test_travel_reschedule_missed_plan_moves_to_future_and_logs() {
        let accounts = vec![sample_account("acc_1", "a@x.com")];
        let today = get_today_date_string();
        // 窗口 08:00~12:00（480~720），计划 08:30 没派出，现在 10:00
        let mut config = travel_config_with(travel_state(&today, 510));
        let (changed, entries) = reschedule_missed_plans(&mut config, &accounts, 600);
        assert!(changed);
        let schedule = config.account_schedules.unwrap().remove("acc_1").unwrap();
        assert!((601..=720).contains(&schedule.scheduled_minute));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, "travel");
        assert_eq!(entries[0].status, "rescheduled");
    }

    #[test]
    fn test_travel_reschedule_skips_departed_done_and_closed_window() {
        let accounts = vec![sample_account("acc_1", "a@x.com")];
        let today = get_today_date_string();
        let build = |mutate: fn(&mut BuddyAccountTravelState)| {
            let mut state = travel_state(&today, 510);
            mutate(&mut state);
            travel_config_with(state)
        };

        // 已派出（旅行中 / 已归来待领取）→ 不重排
        let mut config = build(|s| s.last_depart_date = Some(get_today_date_string()));
        assert!(!reschedule_missed_plans(&mut config, &accounts, 600).0);

        // 当日已完成 → 不重排
        let mut config = build(|s| s.last_done_date = Some(get_today_date_string()));
        assert!(!reschedule_missed_plans(&mut config, &accounts, 600).0);

        // 窗口已全部过去（现在 13:20）→ 今天放弃：计划不动、不写日志
        let mut config = build(|_| {});
        let (changed, entries) = reschedule_missed_plans(&mut config, &accounts, 800);
        assert!(!changed);
        assert!(entries.is_empty());
        assert_eq!(
            config
                .account_schedules
                .unwrap()
                .get("acc_1")
                .unwrap()
                .scheduled_minute,
            510
        );

        // 还在宽限内（08:30 的计划，现在 08:33）→ 不重排，照旧立刻派出
        let mut config = build(|_| {});
        assert!(!reschedule_missed_plans(&mut config, &accounts, 513).0);
    }

    #[test]
    fn test_travel_reschedule_failed_plans_skips_departed_and_done() {
        let today = get_today_date_string();
        let mut traveling = travel_state(&today, 600);
        traveling.last_depart_date = Some(today.clone());
        let mut done = travel_state(&today, 600);
        done.last_done_date = Some(today.clone());
        let mut schedules = HashMap::from([
            ("acc_failed".to_string(), travel_state(&today, 600)),
            ("acc_traveling".to_string(), traveling),
            ("acc_done".to_string(), done),
        ]);
        let attempted = vec![
            "acc_failed".to_string(),
            "acc_traveling".to_string(),
            "acc_done".to_string(),
        ];

        // 窗口 08:00~12:00，现在 10:00 → 只有派出失败的账号挪到窗口内未来时间
        assert!(reschedule_failed_plans(
            &mut schedules, &attempted, &today, 600, 480, 720
        ));
        assert!((601..=720).contains(&schedules["acc_failed"].scheduled_minute));
        assert_eq!(schedules["acc_traveling"].scheduled_minute, 600);
        assert_eq!(schedules["acc_done"].scheduled_minute, 600);
    }

    #[test]
    fn test_config_serde_roundtrip() {
        let config = BuddyAutoTravelConfig {
            enabled: true,
            start_time: "08:00".to_string(),
            end_time: "10:00".to_string(),
            location_id: 3,
            account_schedules: Some(HashMap::from([(
                "acc_1".to_string(),
                BuddyAccountTravelState {
                    scheduled_date: "2026-09-13".to_string(),
                    scheduled_minute: 500,
                    last_depart_date: Some("2026-09-13".to_string()),
                    last_depart_time: Some("08:32:11".to_string()),
                    last_done_date: None,
                    last_reward_credit: Some(8),
                },
            )])),
        };
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"locationId\":3"));
        let deserialized: BuddyAutoTravelConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, config);
    }

    fn travel_state(date: &str, minute: i32) -> BuddyAccountTravelState {
        BuddyAccountTravelState {
            scheduled_date: date.to_string(),
            scheduled_minute: minute,
            last_depart_date: None,
            last_depart_time: None,
            last_done_date: None,
            last_reward_credit: None,
        }
    }

    /// 窗口固定 08:00~12:00（480~720）的单账号配置，便于用注入的 `current_minute` 做确定性测试。
    fn travel_config_with(state: BuddyAccountTravelState) -> BuddyAutoTravelConfig {
        BuddyAutoTravelConfig {
            enabled: true,
            start_time: "08:00".to_string(),
            end_time: "12:00".to_string(),
            location_id: 1,
            account_schedules: Some(HashMap::from([("acc_1".to_string(), state)])),
        }
    }

    #[test]
    fn test_needs_replan_cross_day_and_empty_accounts() {
        let accounts = vec![sample_account("acc_1", "a@x.com")];
        let today = super::super::auto_checkin::get_today_date_string();

        // 没有计划 → 需要
        let mut config = BuddyAutoTravelConfig::default();
        assert!(needs_replan(&config, &accounts));
        // 没有账号 → 不需要（避免空转）
        assert!(!needs_replan(&config, &[]));

        // 今天的计划齐了 → 不需要
        config.account_schedules = Some(HashMap::from([(
            "acc_1".to_string(),
            travel_state(&today, 540),
        )]));
        assert!(!needs_replan(&config, &accounts));

        // 昨天的计划 → 需要（"打开应用即重新计划"）
        config.account_schedules = Some(HashMap::from([(
            "acc_1".to_string(),
            travel_state("2020-01-01", 540),
        )]));
        assert!(needs_replan(&config, &accounts));
    }

    #[test]
    fn test_cross_day_replan_clears_yesterday_progress() {
        let accounts = vec![sample_account("acc_1", "a@x.com")];
        let today = super::super::auto_checkin::get_today_date_string();
        let mut config = BuddyAutoTravelConfig {
            enabled: true,
            start_time: "08:00".to_string(),
            end_time: "12:00".to_string(),
            location_id: 1,
            account_schedules: Some(HashMap::from([(
                "acc_1".to_string(),
                BuddyAccountTravelState {
                    scheduled_date: "2020-01-01".to_string(),
                    scheduled_minute: 540,
                    last_depart_date: Some("2020-01-01".to_string()),
                    last_depart_time: Some("09:05:00".to_string()),
                    last_done_date: Some("2020-01-01".to_string()),
                    last_reward_credit: Some(7),
                },
            )])),
        };

        assert!(ensure_travel_schedules(&mut config, &accounts, true));
        let schedule = config
            .account_schedules
            .unwrap()
            .remove("acc_1")
            .expect("schedule");
        assert_eq!(schedule.scheduled_date, today);
        // 昨天的派出/完成状态不会带到今天（即"每天清空"）
        assert_eq!(schedule.last_depart_date, None);
        assert_eq!(schedule.last_depart_time, None);
        assert_eq!(schedule.last_done_date, None);
        assert_eq!(schedule.last_reward_credit, None);
        assert!((480..=720).contains(&schedule.scheduled_minute));
    }

    #[test]
    fn test_retry_delay_bounded_exponential_backoff() {
        assert_eq!(
            next_retry_delay(INITIAL_RETRY_DELAY),
            Duration::from_secs(10 * 60)
        );
        assert_eq!(next_retry_delay(MAX_RETRY_DELAY), MAX_RETRY_DELAY);
    }
}
