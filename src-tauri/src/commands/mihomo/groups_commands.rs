use super::*;

/// 确保某个覆写条目存在（不存在就建一个 local/yaml 的）。
pub(super) fn ensure_override_item(
    state: &MihomoInner,
    id: &str,
    name: &str,
) -> Result<OverrideItem, String> {
    let mut cfg = state.override_config.lock().unwrap().clone();
    if let Some(item) = cfg.items.iter().find(|i| i.id == id) {
        return Ok(item.clone());
    }
    let item = OverrideItem {
        id: id.to_string(),
        name: name.to_string(),
        ext: "yaml".to_string(),
        global: false,
        type_: "local".to_string(),
        url: None,
        updated: Some(now_secs()),
    };
    cfg.items.push(item.clone());
    save_override_config(&state.data_dir, &cfg).map_err(|e| e.to_string())?;
    *state.override_config.lock().unwrap() = cfg;
    Ok(item)
}

/// 把覆写挂到当前配置上（幂等），并保存配置。
pub(super) fn attach_override_to_current_profile(
    state: &MihomoInner,
    id: &str,
) -> Result<(), String> {
    let mut cfg = state.profile_config.lock().unwrap().clone();
    let current_id = cfg.current.clone();
    let Some(item) = cfg.items.iter_mut().find(|i| i.id == current_id) else {
        return Err("当前配置不存在".into());
    };
    if !item.override_ids.iter().any(|o| o == id) {
        item.override_ids.push(id.to_string());
    }
    save_profile_config(&state.data_dir, &cfg).map_err(|e| e.to_string())?;
    *state.profile_config.lock().unwrap() = cfg;
    Ok(())
}

/// Tauri 命令：当前配置里可编辑的策略组 + 全部可选节点名。
#[tauri::command]
pub fn mihomo_get_group_editor(state: State<'_, MihomoState>) -> Result<Value, String> {
    let (item, overrides) = {
        let pcfg = state.profile_config.lock().unwrap();
        let current_id = pcfg.current.clone();
        let item = pcfg
            .items
            .iter()
            .find(|i| i.id == current_id)
            .cloned()
            .ok_or_else(|| "当前配置不存在".to_string())?;
        (item, state.override_config.lock().unwrap().items.clone())
    };
    crate::commands::mihomo::groups::editor_payload(&state.data_dir, &item, &overrides)
}

/// Tauri 命令：保存策略组（写进覆写 + 挂到当前配置 + 热重载）。
///
/// 传空数组 = 删除覆写，回到订阅自带的策略组。
#[tauri::command]
pub async fn mihomo_save_proxy_groups(
    app: AppHandle,
    state: State<'_, MihomoState>,
    groups: Vec<Value>,
) -> Result<(), String> {
    if groups.is_empty() {
        return mihomo_clear_proxy_groups(app, state).await;
    }
    let yaml = crate::commands::mihomo::groups::groups_to_yaml(&groups)?;
    let item = ensure_override_item(
        &state,
        crate::commands::mihomo::groups::GROUPS_OVERRIDE_ID,
        "自定义策略组",
    )?;
    write_override_content(&state.data_dir, &item, &yaml).map_err(|e| e.to_string())?;
    attach_override_to_current_profile(&state, &item.id)?;
    reload_config(&app, Arc::clone(&*state)).await?;
    emit_state(&app, &state);
    Ok(())
}

/// Tauri 命令：删除自定义策略组覆写（恢复订阅自带策略组）。
#[tauri::command]
pub async fn mihomo_clear_proxy_groups(
    app: AppHandle,
    state: State<'_, MihomoState>,
) -> Result<(), String> {
    let id = crate::commands::mihomo::groups::GROUPS_OVERRIDE_ID;
    {
        let mut cfg = state.override_config.lock().unwrap().clone();
        if let Some(pos) = cfg.items.iter().position(|i| i.id == id) {
            cfg.items.remove(pos);
            save_override_config(&state.data_dir, &cfg).map_err(|e| e.to_string())?;
            *state.override_config.lock().unwrap() = cfg;
        }
        let mut pcfg = state.profile_config.lock().unwrap().clone();
        let current_id = pcfg.current.clone();
        if let Some(it) = pcfg.items.iter_mut().find(|i| i.id == current_id) {
            it.override_ids.retain(|o| o != id);
            save_profile_config(&state.data_dir, &pcfg).map_err(|e| e.to_string())?;
            *state.profile_config.lock().unwrap() = pcfg;
        }
    }
    reload_config(&app, Arc::clone(&*state)).await?;
    emit_state(&app, &state);
    Ok(())
}

/// Tauri 命令：简易模式编译器 —— 按「出站模式」生成规则覆写。
///
/// `mode`：`rule`（常规分流）/ `bypassCN`（绕过大陆）/ `global`（全局代理）。
/// `final_group`：兜底 MATCH 走哪个策略组（通常是手动切换或自动选择）。
#[tauri::command]
pub async fn mihomo_apply_simple_mode(
    app: AppHandle,
    state: State<'_, MihomoState>,
    mode: String,
    final_group: String,
) -> Result<(), String> {
    let final_group = final_group.trim();
    if final_group.is_empty() {
        return Err("请指定兜底策略组".into());
    }
    let yaml = crate::commands::mihomo::groups::simple_rules_yaml(&mode, final_group)?;
    let item = ensure_override_item(
        &state,
        crate::commands::mihomo::groups::SIMPLE_OVERRIDE_ID,
        "简易模式规则",
    )?;
    write_override_content(&state.data_dir, &item, &yaml).map_err(|e| e.to_string())?;
    attach_override_to_current_profile(&state, &item.id)?;
    reload_config(&app, Arc::clone(&*state)).await?;
    emit_state(&app, &state);
    Ok(())
}

/// Tauri 命令：后端常驻采样的流量总览（趋势 + 区间总量）。
///
/// 与渲染进程那套 IndexedDB 统计互补：这份数据**不依赖页面是否打开**，
/// 切页/关窗期间也在采，是「完整历史」的唯一来源。
#[tauri::command]
pub fn mihomo_traffic_overview(
    state: State<'_, MihomoState>,
    since_secs: u64,
    bucket_secs: u64,
) -> Result<serde_json::Value, String> {
    let (points, upload, download) =
        crate::commands::mihomo::traffic::query(&state.data_dir, since_secs, bucket_secs)?;
    Ok(json!({
        "points": points,
        "upload": upload,
        "download": download,
        "recording": crate::commands::mihomo::traffic::recorder_running(),
    }))
}

/// Tauri 命令：清空后端流量历史。
#[tauri::command]
pub fn mihomo_traffic_clear(state: State<'_, MihomoState>) -> Result<(), String> {
    crate::commands::mihomo::traffic::clear(&state.data_dir)
}

/// Tauri 命令：当前所连 Wi-Fi 的 SSID（未连接返回 null）。
#[tauri::command]
pub fn mihomo_get_ssid() -> Option<String> {
    crate::commands::mihomo::ssid::current_ssid()
}

/// Tauri 命令：保存 SSID 感知开关与规则表。
///
/// 规则里的 profile_id 不存在时直接报错 —— 静默保存会让「切过去」永远静默失败。
#[tauri::command]
pub async fn mihomo_set_ssid_rules(
    app: AppHandle,
    state: State<'_, MihomoState>,
    enabled: bool,
    rules: Vec<crate::commands::mihomo::config::SsidRule>,
) -> Result<(), String> {
    {
        let cfg = state.profile_config.lock().unwrap();
        for r in &rules {
            if !cfg.items.iter().any(|i| i.id == r.profile_id) {
                return Err(format!("配置不存在: {}", r.profile_id));
            }
        }
    }
    let mut app_config = state.app_config.lock().unwrap().clone();
    app_config.ssid_switch_enabled = enabled;
    app_config.ssid_rules = rules;
    save_app_config(&state.data_dir, &app_config).map_err(|e| e.to_string())?;
    *state.app_config.lock().unwrap() = app_config;
    emit_state(&app, &state);
    Ok(())
}

/// Tauri 命令：改某个订阅配置的自动更新策略（开关 / 秒级间隔 / cron）。
///
/// `cron` 非空时取代 `update_interval`；传空串表示清除 cron 回到间隔模式。
#[tauri::command]
pub fn mihomo_update_profile_schedule(
    state: State<'_, MihomoState>,
    id: String,
    auto_update: bool,
    update_interval: Option<u64>,
    cron: Option<String>,
) -> Result<(), String> {
    let cron = cron.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    if let Some(expr) = &cron {
        // 立刻校验：写进配置后再发现非法，只会表现为「再也不更新了」
        crate::commands::mihomo::cron::CronExpr::parse(expr)
            .map_err(|e| format!("cron 表达式非法: {e}"))?;
    }
    let mut cfg = state.profile_config.lock().unwrap().clone();
    let Some(item) = cfg.items.iter_mut().find(|i| i.id == id) else {
        return Err("配置不存在".into());
    };
    item.auto_update = auto_update;
    if let Some(v) = update_interval {
        item.update_interval = v;
    }
    item.update_cron = cron;
    save_profile_config(&state.data_dir, &cfg).map_err(|e| e.to_string())?;
    *state.profile_config.lock().unwrap() = cfg;
    Ok(())
}

/// SSID 感知：当前 Wi-Fi 命中规则表且目标配置与当前不同 → 自动切换。
///
/// 只在「SSID 发生变化」时动作（缓存上一次的 SSID），否则每分钟都会判定一次，
/// 用户手动切回别的配置也会被反复拽回去。
pub(super) async fn apply_ssid_rules(app: &AppHandle, inner: &Arc<MihomoInner>) {
    let (enabled, rules, current) = {
        let cfg = inner.app_config.lock().unwrap();
        (
            cfg.ssid_switch_enabled,
            cfg.ssid_rules.clone(),
            cfg.current_profile.clone(),
        )
    };
    if !enabled || rules.is_empty() {
        return;
    }
    let Some(ssid) = crate::commands::mihomo::ssid::current_ssid() else {
        return;
    };
    let Some(rule) = rules
        .iter()
        .find(|r| r.ssid.trim().eq_ignore_ascii_case(ssid.trim()))
    else {
        return;
    };
    if rule.profile_id == current {
        return;
    }
    eprintln!(
        "[mihomo] SSID「{}」命中规则 → 切换配置 {}",
        ssid, rule.profile_id
    );
    if let Err(e) = switch_current_profile(app, inner, &rule.profile_id).await {
        eprintln!("[mihomo] SSID 切换配置失败: {e}");
        return;
    }
    let _ = app.emit(
        "mihomo://profile-changed",
        json!({ "id": rule.profile_id, "by": "ssid", "ssid": ssid }),
    );
}
