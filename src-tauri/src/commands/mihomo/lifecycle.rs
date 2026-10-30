use super::*;

pub fn init_state() -> MihomoState {
    // 数据目录跟随全局设置（config.data_dir，可改到非系统盘），而非 AppData。
    // 这样 mihomo 的全部数据（配置/geo/日志/订阅）都落在用户维护的数据目录下。
    let data_dir = crate::commands::config::get_data_dir().join("mihomo");
    std::fs::create_dir_all(&data_dir).ok();
    // 兜底：若压缩包已提前解压，先同步一次 geo 文件到 data_dir。
    // 主同步在 download_bin_assets 解压完成后调用 sync_mihomo_geo 完成。
    crate::commands::utils::sync_mihomo_geo();
    let mut app_config = load_app_config(&data_dir);
    let controled_config = load_controled(&data_dir);
    let mut profile_config = load_profile_config(&data_dir);
    let override_config = load_override_config(&data_dir);

    migrate_legacy(&data_dir, &mut app_config, &mut profile_config);

    if profile_config.items.is_empty() {
        profile_config.items.push(ProfileItem {
            id: "default".into(),
            name: "默认".into(),
            type_: "subscription".into(),
            file_path: None,
            subscriptions: vec![],
            rule_providers: vec![],
            custom_rules: vec![],
            dns_enabled: true,
            dns_nameservers: vec!["https://dns.google".into(), "https://1.1.1.1".into()],
            update_cron: None,
            age_secret_key: None,
            url: None,
            auth_token: None,
            user_agent: None,
            use_proxy: false,
            auto_update: false,
            skip_verify: false,
            update_interval: 86400,
            update_timeout: 30,
            override_ids: vec![],
            subscription_userinfo: None,
            updated_at: None,
        });
        profile_config.current = "default".into();
        app_config.current_profile = "default".into();
        save_profile_config(&data_dir, &profile_config).ok();
        save_app_config(&data_dir, &app_config).ok();
    }

    let inner = MihomoInner {
        app_config: Mutex::new(app_config),
        controled_config: Mutex::new(controled_config),
        profile_config: Mutex::new(profile_config),
        override_config: Mutex::new(override_config),
        child: Mutex::new(None),
        launched_by_us: Arc::new(AtomicBool::new(false)),
        data_dir: data_dir.clone(),
        stop_flag: Arc::new(AtomicBool::new(true)),
        watchdog_running: Arc::new(AtomicBool::new(false)),
        scheduler_running: Arc::new(AtomicBool::new(false)),
        runtime_config_str: Mutex::new(String::new()),
        core_version: Mutex::new(None),
        auto_last: Mutex::new(HashMap::new()),
        log_file: data_dir.join("mihomo.log"),
        runtime_warnings: Mutex::new(Vec::new()),
    };
    let state = Arc::new(inner);
    refresh_core_version(&state);
    state
}

/// 定时更新远程订阅（对齐 clash-party profileUpdater：按 item.interval 重新下载并热重载）
pub(super) async fn auto_update_profiles(app: &AppHandle, inner: &Arc<MihomoInner>) {
    let now = now_secs();
    let (items, current) = {
        let cfg = inner.profile_config.lock().unwrap();
        (cfg.items.clone(), cfg.current.clone())
    };
    for item in items {
        if !item.auto_update || item.update_interval == 0 {
            continue;
        }
        let Some(url) = item.url.clone() else {
            continue;
        };
        // cron 优先：命中即更新（同一分钟只跑一次），没配 cron 才看秒级间隔
        let by_cron = cron_hit_now(item.update_cron.as_deref());
        if by_cron {
            let key = minute_key();
            let mut last = inner.auto_last.lock().unwrap();
            let slot = format!("profile:{}", item.id);
            if last.get(&slot).copied() == Some(key) {
                continue;
            }
            last.insert(slot, key);
        } else if now.saturating_sub(item.updated_at.unwrap_or(0)) < item.update_interval {
            continue;
        }
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(item.update_timeout.max(5)))
            .build()
        {
            Ok(c) => c,
            Err(_) => continue,
        };
        let mut req = client.get(&url).header(
            "User-Agent",
            item.user_agent
                .clone()
                .unwrap_or_else(|| "clash-verge/v1.7.0".into()),
        );
        if let Some(token) = &item.auth_token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }
        let Ok(resp) = req.send().await else {
            eprintln!("[mihomo] 订阅自动更新失败: {}", item.name);
            continue;
        };
        let userinfo = resp
            .headers()
            .get("subscription-userinfo")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string());
        let Ok(text) = resp.text().await else {
            continue;
        };
        // 嗅探并处理内容（age 解密 / base64 节点解码 / YAML），保持与手动更新一致
        let processed = match crate::commands::mihomo::subparse::process_subscription_content(
            &text,
            item.age_secret_key.as_deref(),
        ) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[mihomo] 订阅自动更新内容解析失败 {}: {}", item.name, e);
                continue;
            }
        };
        if serde_yaml::from_str::<Value>(&processed).is_err() {
            eprintln!("[mihomo] 订阅自动更新内容非法 YAML: {}", item.name);
            continue;
        }
        let mut updated = item.clone();
        updated.subscription_userinfo = parse_subinfo(userinfo.as_deref());
        updated.updated_at = Some(now);
        if write_profile_content(&inner.data_dir, &updated, &processed).is_err() {
            continue;
        }
        {
            let mut cfg = inner.profile_config.lock().unwrap();
            if let Some(it) = cfg.items.iter_mut().find(|i| i.id == updated.id) {
                *it = updated.clone();
            }
            save_profile_config(&inner.data_dir, &cfg).ok();
        }
        eprintln!("[mihomo] 订阅已自动更新: {}", updated.name);
        // 事件推送：前端不必靠 3s 轮询才知道订阅变了（clash-party 用
        // `profileConfigUpdated` 推给渲染进程，这里同样是「改完即推」）。
        let _ = app.emit(
            "mihomo://subscription-updated",
            json!({
                "id": updated.id,
                "name": updated.name,
                "updatedAt": now,
                "isCurrent": current == updated.id,
            }),
        );
        if current == updated.id && !inner.stop_flag.load(std::sync::atomic::Ordering::SeqCst) {
            let _ = reload_config(app, Arc::clone(inner)).await;
        }
    }
}

pub fn start_scheduler(_app: AppHandle, inner: Arc<MihomoInner>) {
    if !crate::runtime_tasks::is_enabled("mihomo") {
        return;
    }
    if inner.scheduler_running.load(Ordering::SeqCst) {
        return;
    }
    inner.scheduler_running.store(true, Ordering::SeqCst);
    // 流量采样下沉到主进程：渲染进程那套（切页/关窗就断采）只作补充
    crate::commands::mihomo::traffic::start_recorder(Arc::clone(&inner));
    crate::runtime_tasks::spawn_task("mihomo", "scheduler", async move {
        // 启动即刷新：对齐 clash-party `autoUpdateProfileOnStart`。
        // 不开的话订阅只在间隔/cron 到期才更新，刚开机那段时间用的是几天前的节点。
        let refresh_on_start = inner
            .app_config
            .lock()
            .unwrap()
            .auto_update_profile_on_start;
        if refresh_on_start {
            eprintln!("[mihomo] 启动即刷新订阅");
            auto_update_profiles(&_app, &inner).await;
        }
        // SSID 感知也在这里先跑一次，避免开机后要等一分钟才切到对应配置
        apply_ssid_rules(&_app, &inner).await;
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            if inner.stop_flag.load(Ordering::SeqCst) {
                inner.scheduler_running.store(false, Ordering::SeqCst);
                break;
            }
            let now = now_secs();
            auto_update_profiles(&_app, &inner).await;
            apply_ssid_rules(&_app, &inner).await;
            let profile = inner.current_profile();
            if profile.type_ != "subscription" {
                continue;
            }
            let mut to_update: Vec<String> = vec![];
            for s in &profile.subscriptions {
                if !s.auto_update {
                    continue;
                }
                if cron_hit_now(s.cron.as_deref()) {
                    let key = minute_key();
                    let mut last = inner.auto_last.lock().unwrap();
                    if last.get(&s.id).copied() == Some(key) {
                        continue;
                    }
                    last.insert(s.id.clone(), key);
                    to_update.push(s.id.clone());
                    continue;
                }
                let last = inner
                    .auto_last
                    .lock()
                    .unwrap()
                    .get(&s.id)
                    .copied()
                    .unwrap_or(0);
                if now - last >= s.interval {
                    to_update.push(s.id.clone());
                }
            }
            for r in &profile.rule_providers {
                if !r.auto_update {
                    continue;
                }
                if cron_hit_now(r.cron.as_deref()) {
                    let key = minute_key();
                    let mut last = inner.auto_last.lock().unwrap();
                    if last.get(&r.id).copied() == Some(key) {
                        continue;
                    }
                    last.insert(r.id.clone(), key);
                    to_update.push(r.id.clone());
                    continue;
                }
                let last = inner
                    .auto_last
                    .lock()
                    .unwrap()
                    .get(&r.id)
                    .copied()
                    .unwrap_or(0);
                if now - last >= r.interval {
                    to_update.push(r.id.clone());
                }
            }
            let app_config = inner.app_config.lock().unwrap().clone();
            for id in to_update {
                inner.auto_last.lock().unwrap().insert(id.clone(), now);
                let _ = mihomo_api_raw(
                    &app_config,
                    reqwest::Method::PUT,
                    &format!("/providers/proxies/{}", id),
                    Some(serde_json::json!({"name": id})),
                )
                .await;
                let _ = mihomo_api_raw(
                    &app_config,
                    reqwest::Method::PUT,
                    &format!("/providers/rules/{}", id),
                    Some(serde_json::json!({"name": id})),
                )
                .await;
            }
            if inner.stop_flag.load(Ordering::SeqCst) {
                inner.scheduler_running.store(false, Ordering::SeqCst);
                break;
            }
        }
    });
}

pub fn kill_on_exit(inner: &MihomoInner) {
    crate::exit_log::exit_log("cleanup: kill_on_exit 进入");
    // 注意：app_config 是 std::sync::Mutex（非重入）。下面先取出 auto_close 后
    // 立即让 Guard 离开作用域释放锁，否则后续 set_sys_proxy 内部再次 lock 会死锁，
    // 导致 ExitRequested 回调卡死、托盘"退出"无响应（仅在开启了代理自动关闭时触发）。
    let auto_close = {
        let g = inner.app_config.lock().unwrap();
        g.auto_close_proxy
    };
    crate::exit_log::exit_log(&format!("cleanup: auto_close_proxy={}", auto_close));
    // 先停止核心（真正 kill mihomo 子进程），确保在有限退出窗口内优先完成；
    // 清系统代理（可能同步卡顿）放到后面，避免阻塞内核关闭。
    crate::exit_log::exit_log("cleanup: 开始 stop_core");
    stop_core(inner);
    crate::exit_log::exit_log("cleanup: stop_core 完成");
    // 兜底：子进程句柄丢失（detach/看门狗接管）时按端口+进程名补杀，
    // 保证 any-version 退出时必然关闭自己拉起的 mihomo 核心。
    crate::commands::mihomo::manager::kill_core_by_port(inner);
    crate::exit_log::exit_log("cleanup: kill_core_by_port 完成");
    if auto_close {
        crate::exit_log::exit_log("cleanup: 开始 set_sys_proxy(false)（同步 shell，可能卡）");
        // F5 修复：退出时清除系统代理失败应上报（此处为强制退出，保持日志但传播）
        if let Err(e) = set_sys_proxy(inner, false) {
            eprintln!("[mihomo] 退出时清除系统代理失败: {e}");
            crate::exit_log::exit_log(&format!("cleanup: set_sys_proxy 失败: {e}"));
        } else {
            crate::exit_log::exit_log("cleanup: set_sys_proxy 完成");
        }
    }
}
