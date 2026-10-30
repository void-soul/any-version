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
fn test_parse_time_to_minutes() {
    assert_eq!(parse_time_to_minutes("06:00"), 360);
    assert_eq!(parse_time_to_minutes("12:30"), 750);
    assert_eq!(parse_time_to_minutes("00:00"), 0);
    assert_eq!(parse_time_to_minutes("23:59"), 1439);
    assert_eq!(parse_time_to_minutes("invalid"), 0);
}

#[test]
fn test_ensure_account_schedules() {
    let mut config = BuddyAutoCheckinConfig {
        enabled: true,
        start_time: "06:00".to_string(),
        end_time: "12:00".to_string(),
        last_checked_date: None,
        account_schedules: None,
    };

    let accounts = vec![
        sample_account("acc_1", "a@x.com"),
        sample_account("acc_2", "b@x.com"),
    ];
    let changed = ensure_account_schedules(&mut config, &accounts, true);
    assert!(changed);

    let schedules = config.account_schedules.unwrap();
    assert_eq!(schedules.len(), 2);
    let sch1 = schedules.get("acc_1").unwrap();
    assert!(sch1.scheduled_minute >= 360 && sch1.scheduled_minute <= 720);
    assert_eq!(sch1.scheduled_date, get_today_date_string());
}

#[test]
fn test_ensure_account_schedules_skips_before_start_time() {
    let mut config = BuddyAutoCheckinConfig {
        enabled: true,
        start_time: "06:00".to_string(),
        end_time: "12:00".to_string(),
        last_checked_date: None,
        account_schedules: None,
    };
    let accounts = vec![sample_account("acc_1", "a@x.com")];
    // 未到达开始时间：不生成当天计划
    assert!(!ensure_account_schedules(&mut config, &accounts, false));
    assert!(config.account_schedules.is_none());
    // 到达开始时间：生成
    assert!(ensure_account_schedules(&mut config, &accounts, true));
    assert!(config.account_schedules.unwrap().contains_key("acc_1"));
}

fn schedule_state(date: &str, minute: i32) -> BuddyAccountScheduleState {
    BuddyAccountScheduleState {
        scheduled_date: date.to_string(),
        scheduled_minute: minute,
        last_checked_date: None,
        last_checked_time: None,
        retry_count: 0,
        retry_at_epoch: None,
    }
}

/// 窗口固定 06:00~12:00（360~720）的配置，便于用注入的 `current_minute` 做确定性测试。
fn config_with_schedules(states: Vec<(&str, BuddyAccountScheduleState)>) -> BuddyAutoCheckinConfig {
    BuddyAutoCheckinConfig {
        enabled: true,
        start_time: "06:00".to_string(),
        end_time: "12:00".to_string(),
        last_checked_date: None,
        account_schedules: Some(
            states
                .into_iter()
                .map(|(id, state)| (id.to_string(), state))
                .collect(),
        ),
    }
}

#[test]
fn test_remaining_window_and_plan_due() {
    // 窗口 06:00~12:00，现在 10:00 → 剩余可用区间从「现在」之后开始
    assert_eq!(remaining_window(360, 720, 600), Some((601, 720)));
    // 还没到开始时间 → 从开始时间起
    assert_eq!(remaining_window(360, 720, 60), Some((360, 720)));
    // 已到 / 已过结束时间 → 今天没有剩余窗口（今天就放弃）
    assert_eq!(remaining_window(360, 720, 720), None);
    assert_eq!(remaining_window(360, 720, 900), None);

    assert!(is_plan_due(600, 600, 720)); // 到点当刻
    assert!(is_plan_due(600, 605, 720)); // 宽限内的小延迟照旧执行
    assert!(!is_plan_due(600, 606, 720)); // 错过超过宽限 → 交给重排，不立刻补执行
    assert!(!is_plan_due(600, 601, 600)); // 窗口已过 → 今天放弃
    assert!(!is_plan_due(700, 600, 720)); // 还没到点
}

#[test]
fn test_draw_plan_minute_prefers_future_within_window() {
    // 窗口内还有剩余时间 → 一定抽在未来（晚开应用不会一生成就「立刻执行」）
    for _ in 0..64 {
        let minute = draw_plan_minute(360, 720, 600);
        assert!((601..=720).contains(&minute), "minute={}", minute);
    }
    // 窗口已全部过去 → 退回整窗随机（当天不会再执行，前端显示「未完成」）
    for _ in 0..64 {
        let minute = draw_plan_minute(360, 720, 900);
        assert!((360..=720).contains(&minute), "minute={}", minute);
    }
}

#[test]
fn test_reschedule_missed_plan_moves_to_future_and_logs() {
    let accounts = vec![sample_account("acc_1", "a@x.com")];
    let today = get_today_date_string();
    // 08:00 的计划没执行，现在 10:00
    let mut config = config_with_schedules(vec![("acc_1", schedule_state(&today, 480))]);

    let (changed, entries) = reschedule_missed_plans(&mut config, &accounts, 600);
    assert!(changed);
    let schedule = config.account_schedules.unwrap().remove("acc_1").unwrap();
    assert!((601..=720).contains(&schedule.scheduled_minute));
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].status, "rescheduled");
    let message = entries[0].message.clone().unwrap_or_default();
    assert!(message.contains("08:00"), "message={}", message);
}

#[test]
fn test_reschedule_skips_window_closed_checked_and_grace() {
    let accounts = vec![sample_account("acc_1", "a@x.com")];
    let today = get_today_date_string();

    // 窗口已全部过去（现在 13:20）→ 今天放弃：计划不动、不写日志
    let mut config = config_with_schedules(vec![("acc_1", schedule_state(&today, 480))]);
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
        480
    );

    // 今天已签到 → 不重排
    let mut state = schedule_state(&today, 480);
    state.last_checked_date = Some(today.clone());
    let mut config = config_with_schedules(vec![("acc_1", state)]);
    assert!(!reschedule_missed_plans(&mut config, &accounts, 600).0);

    // 还在宽限内（08:00 的计划，现在 08:03）→ 不重排，照旧立刻执行
    let mut config = config_with_schedules(vec![("acc_1", schedule_state(&today, 480))]);
    assert!(!reschedule_missed_plans(&mut config, &accounts, 483).0);
}

#[test]
fn test_reschedule_failed_plans_backs_off_and_skips_checked() {
    let today = get_today_date_string();
    let mut checked = schedule_state(&today, 600);
    checked.last_checked_date = Some(today.clone());
    let mut schedules = HashMap::from([
        ("acc_failed".to_string(), schedule_state(&today, 600)),
        ("acc_done".to_string(), checked),
    ]);
    let attempted = vec![
        ("acc_failed".to_string(), true),
        ("acc_done".to_string(), true),
    ];
    let now = 1_700_000_000_i64;

    // 首次失败：计划分钟不动（不再「一分钟内重打」），改为 5 秒后允许重试
    assert!(reschedule_failed_plans(
        &mut schedules,
        &attempted,
        &today,
        600,
        360,
        720,
        now
    ));
    assert_eq!(schedules["acc_failed"].scheduled_minute, 600);
    assert_eq!(schedules["acc_failed"].retry_count, 1);
    assert_eq!(schedules["acc_failed"].retry_at_epoch, Some(now + 5));
    // 已签到的账号不受影响
    assert_eq!(schedules["acc_done"].retry_at_epoch, None);

    // 第二次失败 → 15 秒（退避逐次加长）
    assert!(reschedule_failed_plans(
        &mut schedules,
        &attempted,
        &today,
        600,
        360,
        720,
        now + 5
    ));
    assert_eq!(schedules["acc_failed"].retry_count, 2);
    assert_eq!(schedules["acc_failed"].retry_at_epoch, Some(now + 5 + 15));

    // 窗口已全部过去 → 不改动（今天放弃）
    let mut schedules = HashMap::from([("acc_failed".to_string(), schedule_state(&today, 600))]);
    assert!(!reschedule_failed_plans(
        &mut schedules,
        &attempted,
        &today,
        800,
        360,
        720,
        now
    ));
    assert_eq!(schedules["acc_failed"].retry_at_epoch, None);
}

#[test]
fn test_reschedule_failed_plans_clears_state_for_non_retryable() {
    let today = get_today_date_string();
    let mut state = schedule_state(&today, 600);
    state.retry_count = 3;
    state.retry_at_epoch = Some(1);
    let mut schedules = HashMap::from([("acc_failed".to_string(), state)]);
    let attempted = vec![("acc_failed".to_string(), false)];

    assert!(reschedule_failed_plans(
        &mut schedules,
        &attempted,
        &today,
        600,
        360,
        720,
        1_700_000_000
    ));
    // 4xx / 登录态问题：不排重试，退避状态清空（重试只会刷日志）
    assert_eq!(schedules["acc_failed"].retry_count, 0);
    assert_eq!(schedules["acc_failed"].retry_at_epoch, None);
}

#[test]
fn test_checkin_retry_delay_sequence_and_cap() {
    assert_eq!(checkin_retry_delay_secs(0), 5);
    assert_eq!(checkin_retry_delay_secs(1), 5);
    assert_eq!(checkin_retry_delay_secs(2), 15);
    assert_eq!(checkin_retry_delay_secs(3), 60);
    // 用尽后固定末位，不无限增长
    assert_eq!(checkin_retry_delay_secs(9), 60);
}

#[test]
fn test_checkin_error_is_retryable() {
    // 5xx / 408 / 网络层 → 值得等一会儿重试
    assert!(checkin_error_is_retryable(&api::checkin_http_error(
        500, "boom"
    )));
    assert!(checkin_error_is_retryable(&api::checkin_http_error(
        502,
        "bad gateway"
    )));
    assert!(checkin_error_is_retryable(&api::checkin_http_error(
        408, "timeout"
    )));
    assert!(checkin_error_is_retryable(
        "请求 daily-checkin 失败: connection reset"
    ));
    // 4xx（含登录态问题）→ 不重试
    assert!(!checkin_error_is_retryable(&api::checkin_http_error(
        400,
        "bad request"
    )));
    assert!(!checkin_error_is_retryable(&api::checkin_http_error(
        401,
        "unauthorized"
    )));
    assert!(!checkin_error_is_retryable(&format!(
        "{}dead",
        api::AUTH_REVOKED_PREFIX
    )));
}

#[test]
fn test_needs_replan_cross_day_and_new_account() {
    let accounts = vec![
        sample_account("acc_1", "a@x.com"),
        sample_account("acc_2", "b@x.com"),
    ];
    let today = get_today_date_string();

    // 完全没有计划（首次运行）→ 需要重规划
    let mut config = BuddyAutoCheckinConfig::default();
    assert!(needs_replan(&config, &accounts));

    // 两个账号都是今天的计划 → 不需要（已生成的随机时间不抖动）
    config.account_schedules = Some(HashMap::from([
        ("acc_1".to_string(), schedule_state(&today, 400)),
        ("acc_2".to_string(), schedule_state(&today, 500)),
    ]));
    assert!(!needs_replan(&config, &accounts));

    // 昨天遗留的计划 → 需要重规划（"打开应用即重新计划"就靠这条）
    config.account_schedules = Some(HashMap::from([
        ("acc_1".to_string(), schedule_state("2020-01-01", 400)),
        ("acc_2".to_string(), schedule_state(&today, 500)),
    ]));
    assert!(needs_replan(&config, &accounts));

    // 新增账号（计划表里没有它）→ 需要重规划
    config.account_schedules = Some(HashMap::from([(
        "acc_1".to_string(),
        schedule_state(&today, 400),
    )]));
    assert!(needs_replan(&config, &accounts));

    // 没有账号 → 不需要（避免空转）
    assert!(!needs_replan(&config, &[]));
}

#[test]
fn test_cross_day_replan_resets_daily_state() {
    let accounts = vec![sample_account("acc_1", "a@x.com")];
    let today = get_today_date_string();
    let mut config = BuddyAutoCheckinConfig {
        enabled: true,
        start_time: "06:00".to_string(),
        end_time: "12:00".to_string(),
        last_checked_date: None,
        account_schedules: Some(HashMap::from([(
            "acc_1".to_string(),
            BuddyAccountScheduleState {
                scheduled_date: "2020-01-01".to_string(),
                scheduled_minute: 400,
                last_checked_date: Some("2020-01-01".to_string()),
                last_checked_time: Some("06:40:00".to_string()),
                retry_count: 0,
                retry_at_epoch: None,
            },
        )])),
    };

    // 跨天重规划：计划日期变成今天，昨天的签到状态不会带到今天（即"每天清空"）
    assert!(ensure_account_schedules(&mut config, &accounts, true));
    let schedule = config
        .account_schedules
        .unwrap()
        .remove("acc_1")
        .expect("schedule");
    assert_eq!(schedule.scheduled_date, today);
    assert_eq!(schedule.last_checked_date, None);
    assert_eq!(schedule.last_checked_time, None);
    assert!(schedule.scheduled_minute >= 360 && schedule.scheduled_minute <= 720);
}

#[test]
fn test_format_minutes() {
    assert_eq!(format_minutes(0), "00:00");
    assert_eq!(format_minutes(390), "06:30");
    assert_eq!(format_minutes(1439), "23:59");
    assert_eq!(format_minutes(2000), "23:59");
}

fn detail_with_status(status: &str) -> action_log::BuddyActionLogEntry {
    action_log::BuddyActionLogEntry {
        id: "log_test".to_string(),
        timestamp: "2026-09-13 08:00:00".to_string(),
        date: "2026-09-13".to_string(),
        kind: "checkin".to_string(),
        account_id: "acc_1".to_string(),
        email: "a@x.com".to_string(),
        status: status.to_string(),
        message: None,
        credit: None,
    }
}

#[test]
fn test_derive_task_status() {
    // 无记录 → 待签到
    assert_eq!(derive_task_status(false, None), "pending");
    // 计划已标记完成 → 已签到
    assert_eq!(derive_task_status(true, None), "success");
    // 今日日志显示成功/已签到 → 已签到
    assert_eq!(
        derive_task_status(false, Some(&detail_with_status("success"))),
        "success"
    );
    assert_eq!(
        derive_task_status(false, Some(&detail_with_status("already_checked"))),
        "success"
    );
    // 今日日志显示失败/未开启 → 失败（会继续重试）
    assert_eq!(
        derive_task_status(false, Some(&detail_with_status("failed"))),
        "failed"
    );
    assert_eq!(
        derive_task_status(false, Some(&detail_with_status("inactive"))),
        "failed"
    );
    // 已签到优先于历史失败记录
    assert_eq!(
        derive_task_status(true, Some(&detail_with_status("failed"))),
        "success"
    );
}

#[test]
fn test_config_serde_roundtrip() {
    let config = BuddyAutoCheckinConfig {
        enabled: true,
        start_time: "08:00".to_string(),
        end_time: "10:00".to_string(),
        last_checked_date: Some("2026-09-01".to_string()),
        account_schedules: None,
    };
    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("\"enabled\":true"));
    assert!(json.contains("\"startTime\":\"08:00\""));
    let deserialized: BuddyAutoCheckinConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, config);
}

#[test]
fn test_validation_rejects_invalid_time_and_schedule() {
    let mut config = BuddyAutoCheckinConfig {
        enabled: true,
        start_time: "12:00".to_string(),
        end_time: "06:00".to_string(),
        last_checked_date: None,
        account_schedules: None,
    };
    assert!(validate_config(&config).is_err());

    config.start_time = "06:00".to_string();
    config.account_schedules = Some(HashMap::from([(
        "acc_1".to_string(),
        BuddyAccountScheduleState {
            scheduled_date: "2026-09-01".to_string(),
            scheduled_minute: 1440,
            last_checked_date: None,
            last_checked_time: None,
            retry_count: 0,
            retry_at_epoch: None,
        },
    )]));
    assert!(validate_config(&config).is_err());
}

#[test]
fn test_retry_delay_bounded_exponential_backoff() {
    assert_eq!(
        next_retry_delay(INITIAL_RETRY_DELAY),
        Duration::from_secs(10 * 60)
    );
    assert_eq!(
        next_retry_delay(Duration::from_secs(45 * 60)),
        MAX_RETRY_DELAY
    );
    assert_eq!(next_retry_delay(MAX_RETRY_DELAY), MAX_RETRY_DELAY);
}
