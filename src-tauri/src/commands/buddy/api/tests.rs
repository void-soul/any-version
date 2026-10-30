use super::*;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine as _};

/// 造一个 iss 可指定的 JWT（只要 payload 那段合法即可，签名无所谓）
fn fake_jwt(iss: &str) -> String {
    let payload = B64.encode(json!({"iss": iss, "azp": "console"}).to_string());
    format!("header.{}.signature", payload)
}

#[test]
fn issuer_origin_is_extracted_and_normalized() {
    assert_eq!(
        token_issuer_origin(&fake_jwt("https://www.workbuddy.cn/auth/realms/copilot")).as_deref(),
        Some("https://www.workbuddy.cn")
    );
    assert_eq!(
        token_issuer_origin(&fake_jwt("https://www.codebuddy.cn/auth/realms/copilot")).as_deref(),
        Some("https://www.codebuddy.cn")
    );
    assert_eq!(token_issuer_origin("not-a-jwt"), None);
}

#[test]
fn auth_origin_warning_only_fires_on_mismatch() {
    // 新域 token 切进 WorkBuddy（客户端用 workbuddy.cn）→ 无提示
    assert_eq!(
        auth_origin_warning(
            BuddyPlatform::Workbuddy,
            &fake_jwt("https://www.workbuddy.cn/auth/realms/copilot")
        ),
        None
    );
    // 老域 token 切进 WorkBuddy → 必须提示（客户端会全量 401）
    let warning = auth_origin_warning(
        BuddyPlatform::Workbuddy,
        &fake_jwt("https://www.codebuddy.cn/auth/realms/copilot"),
    )
    .expect("老域 token 切进新域客户端必须提示");
    assert!(warning.contains("www.codebuddy.cn") && warning.contains("www.workbuddy.cn"));
    // 反过来：新域 token 切进 CodeBuddy CN 也要提示
    assert!(
        auth_origin_warning(
            BuddyPlatform::CodebuddyCn,
            &fake_jwt("https://www.workbuddy.cn/auth/realms/copilot")
        )
        .is_some(),
        "CN 客户端只认 codebuddy.cn 域"
    );
}

#[test]
fn api_hosts_put_gateway_first_then_issuer_host() {
    // 老域 token：聚合网关优先，其次老域官方域，最后补新域兜底
    assert_eq!(
        workbuddy_api_hosts(&fake_jwt("https://www.codebuddy.cn/auth/realms/copilot")),
        vec![
            "https://copilot.tencent.com",
            "https://www.codebuddy.cn",
            "https://www.workbuddy.cn",
        ]
    );
    // 新域 token：官方域紧跟其后
    assert_eq!(
        workbuddy_api_hosts(&fake_jwt("https://www.workbuddy.cn/auth/realms/copilot")),
        vec![
            "https://copilot.tencent.com",
            "https://www.workbuddy.cn",
            "https://www.codebuddy.cn",
        ]
    );
}

#[test]
fn api_hosts_fall_back_when_issuer_unparsable() {
    // 非 JWT / 老 token：解不出 iss 也要有官方双域兜底
    assert_eq!(
        workbuddy_api_hosts("opaque-token"),
        vec![
            "https://copilot.tencent.com",
            "https://www.workbuddy.cn",
            "https://www.codebuddy.cn",
        ]
    );
}

#[test]
fn api_hosts_never_duplicate() {
    // 聚合网关自己签发的 token：ISSUER_HOST_MAP 里没有它 → 不能重复入列
    let hosts = workbuddy_api_hosts(&fake_jwt("https://copilot.tencent.com/auth/realms/x"));
    let mut seen = std::collections::HashSet::new();
    for h in &hosts {
        assert!(seen.insert(*h), "候选域重复: {}", h);
    }
    // 国际版 issuer 也要能映射（WorkDaddy ISSUER_HOST_MAP 的四个域都在）
    assert!(
        workbuddy_api_hosts(&fake_jwt("https://www.workbuddy.ai/auth/realms/x"))
            .contains(&"https://www.workbuddy.ai")
    );
}

#[test]
fn revoked_refresh_errors_are_classified() {
    // 2026-10-23 真机两种形态都必须认成「登录态被吊销」，不能当网络错误重试
    assert!(is_revoked_refresh_error(
            "12153:refresh token failed:400 Bad Request: invalid_grant: Session doesn't have required client"
        ));
    assert!(is_revoked_refresh_error("12153:refresh token is invalid"));
    assert!(is_revoked_refresh_error("Token is not active"));
    assert!(!is_revoked_refresh_error("network timeout"));
    // 带前缀的错误要能被上层识别成"需重新登录"
    assert!(revoked_error("invalid_grant").starts_with(AUTH_REVOKED_PREFIX));
}

#[test]
fn refresh_due_matches_reference_cadence() {
    let now = 1_792_728_000_000i64; // 2026-10-23 附近
    let day = DAY_MS;
    let today_ms = now - 3600_000; // 今天已刷过
    let yesterday_ms = now - day - 3600_000; // 昨天刷的

    // 还有 20 天有效期 + 今天刷过 → 不刷（每日一次保活）
    assert!(!refresh_due_at(Some(now + 20 * day), Some(today_ms), now));
    // 还有 20 天但今天还没刷 → 刷一次保活（WorkDaddy `dailyKeepalive` 语义）
    assert!(refresh_due_at(
        Some(now + 20 * day),
        Some(yesterday_ms),
        now
    ));
    // 20 天 + 从没记过刷新时间 → 刷
    assert!(refresh_due_at(Some(now + 20 * day), None, now));
    // 临期（还剩 3 小时）→ 必刷，哪怕今天已经刷过
    assert!(refresh_due_at(
        Some(now + 3 * 3600_000),
        Some(today_ms),
        now
    ));
    // 已过期 → 刷
    assert!(refresh_due_at(Some(now - 1000), Some(today_ms), now));
    // 没有过期时间（老账号/导入）→ 刷
    assert!(refresh_due_at(None, Some(today_ms), now));
}

#[test]
fn last_refresh_time_parses_seconds_and_millis() {
    let mut acc = sample_account_for_refresh();
    acc.auth_raw = Some(json!({ "lastRefreshTime": 1_792_728_000i64 })); // 秒
    assert_eq!(last_refresh_ms(&acc), Some(1_792_728_000_000));
    acc.auth_raw = Some(json!({ "lastRefreshTime": "1792728000000" })); // 字符串毫秒
    assert_eq!(last_refresh_ms(&acc), Some(1_792_728_000_000));
    acc.auth_raw = Some(json!({}));
    assert_eq!(last_refresh_ms(&acc), None);
}

fn sample_account_for_refresh() -> BuddyAccount {
    BuddyAccount {
        id: "test".into(),
        platform: "workbuddy".into(),
        email: "t@example.com".into(),
        uid: None,
        nickname: None,
        enterprise_id: None,
        enterprise_name: None,
        access_token: "a".into(),
        refresh_token: Some("r".into()),
        token_type: None,
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
        tags: Default::default(),
        created_at: 0,
        last_used: 0,
    }
}

#[test]
fn enterprise_usage_wrapping() {
    let body = json!({
        "code": 0,
        "data": {
            "limit_num": 1000,
            "credit": 300,
            "cycle_start_time": "2026-09-01 00:00:00",
            "cycle_end_time": "2026-10-01 00:00:00"
        }
    });
    let wrapped = wrap_enterprise_usage_as_resource(&body).unwrap();
    let items = user_resource_items(&wrapped).unwrap();
    assert_eq!(items.len(), 1);
    let account = &items[0];
    assert_eq!(
        account["PackageCode"].as_str(),
        Some(ENTERPRISE_PACKAGE_CODE)
    );
    assert_eq!(account["CycleCapacitySize"].as_f64(), Some(1000.0));
    assert_eq!(account["CycleCapacityRemain"].as_f64(), Some(700.0));
    assert_eq!(account["CycleCapacityUsed"].as_f64(), Some(300.0));
    assert_eq!(account["Unlimited"].as_bool(), Some(false));
}

#[test]
fn enterprise_unlimited_wrapping() {
    let body = json!({
        "code": 0,
        "data": { "limit_num": -1, "credit": 50 }
    });
    let wrapped = wrap_enterprise_usage_as_resource(&body).unwrap();
    let account = &user_resource_items(&wrapped).unwrap()[0];
    assert_eq!(account["Unlimited"].as_bool(), Some(true));
    assert_eq!(account["CycleCapacityRemain"].as_f64(), Some(-1.0));
}

#[test]
fn checkin_status_loose_parsing() {
    // snake_case + 宽松布尔（0/1 字符串）
    let data = json!({
        "today_checked_in": "1",
        "streak_days": 3,
        "daily_credit": 10
    });
    let status = parse_checkin_status_data(&data).unwrap();
    assert!(status.today_checked_in);
    assert!(status.active); // 缺省 active → true
    assert_eq!(status.streak_days, 3);
    assert_eq!(status.daily_credit, 10);
}

#[test]
fn checkin_status_camel_case() {
    let data = json!({
        "todayCheckedIn": false,
        "Active": false,
        "streakDays": 5
    });
    let status = parse_checkin_status_data(&data).unwrap();
    assert!(!status.today_checked_in);
    assert!(!status.active);
    assert_eq!(status.streak_days, 5);
}

#[test]
fn token_expiry_normalization() {
    // 秒级时间戳 → 毫秒
    assert_eq!(
        token_expiry_at(&json!({"expiresAt": 1750000000})),
        Some(1750000000000)
    );
    // 毫秒级保留
    assert_eq!(
        token_expiry_at(&json!({"expiresAt": 1750000000000i64})),
        Some(1750000000000i64)
    );
    // expiresIn 秒 → 当前毫秒 + 秒*1000
    let from_in = token_expiry_at(&json!({"expiresIn": 3600})).unwrap();
    let now_ms = chrono::Utc::now().timestamp_millis();
    assert!((from_in - (now_ms + 3600_000)).abs() < 2000);
}

// ─── 签到结果分类（对齐 WorkDaddy `checkin-result.js` 的语义） ───

#[test]
fn checkin_classify_success() {
    assert_eq!(
        classify_checkin_result(true, 0, "签到成功"),
        CheckinOutcome::Success
    );
}

#[test]
fn checkin_classify_already_checked() {
    // 官方「今日已签到」业务码 10001 → 视为已签到，而不是失败
    assert_eq!(
        classify_checkin_result(true, 10001, "今日已完成签到"),
        CheckinOutcome::Already
    );
}

#[test]
fn checkin_classify_inactive_beats_already_code() {
    // 活动未开启/已结束时，即使业务码是 10001 也不算「已签到」
    assert_eq!(
        classify_checkin_result(true, 10001, "签到活动已结束"),
        CheckinOutcome::Inactive
    );
    assert_eq!(
        classify_checkin_result(false, 0, "签到活动未开启"),
        CheckinOutcome::Inactive
    );
}

#[test]
fn checkin_classify_failed() {
    assert_eq!(
        classify_checkin_result(true, 500, "系统繁忙"),
        CheckinOutcome::Failed
    );
    assert_eq!(
        classify_checkin_result(false, 0, ""),
        CheckinOutcome::Failed
    );
}

#[test]
fn checkin_http_error_marks_auth_expired() {
    // 401/403 必须带 AUTH_EXPIRED 前缀，调度器才能与「网络失败」区分开
    assert!(checkin_http_error(401, "unauthorized").starts_with(AUTH_EXPIRED_PREFIX));
    assert!(checkin_http_error(403, "forbidden").starts_with(AUTH_EXPIRED_PREFIX));
    assert!(!checkin_http_error(500, "boom").starts_with(AUTH_EXPIRED_PREFIX));
    assert_eq!(TRAVEL_AUTH_EXPIRED_PREFIX, AUTH_EXPIRED_PREFIX);
}
