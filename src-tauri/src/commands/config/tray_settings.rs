use super::*;

/// 记录托盘「启动上次配置」所需的最近一次服务参数
pub fn remember_last_server(kind: &str, value: serde_json::Value) {
    let _ = mutate_config(|config| {
        match kind {
            "http" => config.last_servers.http = Some(value.clone()),
            "rtsp" => config.last_servers.rtsp = Some(value.clone()),
            _ => return Ok(()),
        }
        Ok(())
    });
}

#[tauri::command]
pub fn get_tray_menu_config() -> TrayMenuConfig {
    load_config().tray_menu
}

#[tauri::command]
pub fn set_tray_menu_config(app: tauri::AppHandle, value: TrayMenuConfig) -> Result<(), String> {
    mutate_config(|config| {
        config.tray_menu = value.clone();
        Ok(())
    })?;
    crate::tray::rebuild_tray_menu(&app).map_err(|e| e.to_string())
}
