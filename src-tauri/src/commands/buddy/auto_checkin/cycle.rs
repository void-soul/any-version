//! cycle responsibility.
use super::*;

pub async fn run_auto_checkin_cycle_if_needed(
    platform: BuddyPlatform,
    app: &AppHandle,
    force: bool,
) -> Result<String, String> {
    if IS_CHECKIN_RUNNING.swap(true, Ordering::SeqCst) {
        return Ok("already_running".to_string());
    }
    let _guard = CheckinGuard;

    let mut config = get_config_checked()?;
    if !config.enabled && !force {
        return Ok("disabled".to_string());
    }

    let accounts = store::list_accounts(platform);
    if accounts.is_empty() {
        return Ok("no_accounts".to_string());
    }

    // 生成今日计划的条件：用户强制 / 已到当天开始时间 / 发现过了一天（跨天重规划）。
    // 后者保证「每天打开应用后立刻重新计划」，而不必等到 startTime。
    let now = Local::now();
    let current_minute = (now.hour() * 60 + now.minute()) as i32;
    let (start_min, end_min) = plan_window(&config.start_time, &config.end_time);
    let cross_day = needs_replan(&config, &accounts);
    let allow_generate =
        force || cross_day || current_minute >= parse_time_to_minutes(&config.start_time);
    let before_schedules = config.account_schedules.clone().unwrap_or_default();
    let mut schedule_changed = ensure_account_schedules(&mut config, &accounts, allow_generate);
    // 计划时间已过但今天还没完成（应用没开着 / 上一轮失败）→ 重排到窗口内的新随机时间，
    // 而不是「打开应用就立刻补执行」。手动立即执行（force）不参与重排。
    let (rescheduled, reschedule_entries) = if force {
        (false, Vec::new())
    } else {
        reschedule_missed_plans(&mut config, &accounts, current_minute)
    };
    schedule_changed |= rescheduled;
    if schedule_changed {
        save_config_without_wake(&config)?;
        // 计划变更同步归档：旧日期收尾 + 今日计划时间
        archive_schedule_changes(&before_schedules, &config, &accounts);
        let _ = app.emit("buddy-auto-checkin-config-changed", ());
    }

    let today_str = get_today_date_string();
    // 退避闸用的当前时刻（秒）。force（立即执行）不走它 —— 用户主动点就该马上跑。
    let now_epoch = Local::now().timestamp();

    let target_accounts: Vec<&BuddyAccount> = if force {
        accounts.iter().collect()
    } else {
        accounts
            .iter()
            .filter(|account| {
                // 登录态已被吊销的账号不参与自动签到：refresh 都救不回来，
                // 每轮去撞一次 401 只会刷满失败日志（2026-10-23 起）。
                if account.status.as_deref() == Some(api::STATUS_AUTH_REVOKED) {
                    return false;
                }
                let sch = config
                    .account_schedules
                    .as_ref()
                    .and_then(|s| s.get(&account.id));
                match sch {
                    Some(s) => {
                        if s.last_checked_date.as_deref() == Some(&today_str) {
                            return false;
                        }
                        // 退避未到点：本轮跳过（失败后的 5s / 15s / 60s 等待）
                        if s.retry_at_epoch.is_some_and(|at| at > now_epoch) {
                            return false;
                        }
                        s.scheduled_date == today_str
                            && is_plan_due(s.scheduled_minute, current_minute, end_min)
                    }
                    None => false,
                }
            })
            .collect()
    };

    if target_accounts.is_empty() {
        // 本轮没有账号到点，但可能刚重排过计划：重排日志仍要落盘。
        if !reschedule_entries.is_empty() {
            action_log::append_action_logs(&reschedule_entries)?;
            let _ = app.emit("buddy-action-logs-changed", ());
        }
        return Ok("waiting".to_string());
    }

    eprintln!(
        "[BuddyAutoCheckin] 开始处理后台签到，平台={}, 目标账号数: {}",
        platform.as_str(),
        target_accounts.len()
    );

    let mut retry_needed = false;
    // 登录态过期标记（仅用于收尾日志；失败的重试时机已改为「重排到窗口内新随机时间」）
    let mut auth_expired = false;
    let mut entries: Vec<action_log::BuddyActionLogEntry> = reschedule_entries;
    let retry_before_schedules = config.account_schedules.clone().unwrap_or_default();
    let mut new_schedules = retry_before_schedules.clone();
    // 本轮失败的账号 + 该失败是否值得退避重试（4xx/登录态问题重试只是刷日志）
    let mut failed_ids: Vec<(String, bool)> = Vec::new();
    // 自动调度时账号之间留间隔：上游对「同一秒多个账号签到」会回 500/10001。
    // 「立即执行」（force）不加等待，否则 N 个账号要干等 3N 秒。
    let mut first_account = true;

    for account in target_accounts {
        if !force && !first_account {
            tokio::time::sleep(Duration::from_secs(CHECKIN_ACCOUNT_GAP_SECS)).await;
        }
        first_account = false;
        let email_display = if !account.email.trim().is_empty() {
            account.email.clone()
        } else {
            account.id.clone()
        };

        match api::get_checkin_status(
            &account.access_token,
            account.uid.as_deref(),
            account.enterprise_id.as_deref(),
            account.domain.as_deref(),
        )
        .await
        {
            Ok(status) if status.today_checked_in => {
                entries.push(action_log::make_entry(
                    "checkin",
                    &account.id,
                    &email_display,
                    "already_checked",
                    Some("今日已完成签到".to_string()),
                    Some(status.daily_credit),
                ));
                mark_schedule_checked(&mut new_schedules, &account.id, &today_str, current_minute);
                archive_checkin(
                    account,
                    CheckinPatch::new()
                        .actual_time(daily_history::now_time_string())
                        .status(checkin_status::ALREADY_CHECKED)
                        .source("kira")
                        .credit(Some(status.daily_credit)),
                );
            }
            Ok(status) if !status.active => {
                retry_needed = true;
                entries.push(action_log::make_entry(
                    "checkin",
                    &account.id,
                    &email_display,
                    "inactive",
                    Some("签到活动未开启或不适用".to_string()),
                    None,
                ));
                archive_checkin(
                    account,
                    CheckinPatch::new()
                        .status(checkin_status::INACTIVE)
                        .message(Some("签到活动未开启或不适用".to_string())),
                );
            }
            Ok(_) => match api::perform_checkin(
                &account.access_token,
                account.uid.as_deref(),
                account.enterprise_id.as_deref(),
                account.domain.as_deref(),
            )
            .await
            {
                Ok(res) if res.success && res.already == Some(true) => {
                    // 官方业务码 10001：今日已签到，无需再打接口
                    entries.push(action_log::make_entry(
                        "checkin",
                        &account.id,
                        &email_display,
                        "already_checked",
                        Some("今日已完成签到".to_string()),
                        None,
                    ));
                    mark_schedule_checked(
                        &mut new_schedules,
                        &account.id,
                        &today_str,
                        current_minute,
                    );
                    archive_checkin(
                        account,
                        CheckinPatch::new()
                            .actual_time(daily_history::now_time_string())
                            .status(checkin_status::ALREADY_CHECKED)
                            .source("kira"),
                    );
                }
                Ok(res) if res.inactive == Some(true) => {
                    // 官方明确回复「活动未开启/已结束」：状态记为 inactive（而非 failed）
                    retry_needed = true;
                    let message = res
                        .message
                        .clone()
                        .unwrap_or_else(|| "签到活动未开启或不适用".to_string());
                    entries.push(action_log::make_entry(
                        "checkin",
                        &account.id,
                        &email_display,
                        "inactive",
                        Some(message.clone()),
                        None,
                    ));
                    archive_checkin(
                        account,
                        CheckinPatch::new()
                            .status(checkin_status::INACTIVE)
                            .source("kira")
                            .message(Some(message)),
                    );
                }
                Ok(res) if res.success => {
                    let streak = res
                        .streak_days
                        .unwrap_or_else(|| account.checkin_streak.saturating_add(1));
                    let reward = res.reward.clone().or_else(|| {
                        res.credit
                            .map(|credit| serde_json::json!({ "credit": credit }))
                    });
                    let _ = api::update_checkin_info(
                        platform,
                        &account.id,
                        Some(Local::now().timestamp()),
                        streak,
                        reward,
                    );

                    entries.push(action_log::make_entry(
                        "checkin",
                        &account.id,
                        &email_display,
                        "success",
                        Some("签到成功".to_string()),
                        res.credit,
                    ));

                    mark_schedule_checked(
                        &mut new_schedules,
                        &account.id,
                        &today_str,
                        current_minute,
                    );
                    archive_checkin(
                        account,
                        CheckinPatch::new()
                            .actual_time(daily_history::now_time_string())
                            .status(checkin_status::SUCCESS)
                            .source("kira")
                            .credit(res.credit)
                            .streak(Some(streak))
                            .message(Some("签到成功".to_string())),
                    );
                }
                Ok(res) => {
                    match api::get_checkin_status(
                        &account.access_token,
                        account.uid.as_deref(),
                        account.enterprise_id.as_deref(),
                        account.domain.as_deref(),
                    )
                    .await
                    {
                        Ok(latest_status) if latest_status.today_checked_in => {
                            entries.push(action_log::make_entry(
                                "checkin",
                                &account.id,
                                &email_display,
                                "already_checked",
                                Some("今日已完成签到".to_string()),
                                None,
                            ));
                            mark_schedule_checked(
                                &mut new_schedules,
                                &account.id,
                                &today_str,
                                current_minute,
                            );
                            archive_checkin(
                                account,
                                CheckinPatch::new()
                                    .actual_time(daily_history::now_time_string())
                                    .status(checkin_status::ALREADY_CHECKED)
                                    .source("kira"),
                            );
                        }
                        _ => {
                            retry_needed = true;
                            // 业务层说「没签上」（活动未开启之类）：重试没有意义
                            failed_ids.push((account.id.clone(), false));
                            let message = res
                                .message
                                .clone()
                                .unwrap_or_else(|| "签到失败".to_string());
                            entries.push(action_log::make_entry(
                                "checkin",
                                &account.id,
                                &email_display,
                                "failed",
                                Some(message.clone()),
                                None,
                            ));
                            archive_checkin(
                                account,
                                CheckinPatch::new()
                                    .status(checkin_status::FAILED)
                                    .message(Some(message)),
                            );
                        }
                    }
                }
                Err(err) => {
                    eprintln!(
                        "[BuddyAutoCheckin] 账号 {} 自动签到异常: {}",
                        account.id, err
                    );
                    if err.starts_with(api::AUTH_EXPIRED_PREFIX) {
                        auth_expired = true;
                    }
                    retry_needed = true;
                    // 只有 5xx / 网络层错误才值得 5s 后重试；400/401/403 重试只是刷日志
                    failed_ids.push((account.id.clone(), checkin_error_is_retryable(&err)));
                    entries.push(action_log::make_entry(
                        "checkin",
                        &account.id,
                        &email_display,
                        "failed",
                        Some(err.clone()),
                        None,
                    ));
                    archive_checkin(
                        account,
                        CheckinPatch::new()
                            .status(checkin_status::FAILED)
                            .message(Some(err)),
                    );
                }
            },
            Err(err) => {
                eprintln!(
                    "[BuddyAutoCheckin] 账号 {} 签到状态检查异常: {}",
                    account.id, err
                );
                retry_needed = true;
                failed_ids.push((account.id.clone(), checkin_error_is_retryable(&err)));
                entries.push(action_log::make_entry(
                    "checkin",
                    &account.id,
                    &email_display,
                    "failed",
                    Some(err.clone()),
                    None,
                ));
                archive_checkin(
                    account,
                    CheckinPatch::new()
                        .status(checkin_status::FAILED)
                        .message(Some(err)),
                );
            }
        }
    }

    // 失败账号按退避重试（5s/15s/60s，之后固定 60s）。不做退避的话，下一个轮询
    //（30 秒后）会以「已到点」立刻重打同一个失败账号 —— 上游对此回 500/10001。
    let retry_moved = reschedule_failed_plans(
        &mut new_schedules,
        &failed_ids,
        &today_str,
        current_minute,
        start_min,
        end_min,
        now_epoch,
    );

    config.account_schedules = Some(new_schedules);
    if retry_moved {
        // 计划时间被挪动过 → 同步归档（日历与卡片都以归档为准）
        archive_schedule_changes(&retry_before_schedules, &config, &accounts);
    }
    save_config_without_wake(&config)?;

    // 行为日志（平铺）：每账号一条
    action_log::append_action_logs(&entries)?;
    let _ = app.emit("buddy-action-logs-changed", ());
    let _ = app.emit("buddy-auto-checkin-config-changed", ());

    // 失败不再让调度器拉长间隔：重试时机已改成「重排到窗口内的新随机时间」，
    // 轮询若退避到分钟以上就会错过这些随机时刻。失败信息已写进行为日志。
    if auth_expired || retry_needed {
        eprintln!(
            "[BuddyAutoCheckin] 本轮存在失败：已过计划时间的账号已重排到窗口内新随机时间（窗口已结束的今天放弃）"
        );
    }
    Ok("completed".to_string())
}
