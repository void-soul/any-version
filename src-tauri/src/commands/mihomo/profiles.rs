use super::*;

// ---- Profiles ----
#[tauri::command]
pub fn mihomo_get_profile_config(state: State<'_, MihomoState>) -> ProfileConfig {
    state.profile_config.lock().unwrap().clone()
}
#[tauri::command]
pub fn mihomo_set_profile_config(
    app: AppHandle,
    state: State<'_, MihomoState>,
    cfg: ProfileConfig,
) -> Result<(), String> {
    save_profile_config(&state.data_dir, &cfg).ok();
    *state.profile_config.lock().unwrap() = cfg;
    emit_state(&app, &state);
    Ok(())
}
#[tauri::command]
pub fn mihomo_get_profile_item(
    state: State<'_, MihomoState>,
    id: String,
) -> Result<ProfileItem, String> {
    state
        .profile_config
        .lock()
        .unwrap()
        .items
        .iter()
        .find(|i| i.id == id)
        .cloned()
        .ok_or("profile 不存在".to_string())
}
#[tauri::command]
pub fn mihomo_get_profile_str(state: State<'_, MihomoState>, id: String) -> Result<String, String> {
    let pc = state.profile_config.lock().unwrap();
    let item = pc
        .items
        .iter()
        .find(|i| i.id == id)
        .ok_or("profile 不存在".to_string())?;
    Ok(read_profile_content(&state.data_dir, item).unwrap_or_default())
}

// 返回配置文件的绝对路径，若尚未落盘则先生成并写入 profiles/<id>.yaml（供“打开文件”使用）
#[tauri::command]
pub fn mihomo_get_profile_file_path(
    state: State<'_, MihomoState>,
    id: String,
) -> Result<String, String> {
    let pc = state.profile_config.lock().unwrap();
    let item = pc
        .items
        .iter()
        .find(|i| i.id == id)
        .ok_or("profile 不存在".to_string())?;
    let p = profile_content_path(&state.data_dir, &item.id);
    if !p.exists() {
        let content = if item.type_ == "subscription" {
            serde_yaml::to_string(&build_subscription_profile(item)).map_err(|e| e.to_string())?
        } else if let Some(fp) = &item.file_path {
            std::fs::read_to_string(fp).map_err(|e| e.to_string())?
        } else {
            return Err("配置内容不存在".into());
        };
        write_profile_content(&state.data_dir, item, &content).map_err(|e| e.to_string())?;
    }
    p.to_str()
        .map(|s| s.to_string())
        .ok_or("路径非 UTF-8".into())
}
#[tauri::command]
pub fn mihomo_set_profile_str(
    state: State<'_, MihomoState>,
    id: String,
    content: String,
) -> Result<(), String> {
    let mut cfg = {
        let pc = state.profile_config.lock().unwrap();
        let item = pc
            .items
            .iter()
            .find(|i| i.id == id)
            .ok_or("profile 不存在".to_string())?;
        write_profile_content(&state.data_dir, item, &content).map_err(|e| e.to_string())?;
        pc.clone()
    };
    if let Some(it) = cfg.items.iter_mut().find(|i| i.id == id) {
        it.updated_at = Some(now_secs());
    }
    save_profile_config(&state.data_dir, &cfg).ok();
    *state.profile_config.lock().unwrap() = cfg;
    Ok(())
}
#[tauri::command]
pub fn mihomo_add_profile(
    app: AppHandle,
    state: State<'_, MihomoState>,
    item: ProfileItem,
) -> Result<(), String> {
    let mut cfg = state.profile_config.lock().unwrap().clone();
    if cfg.items.iter().any(|i| i.id == item.id) {
        return Err("id 已存在".into());
    }
    cfg.items.push(item);
    save_profile_config(&state.data_dir, &cfg).ok();
    *state.profile_config.lock().unwrap() = cfg;
    emit_state(&app, &state);
    Ok(())
}
#[tauri::command]
pub fn mihomo_remove_profile(
    app: AppHandle,
    state: State<'_, MihomoState>,
    id: String,
) -> Result<(), String> {
    let mut cfg = state.profile_config.lock().unwrap().clone();
    cfg.items.retain(|i| i.id != id);
    if cfg.current == id {
        cfg.current = cfg.items.first().map(|i| i.id.clone()).unwrap_or_default();
    }
    save_profile_config(&state.data_dir, &cfg).ok();
    *state.profile_config.lock().unwrap() = cfg;
    emit_state(&app, &state);
    Ok(())
}
#[tauri::command]
pub fn mihomo_update_profile(
    app: AppHandle,
    state: State<'_, MihomoState>,
    item: ProfileItem,
) -> Result<(), String> {
    let mut cfg = state.profile_config.lock().unwrap().clone();
    if let Some(it) = cfg.items.iter_mut().find(|i| i.id == item.id) {
        *it = item;
    } else {
        cfg.items.push(item);
    }
    save_profile_config(&state.data_dir, &cfg).ok();
    *state.profile_config.lock().unwrap() = cfg;
    emit_state(&app, &state);
    Ok(())
}
#[tauri::command]
pub async fn mihomo_change_current_profile(
    app: AppHandle,
    state: State<'_, MihomoState>,
    id: String,
) -> Result<(), String> {
    switch_current_profile(&app, &state, &id).await
}

/// 切换当前订阅配置（命令与 SSID 感知共用：都要「写配置 → 推状态 → 热重载」）。
pub async fn switch_current_profile(
    app: &AppHandle,
    inner: &Arc<MihomoInner>,
    id: &str,
) -> Result<(), String> {
    if !inner
        .profile_config
        .lock()
        .unwrap()
        .items
        .iter()
        .any(|i| i.id == id)
    {
        return Err("profile 不存在".into());
    }
    let mut app_config = inner.app_config.lock().unwrap().clone();
    app_config.current_profile = id.to_string();
    save_app_config(&inner.data_dir, &app_config).ok();
    *inner.app_config.lock().unwrap() = app_config;
    emit_state(app, inner);
    if !inner.stop_flag.load(Ordering::SeqCst) {
        reload_config(app, Arc::clone(inner)).await?;
    }
    Ok(())
}
