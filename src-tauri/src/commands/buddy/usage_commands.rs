//! usage commands responsibility.
use super::*;

// ─── Token 刷新 / 用量 ───

#[tauri::command]
pub async fn buddy_refresh_token(
    platform: String,
    account_id: String,
) -> Result<BuddyAccount, String> {
    let platform = platform_from_str(&platform)?;
    let account = store::load_account(platform, &account_id)
        .ok_or_else(|| format!("账号不存在: {}", account_id))?;
    let (refreshed, quota_err) = api::refresh_payload_for_account(platform, &account).await?;
    let mut saved = store::upsert_account(platform, refreshed)?;
    if let Some(err) = quota_err {
        saved.quota_query_last_error = Some(err.clone());
        saved.quota_query_last_error_at = Some(chrono::Utc::now().timestamp_millis());
        saved = store::upsert_account(platform, saved)?;
    }
    Ok(saved)
}

#[tauri::command]
pub async fn buddy_refresh_tokens(
    platform: String,
    account_ids: Vec<String>,
) -> Result<i32, String> {
    let platform = platform_from_str(&platform)?;
    // 空列表 = 全部账号（兼容旧「刷新全部」语义）；非空则只刷新勾选的
    let accounts: Vec<BuddyAccount> = if account_ids.is_empty() {
        store::list_accounts(platform)
    } else {
        account_ids
            .iter()
            .filter_map(|id| store::load_account(platform, id))
            .collect()
    };
    let mut success = 0;
    for account in accounts {
        match api::refresh_payload_for_account(platform, &account).await {
            Ok((refreshed, quota_err)) => {
                let mut saved = store::upsert_account(platform, refreshed)?;
                // 与 buddy_query_usage / buddy_query_all_usage 一致：失败原因必须落盘。
                // 这条路径就是面板的「刷新选中/刷新全部」，以前把 quota_err 丢掉，
                // 于是「为什么这个号没数据 / 为什么它要重新登录」在界面上永远查不到。
                if let Some(err) = quota_err {
                    saved.quota_query_last_error = Some(err);
                    saved.quota_query_last_error_at = Some(chrono::Utc::now().timestamp_millis());
                    saved = store::upsert_account(platform, saved)?;
                }
                success += 1;
            }
            Err(e) => {
                eprintln!("[Buddy] 刷新账号 {} 失败: {}", account.id, e);
            }
        }
    }
    Ok(success)
}

#[tauri::command]
pub async fn buddy_query_usage(
    platform: String,
    account_id: String,
) -> Result<BuddyAccount, String> {
    let platform = platform_from_str(&platform)?;
    let account = store::load_account(platform, &account_id)
        .ok_or_else(|| format!("账号不存在: {}", account_id))?;
    let (refreshed, quota_err) = api::refresh_payload_for_account(platform, &account).await?;
    let mut saved = store::upsert_account(platform, refreshed)?;
    if let Some(err) = quota_err {
        saved.quota_query_last_error = Some(err);
        saved.quota_query_last_error_at = Some(chrono::Utc::now().timestamp_millis());
        saved = store::upsert_account(platform, saved)?;
    }
    Ok(saved)
}

#[tauri::command]
pub async fn buddy_query_all_usage(platform: String) -> Result<i32, String> {
    let platform = platform_from_str(&platform)?;
    let accounts = store::list_accounts(platform);
    let now_ms = chrono::Utc::now().timestamp_millis();
    // 已判吊销的账号在冷却期内跳过：面板每次打开都会自动同步一次，
    // 不设冷却等于每次打开都把死号打成一轮 401 风暴（也正是 2026-10-23 的教训之一）。
    const REVOKED_COOLDOWN_MS: i64 = 60 * 60 * 1000;
    let mut success = 0;
    for account in accounts {
        if account.status.as_deref() == Some(api::STATUS_AUTH_REVOKED) {
            if let Some(at) = account.quota_query_last_error_at {
                if now_ms.saturating_sub(at) < REVOKED_COOLDOWN_MS {
                    continue;
                }
            }
        }
        match api::refresh_payload_for_account(platform, &account).await {
            Ok((refreshed, quota_err)) => {
                let mut saved = store::upsert_account(platform, refreshed)?;
                // 批量路径也要把失败原因落盘：否则「为什么这个号没数据」在界面上永远查不到
                if let Some(err) = quota_err {
                    saved.quota_query_last_error = Some(err);
                    saved.quota_query_last_error_at = Some(now_ms);
                    saved = store::upsert_account(platform, saved)?;
                }
                success += 1;
            }
            Err(e) => {
                eprintln!("[Buddy] 查询账号 {} 用量失败: {}", account.id, e);
            }
        }
    }
    Ok(success)
}

// ─── 新增：OAuth 登录 / 粘贴 token ───

#[tauri::command]
pub async fn buddy_oauth_start(platform: String) -> Result<api::OAuthStartResponse, String> {
    let platform = platform_from_str(&platform)?;
    api::oauth_start(platform).await
}

#[tauri::command]
pub async fn buddy_oauth_complete(
    platform: String,
    login_id: String,
) -> Result<BuddyAccount, String> {
    let platform = platform_from_str(&platform)?;
    let account = api::oauth_complete(platform, &login_id).await?;
    let saved = store::upsert_account(platform, account)?;
    // 登录后补全用量
    match api::refresh_payload_for_account(platform, &saved).await {
        Ok((refreshed, _)) => Ok(store::upsert_account(platform, refreshed)?),
        Err(e) => {
            eprintln!("[Buddy OAuth] 登录后刷新失败，保留原账号信息: {}", e);
            Ok(saved)
        }
    }
}

#[tauri::command]
pub fn buddy_oauth_cancel(platform: String, login_id: Option<String>) -> Result<(), String> {
    let platform = platform_from_str(&platform)?;
    api::oauth_cancel(platform, login_id.as_deref())
}

#[tauri::command]
pub async fn buddy_add_account_with_token(
    platform: String,
    access_token: String,
) -> Result<BuddyAccount, String> {
    let platform = platform_from_str(&platform)?;
    let account = api::build_payload_from_token(platform, access_token.trim()).await?;
    store::upsert_account(platform, account)
}
