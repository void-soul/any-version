use super::*;

struct Scope {
    previous: Option<std::path::PathBuf>,
    root: std::path::PathBuf,
    _guard: TestGuard,
}
impl Scope {
    fn new() -> Self {
        let (_, guard) = test_root();
        let previous = TEST_DATA_ROOT.lock().unwrap().clone();
        let root = std::env::temp_dir().join(format!(
            "buddy-store-regression-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        set_test_data_root(root.clone());
        Self {
            previous,
            root,
            _guard: guard,
        }
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        *TEST_DATA_ROOT.lock().unwrap() = self.previous.take();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn masked_display_identity_does_not_merge_credentials() {
    let _scope = Scope::new();
    let first = sample_account("masked-a", "135****6735");
    let second = sample_account("masked-b", "135****6735");
    upsert_account(test_platform(), first).unwrap();
    upsert_account(test_platform(), second).unwrap();
    assert_eq!(list_accounts(test_platform()).len(), 2);
    assert_eq!(
        load_account(test_platform(), "masked-a")
            .unwrap()
            .access_token,
        "token-masked-a"
    );
}

#[test]
fn uid_matching_precedes_any_email_candidate() {
    let _scope = Scope::new();
    let first = sample_account("a", "shared@test.com");
    let second = sample_account("b", "other@test.com");
    for account in [&first, &second] {
        write_atomic(
            &account_file_path(test_platform(), &account.id).unwrap(),
            &serde_json::to_string(account).unwrap(),
        )
        .unwrap();
    }
    let mut incoming = second.clone();
    incoming.id.clear();
    incoming.email = first.email.clone();
    incoming.access_token = "new-b-token".into();
    let saved = upsert_account(test_platform(), incoming).unwrap();
    assert_eq!(saved.id, "b");
    assert_eq!(
        load_account(test_platform(), "a").unwrap().access_token,
        first.access_token
    );
    assert_eq!(
        load_account(test_platform(), "b").unwrap().access_token,
        "new-b-token"
    );
}

#[test]
fn real_email_legacy_deduplication_remains_supported() {
    let _scope = Scope::new();
    upsert_account(test_platform(), sample_account("real-a", "Case@Test.com")).unwrap();
    let saved =
        upsert_account(test_platform(), sample_account("real-b", " case@test.COM ")).unwrap();
    assert_eq!(saved.id, "real-a");
    assert_eq!(saved.access_token, "token-real-b");
}

#[test]
fn caller_id_collision_cannot_replace_another_identity() {
    let _scope = Scope::new();
    let old = upsert_account(test_platform(), sample_account("collision", "old@test.com")).unwrap();
    let mut incoming = sample_account("collision", "new@test.com");
    incoming.uid = Some("different-uid".into());
    assert!(upsert_account(test_platform(), incoming)
        .unwrap_err()
        .contains("拒绝覆盖"));
    assert_eq!(
        load_account(test_platform(), "collision")
            .unwrap()
            .access_token,
        old.access_token
    );
}

#[test]
fn invalid_import_is_rejected_before_any_writes() {
    let _scope = Scope::new();
    for token in ["", "   "] {
        let valid = sample_account("valid", "valid@test.com");
        let mut invalid = sample_account("invalid", "invalid@test.com");
        invalid.access_token = token.into();
        let json = serde_json::to_string(&vec![valid, invalid]).unwrap();
        assert!(import_accounts(test_platform(), &json)
            .unwrap_err()
            .contains("第 2"));
        assert!(list_accounts(test_platform()).is_empty());
    }
    let mut missing = sample_account("missing", "135****6735");
    missing.uid = Some(" ".into());
    assert!(upsert_account(test_platform(), missing).is_err());
}

#[test]
fn corrupt_or_missing_index_recovers_existing_detail_files() {
    let _scope = Scope::new();
    upsert_account(test_platform(), sample_account("recover-a", "a@test.com")).unwrap();
    let path = index_path(test_platform()).unwrap();
    std::fs::write(&path, b"invalid-json").unwrap();
    assert_eq!(list_accounts(test_platform()).len(), 1);
    upsert_account(test_platform(), sample_account("recover-b", "b@test.com")).unwrap();
    assert_eq!(load_index(test_platform()).accounts.len(), 2);
    std::fs::remove_file(path).unwrap();
    assert_eq!(list_accounts(test_platform()).len(), 2);
}

#[test]
fn failed_index_commit_rolls_back_new_and_existing_details() {
    let _scope = Scope::new();
    let old = upsert_account(test_platform(), sample_account("original", "old@test.com")).unwrap();
    let path = index_path(test_platform()).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap(); // deterministic failing index destination, no ACL assumptions
    assert!(
        upsert_account(test_platform(), sample_account("new", "new@test.com"))
            .unwrap_err()
            .contains("已回滚")
    );
    assert!(load_account(test_platform(), "new").is_none());
    let mut changed = old.clone();
    changed.access_token = "replacement-token".into();
    assert!(upsert_account(test_platform(), changed).is_err());
    assert_eq!(
        load_account(test_platform(), "original")
            .unwrap()
            .access_token,
        old.access_token
    );
}

#[test]
fn reauthentication_preserves_manual_expiry_and_explicit_clear_works() {
    let _scope = Scope::new();
    upsert_account(test_platform(), sample_account("expiry", "expiry@test.com")).unwrap();
    let times = std::collections::HashMap::from([("manual".into(), 123456)]);
    set_expiry_times(test_platform(), "expiry", times.clone()).unwrap();
    let fresh = sample_account("other-id", "expiry@test.com");
    let saved = upsert_account(test_platform(), fresh).unwrap();
    assert_eq!(saved.id, "expiry");
    assert_eq!(saved.expiry_times, times);
    assert!(
        set_expiry_times(test_platform(), "expiry", Default::default())
            .unwrap()
            .expiry_times
            .is_empty()
    );
}

#[test]
fn twins_require_unique_reliable_mailboxes() {
    let _scope = Scope::new();
    let from = BuddyPlatform::Workbuddy;
    let to = BuddyPlatform::CodebuddyCn;
    upsert_account(from, sample_account("source", "135****6735")).unwrap();
    upsert_account(to, sample_account("unrelated", "135****6735")).unwrap();
    let times = std::collections::HashMap::from([("manual".into(), 123456)]);
    set_expiry_times_shared(from, "source", times.clone()).unwrap();
    backfill_expiry_times();
    assert!(load_account(to, "unrelated")
        .unwrap()
        .expiry_times
        .is_empty());
    upsert_account(from, sample_account("source-mail", "same@test.com")).unwrap();
    upsert_account(to, sample_account("twin-mail", "same@test.com")).unwrap();
    set_expiry_times_shared(from, "source-mail", times.clone()).unwrap();
    assert_eq!(load_account(to, "twin-mail").unwrap().expiry_times, times);
    let duplicate = sample_account("duplicate", "same@test.com");
    write_atomic(
        &account_file_path(to, "duplicate").unwrap(),
        &serde_json::to_string(&duplicate).unwrap(),
    )
    .unwrap();
    set_expiry_times_shared(
        from,
        "source-mail",
        std::collections::HashMap::from([("manual".into(), 999999)]),
    )
    .unwrap();
    assert_eq!(load_account(to, "twin-mail").unwrap().expiry_times, times);
    assert!(load_account(to, "duplicate")
        .unwrap()
        .expiry_times
        .is_empty());
}

#[test]
fn ambiguous_source_mailbox_does_not_propagate_or_backfill() {
    let _scope = Scope::new();
    let from = BuddyPlatform::Workbuddy;
    let to = BuddyPlatform::CodebuddyCn;
    upsert_account(from, sample_account("source-a", "duplicate@test.com")).unwrap();
    let duplicate = sample_account("source-b", "duplicate@test.com");
    write_atomic(
        &account_file_path(from, &duplicate.id).unwrap(),
        &serde_json::to_string(&duplicate).unwrap(),
    )
    .unwrap();
    upsert_account(to, sample_account("target", "duplicate@test.com")).unwrap();
    set_expiry_times_shared(
        from,
        "source-a",
        std::collections::HashMap::from([("manual".into(), 123456)]),
    )
    .unwrap();
    backfill_expiry_times();
    assert!(load_account(to, "target").unwrap().expiry_times.is_empty());
}

#[test]
fn twin_reauthentication_keeps_both_manual_values_and_shared_clear() {
    let _scope = Scope::new();
    let from = BuddyPlatform::Workbuddy;
    let to = BuddyPlatform::CodebuddyCn;
    upsert_account(from, sample_account("wb", "twins@test.com")).unwrap();
    upsert_account(to, sample_account("cn", "twins@test.com")).unwrap();
    let times = std::collections::HashMap::from([("manual".into(), 123456)]);
    set_expiry_times_shared(from, "wb", times.clone()).unwrap();
    for (platform, id) in [(from, "wb"), (to, "cn")] {
        let mut fresh = sample_account("fresh", "twins@test.com");
        fresh.uid = Some(format!("reauth-{}", platform.as_str()));
        let saved = upsert_account(platform, fresh).unwrap();
        assert_eq!(saved.id, id);
        assert_eq!(saved.expiry_times, times);
    }
    assert_eq!(
        load_account(from, "wb").unwrap().expiry_times,
        load_account(to, "cn").unwrap().expiry_times
    );
    set_expiry_times_shared(to, "cn", Default::default()).unwrap();
    assert!(load_account(from, "wb").unwrap().expiry_times.is_empty());
    assert!(load_account(to, "cn").unwrap().expiry_times.is_empty());
}

#[test]
fn optional_oauth_metadata_cannot_revive_deleted_account() {
    let _scope = Scope::new();
    let saved = upsert_account(
        test_platform(),
        sample_account("deleted", "deleted@test.com"),
    )
    .unwrap();
    let mut enriched = saved.clone();
    enriched.quota_raw = Some(serde_json::json!({"credits": 99}));
    delete_account(test_platform(), &saved.id).unwrap();
    assert!(
        merge_optional_oauth_enrichment(test_platform(), &saved, enriched)
            .unwrap()
            .is_none()
    );
    assert!(load_account(test_platform(), &saved.id).is_none());
}

#[test]
fn optional_oauth_metadata_rejects_newer_credentials_and_preserves_manual_fields() {
    let _scope = Scope::new();
    let saved = upsert_account(
        test_platform(),
        sample_account("optional", "optional@test.com"),
    )
    .unwrap();
    let mut enriched = saved.clone();
    enriched.quota_raw = Some(serde_json::json!({"credits": 99}));
    enriched.uid = Some("wrong-identity".into());
    enriched.access_token = "must-not-be-stored".into();
    enriched.auth_raw = Some(serde_json::json!({"wrong":"auth"}));
    set_expiry_times(
        test_platform(),
        &saved.id,
        std::collections::HashMap::from([("manual".into(), 123456)]),
    )
    .unwrap();
    let merged = merge_optional_oauth_enrichment(test_platform(), &saved, enriched.clone())
        .unwrap()
        .unwrap();
    assert_eq!(merged.uid, saved.uid);
    assert_eq!(merged.access_token, saved.access_token);
    assert_eq!(merged.auth_raw, saved.auth_raw);
    assert_eq!(merged.expiry_times["manual"], 123456);
    assert_eq!(merged.quota_raw, enriched.quota_raw);
    let mut fresh = saved.clone();
    fresh.refresh_token = Some("rotated-refresh".into());
    upsert_account(test_platform(), fresh).unwrap();
    assert!(
        merge_optional_oauth_enrichment(test_platform(), &saved, enriched)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        load_account(test_platform(), &saved.id)
            .unwrap()
            .refresh_token
            .as_deref(),
        Some("rotated-refresh")
    );
}
