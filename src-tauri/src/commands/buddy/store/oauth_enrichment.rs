use super::*;

/// Optional OAuth metadata is a compare-and-merge, never a login or an upsert.
/// Deletion and newer credentials win; manual fields and authentication are untouched.
pub fn merge_optional_oauth_enrichment(
    platform: BuddyPlatform,
    expected: &BuddyAccount,
    enriched: BuddyAccount,
) -> Result<Option<BuddyAccount>, String> {
    let _lock = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = account_file_path(platform, &expected.id)?;
    let mut current = match fs::read(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("读取现有账号失败: {e}")),
        Ok(bytes) => serde_json::from_slice::<BuddyAccount>(&bytes)
            .map_err(|e| format!("现有账号无法解析，未覆盖: {e}"))?,
    };
    if current.id != expected.id
        || current.access_token != expected.access_token
        || current.refresh_token != expected.refresh_token
    {
        return Ok(None);
    }
    if enriched
        .usage_updated_at
        .zip(current.usage_updated_at)
        .map(|(incoming, stored)| incoming < stored)
        .unwrap_or(false)
    {
        return Ok(Some(current));
    }
    macro_rules! optional {
        ($($field:ident),* $(,)?) => { $(if enriched.$field.is_some() { current.$field = enriched.$field; })* };
    }
    optional!(
        plan_type,
        dosage_notify_code,
        dosage_notify_zh,
        dosage_notify_en,
        payment_type,
        quota_raw,
        usage_raw,
        profile_raw,
        usage_updated_at
    );
    let content = serde_json::to_string_pretty(&current).map_err(|e| e.to_string())?;
    write_atomic(&path, &content)?;
    Ok(Some(current))
}
