//! travel responsibility.
use super::*;

// ─── 派 Buddy 旅行（WorkBuddy 活动，路径不带 /v2/ 前缀） ───

/// 令牌失效统一前缀：签到 / 旅行等所有调度器共用同一约定，据此识别「登录态过期」类错误。
pub const AUTH_EXPIRED_PREFIX: &str = "AUTH_EXPIRED:";

/// 登录态**已被服务端吊销**（refresh token 失效 / 网关直接 401）。
///
/// 与 [`AUTH_EXPIRED_PREFIX`] 的分工（2026-10-23 真机故障后引入）：
/// - `AUTH_EXPIRED`：这次请求没通过鉴权，刷新一次可能还有救；
/// - `AUTH_REVOKED`：连 refresh 都被判无效（`invalid_grant` / `refresh token is invalid`）
///   或网关直接返回 HTML 401 ⇒ **刷新救不回来**，只能重新登录/重新导入。
///   有了它，调度器才不会拿死令牌天天撞 401，界面也能说清"要你重新登录"而不是甩一个 401。
pub const AUTH_REVOKED_PREFIX: &str = "AUTH_REVOKED:";

/// 兼容旧名（旅行调度器历史引用）。
pub const TRAVEL_AUTH_EXPIRED_PREFIX: &str = AUTH_EXPIRED_PREFIX;

/// 账号状态：登录态已被吊销，需重新登录/重新导入。
pub const STATUS_AUTH_REVOKED: &str = "auth_revoked";

/// refresh 接口的错误文本是否属于「会话已被服务端吊销」（这种重试没有任何意义）。
///
/// 实测（2026-10-23，17/20 账号同时中招）两种形态：
/// - 聚合网关：`invalid_grant: Session doesn't have required client`
/// - 官方域：`refresh token is invalid`
pub fn is_revoked_refresh_error(message: &str) -> bool {
    let lower = message.to_lowercase();
    lower.contains("invalid_grant")
        || lower.contains("refresh token is invalid")
        || lower.contains("session doesn't have required client")
        || lower.contains("token is not active")
}

/// 构造「登录态被吊销」的错误文案（带 [`AUTH_REVOKED_PREFIX`]，调用方据此置账号状态）。
pub(super) fn revoked_error(detail: &str) -> String {
    format!(
        "{}该账号登录态已被服务端吊销（{}），刷新也无法恢复：请在对应客户端登录该账号后\
         用「本地导入」重新导入，或用「新增账号」重新授权",
        AUTH_REVOKED_PREFIX,
        detail.trim()
    )
}

/// 业务拒绝错误前缀（code != 0，如「今日太累了」等提示；调度器据此当日放弃，不再重试）。
pub const TRAVEL_REJECTED_PREFIX: &str = "TRAVEL_REJECTED:";

/// 旅行状态（GET /activity/growth/buddy/travel/status）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddyTravelStatus {
    /// arrived（已回来可领取）/ idle（空闲）/ traveling（旅行中）
    pub state: String,
    pub daily_limit_reached: bool,
    /// 当前/最近一次旅行记录 id（claim 需原样回传，可能是数字或字符串）
    pub record_id: Option<Value>,
}

/// 旅行 / 成长中心接口请求的候选域尝试结果。
///
/// `Fatal` = 换 host 没意义（401/403：令牌域与该 host 不匹配，重试只会掩盖真实原因），
/// `Retryable` = 可以换一个 host 再试（网络失败 / 404 / 5xx / 非 JSON 响应）。
pub(super) enum TravelAttempt {
    Fatal(String),
    Retryable(String),
}

/// 旅行 / 成长中心接口通用请求（WorkBuddy 侧）。
///
/// **按 token 签发域路由 + 聚合网关兜底**（抄 WorkDaddy `checkinEndpointsForToken`）：
/// 候选顺序见 `workbuddy_api_hosts`。401/403 直接失败（域不匹配的真相要暴露出来），
/// 网络/5xx/非 JSON 才换下一个 host。
///
/// 注意：成长中心有部分端点只在 `/v2/` 前缀下存在，调用方自行拼接完整 `path`。
pub(crate) async fn travel_request(
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
    access_token: &str,
    uid: Option<&str>,
    enterprise_id: Option<&str>,
    domain: Option<&str>,
) -> Result<Value, String> {
    let hosts = workbuddy_api_hosts(access_token);
    // 有副作用的请求（派出 / 领取）**不允许换域重发**：首个 host 可能已经在服务端生效，
    // 只是响应超时或返回了非 JSON（网关吞包），换域重试会重复派出 / 重复领取。
    // 这类请求只打首选域，失败就如实报错。
    let hosts: Vec<&'static str> = if method == reqwest::Method::GET {
        hosts
    } else {
        hosts.into_iter().take(1).collect()
    };
    let mut last_err: Option<String> = None;
    for (index, host) in hosts.iter().enumerate() {
        // 兜底域用更短的超时：主通道慢 + 兜底也慢会把面板卡住
        let timeout = if index == 0 { 30 } else { 8 };
        match travel_request_once(
            host,
            timeout,
            method.clone(),
            path,
            body.clone(),
            access_token,
            uid,
            enterprise_id,
            domain,
        )
        .await
        {
            Ok(value) => return Ok(value),
            Err(TravelAttempt::Fatal(msg)) => return Err(msg),
            Err(TravelAttempt::Retryable(msg)) => last_err = Some(msg),
        }
    }
    Err(last_err.unwrap_or_else(|| format!("请求 {} 失败：所有候选域均不可用", path)))
}

/// 对单个 host 发一次请求（`travel_request` 的一次尝试）。
pub(super) async fn travel_request_once(
    host: &str,
    timeout_secs: u64,
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
    access_token: &str,
    uid: Option<&str>,
    enterprise_id: Option<&str>,
    domain: Option<&str>,
) -> Result<Value, TravelAttempt> {
    let client = build_client_with_timeout(timeout_secs).map_err(|e| TravelAttempt::Fatal(e))?;
    let url = format!("{}{}", host, path);
    let mut req = if method == reqwest::Method::GET {
        client.get(&url)
    } else {
        client.post(&url)
    }
    .header("Authorization", format!("Bearer {}", access_token))
    .header("Content-Type", "application/json");
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
    if let Some(body) = body {
        req = req.json(&body);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| TravelAttempt::Retryable(format!("请求 {} 失败: {}", path, e)))?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    if status == 401 || status == 403 {
        // 令牌域与该 host 不匹配：换 host 无用，直接暴露（域不匹配的真因要看得见）
        return Err(TravelAttempt::Fatal(format!(
            "{}令牌已失效（HTTP {}，host={}），请打开 WorkBuddy 刷新登录态",
            TRAVEL_AUTH_EXPIRED_PREFIX, status, host
        )));
    }
    if status != 200 {
        return Err(TravelAttempt::Retryable(format!(
            "请求 {} 失败 (HTTP {}, host={}): {}",
            path,
            status,
            host,
            text.chars().take(200).collect::<String>()
        )));
    }
    serde_json::from_str(&text).map_err(|e| {
        TravelAttempt::Retryable(format!("解析 {} 响应失败 (host={}): {}", path, host, e))
    })
}

/// 查询旅行状态。
pub async fn travel_status(
    access_token: &str,
    uid: Option<&str>,
    enterprise_id: Option<&str>,
    domain: Option<&str>,
) -> Result<BuddyTravelStatus, String> {
    let body = travel_request(
        reqwest::Method::GET,
        "/activity/growth/buddy/travel/status",
        None,
        access_token,
        uid,
        enterprise_id,
        domain,
    )
    .await?;
    let data = body.get("data").cloned().unwrap_or_else(|| body.clone());
    let state = data
        .get("state")
        .and_then(Value::as_str)
        .or_else(|| data.get("status").and_then(Value::as_str))
        .unwrap_or_default()
        .to_string();
    let daily_limit_reached = data
        .get("daily_limit_reached")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let record_id = data
        .get("record_id")
        .cloned()
        .filter(|v| !v.is_null())
        .or_else(|| {
            data.get("current_record")
                .and_then(|r| r.get("id"))
                .cloned()
                .filter(|v| !v.is_null())
        });
    Ok(BuddyTravelStatus {
        state,
        daily_limit_reached,
        record_id,
    })
}

/// 派出旅行（body `{"location_id": N}`）。
pub async fn travel_depart(
    location_id: i64,
    access_token: &str,
    uid: Option<&str>,
    enterprise_id: Option<&str>,
    domain: Option<&str>,
) -> Result<(), String> {
    let body = travel_request(
        reqwest::Method::POST,
        "/activity/growth/buddy/travel/depart",
        Some(json!({ "location_id": location_id })),
        access_token,
        uid,
        enterprise_id,
        domain,
    )
    .await?;
    let code = body.get("code").and_then(Value::as_i64).unwrap_or(-1);
    if code != 0 {
        let message = body
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default();
        return Err(format!(
            "{}派出被拒绝 (code={}): {}",
            TRAVEL_REJECTED_PREFIX, code, message
        ));
    }
    Ok(())
}

/// 领取奖励（body `{"record_id": ...}`），返回 reward_credit。
pub async fn travel_claim(
    record_id: Value,
    access_token: &str,
    uid: Option<&str>,
    enterprise_id: Option<&str>,
    domain: Option<&str>,
) -> Result<Option<i64>, String> {
    let body = travel_request(
        reqwest::Method::POST,
        "/activity/growth/buddy/travel/claim",
        Some(json!({ "record_id": record_id })),
        access_token,
        uid,
        enterprise_id,
        domain,
    )
    .await?;
    let code = body.get("code").and_then(Value::as_i64).unwrap_or(-1);
    if code != 0 {
        let message = body
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default();
        return Err(format!(
            "{}领取被拒绝 (code={}): {}",
            TRAVEL_REJECTED_PREFIX, code, message
        ));
    }
    Ok(body
        .get("data")
        .and_then(|d| d.get("reward_credit"))
        .and_then(Value::as_i64))
}
