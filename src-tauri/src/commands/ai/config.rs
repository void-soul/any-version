use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tauri::AppHandle;
use tauri::Emitter;
use crate::commands::ai_registry::{registry, AiToolDefDto, ToolConfig, PathConfig};
use crate::commands::config::get_base_dir;
use crate::commands::tool_version::is_newer;
use crate::commands::hidden_cmd;
use crate::commands::cache::{get_dir_size, format_bytes, create_junction, migrate_pkg_storage_impl, clean_pkg_cache_impl};
use super::models::*;
use super::skills::{normalize_path, load_skills, do_migrate_skills, save_skills};


fn ai_config_path() -> PathBuf {
    get_base_dir().join("ai_config.json")
}

fn ai_sessions_path() -> PathBuf {
    get_base_dir().join("ai_sessions.json")
}

fn last_launch_configs_path() -> PathBuf {
    get_base_dir().join("last_launch_configs.json")
}

// ─── 读写 ───

pub(crate) fn load_ai_config() -> AiConfig {
    let path = ai_config_path();
    if path.exists() {
        if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(config) = serde_json::from_str::<AiConfig>(&data) {
                return config;
            }
        }
    }
    AiConfig {
        providers: Vec::new(),
        active_provider: None,
        active_model: None,
        proxy_port: 15721,
        default_project_path: String::new(),
        skills_dir: String::new(),
        rectifier: RectifierConfig::default(),
        optimizer: OptimizerConfig::default(),
    }
}

fn save_ai_config_to_file(config: &AiConfig) -> Result<(), String> {
    let path = ai_config_path();
    let _ = fs::create_dir_all(path.parent().unwrap());
    let data = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    fs::write(path, data).map_err(|e| e.to_string())
}

pub(crate) fn load_sessions() -> AiSessionsFile {
    let path = ai_sessions_path();
    if path.exists() {
        if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(sessions) = serde_json::from_str::<AiSessionsFile>(&data) {
                return sessions;
            }
        }
    }
    AiSessionsFile::default()
}

pub(crate) fn save_sessions_to_file(sessions: &AiSessionsFile) -> Result<(), String> {
    let path = ai_sessions_path();
    let _ = fs::create_dir_all(path.parent().unwrap());
    let data = serde_json::to_string_pretty(sessions).map_err(|e| e.to_string())?;
    fs::write(path, data).map_err(|e| e.to_string())
}

pub(crate) fn load_last_launch_configs() -> LastLaunchConfigsFile {
    let path = last_launch_configs_path();
    if path.exists() {
        if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(configs) = serde_json::from_str::<LastLaunchConfigsFile>(&data) {
                return configs;
            }
        }
    }
    LastLaunchConfigsFile::default()
}

pub(crate) fn save_last_launch_configs(configs: &LastLaunchConfigsFile) -> Result<(), String> {
    let path = last_launch_configs_path();
    let _ = fs::create_dir_all(path.parent().unwrap());
    let data = serde_json::to_string_pretty(configs).map_err(|e| e.to_string())?;
    fs::write(path, data).map_err(|e| e.to_string())
}

// ─── Provider 预设（从 providers.json 加载）───

/// 获取所有 Provider/Relay 预设（从 ai-tools/providers.json 加载）
#[tauri::command]
pub fn get_provider_presets() -> Result<Vec<crate::commands::ai_registry::ProviderPresetDto>, String> {
    Ok(registry().providers().iter().map(|p| crate::commands::ai_registry::ProviderPresetDto {
        id: p.id.clone(),
        name: p.name.clone(),
        category: p.category.clone(),
        website: p.website.clone(),
        openai_url: p.openai_url.clone(),
        anthropic_url: p.anthropic_url.clone(),
        google_url: p.google_url.clone(),
    }).collect())
}

// ─── AI 工具检测 ───

/// AI 工具定义现在从 ai-tools/ 目录的 JSON 配置文件加载
/// 通过 ai_registry::registry() 访问，不再硬编码。
/// 新增工具只需在 ai-tools/ 下添加 config.json + paths.json。

// 为了向后兼容，保留 DetectedAiTool 类型，但它是 AiToolDefDto 的别名
pub type DetectedAiTool = AiToolDefDto;

#[tauri::command]
pub fn get_ai_config() -> Result<AiConfig, String> {
    Ok(load_ai_config())
}

#[tauri::command]
pub async fn save_ai_config(app: AppHandle, config: AiConfig) -> Result<serde_json::Value, String> {
    let old_config = load_ai_config();
    let old_dir = old_config.skills_dir.clone();
    let new_dir = config.skills_dir.clone();

    // 先保存新配置
    save_ai_config_to_file(&config)?;

    // 检测 skill 目录是否变更，执行迁移
    let mut skill_migrated = false;
    if !new_dir.is_empty() && normalize_path(&old_dir) != normalize_path(&new_dir) {
        let skills_file = load_skills();
        if !skills_file.skills.is_empty() && !old_dir.is_empty() {
            // Clone 需要移入闭包的值
            let old_dir_mv = old_dir.clone();
            let new_dir_mv = new_dir.clone();
            let skills_list = skills_file.skills.clone();
            let app_handle = app.clone();
            let result = tokio::task::spawn_blocking(move || {
                do_migrate_skills(&old_dir_mv, &new_dir_mv, &skills_list, Some(&app_handle))
            }).await.map_err(|e| e.to_string())?;
            skill_migrated = result.moved_count > 0 || result.rebuilt_junctions > 0;

            // 更新 skills.json 中的 directory 路径
            let mut updated_skills = skills_file;
            let old_skills_dir = PathBuf::from(&old_dir);
            let new_skills_dir = PathBuf::from(&new_dir);
            for skill in &mut updated_skills.skills {
                let old_path = PathBuf::from(&skill.directory);
                if let Ok(rel) = old_path.strip_prefix(&old_skills_dir) {
                    skill.directory = new_skills_dir.join(rel).to_string_lossy().to_string();
                }
            }
            save_skills(&updated_skills)?;
        }
    }

    let _ = app.emit("ai-config-changed", serde_json::json!({
        "default_project_path": &config.default_project_path,
        "skills_dir": &config.skills_dir,
        "providers_changed": true,
    }));
    Ok(serde_json::json!({
        "ok": true,
        "skill_migrated": skill_migrated,
    }))
}

// ─── Provider 模型获取 ───

#[tauri::command]
pub fn get_last_launch_config(tool_id: String) -> Result<Option<LastLaunchConfig>, String> {
    let configs = load_last_launch_configs();
    Ok(configs.configs.get(&tool_id).cloned())
}

/// 获取所有工具的上次启动配置（启动时前端加载）
#[tauri::command]
pub fn get_all_last_launch_configs() -> Result<std::collections::HashMap<String, LastLaunchConfig>, String> {
    let configs = load_last_launch_configs();
    Ok(configs.configs)
}

/// 保存工具的本次启动配置（启动成功后调用）
#[tauri::command]
pub fn save_last_launch_config(tool_id: String, config: LastLaunchConfig) -> Result<(), String> {
    let mut configs = load_last_launch_configs();
    configs.configs.insert(tool_id, config);
    save_last_launch_configs(&configs)
}
