use super::*;

pub fn ensure_switch_snapshot_current(
    platform: BuddyPlatform,
    snapshot: &BuddyAccount,
) -> Result<(), String> {
    let _lock = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let latest = load_account(platform, &snapshot.id).ok_or("切号账号已被删除或无法读取")?;
    if !identity::same_identity(&latest, snapshot)
        || latest.access_token != snapshot.access_token
        || latest.refresh_token != snapshot.refresh_token
        || latest.domain != snapshot.domain
        || latest.expires_at != snapshot.expires_at
        || latest.auth_raw != snapshot.auth_raw
    {
        return Err("账号凭据在切号期间已更新，已停止写入，请重试".into());
    }
    Ok(())
}

/// Patch only usage metadata under the store lock. Never save a pre-switch
/// snapshot over credentials refreshed while the client was being switched.
pub fn record_switch_usage(platform: BuddyPlatform, snapshot: &BuddyAccount) -> Result<(), String> {
    let _lock = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = account_file_path(platform, &snapshot.id)?;
    let previous = fs::read(&path).map_err(|e| format!("读取切号账号失败: {e}"))?;
    let mut latest: BuddyAccount = serde_json::from_slice(&previous).map_err(|e| e.to_string())?;
    if !identity::same_identity(&latest, snapshot) {
        return Err("账号身份已变化，未更新使用时间".into());
    }
    let mut index = load_index_checked(platform)?;
    let summary = index
        .accounts
        .iter_mut()
        .find(|item| item.id == snapshot.id)
        .ok_or("账号索引已变化")?;
    latest.last_used = chrono::Utc::now().timestamp();
    summary.last_used = latest.last_used;
    write_atomic(
        &path,
        &serde_json::to_string_pretty(&latest).map_err(|e| e.to_string())?,
    )?;
    if let Err(error) = save_index(platform, &index) {
        return Err(
            match crate::commands::foundation::atomic_write_file(&path, &previous) {
                Ok(()) => format!("保存使用时间失败，已回滚: {error}"),
                Err(rollback) => format!("保存使用时间失败: {error}；回滚未确认: {rollback}"),
            },
        );
    }
    Ok(())
}
