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

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

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
    reqwest::Client::builder()
        .user_agent(HTTP_USER_AGENT)
        .timeout(std::time::Duration::from_secs(30))
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
    let format_time = |value: chrono::DateTime<chrono::Local>| value.format("%Y-%m-%d %H:%M:%S").to_string();
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

// ─── OAuth 登录 ───

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthStartResponse {
    pub login_id: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    pub interval_seconds: u64,
}

fn decorate_login_url(raw_url: &str, version: Option<&str>, login_session_id: &str) -> String {
    let mut url = match url::Url::parse(raw_url) {
        Ok(u) => u,
        Err(_) => return raw_url.to_string(),
    };
    {
        let mut query = url.query_pairs_mut();
        if let Some(version) = version.filter(|value| !value.trim().is_empty()) {
            query.append_pair("version", version);
        }
        query.append_pair("loginSessionId", login_session_id);
    }
    url.to_string()
}

pub async fn oauth_start(platform: BuddyPlatform) -> Result<OAuthStartResponse, String> {
    let client = build_client()?;
    let platform_name = oauth_platform_name(platform);
    let url = format!(
        "{}{}/auth/state?platform={}",
        api_endpoint(platform),
        API_PREFIX,
        platform_name
    );

    eprintln!("[Buddy OAuth] 请求 auth/state: {}", url);

    let resp = client
        .post(&url)
        .header("X-No-Authorization", "true")
        .header("X-No-User-Id", "true")
        .header("X-No-Enterprise-Id", "true")
        .header("X-No-Department-Info", "true")
        .json(&json!({}))
        .send()
        .await
        .map_err(|e| format!("请求 auth/state 失败: {}", e))?;

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 auth/state 响应失败: {}", e))?;

    let data = body.get("data").ok_or_else(|| {
        let mut keys = body
            .as_object()
            .map(|obj| obj.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        keys.sort();
        format!("auth/state 响应缺少 data 字段: body_keys={:?}", keys)
    })?;

    let state = data
        .get("state")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "auth/state 响应缺少 state".to_string())?
        .to_string();

    let auth_url = data
        .get("authUrl")
        .or_else(|| data.get("auth_url"))
        .or_else(|| data.get("url"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let login_id = generate_login_id();
    let login_session_id = random_hex(16);

    let base_verification_uri = if auth_url.is_empty() {
        format!("{}/login?state={}", api_endpoint(platform), state)
    } else {
        auth_url.clone()
    };
    let verification_uri = decorate_login_url(&base_verification_uri, None, &login_session_id);

    pending_states()
        .lock()
        .map_err(|_| "获取锁失败".to_string())?
        .insert(
            platform.as_str().to_string(),
            PendingOAuthState {
                login_id: login_id.clone(),
                expires_at: now_timestamp() + OAUTH_TIMEOUT_SECONDS as i64,
                state: state.clone(),
                cancelled: false,
            },
        );

    eprintln!("[Buddy OAuth] 登录已启动: platform={}, login_id={}", platform.as_str(), login_id);

    Ok(OAuthStartResponse {
        login_id,
        verification_uri: verification_uri.clone(),
        verification_uri_complete: Some(verification_uri),
        expires_in: OAUTH_TIMEOUT_SECONDS,
        interval_seconds: OAUTH_POLL_INTERVAL_MS / 1000 + 1,
    })
}

async fn fetch_account_info(
    client: &reqwest::Client,
    access_token: &str,
    state: &str,
    domain: Option<&str>,
    platform: BuddyPlatform,
) -> Result<
    (
        Option<String>,
        Option<String>,
        String,
        Option<String>,
        Option<String>,
        Option<Value>,
    ),
    String,
> {
    let url = format!(
        "{}{}/login/account?state={}",
        api_endpoint(platform),
        API_PREFIX,
        state
    );

    let mut req = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("X-No-User-Id", "true")
        .header("X-No-Enterprise-Id", "true")
        .header("X-No-Department-Info", "true");

    if let Some(d) = domain {
        req = req.header("X-Domain", d);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("请求 login/account 失败: {}", e))?;

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 login/account 响应失败: {}", e))?;

    let data = body.get("data").cloned().unwrap_or(json!({}));

    let uid = data.get("uid").and_then(|v| v.as_str()).map(|s| s.to_string());
    let nickname = data.get("nickname").and_then(|v| v.as_str()).map(|s| s.to_string());
    let email = data.get("email").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let enterprise_id = data
        .get("enterpriseId")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let enterprise_name = data
        .get("enterpriseName")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let email_final = if email.is_empty() {
        nickname.clone().or_else(|| uid.clone()).unwrap_or_default()
    } else {
        email
    };

    Ok((
        uid,
        nickname,
        email_final,
        enterprise_id,
        enterprise_name,
        Some(data),
    ))
}

pub fn oauth_cancel(platform: BuddyPlatform, login_id: Option<&str>) -> Result<(), String> {
    let mut pending = pending_states()
        .lock()
        .map_err(|_| "获取锁失败".to_string())?;
    if let Some(state) = pending.get_mut(platform.as_str()) {
        if login_id.is_none() || login_id == Some(state.login_id.as_str()) {
            state.cancelled = true;
            pending.remove(platform.as_str());
        }
    }
    Ok(())
}

/// 轮询官方 token 接口直到用户完成授权
pub async fn oauth_complete(
    platform: BuddyPlatform,
    login_id: &str,
) -> Result<BuddyAccount, String> {
    let client = build_client()?;
    let start = now_timestamp();

    loop {
        let state_info = {
            let pending = pending_states()
                .lock()
                .map_err(|_| "获取锁失败".to_string())?;
            match pending.get(platform.as_str()) {
                None => return Err("没有待处理的登录请求".to_string()),
                Some(s) => {
                    if s.login_id != login_id {
                        return Err("login_id 不匹配".to_string());
                    }
                    if s.cancelled {
                        return Err("登录已取消".to_string());
                    }
                    if now_timestamp() > s.expires_at {
                        return Err("登录超时".to_string());
                    }
                    s.clone()
                }
            }
        };

        let url = format!(
            "{}{}/auth/token?state={}",
            api_endpoint(platform),
            API_PREFIX,
            state_info.state
        );

        match client
            .get(&url)
            .header("X-No-Authorization", "true")
            .header("X-No-User-Id", "true")
            .header("X-No-Enterprise-Id", "true")
            .header("X-No-Department-Info", "true")
            .send()
            .await
        {
            Ok(resp) => {
                if let Ok(body) = resp.json::<Value>().await {
                    let code = body.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
                    if code == 0 || code == 200 {
                        if let Some(data) = body.get("data") {
                            let access_token = data
                                .get("accessToken")
                                .or_else(|| data.get("access_token"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            if !access_token.is_empty() {
                                eprintln!("[Buddy OAuth] 获取 token 成功: platform={}", platform.as_str());
                                let refresh_token = data
                                    .get("refreshToken")
                                    .or_else(|| data.get("refresh_token"))
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string());
                                let expires_at = token_expiry_at(data);
                                let domain = data
                                    .get("domain")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string());
                                let token_type = data
                                    .get("tokenType")
                                    .or_else(|| data.get("token_type"))
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string());
                                let auth_raw = Some(data.clone());

                                let account_info = fetch_account_info(
                                    &client,
                                    &access_token,
                                    &state_info.state,
                                    domain.as_deref(),
                                    platform,
                                )
                                .await;

                                let (uid, nickname, email, enterprise_id, enterprise_name, profile_raw) =
                                    match account_info {
                                        Ok(info) => info,
                                        Err(e) => {
                                            eprintln!("[Buddy OAuth] 获取账号信息失败: {}", e);
                                            (None, None, String::new(), None, None, None)
                                        }
                                    };

                                let account = BuddyAccount {
                                    id: String::new(), // upsert 时按 uid/email 去重
                                    platform: platform.as_str().to_string(),
                                    email,
                                    uid,
                                    nickname,
                                    enterprise_id,
                                    enterprise_name,
                                    tags: None,
                                    access_token,
                                    refresh_token,
                                    token_type,
                                    expires_at,
                                    domain,
                                    plan_type: None,
                                    dosage_notify_code: None,
                                    dosage_notify_zh: None,
                                    dosage_notify_en: None,
                                    payment_type: None,
                                    quota_raw: None,
                                    usage_raw: None,
                                    profile_raw,
                                    status: Some("normal".to_string()),
                                    status_reason: None,
                                    quota_query_last_error: None,
                                    quota_query_last_error_at: None,
                                    usage_updated_at: None,
                                    last_checkin_time: None,
                                    checkin_streak: 0,
                                    checkin_rewards: None,
                                    auth_raw,
                                    expiry_times: Default::default(),
                                    created_at: 0,
                                    last_used: 0,
                                };
                                return Ok(account);
                            }
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("[Buddy OAuth] 轮询 token 请求失败: {}", e);
            }
        }

        if now_timestamp() - start > OAUTH_TIMEOUT_SECONDS as i64 {
            pending_states()
                .lock()
                .map_err(|_| "获取锁失败".to_string())?
                .remove(platform.as_str());
            return Err("登录超时".to_string());
        }

        tokio::time::sleep(std::time::Duration::from_millis(OAUTH_POLL_INTERVAL_MS)).await;
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

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析刷新响应失败: {}", e))?;

    let code = body.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
    if code != 0 && code != 200 {
        let msg = body
            .get("message")
            .or_else(|| body.get("msg"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(format!("刷新 token 失败 (code={}): {}", code, msg));
    }

    body.get("data")
        .cloned()
        .ok_or_else(|| "刷新响应缺少 data 字段".to_string())
}

// ─── 用量查询 ───

pub async fn fetch_dosage_notify(
    platform: BuddyPlatform,
    access_token: &str,
    uid: Option<&str>,
    enterprise_id: Option<&str>,
    domain: Option<&str>,
) -> Result<Value, String> {
    let client = build_client()?;
    let url = format!("{}/v2/billing/meter/get-dosage-notify", api_endpoint(platform));

    let mut req = client
        .post(&url)
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

    let resp = req
        .send()
        .await
        .map_err(|e| format!("请求 dosage notify 失败: {}", e))?;

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 dosage 响应失败: {}", e))?;

    Ok(body)
}

pub async fn fetch_payment_type(
    platform: BuddyPlatform,
    access_token: &str,
    uid: Option<&str>,
    enterprise_id: Option<&str>,
    domain: Option<&str>,
) -> Result<Value, String> {
    let client = build_client()?;
    let url = format!("{}/v2/billing/meter/get-payment-type", api_endpoint(platform));

    let mut req = client
        .post(&url)
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

    let resp = req
        .send()
        .await
        .map_err(|e| format!("请求 payment type 失败: {}", e))?;

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 payment type 响应失败: {}", e))?;

    Ok(body)
}

async fn post_user_resource(
    platform: BuddyPlatform,
    access_token: &str,
    uid: Option<&str>,
    enterprise_id: Option<&str>,
    _domain: Option<&str>,
    body: Value,
) -> Result<Value, String> {
    let client = build_client()?;
    let url = format!("{}/v2/billing/meter/get-user-resource", api_endpoint(platform));

    let mut req = client
        .post(&url)
        .header("Accept", "application/json, text/plain, */*")
        .header("Accept-Language", "zh-CN,zh;q=0.9")
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json");

    if let Some(u) = uid {
        req = req.header("X-User-Id", u);
    }
    if let Some(eid) = enterprise_id {
        req = req.header("X-Enterprise-Id", eid);
        req = req.header("X-Tenant-Id", eid);
    }

    let resp = req
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("请求 user resource 失败: {}", e))?;

    let status_code = resp.status();
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 user resource 响应失败: {} (http={})", e, status_code.as_u16()))?;

    if !status_code.is_success() {
        let message = body
            .get("message")
            .or_else(|| body.get("msg"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(format!(
            "请求 user resource 失败 (http={}): {}",
            status_code.as_u16(),
            message
        ));
    }

    if let Some(code) = body.get("code").and_then(|v| v.as_i64()) {
        if code != 0 && code != 200 {
            let message = body
                .get("message")
                .or_else(|| body.get("msg"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(format!("请求 user resource 失败 (code={}): {}", code, message));
        }
    }

    Ok(body)
}

pub async fn fetch_enterprise_user_usage(
    platform: BuddyPlatform,
    access_token: &str,
    uid: Option<&str>,
    enterprise_id: &str,
    domain: Option<&str>,
) -> Result<Value, String> {
    let client = build_client()?;
    let url = format!(
        "{}/v2/billing/meter/get-enterprise-user-usage",
        api_endpoint(platform)
    );

    let mut req = client
        .post(&url)
        .header("Accept", "application/json, text/plain, */*")
        .header("Accept-Language", "zh-CN,zh;q=0.9")
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Content-Type", "application/json")
        .header("X-Enterprise-Id", enterprise_id)
        .header("X-Tenant-Id", enterprise_id);
    if let Some(uid) = uid {
        req = req.header("X-User-Id", uid);
    }
    if let Some(domain) = domain {
        req = req.header("X-Domain", domain);
    }

    let resp = req
        .json(&json!({}))
        .send()
        .await
        .map_err(|e| format!("请求 enterprise user usage 失败: {}", e))?;

    let status_code = resp.status();
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 enterprise user usage 响应失败: {} (http={})", e, status_code.as_u16()))?;

    if !status_code.is_success() {
        let message = body
            .get("message")
            .or_else(|| body.get("msg"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(format!(
            "请求 enterprise user usage 失败 (http={}): {}",
            status_code.as_u16(),
            message
        ));
    }

    if let Some(code) = body.get("code").and_then(|v| v.as_i64()) {
        if code != 0 && code != 200 {
            let message = body
                .get("message")
                .or_else(|| body.get("msg"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(format!("请求 enterprise user usage 失败 (code={}): {}", code, message));
        }
    }

    if enterprise_usage_data(&body)
        .and_then(|data| data.get("limit_num").or_else(|| data.get("limitNum")))
        .and_then(|value| json_f64(Some(value)))
        .is_none()
    {
        return Err("enterprise user usage 响应缺少 limit_num/limitNum".to_string());
    }

    Ok(body)
}

/// 将企业用量响应包装成前端可识别的 get-user-resource 结构
pub fn wrap_enterprise_usage_as_resource(usage_body: &Value) -> Result<Value, String> {
    let data = enterprise_usage_data(usage_body)
        .ok_or_else(|| "enterprise user usage 响应缺少 data".to_string())?;
    let limit_num = data
        .get("limit_num")
        .or_else(|| data.get("limitNum"))
        .and_then(|value| json_f64(Some(value)))
        .ok_or_else(|| "enterprise user usage 响应缺少 limit_num/limitNum".to_string())?;
    let used_num = data
        .get("used_num")
        .or_else(|| data.get("usedNum"))
        .or_else(|| data.get("credit"))
        .and_then(|value| json_f64(Some(value)))
        .ok_or_else(|| "enterprise user usage 响应缺少 credit/used_num".to_string())?;
    let unlimited = limit_num == -1.0;
    let remain = if unlimited { -1.0 } else { (limit_num - used_num).max(0.0) };
    let cycle_start_time = json_scalar_string(
        data.get("cycle_start_time").or_else(|| data.get("cycleStartTime")),
    );
    let cycle_end_time = json_scalar_string(
        data.get("cycle_end_time").or_else(|| data.get("cycleEndTime")),
    );
    let cycle_reset_time = json_scalar_string(
        data.get("cycle_reset_time").or_else(|| data.get("cycleResetTime")),
    );

    let account = json!({
        "PackageCode": ENTERPRISE_PACKAGE_CODE,
        "PackageName": "企业版",
        "CycleCapacitySizePrecise": limit_num.to_string(),
        "CycleCapacityRemainPrecise": remain.to_string(),
        "CycleCapacityUsedPrecise": used_num.to_string(),
        "CycleCapacitySize": limit_num,
        "CycleCapacityRemain": remain,
        "CycleCapacityUsed": used_num,
        "CapacitySize": limit_num,
        "CapacityRemain": remain,
        "CapacityUsed": used_num,
        "CapacityUnit": "credits",
        "CycleStartTime": cycle_start_time,
        "CycleEndTime": cycle_end_time,
        "CycleResetTime": cycle_reset_time,
        "Unlimited": unlimited,
        "Status": 0
    });

    Ok(json!({
        "code": 0,
        "msg": "OK",
        "data": {
            "Response": {
                "Data": {
                    "Accounts": [account],
                    "TotalCount": 1,
                    "TotalDosage": used_num
                }
            }
        }
    }))
}

async fn fetch_user_resource_with_access_token_default(
    platform: BuddyPlatform,
    access_token: &str,
    uid: Option<&str>,
    enterprise_id: Option<&str>,
    domain: Option<&str>,
) -> Result<Value, String> {
    let payload = post_user_resource(
        platform,
        access_token,
        uid,
        enterprise_id,
        domain,
        build_user_resource_request_body(),
    )
    .await?;
    if user_resource_has_payload(&payload) {
        Ok(payload)
    } else if user_resource_has_shape(&payload) {
        Err("user resource 响应未包含可用资源".to_string())
    } else {
        Err("user resource 响应缺少 resources/Accounts".to_string())
    }
}

async fn fetch_quota_resource_for_account(
    platform: BuddyPlatform,
    access_token: &str,
    uid: Option<&str>,
    enterprise_id: Option<&str>,
    domain: Option<&str>,
) -> Result<Value, String> {
    if let Some(enterprise_id) = enterprise_id {
        let body = fetch_enterprise_user_usage(platform, access_token, uid, enterprise_id, domain).await?;
        wrap_enterprise_usage_as_resource(&body)
    } else {
        fetch_user_resource_with_access_token_default(platform, access_token, uid, None, domain).await
    }
}

/// 账号资料 + 用量刷新结果（合并进账号）
#[derive(Debug, Clone)]
pub struct RefreshPayload {
    pub email: String,
    pub uid: Option<String>,
    pub nickname: Option<String>,
    pub enterprise_id: Option<String>,
    pub enterprise_name: Option<String>,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_type: Option<String>,
    pub expires_at: Option<i64>,
    pub domain: Option<String>,
    pub plan_type: Option<String>,
    pub dosage_notify_code: Option<String>,
    pub dosage_notify_zh: Option<String>,
    pub dosage_notify_en: Option<String>,
    pub payment_type: Option<String>,
    pub quota_raw: Option<Value>,
    pub profile_raw: Option<Value>,
    pub usage_raw: Option<Value>,
    pub status: Option<String>,
    pub status_reason: Option<String>,
}

fn payload_to_account(platform: BuddyPlatform, p: RefreshPayload, base: Option<&BuddyAccount>) -> BuddyAccount {
    let now = chrono::Utc::now().timestamp();
    BuddyAccount {
        id: base.map(|b| b.id.clone()).unwrap_or_default(),
        platform: platform.as_str().to_string(),
        email: p.email,
        uid: p.uid,
        nickname: p.nickname,
        enterprise_id: p.enterprise_id,
        enterprise_name: p.enterprise_name,
        tags: base.and_then(|b| b.tags.clone()),
        access_token: p.access_token,
        refresh_token: p.refresh_token,
        token_type: p.token_type,
        expires_at: p.expires_at,
        domain: p.domain,
        plan_type: p.plan_type,
        dosage_notify_code: p.dosage_notify_code,
        dosage_notify_zh: p.dosage_notify_zh,
        dosage_notify_en: p.dosage_notify_en,
        payment_type: p.payment_type,
        quota_raw: p.quota_raw,
        usage_raw: p.usage_raw,
        profile_raw: p.profile_raw,
        status: p.status,
        status_reason: p.status_reason,
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: Some(now),
        last_checkin_time: base.and_then(|b| b.last_checkin_time),
        checkin_streak: base.map(|b| b.checkin_streak).unwrap_or(0),
        checkin_rewards: base.and_then(|b| b.checkin_rewards.clone()),
        auth_raw: base.and_then(|b| b.auth_raw.clone()),
        expiry_times: base.map(|b| b.expiry_times.clone()).unwrap_or_default(),
        created_at: base.map(|b| b.created_at).unwrap_or(now),
        last_used: base.map(|b| b.last_used).unwrap_or(now),
    }
}

/// 用现有账号刷新 token + 用量（参考 `refresh_payload_for_account_inner`）
pub async fn refresh_payload_for_account(
    platform: BuddyPlatform,
    account: &BuddyAccount,
) -> Result<(BuddyAccount, Option<String>), String> {
    let mut new_access_token = account.access_token.clone();
    let mut new_refresh_token = account.refresh_token.clone();
    let mut new_expires_at = account.expires_at;
    let mut new_domain = account.domain.clone();

    if let Some(refresh_tk) = account.refresh_token.as_deref() {
        match refresh_token(platform, &account.access_token, refresh_tk, account.domain.as_deref()).await {
            Ok(token_data) => {
                new_access_token = token_data
                    .get("accessToken")
                    .or_else(|| token_data.get("access_token"))
                    .and_then(|v| v.as_str())
                    .unwrap_or(&account.access_token)
                    .to_string();
                new_refresh_token = token_data
                    .get("refreshToken")
                    .or_else(|| token_data.get("refresh_token"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .or_else(|| account.refresh_token.clone());
                new_expires_at = token_expiry_at(&token_data).or(account.expires_at);
                new_domain = token_data
                    .get("domain")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .or_else(|| account.domain.clone());
            }
            Err(e) => {
                eprintln!("[Buddy] Token 刷新失败，将使用现有 token 查询配额: {}", e);
            }
        }
    }

    let dosage = fetch_dosage_notify(
        platform,
        &new_access_token,
        account.uid.as_deref(),
        account.enterprise_id.as_deref(),
        new_domain.as_deref(),
    )
    .await
    .ok();

    let payment = fetch_payment_type(
        platform,
        &new_access_token,
        account.uid.as_deref(),
        account.enterprise_id.as_deref(),
        new_domain.as_deref(),
    )
    .await
    .ok();

    let mut quota_refresh_error: Option<String> = None;
    let user_resource = match fetch_quota_resource_for_account(
        platform,
        &new_access_token,
        account.uid.as_deref(),
        account.enterprise_id.as_deref(),
        new_domain.as_deref(),
    )
    .await
    {
        Ok(payload) => Some(payload),
        Err(err) => {
            eprintln!("[Buddy] 刷新 user_resource 失败: {}", err);
            quota_refresh_error = Some(err.clone());
            None
        }
    };

    let dosage_data = dosage.as_ref().and_then(|v| v.get("data"));
    let dosage_notify_code = dosage_data
        .and_then(|d| d.get("dosageNotifyCode"))
        .map(|v| match v {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            _ => v.to_string(),
        });
    let dosage_notify_zh = dosage_data
        .and_then(|d| d.get("dosageNotifyZh"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let dosage_notify_en = dosage_data
        .and_then(|d| d.get("dosageNotifyEn"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let payment_data = payment.as_ref().and_then(|v| v.get("data"));
    let payment_type = payment_data
        .and_then(|d| {
            d.as_str().map(|s| s.to_string()).or_else(|| {
                d.get("paymentType").and_then(|v| v.as_str()).map(|s| s.to_string())
            })
        })
        .or_else(|| account.payment_type.clone());

    let mut combined_quota = account
        .quota_raw
        .as_ref()
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(d) = &dosage {
        combined_quota.insert("dosage".to_string(), d.clone());
    }
    if let Some(p) = &payment {
        combined_quota.insert("payment".to_string(), p.clone());
    }
    if let Some(r) = &user_resource {
        combined_quota.insert("userResource".to_string(), r.clone());
    }

    let quota_raw = if combined_quota.is_empty() {
        account.quota_raw.clone()
    } else {
        Some(Value::Object(combined_quota))
    };

    let final_email = normalize_non_empty(Some(account.email.as_str()))
        .unwrap_or_else(|| account.email.clone());

    let payload = RefreshPayload {
        email: final_email,
        uid: account.uid.clone(),
        nickname: account.nickname.clone(),
        enterprise_id: account.enterprise_id.clone(),
        enterprise_name: account.enterprise_name.clone(),
        access_token: new_access_token,
        refresh_token: new_refresh_token,
        token_type: account.token_type.clone(),
        expires_at: new_expires_at,
        domain: new_domain,
        plan_type: account.plan_type.clone(),
        dosage_notify_code,
        dosage_notify_zh,
        dosage_notify_en,
        payment_type,
        quota_raw,
        profile_raw: account.profile_raw.clone(),
        usage_raw: user_resource.or_else(|| account.usage_raw.clone()),
        status: account.status.clone(),
        status_reason: account.status_reason.clone(),
    };

    Ok((payload_to_account(platform, payload, Some(account)), quota_refresh_error))
}

/// 用裸 token 构建账号（拉取资料 + 用量），参考 `build_payload_from_token`
pub async fn build_payload_from_token(
    platform: BuddyPlatform,
    access_token: &str,
) -> Result<BuddyAccount, String> {
    let client = build_client()?;
    let url = format!("{}{}/accounts", api_endpoint(platform), API_PREFIX);

    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await
        .map_err(|e| format!("请求 accounts 失败: {}", e))?;

    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 accounts 响应失败: {}", e))?;

    let accounts = body
        .get("data")
        .and_then(|d| d.get("accounts"))
        .and_then(|a| a.as_array());

    let account_data = accounts
        .and_then(|arr| {
            arr.iter().find(|a| {
                a.get("lastLogin").and_then(|v| v.as_bool()).unwrap_or(false)
            })
        })
        .or_else(|| accounts.and_then(|arr| arr.first()))
        .cloned()
        .unwrap_or(json!({}));

    let uid = account_data.get("uid").and_then(|v| v.as_str()).map(|s| s.to_string());
    let nickname = account_data.get("nickname").and_then(|v| v.as_str()).map(|s| s.to_string());
    let email = account_data.get("email").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let enterprise_id = account_data
        .get("enterpriseId")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let enterprise_name = account_data
        .get("enterpriseName")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let dosage = fetch_dosage_notify(platform, access_token, uid.as_deref(), enterprise_id.as_deref(), None)
        .await
        .ok();
    let payment = fetch_payment_type(platform, access_token, uid.as_deref(), enterprise_id.as_deref(), None)
        .await
        .ok();
    let user_resource = fetch_user_resource_with_access_token_default(
        platform,
        access_token,
        uid.as_deref(),
        enterprise_id.as_deref(),
        None,
    )
    .await
    .ok();

    let dosage_data = dosage.as_ref().and_then(|v| v.get("data"));
    let dosage_notify_code = dosage_data
        .and_then(|d| d.get("dosageNotifyCode"))
        .map(|v| match v {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            _ => v.to_string(),
        });
    let dosage_notify_zh = dosage_data
        .and_then(|d| d.get("dosageNotifyZh"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let dosage_notify_en = dosage_data
        .and_then(|d| d.get("dosageNotifyEn"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let payment_data = payment.as_ref().and_then(|v| v.get("data"));
    let payment_type = payment_data.and_then(|d| {
        d.as_str().map(|s| s.to_string()).or_else(|| {
            d.get("paymentType").and_then(|v| v.as_str()).map(|s| s.to_string())
        })
    });

    let mut combined_quota = serde_json::Map::new();
    if let Some(payload) = dosage.as_ref() {
        combined_quota.insert("dosage".to_string(), payload.clone());
    }
    if let Some(payload) = payment.as_ref() {
        combined_quota.insert("payment".to_string(), payload.clone());
    }
    if let Some(payload) = user_resource.as_ref() {
        combined_quota.insert("userResource".to_string(), payload.clone());
    }
    let quota_raw = if combined_quota.is_empty() {
        None
    } else {
        Some(Value::Object(combined_quota))
    };

    let email_final = if email.is_empty() {
        nickname
            .clone()
            .or_else(|| uid.clone())
            .unwrap_or_else(|| "unknown".to_string())
    } else {
        email
    };

    let now = chrono::Utc::now().timestamp();
    Ok(BuddyAccount {
        id: String::new(),
        platform: platform.as_str().to_string(),
        email: email_final,
        uid,
        nickname,
        enterprise_id,
        enterprise_name,
        tags: None,
        access_token: access_token.to_string(),
        refresh_token: None,
        token_type: Some("Bearer".to_string()),
        expires_at: None,
        domain: None,
        plan_type: None,
        dosage_notify_code,
        dosage_notify_zh,
        dosage_notify_en,
        payment_type,
        quota_raw,
        usage_raw: user_resource,
        profile_raw: Some(account_data),
        status: Some("normal".to_string()),
        status_reason: None,
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: Some(now),
        last_checkin_time: None,
        checkin_streak: 0,
        checkin_rewards: None,
        auth_raw: None,
        expiry_times: Default::default(),
        created_at: 0,
        last_used: 0,
    })
}

// ─── 派 Buddy 旅行（WorkBuddy 活动，路径不带 /v2/ 前缀） ───

/// 令牌失效错误前缀（调度器据此识别「登录态过期」类错误）。
pub const TRAVEL_AUTH_EXPIRED_PREFIX: &str = "AUTH_EXPIRED:";

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

/// 旅行接口通用请求（WorkBuddy 域名，不带 /v2/ 前缀——与签到路径体系不同）。
async fn travel_request(
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
    access_token: &str,
    uid: Option<&str>,
    enterprise_id: Option<&str>,
    domain: Option<&str>,
) -> Result<Value, String> {
    let client = build_client()?;
    let url = format!("{}{}", WORKBUDDY_API_ENDPOINT, path);
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
        .map_err(|e| format!("请求 {} 失败: {}", path, e))?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    if status == 401 || status == 403 {
        return Err(format!(
            "{}令牌已失效（HTTP {}），请打开 WorkBuddy 刷新登录态",
            TRAVEL_AUTH_EXPIRED_PREFIX, status
        ));
    }
    if status != 200 {
        return Err(format!(
            "请求 {} 失败 (HTTP {}): {}",
            path,
            status,
            text.chars().take(200).collect::<String>()
        ));
    }
    serde_json::from_str(&text).map_err(|e| format!("解析 {} 响应失败: {}", path, e))
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
        return Err(format!("派出失败 (code={}): {}", code, message));
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
        return Err(format!("领取失败 (code={}): {}", code, message));
    }
    Ok(body
        .get("data")
        .and_then(|d| d.get("reward_credit"))
        .and_then(Value::as_i64))
}

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
    #[serde(default, alias = "todayCredit", skip_serializing_if = "Option::is_none")]
    pub today_credit: Option<i64>,
    #[serde(default, alias = "nextStreakDay", skip_serializing_if = "Option::is_none")]
    pub next_streak_day: Option<i64>,
    #[serde(default, alias = "isStreakDay", skip_serializing_if = "Option::is_none")]
    pub is_streak_day: Option<bool>,
    #[serde(default, alias = "checkinDates", skip_serializing_if = "Option::is_none")]
    pub checkin_dates: Option<Vec<String>>,
    #[serde(default, alias = "streakBonusDays", skip_serializing_if = "Option::is_none")]
    pub streak_bonus_days: Option<i64>,
    #[serde(default, alias = "streakBonusCredit", skip_serializing_if = "Option::is_none")]
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
}

fn default_checkin_active_true() -> bool {
    true
}

fn json_bool(value: &Value, snake: &str, camel: &str) -> Option<bool> {
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

fn json_i64(value: &Value, snake: &str, camel: &str) -> Option<i64> {
    value
        .get(snake)
        .or_else(|| value.get(camel))
        .and_then(|v| v.as_i64())
}

fn parse_checkin_status_data(data: &Value) -> Result<CheckinStatusResponse, String> {
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
async fn fetch_checkin_status_from(
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
        return Err(format!("请求 {} 失败 (http={}): {}", path, status_code.as_u16(), message));
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
    let url = format!("{}/v2/billing/meter/daily-checkin", CODEBUDDY_CN_API_ENDPOINT);

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
        return Err(format!(
            "请求 daily-checkin 失败 (http={}): {}",
            status_code.as_u16(),
            message
        ));
    }

    let code = body.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
    let api_msg = body
        .get("message")
        .or_else(|| body.get("msg"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown error")
        .to_string();

    if code != 0 {
        return Ok(CheckinResponse {
            success: false,
            message: Some(api_msg),
            reward: None,
            credit: None,
            streak_days: None,
            is_streak_day: None,
            next_checkin_in: None,
        });
    }

    let data = body
        .get("data")
        .ok_or_else(|| "daily-checkin 响应缺少 data 字段".to_string())?;

    let success = data
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let message = data.get("message").and_then(|v| v.as_str()).map(|s| s.to_string());
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
        message,
        reward,
        credit,
        streak_days,
        is_streak_day,
        next_checkin_in,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enterprise_usage_wrapping() {
        let body = json!({
            "code": 0,
            "data": {
                "limit_num": 1000,
                "credit": 300,
                "cycle_start_time": "2026-09-01 00:00:00",
                "cycle_end_time": "2026-10-01 00:00:00"
            }
        });
        let wrapped = wrap_enterprise_usage_as_resource(&body).unwrap();
        let items = user_resource_items(&wrapped).unwrap();
        assert_eq!(items.len(), 1);
        let account = &items[0];
        assert_eq!(account["PackageCode"].as_str(), Some(ENTERPRISE_PACKAGE_CODE));
        assert_eq!(account["CycleCapacitySize"].as_f64(), Some(1000.0));
        assert_eq!(account["CycleCapacityRemain"].as_f64(), Some(700.0));
        assert_eq!(account["CycleCapacityUsed"].as_f64(), Some(300.0));
        assert_eq!(account["Unlimited"].as_bool(), Some(false));
    }

    #[test]
    fn enterprise_unlimited_wrapping() {
        let body = json!({
            "code": 0,
            "data": { "limit_num": -1, "credit": 50 }
        });
        let wrapped = wrap_enterprise_usage_as_resource(&body).unwrap();
        let account = &user_resource_items(&wrapped).unwrap()[0];
        assert_eq!(account["Unlimited"].as_bool(), Some(true));
        assert_eq!(account["CycleCapacityRemain"].as_f64(), Some(-1.0));
    }

    #[test]
    fn checkin_status_loose_parsing() {
        // snake_case + 宽松布尔（0/1 字符串）
        let data = json!({
            "today_checked_in": "1",
            "streak_days": 3,
            "daily_credit": 10
        });
        let status = parse_checkin_status_data(&data).unwrap();
        assert!(status.today_checked_in);
        assert!(status.active); // 缺省 active → true
        assert_eq!(status.streak_days, 3);
        assert_eq!(status.daily_credit, 10);
    }

    #[test]
    fn checkin_status_camel_case() {
        let data = json!({
            "todayCheckedIn": false,
            "Active": false,
            "streakDays": 5
        });
        let status = parse_checkin_status_data(&data).unwrap();
        assert!(!status.today_checked_in);
        assert!(!status.active);
        assert_eq!(status.streak_days, 5);
    }

    #[test]
    fn token_expiry_normalization() {
        // 秒级时间戳 → 毫秒
        assert_eq!(token_expiry_at(&json!({"expiresAt": 1750000000})), Some(1750000000000));
        // 毫秒级保留
        assert_eq!(token_expiry_at(&json!({"expiresAt": 1750000000000i64})), Some(1750000000000i64));
        // expiresIn 秒 → 当前毫秒 + 秒*1000
        let from_in = token_expiry_at(&json!({"expiresIn": 3600})).unwrap();
        let now_ms = chrono::Utc::now().timestamp_millis();
        assert!((from_in - (now_ms + 3600_000)).abs() < 2000);
    }
}