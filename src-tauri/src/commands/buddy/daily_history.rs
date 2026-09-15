//! Buddy 每日归档（签到 + 派旅行）：把「某天某账号」的计划与实绩持久化下来。
//!
//! 为什么需要它：`auto_checkin_config.json` / `auto_travel_config.json` 里的
//! `account_schedules` 只保留**当天**的工作状态，跨天重规划时整条被覆盖，
//! 所以回答不了「上周三这个账号是几点签到的、派了几次、几点回来的」。
//! 本模块按月份归档到 `<数据目录>/buddy/daily/<YYYY-MM>.json`，
//! 供前端日历与历史查询使用（设计见
//! docs/plans/2026-09-14-buddy-checkin-calendar-design.md）。
//!
//! 设计要点：
//! - **按日期分桶** → 「每天清空」天然成立（UI 只读今天的桶，派出次数每天从 0 起）；
//! - 所有写入都是**字段级 merge**（`upsert_*`），未传入的字段保持原值，
//!   避免"记录实际时间"时把计划时间冲掉；
//! - 文件写入沿用项目风格：建目录 → 写 `.tmp` → `rename` 原子替换，配一把模块级锁；
//! - 读取失败（文件缺失/损坏）按「空」处理，不让日历把整个页面拖崩。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};

static HISTORY_LOCK: Mutex<()> = Mutex::new(());

/// 签到记录的状态取值（与前端 `buddy.calendar.*` 文案对应）。
pub mod checkin_status {
    /// 已生成今日计划，等待执行
    pub const PENDING: &str = "pending";
    /// Kira 执行签到成功
    pub const SUCCESS: &str = "success";
    /// 巡检时发现服务端显示今日已签到（时间 = 我们发现它的时刻）
    pub const ALREADY_CHECKED: &str = "already_checked";
    /// 签到失败（会继续重试）
    pub const FAILED: &str = "failed";
    /// 签到活动未开启 / 不适用
    pub const INACTIVE: &str = "inactive";
    /// 跨天收尾：当天既没成功也没失败
    pub const UNFINISHED: &str = "unfinished";
}

/// 派旅行记录的状态取值。
pub mod travel_status {
    pub const PENDING: &str = "pending";
    /// 已派出，旅行中
    pub const TRAVELING: &str = "traveling";
    /// 轮询到已归来、待领取
    pub const ARRIVED: &str = "arrived";
    /// 已领取奖励
    pub const CLAIMED: &str = "claimed";
    /// 今日已达派出上限
    pub const LIMIT_REACHED: &str = "limit_reached";
    /// 派出被业务拒绝（如"今日太累了"）
    pub const REJECTED: &str = "rejected";
    /// 网络/服务端临时错误
    pub const FAILED: &str = "failed";
    /// 跨天收尾：当天未结束（旅行中 / 已归来没领 / 一直没派）
    pub const UNFINISHED: &str = "unfinished";
    /// 视图专用：既没有计划也没有任何记录（前端显示「未安排」）
    pub const NONE: &str = "none";
}

/// 单账号某天的签到归档。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyCheckinRecord {
    /// 计划签到时间 `HH:MM`（本地随机生成）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_time: Option<String>,
    /// 实际签到时间 `HH:MM:SS`（仅 Kira 执行 / 巡检确认时写入）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual_time: Option<String>,
    /// 见 `checkin_status`
    #[serde(default)]
    pub status: String,
    /// 时间来源：`kira`（保留字段，未来若要记录外部探测结果可写 `external`）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streak: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// 单账号某天的派旅行归档。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyTravelRecord {
    /// 计划派出时间 `HH:MM`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_time: Option<String>,
    /// 实际派出时间 `HH:MM:SS`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depart_time: Option<String>,
    /// 归来时间 `HH:MM:SS`（轮询到 `arrived` 的时刻，误差 ≤ 轮询间隔）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub back_time: Option<String>,
    /// 领取奖励时间 `HH:MM:SS`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_time: Option<String>,
    /// 当日派出次数（每天从 0 开始）
    #[serde(default)]
    pub depart_count: i64,
    /// 见 `travel_status`
    #[serde(default)]
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// 单账号某天的完整归档。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyAccountRecord {
    #[serde(default)]
    pub email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkin: Option<DailyCheckinRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub travel: Option<DailyTravelRecord>,
}

/// `日期 -> 账号id -> 记录`（月份文件与查询结果都用这个形状）。
pub type DailyMap = BTreeMap<String, BTreeMap<String, DailyAccountRecord>>;

/// 签到归档的字段补丁：`None` = 不修改该字段。
#[derive(Debug, Clone, Default)]
pub struct CheckinPatch {
    pub plan_time: Option<String>,
    pub actual_time: Option<String>,
    pub status: Option<String>,
    pub source: Option<String>,
    pub credit: Option<i64>,
    pub streak: Option<i64>,
    pub message: Option<String>,
}

impl CheckinPatch {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn actual_time(mut self, value: impl Into<String>) -> Self {
        self.actual_time = Some(value.into());
        self
    }
    pub fn status(mut self, value: impl Into<String>) -> Self {
        self.status = Some(value.into());
        self
    }
    pub fn source(mut self, value: impl Into<String>) -> Self {
        self.source = Some(value.into());
        self
    }
    pub fn credit(mut self, value: Option<i64>) -> Self {
        self.credit = value;
        self
    }
    pub fn streak(mut self, value: Option<i64>) -> Self {
        self.streak = value;
        self
    }
    pub fn message(mut self, value: Option<String>) -> Self {
        self.message = value;
        self
    }
}

/// 派旅行归档的字段补丁：`None` = 不修改该字段。
#[derive(Debug, Clone, Default)]
pub struct TravelPatch {
    pub plan_time: Option<String>,
    pub depart_time: Option<String>,
    pub back_time: Option<String>,
    pub claim_time: Option<String>,
    /// 本次是否累加派出次数（每次成功派出传 `true`）
    pub add_depart_count: bool,
    pub status: Option<String>,
    pub credit: Option<i64>,
    pub message: Option<String>,
}

impl TravelPatch {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn depart_time(mut self, value: impl Into<String>) -> Self {
        self.depart_time = Some(value.into());
        self
    }
    pub fn back_time(mut self, value: impl Into<String>) -> Self {
        self.back_time = Some(value.into());
        self
    }
    pub fn claim_time(mut self, value: impl Into<String>) -> Self {
        self.claim_time = Some(value.into());
        self
    }
    pub fn add_depart(mut self) -> Self {
        self.add_depart_count = true;
        self
    }
    pub fn status(mut self, value: impl Into<String>) -> Self {
        self.status = Some(value.into());
        self
    }
    pub fn credit(mut self, value: Option<i64>) -> Self {
        self.credit = value;
        self
    }
    pub fn message(mut self, value: Option<String>) -> Self {
        self.message = value;
        self
    }
}

/// 当前本地时间 `HH:MM:SS`（归档里的"实际时间"一律用它）。
pub fn now_time_string() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

/// 当前本地日期 `YYYY-MM-DD`。
pub fn today_string() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// 归档根目录：`<数据目录>/buddy/daily`。
fn daily_dir() -> PathBuf {
    crate::commands::config::get_data_dir()
        .join("buddy")
        .join("daily")
}

/// `YYYY-MM-DD` → `YYYY-MM`。
fn month_of(date: &str) -> Option<&str> {
    if date.len() >= 7 && date.as_bytes().get(4) == Some(&b'-') {
        date.get(0..7)
    } else {
        None
    }
}

fn month_path(base: &Path, month: &str) -> PathBuf {
    base.join(format!("{}.json", month))
}

fn parse_date(value: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| format!("日期格式非法: {}", value))
}

fn lock_history() -> Result<MutexGuard<'static, ()>, String> {
    HISTORY_LOCK
        .lock()
        .map_err(|_| "Buddy 每日归档锁已损坏".to_string())
}

fn read_month_from(path: &Path) -> DailyMap {
    if !path.exists() {
        return DailyMap::new();
    }
    match fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str::<DailyMap>(&content).ok())
    {
        Some(map) => map,
        None => {
            // 归档损坏不该影响签到/派出主流程，也不覆盖原文件（下次写入前会重建）。
            eprintln!(
                "[BuddyDaily] 归档文件解析失败，按空处理: {}",
                path.display()
            );
            DailyMap::new()
        }
    }
}

fn write_month_to(path: &Path, map: &DailyMap) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建归档目录失败: {}", e))?;
    }
    let content =
        serde_json::to_string_pretty(map).map_err(|e| format!("序列化每日归档失败: {}", e))?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, content).map_err(|e| format!("写入归档临时文件失败: {}", e))?;
    fs::rename(&tmp, path).map_err(|e| format!("归档原子替换失败: {}", e))
}

/// 读出某天某账号的归档（不改），再交给 `mutate` 修改后写回。
fn with_day<F>(
    base: &Path,
    date: &str,
    account_id: &str,
    email: &str,
    mutate: F,
) -> Result<(), String>
where
    F: FnOnce(&mut DailyAccountRecord),
{
    let _guard = lock_history()?;
    let month = month_of(date).ok_or_else(|| format!("日期格式非法: {}", date))?;
    let path = month_path(base, month);
    let mut all = read_month_from(&path);
    let record = all
        .entry(date.to_string())
        .or_default()
        .entry(account_id.to_string())
        .or_default();
    if !email.trim().is_empty() {
        record.email = email.to_string();
    }
    mutate(record);
    write_month_to(&path, &all)
}

fn merge_checkin(slot: &mut DailyCheckinRecord, patch: &CheckinPatch) {
    if let Some(value) = &patch.plan_time {
        slot.plan_time = Some(value.clone());
    }
    // 已成功是当日终态：强制巡检会把同一账号再跑一轮，没有这道防线
    // success 会被降级成 already_checked / failed，实际时间也会被覆盖。
    let finalized = slot.status == checkin_status::SUCCESS;
    if let Some(value) = &patch.actual_time {
        if !finalized {
            slot.actual_time = Some(value.clone());
        }
    }
    if let Some(value) = &patch.status {
        if !finalized {
            slot.status = value.clone();
        }
    }
    if let Some(value) = &patch.source {
        slot.source = Some(value.clone());
    }
    if patch.credit.is_some() {
        slot.credit = patch.credit;
    }
    if patch.streak.is_some() {
        slot.streak = patch.streak;
    }
    if patch.message.is_some() {
        slot.message = patch.message.clone();
    }
}

fn merge_travel(slot: &mut DailyTravelRecord, patch: &TravelPatch) {
    if let Some(value) = &patch.plan_time {
        slot.plan_time = Some(value.clone());
    }
    if let Some(value) = &patch.depart_time {
        slot.depart_time = Some(value.clone());
    }
    if let Some(value) = &patch.back_time {
        slot.back_time = Some(value.clone());
    }
    if let Some(value) = &patch.claim_time {
        slot.claim_time = Some(value.clone());
    }
    if patch.add_depart_count {
        slot.depart_count += 1;
    }
    // 已领取是当日终态：重复巡检 / 重复 claim 不允许把它改回 traveling 之类
    if let Some(value) = &patch.status {
        if slot.status != travel_status::CLAIMED {
            slot.status = value.clone();
        }
    }
    if patch.credit.is_some() {
        slot.credit = patch.credit;
    }
    if patch.message.is_some() {
        slot.message = patch.message.clone();
    }
}

// ─── 对外 API ───

/// 写签到归档（字段级 merge）。
pub fn upsert_checkin(
    date: &str,
    account_id: &str,
    email: &str,
    patch: CheckinPatch,
) -> Result<(), String> {
    with_day(&daily_dir(), date, account_id, email, |record| {
        let slot = record.checkin.get_or_insert_with(DailyCheckinRecord::default);
        merge_checkin(slot, &patch);
    })
}

/// 写派旅行归档（字段级 merge）。
pub fn upsert_travel(
    date: &str,
    account_id: &str,
    email: &str,
    patch: TravelPatch,
) -> Result<(), String> {
    with_day(&daily_dir(), date, account_id, email, |record| {
        let slot = record.travel.get_or_insert_with(DailyTravelRecord::default);
        merge_travel(slot, &patch);
    })
}

/// 记录今天的计划时间（跨天重规划 / 首轮生成计划时调用）。
///
/// 状态只在**原本为空**时才置为 `pending`：同一天因修改时间窗等原因重新生成计划时，
/// 不会把已经成功的记录降级。
pub fn upsert_checkin_plan(
    date: &str,
    account_id: &str,
    email: &str,
    plan_time: &str,
) -> Result<(), String> {
    upsert_checkin_plan_at(&daily_dir(), date, account_id, email, plan_time)
}

fn upsert_checkin_plan_at(
    base: &Path,
    date: &str,
    account_id: &str,
    email: &str,
    plan_time: &str,
) -> Result<(), String> {
    with_day(base, date, account_id, email, |record| {
        let slot = record.checkin.get_or_insert_with(DailyCheckinRecord::default);
        slot.plan_time = Some(plan_time.to_string());
        if slot.status.is_empty() {
            slot.status = checkin_status::PENDING.to_string();
        }
    })
}

/// 记录今天的计划派出时间（语义同 `upsert_checkin_plan`）。
pub fn upsert_travel_plan(
    date: &str,
    account_id: &str,
    email: &str,
    plan_time: &str,
) -> Result<(), String> {
    upsert_travel_plan_at(&daily_dir(), date, account_id, email, plan_time)
}

fn upsert_travel_plan_at(
    base: &Path,
    date: &str,
    account_id: &str,
    email: &str,
    plan_time: &str,
) -> Result<(), String> {
    with_day(base, date, account_id, email, |record| {
        let slot = record.travel.get_or_insert_with(DailyTravelRecord::default);
        slot.plan_time = Some(plan_time.to_string());
        if slot.status.is_empty() {
            slot.status = travel_status::PENDING.to_string();
        }
    })
}

/// 读某天的全部账号归档（日历「点某天」用）。
pub fn get_day(date: &str) -> Result<BTreeMap<String, DailyAccountRecord>, String> {
    let month = month_of(date).ok_or_else(|| format!("日期格式非法: {}", date))?;
    let _guard = lock_history()?;
    let all = read_month_from(&month_path(&daily_dir(), month));
    Ok(all.get(date).cloned().unwrap_or_default())
}

/// 读 `[from, to]` 区间（含端点）的归档，跨月自动读多个文件（日历月视图用）。
pub fn get_range(from: &str, to: &str) -> Result<DailyMap, String> {
    let start = parse_date(from)?;
    let end = parse_date(to)?;
    if start > end {
        return Err(format!("日期区间非法: {} ~ {}", from, to));
    }
    let _guard = lock_history()?;
    let base = daily_dir();
    let mut result = DailyMap::new();
    for month in months_between(start, end) {
        for (date, accounts) in read_month_from(&month_path(&base, &month)) {
            if let Ok(parsed) = parse_date(&date) {
                if parsed >= start && parsed <= end {
                    result.insert(date, accounts);
                }
            }
        }
    }
    Ok(result)
}

/// 跨天收尾：把旧日期里没结束的记录标成 `unfinished`。
///
/// 在"发现计划日期不是今天、准备重规划"之前调用，保证旧桶不会再被改动，
/// 日历上也能明确区分「当天没做」与「做过但没完成」。
pub fn finalize_stale_day(date: &str) -> Result<(), String> {
    finalize_stale_day_at(&daily_dir(), date)
}

fn finalize_stale_day_at(base: &Path, date: &str) -> Result<(), String> {
    let month = month_of(date).ok_or_else(|| format!("日期格式非法: {}", date))?;
    let _guard = lock_history()?;
    let path = month_path(base, month);
    let mut all = read_month_from(&path);
    let Some(day) = all.get_mut(date) else {
        return Ok(());
    };
    let mut changed = false;
    for record in day.values_mut() {
        if let Some(checkin) = record.checkin.as_mut() {
            if checkin.status.is_empty() || checkin.status == checkin_status::PENDING {
                checkin.status = checkin_status::UNFINISHED.to_string();
                changed = true;
            }
        }
        if let Some(travel) = record.travel.as_mut() {
            if travel.status.is_empty()
                || travel.status == travel_status::PENDING
                || travel.status == travel_status::TRAVELING
                || travel.status == travel_status::ARRIVED
            {
                travel.status = travel_status::UNFINISHED.to_string();
                changed = true;
            }
        }
    }
    if changed {
        write_month_to(&path, &all)?;
    }
    Ok(())
}

fn months_between(start: NaiveDate, end: NaiveDate) -> Vec<String> {
    let mut months = Vec::new();
    let mut cursor = start.with_day(1).unwrap_or(start);
    let last = end.with_day(1).unwrap_or(end);
    while cursor <= last {
        months.push(cursor.format("%Y-%m").to_string());
        cursor = next_month(cursor);
    }
    months
}

fn next_month(date: NaiveDate) -> NaiveDate {
    let (year, month) = (date.year(), date.month());
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    NaiveDate::from_ymd_opt(next_year, next_month, 1).unwrap_or(date)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// 每个测试用独立的临时目录，避免相互影响、也不污染真实数据目录。
    fn temp_base(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("kira-buddy-daily-{}-{}", tag, nanos));
        let _ = fs::create_dir_all(&dir);
        dir
    }

    fn get_day_at(base: &Path, date: &str) -> BTreeMap<String, DailyAccountRecord> {
        let month = month_of(date).expect("month");
        read_month_from(&month_path(base, month))
            .get(date)
            .cloned()
            .unwrap_or_default()
    }

    #[test]
    fn test_upsert_checkin_merges_without_clobbering_plan() {
        let base = temp_base("checkin");
        let date = "2026-09-14";

        upsert_checkin_plan_at(&base, date, "acc_1", "a@x.com", "07:32").unwrap();
        with_day(&base, date, "acc_1", "a@x.com", |record| {
            let slot = record.checkin.get_or_insert_with(DailyCheckinRecord::default);
            merge_checkin(
                slot,
                &CheckinPatch::new()
                    .actual_time("07:35:12")
                    .status(checkin_status::SUCCESS)
                    .source("kira")
                    .credit(Some(5))
                    .streak(Some(3)),
            );
        })
        .unwrap();

        let day = get_day_at(&base, date);
        let record = day.get("acc_1").expect("record");
        assert_eq!(record.email, "a@x.com");
        let checkin = record.checkin.as_ref().expect("checkin");
        // 计划时间没被冲掉
        assert_eq!(checkin.plan_time.as_deref(), Some("07:32"));
        assert_eq!(checkin.actual_time.as_deref(), Some("07:35:12"));
        assert_eq!(checkin.status, checkin_status::SUCCESS);
        assert_eq!(checkin.source.as_deref(), Some("kira"));
        assert_eq!(checkin.credit, Some(5));
        assert_eq!(checkin.streak, Some(3));
    }

    #[test]
    fn test_travel_depart_count_accumulates_and_keeps_other_fields() {
        let base = temp_base("travel");
        let date = "2026-09-14";

        upsert_travel_plan_at(&base, date, "acc_1", "a@x.com", "09:10").unwrap();
        for time in ["09:11:03", "15:20:41"] {
            with_day(&base, date, "acc_1", "a@x.com", |record| {
                let slot = record.travel.get_or_insert_with(DailyTravelRecord::default);
                merge_travel(
                    slot,
                    &TravelPatch::new()
                        .depart_time(time)
                        .status(travel_status::TRAVELING)
                        .add_depart(),
                );
            })
            .unwrap();
        }
        with_day(&base, date, "acc_1", "a@x.com", |record| {
            let slot = record.travel.get_or_insert_with(DailyTravelRecord::default);
            merge_travel(
                slot,
                &TravelPatch::new()
                    .back_time("12:40:22")
                    .claim_time("12:40:25")
                    .status(travel_status::CLAIMED)
                    .credit(Some(8)),
            );
        })
        .unwrap();

        let day = get_day_at(&base, date);
        let travel = day
            .get("acc_1")
            .and_then(|r| r.travel.as_ref())
            .expect("travel");
        assert_eq!(travel.plan_time.as_deref(), Some("09:10"));
        assert_eq!(travel.depart_time.as_deref(), Some("15:20:41"));
        assert_eq!(travel.back_time.as_deref(), Some("12:40:22"));
        assert_eq!(travel.claim_time.as_deref(), Some("12:40:25"));
        assert_eq!(travel.depart_count, 2);
        assert_eq!(travel.status, travel_status::CLAIMED);
        assert_eq!(travel.credit, Some(8));
    }

    #[test]
    fn test_plan_only_sets_pending_when_status_empty() {
        let base = temp_base("plan");
        let date = "2026-09-14";

        // 首次生成计划：状态置 pending
        upsert_checkin_plan_at(&base, date, "acc_1", "a@x.com", "06:30").unwrap();
        upsert_travel_plan_at(&base, date, "acc_1", "a@x.com", "09:10").unwrap();
        let day = get_day_at(&base, date);
        assert_eq!(
            day["acc_1"].checkin.as_ref().unwrap().status,
            checkin_status::PENDING
        );
        assert_eq!(
            day["acc_1"].travel.as_ref().unwrap().status,
            travel_status::PENDING
        );

        // 签到成功后，同日再"生成计划"（例如用户改了时间窗）：计划时间更新，但状态不降级
        with_day(&base, date, "acc_1", "a@x.com", |record| {
            let slot = record.checkin.get_or_insert_with(DailyCheckinRecord::default);
            merge_checkin(
                slot,
                &CheckinPatch::new()
                    .actual_time("06:31:00")
                    .status(checkin_status::SUCCESS),
            );
        })
        .unwrap();
        upsert_checkin_plan_at(&base, date, "acc_1", "a@x.com", "08:00").unwrap();

        let checkin = get_day_at(&base, date)
            .remove("acc_1")
            .and_then(|r| r.checkin)
            .expect("checkin");
        assert_eq!(checkin.plan_time.as_deref(), Some("08:00"));
        assert_eq!(checkin.status, checkin_status::SUCCESS);
        assert_eq!(checkin.actual_time.as_deref(), Some("06:31:00"));
    }

    #[test]
    fn test_finalize_stale_day_marks_unfinished_only_incomplete() {
        let base = temp_base("finalize");
        let date = "2026-09-13";
        for (id, checkin_status_value, travel_status_value) in [
            ("acc_done", checkin_status::SUCCESS, travel_status::CLAIMED),
            ("acc_pending", checkin_status::PENDING, travel_status::PENDING),
            ("acc_traveling", checkin_status::FAILED, travel_status::TRAVELING),
        ] {
            with_day(&base, date, id, "a@x.com", |record| {
                let checkin = record.checkin.get_or_insert_with(DailyCheckinRecord::default);
                checkin.status = checkin_status_value.to_string();
                let travel = record.travel.get_or_insert_with(DailyTravelRecord::default);
                travel.status = travel_status_value.to_string();
            })
            .unwrap();
        }

        finalize_stale_day_at(&base, date).unwrap();

        let day = get_day_at(&base, date);
        assert_eq!(
            day["acc_done"].checkin.as_ref().unwrap().status,
            checkin_status::SUCCESS
        );
        assert_eq!(
            day["acc_done"].travel.as_ref().unwrap().status,
            travel_status::CLAIMED
        );
        assert_eq!(
            day["acc_pending"].checkin.as_ref().unwrap().status,
            checkin_status::UNFINISHED
        );
        assert_eq!(
            day["acc_pending"].travel.as_ref().unwrap().status,
            travel_status::UNFINISHED
        );
        assert_eq!(
            day["acc_traveling"].travel.as_ref().unwrap().status,
            travel_status::UNFINISHED
        );
        // 失败记录保持原样（它代表"尝试过但失败"，不是"没做完"）
        assert_eq!(
            day["acc_traveling"].checkin.as_ref().unwrap().status,
            checkin_status::FAILED
        );
    }

    #[test]
    fn test_read_missing_month_returns_empty() {
        let base = temp_base("missing");
        assert!(read_month_from(&month_path(&base, "1999-01")).is_empty());
    }

    #[test]
    fn test_months_between_covers_year_boundary() {
        let start = NaiveDate::from_ymd_opt(2026, 11, 15).unwrap();
        let end = NaiveDate::from_ymd_opt(2027, 2, 3).unwrap();
        assert_eq!(
            months_between(start, end),
            vec!["2026-11", "2026-12", "2027-01", "2027-02"]
        );
    }

    #[test]
    fn test_month_of_rejects_bad_input() {
        assert_eq!(month_of("2026-09-14"), Some("2026-09"));
        assert_eq!(month_of("2026/09/14"), None);
        assert_eq!(month_of("bad"), None);
    }
}
