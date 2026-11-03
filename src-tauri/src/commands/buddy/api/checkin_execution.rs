use super::*;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

fn account_lock(key: String) -> Arc<AsyncMutex<()>> {
    static LOCKS: OnceLock<Mutex<std::collections::HashMap<String, Weak<AsyncMutex<()>>>>> = OnceLock::new();
    let mut locks = LOCKS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) { return lock; }
    let lock = Arc::new(AsyncMutex::new(()));
    locks.insert(key, Arc::downgrade(&lock));
    lock
}

/// Manual and scheduled check-ins share the same identity lock through persistence.
pub(crate) async fn claim_checkin(platform: BuddyPlatform, id: &str) -> Result<(BuddyAccount, OwnedMutexGuard<()>), String> {
    let load = || store::load_account(platform, id).ok_or_else(|| format!("账号不存在: {id}"));
    let account = load()?;
    let key = account.uid.as_deref().filter(|uid| !uid.trim().is_empty())
        .map(|uid| format!("uid:{uid}"))
        .unwrap_or_else(|| format!("{}:{id}", platform.as_str()));
    let guard = account_lock(key).lock_owned().await;
    Ok((load()?, guard))
}

pub(crate) fn resolved_checkin_streak(account: &BuddyAccount, response: &CheckinResponse, now: i64) -> i64 {
    if response.inactive == Some(true) || !response.success { return account.checkin_streak; }
    if let Some(streak) = response.streak_days { return streak.max(0); }
    let local_day = |timestamp| chrono::DateTime::from_timestamp(timestamp, 0)
        .map(|date| date.with_timezone(&chrono::Local).date_naive());
    let checked_today = account.last_checkin_time.and_then(local_day).is_some_and(|day| Some(day) == local_day(now));
    if response.already == Some(true) || checked_today { account.checkin_streak }
    else { account.checkin_streak.saturating_add(1) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn same_identity_waits_while_other_accounts_can_run_and_cancelled_waiters_release() {
        let first = account_lock("checkin-test-a".into()).lock_owned().await;
        let same = account_lock("checkin-test-a".into());
        assert!(same.try_lock().is_err());
        assert!(account_lock("checkin-test-b".into()).try_lock_owned().is_ok());
        let waiting = tokio::spawn(async move { same.lock_owned().await });
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());
        waiting.abort();
        let _ = waiting.await;
        drop(first);
        assert!(account_lock("checkin-test-a".into()).try_lock_owned().is_ok());
    }

    #[test]
    fn already_and_same_day_success_do_not_increment_the_streak() {
        let now = 1_793_664_000;
        let mut account: BuddyAccount = serde_json::from_value(json!({
            "id":"test", "platform":"workbuddy", "email":"", "accessToken":"fixture", "checkinStreak":5,
            "createdAt":0, "lastUsed":0
        })).unwrap();
        let mut response = decode_checkin_response(200, r#"{"code":10001,"msg":"今日已签到"}"#).unwrap();
        assert_eq!(resolved_checkin_streak(&account, &response, now), 5);
        response.already = None;
        assert_eq!(resolved_checkin_streak(&account, &response, now), 6);
        account.last_checkin_time = Some(now);
        assert_eq!(resolved_checkin_streak(&account, &response, now), 5);
        response.streak_days = Some(12);
        assert_eq!(resolved_checkin_streak(&account, &response, now), 12);
        response.inactive = Some(true);
        assert_eq!(resolved_checkin_streak(&account, &response, now), 5);
    }
}
