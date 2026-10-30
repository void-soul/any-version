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

/// 相邻账号之间的签到间隔（秒）。
///
/// 上游对「同一秒内多个账号签到」会直接回 `500 {"code":10001,"msg":"签到失败，请稍后重试"}`
/// （参考实现的事故记录：12 个账号里 3 个落在 3 秒窗口内 → 全部 500，而当天手动逐个点全部成功）。
/// 只作用于**自动调度**；用户点「立即执行」（`force`）时不加等待，否则 N 个账号要干等 3N 秒。
pub const CHECKIN_ACCOUNT_GAP_SECS: u64 = 3;

/// 失败退避序列（秒）：第 N 次失败后等这么久再重试，用尽后固定末位。
pub const CHECKIN_RETRY_DELAYS_SECS: [u64; 3] = [5, 15, 60];

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
    /// 连续失败次数：决定下次重试的退避（5s / 15s / 60s，用尽后固定 60s）。
    /// 成功、已签到、跨天重规划都会清零。
    #[serde(default)]
    pub retry_count: u32,
    /// 下次允许重试的时刻（Unix 秒）。到点前调度跳过该账号 ——
    /// 退避的落地点在这里：`scheduled_minute` 是分钟粒度，表达不了秒级退避。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_at_epoch: Option<i64>,
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
    let config: BuddyAutoCheckinConfig =
        serde_json::from_str(&content).map_err(|e| format!("解析自动签到配置失败: {}", e))?;
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

fn next_retry_delay(current: Duration) -> Duration {
    current.saturating_mul(2).min(MAX_RETRY_DELAY)
}

/// 启动后台自动签到调度（仅 WorkBuddy：签到/派出是 WorkBuddy 专属功能）。
pub fn start_auto_checkin_scheduler(app: AppHandle) {
    let wake = scheduler_wake();
    let platform = BuddyPlatform::Workbuddy;
    crate::runtime_tasks::spawn_task("buddy", "auto-checkin", async move {
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
                    eprintln!(
                        "[BuddyAutoCheckin] 调度异常: {}，{} 秒后重试",
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
