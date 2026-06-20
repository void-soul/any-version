use std::fs;
use std::path::PathBuf;
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    pub versions_dir: String,
    pub links_dir: String,
}

pub fn get_base_dir() -> PathBuf {
    let user_profile = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| {
            let drive = std::env::var("HOMEDRIVE").unwrap_or_default();
            let path = std::env::var("HOMEPATH").unwrap_or_default();
            if drive.is_empty() && path.is_empty() {
                "C:\\any-version".to_string()
            } else {
                format!("{}{}", drive, path)
            }
        });
    let mut path = PathBuf::from(user_profile);
    if path.as_os_str().is_empty() || path == PathBuf::from("C:\\any-version") {
        PathBuf::from("C:\\any-version")
    } else {
        path.push(".any-version");
        path
    }
}

pub fn load_config() -> Config {
    let base_dir = get_base_dir();
    let config_path = base_dir.join("config.json");
    if config_path.exists() {
        if let Ok(data) = fs::read_to_string(&config_path) {
            if let Ok(config) = serde_json::from_str::<Config>(&data) {
                return config;
            }
        }
    }
    let default_config = Config {
        versions_dir: base_dir.join("versions").to_string_lossy().to_string(),
        links_dir: base_dir.join("links").to_string_lossy().to_string(),
    };
    let _ = fs::create_dir_all(&base_dir);
    let _ = save_config(&default_config);
    default_config
}

pub fn save_config(config: &Config) -> Result<(), String> {
    let base_dir = get_base_dir();
    let config_path = base_dir.join("config.json");
    let data = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    fs::write(config_path, data).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn get_config() -> Result<Config, String> {
    Ok(load_config())
}

#[tauri::command]
pub fn update_config(versions_dir: String, links_dir: String) -> Result<(), String> {
    let mut config = load_config();
    config.versions_dir = versions_dir;
    config.links_dir = links_dir;
    save_config(&config)?;
    Ok(())
}
