use super::*;

fn sample_account() -> BuddyAccount {
    BuddyAccount {
        id: "workbuddy_abc".to_string(),
        platform: "workbuddy".to_string(),
        email: "user@example.com".to_string(),
        uid: Some("u123".to_string()),
        nickname: Some("Alice".to_string()),
        enterprise_id: None,
        enterprise_name: None,
        tags: None,
        access_token: "wb-token-1".to_string(),
        refresh_token: Some("wb-refresh-1".to_string()),
        token_type: Some("Bearer".to_string()),
        expires_at: Some(1770000000),
        domain: Some("example.com".to_string()),
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

#[test]
fn session_contains_official_fields_and_current_account() {
    let session = build_auth_session(&sample_account(), BuddyPlatform::Workbuddy, None);
    let obj = session.as_object().unwrap();
    for key in ["account", "auth", "accounts", "allAccounts"] {
        assert!(obj.contains_key(key), "缺少官方字段 {}", key);
    }
    let account = obj["account"].as_object().unwrap();
    assert_eq!(account["uid"].as_str(), Some("u123"));
    assert_eq!(account["nickname"].as_str(), Some("Alice"));
    assert_eq!(account["lastLogin"].as_bool(), Some(true));
    let auth = obj["auth"].as_object().unwrap();
    assert_eq!(auth["accessToken"].as_str(), Some("wb-token-1"));
    assert_eq!(auth["refreshToken"].as_str(), Some("wb-refresh-1"));
    // accounts 数组中包含目标账号且标记 lastLogin
    let accounts = obj["accounts"].as_array().unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0]["uid"].as_str(), Some("u123"));
}

#[test]
fn session_merges_existing_accounts_and_marks_others_not_login() {
    let base = serde_json::json!({
        "accounts": [
            {"uid": "other1", "nickname": "Bob", "lastLogin": true},
            {"uid": "u123", "nickname": "OldAlice", "lastLogin": false}
        ]
    });
    let session = build_auth_session(&sample_account(), BuddyPlatform::Workbuddy, Some(&base));
    let obj = session.as_object().unwrap();
    let accounts = obj["accounts"].as_array().unwrap();
    assert_eq!(accounts.len(), 2);
    // 目标账号被替换为最新信息
    let target = accounts
        .iter()
        .find(|a| a["uid"].as_str() == Some("u123"))
        .unwrap();
    assert_eq!(target["nickname"].as_str(), Some("Alice"));
    // 其它账号 lastLogin 置 false
    let other = accounts
        .iter()
        .find(|a| a["uid"].as_str() == Some("other1"))
        .unwrap();
    assert_eq!(other["lastLogin"].as_bool(), Some(false));
}

#[test]
fn encrypted_wrapper_detection() {
    assert!(contains_encrypted_wrapper(
        &serde_json::json!({"$wbEncrypted": true})
    ));
    assert!(contains_encrypted_wrapper(
        &serde_json::json!({"nested": {"a": {"$wbEncrypted": 1}}})
    ));
    assert!(contains_encrypted_wrapper(
        &serde_json::json!([{"$wbEncrypted": "x"}])
    ));
    assert!(!contains_encrypted_wrapper(
        &serde_json::json!({"account": {"uid": "1"}})
    ));
}

#[test]
fn token_parts_extraction() {
    assert_eq!(
        extract_local_token_parts("u123+abc.def").unwrap(),
        (Some("u123".to_string()), "abc.def".to_string())
    );
    assert_eq!(
        extract_local_token_parts("plain-token").unwrap(),
        (None, "plain-token".to_string())
    );
    assert_eq!(normalize_local_token("u123+abc").unwrap(), "abc");
    assert_eq!(normalize_local_token("abc").unwrap(), "abc");
}

#[test]
fn jwt_sub_extraction() {
    // header.payload.signature，payload 含 sub
    let payload = general_purpose::URL_SAFE_NO_PAD.encode(r#"{"sub":"u999"}"#);
    let token = format!("eyJhbGciOiJIUzI1NiJ9.{}.sig", payload);
    assert_eq!(extract_uid_from_jwt(&token).unwrap(), "u999");
    assert_eq!(extract_uid_from_jwt("not-a-jwt"), None);
}

fn temp_marker_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "kira-logout-marker-{}-{}-{}",
        tag,
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn retire_marker_removes_stale_logout_marker_even_if_content_drifted() {
    // 遗留标记（写回前就存在）必须清掉：标记在时客户端**不读登录文件**，
    // 不清就是"切了号却提示重新登录"。内容在写回期间被刷新过也必须清
    // （旧实现按 hash 比对，内容一变就漏删）。
    let dir = temp_marker_dir("stale");
    let marker = dir.join("workbuddy-desktop.info.logged-out");
    fs::write(&marker, br#"{"at":1}"#).unwrap();
    fs::write(&marker, br#"{"at":2}"#).unwrap();
    retire_logout_marker(&marker, true).expect("遗留标记必须被清掉");
    assert!(!marker.exists(), "遗留标记未删除 → 客户端会忽略登录态");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn retire_marker_is_noop_when_marker_absent() {
    let dir = temp_marker_dir("absent");
    let marker = dir.join("workbuddy-desktop.info.logged-out");
    retire_logout_marker(&marker, false).expect("本来就没有标记时不该报错");
    retire_logout_marker(&marker, true).expect("标记已被删掉时也不该报错");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn retire_marker_keeps_marker_that_appeared_after_write() {
    // 写回后才出现的标记 = 客户端在切号窗口里又登出了（它没退干净）：
    // 保留标记并报错，不能把它当成遗留标记吞掉
    let dir = temp_marker_dir("late");
    let marker = dir.join("workbuddy-desktop.info.logged-out");
    fs::write(&marker, b"fresh").unwrap();
    let err = retire_logout_marker(&marker, false).expect_err("必须报错而不是静默成功");
    assert!(err.contains("切号期间"), "{}", err);
    assert!(err.contains("完全退出"), "要给可操作提示: {}", err);
    assert!(marker.exists(), "不能吞掉客户端的登出动作");
    let _ = fs::remove_dir_all(&dir);
}
