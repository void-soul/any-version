use super::*;

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

#[test]
fn test_validate_config_rejects_bad_time_and_location() {
    let mut config = BuddyAutoTravelConfig::default();
    assert!(validate_config(&config).is_ok());

    config.end_time = "06:00".to_string(); // start 08:00 > end
    assert!(validate_config(&config).is_err());

    config.start_time = "08:00".to_string();
    config.end_time = "12:00".to_string();
    config.location_id = 5;
    assert!(validate_config(&config).is_err());
    config.location_id = 1;
    assert!(validate_config(&config).is_ok());
}

#[test]
fn test_ensure_travel_schedules_respects_allow_generate() {
    let mut config = BuddyAutoTravelConfig::default();
    let accounts = vec![sample_account("acc_1", "a@x.com")];

    // 未到开始时间：不生成
    assert!(!ensure_travel_schedules(&mut config, &accounts, false));
    assert!(config.account_schedules.is_none());

    // 到达开始时间：生成，分钟落在窗口内
    assert!(ensure_travel_schedules(&mut config, &accounts, true));
    let schedules = config.account_schedules.clone().unwrap();
    let sch = schedules.get("acc_1").unwrap();
    assert_eq!(sch.scheduled_date, get_today_date_string());
    assert!((480..=720).contains(&sch.scheduled_minute));

    // 同日重新生成保留当日进度
    if let Some(map) = config.account_schedules.as_mut() {
        let sch = map.get_mut("acc_1").unwrap();
        sch.last_depart_date = Some(get_today_date_string());
    }
    // 把 scheduled_minute 移出窗口强制重新生成
    if let Some(map) = config.account_schedules.as_mut() {
        map.get_mut("acc_1").unwrap().scheduled_minute = 0;
    }
    assert!(ensure_travel_schedules(&mut config, &accounts, true));
    let sch = config.account_schedules.unwrap().remove("acc_1").unwrap();
    assert_eq!(
        sch.last_depart_date.as_deref(),
        Some(get_today_date_string().as_str())
    );
}

#[test]
fn test_travel_reschedule_missed_plan_moves_to_future_and_logs() {
    let accounts = vec![sample_account("acc_1", "a@x.com")];
    let today = get_today_date_string();
    // 窗口 08:00~12:00（480~720），计划 08:30 没派出，现在 10:00
    let mut config = travel_config_with(travel_state(&today, 510));
    let (changed, entries) = reschedule_missed_plans(&mut config, &accounts, 600);
    assert!(changed);
    let schedule = config.account_schedules.unwrap().remove("acc_1").unwrap();
    assert!((601..=720).contains(&schedule.scheduled_minute));
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].kind, "travel");
    assert_eq!(entries[0].status, "rescheduled");
}

#[test]
fn test_travel_reschedule_skips_departed_done_and_closed_window() {
    let accounts = vec![sample_account("acc_1", "a@x.com")];
    let today = get_today_date_string();
    let build = |mutate: fn(&mut BuddyAccountTravelState)| {
        let mut state = travel_state(&today, 510);
        mutate(&mut state);
        travel_config_with(state)
    };

    // 已派出（旅行中 / 已归来待领取）→ 不重排
    let mut config = build(|s| s.last_depart_date = Some(get_today_date_string()));
    assert!(!reschedule_missed_plans(&mut config, &accounts, 600).0);

    // 当日已完成 → 不重排
    let mut config = build(|s| s.last_done_date = Some(get_today_date_string()));
    assert!(!reschedule_missed_plans(&mut config, &accounts, 600).0);

    // 窗口已全部过去（现在 13:20）→ 今天放弃：计划不动、不写日志
    let mut config = build(|_| {});
    let (changed, entries) = reschedule_missed_plans(&mut config, &accounts, 800);
    assert!(!changed);
    assert!(entries.is_empty());
    assert_eq!(
        config
            .account_schedules
            .unwrap()
            .get("acc_1")
            .unwrap()
            .scheduled_minute,
        510
    );

    // 还在宽限内（08:30 的计划，现在 08:33）→ 不重排，照旧立刻派出
    let mut config = build(|_| {});
    assert!(!reschedule_missed_plans(&mut config, &accounts, 513).0);
}

#[test]
fn test_travel_reschedule_failed_plans_skips_departed_and_done() {
    let today = get_today_date_string();
    let mut traveling = travel_state(&today, 600);
    traveling.last_depart_date = Some(today.clone());
    let mut done = travel_state(&today, 600);
    done.last_done_date = Some(today.clone());
    let mut schedules = HashMap::from([
        ("acc_failed".to_string(), travel_state(&today, 600)),
        ("acc_traveling".to_string(), traveling),
        ("acc_done".to_string(), done),
    ]);
    let attempted = vec![
        "acc_failed".to_string(),
        "acc_traveling".to_string(),
        "acc_done".to_string(),
    ];

    // 窗口 08:00~12:00，现在 10:00 → 只有派出失败的账号挪到窗口内未来时间
    assert!(reschedule_failed_plans(
        &mut schedules,
        &attempted,
        &today,
        600,
        480,
        720
    ));
    assert!((601..=720).contains(&schedules["acc_failed"].scheduled_minute));
    assert_eq!(schedules["acc_traveling"].scheduled_minute, 600);
    assert_eq!(schedules["acc_done"].scheduled_minute, 600);
}

#[test]
fn test_config_serde_roundtrip() {
    let config = BuddyAutoTravelConfig {
        enabled: true,
        start_time: "08:00".to_string(),
        end_time: "10:00".to_string(),
        location_id: 3,
        account_schedules: Some(HashMap::from([(
            "acc_1".to_string(),
            BuddyAccountTravelState {
                scheduled_date: "2026-09-13".to_string(),
                scheduled_minute: 500,
                last_depart_date: Some("2026-09-13".to_string()),
                last_depart_time: Some("08:32:11".to_string()),
                last_done_date: None,
                last_reward_credit: Some(8),
            },
        )])),
    };
    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("\"locationId\":3"));
    let deserialized: BuddyAutoTravelConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, config);
}

fn travel_state(date: &str, minute: i32) -> BuddyAccountTravelState {
    BuddyAccountTravelState {
        scheduled_date: date.to_string(),
        scheduled_minute: minute,
        last_depart_date: None,
        last_depart_time: None,
        last_done_date: None,
        last_reward_credit: None,
    }
}

/// 窗口固定 08:00~12:00（480~720）的单账号配置，便于用注入的 `current_minute` 做确定性测试。
fn travel_config_with(state: BuddyAccountTravelState) -> BuddyAutoTravelConfig {
    BuddyAutoTravelConfig {
        enabled: true,
        start_time: "08:00".to_string(),
        end_time: "12:00".to_string(),
        location_id: 1,
        account_schedules: Some(HashMap::from([("acc_1".to_string(), state)])),
    }
}

#[test]
fn test_needs_replan_cross_day_and_empty_accounts() {
    let accounts = vec![sample_account("acc_1", "a@x.com")];
    let today = super::super::auto_checkin::get_today_date_string();

    // 没有计划 → 需要
    let mut config = BuddyAutoTravelConfig::default();
    assert!(needs_replan(&config, &accounts));
    // 没有账号 → 不需要（避免空转）
    assert!(!needs_replan(&config, &[]));

    // 今天的计划齐了 → 不需要
    config.account_schedules = Some(HashMap::from([(
        "acc_1".to_string(),
        travel_state(&today, 540),
    )]));
    assert!(!needs_replan(&config, &accounts));

    // 昨天的计划 → 需要（"打开应用即重新计划"）
    config.account_schedules = Some(HashMap::from([(
        "acc_1".to_string(),
        travel_state("2020-01-01", 540),
    )]));
    assert!(needs_replan(&config, &accounts));
}

#[test]
fn test_cross_day_replan_clears_yesterday_progress() {
    let accounts = vec![sample_account("acc_1", "a@x.com")];
    let today = super::super::auto_checkin::get_today_date_string();
    let mut config = BuddyAutoTravelConfig {
        enabled: true,
        start_time: "08:00".to_string(),
        end_time: "12:00".to_string(),
        location_id: 1,
        account_schedules: Some(HashMap::from([(
            "acc_1".to_string(),
            BuddyAccountTravelState {
                scheduled_date: "2020-01-01".to_string(),
                scheduled_minute: 540,
                last_depart_date: Some("2020-01-01".to_string()),
                last_depart_time: Some("09:05:00".to_string()),
                last_done_date: Some("2020-01-01".to_string()),
                last_reward_credit: Some(7),
            },
        )])),
    };

    assert!(ensure_travel_schedules(&mut config, &accounts, true));
    let schedule = config
        .account_schedules
        .unwrap()
        .remove("acc_1")
        .expect("schedule");
    assert_eq!(schedule.scheduled_date, today);
    // 昨天的派出/完成状态不会带到今天（即"每天清空"）
    assert_eq!(schedule.last_depart_date, None);
    assert_eq!(schedule.last_depart_time, None);
    assert_eq!(schedule.last_done_date, None);
    assert_eq!(schedule.last_reward_credit, None);
    assert!((480..=720).contains(&schedule.scheduled_minute));
}

#[test]
fn test_retry_delay_bounded_exponential_backoff() {
    assert_eq!(
        next_retry_delay(INITIAL_RETRY_DELAY),
        Duration::from_secs(10 * 60)
    );
    assert_eq!(next_retry_delay(MAX_RETRY_DELAY), MAX_RETRY_DELAY);
}
