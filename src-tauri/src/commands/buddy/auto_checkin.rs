//! Buddy 自动签到调度器（复刻自 cockpit-tools `workbuddy_auto_checkin.rs`）。
//!
//! - 配置：enabled / startTime / endTime / accountSchedules（每账号当天随机签到分钟）
//! - 每轮：查询签到状态 → 未签到则执行 daily-checkin → 更新账号签到信息
//! - 错过不补执行：打开应用时发现「计划时间已过但今天没做过」的账号，重排到窗口内的
//!   新随机时间（`reschedule_missed_plans`）；窗口已全过去则今天放弃，等明天跨天重规划
//! - 失败重试：也改为重排新随机时间（`reschedule_failed_plans`），不再原地退避重试
//! - 调度：固定 30 秒轮询（应用启动时立即跑一轮），仅调度异常本身按退避重试
//! - 日志：按天记录，保留 30 天，前端可查看/清空

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
use super::daily_history::{self, checkin_status, CheckinPatch};
use super::models::{BuddyAccount, BuddyPlatform};
use super::{api, store};

static IS_CHECKIN_RUNNING: AtomicBool = AtomicBool::new(false);
static STORAGE_LOCK: Mutex<()> = Mutex::new(());
static SCHEDULER_WAKE: OnceLock<Notify> = OnceLock::new();

const SCHEDULER_POLL_DELAY: Duration = Duration::from_secs(30);
const INITIAL_RETRY_DELAY: Duration = Duration::from_secs(5 * 60);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(60 * 60);
/// 「错过计划时间」的宽限（分钟）。
///
/// 调度轮询是 30 秒一次，正常路径应当落在计划分钟当刻执行；但一轮可能因为并发
/// （用户点了「立即签到」）或账号多而跨过整分钟，所以只有超过宽限仍未执行的计划
/// 才算「错过」，据此重排——避免把正常的小延迟误判成错过。
pub const MISSED_PLAN_GRACE_MIN: i32 = 5;

struct CheckinGuard;
impl Drop for CheckinGuard {
    fn drop(&mut self) {
        IS_CHECKIN_RUNNING.store(false, Ordering::SeqCst);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddyAccountScheduleState {
    pub scheduled_date: String,
    pub scheduled_minute: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checked_date: Option<String>,
    /// 实际签到时刻（HH:MM:SS，当日成功/已签时记录）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checked_time: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddyAutoCheckinConfig {
    pub enabled: bool,
    pub start_time: String,
    pub end_time: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checked_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_schedules: Option<HashMap<String, BuddyAccountScheduleState>>,
}

impl Default for BuddyAutoCheckinConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            start_time: "06:00".to_string(),
            end_time: "12:00".to_string(),
            last_checked_date: None,
            account_schedules: None,
        }
    }
}

/// 单个账号的「今日签到任务」视图（前端任务列表的一行）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddyCheckinTask {
    pub account_id: String,
    pub email: String,
    /// "pending"（待签到）| "success"（已签到）| "failed"（失败，会继续重试）
    pub status: String,
    /// 今日计划时间（HH:MM）；今日计划尚未生成时为 None
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheduled_time: Option<String>,
    /// 最近一次尝试时间（HH:MM:SS）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_attempt_time: Option<String>,
    /// 实际签到时刻（HH:MM:SS，当日已签到时存在）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checkin_time: Option<String>,
    /// 实际时间来源（`kira`）；旧记录可能为空
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// 自动签到任务列表视图（含「今日计划是否已生成」）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddyCheckinTasksView {
    pub enabled: bool,
    pub start_time: String,
    pub end_time: String,
    /// 今日计划是否已生成（到达开始时间后才生成）
    pub generated: bool,
    pub tasks: Vec<BuddyCheckinTask>,
}

/// 把分钟数格式化为 HH:MM。
pub fn format_minutes(minute: i32) -> String {
    let clamped = minute.clamp(0, 1439);
    format!("{:02}:{:02}", clamped / 60, clamped % 60)
}

fn get_config_file_path() -> PathBuf {
    crate::commands::config::get_data_dir()
        .join("buddy")
        .join("auto_checkin_config.json")
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
        .map_err(|_| "Buddy 自动签到存储锁已损坏".to_string())
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

fn validate_config(config: &BuddyAutoCheckinConfig) -> Result<(), String> {
    let start = validate_time(&config.start_time)
        .ok_or_else(|| format!("自动签到开始时间无效: {}", config.start_time))?;
    let end = validate_time(&config.end_time)
        .ok_or_else(|| format!("自动签到结束时间无效: {}", config.end_time))?;
    if start > end {
        return Err("自动签到开始时间不能晚于结束时间".to_string());
    }
    if let Some(schedules) = &config.account_schedules {
        for (account_id, schedule) in schedules {
            if !(0..=1439).contains(&schedule.scheduled_minute) {
                return Err(format!(
                    "账号 {} 的自动签到分钟无效: {}",
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

fn read_config_from_path(path: &Path) -> Result<Option<BuddyAutoCheckinConfig>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(path).map_err(|e| format!("读取自动签到配置失败: {}", e))?;
    let config: BuddyAutoCheckinConfig = serde_json::from_str(&content)
        .map_err(|e| format!("解析自动签到配置失败: {}", e))?;
    validate_config(&config)?;
    Ok(Some(config))
}

fn write_config_to_path(path: &Path, config: &BuddyAutoCheckinConfig) -> Result<(), String> {
    validate_config(config)?;
    let content = serde_json::to_string_pretty(config)
        .map_err(|e| format!("序列化自动签到配置失败: {}", e))?;
    write_atomic(path, &content)
}

pub fn get_config_checked() -> Result<BuddyAutoCheckinConfig, String> {
    let _guard = lock_storage()?;
    Ok(read_config_from_path(&get_config_file_path())?.unwrap_or_default())
}

pub fn save_config(config: &BuddyAutoCheckinConfig) -> Result<(), String> {
    let result = save_config_without_wake(config);
    if result.is_ok() {
        wake_scheduler();
    }
    result
}

fn save_config_without_wake(config: &BuddyAutoCheckinConfig) -> Result<(), String> {
    let _guard = lock_storage()?;
    write_config_to_path(&get_config_file_path(), config)
}

pub fn parse_time_to_minutes(time_str: &str) -> i32 {
    let parts: Vec<&str> = time_str.split(':').collect();
    let h = parts.first().and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
    let m = parts.get(1).and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
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
fn random_minute_in(from: i32, to: i32) -> i32 {
    let span = (to - from).max(0) as u32;
    from + random_u32_below(span + 1) as i32
}

/// 展示用邮箱（缺失时退回账号 id）。
fn account_email(account: &BuddyAccount) -> String {
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
/// - 已经尝试过但失败的账号同样重排，不再按退避间隔原地重试。
///
/// 返回 `(计划是否有改动, 新增的行为日志)`。
fn reschedule_missed_plans(
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

/// 把本轮「尝试过但没成功」的账号的计划挪到窗口内的新随机时间（失败的重试时机）。
///
/// 为什么不能只靠 `reschedule_missed_plans`：那条路要等「错过超过宽限」才动手，
/// 而计划在宽限内仍算「到点」，下一个轮询会立刻把同一个失败账号再打一遍。
///
/// 窗口已全部过去时不改（今天放弃；失败状态保留在归档与行为日志里）。
/// 返回是否有账号被挪动。
fn reschedule_failed_plans(
    schedules: &mut HashMap<String, BuddyAccountScheduleState>,
    attempted: &[String],
    today: &str,
    current_minute: i32,
    start_min: i32,
    end_min: i32,
) -> bool {
    let Some((from, to)) = remaining_window(start_min, end_min, current_minute) else {
        return false;
    };
    let mut changed = false;
    for account_id in attempted {
        let Some(state) = schedules.get_mut(account_id) else {
            continue;
        };
        // 成功签到会把 last_checked_date 置为今天：没置上的就是本轮失败的
        if state.scheduled_date != today || state.last_checked_date.as_deref() == Some(today) {
            continue;
        }
        state.scheduled_minute = draw_plan_minute(from, to, current_minute);
        changed = true;
    }
    changed
}

fn mark_schedule_checked(
    schedules: &mut HashMap<String, BuddyAccountScheduleState>,
    account_id: &str,
    today: &str,
    current_minute: i32,
) {
    let schedule = schedules
        .entry(account_id.to_string())
        .or_insert_with(|| BuddyAccountScheduleState {
            scheduled_date: today.to_string(),
            scheduled_minute: current_minute,
            last_checked_date: None,
            last_checked_time: None,
        });
    schedule.last_checked_date = Some(today.to_string());
    schedule.last_checked_time = Some(Local::now().format("%H:%M:%S").to_string());
}

/// 把一条签到结果写进每日归档（失败只打日志，绝不影响签到主流程）。
fn archive_checkin(account: &BuddyAccount, patch: CheckinPatch) {
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
fn archive_schedule_changes(
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

pub async fn run_auto_checkin_cycle_if_needed(
    platform: BuddyPlatform,
    app: &AppHandle,
    force: bool,
) -> Result<String, String> {
    if IS_CHECKIN_RUNNING.swap(true, Ordering::SeqCst) {
        return Ok("already_running".to_string());
    }
    let _guard = CheckinGuard;

    let mut config = get_config_checked()?;
    if !config.enabled && !force {
        return Ok("disabled".to_string());
    }

    let accounts = store::list_accounts(platform);
    if accounts.is_empty() {
        return Ok("no_accounts".to_string());
    }

    // 生成今日计划的条件：用户强制 / 已到当天开始时间 / 发现过了一天（跨天重规划）。
    // 后者保证「每天打开应用后立刻重新计划」，而不必等到 startTime。
    let now = Local::now();
    let current_minute = (now.hour() * 60 + now.minute()) as i32;
    let (start_min, end_min) = plan_window(&config.start_time, &config.end_time);
    let cross_day = needs_replan(&config, &accounts);
    let allow_generate =
        force || cross_day || current_minute >= parse_time_to_minutes(&config.start_time);
    let before_schedules = config.account_schedules.clone().unwrap_or_default();
    let mut schedule_changed = ensure_account_schedules(&mut config, &accounts, allow_generate);
    // 计划时间已过但今天还没完成（应用没开着 / 上一轮失败）→ 重排到窗口内的新随机时间，
    // 而不是「打开应用就立刻补执行」。手动立即执行（force）不参与重排。
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
        let _ = app.emit("buddy-auto-checkin-config-changed", ());
    }

    let today_str = get_today_date_string();

    let target_accounts: Vec<&BuddyAccount> = if force {
        accounts.iter().collect()
    } else {
        accounts
            .iter()
            .filter(|account| {
                let sch = config
                    .account_schedules
                    .as_ref()
                    .and_then(|s| s.get(&account.id));
                match sch {
                    Some(s) => {
                        if s.last_checked_date.as_deref() == Some(&today_str) {
                            return false;
                        }
                        s.scheduled_date == today_str
                            && is_plan_due(s.scheduled_minute, current_minute, end_min)
                    }
                    None => false,
                }
            })
            .collect()
    };

    if target_accounts.is_empty() {
        // 本轮没有账号到点，但可能刚重排过计划：重排日志仍要落盘。
        if !reschedule_entries.is_empty() {
            action_log::append_action_logs(&reschedule_entries)?;
            let _ = app.emit("buddy-action-logs-changed", ());
        }
        return Ok("waiting".to_string());
    }

    eprintln!(
        "[BuddyAutoCheckin] 开始处理后台签到，平台={}, 目标账号数: {}",
        platform.as_str(),
        target_accounts.len()
    );

    let mut retry_needed = false;
    // 登录态过期标记（仅用于收尾日志；失败的重试时机已改为「重排到窗口内新随机时间」）
    let mut auth_expired = false;
    let mut entries: Vec<action_log::BuddyActionLogEntry> = reschedule_entries;
    let retry_before_schedules = config.account_schedules.clone().unwrap_or_default();
    let mut new_schedules = retry_before_schedules.clone();
    // 本轮尝试过的账号（失败者要把重试时机挪到新的随机时刻）
    let target_ids: Vec<String> = target_accounts.iter().map(|a| a.id.clone()).collect();

    for account in target_accounts {
        let email_display = if !account.email.trim().is_empty() {
            account.email.clone()
        } else {
            account.id.clone()
        };

        match api::get_checkin_status(
            &account.access_token,
            account.uid.as_deref(),
            account.enterprise_id.as_deref(),
            account.domain.as_deref(),
        )
        .await
        {
            Ok(status) if status.today_checked_in => {
                entries.push(action_log::make_entry(
                    "checkin",
                    &account.id,
                    &email_display,
                    "already_checked",
                    Some("今日已完成签到".to_string()),
                    Some(status.daily_credit),
                ));
                mark_schedule_checked(&mut new_schedules, &account.id, &today_str, current_minute);
                archive_checkin(
                    account,
                    CheckinPatch::new()
                        .actual_time(daily_history::now_time_string())
                        .status(checkin_status::ALREADY_CHECKED)
                        .source("kira")
                        .credit(Some(status.daily_credit)),
                );
            }
            Ok(status) if !status.active => {
                retry_needed = true;
                entries.push(action_log::make_entry(
                    "checkin",
                    &account.id,
                    &email_display,
                    "inactive",
                    Some("签到活动未开启或不适用".to_string()),
                    None,
                ));
                archive_checkin(
                    account,
                    CheckinPatch::new()
                        .status(checkin_status::INACTIVE)
                        .message(Some("签到活动未开启或不适用".to_string())),
                );
            }
            Ok(_) => match api::perform_checkin(
                &account.access_token,
                account.uid.as_deref(),
                account.enterprise_id.as_deref(),
                account.domain.as_deref(),
            )
            .await
            {
                Ok(res) if res.success && res.already == Some(true) => {
                    // 官方业务码 10001：今日已签到，无需再打接口
                    entries.push(action_log::make_entry(
                        "checkin",
                        &account.id,
                        &email_display,
                        "already_checked",
                        Some("今日已完成签到".to_string()),
                        None,
                    ));
                    mark_schedule_checked(&mut new_schedules, &account.id, &today_str, current_minute);
                    archive_checkin(
                        account,
                        CheckinPatch::new()
                            .actual_time(daily_history::now_time_string())
                            .status(checkin_status::ALREADY_CHECKED)
                            .source("kira"),
                    );
                }
                Ok(res) if res.inactive == Some(true) => {
                    // 官方明确回复「活动未开启/已结束」：状态记为 inactive（而非 failed）
                    retry_needed = true;
                    let message = res
                        .message
                        .clone()
                        .unwrap_or_else(|| "签到活动未开启或不适用".to_string());
                    entries.push(action_log::make_entry(
                        "checkin",
                        &account.id,
                        &email_display,
                        "inactive",
                        Some(message.clone()),
                        None,
                    ));
                    archive_checkin(
                        account,
                        CheckinPatch::new()
                            .status(checkin_status::INACTIVE)
                            .source("kira")
                            .message(Some(message)),
                    );
                }
                Ok(res) if res.success => {
                    let streak = res
                        .streak_days
                        .unwrap_or_else(|| account.checkin_streak.saturating_add(1));
                    let reward = res.reward.clone().or_else(|| {
                        res.credit.map(|credit| serde_json::json!({ "credit": credit }))
                    });
                    let _ = api::update_checkin_info(
                        platform,
                        &account.id,
                        Some(Local::now().timestamp()),
                        streak,
                        reward,
                    );

                    entries.push(action_log::make_entry(
                        "checkin",
                        &account.id,
                        &email_display,
                        "success",
                        Some("签到成功".to_string()),
                        res.credit,
                    ));

                    mark_schedule_checked(&mut new_schedules, &account.id, &today_str, current_minute);
                    archive_checkin(
                        account,
                        CheckinPatch::new()
                            .actual_time(daily_history::now_time_string())
                            .status(checkin_status::SUCCESS)
                            .source("kira")
                            .credit(res.credit)
                            .streak(Some(streak))
                            .message(Some("签到成功".to_string())),
                    );
                }
                Ok(res) => {
                    match api::get_checkin_status(
                        &account.access_token,
                        account.uid.as_deref(),
                        account.enterprise_id.as_deref(),
                        account.domain.as_deref(),
                    )
                    .await
                    {
                        Ok(latest_status) if latest_status.today_checked_in => {
                            entries.push(action_log::make_entry(
                                "checkin",
                                &account.id,
                                &email_display,
                                "already_checked",
                                Some("今日已完成签到".to_string()),
                                None,
                            ));
                            mark_schedule_checked(
                                &mut new_schedules,
                                &account.id,
                                &today_str,
                                current_minute,
                            );
                            archive_checkin(
                                account,
                                CheckinPatch::new()
                                    .actual_time(daily_history::now_time_string())
                                    .status(checkin_status::ALREADY_CHECKED)
                                    .source("kira"),
                            );
                        }
                        _ => {
                            retry_needed = true;
                            let message = res
                                .message
                                .clone()
                                .unwrap_or_else(|| "签到失败".to_string());
                            entries.push(action_log::make_entry(
                                "checkin",
                                &account.id,
                                &email_display,
                                "failed",
                                Some(message.clone()),
                                None,
                            ));
                            archive_checkin(
                                account,
                                CheckinPatch::new()
                                    .status(checkin_status::FAILED)
                                    .message(Some(message)),
                            );
                        }
                    }
                }
                Err(err) => {
                    eprintln!("[BuddyAutoCheckin] 账号 {} 自动签到异常: {}", account.id, err);
                    if err.starts_with(api::AUTH_EXPIRED_PREFIX) {
                        auth_expired = true;
                    }
                    retry_needed = true;
                    entries.push(action_log::make_entry(
                        "checkin",
                        &account.id,
                        &email_display,
                        "failed",
                        Some(err.clone()),
                        None,
                    ));
                    archive_checkin(
                        account,
                        CheckinPatch::new()
                            .status(checkin_status::FAILED)
                            .message(Some(err)),
                    );
                }
            },
            Err(err) => {
                eprintln!("[BuddyAutoCheckin] 账号 {} 签到状态检查异常: {}", account.id, err);
                retry_needed = true;
                entries.push(action_log::make_entry(
                    "checkin",
                    &account.id,
                    &email_display,
                    "failed",
                    Some(err.clone()),
                    None,
                ));
                archive_checkin(
                    account,
                    CheckinPatch::new()
                        .status(checkin_status::FAILED)
                        .message(Some(err)),
                );
            }
        }
    }

    // 失败账号的重试时机也改成「窗口内新随机时间」：否则下一个轮询（30 秒后）会以
    // 「已到点」立刻重打同一个失败账号，同一分钟内反复打官方接口。
    let retry_moved = reschedule_failed_plans(
        &mut new_schedules,
        &target_ids,
        &today_str,
        current_minute,
        start_min,
        end_min,
    );

    config.account_schedules = Some(new_schedules);
    if retry_moved {
        // 计划时间被挪动过 → 同步归档（日历与卡片都以归档为准）
        archive_schedule_changes(&retry_before_schedules, &config, &accounts);
    }
    save_config_without_wake(&config)?;

    // 行为日志（平铺）：每账号一条
    action_log::append_action_logs(&entries)?;
    let _ = app.emit("buddy-action-logs-changed", ());
    let _ = app.emit("buddy-auto-checkin-config-changed", ());

    // 失败不再让调度器拉长间隔：重试时机已改成「重排到窗口内的新随机时间」，
    // 轮询若退避到分钟以上就会错过这些随机时刻。失败信息已写进行为日志。
    if auth_expired || retry_needed {
        eprintln!(
            "[BuddyAutoCheckin] 本轮存在失败：已过计划时间的账号已重排到窗口内新随机时间（窗口已结束的今天放弃）"
        );
    }
    Ok("completed".to_string())
}

/// 由「今日是否已签到」+「今日日志中最近一条结果」推导任务状态。
fn derive_task_status(
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
        today_results.entry(entry.account_id.clone()).or_insert(entry);
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
            .map(|d| matches!(d.status.as_str(), "success" | "already_checked" | "failed" | "inactive"))
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
            last_attempt_time: detail.map(|d| d.timestamp.get(11..).unwrap_or_default().to_string()),
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

fn next_retry_delay(current: Duration) -> Duration {
    current.saturating_mul(2).min(MAX_RETRY_DELAY)
}

/// 启动后台自动签到调度（仅 WorkBuddy：签到/派出是 WorkBuddy 专属功能）。
pub fn start_auto_checkin_scheduler(app: AppHandle) {
    let wake = scheduler_wake();
    let platform = BuddyPlatform::Workbuddy;
    tauri::async_runtime::spawn(async move {
        eprintln!(
            "[BuddyAutoCheckin] 后台自动签到调度服务已启动: {}",
            platform.as_str()
        );
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
            match run_auto_checkin_cycle_if_needed(platform, &app, false).await {
                // 固定短间隔轮询：账号级失败已改成「重排到窗口内新随机时间」，
                // 轮询若退避到分钟级就会错过这些随机时刻。
                Ok(_) => {
                    next_delay = SCHEDULER_POLL_DELAY;
                    retry_delay = INITIAL_RETRY_DELAY;
                }
                Err(err) => {
                    next_delay = retry_delay;
                    retry_delay = next_retry_delay(retry_delay);
                    eprintln!("[BuddyAutoCheckin] 调度异常: {}，{} 秒后重试", err, next_delay.as_secs());
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
    fn test_parse_time_to_minutes() {
        assert_eq!(parse_time_to_minutes("06:00"), 360);
        assert_eq!(parse_time_to_minutes("12:30"), 750);
        assert_eq!(parse_time_to_minutes("00:00"), 0);
        assert_eq!(parse_time_to_minutes("23:59"), 1439);
        assert_eq!(parse_time_to_minutes("invalid"), 0);
    }

    #[test]
    fn test_ensure_account_schedules() {
        let mut config = BuddyAutoCheckinConfig {
            enabled: true,
            start_time: "06:00".to_string(),
            end_time: "12:00".to_string(),
            last_checked_date: None,
            account_schedules: None,
        };

        let accounts = vec![sample_account("acc_1", "a@x.com"), sample_account("acc_2", "b@x.com")];
        let changed = ensure_account_schedules(&mut config, &accounts, true);
        assert!(changed);

        let schedules = config.account_schedules.unwrap();
        assert_eq!(schedules.len(), 2);
        let sch1 = schedules.get("acc_1").unwrap();
        assert!(sch1.scheduled_minute >= 360 && sch1.scheduled_minute <= 720);
        assert_eq!(sch1.scheduled_date, get_today_date_string());
    }

    #[test]
    fn test_ensure_account_schedules_skips_before_start_time() {
        let mut config = BuddyAutoCheckinConfig {
            enabled: true,
            start_time: "06:00".to_string(),
            end_time: "12:00".to_string(),
            last_checked_date: None,
            account_schedules: None,
        };
        let accounts = vec![sample_account("acc_1", "a@x.com")];
        // 未到达开始时间：不生成当天计划
        assert!(!ensure_account_schedules(&mut config, &accounts, false));
        assert!(config.account_schedules.is_none());
        // 到达开始时间：生成
        assert!(ensure_account_schedules(&mut config, &accounts, true));
        assert!(config.account_schedules.unwrap().contains_key("acc_1"));
    }

    fn schedule_state(date: &str, minute: i32) -> BuddyAccountScheduleState {
        BuddyAccountScheduleState {
            scheduled_date: date.to_string(),
            scheduled_minute: minute,
            last_checked_date: None,
            last_checked_time: None,
        }
    }

    /// 窗口固定 06:00~12:00（360~720）的配置，便于用注入的 `current_minute` 做确定性测试。
    fn config_with_schedules(
        states: Vec<(&str, BuddyAccountScheduleState)>,
    ) -> BuddyAutoCheckinConfig {
        BuddyAutoCheckinConfig {
            enabled: true,
            start_time: "06:00".to_string(),
            end_time: "12:00".to_string(),
            last_checked_date: None,
            account_schedules: Some(
                states
                    .into_iter()
                    .map(|(id, state)| (id.to_string(), state))
                    .collect(),
            ),
        }
    }

    #[test]
    fn test_remaining_window_and_plan_due() {
        // 窗口 06:00~12:00，现在 10:00 → 剩余可用区间从「现在」之后开始
        assert_eq!(remaining_window(360, 720, 600), Some((601, 720)));
        // 还没到开始时间 → 从开始时间起
        assert_eq!(remaining_window(360, 720, 60), Some((360, 720)));
        // 已到 / 已过结束时间 → 今天没有剩余窗口（今天就放弃）
        assert_eq!(remaining_window(360, 720, 720), None);
        assert_eq!(remaining_window(360, 720, 900), None);

        assert!(is_plan_due(600, 600, 720)); // 到点当刻
        assert!(is_plan_due(600, 605, 720)); // 宽限内的小延迟照旧执行
        assert!(!is_plan_due(600, 606, 720)); // 错过超过宽限 → 交给重排，不立刻补执行
        assert!(!is_plan_due(600, 601, 600)); // 窗口已过 → 今天放弃
        assert!(!is_plan_due(700, 600, 720)); // 还没到点
    }

    #[test]
    fn test_draw_plan_minute_prefers_future_within_window() {
        // 窗口内还有剩余时间 → 一定抽在未来（晚开应用不会一生成就「立刻执行」）
        for _ in 0..64 {
            let minute = draw_plan_minute(360, 720, 600);
            assert!((601..=720).contains(&minute), "minute={}", minute);
        }
        // 窗口已全部过去 → 退回整窗随机（当天不会再执行，前端显示「未完成」）
        for _ in 0..64 {
            let minute = draw_plan_minute(360, 720, 900);
            assert!((360..=720).contains(&minute), "minute={}", minute);
        }
    }

    #[test]
    fn test_reschedule_missed_plan_moves_to_future_and_logs() {
        let accounts = vec![sample_account("acc_1", "a@x.com")];
        let today = get_today_date_string();
        // 08:00 的计划没执行，现在 10:00
        let mut config = config_with_schedules(vec![("acc_1", schedule_state(&today, 480))]);

        let (changed, entries) = reschedule_missed_plans(&mut config, &accounts, 600);
        assert!(changed);
        let schedule = config.account_schedules.unwrap().remove("acc_1").unwrap();
        assert!((601..=720).contains(&schedule.scheduled_minute));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, "rescheduled");
        let message = entries[0].message.clone().unwrap_or_default();
        assert!(message.contains("08:00"), "message={}", message);
    }

    #[test]
    fn test_reschedule_skips_window_closed_checked_and_grace() {
        let accounts = vec![sample_account("acc_1", "a@x.com")];
        let today = get_today_date_string();

        // 窗口已全部过去（现在 13:20）→ 今天放弃：计划不动、不写日志
        let mut config = config_with_schedules(vec![("acc_1", schedule_state(&today, 480))]);
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
            480
        );

        // 今天已签到 → 不重排
        let mut state = schedule_state(&today, 480);
        state.last_checked_date = Some(today.clone());
        let mut config = config_with_schedules(vec![("acc_1", state)]);
        assert!(!reschedule_missed_plans(&mut config, &accounts, 600).0);

        // 还在宽限内（08:00 的计划，现在 08:03）→ 不重排，照旧立刻执行
        let mut config = config_with_schedules(vec![("acc_1", schedule_state(&today, 480))]);
        assert!(!reschedule_missed_plans(&mut config, &accounts, 483).0);
    }

    #[test]
    fn test_reschedule_failed_plans_moves_retry_time_and_skips_checked() {
        let today = get_today_date_string();
        let mut checked = schedule_state(&today, 600);
        checked.last_checked_date = Some(today.clone());
        let mut schedules = HashMap::from([
            ("acc_failed".to_string(), schedule_state(&today, 600)),
            ("acc_done".to_string(), checked),
        ]);
        let attempted = vec!["acc_failed".to_string(), "acc_done".to_string()];

        // 窗口 06:00~12:00，现在 10:00 → 失败账号挪到窗口内未来时间，已签到的不动
        assert!(reschedule_failed_plans(
            &mut schedules, &attempted, &today, 600, 360, 720
        ));
        assert!((601..=720).contains(&schedules["acc_failed"].scheduled_minute));
        assert_eq!(schedules["acc_done"].scheduled_minute, 600);

        // 窗口已全部过去 → 不挪（今天放弃）
        let mut schedules =
            HashMap::from([("acc_failed".to_string(), schedule_state(&today, 600))]);
        assert!(!reschedule_failed_plans(
            &mut schedules, &attempted, &today, 800, 360, 720
        ));
        assert_eq!(schedules["acc_failed"].scheduled_minute, 600);
    }

    #[test]
    fn test_needs_replan_cross_day_and_new_account() {
        let accounts = vec![
            sample_account("acc_1", "a@x.com"),
            sample_account("acc_2", "b@x.com"),
        ];
        let today = get_today_date_string();

        // 完全没有计划（首次运行）→ 需要重规划
        let mut config = BuddyAutoCheckinConfig::default();
        assert!(needs_replan(&config, &accounts));

        // 两个账号都是今天的计划 → 不需要（已生成的随机时间不抖动）
        config.account_schedules = Some(HashMap::from([
            ("acc_1".to_string(), schedule_state(&today, 400)),
            ("acc_2".to_string(), schedule_state(&today, 500)),
        ]));
        assert!(!needs_replan(&config, &accounts));

        // 昨天遗留的计划 → 需要重规划（"打开应用即重新计划"就靠这条）
        config.account_schedules = Some(HashMap::from([
            ("acc_1".to_string(), schedule_state("2020-01-01", 400)),
            ("acc_2".to_string(), schedule_state(&today, 500)),
        ]));
        assert!(needs_replan(&config, &accounts));

        // 新增账号（计划表里没有它）→ 需要重规划
        config.account_schedules = Some(HashMap::from([(
            "acc_1".to_string(),
            schedule_state(&today, 400),
        )]));
        assert!(needs_replan(&config, &accounts));

        // 没有账号 → 不需要（避免空转）
        assert!(!needs_replan(&config, &[]));
    }

    #[test]
    fn test_cross_day_replan_resets_daily_state() {
        let accounts = vec![sample_account("acc_1", "a@x.com")];
        let today = get_today_date_string();
        let mut config = BuddyAutoCheckinConfig {
            enabled: true,
            start_time: "06:00".to_string(),
            end_time: "12:00".to_string(),
            last_checked_date: None,
            account_schedules: Some(HashMap::from([(
                "acc_1".to_string(),
                BuddyAccountScheduleState {
                    scheduled_date: "2020-01-01".to_string(),
                    scheduled_minute: 400,
                    last_checked_date: Some("2020-01-01".to_string()),
                    last_checked_time: Some("06:40:00".to_string()),
                },
            )])),
        };

        // 跨天重规划：计划日期变成今天，昨天的签到状态不会带到今天（即"每天清空"）
        assert!(ensure_account_schedules(&mut config, &accounts, true));
        let schedule = config
            .account_schedules
            .unwrap()
            .remove("acc_1")
            .expect("schedule");
        assert_eq!(schedule.scheduled_date, today);
        assert_eq!(schedule.last_checked_date, None);
        assert_eq!(schedule.last_checked_time, None);
        assert!(schedule.scheduled_minute >= 360 && schedule.scheduled_minute <= 720);
    }

    #[test]
    fn test_format_minutes() {
        assert_eq!(format_minutes(0), "00:00");
        assert_eq!(format_minutes(390), "06:30");
        assert_eq!(format_minutes(1439), "23:59");
        assert_eq!(format_minutes(2000), "23:59");
    }

    fn detail_with_status(status: &str) -> action_log::BuddyActionLogEntry {
        action_log::BuddyActionLogEntry {
            id: "log_test".to_string(),
            timestamp: "2026-09-13 08:00:00".to_string(),
            date: "2026-09-13".to_string(),
            kind: "checkin".to_string(),
            account_id: "acc_1".to_string(),
            email: "a@x.com".to_string(),
            status: status.to_string(),
            message: None,
            credit: None,
        }
    }

    #[test]
    fn test_derive_task_status() {
        // 无记录 → 待签到
        assert_eq!(derive_task_status(false, None), "pending");
        // 计划已标记完成 → 已签到
        assert_eq!(derive_task_status(true, None), "success");
        // 今日日志显示成功/已签到 → 已签到
        assert_eq!(
            derive_task_status(false, Some(&detail_with_status("success"))),
            "success"
        );
        assert_eq!(
            derive_task_status(false, Some(&detail_with_status("already_checked"))),
            "success"
        );
        // 今日日志显示失败/未开启 → 失败（会继续重试）
        assert_eq!(
            derive_task_status(false, Some(&detail_with_status("failed"))),
            "failed"
        );
        assert_eq!(
            derive_task_status(false, Some(&detail_with_status("inactive"))),
            "failed"
        );
        // 已签到优先于历史失败记录
        assert_eq!(
            derive_task_status(true, Some(&detail_with_status("failed"))),
            "success"
        );
    }

    #[test]
    fn test_config_serde_roundtrip() {
        let config = BuddyAutoCheckinConfig {
            enabled: true,
            start_time: "08:00".to_string(),
            end_time: "10:00".to_string(),
            last_checked_date: Some("2026-09-01".to_string()),
            account_schedules: None,
        };
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"enabled\":true"));
        assert!(json.contains("\"startTime\":\"08:00\""));
        let deserialized: BuddyAutoCheckinConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, config);
    }

    #[test]
    fn test_validation_rejects_invalid_time_and_schedule() {
        let mut config = BuddyAutoCheckinConfig {
            enabled: true,
            start_time: "12:00".to_string(),
            end_time: "06:00".to_string(),
            last_checked_date: None,
            account_schedules: None,
        };
        assert!(validate_config(&config).is_err());

        config.start_time = "06:00".to_string();
        config.account_schedules = Some(HashMap::from([(
            "acc_1".to_string(),
            BuddyAccountScheduleState {
                scheduled_date: "2026-09-01".to_string(),
                scheduled_minute: 1440,
                last_checked_date: None,
                last_checked_time: None,
            },
        )]));
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn test_retry_delay_bounded_exponential_backoff() {
        assert_eq!(
            next_retry_delay(INITIAL_RETRY_DELAY),
            Duration::from_secs(10 * 60)
        );
        assert_eq!(next_retry_delay(Duration::from_secs(45 * 60)), MAX_RETRY_DELAY);
        assert_eq!(next_retry_delay(MAX_RETRY_DELAY), MAX_RETRY_DELAY);
    }
}