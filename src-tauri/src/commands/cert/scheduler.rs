use super::*;

// ---------------------------------------------------------------------------
// 调度器
// ---------------------------------------------------------------------------

/// 启动应用内后台调度器（在 lib.rs setup 中调用）。
/// 节奏：启动 60 秒后先做首次扫描（避免到期证书要等满一个间隔——默认 6 小时——
/// 才被检查；也让调度页尽快有 last_run/next_run 可显示），之后按配置间隔循环。
pub fn start_scheduler(app: tauri::AppHandle) {
    crate::runtime_tasks::spawn_task("cert", "scheduler", async move {
        let app = app;
        // 首轮仅延迟 60s（尽快做首次扫描 + 让调度页有 last_run/next_run 可显示）；
        // 之后每次睡眠前读取最新 interval，用户在睡眠期间修改间隔时下一轮立即生效。
        let mut first = true;
        loop {
            // 读取间隔（短锁，不跨 await）
            let interval = {
                let st = app.state::<CertScheduler>();
                let s = st.state.lock().unwrap();
                s.interval_minutes
            };
            let delay_secs = if first {
                60u64
            } else {
                interval.saturating_mul(60).max(60)
            };
            first = false;
            tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;

            // 收集本周期需要续期的证书（短锁，不跨 await）
            let due_certs: Vec<String> = {
                let st = app.state::<CertScheduler>();
                let s = st.state.lock().unwrap();
                if !s.enabled {
                    Vec::new()
                } else {
                    let certs = load_certs();
                    let now = Utc::now();
                    certs
                        .iter()
                        .filter_map(|c| {
                            let due = c.not_after.as_ref().and_then(|na| {
                                DateTime::parse_from_rfc3339(na).ok().map(|d| {
                                    now + Duration::days(c.renew_before_days)
                                        >= d.with_timezone(&Utc)
                                })
                            });
                            if due == Some(true) {
                                Some(c.id.clone())
                            } else {
                                None
                            }
                        })
                        .collect()
                }
            };

            for id in &due_certs {
                // 定时任务不强制：到没到期由 lego 按 renew_before_days 判断
                let result = issue_and_deploy(Some(app.clone()), &id, false).await;
                let (ok, message) = match &result {
                    Ok(r) if r.deployment.fail_count > 0 => (
                        false,
                        format!("证书已签发或仍有效，但 {} 个部署失败，请查看部署日志", r.deployment.fail_count),
                    ),
                    Ok(r) if r.renewed => (true, "已续期".to_string()),
                    Ok(r) => (
                        true,
                        format!(
                            "未到续期时间，已跳过（剩余 {} 天）",
                            r.days_left.unwrap_or(0)
                        ),
                    ),
                    Err(e) => (false, e.clone()),
                };
                let deployment_failed = result.as_ref().is_ok_and(|r| r.deployment.fail_count > 0);
                {
                    let st = app.state::<CertScheduler>();
                    let mut s = st.state.lock().unwrap();
                    s.push_log(SchedulerLogEntry {
                        at: Utc::now().to_rfc3339(),
                        cert_id: id.clone(),
                        action: if deployment_failed { "deploy" } else { "renew" }.into(),
                        ok,
                        message: message.clone(),
                    });
                }
                // 续期结果通过已启用的通知渠道推送（未配置渠道时无操作）
                let channels = load_json::<NotifyChannel>("notify_channels.json");
                notify_all(
                    &channels,
                    if deployment_failed {
                        "证书部署失败"
                    } else if ok {
                        "证书续期成功"
                    } else {
                        "证书续期失败"
                    },
                    &format!("证书 {}：{}", id, message),
                )
                .await;
            }

            // A fresh certificate is no longer due for issuance. Retry failed local deployments
            // from its existing PEM on later cycles, without requesting another ACME certificate.
            let retry_enabled = app.state::<CertScheduler>().state.lock().unwrap().enabled;
            if retry_enabled {
                super::local_retry::retry_local_deployments(&app, &due_certs).await;
            }

            // 到期扫描后顺带检查外部域名证书（同一调度周期）
            let scheduler_enabled = {
                let st = app.state::<CertScheduler>();
                let s = st.state.lock().unwrap();
                s.enabled
            };
            if scheduler_enabled {
                run_monitor_checks(true).await;
            }

            // 更新运行时间（短锁）
            let st = app.state::<CertScheduler>();
            let mut s = st.state.lock().unwrap();
            let now = Utc::now();
            s.last_run_at = Some(now.to_rfc3339());
            s.next_run_at = Some((now + Duration::minutes(interval as i64)).to_rfc3339());
        }
    });
}
