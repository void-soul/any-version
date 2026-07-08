use std::fs;
use std::path::PathBuf;
use tauri::AppHandle;
use tauri::Emitter;
use crate::commands::ai_registry::{registry, AiToolDefDto};
use crate::commands::config::get_base_dir;
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
            // AiProvider 的自定义 Deserialize 已内置迁移逻辑：
            // 自动将旧版 protocols HashMap / 旧版扁平字段转换为新版扁平 URL 结构。
            if let Ok(config) = serde_json::from_str::<AiConfig>(&data) {
                // 检测是否需要迁移（旧格式 → 新格式），若需要则回写
                if let Ok(raw) = serde_json::from_str::<serde_json::Value>(&data) {
                    let needs_migrate = raw.get("providers")
                        .and_then(|p| p.as_array())
                        .map(|arr| arr.iter().any(|p| p.get("protocols").is_some()
                            || p.get("openai_enabled").is_some()
                            || p.get("anthropic_use_proxy").is_some()))
                        .unwrap_or(false);
                    if needs_migrate {
                        let _ = save_ai_config_to_file(&config);
                        eprintln!("[config] ✓ 已迁移 ai_config.json 至扁平 URL 格式");
                    }
                }
                return config;
            }
        }
    }
    AiConfig {
        providers: Vec::new(),
        active_provider: None,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_migrate_old_provider_json() {
        // 测试从旧版扁平字段格式迁移
        let old_json = r#"{
            "providers": [
                {
                    "id": "longcat",
                    "name": "LongCat",
                    "category": "relay",
                    "api_key": "test_key",
                    "website": "https://example.com",
                    "openai_enabled": true,
                    "openai_url": "https://api.longcat.com/v1",
                    "openai_use_proxy": false,
                    "anthropic_enabled": true,
                    "anthropic_url": "https://api.longcat.com/anthropic",
                    "anthropic_use_proxy": true,
                    "google_enabled": false,
                    "google_url": "",
                    "anthropic_model_aliases": {
                        "fable": "gpt-4o"
                    },
                    "anthropic_default_model": "gpt-4o",
                    "openai_model_aliases": {},
                    "openai_default_model": null,
                    "google_model_aliases": {},
                    "google_default_model": null,
                    "models": [],
                    "active_model_id": null
                }
            ],
            "active_provider": "longcat",
            "proxy_port": 15721,
            "default_project_path": "",
            "rectifier": {"enabled": false, "thinking_signature": false, "thinking_budget": false, "media_fallback": false},
            "optimizer": {"enabled": false, "cache_injection": false, "thinking_optimizer": false, "deepseek_normalize": false},
            "skills_dir": ""
        }"#;

        let config: AiConfig = serde_json::from_str(old_json).expect("Should deserialize & migrate old config");
        assert_eq!(config.providers.len(), 1);
        let p = &config.providers[0];
        assert_eq!(p.id, "longcat");
        assert_eq!(p.api_key, "test_key");
        assert_eq!(p.openai_url, "https://api.longcat.com/v1");
        assert_eq!(p.anthropic_url, "https://api.longcat.com/anthropic");
        assert_eq!(p.google_url, "");
        assert_eq!(p.default_model.as_deref(), Some("gpt-4o"));
        assert_eq!(p.model_aliases.get("fable").map(|s| s.as_str()), Some("gpt-4o"));
    }

    #[test]
    fn test_migrate_protocols_format() {
        // 测试从 protocols HashMap 格式迁移
        let protocols_json = r#"{
            "providers": [
                {
                    "id": "deepseek",
                    "name": "DeepSeek",
                    "category": "provider",
                    "api_key": "sk-test",
                    "website": "",
                    "protocols": {
                        "openai": {"enabled": true, "url": "https://api.deepseek.com", "use_proxy": false, "model_aliases": {}, "default_model": null},
                        "anthropic": {"enabled": true, "url": "https://api.deepseek.com/anthropic", "use_proxy": true, "model_aliases": {"sonnet": "deepseek-chat"}, "default_model": "deepseek-chat"},
                        "google": {"enabled": false, "url": "", "use_proxy": false, "model_aliases": {}, "default_model": null}
                    },
                    "models": [],
                    "active_model_id": null
                }
            ],
            "active_provider": "deepseek",
            "proxy_port": 15721,
            "default_project_path": "",
            "rectifier": {"enabled": true, "thinking_signature": true, "thinking_budget": true, "media_fallback": true},
            "optimizer": {"enabled": true, "cache_injection": true, "thinking_optimizer": true, "deepseek_normalize": true},
            "skills_dir": ""
        }"#;

        let config: AiConfig = serde_json::from_str(protocols_json).expect("Should deserialize & migrate protocols format");
        assert_eq!(config.providers.len(), 1);
        let p = &config.providers[0];
        assert_eq!(p.openai_url, "https://api.deepseek.com");
        assert_eq!(p.anthropic_url, "https://api.deepseek.com/anthropic");
        assert_eq!(p.model_aliases.get("sonnet").map(|s| s.as_str()), Some("deepseek-chat"));
        assert_eq!(p.default_model.as_deref(), Some("deepseek-chat"));
    }

    #[test]
    fn test_new_flat_format() {
        // 测试新格式直接反序列化
        let new_json = r#"{
            "providers": [
                {
                    "id": "openai",
                    "name": "OpenAI",
                    "category": "provider",
                    "api_key": "sk-test",
                    "website": "https://openai.com",
                    "openai_url": "https://api.openai.com/v1",
                    "anthropic_url": "",
                    "google_url": "",
                    "model_aliases": {},
                    "default_model": null,
                    "models": [{"id": "gpt-4o", "name": "gpt-4o"}],
                    "active_model_id": null
                }
            ],
            "active_provider": "openai",
            "proxy_port": 15721,
            "default_project_path": "",
            "rectifier": {"enabled": true, "thinking_signature": true, "thinking_budget": true, "media_fallback": true},
            "optimizer": {"enabled": true, "cache_injection": true, "thinking_optimizer": true, "deepseek_normalize": true},
            "skills_dir": ""
        }"#;

        let config: AiConfig = serde_json::from_str(new_json).expect("Should deserialize new flat format");
        assert_eq!(config.providers.len(), 1);
        let p = &config.providers[0];
        assert_eq!(p.openai_url, "https://api.openai.com/v1");
        assert_eq!(p.anthropic_url, "");
        assert_eq!(p.models.len(), 1);
    }
}
