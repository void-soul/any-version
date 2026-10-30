use super::*;

// ---------- 通用 patch ----------
pub(super) fn patch_struct<T: Serialize + for<'de> Deserialize<'de>>(
    cur: &T,
    patch: &Value,
) -> Result<T, String> {
    let mut cur_v = serde_json::to_value(cur).map_err(|e| e.to_string())?;
    deep_merge(&mut cur_v, patch);
    serde_json::from_value(cur_v).map_err(|e| e.to_string())
}

// ================= 命令 =================
// 克隆 app_config 后再 await，避免锁跨 await 导致 future 非 Send
pub(super) async fn mihomo_api_call(
    state: &State<'_, MihomoState>,
    method: Method,
    url: &str,
    body: Option<Value>,
) -> Result<String, String> {
    let app_config = state.app_config.lock().unwrap().clone();
    mihomo_api_raw(&app_config, method, url, body).await
}
// 用 async 让它跑在工作线程而非主线程，避免任何意外阻塞冻结整个界面
#[tauri::command]
pub async fn mihomo_get_state(state: State<'_, MihomoState>) -> Result<MihomoStateView, String> {
    Ok(build_state_view(&state))
}

/// 清除内核运行期告警（runtime_warnings）。运行期产生的 TUN/权限类错误会累积显示，
/// 且只在重启时清空，这里提供手动清除入口。
#[tauri::command]
pub fn mihomo_clear_warnings(app: AppHandle, state: State<'_, MihomoState>) -> Result<(), String> {
    state
        .runtime_warnings
        .lock()
        .map_err(|e| e.to_string())?
        .clear();
    emit_state(&app, &state);
    Ok(())
}

#[tauri::command]
pub fn mihomo_controller_info(state: State<'_, MihomoState>) -> Value {
    let app = state.app_config.lock().unwrap();
    serde_json::json!({
        "port": app.controller_port,
        "secret": app.secret,
        "wsBase": format!("ws://127.0.0.1:{}", app.controller_port),
    })
}

#[tauri::command]
pub async fn mihomo_start(app: AppHandle, state: State<'_, MihomoState>) -> Result<(), String> {
    launch_core(&app, Arc::clone(&*state)).await?;
    let need_proxy = state.app_config.lock().unwrap().sys_proxy_enable;
    if need_proxy {
        set_sys_proxy(&state, true)?;
    }
    // 托盘状态灯：代理运行中 → 绿色光环
    let _ = crate::tray::set_tray_status_badge(&app, Some((16, 185, 129)));
    Ok(())
}

#[tauri::command]
pub async fn mihomo_stop(app: AppHandle, state: State<'_, MihomoState>) -> Result<(), String> {
    stop_core(&state);
    let need_proxy = state.app_config.lock().unwrap().sys_proxy_enable;
    if need_proxy {
        set_sys_proxy(&state, false)?;
    }
    // 托盘状态灯：停止 → 恢复（计划红点仍会优先保留）
    let _ = crate::tray::set_tray_status_badge(&app, None);
    emit_state(&app, &state);
    Ok(())
}

#[tauri::command]
pub async fn mihomo_restart(app: AppHandle, state: State<'_, MihomoState>) -> Result<(), String> {
    launch_core(&app, Arc::clone(&*state)).await
}

#[tauri::command]
pub fn mihomo_set_core_path(
    app: AppHandle,
    state: State<'_, MihomoState>,
    path: Option<String>,
) -> Result<(), String> {
    let mut app_config = state.app_config.lock().unwrap().clone();
    app_config.core_path = path;
    save_app_config(&state.data_dir, &app_config).map_err(|e| e.to_string())?;
    *state.app_config.lock().unwrap() = app_config;
    refresh_core_version(&state);
    emit_state(&app, &state);
    Ok(())
}

#[tauri::command]
pub async fn mihomo_close_all_connections(state: State<'_, MihomoState>) -> Result<(), String> {
    mihomo_api_call(
        &state,
        reqwest::Method::DELETE,
        "/connections",
        Some(serde_json::json!({"all": true})),
    )
    .await?;
    Ok(())
}

// ---- AppConfig ----
#[tauri::command]
pub fn mihomo_get_app_config(state: State<'_, MihomoState>) -> AppConfig {
    state.app_config.lock().unwrap().clone()
}
#[tauri::command]
pub fn mihomo_patch_app_config(
    app: AppHandle,
    state: State<'_, MihomoState>,
    patch: Value,
) -> Result<AppConfig, String> {
    let new_cfg = patch_struct(&*state.app_config.lock().unwrap(), &patch)?;
    save_app_config(&state.data_dir, &new_cfg).ok();
    *state.app_config.lock().unwrap() = new_cfg.clone();
    emit_state(&app, &state);
    Ok(new_cfg)
}

// ---- Secondary proxies（二级代理）----
// 保存整份二级代理列表 + 当前启用项，并热重载配置使链式生效
#[tauri::command]
pub async fn mihomo_save_secondary_proxies(
    app: AppHandle,
    state: State<'_, MihomoState>,
    items: Vec<crate::commands::mihomo::config::SecondaryProxy>,
    active_id: Option<String>,
) -> Result<(), String> {
    let mut app_config = state.app_config.lock().unwrap().clone();
    app_config.secondary_proxies = items;
    app_config.secondary_active_id = active_id;
    save_app_config(&state.data_dir, &app_config).map_err(|e| e.to_string())?;
    *state.app_config.lock().unwrap() = app_config;
    if !state.stop_flag.load(Ordering::SeqCst) {
        reload_config(&app, Arc::clone(&*state)).await?;
    }
    emit_state(&app, &state);
    Ok(())
}

// ---- ControledMihomoConfig ----
#[tauri::command]
pub fn mihomo_get_controled_config(state: State<'_, MihomoState>) -> Value {
    state.controled_config.lock().unwrap().clone()
}
#[tauri::command]
pub fn mihomo_patch_controled_config(
    app: AppHandle,
    state: State<'_, MihomoState>,
    patch: Value,
) -> Result<Value, String> {
    let mut cur = state.controled_config.lock().unwrap().clone();
    deep_merge(&mut cur, &patch);
    save_controled(&state.data_dir, &cur).ok();
    *state.controled_config.lock().unwrap() = cur.clone();
    emit_state(&app, &state);
    Ok(cur)
}

// ---- 运行配置 ----
#[tauri::command]
pub fn mihomo_get_runtime_config(state: State<'_, MihomoState>) -> String {
    state.runtime_config_str.lock().unwrap().clone()
}
#[tauri::command]
pub async fn mihomo_update_runtime_config(
    app: AppHandle,
    state: State<'_, MihomoState>,
) -> Result<(), String> {
    reload_config(&app, Arc::clone(&*state)).await
}
