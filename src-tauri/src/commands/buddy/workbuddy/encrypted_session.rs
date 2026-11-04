//! encrypted session responsibility.
use super::*;

// ─── 切换：写回 auth 文件 ───

pub(super) fn contains_encrypted_wrapper(value: &serde_json::Value) -> bool {
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
pub(super) fn build_auth_account_value(account: &BuddyAccount) -> serde_json::Value {
    let mut account_obj = account
        .auth_raw
        .as_ref()
        .and_then(|v| v.as_object())
        .and_then(|obj| obj.get("account").and_then(|v| v.as_object()))
        .cloned()
        .unwrap_or_default();

    if let Some(uid) = account
        .uid
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        account_obj.insert(
            "uid".to_string(),
            serde_json::Value::String(uid.to_string()),
        );
    }
    if let Some(nickname) = account
        .nickname
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        account_obj.insert(
            "nickname".to_string(),
            serde_json::Value::String(nickname.to_string()),
        );
    }
    if let Some(eid) = account
        .enterprise_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        account_obj.insert(
            "enterpriseId".to_string(),
            serde_json::Value::String(eid.to_string()),
        );
    }
    if let Some(ename) = account
        .enterprise_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        account_obj.insert(
            "enterpriseName".to_string(),
            serde_json::Value::String(ename.to_string()),
        );
    }
    account_obj
        .entry("type".to_string())
        .or_insert_with(|| serde_json::Value::String("personal".to_string()));
    account_obj.insert("lastLogin".to_string(), serde_json::Value::Bool(true));
    account_obj.insert("pluginEnabled".to_string(), serde_json::Value::Bool(true));
    serde_json::Value::Object(account_obj)
}

/// 构建写回客户端的 auth 对象
pub(super) fn build_auth_value(
    account: &BuddyAccount,
    platform: BuddyPlatform,
) -> serde_json::Value {
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
    // `auth.domain` 写**客户端自己使用的域**，不是账号的域。
    // 2026-10-18/19 实验：客户端拿老域账号（DEDEDE, iss=codebuddy.cn）切进来时，
    // 若这里沿用账号域，登录态里就是 `www.codebuddy.cn`，而 WorkBuddy 5.8.x 的 endpoint
    // 是 `www.workbuddy.cn` —— 客户端照着这个域发请求，网关层直接 401（响应是 HTML）。
    // 域是谁的与 token 是谁的有解耦意义，这里必须是"客户端的域"。
    let domain = super::super::api::client_auth_host(platform);

    auth_obj.insert(
        "accessToken".to_string(),
        serde_json::Value::String(account.access_token.clone()),
    );
    auth_obj.insert(
        "refreshToken".to_string(),
        serde_json::Value::String(refresh_token.to_string()),
    );
    auth_obj.insert(
        "tokenType".to_string(),
        serde_json::Value::String(token_type.to_string()),
    );
    auth_obj.insert(
        "domain".to_string(),
        serde_json::Value::String(domain.to_string()),
    );
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
pub(super) fn build_auth_session(
    account: &BuddyAccount,
    platform: BuddyPlatform,
    base_session: Option<&serde_json::Value>,
) -> serde_json::Value {
    let account_value = build_auth_account_value(account);
    let root_obj = base_session
        .and_then(serde_json::Value::as_object)
        .or_else(|| {
            account
                .auth_raw
                .as_ref()
                .and_then(serde_json::Value::as_object)
        });
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
    session.insert("auth".to_string(), build_auth_value(account, platform));
    session.insert("accounts".to_string(), serde_json::Value::Array(accounts));
    session.insert(
        "allAccounts".to_string(),
        serde_json::Value::Array(all_accounts),
    );
    serde_json::Value::Object(session)
}

/// Old encrypted paths describe the old identity and array positions, not the
/// new session. Reject unknown fields before rebuilding, then seal actual fields.
pub(super) fn prepare_switched_session(
    account: &BuddyAccount,
    platform: BuddyPlatform,
    existing: &serde_json::Value,
    secret: &str,
) -> Result<serde_json::Value, String> {
    use super::super::twoapi::atrest;
    let (plain, paths) = atrest::decrypt_tree_with_paths(existing, secret)?;
    for path in paths {
        let parts: Vec<_> = path.split('.').collect();
        let known = match parts.as_slice() {
            ["auth", field] => matches!(*field, "accessToken" | "refreshToken"),
            ["account", field] => matches!(*field, "phoneNumber" | "departmentFullName" | "nickname"),
            ["accounts" | "allAccounts", index, field] => index.parse::<usize>().is_ok()
                && matches!(*field, "phoneNumber" | "departmentFullName" | "nickname"),
            _ => false,
        };
        if !known { return Err("WorkBuddy 登录信息包含未知加密字段，已停止覆盖，请更新客户端适配".into()); }
    }
    let session = build_auth_session(account, platform, Some(&plain));
    atrest::encrypt_session_fields(&session, secret)
}

/// 读取当前 WorkBuddy 登录文件里的 access token（延时复查用）。
///
/// 登录文件不存在 / 不是 JSON / 取不到字段 → `Ok(None)`（登录态确实不在了）。
/// **取不到官方密钥** → `Err`（探测不可用，调用方不应据此判定切号失败）。
pub(crate) fn read_session_access_token() -> Result<Option<String>, String> {
    use super::super::twoapi::atrest;
    let auth_file = default_auth_file_path(BuddyPlatform::Workbuddy)
        .ok_or_else(|| "无法定位 WorkBuddy 登录信息路径".to_string())?;
    if !auth_file.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(&auth_file)
        .map_err(|e| format!("读取 WorkBuddy 登录信息失败: {}", e))?;
    let json: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("WorkBuddy 登录信息不是有效 JSON: {}", e))?;
    let field = match json.get("auth").and_then(|a| a.get("accessToken")) {
        Some(f) => f.clone(),
        None => return Ok(None),
    };
    let secret = atrest::fetch_secret()?;
    let token = atrest::decrypt_field(&field, &secret)?;
    if token.is_empty() {
        Ok(None)
    } else {
        Ok(Some(token))
    }
}

pub(super) fn backup_auth_file(auth_file: &std::path::Path) {
    if !auth_file.exists() {
        return;
    }
    let mut backup = auth_file.as_os_str().to_os_string();
    backup.push(format!(
        ".kira-switch-{}.bak",
        chrono::Utc::now().timestamp()
    ));
    if let Err(e) = fs::copy(auth_file, std::path::PathBuf::from(backup)) {
        eprintln!("[buddy] 备份 WorkBuddy 登录信息失败（继续切号）: {e}");
    }
}
