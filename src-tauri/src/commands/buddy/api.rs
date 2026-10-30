//! Buddy 共享官方 API 通道（复刻自 cockpit-tools `codebuddy_cn_oauth.rs` / `workbuddy_oauth.rs`）。
//!
//! 两个平台共用同一套协议：
//! - 账号资料：`GET /v2/plugin/login/account?state=...`、`GET /v2/plugin/accounts`
//! - Token 刷新：`POST /v2/plugin/auth/token/refresh`
//! - 用量：`POST /v2/billing/meter/get-dosage-notify`、`get-payment-type`、
//!   `get-user-resource`（企业版走 `get-enterprise-user-usage`）
//! - 签到：`POST /v2/billing/meter/checkin-activity-status` / `daily-checkin`
//! - OAuth：`POST /v2/plugin/auth/state?platform=...` → 轮询 `GET /v2/plugin/auth/token?state=...`
//!
//! 仅端点域名不同：WorkBuddy = https://copilot.tencent.com，CodeBuddy CN = https://www.codebuddy.cn。
//! 签到接口统一走 CodeBuddy CN 域名（与参考实现一致）。

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use chrono::TimeZone;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::models::{BuddyAccount, BuddyPlatform};
use super::store;

const WORKBUDDY_API_ENDPOINT: &str = "https://copilot.tencent.com";
const CODEBUDDY_CN_API_ENDPOINT: &str = "https://www.codebuddy.cn";
const API_PREFIX: &str = "/v2/plugin";
const PLATFORM_CODEBUDDY_CN: &str = "ide";
const PLATFORM_WORKBUDDY: &str = "workbuddy";
const HTTP_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const OAUTH_TIMEOUT_SECONDS: u64 = 600;
const OAUTH_POLL_INTERVAL_MS: u64 = 1500;
pub const ENTERPRISE_PACKAGE_CODE: &str = "TCACA_code_enterprise";

// ─── OAuth 待处理状态（按平台隔离） ───

#[derive(Clone)]
struct PendingOAuthState {
    login_id: String,
    expires_at: i64,
    state: String,
    cancelled: bool,
}

fn pending_states() -> &'static Mutex<HashMap<String, PendingOAuthState>> {
    static STATES: OnceLock<Mutex<HashMap<String, PendingOAuthState>>> = OnceLock::new();
    STATES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn now_timestamp() -> i64 {
    chrono::Utc::now().timestamp()
}

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    let _ = getrandom::getrandom(&mut buf);
    buf.iter().map(|b| format!("{:02x}", b)).collect()
}

fn generate_login_id() -> String {
    format!("cb_{}", random_hex(16))
}

// ─── HTTP 客户端 ───

fn build_client() -> Result<reqwest::Client, String> {
    build_client_with_timeout(30)
}

fn build_client_with_timeout(secs: u64) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(HTTP_USER_AGENT)
        .timeout(std::time::Duration::from_secs(secs))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))
}

/// 平台 → 官方 API 端点
pub fn api_endpoint(platform: BuddyPlatform) -> &'static str {
    match platform {
        BuddyPlatform::Workbuddy => WORKBUDDY_API_ENDPOINT,
        BuddyPlatform::CodebuddyCn => CODEBUDDY_CN_API_ENDPOINT,
    }
}

/// 客户端实际使用的认证域（它拿 token 去打哪个域的 API）。
///
/// 抄 WorkDaddy `profiles.js` / `checkin-result.js` 的"按 issuer 路由"思路：
/// 官方把国内版认证域从 `www.codebuddy.cn` 迁到了 `www.workbuddy.cn`，
/// **两个域签发的 token 不通用** —— 客户端只会打自己那个域，域不对就是全量 401
/// （真机表现：账号显示正确、但"积分获取失败"、发消息报"登录状态已失效"）。
/// WorkBuddy 的值来自 5.8.3 客户端日志里的 `current endpoint=https://www.workbuddy.cn`。
pub fn client_auth_origin(platform: BuddyPlatform) -> &'static str {
    match platform {
        BuddyPlatform::Workbuddy => "https://www.workbuddy.cn",
        BuddyPlatform::CodebuddyCn => "https://www.codebuddy.cn",
    }
}

/// 同一个域的**纯 host 形式**（不带 scheme）。
///
/// 客户端登录文件里的 `domain` 字段历来是纯 host（例如 `example.com` /
/// `www.codebuddy.cn`），客户端拿它拼 endpoint；写进 `https://...` 会被拼成
/// `https://https://...`，反而制造登录失败。写文件时用这个，比较 issuer 时用
/// [`client_auth_origin`]（带 scheme）。
pub fn client_auth_host(platform: BuddyPlatform) -> &'static str {
    match platform {
        BuddyPlatform::Workbuddy => "www.workbuddy.cn",
        BuddyPlatform::CodebuddyCn => "www.codebuddy.cn",
    }
}

/// 从 access token 解出签发方 origin（抄 WorkDaddy `tokenIssuerOrigin`）。
///
/// 取 JWT payload 的 `iss`，只保留 `scheme://host` 并转小写；解不出来返回 None
/// （老 token / 非 JWT 格式），调用方据此跳过校验。
pub fn token_issuer_origin(access_token: &str) -> Option<String> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine as _};
    let payload = access_token.split('.').nth(1)?;
    let bytes = B64.decode(payload).ok()?;
    let json: Value = serde_json::from_slice(&bytes).ok()?;
    let iss = json.get("iss")?.as_str()?;
    let (scheme, rest) = iss.split_once("://")?;
    let host = rest.split('/').next().unwrap_or_default();
    if scheme.is_empty() || host.is_empty() {
        return None;
    }
    Some(format!("{}://{}", scheme, host).to_lowercase())
}

/// 切号前的认证域体检：账号 token 的签发域与客户端使用的域不一致时给出提示。
///
/// 不阻断切换（我们的签到/用量走自己的端点，老域账号仍能刷新），
/// 但要**明确说出来** —— 否则用户看到的是"切成功了、客户端却报登录失效"这种哑故障。
pub fn auth_origin_warning(platform: BuddyPlatform, access_token: &str) -> Option<String> {
    let token_origin = token_issuer_origin(access_token)?;
    let client_origin = client_auth_origin(platform);
    if token_origin == client_origin {
        return None;
    }
    Some(format!(
        "该账号的令牌由 {} 签发，而当前客户端使用 {}，两者不通用：客户端会全量 401\
         （积分获取失败 / 提示登录失效）。请先在客户端里登录一次该账号，再回到这里重新导入/切换。",
        token_origin, client_origin
    ))
}

/// token 签发域（issuer）→ 官方 API host。抄 WorkDaddy `checkin-result.js::ISSUER_HOST_MAP`
/// （2026-09-04 `87d8692` 引入，与 `c28c2da` 认证域路由同源）。
///
/// 注意它**不含** `copilot.tencent.com` —— 那个是聚合网关，属于"我们额外加的通道"，不是官方域。
const ISSUER_HOST_MAP: [(&str, &str); 4] = [
    ("https://www.workbuddy.ai", "https://www.workbuddy.ai"),
    ("https://www.workbuddy.cn", "https://www.workbuddy.cn"),
    ("https://www.codebuddy.cn", "https://www.codebuddy.cn"),
    ("https://www.codebuddy.ai", "https://www.codebuddy.ai"),
];

/// WorkBuddy 侧请求的**候选 host 顺序**（抄 WorkDaddy `checkinEndpointsForToken` 的
/// "按 token 签发域路由，解不出就多域兜底"，并按我们的实际情况调整优先级）。
///
/// 与 WorkDaddy 的**有意差异**：聚合网关排第一。
/// 我们已验证 `copilot.tencent.com` 通吃两个域签发的令牌，且承载了部分官方域没有的接口；
/// 官方域作为后续候选，保证"聚合网关没同步的新端点"也能取到数据。
/// WorkDaddy 是"按 iss 只打那一个域"，因为它没有聚合网关这条通道。
///
/// 401/403 **不**换 host —— 那说明令牌域与该 host 不匹配，重试只会掩盖真实原因
/// （沿用 `TRAVEL_AUTH_EXPIRED` 语义直接返回）。
pub fn workbuddy_api_hosts(access_token: &str) -> Vec<&'static str> {
    let mut hosts: Vec<&'static str> = vec![WORKBUDDY_API_ENDPOINT];
    if let Some(origin) = token_issuer_origin(access_token) {
        if let Some((_, host)) = ISSUER_HOST_MAP.iter().find(|(iss, _)| *iss == origin) {
            if !hosts.contains(host) {
                hosts.push(host);
            }
        }
    }
    // 官方双域兜底：解不出 iss（非 JWT / 老 token）时至少还能试官方两域
    for host in ["https://www.workbuddy.cn", "https://www.codebuddy.cn"] {
        if !hosts.contains(&host) {
            hosts.push(host);
        }
    }
    hosts
}

fn oauth_platform_name(platform: BuddyPlatform) -> &'static str {
    match platform {
        BuddyPlatform::Workbuddy => PLATFORM_WORKBUDDY,
        BuddyPlatform::CodebuddyCn => PLATFORM_CODEBUDDY_CN,
    }
}

fn normalize_non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
}

// ─── 用量响应解析辅助 ───

fn build_user_resource_request_body() -> Value {
    let now = chrono::Local::now();
    let end = now + chrono::Duration::days(365 * 101);
    let format_time =
        |value: chrono::DateTime<chrono::Local>| value.format("%Y-%m-%d %H:%M:%S").to_string();
    json!({
        "PageNumber": 1,
        "PageSize": 100,
        "ProductCode": "p_tcaca",
        "Status": [0, 3],
        "PackageEndTimeRangeBegin": format_time(now),
        "PackageEndTimeRangeEnd": format_time(end)
    })
}

fn user_resource_items(body: &Value) -> Option<&Vec<Value>> {
    [
        "/data/resources",
        "/data/data/resources",
        "/data/Response/Data/Accounts",
        "/data/data/Response/Data/Accounts",
        "/Response/Data/Accounts",
    ]
    .into_iter()
    .find_map(|path| body.pointer(path).and_then(Value::as_array))
}

fn user_resource_has_payload(body: &Value) -> bool {
    user_resource_items(body).is_some_and(|items| !items.is_empty())
}

fn user_resource_has_shape(body: &Value) -> bool {
    user_resource_items(body).is_some()
}

fn enterprise_usage_data(body: &Value) -> Option<&Value> {
    body.pointer("/data/data")
        .or_else(|| body.get("data"))
        .or(Some(body))
}

fn token_expiry_at(data: &Value) -> Option<i64> {
    let parse = |value: Option<&Value>| {
        value.and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str()?.trim().parse::<i64>().ok())
        })
    };
    parse(data.get("expiresAt").or_else(|| data.get("expires_at")))
        .map(|value| {
            if value > 0 && value < 100_000_000_000 {
                value.saturating_mul(1000)
            } else {
                value
            }
        })
        .or_else(|| {
            parse(data.get("expiresIn").or_else(|| data.get("expires_in")))
                .map(|seconds| chrono::Utc::now().timestamp_millis() + seconds.saturating_mul(1000))
        })
}

fn json_f64(value: Option<&Value>) -> Option<f64> {
    value.and_then(|v| {
        v.as_f64()
            .or_else(|| v.as_str()?.trim().parse::<f64>().ok())
    })
}

fn json_scalar_string(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

// ─── Token 刷新 ───

pub async fn refresh_token(
    platform: BuddyPlatform,
    access_token: &str,
    refresh_token: &str,
    domain: Option<&str>,
) -> Result<Value, String> {
    let client = build_client()?;
    let url = format!(
        "{}{}/auth/token/refresh",
        api_endpoint(platform),
        API_PREFIX
    );

    let mut req = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("X-Refresh-Token", refresh_token)
        .header("X-Auth-Refresh-Source", "ide-main");

    if let Some(d) = domain {
        req = req.header("X-Domain", d);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("刷新 token 失败: {}", e))?;

    // 先拿状态码与原文：401 时网关可能返回 HTML（不是 JSON），
    // 以前直接 `.json()` 会退化成"解析刷新响应失败"，把"登录态被吊销"这个真因埋掉。
    let status_code = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);

    if status_code == reqwest::StatusCode::UNAUTHORIZED
        || status_code == reqwest::StatusCode::FORBIDDEN
    {
        let detail = body
            .get("msg")
            .or_else(|| body.get("message"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                if body.is_null() {
                    format!("HTTP {}", status_code.as_u16())
                } else {
                    text.chars().take(120).collect()
                }
            });
        if is_revoked_refresh_error(&detail) || body.is_null() {
            return Err(revoked_error(&detail));
        }
        return Err(format!(
            "刷新 token 失败 (http={}): {}",
            status_code.as_u16(),
            detail
        ));
    }

    let code = body.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
    if code != 0 && code != 200 {
        let msg = body
            .get("message")
            .or_else(|| body.get("msg"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        if is_revoked_refresh_error(msg) {
            return Err(revoked_error(msg));
        }
        return Err(format!("刷新 token 失败 (code={}): {}", code, msg));
    }

    body.get("data")
        .cloned()
        .ok_or_else(|| "刷新响应缺少 data 字段".to_string())
}

mod oauth;
pub use oauth::*;

mod quota;
pub use quota::*;

mod refresh;
pub use refresh::*;

mod travel;
pub use travel::*;

mod checkin;
pub use checkin::*;

#[cfg(test)]
mod tests;
