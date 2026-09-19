//! 成长计划 / Buddy 状态（WorkBuddy only）。
//!
//! 语义取自 WorkDaddy `scripts/growth-daily.js` 与 `growth-active.js`
//! （见 `.agents/skills/cockpit-buddy-sync/references/workdaddy.md` §19）：
//! 一次拉齐「任务进度 + 奖励领取 + Buddy（猫猫）旅行 + 连续活跃天数」，
//! 归一成一个只读视图给前端展示。
//!
//! 与参考的差异（适配说明）：
//! - 参考把结果注入 renderer 并自带缓存；我们走 Tauri command + 进程内 60 秒缓存，
//!   纯逻辑（归一、白名单、按天进度）完全一致。
//! - **平台范围：仅 WorkBuddy**（接口域名与任务码都是 WorkBuddy 特有），
//!   不参与 CodeBuddy CN 的任何路径。
//! - 本模块只做**读取与展示**；派旅行/领奖仍走既有的 `auto_travel.rs` 状态机与
//!   `api::travel_depart` / `api::travel_claim`，避免两套时序打架。

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use super::models::{BuddyAccount, BuddyPlatform};

/// 可由自动化完成的任务码白名单（参考 `AUTOMATABLE_TASK_CODES`）。
pub(crate) const AUTOMATABLE_TASK_CODES: [&str; 14] = [
    "create_canvas",
    "template_5",
    "expert_5",
    "Expert_team_use_3",
    "automation_1",
    "playbook_prompt",
    "Expert_lighthouse",
    "Buddy_App",
    "Buddy_App_QQ",
    "Hp_Appearance",
    "chat_5",
    "Model_chat_GLM5.2",
    "black_cat",
    "Library_read",
];

/// 尚未领养 Buddy 时只展示的任务码（参考 `LOCKED_BUDDY_TASK_CODES`）。
pub(crate) const LOCKED_BUDDY_TASK_CODES: [&str; 2] = ["first_buddy", "RichMeow_Chat"];

/// 展示上限（防止官方返回异常大的列表拖垮 UI）
const MAX_TASKS: usize = 50;
const MAX_BUDDIES: usize = 50;
const MAX_MANUAL_TASKS: usize = 8;

/// 视图缓存 TTL（参考 `createDailyProgressCache` 的默认 60 秒）
const CACHE_TTL: Duration = Duration::from_secs(60);
/// 连续活跃天数的成功缓存 5 分钟 / 失败缓存 30 秒（参考 `createGrowthStreakCache`）
const STREAK_TTL_READY: Duration = Duration::from_secs(300);
const STREAK_TTL_UNAVAILABLE: Duration = Duration::from_secs(30);

// ─── 视图数据结构 ───

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthTaskReward {
    pub credits: i64,
    pub energy: i64,
    pub buddy: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthTask {
    pub task_code: String,
    pub title: String,
    pub guide: String,
    pub tag: String,
    pub deadline: Option<String>,
    pub reward: GrowthTaskReward,
    pub current: i64,
    pub target: i64,
    /// claimed | completed | not_accepted | in_progress
    pub state: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthProgress {
    pub completed: usize,
    pub total: usize,
    pub ratio: f64,
    pub tasks: Vec<GrowthTask>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthRewards {
    pub claimed: usize,
    pub total: usize,
    pub pending: usize,
    pub ratio: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthBuddy {
    pub instance_id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthCat {
    /// locked | unknown | needs_selection | idle | traveling | arrived | 官方原值
    pub state: String,
    pub progress: f64,
    pub arrive_at: Option<i64>,
    pub daily_limit_reached: bool,
    pub available: bool,
    pub active_buddy: bool,
    pub buddies: Vec<GrowthBuddy>,
    pub reward_credits: i64,
    pub reward_energy: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthActions {
    pub gacha_available: bool,
    pub gacha_count: Option<i64>,
    pub gacha_energy: Option<i64>,
    pub gacha_cost: Option<i64>,
    pub lottery_available: bool,
    pub lottery_count: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthTier {
    pub key: String,
    pub days: i64,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthStreak {
    pub days: Option<i64>,
    pub progress_days: i64,
    pub next_tier: Option<String>,
    pub next_tier_remaining: Option<i64>,
    pub makeup_cards: Option<i64>,
    pub tiers: Vec<GrowthTier>,
    /// ready | unavailable
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthOverview {
    pub fetched_at: i64,
    /// 已完成任务的接取/领取状态是否已确认（false = Buddy 信息未知）
    pub buddy_known: bool,
    /// 是否已领养 Buddy
    pub unlocked: bool,
    pub growth: GrowthProgress,
    pub rewards: GrowthRewards,
    pub cat: GrowthCat,
    pub actions: GrowthActions,
    /// 白名单外、需要人工完成的任务标题
    pub manual_tasks: Vec<String>,
    pub streak: Option<GrowthStreak>,
    /// 拉取失败的可选接口（展示用，不影响主流程）
    pub warnings: Vec<String>,
}

/// 归一化输入（对应参考 `normalizeDailyProgress` 的 `options`）。
pub(crate) struct GrowthNormalizeOptions<'a> {
    /// 可选接口是否至少成功一个（否则 Buddy 状态未知，不能判成未解锁）
    pub buddy_known: bool,
    pub has_buddy: bool,
    pub active_buddy: bool,
    pub buddies: Vec<GrowthBuddy>,
    pub gacha: Option<&'a Value>,
    pub lottery: Option<&'a Value>,
    pub fetched_at: i64,
}

fn ratio(value: i64, total: i64) -> f64 {
    if total > 0 {
        (value as f64 / total as f64).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn non_negative(value: Option<&Value>) -> i64 {
    value
        .and_then(Value::as_i64)
        .filter(|number| *number >= 0)
        .unwrap_or(0)
}

fn optional_non_negative(value: Option<&Value>) -> Option<i64> {
    value.and_then(Value::as_i64).filter(|number| *number >= 0)
}

fn bounded_text(value: Option<&Value>, max_chars: usize) -> String {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(|text| text.chars().take(max_chars).collect())
        .unwrap_or_default()
}

fn is_automatable(task_code: &str) -> bool {
    AUTOMATABLE_TASK_CODES.contains(&task_code)
}

/// 把官方任务 / 旅行 / Buddy / 盲盒 / 抽奖响应归一成展示视图。
pub(crate) fn normalize_day_progress(
    tasks: &[Value],
    travel: &Value,
    options: GrowthNormalizeOptions<'_>,
) -> GrowthOverview {
    let has_versioned_codes = tasks.iter().any(|task| {
        task.get("task_code")
            .and_then(Value::as_str)
            .is_some_and(|code| !code.is_empty())
    });
    // 未领养 Buddy 且官方已按 task_code 版本返回时，只展示两个"领养前"任务
    let visible: Vec<&Value> = if options.buddy_known && !options.has_buddy && has_versioned_codes {
        tasks
            .iter()
            .filter(|task| {
                task.get("task_code")
                    .and_then(Value::as_str)
                    .is_some_and(|code| LOCKED_BUDDY_TASK_CODES.contains(&code))
            })
            .collect()
    } else {
        tasks.iter().collect()
    };

    let mut completed = 0usize;
    let mut reward_total = 0usize;
    let mut reward_claimed = 0usize;
    let mut reward_pending = 0usize;
    let mut manual_tasks: Vec<String> = Vec::new();
    let mut details: Vec<GrowthTask> = Vec::new();

    for task in &visible {
        let status = task
            .get("accept_status")
            .and_then(Value::as_str)
            .unwrap_or("not_accepted");
        let progress = task.get("progress").filter(|value| value.is_object());
        let current = progress
            .and_then(|value| value.get("current"))
            .and_then(Value::as_i64)
            .unwrap_or(0)
            .max(0);
        let target = progress
            .and_then(|value| value.get("target"))
            .and_then(Value::as_i64)
            .unwrap_or(1)
            .max(1);
        let is_complete = status == "completed" || status == "claimed" || current >= target;
        let state = if status == "claimed" {
            "claimed"
        } else if is_complete {
            "completed"
        } else if status == "not_accepted" {
            "not_accepted"
        } else {
            "in_progress"
        };
        let task_code = task
            .get("task_code")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|code| {
                !code.is_empty()
                    && code.len() <= 96
                    && code
                        .chars()
                        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-'))
            })
            .unwrap_or_default()
            .to_string();
        let title = {
            let text = bounded_text(
                task.get("title")
                    .or_else(|| task.get("task_desc"))
                    .or_else(|| task.get("task_code")),
                80,
            );
            if text.is_empty() { "未识别任务".to_string() } else { text }
        };
        let guide = {
            let text = bounded_text(task.get("description"), 360);
            if text.is_empty() {
                bounded_text(task.get("task_desc"), 360)
            } else {
                text
            }
        };
        if details.len() < MAX_TASKS {
            details.push(GrowthTask {
                task_code: task_code.clone(),
                title: title.clone(),
                guide,
                tag: bounded_text(task.get("tag"), 24),
                deadline: {
                    let text = bounded_text(task.get("valid_end"), 64);
                    if text.is_empty() { None } else { Some(text) }
                },
                reward: GrowthTaskReward {
                    credits: non_negative(task.get("reward_credit")),
                    energy: non_negative(task.get("reward_energy")),
                    buddy: task.get("reward_buddy").and_then(Value::as_bool) == Some(true),
                },
                current,
                target,
                state: state.to_string(),
            });
        }
        if is_complete {
            completed += 1;
        }
        let credits = task.get("reward_credit").and_then(Value::as_i64).unwrap_or(0);
        let energy = task.get("reward_energy").and_then(Value::as_i64).unwrap_or(0);
        if credits > 0 || energy > 0 {
            reward_total += 1;
            if status == "claimed" {
                reward_claimed += 1;
            } else if is_complete {
                reward_pending += 1;
            }
        }
        if !task_code.is_empty() && !is_automatable(&task_code) && !is_complete {
            if manual_tasks.len() < MAX_MANUAL_TASKS {
                manual_tasks.push(title);
            }
        }
    }

    // 已确认"没有 Buddy"时把旅行视为不可用（参考：`{...travel, available: buddyKnown ? hasBuddy : undefined}`）；
    // Buddy 信息未知时不能用它反推，回落到官方字段。
    let available = if options.buddy_known {
        options.has_buddy
    } else {
        travel.get("available").and_then(Value::as_bool).unwrap_or(true)
    };
    let official_state = travel
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let state = if !available {
        "locked".to_string()
    } else if !options.buddy_known {
        "unknown".to_string()
    } else if !options.active_buddy && !options.buddies.is_empty() && official_state == "idle" {
        "needs_selection".to_string()
    } else {
        official_state
    };
    let daily_limit_reached = travel.get("daily_limit_reached").and_then(Value::as_bool) == Some(true);
    let cat_progress = if daily_limit_reached || state == "traveling" {
        1.0
    } else if state == "arrived" {
        0.72
    } else {
        0.0
    };
    let arrive_at = travel
        .get("arrive_at")
        .and_then(Value::as_i64)
        .filter(|seconds| *seconds > 0)
        .map(|seconds| seconds.saturating_mul(1000));

    let gacha = options.gacha.filter(|value| value.is_object());
    let lottery = options.lottery.filter(|value| value.is_object());

    GrowthOverview {
        fetched_at: options.fetched_at,
        buddy_known: options.buddy_known,
        unlocked: options.has_buddy,
        growth: GrowthProgress {
            completed,
            total: visible.len(),
            ratio: ratio(completed as i64, visible.len() as i64),
            tasks: details,
        },
        rewards: GrowthRewards {
            claimed: reward_claimed,
            total: reward_total,
            pending: reward_pending,
            ratio: ratio(reward_claimed as i64, reward_total as i64),
        },
        cat: GrowthCat {
            state,
            progress: cat_progress,
            arrive_at,
            daily_limit_reached,
            available,
            active_buddy: options.active_buddy,
            buddies: options.buddies.into_iter().take(MAX_BUDDIES).collect(),
            reward_credits: non_negative(travel.get("reward_credit")),
            reward_energy: non_negative(travel.get("reward_energy")),
        },
        actions: GrowthActions {
            gacha_available: gacha.is_some(),
            gacha_count: gacha.and_then(|value| optional_non_negative(value.get("affordable"))),
            gacha_energy: gacha.and_then(|value| optional_non_negative(value.get("balance"))),
            gacha_cost: gacha.and_then(|value| optional_non_negative(value.get("cost_per_open"))),
            lottery_available: lottery.is_some(),
            lottery_count: lottery.and_then(|value| optional_non_negative(value.get("balance"))),
        },
        manual_tasks,
        streak: None,
        warnings: Vec::new(),
    }
}

/// 解析连续活跃天数响应（参考 `fetchGrowthStreak` 1.2.76 扩展）。
pub(crate) fn parse_streak(data: &Value) -> Result<GrowthStreak, String> {
    let streak = data.get("streak").unwrap_or(&Value::Null);
    let days = streak
        .get("days")
        .and_then(Value::as_i64)
        .filter(|value| *value >= 0)
        .ok_or_else(|| "成长活跃接口缺少有效连续天数".to_string())?;
    let makeup_cards = data
        .get("makeup_cards")
        .and_then(|value| value.get("balance"))
        .and_then(Value::as_i64)
        .filter(|value| *value >= 0);
    let redemption = data.get("redemption_status").filter(|value| value.is_object());
    let progress_days = redemption
        .and_then(|value| value.get("remaining_days"))
        .and_then(Value::as_i64)
        .filter(|value| *value >= 0)
        .unwrap_or(days);
    // tiers 缺失时按 7/14/28 补齐（参考的 defaults）
    let defaults: [(&str, i64); 3] = [("7d", 7), ("14d", 14), ("28d", 28)];
    let configured: Vec<Value> = redemption
        .and_then(|value| value.get("tiers"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let tiers = defaults
        .iter()
        .map(|(key, fallback_days)| {
            let source = configured
                .iter()
                .find(|tier| tier.get("tier").and_then(Value::as_str) == Some(*key));
            let days = source
                .and_then(|tier| tier.get("days"))
                .and_then(Value::as_i64)
                .filter(|value| *value > 0)
                .unwrap_or(*fallback_days);
            let configured_status = source
                .and_then(|tier| tier.get("status"))
                .and_then(Value::as_str);
            let tier_status_key = format!("tier_{}_status", key);
            let status = redemption
                .and_then(|value| value.get(&tier_status_key))
                .and_then(Value::as_str)
                .or(configured_status)
                .unwrap_or("locked")
                .to_string();
            GrowthTier {
                key: (*key).to_string(),
                days,
                status,
            }
        })
        .collect();
    Ok(GrowthStreak {
        days: Some(days),
        progress_days,
        next_tier: streak
            .get("next_tier")
            .and_then(Value::as_str)
            .map(str::to_string),
        next_tier_remaining: streak
            .get("next_tier_remaining")
            .and_then(Value::as_i64)
            .filter(|value| *value >= 0),
        makeup_cards,
        tiers,
        status: "ready".to_string(),
    })
}

// ─── 官方接口 ───

async fn read_json(
    path: &str,
    account: &BuddyAccount,
) -> Result<Value, String> {
    let body = super::api::travel_request(
        reqwest::Method::GET,
        path,
        None,
        &account.access_token,
        account.uid.as_deref(),
        account.enterprise_id.as_deref(),
        account.domain.as_deref(),
    )
    .await?;
    let code = body.get("code").and_then(Value::as_i64);
    if !matches!(code, Some(0) | None) {
        let message = body
            .get("message")
            .or_else(|| body.get("msg"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        return Err(format!("接口 {} 返回 code={}: {}", path, code.unwrap_or(-1), message));
    }
    Ok(body.get("data").cloned().unwrap_or(body))
}

/// 依次尝试多个路径，返回第一个成功的结果（参考的多路径兜底）。
async fn read_first_ok(paths: &[&str], account: &BuddyAccount) -> Result<Value, String> {
    let mut last_error = String::from("没有可用端点");
    for path in paths {
        match read_json(path, account).await {
            Ok(value) => return Ok(value),
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}

/// 可选接口：失败不算错，只记录告警。
async fn read_optional(
    paths: &[&str],
    account: &BuddyAccount,
    warnings: &mut Vec<String>,
) -> Option<Value> {
    match read_first_ok(paths, account).await {
        Ok(value) => Some(value),
        Err(error) => {
            warnings.push(error);
            None
        }
    }
}

/// 拉取成长计划总览（含 60 秒缓存）。
pub(crate) async fn fetch_overview(
    account: &BuddyAccount,
    force: bool,
) -> Result<GrowthOverview, String> {
    if account.platform != BuddyPlatform::Workbuddy.as_str() {
        return Err("成长计划仅支持 WorkBuddy 账号".to_string());
    }
    if account.access_token.trim().is_empty() {
        return Err("账号缺少访问令牌，请先刷新登录态".to_string());
    }
    let cache_key = account.id.clone();
    if !force {
        if let Some(cached) = cache_get(&cache_key) {
            return Ok(cached);
        }
    }

    let mut warnings = Vec::new();
    let tasks_response = read_first_ok(
        &["/v2/activity/growth/tasks", "/activity/growth/tasks"],
        account,
    )
    .await?;
    let travel = read_first_ok(
        &[
            "/activity/growth/buddy/travel/status",
            "/v2/activity/growth/buddy/travel/status",
        ],
        account,
    )
    .await
    .unwrap_or_else(|_| Value::Null);
    let buddy_info = read_optional(&["/activity/growth/buddy/info"], account, &mut warnings).await;
    let buddy_list = read_optional(&["/activity/growth/buddy/list"], account, &mut warnings).await;
    let gacha = read_optional(&["/activity/growth/buddy/quota"], account, &mut warnings).await;
    let lottery = read_optional(&["/activity/growth/lottery/chances"], account, &mut warnings).await;
    let streak = match read_json("/activity/growth/streak", account).await {
        Ok(data) => parse_streak(&data).ok(),
        Err(error) => {
            warnings.push(error);
            None
        }
    };

    let buddy_known = buddy_info.is_some() || buddy_list.is_some();
    let buddy_rows = buddy_list
        .as_ref()
        .and_then(|value| value.get("buddies"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let has_buddy = buddy_info
        .as_ref()
        .and_then(|value| value.get("buddy"))
        .is_some_and(|buddy| !buddy.is_null())
        || !buddy_rows.is_empty();
    let active_buddy = buddy_info
        .as_ref()
        .and_then(|value| value.get("buddy"))
        .is_some_and(|buddy| !buddy.is_null())
        || buddy_rows.iter().any(|buddy| {
            matches!(
                buddy.get("current_buddy"),
                Some(Value::Bool(true)) | Some(Value::Number(_))
            ) && buddy.get("current_buddy").and_then(Value::as_i64).map(|v| v != 0).unwrap_or(true)
        });
    let buddies: Vec<GrowthBuddy> = buddy_rows
        .iter()
        .filter_map(|buddy| {
            let instance_id = buddy.get("instance_id").and_then(Value::as_i64)?;
            if instance_id <= 0 {
                return None;
            }
            let name = bounded_text(buddy.get("name"), 80);
            Some(GrowthBuddy {
                instance_id,
                name: if name.is_empty() { "Buddy".to_string() } else { name },
            })
        })
        .collect();

    let tasks = tasks_response
        .get("tasks")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut overview = normalize_day_progress(
        &tasks,
        &travel,
        GrowthNormalizeOptions {
            buddy_known,
            has_buddy,
            active_buddy,
            buddies,
            gacha: gacha.as_ref(),
            lottery: lottery.as_ref(),
            fetched_at: now_millis(),
        },
    );
    overview.streak = streak;
    overview.warnings = warnings;
    cache_put(cache_key, overview.clone());
    Ok(overview)
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

// ─── 进程内缓存 ───

static CACHE: LazyLock<Mutex<HashMap<String, (Instant, GrowthOverview)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static STREAK_CACHE: LazyLock<Mutex<HashMap<String, (Instant, GrowthStreak)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn cache_get(key: &str) -> Option<GrowthOverview> {
    let guard = CACHE.lock().ok()?;
    let (at, value) = guard.get(key)?;
    if at.elapsed() < CACHE_TTL {
        Some(value.clone())
    } else {
        None
    }
}

fn cache_put(key: String, value: GrowthOverview) {
    let Ok(mut guard) = CACHE.lock() else { return };
    if guard.len() > 200 {
        guard.clear();
    }
    guard.insert(key, (Instant::now(), value));
}

pub(crate) fn cache_invalidate(account_id: &str) {
    if let Ok(mut guard) = CACHE.lock() {
        guard.remove(account_id);
    }
    if let Ok(mut guard) = STREAK_CACHE.lock() {
        guard.remove(account_id);
    }
}

fn streak_cache_get(key: &str) -> Option<GrowthStreak> {
    let guard = STREAK_CACHE.lock().ok()?;
    let (at, value) = guard.get(key)?;
    let ttl = if value.status == "ready" {
        STREAK_TTL_READY
    } else {
        STREAK_TTL_UNAVAILABLE
    };
    if at.elapsed() < ttl {
        Some(value.clone())
    } else {
        None
    }
}

fn streak_cache_put(key: String, value: GrowthStreak) {
    let Ok(mut guard) = STREAK_CACHE.lock() else { return };
    if guard.len() > 200 {
        guard.clear();
    }
    guard.insert(key, (Instant::now(), value));
}

/// 只取连续活跃天数（命中整份缓存值，不再只回 days —— 参考 1.2.76 修正点）。
pub(crate) async fn fetch_streak(
    account: &BuddyAccount,
    force: bool,
) -> Result<GrowthStreak, String> {
    if !force {
        if let Some(cached) = streak_cache_get(&account.id) {
            return Ok(cached);
        }
    }
    let data = read_json("/activity/growth/streak", account).await?;
    let streak = parse_streak(&data)?;
    streak_cache_put(account.id.clone(), streak.clone());
    Ok(streak)
}

// ─── 命令层 ───

#[tauri::command]
pub async fn buddy_growth_overview(
    platform: String,
    account_id: String,
    force: Option<bool>,
) -> Result<GrowthOverview, String> {
    let platform = super::platform_from_str(&platform)?;
    let account = super::store::load_account(platform, &account_id)
        .ok_or_else(|| format!("账号不存在: {}", account_id))?;
    fetch_overview(&account, force.unwrap_or(false)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn options(buddy_known: bool, has_buddy: bool) -> GrowthNormalizeOptions<'static> {
        GrowthNormalizeOptions {
            buddy_known,
            has_buddy,
            active_buddy: has_buddy,
            buddies: Vec::new(),
            gacha: None,
            lottery: None,
            fetched_at: 1,
        }
    }

    #[test]
    fn counts_task_states_and_rewards() {
        let tasks = vec![
            json!({"task_code": "chat_5", "accept_status": "claimed", "reward_credit": 10,
                   "progress": {"current": 5, "target": 5}}),
            json!({"task_code": "chat_5", "accept_status": "accepted", "reward_energy": 3,
                   "progress": {"current": 5, "target": 3}}),
            json!({"task_code": "chat_5", "accept_status": "not_accepted",
                   "progress": {"current": 0, "target": 1}}),
        ];
        let view = normalize_day_progress(&tasks, &json!({}), options(true, true));
        assert_eq!(view.growth.total, 3);
        assert_eq!(view.growth.completed, 2);
        assert_eq!(view.growth.tasks[0].state, "claimed");
        // 第二个任务 current>=target 兜底判成 completed（官方可能不推进 status）
        assert_eq!(view.growth.tasks[1].state, "completed");
        assert_eq!(view.growth.tasks[2].state, "not_accepted");
        assert_eq!(view.rewards.total, 2);
        assert_eq!(view.rewards.claimed, 1);
        assert_eq!(view.rewards.pending, 1);
    }

    #[test]
    fn locked_buddy_only_shows_adoption_tasks() {
        let tasks = vec![
            json!({"task_code": "first_buddy", "accept_status": "not_accepted"}),
            json!({"task_code": "chat_5", "accept_status": "not_accepted"}),
        ];
        let view = normalize_day_progress(&tasks, &json!({}), options(true, false));
        assert_eq!(view.growth.total, 1);
        assert_eq!(view.growth.tasks[0].task_code, "first_buddy");
        // 未领养 → 旅行视为不可用
        assert_eq!(view.cat.state, "locked");
        assert!(!view.cat.available);
        // Buddy 信息未知时不能判成 locked
        let unknown = normalize_day_progress(&tasks, &json!({}), options(false, false));
        assert_eq!(unknown.growth.total, 2);
        assert_eq!(unknown.cat.state, "unknown");
    }

    #[test]
    fn cat_state_and_progress_follow_travel_status() {
        let travel = json!({"state": "arrived", "arrive_at": 5, "reward_credit": 7});
        let view = normalize_day_progress(&[], &travel, options(true, true));
        assert_eq!(view.cat.state, "arrived");
        assert!((view.cat.progress - 0.72).abs() < f64::EPSILON);
        assert_eq!(view.cat.arrive_at, Some(5000));
        assert_eq!(view.cat.reward_credits, 7);

        let traveling = json!({"state": "traveling"});
        let view = normalize_day_progress(&[], &traveling, options(true, true));
        assert!((view.cat.progress - 1.0).abs() < f64::EPSILON);

        let limited = json!({"state": "idle", "daily_limit_reached": true});
        let view = normalize_day_progress(&[], &limited, options(true, true));
        assert!((view.cat.progress - 1.0).abs() < f64::EPSILON);
        assert!(view.cat.daily_limit_reached);
    }

    #[test]
    fn manual_tasks_collects_non_automatable_incomplete_only() {
        let tasks = vec![
            json!({"task_code": "unknown_code", "title": "人工任务", "accept_status": "not_accepted"}),
            json!({"task_code": "chat_5", "title": "自动任务", "accept_status": "not_accepted"}),
            json!({"task_code": "unknown_code", "title": "已完成", "accept_status": "claimed"}),
        ];
        let view = normalize_day_progress(&tasks, &json!({}), options(true, true));
        assert_eq!(view.manual_tasks, vec!["人工任务".to_string()]);
    }

    #[test]
    fn streak_parses_tiers_and_falls_back_to_defaults() {
        let data = json!({
            "streak": {"days": 9, "next_tier": "14d", "next_tier_remaining": 5},
            "makeup_cards": {"balance": 2},
            "redemption_status": {"remaining_days": 4, "tier_14d_status": "claimed"}
        });
        let streak = parse_streak(&data).unwrap();
        assert_eq!(streak.days, Some(9));
        assert_eq!(streak.progress_days, 4);
        assert_eq!(streak.next_tier.as_deref(), Some("14d"));
        assert_eq!(streak.next_tier_remaining, Some(5));
        assert_eq!(streak.makeup_cards, Some(2));
        assert_eq!(streak.tiers.len(), 3);
        assert_eq!(streak.tiers[0].key, "7d");
        assert_eq!(streak.tiers[0].days, 7);
        assert_eq!(streak.tiers[0].status, "locked");
        assert_eq!(streak.tiers[1].status, "claimed");

        assert!(parse_streak(&json!({"streak": {}})).is_err());
    }

    #[test]
    fn buddy_helpers_are_non_negative() {
        assert_eq!(non_negative(Some(&json!(-5))), 0);
        assert_eq!(non_negative(Some(&json!(5))), 5);
        assert_eq!(non_negative(None), 0);
        assert_eq!(optional_non_negative(Some(&json!(-5))), None);
    }
}
