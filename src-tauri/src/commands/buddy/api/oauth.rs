//! oauth responsibility.
use super::*;

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

pub(super) fn decorate_login_url(
    raw_url: &str,
    version: Option<&str>,
    login_session_id: &str,
) -> String {
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

    eprintln!(
        "[Buddy OAuth] 登录已启动: platform={}, login_id={}",
        platform.as_str(),
        login_id
    );

    Ok(OAuthStartResponse {
        login_id,
        verification_uri: verification_uri.clone(),
        verification_uri_complete: Some(verification_uri),
        expires_in: OAUTH_TIMEOUT_SECONDS,
        interval_seconds: OAUTH_POLL_INTERVAL_MS / 1000 + 1,
    })
}

pub(super) async fn fetch_account_info(
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

    let uid = data
        .get("uid")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let nickname = data
        .get("nickname")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let email = data
        .get("email")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
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
                                eprintln!(
                                    "[Buddy OAuth] 获取 token 成功: platform={}",
                                    platform.as_str()
                                );
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

                                let info_err = match &account_info {
                                    Ok(_) => None,
                                    Err(e) => Some(e.clone()),
                                };
                                let (
                                    uid,
                                    nickname,
                                    email,
                                    enterprise_id,
                                    enterprise_name,
                                    profile_raw,
                                ) = match account_info {
                                    Ok(info) => info,
                                    Err(e) => {
                                        eprintln!("[Buddy OAuth] 获取账号信息失败: {}", e);
                                        (None, None, String::new(), None, None, None)
                                    }
                                };
                                // uid / email 都拿不到：这个账号既识别不了、也用不了（上游 X-User-Id 就来自 uid）。
                                // 以前会继续往下走，最后在 store 的路径校验处报「账号 ID 不能为空」，
                                // 把真因（账号信息接口失败）埋掉了；这里带上原始错误直接失败。
                                if uid.is_none() && email.trim().is_empty() {
                                    return Err(format!(
                                        "官方接口未返回账号信息（uid / email 均为空），无法保存账号{}",
                                        info_err
                                            .map(|e| format!("：{}", e))
                                            .unwrap_or_default()
                                    ));
                                }

                                let account = BuddyAccount {
                                    // id 由 store::upsert_account 按 uid/email 派生（{platform}_{md5}）；
                                    // 库里已有同 uid/email 时直接复用旧 id，所以这里留空即可。
                                    id: String::new(),
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
