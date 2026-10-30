//! checkin responsibility.
use super::*;

// ─── 签到 ───

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CheckinStatusResponse {
    #[serde(default, alias = "todayCheckedIn")]
    pub today_checked_in: bool,
    #[serde(default = "default_checkin_active_true", alias = "Active")]
    pub active: bool,
    #[serde(default, alias = "streakDays")]
    pub streak_days: i64,
    #[serde(default, alias = "dailyCredit")]
    pub daily_credit: i64,
    #[serde(
        default,
        alias = "todayCredit",
        skip_serializing_if = "Option::is_none"
    )]
    pub today_credit: Option<i64>,
    #[serde(
        default,
        alias = "nextStreakDay",
        skip_serializing_if = "Option::is_none"
    )]
    pub next_streak_day: Option<i64>,
    #[serde(
        default,
        alias = "isStreakDay",
        skip_serializing_if = "Option::is_none"
    )]
    pub is_streak_day: Option<bool>,
    #[serde(
        default,
        alias = "checkinDates",
        skip_serializing_if = "Option::is_none"
    )]
    pub checkin_dates: Option<Vec<String>>,
    #[serde(
        default,
        alias = "streakBonusDays",
        skip_serializing_if = "Option::is_none"
    )]
    pub streak_bonus_days: Option<i64>,
    #[serde(
        default,
        alias = "streakBonusCredit",
        skip_serializing_if = "Option::is_none"
    )]
    pub streak_bonus_credit: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CheckinResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reward: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub streak_days: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_streak_day: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_checkin_in: Option<i64>,
    /// 今日已完成签到（官方业务码 10001）——此时 `success` 同样为 true
    #[serde(skip_serializing_if = "Option::is_none")]
    pub already: Option<bool>,
    /// 签到活动未开启 / 已结束：不是失败，当日重试也没有意义
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inactive: Option<bool>,
}

// ─── 签到结果分类（对齐 WorkDaddy `checkin-result.js`） ───

/// 官方「今日已签到」业务码。
pub const CHECKIN_ALREADY_CODE: i64 = 10001;

/// 活动未开启 / 已结束的提示词（官方没有稳定的 inactive 字段，只能按文案判定）。
pub(super) const CHECKIN_INACTIVE_HINTS: &[&str] = &[
    "未开启",
    "未开始",
    "未开放",
    "已过期",
    "无活动",
    "活动结束",
    "活动已结束",
    "活动关闭",
    "活动已关闭",
    "活动暂停",
    "活动已暂停",
];

/// 签到结果分类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckinOutcome {
    /// 本次签到成功
    Success,
    /// 今日已完成签到（业务码 10001）
    Already,
    /// 活动未开启 / 已结束
    Inactive,
    /// 其他失败（可重试）
    Failed,
}

/// 判定提示文案是否表示「活动未开启 / 已结束」。
pub fn is_checkin_inactive_message(message: &str) -> bool {
    let text = message.trim();
    if text.is_empty() {
        return false;
    }
    CHECKIN_INACTIVE_HINTS
        .iter()
        .any(|hint| text.contains(hint))
}

/// 分类签到结果。
///
/// 判定顺序与参考实现一致：**先判 inactive**，再判 10001——
/// 活动已结束却返回 10001 时不算「已签到」。
pub fn classify_checkin_result(http_ok: bool, code: i64, message: &str) -> CheckinOutcome {
    if is_checkin_inactive_message(message) {
        return CheckinOutcome::Inactive;
    }
    if code == CHECKIN_ALREADY_CODE {
        return CheckinOutcome::Already;
    }
    if http_ok && code == 0 {
        return CheckinOutcome::Success;
    }
    CheckinOutcome::Failed
}

/// 构造签到接口的错误信息：401/403 统一带 [`AUTH_EXPIRED_PREFIX`]，调度器据此拉长重试。
pub fn checkin_http_error(status: u16, message: &str) -> String {
    if status == 401 || status == 403 {
        return format!(
            "{}令牌已失效（HTTP {}），请在账号卡片刷新令牌后重试",
            AUTH_EXPIRED_PREFIX, status
        );
    }
    format!("请求 daily-checkin 失败 (http={}): {}", status, message)
}

pub(super) fn default_checkin_active_true() -> bool {
    true
}

pub(super) fn json_bool(value: &Value, snake: &str, camel: &str) -> Option<bool> {
    let raw = value.get(snake).or_else(|| value.get(camel))?;
    if let Some(b) = raw.as_bool() {
        return Some(b);
    }
    if let Some(n) = raw.as_i64() {
        return Some(n != 0);
    }
    if let Some(s) = raw.as_str() {
        let lower = s.trim().to_ascii_lowercase();
        if lower == "true" || lower == "1" {
            return Some(true);
        }
        if lower == "false" || lower == "0" {
            return Some(false);
        }
    }
    None
}

pub(super) fn json_i64(value: &Value, snake: &str, camel: &str) -> Option<i64> {
    value
        .get(snake)
        .or_else(|| value.get(camel))
        .and_then(|v| v.as_i64())
}

pub(super) fn parse_checkin_status_data(data: &Value) -> Result<CheckinStatusResponse, String> {
    let today_checked_in = json_bool(data, "today_checked_in", "todayCheckedIn").unwrap_or(false);
    let active = json_bool(data, "active", "Active").unwrap_or(true);

    if let Ok(mut status) = serde_json::from_value::<CheckinStatusResponse>(data.clone()) {
        if data.get("active").is_none() && data.get("Active").is_none() {
            status.active = true;
        } else if let Some(a) = json_bool(data, "active", "Active") {
            status.active = a;
        }
        if let Some(t) = json_bool(data, "today_checked_in", "todayCheckedIn") {
            status.today_checked_in = t;
        }
        return Ok(status);
    }

    let streak_days = json_i64(data, "streak_days", "streakDays").unwrap_or(0);
    let daily_credit = json_i64(data, "daily_credit", "dailyCredit").unwrap_or(0);
    let today_credit = json_i64(data, "today_credit", "todayCredit");
    let next_streak_day = json_i64(data, "next_streak_day", "nextStreakDay");
    let is_streak_day = json_bool(data, "is_streak_day", "isStreakDay");
    let streak_bonus_days = json_i64(data, "streak_bonus_days", "streakBonusDays");
    let streak_bonus_credit = json_i64(data, "streak_bonus_credit", "streakBonusCredit");
    let checkin_dates = data
        .get("checkin_dates")
        .or_else(|| data.get("checkinDates"))
        .and_then(|v| {
            v.as_array().map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(|s| s.to_string()))
                    .collect::<Vec<_>>()
            })
        });

    Ok(CheckinStatusResponse {
        today_checked_in,
        active,
        streak_days,
        daily_credit,
        today_credit,
        next_streak_day,
        is_streak_day,
        checkin_dates,
        streak_bonus_days,
        streak_bonus_credit,
    })
}

/// 签到接口统一走 CodeBuddy CN 域名（与参考实现一致）
pub(super) async fn fetch_checkin_status_from(
    path: &str,
    access_token: &str,
    uid: Option<&str>,
    enterprise_id: Option<&str>,
    domain: Option<&str>,
) -> Result<CheckinStatusResponse, String> {
    let client = build_client()?;
    let url = format!("{}{}", CODEBUDDY_CN_API_ENDPOINT, path);

    let mut req = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .json(&json!({}));

    if let Some(u) = uid {
        req = req.header("X-User-Id", u);
    }
    if let Some(eid) = enterprise_id {
        req = req.header("X-Enterprise-Id", eid);
        req = req.header("X-Tenant-Id", eid);
    }
    if let Some(d) = domain {
        req = req.header("X-Domain", d);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("请求 {} 失败: {}", path, e))?;

    let status_code = resp.status();
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 {} 响应失败: {}", path, e))?;

    if !status_code.is_success() {
        let message = body
            .get("message")
            .or_else(|| body.get("msg"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(format!(
            "请求 {} 失败 (http={}): {}",
            path,
            status_code.as_u16(),
            message
        ));
    }

    let code = body.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
    if code != 0 {
        let message = body
            .get("message")
            .or_else(|| body.get("msg"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(format!("请求 {} 失败 (code={}): {}", path, code, message));
    }

    let data = body
        .get("data")
        .ok_or_else(|| format!("{} 响应缺少 data 字段", path))?;

    parse_checkin_status_data(data).map_err(|e| format!("解析 {} data 失败: {}", path, e))
}

pub async fn get_checkin_status(
    access_token: &str,
    uid: Option<&str>,
    enterprise_id: Option<&str>,
    domain: Option<&str>,
) -> Result<CheckinStatusResponse, String> {
    match fetch_checkin_status_from(
        "/v2/billing/meter/checkin-activity-status",
        access_token,
        uid,
        enterprise_id,
        domain,
    )
    .await
    {
        Ok(status) => Ok(status),
        Err(activity_err) => {
            match fetch_checkin_status_from(
                "/v2/billing/meter/checkin-status",
                access_token,
                uid,
                enterprise_id,
                domain,
            )
            .await
            {
                Ok(status) => Ok(status),
                Err(legacy_err) => Err(format!(
                    "查询签到状态失败: activity=({}) legacy=({})",
                    activity_err, legacy_err
                )),
            }
        }
    }
}

pub async fn perform_checkin(
    access_token: &str,
    uid: Option<&str>,
    enterprise_id: Option<&str>,
    domain: Option<&str>,
) -> Result<CheckinResponse, String> {
    let client = build_client()?;
    let url = format!(
        "{}/v2/billing/meter/daily-checkin",
        CODEBUDDY_CN_API_ENDPOINT
    );

    let mut req = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .json(&json!({}));

    if let Some(u) = uid {
        req = req.header("X-User-Id", u);
    }
    if let Some(eid) = enterprise_id {
        req = req.header("X-Enterprise-Id", eid);
        req = req.header("X-Tenant-Id", eid);
    }
    if let Some(d) = domain {
        req = req.header("X-Domain", d);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("请求 daily-checkin 失败: {}", e))?;

    let status_code = resp.status();
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 daily-checkin 响应失败: {}", e))?;

    if !status_code.is_success() {
        let message = body
            .get("message")
            .or_else(|| body.get("msg"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(checkin_http_error(status_code.as_u16(), message));
    }

    let code = body.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
    let api_msg = body
        .get("message")
        .or_else(|| body.get("msg"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown error")
        .to_string();

    match classify_checkin_result(true, code, &api_msg) {
        // 官方「今日已签到」：按已签到返回，调用方与调度器都不必再回查状态
        CheckinOutcome::Already => {
            return Ok(CheckinResponse {
                success: true,
                message: Some(api_msg),
                reward: None,
                credit: None,
                streak_days: None,
                is_streak_day: None,
                next_checkin_in: None,
                already: Some(true),
                inactive: None,
            })
        }
        // 活动未开启 / 已结束：不是失败，当日重试也没有意义
        CheckinOutcome::Inactive => {
            return Ok(CheckinResponse {
                success: false,
                message: Some(api_msg),
                reward: None,
                credit: None,
                streak_days: None,
                is_streak_day: None,
                next_checkin_in: None,
                already: None,
                inactive: Some(true),
            })
        }
        CheckinOutcome::Failed => {
            return Ok(CheckinResponse {
                success: false,
                message: Some(api_msg),
                reward: None,
                credit: None,
                streak_days: None,
                is_streak_day: None,
                next_checkin_in: None,
                already: None,
                inactive: None,
            })
        }
        CheckinOutcome::Success => {}
    }

    let data = body
        .get("data")
        .ok_or_else(|| "daily-checkin 响应缺少 data 字段".to_string())?;

    let success = data
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let message = data
        .get("message")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let reward = data.get("reward").cloned();
    let credit = data
        .get("credit")
        .or_else(|| data.get("today_credit"))
        .and_then(|v| v.as_i64());
    let streak_days = data.get("streak_days").and_then(|v| v.as_i64());
    let is_streak_day = data.get("is_streak_day").and_then(|v| v.as_bool());
    let next_checkin_in = data
        .get("nextCheckinIn")
        .or_else(|| data.get("next_checkin_in"))
        .and_then(|v| v.as_i64());

    Ok(CheckinResponse {
        success,
        message: message.clone(),
        reward,
        credit,
        streak_days,
        is_streak_day,
        next_checkin_in,
        already: None,
        inactive: message
            .as_deref()
            .map(is_checkin_inactive_message)
            .filter(|flag| *flag),
    })
}

/// 更新账号签到信息并保存
pub fn update_checkin_info(
    platform: BuddyPlatform,
    account_id: &str,
    last_checkin_time: Option<i64>,
    streak: i64,
    rewards: Option<Value>,
) -> Result<BuddyAccount, String> {
    let account = store::load_account(platform, account_id)
        .ok_or_else(|| format!("账号不存在: {}", account_id))?;
    let mut updated = account;
    if last_checkin_time.is_some() {
        updated.last_checkin_time = last_checkin_time;
    }
    updated.checkin_streak = streak;
    if rewards.is_some() {
        updated.checkin_rewards = rewards;
    }
    store::upsert_account(platform, updated)
}
