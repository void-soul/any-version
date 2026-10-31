use super::*;

fn uid(account: &BuddyAccount) -> Option<String> {
    account
        .uid
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_lowercase)
}

/// Only actual mailbox identities can participate in email deduplication/twin sharing.
/// Display labels, masked phone numbers and placeholders are never identity evidence.
pub(super) fn reliable_email(value: &str) -> Option<String> {
    let value = value.trim();
    let mut parts = value.split('@');
    let local = parts.next()?;
    let domain = parts.next()?;
    if local.is_empty()
        || domain.is_empty()
        || parts.next().is_some()
        || value
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '*' | '<' | '>' | '"'))
    {
        return None;
    }
    Some(value.to_lowercase())
}

pub(super) fn validate_account(account: &BuddyAccount) -> Result<(), String> {
    if account.access_token.trim().is_empty() {
        return Err("accessToken 不能为空".into());
    }
    if uid(account).is_none() && reliable_email(&account.email).is_none() {
        return Err("账号必须具有非空 uid 或可靠邮箱，昵称和脱敏手机号不能作为身份".into());
    }
    if !account.id.trim().is_empty() {
        normalize_account_id(&account.id)?;
    }
    Ok(())
}

pub(super) fn same_identity(a: &BuddyAccount, b: &BuddyAccount) -> bool {
    if uid(a).zip(uid(b)).map(|(a, b)| a == b).unwrap_or(false) {
        return true;
    }
    // Preserve the product's legacy real-email deduplication even when platform UIDs differ.
    reliable_email(&a.email)
        .zip(reliable_email(&b.email))
        .map(|(a, b)| a == b)
        .unwrap_or(false)
}

pub(super) fn matching_account_id(
    accounts: &[BuddyAccount],
    incoming: &BuddyAccount,
) -> Result<Option<String>, String> {
    let incoming_uid = uid(incoming);
    let by_uid: Vec<_> = accounts
        .iter()
        .filter(|a| incoming_uid.is_some() && uid(a) == incoming_uid)
        .collect();
    if let [account] = by_uid.as_slice() {
        return Ok(Some(account.id.clone()));
    }
    if by_uid.len() > 1 {
        return Err("同 uid 存在多个账号记录，请先整理重复账号".into());
    }
    let Some(email) = reliable_email(&incoming.email) else {
        return Ok(None);
    };
    let by_email: Vec<_> = accounts
        .iter()
        .filter(|a| reliable_email(&a.email).as_deref() == Some(email.as_str()))
        .collect();
    match by_email.as_slice() {
        [] => Ok(None),
        [account] => Ok(Some(account.id.clone())),
        _ => Err("同邮箱存在多个候选账号，无法安全确定更新目标".into()),
    }
}
