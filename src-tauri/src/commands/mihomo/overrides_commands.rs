use super::*;

// ---- Overrides ----
#[tauri::command]
pub fn mihomo_get_override_config(state: State<'_, MihomoState>) -> OverrideConfig {
    state.override_config.lock().unwrap().clone()
}
#[tauri::command]
pub fn mihomo_set_override_config(
    app: AppHandle,
    state: State<'_, MihomoState>,
    cfg: OverrideConfig,
) -> Result<(), String> {
    save_override_config(&state.data_dir, &cfg).ok();
    *state.override_config.lock().unwrap() = cfg;
    emit_state(&app, &state);
    Ok(())
}
#[tauri::command]
pub fn mihomo_get_override_item(
    state: State<'_, MihomoState>,
    id: String,
) -> Result<OverrideItem, String> {
    state
        .override_config
        .lock()
        .unwrap()
        .items
        .iter()
        .find(|i| i.id == id)
        .cloned()
        .ok_or("override 不存在".to_string())
}
#[tauri::command]
pub fn mihomo_add_override(
    app: AppHandle,
    state: State<'_, MihomoState>,
    item: OverrideItem,
) -> Result<(), String> {
    let mut cfg = state.override_config.lock().unwrap().clone();
    if cfg.items.iter().any(|i| i.id == item.id) {
        return Err("id 已存在".into());
    }
    cfg.items.push(item);
    save_override_config(&state.data_dir, &cfg).ok();
    *state.override_config.lock().unwrap() = cfg;
    emit_state(&app, &state);
    Ok(())
}
#[tauri::command]
pub fn mihomo_remove_override(
    app: AppHandle,
    state: State<'_, MihomoState>,
    id: String,
) -> Result<(), String> {
    let mut cfg = state.override_config.lock().unwrap().clone();
    cfg.items.retain(|i| i.id != id);
    save_override_config(&state.data_dir, &cfg).ok();
    *state.override_config.lock().unwrap() = cfg;
    emit_state(&app, &state);
    Ok(())
}
#[tauri::command]
pub fn mihomo_update_override(
    app: AppHandle,
    state: State<'_, MihomoState>,
    item: OverrideItem,
) -> Result<(), String> {
    let mut cfg = state.override_config.lock().unwrap().clone();
    if let Some(it) = cfg.items.iter_mut().find(|i| i.id == item.id) {
        *it = item;
    } else {
        cfg.items.push(item);
    }
    save_override_config(&state.data_dir, &cfg).ok();
    *state.override_config.lock().unwrap() = cfg;
    emit_state(&app, &state);
    Ok(())
}
#[tauri::command]
pub fn mihomo_get_override(state: State<'_, MihomoState>, id: String) -> Result<String, String> {
    let oc = state.override_config.lock().unwrap();
    let item = oc
        .items
        .iter()
        .find(|i| i.id == id)
        .ok_or("override 不存在".to_string())?;
    Ok(read_override_content(&state.data_dir, item).unwrap_or_default())
}
#[tauri::command]
pub fn mihomo_set_override(
    app: AppHandle,
    state: State<'_, MihomoState>,
    id: String,
    content: String,
) -> Result<(), String> {
    {
        let oc = state.override_config.lock().unwrap();
        let item = oc
            .items
            .iter()
            .find(|i| i.id == id)
            .ok_or("override 不存在".to_string())?;
        write_override_content(&state.data_dir, item, &content).map_err(|e| e.to_string())?;
    }
    emit_state(&app, &state);
    Ok(())
}
