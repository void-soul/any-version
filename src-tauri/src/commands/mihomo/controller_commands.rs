use super::*;

// ---- Controller API ----
#[tauri::command]
pub async fn mihomo_api(
    state: State<'_, MihomoState>,
    method: String,
    url: String,
    body: Option<Value>,
) -> Result<String, String> {
    let m = match method.to_uppercase().as_str() {
        "GET" => reqwest::Method::GET,
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "PATCH" => reqwest::Method::PATCH,
        "DELETE" => reqwest::Method::DELETE,
        _ => return Err("不支持的方法".into()),
    };
    mihomo_api_call(&state, m, &url, body).await
}
#[tauri::command]
pub async fn mihomo_version(state: State<'_, MihomoState>) -> Result<Value, String> {
    let s = mihomo_api_call(&state, reqwest::Method::GET, "/version", None).await?;
    Ok(serde_json::from_str(&s).unwrap_or(Value::Null))
}
#[tauri::command]
pub async fn mihomo_proxies(state: State<'_, MihomoState>) -> Result<Value, String> {
    let s = mihomo_api_call(&state, reqwest::Method::GET, "/proxies", None).await?;
    Ok(serde_json::from_str(&s).unwrap_or(Value::Null))
}
#[tauri::command]
pub async fn mihomo_groups(state: State<'_, MihomoState>) -> Result<Value, String> {
    let s = mihomo_api_call(&state, reqwest::Method::GET, "/providers/proxies", None).await?;
    Ok(serde_json::from_str(&s).unwrap_or(Value::Null))
}
#[tauri::command]
pub async fn mihomo_rules(state: State<'_, MihomoState>) -> Result<Value, String> {
    let s = mihomo_api_call(&state, reqwest::Method::GET, "/rules", None).await?;
    Ok(serde_json::from_str(&s).unwrap_or(Value::Null))
}
#[tauri::command]
pub async fn mihomo_proxy_providers(state: State<'_, MihomoState>) -> Result<Value, String> {
    let s = mihomo_api_call(&state, reqwest::Method::GET, "/providers/proxies", None).await?;
    Ok(serde_json::from_str(&s).unwrap_or(Value::Null))
}
#[tauri::command]
pub async fn mihomo_rule_providers(state: State<'_, MihomoState>) -> Result<Value, String> {
    let s = mihomo_api_call(&state, reqwest::Method::GET, "/providers/rules", None).await?;
    Ok(serde_json::from_str(&s).unwrap_or(Value::Null))
}
#[tauri::command]
pub async fn mihomo_change_proxy(
    state: State<'_, MihomoState>,
    group: String,
    name: String,
) -> Result<(), String> {
    // 入参 group 为「原始组名」（可能含中文/emoji），本函数内部负责 urlencode；
    // 调用方须传入原始名，勿预先 encode，否则会双重编码导致 404。
    mihomo_api_call(
        &state,
        reqwest::Method::PUT,
        &format!("/proxies/{}", urlencoding(&group)),
        Some(serde_json::json!({ "name": name })),
    )
    .await?;
    Ok(())
}
#[tauri::command]
pub async fn mihomo_unfixed_proxy(
    state: State<'_, MihomoState>,
    group: String,
    name: String,
) -> Result<(), String> {
    mihomo_change_proxy(state, group, name).await
}
#[tauri::command]
pub async fn mihomo_proxy_delay(
    state: State<'_, MihomoState>,
    name: String,
    url: String,
    timeout: u64,
) -> Result<Value, String> {
    let path = format!(
        "/proxies/{}/delay?url={}&timeout={}",
        urlencoding(&name),
        urlencoding(&url),
        timeout
    );
    let s = mihomo_api_call(&state, reqwest::Method::GET, &path, None).await?;
    Ok(serde_json::from_str(&s).unwrap_or(Value::Null))
}
#[tauri::command]
pub async fn mihomo_test_delay(
    state: State<'_, MihomoState>,
    name: String,
    url: String,
    timeout: u64,
) -> Result<Value, String> {
    mihomo_proxy_delay(state, name, url, timeout).await
}
/// 测试整个策略组延迟（对齐 mihomoGroupDelay）
#[tauri::command]
pub async fn mihomo_group_delay(
    state: State<'_, MihomoState>,
    group: String,
    url: String,
    timeout: u64,
) -> Result<Value, String> {
    let path = format!(
        "/group/{}/delay?url={}&timeout={}",
        urlencoding(&group),
        urlencoding(&url),
        timeout
    );
    let s = mihomo_api_call(&state, reqwest::Method::GET, &path, None).await?;
    Ok(serde_json::from_str(&s).unwrap_or(Value::Null))
}
/// 代理集合健康检查（对齐 mihomoProxyDelay 的 provider 分支）
#[tauri::command]
pub async fn mihomo_provider_healthcheck(
    state: State<'_, MihomoState>,
    provider: String,
    name: Option<String>,
    url: String,
    timeout: u64,
) -> Result<Value, String> {
    let path = match &name {
        Some(n) => format!(
            "/providers/proxies/{}/{}/healthcheck?url={}&timeout={}",
            urlencoding(&provider),
            urlencoding(n),
            urlencoding(&url),
            timeout
        ),
        None => format!("/providers/proxies/{}/healthcheck", urlencoding(&provider)),
    };
    let s = mihomo_api_call(&state, reqwest::Method::GET, &path, None).await?;
    Ok(serde_json::from_str(&s).unwrap_or(Value::Null))
}
/// 更新代理集合（对齐 mihomoUpdateProxyProviders）
#[tauri::command]
pub async fn mihomo_update_proxy_provider(
    state: State<'_, MihomoState>,
    name: String,
) -> Result<(), String> {
    mihomo_api_call(
        &state,
        reqwest::Method::PUT,
        &format!("/providers/proxies/{}", urlencoding(&name)),
        None,
    )
    .await?;
    Ok(())
}
/// 更新规则集合（对齐 mihomoUpdateRuleProviders）
#[tauri::command]
pub async fn mihomo_update_rule_provider(
    state: State<'_, MihomoState>,
    name: String,
) -> Result<(), String> {
    mihomo_api_call(
        &state,
        reqwest::Method::PUT,
        &format!("/providers/rules/{}", urlencoding(&name)),
        None,
    )
    .await?;
    Ok(())
}
/// 启用/禁用规则（对齐 mihomoRulesDisable，PATCH /rules/disable）
#[tauri::command]
pub async fn mihomo_rules_disable(
    state: State<'_, MihomoState>,
    rules: Value,
) -> Result<(), String> {
    mihomo_api_call(
        &state,
        reqwest::Method::PATCH,
        "/rules/disable",
        Some(rules),
    )
    .await?;
    Ok(())
}
/// Smart 内核：查询策略组权重（对齐 mihomoSmartGroupWeights）
#[tauri::command]
pub async fn mihomo_smart_group_weights(
    state: State<'_, MihomoState>,
    group: String,
) -> Result<Value, String> {
    let s = mihomo_api_call(
        &state,
        reqwest::Method::GET,
        &format!("/group/{}/weights", urlencoding(&group)),
        None,
    )
    .await?;
    Ok(serde_json::from_str(&s).unwrap_or(Value::Null))
}
/// Smart 内核：清空学习缓存（对齐 mihomoSmartFlushCache）
#[tauri::command]
pub async fn mihomo_smart_flush_cache(
    state: State<'_, MihomoState>,
    config_name: Option<String>,
) -> Result<(), String> {
    let path = match &config_name {
        Some(n) if !n.is_empty() => format!("/cache/smart/flush/{}", urlencoding(n)),
        _ => "/cache/smart/flush".to_string(),
    };
    mihomo_api_call(&state, reqwest::Method::POST, &path, None).await?;
    Ok(())
}
/// 通用配置补丁（对齐 patchMihomoConfig）
#[tauri::command]
pub async fn mihomo_patch_config(
    state: State<'_, MihomoState>,
    patch: Value,
) -> Result<(), String> {
    mihomo_api_call(&state, reqwest::Method::PATCH, "/configs", Some(patch)).await?;
    Ok(())
}
/// 热重载内核配置（对齐 mihomoHotReloadConfig，PUT /configs?force=true）
#[tauri::command]
pub async fn mihomo_hot_reload_config(state: State<'_, MihomoState>) -> Result<(), String> {
    let (diff, current) = {
        let app_config = state.app_config.lock().unwrap();
        let profile = state.profile_config.lock().unwrap();
        (app_config.diff_work_dir, profile.current.clone())
    };
    let path = core_work_config_path(&state.data_dir, diff, &current);
    mihomo_api_call(
        &state,
        reqwest::Method::PUT,
        "/configs?force=true",
        Some(serde_json::json!({ "path": path.to_string_lossy() })),
    )
    .await?;
    Ok(())
}
#[tauri::command]
pub async fn mihomo_set_mode(state: State<'_, MihomoState>, mode: String) -> Result<(), String> {
    mihomo_api_call(
        &state,
        reqwest::Method::PATCH,
        "/configs",
        Some(serde_json::json!({ "mode": mode })),
    )
    .await?;
    Ok(())
}
#[tauri::command]
pub async fn mihomo_select_proxy(
    app: AppHandle,
    state: State<'_, MihomoState>,
    name: String,
) -> Result<(), String> {
    let need_reload = {
        let cfg = state.app_config.lock().unwrap();
        cfg.secondary_active_id.is_some() && !cfg.secondary_proxies.is_empty()
    };
    let mut app_config = state.app_config.lock().unwrap().clone();
    app_config.default_proxy = Some(name);
    save_app_config(&state.data_dir, &app_config).ok();
    *state.app_config.lock().unwrap() = app_config;
    emit_state(&app, &state);
    // 已启用二级代理时，切换一级代理需重新生成配置让 dialer-proxy 跟随新的一级节点
    if need_reload && !state.stop_flag.load(Ordering::SeqCst) {
        reload_config(&app, Arc::clone(&*state)).await?;
    }
    Ok(())
}
#[tauri::command]
pub async fn mihomo_set_tun(
    app: AppHandle,
    state: State<'_, MihomoState>,
    enable: bool,
) -> Result<(), String> {
    let mut app_config = state.app_config.lock().unwrap().clone();
    app_config.tun_enabled = enable;
    save_app_config(&state.data_dir, &app_config).ok();
    *state.app_config.lock().unwrap() = app_config;
    emit_state(&app, &state);
    reload_config(&app, Arc::clone(&*state)).await
}
#[tauri::command]
pub async fn mihomo_close_connection(
    state: State<'_, MihomoState>,
    id: String,
) -> Result<(), String> {
    mihomo_api_call(
        &state,
        reqwest::Method::DELETE,
        &format!("/connections/{}", urlencoding(&id)),
        None,
    )
    .await?;
    Ok(())
}
#[tauri::command]
pub async fn mihomo_get_connections(state: State<'_, MihomoState>) -> Result<Value, String> {
    let s = mihomo_api_call(&state, reqwest::Method::GET, "/connections", None).await?;
    Ok(serde_json::from_str(&s).unwrap_or(Value::Null))
}
#[tauri::command]
pub async fn mihomo_get_memory(state: State<'_, MihomoState>) -> Result<Value, String> {
    let s = mihomo_api_call(&state, reqwest::Method::GET, "/memory", None).await?;
    Ok(serde_json::from_str(&s).unwrap_or(Value::Null))
}
#[tauri::command]
pub fn mihomo_get_logs(state: State<'_, MihomoState>) -> Vec<String> {
    let content = std::fs::read_to_string(&state.log_file).unwrap_or_default();
    let lines: Vec<&str> = content.lines().collect();
    lines
        .into_iter()
        .rev()
        .take(300)
        .rev()
        .map(|s| s.to_string())
        .collect()
}
#[tauri::command]
pub async fn mihomo_set_sys_proxy(
    app: AppHandle,
    state: State<'_, MihomoState>,
    enable: bool,
) -> Result<(), String> {
    set_sys_proxy(&state, enable)?;
    let mut app_config = state.app_config.lock().unwrap().clone();
    app_config.sys_proxy_enable = enable;
    save_app_config(&state.data_dir, &app_config).ok();
    *state.app_config.lock().unwrap() = app_config;
    emit_state(&app, &state);
    Ok(())
}
#[tauri::command]
pub fn mihomo_get_sys_proxy() -> Value {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(key) = hkcu.open_subkey_with_flags(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings",
        KEY_READ,
    ) {
        let enabled: u32 = key.get_value("ProxyEnable").unwrap_or(0);
        let server: String = key.get_value("ProxyServer").unwrap_or_default();
        serde_json::json!({ "enable": enabled != 0, "server": server })
    } else {
        serde_json::json!({ "enable": false, "server": "" })
    }
}
