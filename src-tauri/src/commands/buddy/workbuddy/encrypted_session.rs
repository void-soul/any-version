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

/// 官方 5.6 加密格式的切号写回：解密 → 换目标账号 → 原样加密 → 原子写回。
///
/// 三重保护，任一环节不满足就**不写**：
/// 1. 拿不到官方密钥 → 拒绝（写明文会破坏登录态）
/// 2. 重新加密时若某条路径在重建后的结构里不存在 → 拒绝（宁可切不了号）
/// 3. 写回用 hash 比对（防切号期间被官方客户端改过）+ 写后校验加密包装仍在
pub(super) fn write_encrypted_session(
    account: &BuddyAccount,
    platform: BuddyPlatform,
    auth_file: &std::path::PathBuf,
    existing: &serde_json::Value,
    expected_hash: &[u8; 32],
    marker_path: &std::path::PathBuf,
    marker_existed_before: bool,
) -> Result<String, String> {
    use super::super::twoapi::atrest;
    let secret = atrest::fetch_secret().map_err(|e| {
        format!("无法取得 WorkBuddy 官方加密密钥，已停止覆盖以避免破坏登录状态：{e}")
    })?;
    // 1. 整棵解密，并记下哪些字段原本是加密的
    let (plain_existing, paths) = atrest::decrypt_tree_with_paths(existing, &secret)?;
    // 2. 用目标账号的凭据重建 session（沿用官方其余字段，如 domain / accounts / allAccounts）
    let session = build_auth_session(account, platform, Some(&plain_existing));
    // 3. 按原路径重新加密；路径失效会直接报错，不会写出半加密文件
    let encrypted = atrest::encrypt_tree(&session, &paths, &secret)?;
    let content = serde_json::to_string_pretty(&encrypted)
        .map_err(|e| format!("序列化登录信息失败: {}", e))?;
    // 4. 原子写回（hash 比对防并发覆盖）
    if !store::write_atomic_if_hash_matches(auth_file, expected_hash, &content)? {
        return Err("WorkBuddy 登录信息在切号期间被官方客户端更新，已停止覆盖，请重试".to_string());
    }
    // 5. 写后校验：加密包装必须还在（否则等于把登录态写坏了）
    let written_text =
        fs::read_to_string(auth_file).map_err(|e| format!("校验 WorkBuddy 登录信息失败: {e}"))?;
    let written: serde_json::Value = serde_json::from_str(&written_text)
        .map_err(|e| format!("校验 WorkBuddy 登录信息 JSON 失败: {e}"))?;
    if !contains_encrypted_wrapper(&written) {
        return Err(
            "写回结果丢失了官方加密包装，WorkBuddy 登录态可能已损坏，请重新登录 WorkBuddy"
                .to_string(),
        );
    }
    retire_logout_marker(marker_path, marker_existed_before)?;
    Ok(format!(
        "已切换到 {}（{}，官方加密字段已原样保留）",
        account.display_name(),
        auth_file.display()
    ))
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

/// 切换默认客户端到指定账号（写回 auth 文件；无文件时按 account 重建）。
/// 返回写入后的校验信息。
pub fn write_account_to_default_client(
    platform: BuddyPlatform,
    account: &BuddyAccount,
) -> Result<String, String> {
    let auth_file = default_auth_file_path(platform)
        .ok_or_else(|| "无法定位默认 WorkBuddy 登录信息路径".to_string())?;

    let marker_path = logout_marker_path(&auth_file);
    // 只记「写回前标记在不在」：内容变化不阻止退休（见 retire_logout_marker 的分支说明）
    let marker_existed_before = marker_path.exists();

    if auth_file.exists() {
        let existing =
            fs::read(&auth_file).map_err(|e| format!("读取现有 WorkBuddy 登录信息失败: {}", e))?;
        let existing_json: serde_json::Value = serde_json::from_slice(&existing)
            .map_err(|e| format!("现有 WorkBuddy 登录信息不是有效 JSON，已停止覆盖: {}", e))?;
        let expected_hash: [u8; 32] = Sha256::digest(&existing).into();
        // WorkBuddy 5.6 起 token 等字段是官方加密的（$wbEncrypted）。
        // 这里解密 → 换目标账号 → 按**原字段路径**重新加密 → 原子写回，
        // 加密包装一个不少，客户端读到的结构与原来一致。
        // （早先这里是直接拒绝：那时还拿不到官方密钥；现在 twoapi::atrest 有完整能力）
        if contains_encrypted_wrapper(&existing_json) {
            return write_encrypted_session(
                account,
                platform,
                &auth_file,
                &existing_json,
                &expected_hash,
                &marker_path,
                marker_existed_before,
            );
        }
        // 原文件是 5.6 之前的明文遗留文件：同样必须写成加密态。
        // （2026-10-17 真机故障：这里写明文 → 客户端读出来就是"没登录" → 提示重新登录）
        backup_auth_file(&auth_file);
        let session = build_auth_session(account, platform, Some(&existing_json));
        return write_session_as_encrypted(
            account,
            &auth_file,
            &session,
            Some(&expected_hash),
            &marker_path,
            marker_existed_before,
        );
    }

    // 文件不存在（客户端已登出并删除了登录文件）：重建一份**加密**的会话。
    // 抄 cockpit-tools `workbuddy_account.rs`：切号写回不依赖"原文件是否加密"，
    // 而是取得到官方密钥就加密 —— 所以客户端登出后也能直接切号救回来。
    let session = build_auth_session(account, platform, None);
    write_session_as_encrypted(
        account,
        &auth_file,
        &session,
        None,
        &marker_path,
        marker_existed_before,
    )
}

/// 切号写回前备份现有登录文件（客户端自己的 `.bak` 会被它覆盖，用独立后缀）。
///
/// 备份失败**不阻断**切号 —— 它只是保险，不该让切号失败。
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

/// 把明文会话加密后写回登录文件（用于「文件不存在 / 原文件未加密」两种情况）。
///
/// 抄 cockpit-tools `workbuddy_auth_crypto::encode_session` 的语义：
/// **取不到官方密钥就报错停止覆盖**，绝不退回写明文 —— 明文登录态在 5.6+ 客户端上
/// 等于没切（表现为打开客户端提示登录）。
pub(super) fn write_session_as_encrypted(
    account: &BuddyAccount,
    auth_file: &std::path::PathBuf,
    session: &serde_json::Value,
    expected_hash: Option<&[u8; 32]>,
    marker_path: &std::path::PathBuf,
    marker_existed_before: bool,
) -> Result<String, String> {
    use super::super::twoapi::atrest;

    let secret = atrest::fetch_secret().map_err(|e| {
        format!("无法取得 WorkBuddy 官方加密密钥，已停止覆盖（写明文登录态 5.6+ 客户端不认）：{e}")
    })?;
    let encrypted = atrest::encrypt_session_fields(session, &secret)?;
    let content = serde_json::to_string_pretty(&encrypted)
        .map_err(|e| format!("序列化登录信息失败: {}", e))?;

    match expected_hash {
        Some(hash) => {
            if !store::write_atomic_if_hash_matches(auth_file, hash, &content)? {
                return Err(
                    "WorkBuddy 登录信息在切号期间被官方客户端更新，已停止覆盖，请重试".to_string(),
                );
            }
        }
        None => store::write_atomic(auth_file, &content)?,
    }

    // 写后校验：加密包装必须在、四个官方字段必须在、解出来的 token 必须是目标账号
    let written =
        fs::read_to_string(auth_file).map_err(|e| format!("校验 WorkBuddy 登录信息失败: {}", e))?;
    let written_json: serde_json::Value = serde_json::from_str(&written)
        .map_err(|e| format!("校验 WorkBuddy 登录信息 JSON 失败: {}", e))?;
    if !contains_encrypted_wrapper(&written_json) {
        return Err(
            "写回结果不是官方加密态，WorkBuddy 登录态可能已损坏，请重新登录 WorkBuddy".to_string(),
        );
    }
    for key in ["account", "auth", "accounts", "allAccounts"] {
        if written_json.get(key).is_none() {
            return Err(format!("校验 WorkBuddy 登录信息失败，缺少官方字段 {}", key));
        }
    }
    let written_token = written_json
        .get("auth")
        .and_then(|auth| auth.get("accessToken"))
        .map(|field| atrest::decrypt_field(field, &secret).unwrap_or_default());
    if written_token.as_deref() != Some(account.access_token.as_str()) {
        return Err(format!(
            "校验 WorkBuddy 登录信息失败，未写入目标账号: {}",
            auth_file.display()
        ));
    }

    retire_logout_marker(marker_path, marker_existed_before)?;
    Ok(format!(
        "已切换到 {}（{}，已按官方加密格式写回）",
        account.display_name(),
        auth_file.display()
    ))
}
