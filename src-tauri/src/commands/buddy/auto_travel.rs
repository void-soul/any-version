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
    let start_min = super::auto_checkin::parse_time_to_minutes(&config.start_time);
    let mut end_min = super::auto_checkin::parse_time_to_minutes(&config.end_time);
    if end_min < start_min {
        end_min = start_min;
    }
    let min_range = (end_min - start_min).max(0);

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

        let random_offset = if min_range > 0 {
            super::auto_checkin::random_u32_below(min_range as u32 + 1) as i32
        } else {
            0
        };

        let (depart, done, reward) = existing
            .filter(|e| e.scheduled_date == today_str)
            .map(|e| {
                (
                    e.last_depart_date.clone(),
                    e.last_done_date.clone(),
                    e.last_reward_credit,
                )
            })
            .unwrap_or((None, None, None));

        schedules.insert(
            account.id.clone(),
            BuddyAccountTravelState {
                scheduled_date: today_str.clone(),
                scheduled_minute: start_min + random_offset,
                last_depart_date: depart,
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

/// 执行一轮自动旅行状态机（每轮「查状态 → 至多一个动作」）。
///
/// - 派出：仅当到达当日计划时间（或 force）且当日未派出过；
/// - 领取：全天检测 arrived（旅行时长 1~4 小时随机，返回后下一轮领取）；
/// - 已达派出上限：当日完成；
/// - 令牌失效：返回 auth_expired，由调度器拉长重试间隔。
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
    let allow_generate =
        force || current_minute >= super::auto_checkin::parse_time_to_minutes(&config.start_time);
    if ensure_travel_schedules(&mut config, &accounts, allow_generate) {
        save_config_without_wake(&config)?;
        let _ = app.emit("buddy-auto-travel-config-changed", ());
    }

    let schedules = config.account_schedules.clone().unwrap_or_default();
    let mut new_schedules = schedules.clone();
    let mut acted = false;
    let mut auth_expired = false;
    let mut entries: Vec<action_log::BuddyActionLogEntry> = Vec::new();

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
        let due = sch.last_depart_date.as_deref() == Some(&today_str)
            || force
            || current_minute >= sch.scheduled_minute;
        if !due {
            continue;
        }

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
                    continue;
                }
                eprintln!("[BuddyAutoTravel] 账号 {} 查询旅行状态失败: {}", account.id, e);
                entries.push(action_log::make_entry(
                    "travel",
                    &account.id,
                    &email_display,
                    "failed",
                    Some(e),
                    None,
                ));
                continue;
            }
        };

        let Some(state) = new_schedules.get_mut(&account.id) else {
            continue;
        };

        if status.state == "arrived" {
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
                            Some(e),
                            None,
                        ));
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
                        state.last_depart_date = Some(today_str.clone());
                        acted = true;
                        entries.push(action_log::make_entry(
                            "travel",
                            &account.id,
                            &email_display,
                            "departed",
                            Some(format!("已派出旅行（地点 {}）", config.location_id)),
                            None,
                        ));
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
                                Some(e),
                                None,
                            ));
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
                                Some(e),
                                None,
                            ));
                        }
                    }
                }
            }
        }
        // traveling：旅行中，等下一轮领取
    }

    config.account_schedules = Some(new_schedules);
    save_config_without_wake(&config)?;
    action_log::append_action_logs(&entries)?;
    let _ = app.emit("buddy-action-logs-changed", ());
    let _ = app.emit("buddy-auto-travel-config-changed", ());

    if auth_expired {
        return Ok("auth_expired".to_string());
    }
    Ok("completed".to_string())
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
                Ok(result) if result == "auth_expired" => {
                    // 令牌失效：拉长重试间隔，用户刷新登录态后由定时或 wake 恢复
                    next_delay = MAX_RETRY_DELAY;
                    retry_delay = INITIAL_RETRY_DELAY;
                    eprintln!("[BuddyAutoTravel] 令牌失效，{} 秒后重试", next_delay.as_secs());
                }
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

    #[test]
    fn test_retry_delay_bounded_exponential_backoff() {
        assert_eq!(
            next_retry_delay(INITIAL_RETRY_DELAY),
            Duration::from_secs(10 * 60)
        );
        assert_eq!(next_retry_delay(MAX_RETRY_DELAY), MAX_RETRY_DELAY);
    }
}
