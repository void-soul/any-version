//! CodeBuddy CN 平台：本地导入 + 切换账号。
//!
//! 移植自 cockpit-tools `codebuddy_cn_account.rs`（src-tauri 版）：
//! - 本地登录态存于 `%APPDATA%/CodeBuddy CN/User/globalStorage/state.vscdb` 的
//!   `secret://{"extensionId":"tencent-cloud.coding-copilot","key":"planning-genie.new.accessTokencn"}`
//! - 切换 = 用目标账号构建会话 JSON，经 safe storage 加密后写回 state.vscdb。

use std::path::PathBuf;

use super::crypto::Platform;
use super::models::{BuddyAccount, BuddyPlatform};
use super::store;

const SECRET_EXTENSION_ID: &str = "tencent-cloud.coding-copilot";
const SECRET_KEY: &str = "planning-genie.new.accessTokencn";

fn secret_db_key() -> String {
    format!(
        r#"secret://{{"extensionId":"{}","key":"{}"}}"#,
        SECRET_EXTENSION_ID, SECRET_KEY
    )
}

/// CodeBuddy CN 数据根目录
pub fn default_data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        return dirs::data_dir().map(|d| d.join("CodeBuddy CN"));
    }
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir()?;
        return Some(home.join("Library/Application Support/CodeBuddy CN"));
    }
    #[cfg(target_os = "linux")]
    {
        return dirs::config_dir().map(|d| d.join("CodeBuddy CN"));
    }
    #[allow(unreachable_code)]
    None
}

pub fn default_state_db_path() -> Option<PathBuf> {
    let data_dir = default_data_dir()?;
    // 复刻 cockpit-tools ensure_codebuddy_cn_state_db_path 的候选顺序
    let candidates = [
        data_dir.join("User").join("globalStorage").join("state.vscdb"),
        data_dir.join("globalStorage").join("state.vscdb"),
        data_dir.join("state.vscdb"),
    ];
    Some(
        candidates
            .iter()
            .find(|path| path.exists())
            .cloned()
            .unwrap_or_else(|| candidates[0].clone()),
    )
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

fn build_account_from_local(
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
        .unwrap_or_else(|| "codebuddy_cn_user".to_string())
        .to_lowercase();
    let generated_id = format!("codebuddy_cn_{:x}", md5::compute(identity_seed.as_bytes()));

    BuddyAccount {
        id: generated_id,
        platform: "codebuddy-cn".to_string(),
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

/// 从本机 CodeBuddy CN 客户端导入当前登录账号（state.vscdb secret）。
pub fn import_payload_from_local() -> Result<Option<BuddyAccount>, String> {
    let state_db = match default_state_db_path() {
        Some(p) => p,
        None => return Ok(None),
    };
    if !state_db.exists() {
        return Ok(None);
    }
    let data_root = default_data_dir().ok_or("无法定位 CodeBuddy CN 数据目录")?;
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
        Platform::CodeBuddyCn,
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
        return Err("本地 CodeBuddy CN 登录信息解析失败: 未找到 access token".to_string());
    };
    let (uid_from_token, normalized) = extract_local_token_parts(&raw_token)
        .ok_or_else(|| "本地 CodeBuddy CN 登录信息解析失败: access token 无效".to_string())?;
    let access_token = normalize_local_token(&normalized)
        .ok_or_else(|| "本地 CodeBuddy CN 登录信息解析失败: access token 为空".to_string())?;
    Ok(Some(build_account_from_local(
        access_token,
        parsed_json,
        uid_from_token,
    )))
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

/// 判断本机客户端当前使用哪个账号
pub fn resolve_current_account_id(accounts: &[BuddyAccount]) -> Option<String> {
    if let Ok(Some(payload)) = import_payload_from_local() {
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

/// 构建写回客户端的会话 JSON
fn build_default_client_session_json(account: &BuddyAccount) -> String {
    let uid = account.uid.as_deref().unwrap_or("");
    let nickname = account.nickname.as_deref().unwrap_or("");
    let enterprise_id = account.enterprise_id.as_deref().unwrap_or("");
    let enterprise_name = account.enterprise_name.as_deref().unwrap_or("");
    let domain = account.domain.as_deref().unwrap_or("");
    let refresh_token = account.refresh_token.as_deref().unwrap_or("");
    let expires_at = account.expires_at.unwrap_or(0);

    serde_json::json!({
        "id": "Tencent-Cloud.genie-ide-cn",
        "token": account.access_token,
        "refreshToken": refresh_token,
        "expiresAt": expires_at,
        "domain": domain,
        "accessToken": format!("{}+{}", uid, account.access_token),
        "converted": true,
        "account": {
            "id": uid,
            "uid": uid,
            "label": nickname,
            "nickname": nickname,
            "enterpriseId": enterprise_id,
            "enterpriseName": enterprise_name,
            "pluginEnabled": true,
            "lastLogin": true,
        },
        "auth": {
            "accessToken": account.access_token,
            "refreshToken": refresh_token,
            "tokenType": account.token_type.as_deref().unwrap_or("Bearer"),
            "domain": domain,
            "expiresAt": expires_at,
            "expiresIn": expires_at,
            "refreshExpiresIn": 0,
            "refreshExpiresAt": 0,
            "lastRefreshTime": chrono::Utc::now().timestamp_millis(),
        }
    })
    .to_string()
}

/// 注入失败时补充可操作的提示（复刻 cockpit-tools 的 friendly error）
fn friendly_inject_error(err: &str) -> String {
    if err.contains("Safe Storage")
        || err.contains("Local State")
        || err.contains("Keychain")
        || err.contains("safe_storage")
        || err.contains("解密")
        || err.contains("密钥")
    {
        format!(
            "注入登录状态失败：{}\n\n可能的原因：\n1. CodeBuddy CN 从未登录过，请先手动打开 CodeBuddy CN 并登录一次\n2. 系统密钥环（macOS Keychain / Local State）中缺少加密密钥条目\n\n请尝试：打开 CodeBuddy CN → 登录任意账号 → 退出 → 再使用切号功能",
            err
        )
    } else {
        err.to_string()
    }
}

/// 注入后校验：state.vscdb 中目标 key 必须存在且非空（复刻 verify_state_db_injection）
fn verify_state_db_injection(state_db_path: &std::path::Path, db_key: &str) -> Result<(), String> {
    let conn = rusqlite::Connection::open(state_db_path)
        .map_err(|e| format!("注入校验失败，无法打开 state.vscdb: {}", e))?;
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = ?1",
            [db_key],
            |row| row.get(0),
        )
        .ok();
    match value {
        Some(stored) if !stored.trim().is_empty() => Ok(()),
        _ => Err(format!(
            "注入校验失败，未在 state.vscdb 找到目标 key: db={}, key={}",
            state_db_path.display(),
            db_key
        )),
    }
}

/// 切换默认客户端到指定账号（写回 state.vscdb）。
pub fn write_account_to_default_client(account: &BuddyAccount) -> Result<String, String> {
    let state_db = default_state_db_path()
        .ok_or_else(|| "无法定位默认 CodeBuddy CN state.vscdb 路径".to_string())?;
    if let Some(parent) = state_db.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建 globalStorage 目录失败: {}", e))?;
    }
    let data_root = default_data_dir().ok_or("无法定位 CodeBuddy CN 数据目录")?;
    let db_key = secret_db_key();
    let session_json = build_default_client_session_json(account);
    if let Err(err) = super::crypto::inject_secret_to_state_db(
        &state_db,
        &db_key,
        &session_json,
        &data_root,
        Platform::CodeBuddyCn,
    ) {
        return Err(friendly_inject_error(&err));
    }
    verify_state_db_injection(&state_db, &db_key)?;
    Ok(format!(
        "已切换到 {}（{}）",
        account.display_name(),
        state_db.display()
    ))
}

/// 公开：加载账号并切换
pub fn switch_account(account_id: &str) -> Result<String, String> {
    let account = store::load_account(BuddyPlatform::CodebuddyCn, account_id)
        .ok_or_else(|| format!("CodeBuddy CN 账号不存在: {}", account_id))?;
    let result = write_account_to_default_client(&account)?;
    let mut updated = account;
    updated.last_used = chrono::Utc::now().timestamp();
    let _ = store::upsert_account(BuddyPlatform::CodebuddyCn, updated);
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;

    fn sample_account() -> BuddyAccount {
        BuddyAccount {
            id: "codebuddy_cn_abc".to_string(),
            platform: "codebuddy-cn".to_string(),
            email: "cn@example.com".to_string(),
            uid: Some("cn123".to_string()),
            nickname: Some("小王".to_string()),
            enterprise_id: Some("e1".to_string()),
            enterprise_name: Some("腾讯云".to_string()),
            tags: None,
            access_token: "cn-token-1".to_string(),
            refresh_token: Some("cn-refresh-1".to_string()),
            token_type: Some("Bearer".to_string()),
            expires_at: Some(1770000000),
            domain: Some("cloud.tencent.com".to_string()),
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
    fn session_json_contains_expected_shape() {
        let json = build_default_client_session_json(&sample_account());
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["id"].as_str(), Some("Tencent-Cloud.genie-ide-cn"));
        assert_eq!(v["token"].as_str(), Some("cn-token-1"));
        assert_eq!(v["accessToken"].as_str(), Some("cn123+cn-token-1"));
        assert_eq!(v["converted"].as_bool(), Some(true));
        let account = v["account"].as_object().unwrap();
        assert_eq!(account["uid"].as_str(), Some("cn123"));
        assert_eq!(account["nickname"].as_str(), Some("小王"));
        assert_eq!(account["lastLogin"].as_bool(), Some(true));
        let auth = v["auth"].as_object().unwrap();
        assert_eq!(auth["accessToken"].as_str(), Some("cn-token-1"));
        assert_eq!(auth["refreshToken"].as_str(), Some("cn-refresh-1"));
    }

    #[test]
    fn token_parts_and_normalize() {
        assert_eq!(
            extract_local_token_parts("cn123+abc").unwrap(),
            (Some("cn123".to_string()), "abc".to_string())
        );
        assert_eq!(normalize_local_token("cn123+abc").unwrap(), "abc");
        assert_eq!(normalize_local_token("plain").unwrap(), "plain");
    }

    #[test]
    fn verify_state_db_injection_checks_target_key() {
        let dir = std::env::temp_dir().join(format!(
            "kira-cn-verify-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("state.vscdb");
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value TEXT);
                 INSERT INTO ItemTable VALUES ('secret://k', 'v1');
                 INSERT INTO ItemTable VALUES ('secret://empty', '  ');",
            )
            .unwrap();
        }
        assert!(verify_state_db_injection(&db, "secret://k").is_ok());
        assert!(verify_state_db_injection(&db, "secret://empty").is_err());
        assert!(verify_state_db_injection(&db, "secret://missing").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn friendly_inject_error_appends_hints() {
        let friendly = friendly_inject_error("读取 Local State 失败: 系统找不到指定的文件");
        assert!(friendly.contains("从未登录过"));
        assert_eq!(friendly_inject_error("disk full"), "disk full");
    }
}
