//! Buddy 自动行为统一日志（签到 + 派旅行）：平铺追加式，每条一个动作事件。
//!
//! 供前端「行为日志」平铺展示与签到任务状态推导（取每账号当日最新一条签到事件）。

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::{Local, NaiveDateTime, TimeZone};
use serde::{Deserialize, Serialize};

static LOGS_LOCK: Mutex<()> = Mutex::new(());

const THIRTY_DAYS_SECS: i64 = 30 * 24 * 60 * 60;

/// 一条行为日志。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddyActionLogEntry {
    pub id: String,
    /// "%Y-%m-%d %H:%M:%S"
    pub timestamp: String,
    /// "%Y-%m-%d"（当日过滤用）
    pub date: String,
    /// "checkin" | "travel"
    pub kind: String,
    pub account_id: String,
    pub email: String,
    /// 签到：success / already_checked / failed / inactive / no_accounts
    /// 旅行：departed / claimed / traveling / limit_reached / depart_rejected / failed
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit: Option<i64>,
}

fn logs_path() -> PathBuf {
    crate::commands::config::get_data_dir()
        .join("buddy")
        .join("action_logs.json")
}

fn read_logs_from_disk() -> Vec<BuddyActionLogEntry> {
    let path = logs_path();
    if !path.exists() {
        return Vec::new();
    }
    fs::read_to_string(&path)
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_default()
}

fn write_logs_to_disk(logs: &[BuddyActionLogEntry]) -> Result<(), String> {
    let path = logs_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {}", e))?;
    }
    let content = serde_json::to_string_pretty(logs)
        .map_err(|e| format!("序列化行为日志失败: {}", e))?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, content).map_err(|e| format!("写入临时文件失败: {}", e))?;
    fs::rename(&tmp, path).map_err(|e| format!("原子替换文件失败: {}", e))
}

/// 追加行为日志（新的排在最前），保留 30 天。
pub fn append_action_logs(entries: &[BuddyActionLogEntry]) -> Result<(), String> {
    if entries.is_empty() {
        return Ok(());
    }
    let _guard = LOGS_LOCK.lock().map_err(|_| "行为日志锁已损坏".to_string())?;
    let mut all = entries.to_vec();
    all.extend(read_logs_from_disk());
    let cutoff = Local::now().timestamp() - THIRTY_DAYS_SECS;
    all.retain(|e| {
        match NaiveDateTime::parse_from_str(&e.timestamp, "%Y-%m-%d %H:%M:%S") {
            Ok(ndt) => Local
                .from_local_datetime(&ndt)
                .single()
                .map(|dt| dt.timestamp() >= cutoff)
                .unwrap_or_else(|| ndt.and_utc().timestamp() >= cutoff),
            Err(_) => true,
        }
    });
    write_logs_to_disk(&all)
}

pub fn get_action_logs() -> Result<Vec<BuddyActionLogEntry>, String> {
    let _guard = LOGS_LOCK.lock().map_err(|_| "行为日志锁已损坏".to_string())?;
    Ok(read_logs_from_disk())
}

pub fn clear_action_logs() -> Result<(), String> {
    let _guard = LOGS_LOCK.lock().map_err(|_| "行为日志锁已损坏".to_string())?;
    write_logs_to_disk(&[])
}

/// 生成一条日志（时间戳/日期由当前时间填充，id 用纳秒时间戳保证唯一）。
#[allow(clippy::too_many_arguments)]
pub fn make_entry(
    kind: &str,
    account_id: &str,
    email: &str,
    status: &str,
    message: Option<String>,
    credit: Option<i64>,
) -> BuddyActionLogEntry {
    let now = Local::now();
    BuddyActionLogEntry {
        id: format!(
            "log_{}_{}",
            now.timestamp_millis(),
            now.timestamp_subsec_nanos()
        ),
        timestamp: now.format("%Y-%m-%d %H:%M:%S").to_string(),
        date: now.format("%Y-%m-%d").to_string(),
        kind: kind.to_string(),
        account_id: account_id.to_string(),
        email: email.to_string(),
        status: status.to_string(),
        message,
        credit,
    }
}
