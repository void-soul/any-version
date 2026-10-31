use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
fn sample_account(id: &str, email: &str) -> BuddyAccount {
    BuddyAccount {
        id: id.to_string(),
        platform: "workbuddy".to_string(),
        email: email.to_string(),
        uid: Some(format!("uid-{}", id)),
        nickname: None,
        enterprise_id: None,
        enterprise_name: None,
        tags: None,
        access_token: format!("token-{}", id),
        refresh_token: None,
        token_type: Some("Bearer".to_string()),
        expires_at: None,
        domain: None,
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

fn begin(sessions: &OAuthSessions, id: &str) {
    sessions
        .begin(
            BuddyPlatform::Workbuddy,
            PendingOAuthState {
                login_id: id.into(),
                expires_at: now_timestamp() + 600,
                state: "fake-state".into(),
                token_ready: None,
                ready: None,
            },
        )
        .unwrap();
}
async fn no_enrichment(_: BuddyAccount) -> Result<BuddyAccount, String> {
    Err("isolated optional failure".into())
}

#[tokio::test]
async fn failed_required_write_reuses_ready_and_completed_is_idempotent() {
    let sessions = OAuthSessions::default();
    begin(&sessions, "login");
    let polls = AtomicUsize::new(0);
    let result = sessions
        .finish(
            BuddyPlatform::Workbuddy,
            "login",
            || async {
                polls.fetch_add(1, Ordering::SeqCst);
                Ok(sample_account("fake-id", "fake@example.invalid"))
            },
            |_| Err("injected disk failure".into()),
            no_enrichment,
            |_, value| Ok(Some(value)),
        )
        .await;
    assert!(result.is_err());
    assert_eq!(polls.load(Ordering::SeqCst), 1);
    assert!(sessions
        .completed(BuddyPlatform::Workbuddy, "login", now_timestamp())
        .unwrap()
        .is_none());
    assert!(sessions
        .state(BuddyPlatform::Workbuddy, "login", now_timestamp())
        .unwrap()
        .ready
        .is_some());
    let saved = sessions
        .finish(
            BuddyPlatform::Workbuddy,
            "login",
            || async { panic!("must not repoll official ready") },
            Ok,
            no_enrichment,
            |_, value| Ok(Some(value)),
        )
        .await
        .unwrap();
    let repeated = sessions
        .finish(
            BuddyPlatform::Workbuddy,
            "login",
            || async { panic!("completed must not poll") },
            |_| panic!("completed must not write"),
            |_| async { panic!("completed must not enrich") },
            |_, _| panic!("completed must not merge"),
        )
        .await
        .unwrap();
    assert_eq!(saved.id, repeated.id);
    assert!(sessions
        .state(BuddyPlatform::Workbuddy, "login", now_timestamp())
        .is_err());
    assert!(sessions
        .completed(
            BuddyPlatform::Workbuddy,
            "login",
            now_timestamp() + COMPLETED_TTL + 1
        )
        .unwrap()
        .is_none());
}
#[tokio::test]
async fn optional_save_failure_does_not_turn_saved_login_into_failure() {
    let sessions = OAuthSessions::default();
    begin(&sessions, "login");
    let account = sessions
        .finish(
            BuddyPlatform::Workbuddy,
            "login",
            || async { Ok(sample_account("fake-id", "fake@example.invalid")) },
            Ok,
            |mut value| async move {
                value.nickname = Some("optional".into());
                Ok(value)
            },
            |_, _| Err("optional write failed".into()),
        )
        .await
        .unwrap();
    assert_eq!(account.nickname, None);
    assert_eq!(
        sessions
            .completed(BuddyPlatform::Workbuddy, "login", now_timestamp())
            .unwrap()
            .unwrap()
            .nickname,
        None
    );
}
#[tokio::test]
async fn new_login_or_cancel_during_enrichment_cannot_write_old_metadata() {
    for intent in ["new_state", "cancel", "start_intent"] {
        let sessions = OAuthSessions::default();
        begin(&sessions, "old");
        let merges = AtomicUsize::new(0);
        let sessions_ref = &sessions;
        let saved = sessions
            .finish(
                BuddyPlatform::Workbuddy,
                "old",
                || async { Ok(sample_account("fake-id", "fake@example.invalid")) },
                Ok,
                |value| async move {
                    if intent == "cancel" {
                        sessions_ref
                            .cancel(BuddyPlatform::Workbuddy, Some("old"))
                            .unwrap();
                    } else if intent == "start_intent" {
                        // oauth_start performs this invalidation before requesting auth/state.
                        sessions_ref.cancel(BuddyPlatform::Workbuddy, None).unwrap();
                    } else {
                        begin(sessions_ref, "new");
                    }
                    Ok(value)
                },
                |_, value| {
                    merges.fetch_add(1, Ordering::SeqCst);
                    Ok(Some(value))
                },
            )
            .await
            .unwrap();
        assert_eq!(saved.id, "fake-id");
        assert_eq!(merges.load(Ordering::SeqCst), 0);
    }
}
#[tokio::test]
async fn cancelled_or_expired_ready_never_reaches_persistence() {
    let sessions = OAuthSessions::default();
    begin(&sessions, "login");
    sessions
        .remember_token(
            BuddyPlatform::Workbuddy,
            "login",
            json!({"accessToken":"fake"}),
        )
        .unwrap();
    assert!(sessions
        .state(BuddyPlatform::Workbuddy, "login", now_timestamp())
        .unwrap()
        .token_ready
        .is_some());
    sessions
        .cancel(BuddyPlatform::Workbuddy, Some("login"))
        .unwrap();
    assert!(sessions
        .finish(
            BuddyPlatform::Workbuddy,
            "login",
            || async { panic!("cancelled poll") },
            |_| panic!("cancelled write"),
            no_enrichment,
            |_, _| panic!("cancelled merge")
        )
        .await
        .is_err());
    begin(&sessions, "expired");
    assert!(sessions
        .state(BuddyPlatform::Workbuddy, "expired", now_timestamp() + 601)
        .is_err());
    assert!(sessions
        .state(BuddyPlatform::Workbuddy, "expired", now_timestamp())
        .is_err());
}
#[tokio::test]
async fn concurrent_completion_polls_and_commits_once() {
    let sessions = OAuthSessions::default();
    begin(&sessions, "same");
    let polls = AtomicUsize::new(0);
    let writes = AtomicUsize::new(0);
    let finish = || {
        sessions.finish(
            BuddyPlatform::Workbuddy,
            "same",
            || async {
                polls.fetch_add(1, Ordering::SeqCst);
                tokio::task::yield_now().await;
                Ok(sample_account("fake-id", "fake@example.invalid"))
            },
            |value| {
                writes.fetch_add(1, Ordering::SeqCst);
                Ok(value)
            },
            no_enrichment,
            |_, value| Ok(Some(value)),
        )
    };
    let (a, b) = tokio::join!(finish(), finish());
    assert_eq!(a.unwrap().id, b.unwrap().id);
    assert_eq!(polls.load(Ordering::SeqCst), 1);
    assert_eq!(writes.load(Ordering::SeqCst), 1);
}
