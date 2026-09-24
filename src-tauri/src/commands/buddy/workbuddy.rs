//! WorkBuddy 平台：本地导入 + 切换账号。
//!
//! 移植自 cockpit-tools `workbuddy_account.rs`（src-tauri 版）：
//! - 本地登录态存于 `%LOCALAPPDATA%/CodeBuddyExtension/Data/Public/auth/workbuddy-desktop.info`
//!   （macOS: `~/Library/Application Support/CodeBuddyExtension/Data/Public/auth/`；
//!    Linux: `~/.local/share/CodeBuddyExtension/Data/Public/auth/`）
//! - 切换 = 用目标账号重建该 JSON（account/auth/accounts/allAccounts），原子写回，
//!   带「官方加密字段保护」与「登出标记清理」。

use std::fs;
use std::path::PathBuf;

use base64::{engine::general_purpose, Engine as _};
use sha2::{Digest, Sha256};

use super::crypto::Platform;
use super::models::{BuddyAccount, BuddyPlatform};
use super::store;

const SECRET_EXTENSION_ID: &str = "tencent-cloud.coding-copilot";
const SECRET_KEY: &str = "planning-genie.new.accessTokencn";

/// 默认 auth 文件路径（三个平台）
pub fn default_auth_file_path(platform: BuddyPlatform) -> Option<PathBuf> {
    // 两个 WorkBuddy 版本共用同一个 CodeBuddyExtension 目录，只靠文件名区分
    let file_name = platform.auth_file_name();
    if file_name.is_empty() {
        return None;
    }
    let home = dirs::home_dir()?;
    #[cfg(target_os = "windows")]
    {
        return Some(
            home.join("AppData")
                .join("Local")
                .join("CodeBuddyExtension")
                .join("Data")
                .join("Public")
                .join("auth")
                .join(file_name),
        );
    }
    #[cfg(target_os = "macos")]
    {
        return Some(
            home.join("Library")
                .join("Application Support")
                .join("CodeBuddyExtension")
                .join("Data")
                .join("Public")
                .join("auth")
                .join(file_name),
        );
    }
    #[cfg(target_os = "linux")]
    {
        return Some(
            home.join(".local")
                .join("share")
                .join("CodeBuddyExtension")
                .join("Data")
                .join("Public")
                .join("auth")
                .join(file_name),
        );
    }
    #[allow(unreachable_code)]
    None
}

/// WorkBuddy 数据根目录（state.vscdb 所在目录的上一级）。
///
/// 仅 CN 版有该目录；WorkBuddy AI 的数据目录未公开，参考实现也只读写登录文件，
/// 故返回 None（调用方据此跳过会话/secret 相关分支）。
pub fn default_data_dir(platform: BuddyPlatform) -> Option<PathBuf> {
    if !matches!(platform, BuddyPlatform::Workbuddy) {
        return None;
    }
    dirs::data_dir().map(|d| d.join("WorkBuddy"))
}

pub fn default_state_db_path(platform: BuddyPlatform) -> Option<PathBuf> {
    default_data_dir(platform).map(|d| d.join("User").join("globalStorage").join("state.vscdb"))
}

fn logout_marker_path(auth_file: &PathBuf) -> PathBuf {
    PathBuf::from(format!("{}.logged-out", auth_file.to_string_lossy()))
}

fn json_string_field(obj: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(v) = obj.get(*key).and_then(|v| v.as_str()) {
            let t = v.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    None
}

fn json_i64_field(obj: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> Option<i64> {
    for key in keys {
        let Some(raw) = obj.get(*key) else { continue };
        if let Some(v) = raw.as_i64() {
            return Some(v);
        }
        if let Some(v) = raw.as_u64() {
            if let Ok(parsed) = i64::try_from(v) {
                return Some(parsed);
            }
        }
        if let Some(v) = raw.as_str() {
            if let Ok(parsed) = v.trim().parse::<i64>() {
                return Some(parsed);
            }
        }
    }
    None
}

/// 从本地登录 JSON 提取 access token（兼容多层结构）
fn parse_local_access_token(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }
        serde_json::Value::Array(arr) => arr.iter().find_map(parse_local_access_token),
        serde_json::Value::Object(obj) => {
            let direct = json_string_field(obj, &["token", "access_token", "accessToken"]);
            if let Some(token) = direct {
                return Some(token);
            }
            let auth_token = obj
                .get("auth")
                .and_then(|v| v.as_object())
                .and_then(|auth| json_string_field(auth, &["accessToken", "access_token"]));
            if let Some(token) = auth_token {
                return Some(token);
            }
            let nested = obj
                .get("session")
                .or_else(|| obj.get("data"))
                .and_then(parse_local_access_token);
            if nested.is_some() {
                return nested;
            }
            None
        }
        _ => None,
    }
}

/// 规范化 token：`uid+token` 形式取 `+` 之后
fn normalize_local_token(token: &str) -> Option<String> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some((_, suffix)) = trimmed.split_once('+') {
        let suffix = suffix.trim();
        if !suffix.is_empty() {
            return Some(suffix.to_string());
        }
    }
    Some(trimmed.to_string())
}

/// 提取 `uid+token` 的前缀 uid
fn extract_local_token_parts(token: &str) -> Option<(Option<String>, String)> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some((prefix, suffix)) = trimmed.split_once('+') {
        let uid = prefix.trim();
        let token_value = suffix.trim();
        if token_value.is_empty() {
            return None;
        }
        let uid_opt = if uid.is_empty() {
            None
        } else {
            Some(uid.to_string())
        };
        return Some((uid_opt, token_value.to_string()));
    }
    Some((None, trimmed.to_string()))
}

/// 从 JWT 提取 `sub`（新版 WorkBuddy token 无 `uid+` 前缀时回退）
fn extract_uid_from_jwt(token: &str) -> Option<String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() < 2 {
        return None;
    }
    let decoded = general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .or_else(|_| general_purpose::STANDARD.decode(parts[1]))
        .ok()?;
    let value: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    value
        .get("sub")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn normalize_identity(v: Option<&str>) -> Option<String> {
    v.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
}

fn normalize_email_identity(v: Option<&str>) -> Option<String> {
    normalize_identity(v).filter(|s| s.contains('@'))
}

fn accounts_match(
    existing_uid: Option<&String>,
    existing_email: Option<&String>,
    incoming_uid: Option<&String>,
    incoming_email: Option<&String>,
) -> bool {
    if let (Some(existing), Some(incoming)) = (existing_uid, incoming_uid) {
        if existing == incoming {
            return true;
        }
    }
    if let (Some(existing), Some(incoming)) = (existing_email, incoming_email) {
        if existing == incoming {
            if let (Some(eu), Some(iu)) = (existing_uid, incoming_uid) {
                if eu != iu {
                    return false;
                }
            }
            return true;
        }
    }
    false
}

/// 从本地登录 JSON 构建账号
fn build_account_from_local(
    platform: BuddyPlatform,
    access_token: String,
    parsed_json: Option<serde_json::Value>,
    uid_from_token: Option<String>,
) -> BuddyAccount {
    let root_obj = parsed_json.as_ref().and_then(|v| v.as_object());
    let account_obj = root_obj.and_then(|obj| obj.get("account").and_then(|v| v.as_object()));
    let auth_obj = root_obj.and_then(|obj| obj.get("auth").and_then(|v| v.as_object()));

    let uid = root_obj
        .and_then(|obj| json_string_field(obj, &["uid"]))
        .or_else(|| account_obj.and_then(|obj| json_string_field(obj, &["uid", "id"])))
        .or(uid_from_token);

    let nickname = root_obj
        .and_then(|obj| json_string_field(obj, &["nickname", "name"]))
        .or_else(|| account_obj.and_then(|obj| json_string_field(obj, &["nickname", "label"])));

    let email = root_obj
        .and_then(|obj| json_string_field(obj, &["email"]))
        .or_else(|| account_obj.and_then(|obj| json_string_field(obj, &["email"])))
        .or_else(|| auth_obj.and_then(|obj| json_string_field(obj, &["email"])))
        .or_else(|| nickname.clone())
        .or_else(|| uid.clone())
        .unwrap_or_else(|| "unknown".to_string());

    let enterprise_id = root_obj
        .and_then(|obj| json_string_field(obj, &["enterpriseId", "enterprise_id"]))
        .or_else(|| account_obj.and_then(|obj| json_string_field(obj, &["enterpriseId", "enterprise_id"])));
    let enterprise_name = root_obj
        .and_then(|obj| json_string_field(obj, &["enterpriseName", "enterprise_name"]))
        .or_else(|| account_obj.and_then(|obj| json_string_field(obj, &["enterpriseName", "enterprise_name"])));

    let refresh_token = root_obj
        .and_then(|obj| json_string_field(obj, &["refreshToken", "refresh_token"]))
        .or_else(|| auth_obj.and_then(|obj| json_string_field(obj, &["refreshToken", "refresh_token"])));
    let token_type = root_obj
        .and_then(|obj| json_string_field(obj, &["tokenType", "token_type"]))
        .or_else(|| auth_obj.and_then(|obj| json_string_field(obj, &["tokenType", "token_type"])))
        .or_else(|| Some("Bearer".to_string()));
    let domain = root_obj
        .and_then(|obj| json_string_field(obj, &["domain"]))
        .or_else(|| auth_obj.and_then(|obj| json_string_field(obj, &["domain"])));
    let expires_at = root_obj
        .and_then(|obj| json_i64_field(obj, &["expiresAt", "expires_at"]))
        .or_else(|| auth_obj.and_then(|obj| json_i64_field(obj, &["expiresAt", "expires_at"])));

    let identity_seed = uid
        .clone()
        .or_else(|| Some(email.clone()))
        .unwrap_or_else(|| format!("{}_user", platform.id_prefix()))
        .to_lowercase();
    let generated_id = format!("{}_{:x}", platform.id_prefix(), md5::compute(identity_seed.as_bytes()));

    BuddyAccount {
        id: generated_id,
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
        profile_raw: None,
        status: None,
        status_reason: None,
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: None,
        last_checkin_time: None,
        checkin_streak: 0,
        checkin_rewards: None,
        auth_raw: parsed_json,
        expiry_times: Default::default(),
        created_at: 0,
        last_used: 0,
    }
}

/// 从本机 WorkBuddy 客户端导入当前登录账号。
/// 优先读 auth 文件（新版）；无 auth 文件时回退读 state.vscdb secret（旧版）。
pub fn import_payload_from_local(platform: BuddyPlatform) -> Result<Option<BuddyAccount>, String> {
    // 1) 新版：auth 文件
    if let Some(auth_file) = default_auth_file_path(platform) {
        if auth_file.exists() && !logout_marker_path(&auth_file).exists() {
            let secret = fs::read_to_string(&auth_file)
                .map_err(|e| format!("读取本机 WorkBuddy 登录信息失败: {}", e))?;
            let parsed_json = serde_json::from_str::<serde_json::Value>(&secret).ok();
            let token_candidate = parsed_json
                .as_ref()
                .and_then(parse_local_access_token)
                .or_else(|| {
                    let raw = secret.trim();
                    if raw.is_empty() {
                        None
                    } else {
                        Some(raw.to_string())
                    }
                });
            if let Some(raw_token) = token_candidate {
                let (uid_from_token, normalized) = extract_local_token_parts(&raw_token)
                    .ok_or_else(|| "本地 WorkBuddy 登录信息解析失败: access token 无效".to_string())?;
                let uid_from_token = uid_from_token.or_else(|| extract_uid_from_jwt(&raw_token));
                let access_token = normalize_local_token(&normalized)
                    .ok_or_else(|| "本地 WorkBuddy 登录信息解析失败: access token 为空".to_string())?;
                return Ok(Some(build_account_from_local(
                    platform,
                    access_token,
                    parsed_json,
                    uid_from_token,
                )));
            }
        }
    }

    // 2) 旧版：state.vscdb secret
    //    仅 CN WorkBuddy 有该库；WorkBuddy AI 既无此库、其 secret 键名也未公开，
    //    故只走「读登录文件」这一条路（与参考实现一致）。
    if !matches!(platform, BuddyPlatform::Workbuddy) {
        return Ok(None);
    }
    let state_db = match default_state_db_path(platform) {
        Some(p) => p,
        None => return Ok(None),
    };
    if !state_db.exists() {
        return Ok(None);
    }
    let data_root = default_data_dir(platform).ok_or("无法定位 WorkBuddy 数据目录")?;
    let raw_secret = super::crypto::read_secret_storage_value(
        &state_db,
        SECRET_EXTENSION_ID,
        SECRET_KEY,
    )?;
    let Some(secret) = raw_secret else {
        return Ok(None);
    };
    let decrypted = super::crypto::decode_secret_storage_value(
        &secret,
        &data_root,
        Platform::WorkBuddy,
    )?;
    let parsed_json = serde_json::from_str::<serde_json::Value>(&decrypted).ok();
    let token_candidate = parsed_json
        .as_ref()
        .and_then(parse_local_access_token)
        .or_else(|| {
            let raw = decrypted.trim();
            if raw.is_empty() {
                None
            } else {
                Some(raw.to_string())
            }
        });
    let Some(raw_token) = token_candidate else {
        return Ok(None);
    };
    let (uid_from_token, normalized) = extract_local_token_parts(&raw_token)
        .ok_or_else(|| "本地 WorkBuddy 登录信息解析失败: access token 无效".to_string())?;
    let uid_from_token = uid_from_token.or_else(|| extract_uid_from_jwt(&raw_token));
    let access_token = normalize_local_token(&normalized)
        .ok_or_else(|| "本地 WorkBuddy 登录信息解析失败: access token 为空".to_string())?;
    Ok(Some(build_account_from_local(
        platform,
        access_token,
        parsed_json,
        uid_from_token,
    )))
}

/// 从登录文件原文构建 WorkBuddy 账号（供第三方导出导入复用，见 `third_party_import.rs`）。
///
/// 解析链与 `import_payload_from_local` 的 auth 文件分支一致，只是输入换成文本而非路径：
/// 先按 JSON 找 access token，找不到则把整段文本当作裸 token。
/// `uid_hint`（WorkDaddy 备份的 `uid`）只在文件内取不到 uid 时兜底。
pub(crate) fn build_account_from_auth_text(
    info: &str,
    uid_hint: Option<&str>,
) -> Result<Option<BuddyAccount>, String> {
    let trimmed = info.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let parsed_json = serde_json::from_str::<serde_json::Value>(trimmed).ok();
    let token_candidate = match parsed_json.as_ref() {
        Some(value) => parse_local_access_token(value),
        None => Some(trimmed.to_string()),
    };
    let Some(raw_token) = token_candidate else {
        return Ok(None);
    };
    let (uid_from_token, normalized) = extract_local_token_parts(&raw_token)
        .ok_or_else(|| "登录信息解析失败: access token 无效".to_string())?;
    let uid_from_token = uid_from_token
        .or_else(|| extract_uid_from_jwt(&raw_token))
        .or_else(|| {
            uid_hint
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        });
    let access_token = normalize_local_token(&normalized)
        .ok_or_else(|| "登录信息解析失败: access token 为空".to_string())?;
    // 第三方导出（WorkDaddy / cockpit）只覆盖 CN WorkBuddy 账号
    Ok(Some(build_account_from_local(
        BuddyPlatform::Workbuddy,
        access_token,
        parsed_json,
        uid_from_token,
    )))
}

/// 判断本机客户端当前使用哪个账号
pub fn resolve_current_account_id(
    platform: BuddyPlatform,
    accounts: &[BuddyAccount],
) -> Option<String> {
    if let Ok(Some(payload)) = import_payload_from_local(platform) {
        let incoming_uid = normalize_identity(payload.uid.as_deref());
        let incoming_email = normalize_email_identity(Some(payload.email.as_str()));
        if let Some(account_id) = accounts.iter().find(|account| {
            accounts_match(
                account.uid.as_ref(),
                normalize_email_identity(Some(account.email.as_str())).as_ref(),
                incoming_uid.as_ref(),
                incoming_email.as_ref(),
            )
        }) {
            return Some(account_id.id.clone());
        }
    }
    None
}

// ─── 切换：写回 auth 文件 ───

fn contains_encrypted_wrapper(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(obj) => {
            if obj.get("$wbEncrypted").is_some() {
                return true;
            }
            obj.values().any(contains_encrypted_wrapper)
        }
        serde_json::Value::Array(items) => items.iter().any(contains_encrypted_wrapper),
        _ => false,
    }
}

/// 构建写回客户端的 account 对象
fn build_auth_account_value(account: &BuddyAccount) -> serde_json::Value {
    let mut account_obj = account
        .auth_raw
        .as_ref()
        .and_then(|v| v.as_object())
        .and_then(|obj| obj.get("account").and_then(|v| v.as_object()))
        .cloned()
        .unwrap_or_default();

    if let Some(uid) = account.uid.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        account_obj.insert("uid".to_string(), serde_json::Value::String(uid.to_string()));
    }
    if let Some(nickname) = account.nickname.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        account_obj.insert("nickname".to_string(), serde_json::Value::String(nickname.to_string()));
    }
    if let Some(eid) = account.enterprise_id.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        account_obj.insert("enterpriseId".to_string(), serde_json::Value::String(eid.to_string()));
    }
    if let Some(ename) = account.enterprise_name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        account_obj.insert("enterpriseName".to_string(), serde_json::Value::String(ename.to_string()));
    }
    account_obj
        .entry("type".to_string())
        .or_insert_with(|| serde_json::Value::String("personal".to_string()));
    account_obj.insert("lastLogin".to_string(), serde_json::Value::Bool(true));
    account_obj.insert("pluginEnabled".to_string(), serde_json::Value::Bool(true));
    serde_json::Value::Object(account_obj)
}

/// 构建写回客户端的 auth 对象
fn build_auth_value(account: &BuddyAccount) -> serde_json::Value {
    let root_obj = account.auth_raw.as_ref().and_then(|v| v.as_object());
    let raw_auth_obj = root_obj.and_then(|obj| obj.get("auth").and_then(|v| v.as_object()));
    let mut auth_obj = raw_auth_obj
        .cloned()
        .or_else(|| {
            root_obj
                .filter(|obj| obj.contains_key("accessToken") || obj.contains_key("refreshToken"))
                .cloned()
        })
        .unwrap_or_default();

    let now_ms = chrono::Utc::now().timestamp_millis();
    let refresh_token = account.refresh_token.as_deref().unwrap_or("");
    let token_type = account.token_type.as_deref().unwrap_or("Bearer");
    let domain = account.domain.as_deref().unwrap_or("");

    auth_obj.insert("accessToken".to_string(), serde_json::Value::String(account.access_token.clone()));
    auth_obj.insert("refreshToken".to_string(), serde_json::Value::String(refresh_token.to_string()));
    auth_obj.insert("tokenType".to_string(), serde_json::Value::String(token_type.to_string()));
    auth_obj.insert("domain".to_string(), serde_json::Value::String(domain.to_string()));
    auth_obj.insert(
        "lastRefreshTime".to_string(),
        serde_json::Value::Number(serde_json::Number::from(now_ms)),
    );
    if let Some(expires_at) = account.expires_at {
        auth_obj.insert(
            "expiresAt".to_string(),
            serde_json::Value::Number(serde_json::Number::from(expires_at)),
        );
    }
    serde_json::Value::Object(auth_obj)
}

/// 构建完整登录会话 JSON（合并已有 accounts/allAccounts）
fn build_auth_session(account: &BuddyAccount, base_session: Option<&serde_json::Value>) -> serde_json::Value {
    let account_value = build_auth_account_value(account);
    let root_obj = base_session
        .and_then(serde_json::Value::as_object)
        .or_else(|| account.auth_raw.as_ref().and_then(serde_json::Value::as_object));
    let existing_accounts = root_obj
        .and_then(|obj| obj.get("accounts"))
        .and_then(serde_json::Value::as_array)
        .cloned();
    let existing_all_accounts = root_obj
        .and_then(|obj| obj.get("allAccounts"))
        .and_then(serde_json::Value::as_array)
        .cloned();

    let merge_current = |items: Option<Vec<serde_json::Value>>| {
        let target_uid = account_value.get("uid").and_then(serde_json::Value::as_str);
        let mut found = false;
        let mut merged = items
            .unwrap_or_default()
            .into_iter()
            .map(|mut item| {
                let matches = target_uid.is_some()
                    && item.get("uid").and_then(serde_json::Value::as_str) == target_uid;
                if matches {
                    found = true;
                    return account_value.clone();
                }
                if let Some(obj) = item.as_object_mut() {
                    obj.insert("lastLogin".to_string(), serde_json::Value::Bool(false));
                }
                item
            })
            .collect::<Vec<_>>();
        if !found {
            merged.push(account_value.clone());
        }
        merged
    };

    let accounts = merge_current(existing_accounts);
    let all_accounts = merge_current(existing_all_accounts);
    let mut session = root_obj.cloned().unwrap_or_default();
    session.insert("account".to_string(), account_value);
    session.insert("auth".to_string(), build_auth_value(account));
    session.insert("accounts".to_string(), serde_json::Value::Array(accounts));
    session.insert("allAccounts".to_string(), serde_json::Value::Array(all_accounts));
    serde_json::Value::Object(session)
}

/// 切换默认客户端到指定账号（写回 auth 文件；无文件时按 account 重建）。
/// 返回写入后的校验信息。
pub fn write_account_to_default_client(
    platform: BuddyPlatform,
    account: &BuddyAccount,
) -> Result<String, String> {
    let auth_file = default_auth_file_path(platform)
        .ok_or_else(|| "无法定位默认 WorkBuddy 登录信息路径".to_string())?;

    if let Some(raw) = account.auth_raw.as_ref() {
        if contains_encrypted_wrapper(raw) {
            return Err(
                "当前 WorkBuddy 登录文件包含官方加密字段，未取得官方密钥，已停止覆盖以避免破坏登录状态"
                    .to_string(),
            );
        }
    }

    let marker_path = logout_marker_path(&auth_file);
    let marker_hash_before: Option<[u8; 32]> =
        fs::read(&marker_path).ok().map(|bytes| Sha256::digest(&bytes).into());

    if auth_file.exists() {
        let existing = fs::read(&auth_file)
            .map_err(|e| format!("读取现有 WorkBuddy 登录信息失败: {}", e))?;
        let existing_json: serde_json::Value = serde_json::from_slice(&existing)
            .map_err(|e| format!("现有 WorkBuddy 登录信息不是有效 JSON，已停止覆盖: {}", e))?;
        if contains_encrypted_wrapper(&existing_json) {
            return Err(
                "当前 WorkBuddy 登录文件包含官方加密字段，未取得官方密钥，已停止覆盖以避免破坏登录状态"
                    .to_string(),
            );
        }
        let expected_hash: [u8; 32] = Sha256::digest(&existing).into();
        let session = build_auth_session(account, Some(&existing_json));
        let content = serde_json::to_string_pretty(&session)
            .map_err(|e| format!("序列化登录信息失败: {}", e))?;
        let written = store::write_atomic_if_hash_matches(&auth_file, &expected_hash, &content)?;
        if !written {
            return Err(
                "WorkBuddy 登录信息在切号期间被官方客户端更新，已停止覆盖，请重试".to_string(),
            );
        }
    } else {
        let session = build_auth_session(account, None);
        let content = serde_json::to_string_pretty(&session)
            .map_err(|e| format!("序列化登录信息失败: {}", e))?;
        store::write_atomic(&auth_file, &content)?;
    }

    if let Some(expected_hash) = marker_hash_before {
        let _ = store::remove_file_if_hash_matches(&marker_path, &expected_hash);
    }

    // 校验写回结果
    let written = fs::read_to_string(&auth_file)
        .map_err(|e| format!("校验 WorkBuddy 登录信息失败: {}", e))?;
    let written_json: serde_json::Value = serde_json::from_str(&written)
        .map_err(|e| format!("校验 WorkBuddy 登录信息 JSON 失败: {}", e))?;
    let written_token = written_json
        .get("auth")
        .and_then(|auth| auth.get("accessToken"))
        .and_then(|v| v.as_str());
    if written_token != Some(account.access_token.as_str()) {
        return Err(format!(
            "校验 WorkBuddy 登录信息失败，未写入目标账号: {}",
            auth_file.display()
        ));
    }
    for key in ["account", "auth", "accounts", "allAccounts"] {
        if written_json.get(key).is_none() {
            return Err(format!("校验 WorkBuddy 登录信息失败，缺少官方字段 {}", key));
        }
    }
    Ok(format!(
        "已切换到 {}（{}）",
        account.display_name(),
        auth_file.display()
    ))
}

/// 公开：加载账号并切换
pub fn switch_account(platform: BuddyPlatform, account_id: &str) -> Result<String, String> {
    let account = store::load_account(platform, account_id)
        .ok_or_else(|| format!("{} 账号不存在: {}", platform.as_str(), account_id))?;
    let result = write_account_to_default_client(platform, &account)?;
    // 更新 last_used
    let mut updated = account;
    updated.last_used = chrono::Utc::now().timestamp();
    let _ = store::upsert_account(platform, updated);
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;

    fn sample_account() -> BuddyAccount {
        BuddyAccount {
            id: "workbuddy_abc".to_string(),
            platform: "workbuddy".to_string(),
            email: "user@example.com".to_string(),
            uid: Some("u123".to_string()),
            nickname: Some("Alice".to_string()),
            enterprise_id: None,
            enterprise_name: None,
            tags: None,
            access_token: "wb-token-1".to_string(),
            refresh_token: Some("wb-refresh-1".to_string()),
            token_type: Some("Bearer".to_string()),
            expires_at: Some(1770000000),
            domain: Some("example.com".to_string()),
            plan_type: None,
            dosage_notify_code: None,
            dosage_notify_zh: None,
            dosage_notify_en: None,
            payment_type: None,
            quota_raw: None,
            usage_raw: None,
            profile_raw: None,
            status: None,
            status_reason: None,
            quota_query_last_error: None,
            quota_query_last_error_at: None,
            usage_updated_at: None,
            last_checkin_time: None,
            checkin_streak: 0,
            checkin_rewards: None,
            auth_raw: None,
            expiry_times: Default::default(),
            created_at: 0,
            last_used: 0,
        }
    }

    #[test]
    fn session_contains_official_fields_and_current_account() {
        let session = build_auth_session(&sample_account(), None);
        let obj = session.as_object().unwrap();
        for key in ["account", "auth", "accounts", "allAccounts"] {
            assert!(obj.contains_key(key), "缺少官方字段 {}", key);
        }
        let account = obj["account"].as_object().unwrap();
        assert_eq!(account["uid"].as_str(), Some("u123"));
        assert_eq!(account["nickname"].as_str(), Some("Alice"));
        assert_eq!(account["lastLogin"].as_bool(), Some(true));
        let auth = obj["auth"].as_object().unwrap();
        assert_eq!(auth["accessToken"].as_str(), Some("wb-token-1"));
        assert_eq!(auth["refreshToken"].as_str(), Some("wb-refresh-1"));
        // accounts 数组中包含目标账号且标记 lastLogin
        let accounts = obj["accounts"].as_array().unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0]["uid"].as_str(), Some("u123"));
    }

    #[test]
    fn session_merges_existing_accounts_and_marks_others_not_login() {
        let base = serde_json::json!({
            "accounts": [
                {"uid": "other1", "nickname": "Bob", "lastLogin": true},
                {"uid": "u123", "nickname": "OldAlice", "lastLogin": false}
            ]
        });
        let session = build_auth_session(&sample_account(), Some(&base));
        let obj = session.as_object().unwrap();
        let accounts = obj["accounts"].as_array().unwrap();
        assert_eq!(accounts.len(), 2);
        // 目标账号被替换为最新信息
        let target = accounts
            .iter()
            .find(|a| a["uid"].as_str() == Some("u123"))
            .unwrap();
        assert_eq!(target["nickname"].as_str(), Some("Alice"));
        // 其它账号 lastLogin 置 false
        let other = accounts
            .iter()
            .find(|a| a["uid"].as_str() == Some("other1"))
            .unwrap();
        assert_eq!(other["lastLogin"].as_bool(), Some(false));
    }

    #[test]
    fn encrypted_wrapper_detection() {
        assert!(contains_encrypted_wrapper(&serde_json::json!({"$wbEncrypted": true})));
        assert!(contains_encrypted_wrapper(&serde_json::json!({"nested": {"a": {"$wbEncrypted": 1}}})));
        assert!(contains_encrypted_wrapper(&serde_json::json!([{"$wbEncrypted": "x"}])));
        assert!(!contains_encrypted_wrapper(&serde_json::json!({"account": {"uid": "1"}})));
    }

    #[test]
    fn token_parts_extraction() {
        assert_eq!(
            extract_local_token_parts("u123+abc.def").unwrap(),
            (Some("u123".to_string()), "abc.def".to_string())
        );
        assert_eq!(
            extract_local_token_parts("plain-token").unwrap(),
            (None, "plain-token".to_string())
        );
        assert_eq!(normalize_local_token("u123+abc").unwrap(), "abc");
        assert_eq!(normalize_local_token("abc").unwrap(), "abc");
    }

    #[test]
    fn jwt_sub_extraction() {
        // header.payload.signature，payload 含 sub
        let payload = general_purpose::URL_SAFE_NO_PAD.encode(r#"{"sub":"u999"}"#);
        let token = format!("eyJhbGciOiJIUzI1NiJ9.{}.sig", payload);
        assert_eq!(extract_uid_from_jwt(&token).unwrap(), "u999");
        assert_eq!(extract_uid_from_jwt("not-a-jwt"), None);
    }
}
