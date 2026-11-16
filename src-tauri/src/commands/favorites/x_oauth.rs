//! X 官方 API 的 OAuth 2.0（PKCE）授权与 token 管理。
//!
//! 桌面端是 **public client**：只有 `client_id`，没有 `client_secret`；
//! PKCE 用 S256，回调走本机 loopback（`127.0.0.1:17890/callback`），
//! 必须与 Developer Console 里登记的 Callback URI 完全一致。
//!
//! token 只存本地 `favorite_credential` 表，日志与错误信息里一律不出现内容。

use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri_plugin_opener::OpenerExt;

use super::db;

/// 凭证键：存 OAuth token 的 JSON。
pub const OAUTH_KEY: &str = "x-oauth";

/// 本地回调端口：与 App 里登记的 Callback URI 绑定，不能随便换。
pub const CALLBACK_PORT: u16 = 17890;

/// 本地回调路径。
pub const CALLBACK_PATH: &str = "/callback";

/// 与 Developer Console 里登记的 Callback URI 必须逐字符一致。
pub const REDIRECT_URI: &str = "http://127.0.0.1:17890/callback";

pub const AUTHORIZE_URL: &str = "https://x.com/i/oauth2/authorize";
pub const TOKEN_URL: &str = "https://api.x.com/2/oauth2/token";
pub const ME_URL: &str = "https://api.x.com/2/users/me";

/// 只读书签所需的最小 scope 集合；`offline.access` 用来换 refresh token。
pub const SCOPES: &str = "tweet.read users.read bookmark.read offline.access";

/// 提前多少秒刷新：避免拿着一个正好在请求途中过期的 token。
pub const REFRESH_MARGIN_SECS: i64 = 120;

/// 授权完成后落库的完整凭证。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct XToken {
    pub client_id: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// unix 秒；0 表示未知（按需要刷新处理）
    pub expires_at: i64,
    pub user_id: String,
    pub screen_name: String,
    pub name: String,
}

/// token 端点返回的原始字段（还没有用户信息）。
#[derive(Debug, Clone, PartialEq)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: i64,
}

pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 生成 PKCE `code_verifier`：32 字节随机数的 base64url（无填充，43 字符）。
pub fn code_verifier() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|e| format!("生成 PKCE verifier 失败: {e}"))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

/// `code_challenge = BASE64URL(SHA256(verifier))`（无填充）。
pub fn code_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

/// 构造授权 URL（`state` 用于回调时防 CSRF）。
pub fn authorize_url(client_id: &str, state: &str, challenge: &str) -> String {
    let mut url = url::Url::parse(AUTHORIZE_URL).expect("内置授权地址必须合法");
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("scope", SCOPES)
        .append_pair("state", state)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256");
    url.to_string()
}

/// 解析 token 端点响应，把 `expires_in` 换算成本地绝对时间。
pub fn parse_token_response(body: &str, now: i64) -> Result<TokenResponse, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("解析 token 响应失败: {e}"))?;
    let access_token = value
        .get("access_token")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            let detail = value
                .get("error_description")
                .and_then(|v| v.as_str())
                .or_else(|| value.get("error").and_then(|v| v.as_str()))
                .unwrap_or("响应里没有 access_token");
            format!("X 授权失败: {detail}")
        })?;
    let refresh_token = value
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    // 官方通常给 7200 秒；缺失时按 2 小时处理，避免把 token 当成永久有效
    let expires_in = value
        .get("expires_in")
        .and_then(|v| v.as_i64())
        .unwrap_or(7200)
        .max(0);
    Ok(TokenResponse {
        access_token,
        refresh_token,
        expires_at: now + expires_in,
    })
}

/// token 是否该刷新了（剩余寿命小于安全边界）。
pub fn token_needs_refresh(token: &XToken, now: i64) -> bool {
    token.expires_at <= now + REFRESH_MARGIN_SECS
}

// ─── 本地凭证 ───

/// token 等同于密码：落库前必须加密（access/refresh token 可直接调用 API）。
pub fn encode_token(token: &XToken) -> Result<String, String> {
    let json = serde_json::to_string(token).map_err(|e| format!("序列化 X 授权信息失败: {e}"))?;
    crate::commands::secrets::encrypt_secret(&json)
}

/// 读取兼容旧版明文记录；下次 `save_token` 会自动转成密文。
pub fn decode_token(stored: &str) -> Result<XToken, String> {
    let json = if stored.trim_start().starts_with("ENC_V2:") {
        crate::commands::secrets::decrypt_secret(stored.trim())?
    } else {
        stored.to_string()
    };
    serde_json::from_str(&json).map_err(|e| format!("本地 X 授权信息已损坏，请重新授权: {e}"))
}

/// 手动粘贴 token 的本地形态检查（可用性由 `fetch_me` 实际验证）。
pub fn validate_manual_token(client_id: &str, access_token: &str) -> Result<(), String> {
    if client_id.trim().is_empty() {
        return Err("请填写 OAuth 2.0 Client ID".to_string());
    }
    if access_token.trim().is_empty() {
        return Err("请填写 Access Token".to_string());
    }
    Ok(())
}

/// 读本地 token；没有配置返回 `None`。
pub fn load_token() -> Result<Option<XToken>, String> {
    let raw = db::with_conn(|conn| db::get_credential(conn, OAUTH_KEY))?;
    match raw {
        Some(text) if !text.trim().is_empty() => decode_token(&text).map(Some),
        _ => Ok(None),
    }
}

pub fn save_token(token: &XToken) -> Result<(), String> {
    let text = encode_token(token)?;
    db::with_conn(|conn| db::set_credential(conn, OAUTH_KEY, &text))
}

pub fn clear_token() -> Result<(), String> {
    db::with_conn(|conn| db::set_credential(conn, OAUTH_KEY, ""))
}

// ─── 本地回调 ───

struct CallbackResult {
    code: String,
    state: String,
}

fn random_state() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|e| format!("生成 state 失败: {e}"))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

/// 起一个只服务一次的本地回调：拿到 `code` 就返回，并给浏览器一个可关闭的页面。
async fn serve_once(listener: tokio::net::TcpListener) -> Result<CallbackResult, String> {
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<CallbackResult, String>>();
    let tx = std::sync::Arc::new(tokio::sync::Mutex::new(Some(tx)));

    let router = axum::Router::new().route(
        CALLBACK_PATH,
        axum::routing::get({
            let tx = tx.clone();
            move |axum::extract::Query(params): axum::extract::Query<
                std::collections::HashMap<String, String>,
            >| {
                let tx = tx.clone();
                async move {
                    let outcome = if let Some(error) = params.get("error") {
                        Err(format!("X 授权被拒绝: {error}"))
                    } else if let Some(code) = params.get("code") {
                        Ok(CallbackResult {
                            code: code.clone(),
                            state: params.get("state").cloned().unwrap_or_default(),
                        })
                    } else {
                        Err("X 授权回调缺少 code".to_string())
                    };
                    let mut slot = tx.lock().await;
                    if let Some(sender) = slot.take() {
                        let _ = sender.send(outcome);
                    }
                    axum::response::Html(
                        "<!doctype html><meta charset=\"utf-8\"><title>X 授权完成</title>\
                         <body style=\"font-family:system-ui,sans-serif;padding:48px;background:#0f172a;color:#e2e8f0\">\
                         <h2>X 授权已完成</h2><p>可以关闭此页面，回到应用继续。</p></body>",
                    )
                }
            }
        }),
    );

    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    let received = tokio::time::timeout(std::time::Duration::from_secs(300), rx).await;
    // 无论成功、超时还是用户取消，都释放端口
    server.abort();

    match received {
        Err(_) => Err("等待 X 授权超时（5 分钟），请重新点「开始授权」".to_string()),
        Ok(Err(_)) => Err("X 授权回调通道已关闭".to_string()),
        Ok(Ok(outcome)) => outcome,
    }
}

// ─── token 交换与刷新 ───

async fn post_token(form: &[(&str, &str)]) -> Result<TokenResponse, String> {
    let resp = crate::commands::utils::get_http_client()
        .post(TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(form)
        .send()
        .await
        .map_err(|e| format!("请求 X token 失败: {e}"))?;
    let status = resp.status().as_u16();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("读取 X token 响应失败: {e}"))?;
    if !(200..300).contains(&status) {
        return Err(format!(
            "X 授权换 token 失败 (HTTP {status}): {}",
            crate::commands::utils::truncate_utf8(&body, 200)
        ));
    }
    parse_token_response(&body, now_unix())
}

async fn exchange_code(
    client_id: &str,
    code: &str,
    verifier: &str,
) -> Result<TokenResponse, String> {
    post_token(&[
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", REDIRECT_URI),
        ("client_id", client_id),
        ("code_verifier", verifier),
    ])
    .await
}

async fn refresh_access_token(token: &XToken) -> Result<TokenResponse, String> {
    let refresh = token
        .refresh_token
        .as_deref()
        .ok_or_else(|| "X 授权已过期且没有 refresh token，请重新点「X 官方授权」".to_string())?;
    match post_token(&[
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh),
        ("client_id", token.client_id.as_str()),
    ])
    .await
    {
        Ok(next) => Ok(next),
        Err(err) => {
            let _ = db::with_conn(|conn| db::mark_credential_status(conn, OAUTH_KEY, "expired"));
            Err(format!("{err}；请重新点「X 官方授权」"))
        }
    }
}

async fn fetch_me(access_token: &str) -> Result<(String, String, String), String> {
    let resp = crate::commands::utils::get_http_client()
        .get(ME_URL)
        .query(&[("user.fields", "username,name")])
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| format!("读取 X 账号信息失败: {e}"))?;
    let status = resp.status().as_u16();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("读取 X 账号信息失败: {e}"))?;
    if !(200..300).contains(&status) {
        return Err(format!(
            "读取 X 账号信息失败 (HTTP {status}): {}",
            crate::commands::utils::truncate_utf8(&body, 160)
        ));
    }
    let value: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("解析 X 账号信息失败: {e}"))?;
    let data = value
        .get("data")
        .ok_or_else(|| "X 没有返回账号信息".to_string())?;
    let user_id = data
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "X 没有返回用户 id".to_string())?
        .to_string();
    Ok((
        user_id,
        data.get("username")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        data.get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    ))
}

/// 直接用已有的 access / refresh token 落库，跳过浏览器授权流程。
///
/// 保存前用 `/2/users/me` 实际验证一次：用户手工复制 token 很容易漏字符，
/// 存进库等到导入时才报 401 会非常难排查。
pub async fn save_manual_token(
    client_id: &str,
    access_token: &str,
    refresh_token: Option<&str>,
) -> Result<XToken, String> {
    validate_manual_token(client_id, access_token)?;
    let access = access_token.trim();
    let (user_id, screen_name, name) = fetch_me(access)
        .await
        .map_err(|e| format!("{e}（请确认 Access Token 未过期，且授权含 users.read 权限）"))?;
    let token = XToken {
        client_id: client_id.trim().to_string(),
        access_token: access.to_string(),
        refresh_token: refresh_token
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .map(str::to_string),
        // 手填的 token 拿不到原始签发时间，按 X 默认 2 小时有效期保守估计；
        // 有 refresh token 时会在到期前自动刷新。
        expires_at: now_unix() + 7200,
        user_id,
        screen_name,
        name,
    };
    save_token(&token)?;
    Ok(token)
}

/// 确保拿到可用的 access token（必要时自动刷新）。
pub async fn ensure_access_token() -> Result<XToken, String> {
    let token = load_token()?.ok_or_else(|| {
        "尚未授权 X 官方 API：请在「X 官方授权」里填写 Client ID 并完成一次授权".to_string()
    })?;
    if !token_needs_refresh(&token, now_unix()) {
        return Ok(token);
    }

    let refreshed = refresh_access_token(&token).await?;
    let XToken {
        client_id,
        refresh_token,
        user_id,
        screen_name,
        name,
        ..
    } = token;
    let next = XToken {
        client_id,
        access_token: refreshed.access_token,
        refresh_token: refreshed.refresh_token.or(refresh_token),
        expires_at: refreshed.expires_at,
        user_id,
        screen_name,
        name,
    };
    save_token(&next)?;
    Ok(next)
}

/// 完整授权流程：占端口 → 开浏览器 → 等回调 → 换 token → 记下用户信息。
pub async fn start_authorization(
    app: &tauri::AppHandle,
    client_id: &str,
) -> Result<XToken, String> {
    let client_id = client_id.trim();
    if client_id.is_empty() {
        return Err("请先填写 X 的 OAuth 2.0 Client ID".to_string());
    }

    let verifier = code_verifier()?;
    let challenge = code_challenge(&verifier);
    let state = random_state()?;

    // 先占端口：占了再开浏览器，避免用户授权完才发现端口被占用
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", CALLBACK_PORT))
        .await
        .map_err(|e| {
            format!(
                "无法监听本地回调端口 {CALLBACK_PORT}：上一次授权可能仍在等待（最多 5 分钟），\
                 或该端口被其它程序占用（{e}）"
            )
        })?;

    let url = authorize_url(client_id, &state, &challenge);
    app.opener()
        .open_url(&url, None::<&str>)
        .map_err(|e| format!("打开浏览器失败: {e}"))?;

    let callback = serve_once(listener).await?;
    if callback.state != state {
        return Err("X 授权回调的 state 不匹配，已中止（请重新授权）".to_string());
    }

    let token = exchange_code(client_id, &callback.code, &verifier).await?;
    let (user_id, screen_name, name) = fetch_me(&token.access_token).await?;
    let full = XToken {
        client_id: client_id.to_string(),
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        expires_at: token.expires_at,
        user_id,
        screen_name,
        name,
    };
    save_token(&full)?;
    Ok(full)
}

#[cfg(test)]
mod tests;
