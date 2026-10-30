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
/// 仅 CN 版有该目录，其余平台返回 None（调用方据此跳过会话/secret 相关分支）。
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

/// 客户端是否仍带着「已登出」标记（`<AUTH_FILE>.logged-out`）。
///
/// 标记在时 WorkBuddy **完全不读登录文件**（= 打开客户端就是登录页），
/// 所以它既让导入/识别失效，也能让一次写回成功的切号"看起来没生效"。
pub fn logout_marker_present() -> bool {
    default_auth_file_path(BuddyPlatform::Workbuddy)
        .map(|file| logout_marker_path(&file).exists())
        .unwrap_or(false)
}

/// 切号写回成功后"退休"登录退出标记（对齐 WorkDaddy `retireLogoutMarker`）。
///
/// 语义与参考实现一致的两点：
/// 1. 标记在 → 客户端忽略登录文件，所以写回成功后**必须**清掉，否则用户看到"切了号却要我重新登录"；
/// 2. 清不掉要**明确报错**（参考侧 rename/删除失败也是 throw），不能 `let _ =` 静默成功 ——
///    静默成功比失败更糟：用户以为切好了，实际客户端根本不认这份登录态。
///
/// 与参考侧的差异（我们有"客户端必须已退出"这个前提）：按标记**来源**分两种
/// - `existed_before = true`：上次登出的遗留标记，内容变了也无所谓（能走到这里说明登录文件
///   是我们刚写好的，任何残留标记都只会让客户端不认它）→ 直接删；
/// - `existed_before = false`：写回之后才冒出来的标记 = 客户端在切号窗口里又登出了
///   （说明它其实没退干净）→ **保留**并报错，不吞掉用户/客户端的登出动作。
fn retire_logout_marker(marker_path: &std::path::Path, existed_before: bool) -> Result<(), String> {
    if !marker_path.exists() {
        return Ok(());
    }
    if !existed_before {
        return Err(format!(
            "登录态已写入，但客户端在切号期间重新生成了登录退出标记（{}）：\
             客户端只在完全退出后才不会覆盖登录态，请彻底退出（含托盘）后重试",
            marker_path.display()
        ));
    }
    match fs::remove_file(marker_path) {
        Ok(()) => {}
        Err(first) => {
            // 客户端托盘/杀软偶发占用 → 小睡后重试一次，仍失败就如实报错
            std::thread::sleep(std::time::Duration::from_millis(120));
            fs::remove_file(marker_path).map_err(|second| {
                format!(
                    "登录态已写入，但清理登录退出标记失败（{}）：先 {} 后 {}；\
                     该标记存在时客户端会忽略登录态，会提示重新登录，请彻底退出客户端后重试",
                    marker_path.display(),
                    first,
                    second
                )
            })?;
        }
    }
    if marker_path.exists() {
        return Err(format!(
            "登录态已写入，但登录退出标记仍然存在（{}）：客户端会忽略登录态并提示重新登录，\
             请彻底退出客户端后重试",
            marker_path.display()
        ));
    }
    Ok(())
}

fn json_string_field(
    obj: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<String> {
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
        .or_else(|| {
            account_obj.and_then(|obj| json_string_field(obj, &["enterpriseId", "enterprise_id"]))
        });
    let enterprise_name = root_obj
        .and_then(|obj| json_string_field(obj, &["enterpriseName", "enterprise_name"]))
        .or_else(|| {
            account_obj
                .and_then(|obj| json_string_field(obj, &["enterpriseName", "enterprise_name"]))
        });

    let refresh_token = root_obj
        .and_then(|obj| json_string_field(obj, &["refreshToken", "refresh_token"]))
        .or_else(|| {
            auth_obj.and_then(|obj| json_string_field(obj, &["refreshToken", "refresh_token"]))
        });
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
    let generated_id = format!(
        "{}_{:x}",
        platform.id_prefix(),
        md5::compute(identity_seed.as_bytes())
    );

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
                    .ok_or_else(|| {
                        "本地 WorkBuddy 登录信息解析失败: access token 无效".to_string()
                    })?;
                let uid_from_token = uid_from_token.or_else(|| extract_uid_from_jwt(&raw_token));
                let access_token = normalize_local_token(&normalized).ok_or_else(|| {
                    "本地 WorkBuddy 登录信息解析失败: access token 为空".to_string()
                })?;
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
    //    仅 CN WorkBuddy 有该库，故只走「读登录文件」这一条路（与参考实现一致）。
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
    let raw_secret =
        super::crypto::read_secret_storage_value(&state_db, SECRET_EXTENSION_ID, SECRET_KEY)?;
    let Some(secret) = raw_secret else {
        return Ok(None);
    };
    let decrypted =
        super::crypto::decode_secret_storage_value(&secret, &data_root, Platform::WorkBuddy)?;
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

/// 公开：加载账号并切换
pub fn switch_account(platform: BuddyPlatform, account_id: &str) -> Result<String, String> {
    let account = store::load_account(platform, account_id)
        .ok_or_else(|| format!("{} 账号不存在: {}", platform.as_str(), account_id))?;
    let result = write_account_to_default_client(platform, &account)?;
    // 更新 last_used
    let mut updated = account;
    updated.last_used = chrono::Utc::now().timestamp();
    let _ = store::upsert_account(platform, updated);
    // 2API 与 Buddy 读同一个登录态文件：切号后立刻把新鉴权头热更新进正在运行的
    // 2API 服务，否则它会继续用旧账号的凭据（要等到 token 过期才暴露）。
    // 放后台任务：读凭据可能要调 WorkBuddy 的 Electron 取密钥，不能挡住切号 UI；
    // 同进程调用、无竞态；服务没在跑时静默跳过。
    if platform.is_workbuddy_family() {
        tauri::async_runtime::spawn(async move {
            if let Err(e) = crate::commands::buddy::twoapi::on_account_switched().await {
                eprintln!("[buddy] 2API 凭据同步失败：{e}");
                // 记进服务状态：只打日志的话用户看不到，而服务会继续用旧账号发请求
                crate::commands::buddy::twoapi::note_sync_error(&e);
            }
        });
    }
    Ok(result)
}

mod encrypted_session;
pub use encrypted_session::*;

#[cfg(test)]
mod tests;
