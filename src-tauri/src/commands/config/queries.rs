use super::*;

#[tauri::command]
pub fn get_config() -> Result<Config, String> {
    Ok(load_config())
}

/// 获取应用版本号（从 Cargo.toml）
#[tauri::command]
pub fn get_app_version() -> Result<String, String> {
    Ok(env!("CARGO_PKG_VERSION").to_string())
}
