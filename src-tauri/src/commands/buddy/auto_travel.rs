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

fn next_retry_delay(current: Duration) -> Duration {
    current.saturating_mul(2).min(MAX_RETRY_DELAY)
}

/// 启动后台自动旅行调度（仅 WorkBuddy：旅行是 WorkBuddy 专属活动）。
pub fn start_auto_travel_scheduler(app: AppHandle) {
    let wake = scheduler_wake();
    crate::runtime_tasks::spawn_task("buddy", "auto-travel", async move {
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

mod planning;
pub use planning::*;

mod cycle;
pub use cycle::*;

mod task_view;
pub use task_view::*;

#[cfg(test)]
mod tests;
