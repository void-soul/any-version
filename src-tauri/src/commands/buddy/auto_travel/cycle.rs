//! cycle responsibility.
use super::*;

/// 执行一轮自动旅行状态机（每轮「查状态 → 至多一个动作」）。
///
/// - 派出：仅当到达当日计划时间（或 force）且当日未派出过；计划时间已过（应用没开着 /
///   上一轮失败）时由 `reschedule_missed_plans` 重排到窗口内的新随机时间，不「立刻补派」；
/// - 领取：全天检测 arrived（旅行时长 1~4 小时随机，返回后下一轮领取）；
/// - 已达派出上限：当日完成；
/// - 窗口已全部过去：今天放弃（等明天跨天重规划）。
pub async fn run_auto_travel_cycle_if_needed(
    app: &AppHandle,
    force: bool,
) -> Result<String, String> {
    if IS_TRAVEL_RUNNING.swap(true, Ordering::SeqCst) {
        return Ok("already_running".to_string());
    }
    let _guard = TravelGuard;

    let mut config = get_config_checked()?;
    if !config.enabled && !force {
        return Ok("disabled".to_string());
    }

    // 旅行是 WorkBuddy 专属活动
    let accounts = store::list_accounts(BuddyPlatform::Workbuddy);
    if accounts.is_empty() {
        return Ok("no_accounts".to_string());
    }

    let now = Local::now();
    let current_minute = (now.hour() * 60 + now.minute()) as i32;
    let today_str = get_today_date_string();
    let (start_min, end_min) =
        super::super::auto_checkin::plan_window(&config.start_time, &config.end_time);
    let cross_day = needs_replan(&config, &accounts);
    let allow_generate = force
        || cross_day
        || current_minute >= super::super::auto_checkin::parse_time_to_minutes(&config.start_time);
    let before_schedules = config.account_schedules.clone().unwrap_or_default();
    let mut schedule_changed = ensure_travel_schedules(&mut config, &accounts, allow_generate);
    // 计划时间已过但今天还没派出（应用没开着 / 上一轮失败）→ 重排到窗口内的新随机时间，
    // 而不是「打开应用就立刻补派」。手动立即巡检（force）不参与重排。
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
        let _ = app.emit("buddy-auto-travel-config-changed", ());
    }

    let schedules = config.account_schedules.clone().unwrap_or_default();
    let mut new_schedules = schedules.clone();
    let mut acted = false;
    let mut auth_expired = false;
    let mut entries: Vec<action_log::BuddyActionLogEntry> = reschedule_entries;
    // 本轮到点、真正跑过状态机的账号（派出失败者要把重试时机挪到新的随机时刻）
    let mut attempted_ids: Vec<String> = Vec::new();

    for account in &accounts {
        let Some(sch) = schedules.get(&account.id).cloned() else {
            continue;
        };
        let email_display = if !account.email.trim().is_empty() {
            account.email.clone()
        } else {
            account.id.clone()
        };
        if sch.scheduled_date != today_str {
            continue;
        }
        // 登录态已被吊销：refresh 救不回来，自动派出只会撞 401（与签到侧同构）
        if !force && account.status.as_deref() == Some(super::super::api::STATUS_AUTH_REVOKED) {
            continue;
        }
        if sch.last_done_date.as_deref() == Some(&today_str) {
            continue; // 当日流程已完成
        }
        // 已派出过 → 继续跟领取（不受窗口限制：旅行可能在窗口结束后才回来）；
        // 否则只有「到点且仍在窗口内、没错过太久」才派出。
        let due = sch.last_depart_date.as_deref() == Some(&today_str)
            || force
            || super::super::auto_checkin::is_plan_due(
                sch.scheduled_minute,
                current_minute,
                end_min,
            );
        if !due {
            continue;
        }
        attempted_ids.push(account.id.clone());

        // 这笔旅行归属的日期：归来/领取都归档到出发那天
        // （跨零点时，比如 23:50 出发、次日 01:00 归来，仍记在出发日）
        let travel_day = sch
            .last_depart_date
            .clone()
            .unwrap_or_else(|| today_str.clone());

        let status = match api::travel_status(
            &account.access_token,
            account.uid.as_deref(),
            account.enterprise_id.as_deref(),
            account.domain.as_deref(),
        )
        .await
        {
            Ok(s) => s,
            Err(e) => {
                if e.starts_with(api::TRAVEL_AUTH_EXPIRED_PREFIX) {
                    auth_expired = true;
                    eprintln!("[BuddyAutoTravel] 账号 {} 令牌失效: {}", account.id, e);
                    entries.push(action_log::make_entry(
                        "travel",
                        &account.id,
                        &email_display,
                        "failed",
                        Some("令牌已失效，请打开 WorkBuddy 刷新登录态".to_string()),
                        None,
                    ));
                    archive_travel(
                        account,
                        &travel_day,
                        TravelPatch::new()
                            .status(travel_status::FAILED)
                            .message(Some("令牌已失效，请打开 WorkBuddy 刷新登录态".to_string())),
                    );
                    continue;
                }
                eprintln!(
                    "[BuddyAutoTravel] 账号 {} 查询旅行状态失败: {}",
                    account.id, e
                );
                entries.push(action_log::make_entry(
                    "travel",
                    &account.id,
                    &email_display,
                    "failed",
                    Some(e.clone()),
                    None,
                ));
                archive_travel(
                    account,
                    &travel_day,
                    TravelPatch::new()
                        .status(travel_status::FAILED)
                        .message(Some(e)),
                );
                continue;
            }
        };

        let Some(state) = new_schedules.get_mut(&account.id) else {
            continue;
        };

        if status.state == "arrived" {
            // 我们是"这一轮才发现它回来"，所以归来时间取当下，误差 ≤ 轮询间隔
            let back_time = now_time_string();
            let Some(record_id) = status.record_id.clone() else {
                eprintln!(
                    "[BuddyAutoTravel] 账号 {} arrived 但缺少 record_id",
                    account.id
                );
                entries.push(action_log::make_entry(
                    "travel",
                    &account.id,
                    &email_display,
                    "failed",
                    Some("旅行已归来但缺少 record_id，无法领取".to_string()),
                    None,
                ));
                // 它的确已经回来了，只是拿不到 record_id 领不了：按「已归来待领取」归档
                archive_travel(
                    account,
                    &travel_day,
                    TravelPatch::new()
                        .back_time(back_time)
                        .status(travel_status::ARRIVED)
                        .message(Some("旅行已归来但缺少 record_id，无法领取".to_string())),
                );
                continue;
            };
            match api::travel_claim(
                record_id,
                &account.access_token,
                account.uid.as_deref(),
                account.enterprise_id.as_deref(),
                account.domain.as_deref(),
            )
            .await
            {
                Ok(reward) => {
                    state.last_done_date = Some(today_str.clone());
                    state.last_reward_credit = reward;
                    acted = true;
                    entries.push(action_log::make_entry(
                        "travel",
                        &account.id,
                        &email_display,
                        "claimed",
                        Some("领取旅行奖励".to_string()),
                        reward,
                    ));
                    archive_travel(
                        account,
                        &travel_day,
                        TravelPatch::new()
                            .back_time(back_time.clone())
                            .claim_time(now_time_string())
                            .status(travel_status::CLAIMED)
                            .credit(reward),
                    );
                    eprintln!(
                        "[BuddyAutoTravel] 账号 {} 领取旅行奖励: {:?} 积分",
                        account.id, reward
                    );
                }
                Err(e) => {
                    if e.starts_with(super::super::api::TRAVEL_REJECTED_PREFIX)
                        && (e.contains("已领取") || e.to_lowercase().contains("already"))
                    {
                        // 奖励已被领取过（可能在客户端手动领了）→ 当日完成
                        state.last_done_date = Some(today_str.clone());
                        acted = true;
                        entries.push(action_log::make_entry(
                            "travel",
                            &account.id,
                            &email_display,
                            "claimed",
                            Some("奖励已在客户端领取".to_string()),
                            None,
                        ));
                        archive_travel(
                            account,
                            &travel_day,
                            TravelPatch::new()
                                .back_time(back_time)
                                .claim_time(now_time_string())
                                .status(travel_status::CLAIMED)
                                .message(Some("奖励已在客户端领取".to_string())),
                        );
                        eprintln!(
                            "[BuddyAutoTravel] 账号 {} 奖励已领取过，当日完成",
                            account.id
                        );
                    } else {
                        eprintln!(
                            "[BuddyAutoTravel] 账号 {} 领取失败（下轮重试）: {}",
                            account.id, e
                        );
                        entries.push(action_log::make_entry(
                            "travel",
                            &account.id,
                            &email_display,
                            "failed",
                            Some(e.clone()),
                            None,
                        ));
                        archive_travel(
                            account,
                            &travel_day,
                            TravelPatch::new()
                                .back_time(back_time)
                                .status(travel_status::FAILED)
                                .message(Some(e)),
                        );
                    }
                }
            }
        } else if status.state == "idle" && status.daily_limit_reached {
            // 今日已派出过（可能是在客户端手动派的）→ 当日完成
            state.last_done_date = Some(today_str.clone());
            acted = true;
            entries.push(action_log::make_entry(
                "travel",
                &account.id,
                &email_display,
                "limit_reached",
                Some("今日已达派出上限".to_string()),
                None,
            ));
            // 从没由我们派出过 → 说明名额被外部用掉了，次数至少记 1 次
            let patch = TravelPatch::new().status(travel_status::LIMIT_REACHED);
            let patch = if sch.last_depart_date.as_deref() == Some(&today_str) {
                patch
            } else {
                patch.add_depart()
            };
            archive_travel(account, &travel_day, patch);
        } else if status.state == "idle" {
            // 空闲且未达上限：到达计划时间后派出（当日只派一次）
            if sch.last_depart_date.as_deref() != Some(&today_str) {
                match api::travel_depart(
                    config.location_id,
                    &account.access_token,
                    account.uid.as_deref(),
                    account.enterprise_id.as_deref(),
                    account.domain.as_deref(),
                )
                .await
                {
                    Ok(()) => {
                        let depart_time = now_time_string();
                        state.last_depart_date = Some(today_str.clone());
                        state.last_depart_time = Some(depart_time.clone());
                        acted = true;
                        entries.push(action_log::make_entry(
                            "travel",
                            &account.id,
                            &email_display,
                            "departed",
                            Some(format!("已派出旅行（地点 {}）", config.location_id)),
                            None,
                        ));
                        archive_travel(
                            account,
                            &today_str,
                            TravelPatch::new()
                                .depart_time(depart_time)
                                .status(travel_status::TRAVELING)
                                .add_depart(),
                        );
                        eprintln!(
                            "[BuddyAutoTravel] 账号 {} 已派出旅行（地点 {}）",
                            account.id, config.location_id
                        );
                    }
                    Err(e) => {
                        if e.starts_with(super::super::api::TRAVEL_REJECTED_PREFIX) {
                            // 业务拒绝（如「今日太累了」等提示）：当日不再派出，
                            // 否则会以调度间隔无限重试打扰官方接口
                            state.last_done_date = Some(today_str.clone());
                            acted = true;
                            eprintln!(
                                "[BuddyAutoTravel] 账号 {} 派出被拒绝，当日放弃: {}",
                                account.id, e
                            );
                            entries.push(action_log::make_entry(
                                "travel",
                                &account.id,
                                &email_display,
                                "depart_rejected",
                                Some(e.clone()),
                                None,
                            ));
                            archive_travel(
                                account,
                                &travel_day,
                                TravelPatch::new()
                                    .status(travel_status::REJECTED)
                                    .message(Some(e)),
                            );
                        } else {
                            // 网络/服务端临时错误：下一轮重试
                            eprintln!(
                                "[BuddyAutoTravel] 账号 {} 派出失败（下轮重试）: {}",
                                account.id, e
                            );
                            entries.push(action_log::make_entry(
                                "travel",
                                &account.id,
                                &email_display,
                                "failed",
                                Some(e.clone()),
                                None,
                            ));
                            archive_travel(
                                account,
                                &travel_day,
                                TravelPatch::new()
                                    .status(travel_status::FAILED)
                                    .message(Some(e)),
                            );
                        }
                    }
                }
            }
        }
        // traveling：旅行中，等下一轮领取
    }

    // 派出失败（网络错误/令牌失效等）的重试时机也改成「窗口内新随机时间」：否则下一个
    // 轮询就会以「已到点」立刻重打同一个失败账号。
    let retry_moved = reschedule_failed_plans(
        &mut new_schedules,
        &attempted_ids,
        &today_str,
        current_minute,
        start_min,
        end_min,
    );

    config.account_schedules = Some(new_schedules);
    if retry_moved {
        // 计划时间被挪动过 → 同步归档（日历与卡片都以归档为准）
        archive_schedule_changes(&schedules, &config, &accounts);
    }
    save_config_without_wake(&config)?;
    action_log::append_action_logs(&entries)?;
    let _ = app.emit("buddy-action-logs-changed", ());
    let _ = app.emit("buddy-auto-travel-config-changed", ());

    // 失败不再让调度器拉长间隔：重试时机已改成「重排到窗口内的新随机时间」，
    // 轮询若退避到分钟以上就会错过这些随机时刻。失败信息已写进行为日志。
    if auth_expired {
        eprintln!(
            "[BuddyAutoTravel] 本轮存在令牌失效账号：已过计划时间的账号已重排到窗口内新随机时间（窗口已结束的今天放弃）"
        );
    }
    Ok("completed".to_string())
}
