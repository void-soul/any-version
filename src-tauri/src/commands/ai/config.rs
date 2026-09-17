use std::fs;
use std::path::PathBuf;
use tauri::AppHandle;
use tauri::Emitter;
use crate::commands::ai_registry::{registry, AiToolDefDto};
use crate::commands::config::get_data_dir;
use super::models::*;
use super::skills::{normalize_path, load_skills, do_migrate_skills, save_skills, resolve_skills_dir};


fn ai_config_path() -> PathBuf {
    get_data_dir().join("ai_config.json")
}

fn ai_sessions_path() -> PathBuf {
    get_data_dir().join("ai_sessions.json")
}

fn last_launch_configs_path() -> PathBuf {
    get_data_dir().join("last_launch_configs.json")
}

// ─── 读写 ───

pub(crate) fn load_ai_config() -> AiConfig {
    let path = ai_config_path();
    if path.exists() {
        if let Ok(data) = fs::read_to_string(&path) {
            // AiProvider 的自定义 Deserialize 已内置迁移逻辑：
            // 自动将旧版 protocols HashMap / 旧版扁平字段转换为新版扁平 URL 结构。
            if let Ok(mut config) = serde_json::from_str::<AiConfig>(&data) {
                let mut save_needed = false;
                // 预设分类同步：providers.json 里某预设改了 category（如 free-router /
                // workbuddy2api → local）后，已保存供应商仍存着旧分类，这里自动跟随，
                // 避免列表徽标与预设库不一致。仅同步与预设 id 完全一致的条目。
                {
                    let presets = crate::commands::ai_registry::registry().providers();
                    if sync_provider_categories(&mut config.providers, presets) {
                        save_needed = true;
                        eprintln!("[config] 已按预设同步供应商分类");
                    }
                }
                // 检测是否需要迁移（旧格式 → 新格式），若需要则回写
                if let Ok(raw) = serde_json::from_str::<serde_json::Value>(&data) {
                    let needs_migrate = raw.get("providers")
                        .and_then(|p| p.as_array())
                        .map(|arr| arr.iter().any(|p| p.get("protocols").is_some()
                            || p.get("openai_enabled").is_some()
                            || p.get("anthropic_use_proxy").is_some()))
                        .unwrap_or(false);
                    if needs_migrate {
                        save_needed = true;
                        eprintln!("[config] ✓ 已迁移 ai_config.json 至扁平 URL 格式");
                    }
                    // 旧版加密 API key（ENC_V2: 前缀）→ 回写为明文存储，一次性原地迁移。
                    // 注意：deserialize 后 api_key 已是解密明文，须从原始 raw JSON 里检测。
                    let has_enc_key = raw.get("providers")
                        .and_then(|p| p.as_array())
                        .map(|arr| arr.iter().any(|p| p.get("api_key")
                            .and_then(|k| k.as_str())
                            .map(|s| s.starts_with("ENC_V2:"))
                            .unwrap_or(false)))
                        .unwrap_or(false);
                    if has_enc_key {
                        save_needed = true;
                        eprintln!("[config] 检测到加密 API key，迁移为明文存储");
                    }
                }
                if save_needed {
                    let _ = save_ai_config_to_file(&config);
                }
                return config;
            }
        }
    }
    AiConfig {
        providers: Vec::new(),
        proxy_port: 15721,
        default_project_path: String::new(),
        skills_dir: String::new(),
        rectifier: RectifierConfig::default(),
        headroom: HeadroomConfig::default(),
        optimizer: OptimizerConfig::default(),
        tool_symlinks: std::collections::HashMap::new(),
        route_chain: Vec::new(),
        aggregate: AggregateConfig::default(),
    }
}

pub(crate) fn save_ai_config_to_file(config: &AiConfig) -> Result<(), String> {
    let data = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    crate::commands::config::atomic_write_file(&ai_config_path(), data.as_bytes())
}

/// 把已保存供应商的 category 与预设同步（仅 id 与预设完全一致的条目）。
/// 返回是否发生变更。
pub(crate) fn sync_provider_categories(
    providers: &mut [crate::commands::ai::models::AiProvider],
    presets: &[crate::commands::ai_registry::ProviderPreset],
) -> bool {
    let mut changed = false;
    for provider in providers.iter_mut() {
        if let Some(preset) = presets.iter().find(|x| x.id == provider.id) {
            if provider.category != preset.category {
                provider.category = preset.category.clone();
                changed = true;
            }
        }
    }
    changed
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
    let data = serde_json::to_string_pretty(sessions).map_err(|e| e.to_string())?;
    crate::commands::config::atomic_write_file(&ai_sessions_path(), data.as_bytes())
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
    let data = serde_json::to_string_pretty(configs).map_err(|e| e.to_string())?;
    crate::commands::config::atomic_write_file(&last_launch_configs_path(), data.as_bytes())
}

// ─── Provider 预设（从 providers.json 加载）───

/// 获取所有 Provider/Relay 预设（从 ai-tools/providers.json 加载）
#[tauri::command]
pub fn get_provider_presets() -> Result<Vec<crate::commands::ai_registry::ProviderPresetDto>, String> {
    // 本地聚合预设的端口取自「聚合」页设置（预设里写 `{port}` 占位符）
    let port = load_ai_config().aggregate.port;
    Ok(registry().providers().iter().map(|p| {
        let mut dto = crate::commands::ai_registry::ProviderPresetDto {
            id: p.id.clone(),
            name: p.name.clone(),
            category: p.category.clone(),
            website: p.website.clone(),
            openai_url: p.openai_url.clone(),
            anthropic_url: p.anthropic_url.clone(),
            google_url: p.google_url.clone(),
        };
        render_local_placeholders(&mut dto, port);
        dto
    }).collect())
}

/// 把预设里的 `{port}` / `{port:v1}` 占位符渲染成本地服务的实际端口。
/// 目前只有「本地聚合」用（端口在聚合页可改），其余预设不受影响。
pub(crate) fn render_local_placeholders(
    preset: &mut crate::commands::ai_registry::ProviderPresetDto,
    port: u16,
) {
    let port_s = port.to_string();
    for value in [
        &mut preset.openai_url,
        &mut preset.anthropic_url,
        &mut preset.google_url,
        &mut preset.website,
    ] {
        if value.contains("{port}") {
            *value = value.replace("{port}", &port_s);
        }
    }
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
    // 解析为实际路径（空字符串 → 默认 ~/.agents/skills），使默认目录与新目录间的迁移也能正确触发
    let old_resolved = resolve_skills_dir(&old_config.skills_dir);
    let new_resolved = resolve_skills_dir(&config.skills_dir);

    // 先保存新配置
    save_ai_config_to_file(&config)?;

    // 检测 skill 目录是否变更，执行迁移
    let mut skill_migrated = false;
    if normalize_path(&old_resolved.to_string_lossy()) != normalize_path(&new_resolved.to_string_lossy()) {
        let skills_file = load_skills();
        if !skills_file.skills.is_empty() {
            // Clone 需要移入闭包的值
            let old_str = old_resolved.to_string_lossy().to_string();
            let new_str = new_resolved.to_string_lossy().to_string();
            let skills_list = skills_file.skills.clone();
            let app_handle = app.clone();
            let result = tokio::task::spawn_blocking(move || {
                do_migrate_skills(&old_str, &new_str, &skills_list, Some(&app_handle))
            }).await.map_err(|e| e.to_string())?;
            skill_migrated = result.moved_count > 0 || result.rebuilt_junctions > 0;

            // 更新 skills.json 中的 directory 路径
            let mut updated_skills = skills_file;
            for skill in &mut updated_skills.skills {
                let old_path = PathBuf::from(&skill.directory);
                if let Ok(rel) = old_path.strip_prefix(&old_resolved) {
                    skill.directory = new_resolved.join(rel).to_string_lossy().to_string();
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
    fn test_sync_provider_categories_follows_presets() {
        use crate::commands::ai::models::{AiProvider, ModelEntry};
        use crate::commands::ai_registry::ProviderPreset;

        let preset = |id: &str, category: &str| ProviderPreset {
            id: id.to_string(),
            name: id.to_string(),
            category: category.to_string(),
            website: String::new(),
            openai_url: "http://127.0.0.1:1/v1".to_string(),
            anthropic_url: String::new(),
            google_url: String::new(),
        };
        let presets = vec![
            preset("free-router", "local"),
            preset("openai", "provider"),
        ];
        let provider = |id: &str, category: &str| AiProvider {
            id: id.to_string(),
            name: id.to_string(),
            category: category.to_string(),
            api_key: String::new(),
            website: String::new(),
            openai_url: String::new(),
            anthropic_url: String::new(),
            google_url: String::new(),
            models: Vec::<ModelEntry>::new(),
            active_model_id: None,
        };

        let mut providers = vec![
            provider("free-router", "relay"), // 旧分类 → 应同步为 local
            provider("openai", "provider"),   // 一致 → 不变
            provider("custom_abc", "relay"),  // 自定义（无预设）→ 不动
        ];
        assert!(sync_provider_categories(&mut providers, &presets));
        assert_eq!(providers[0].category, "local");
        assert_eq!(providers[1].category, "provider");
        assert_eq!(providers[2].category, "relay");

        // 已一致时不再产生变更
        assert!(!sync_provider_categories(&mut providers, &presets));
    }

    // ─── 本地预设占位符渲染（本地聚合：端口取自聚合页设置） ───

    #[test]
    fn test_render_local_placeholders_replaces_port() {
        let mut preset = crate::commands::ai_registry::ProviderPresetDto {
            id: "local-aggregate".into(),
            name: "本地聚合".into(),
            category: "local".into(),
            website: String::new(),
            openai_url: "http://127.0.0.1:{port}/v1".into(),
            anthropic_url: String::new(),
            google_url: String::new(),
        };
        render_local_placeholders(&mut preset, 15888);
        assert_eq!(preset.openai_url, "http://127.0.0.1:15888/v1");
        // 再换端口仍可幂等重渲染的前提：已被替换过就不再含占位符
        render_local_placeholders(&mut preset, 16000);
        assert_eq!(preset.openai_url, "http://127.0.0.1:15888/v1");
    }

    #[test]
    fn test_render_local_placeholders_leaves_normal_presets() {
        let mut preset = crate::commands::ai_registry::ProviderPresetDto {
            id: "openai".into(),
            name: "OpenAI".into(),
            category: "provider".into(),
            website: "https://openai.com".into(),
            openai_url: "https://api.openai.com/v1".into(),
            anthropic_url: String::new(),
            google_url: String::new(),
        };
        render_local_placeholders(&mut preset, 15888);
        assert_eq!(preset.openai_url, "https://api.openai.com/v1");
        assert_eq!(preset.website, "https://openai.com");
    }

    #[test]
    fn test_local_aggregate_preset_exists_with_port_placeholder() {
        // providers.json 里必须有本地聚合预设，且端口用占位符（由后端按聚合页端口渲染）
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../ai-tools/providers.json");
        let raw = std::fs::read_to_string(&path).expect("providers.json 读取失败");
        let presets: Vec<crate::commands::ai_registry::ProviderPreset> =
            serde_json::from_str(&raw).expect("providers.json 解析失败");
        let entry = presets
            .iter()
            .find(|p| p.id == "local-aggregate")
            .expect("缺少 local-aggregate 本地聚合预设");
        assert_eq!(entry.category, "local");
        assert!(entry.openai_url.contains("{port}"), "端口应使用 {{port}} 占位符以便动态渲染");
    }

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
        // 旧格式 v1 扁平字段：openai_url / anthropic_url 直接读取，default_model 已丢弃
        assert_eq!(p.openai_url, "https://api.longcat.com/v1");
        assert_eq!(p.anthropic_url, "https://api.longcat.com/anthropic");
        assert_eq!(p.google_url, "");
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
            "proxy_port": 15721,
            "default_project_path": "",
            "rectifier": {"enabled": true, "thinking_signature": true, "thinking_budget": true, "media_fallback": true},
            "optimizer": {"enabled": true, "cache_injection": true, "thinking_optimizer": true, "deepseek_normalize": true},
            "skills_dir": ""
        }"#;

        let config: AiConfig = serde_json::from_str(protocols_json).expect("Should deserialize & migrate protocols format");
        assert_eq!(config.providers.len(), 1);
        let p = &config.providers[0];
        // protocols 格式逐协议填入对应 URL 字段
        assert_eq!(p.openai_url, "https://api.deepseek.com");
        assert_eq!(p.anthropic_url, "https://api.deepseek.com/anthropic");
        assert_eq!(p.google_url, "");
    }

    #[test]
    fn test_new_flat_format() {
        // 测试新格式直接反序列化（纯供应商：单一协议 + base_url）
        let new_json = r#"{
            "providers": [
                {
                    "id": "openai",
                    "name": "OpenAI",
                    "category": "provider",
                    "api_key": "sk-test",
                    "website": "https://openai.com",
                    "base_url": "https://api.openai.com/v1",
                    "protocol": "openai",
                    "default_model": null,
                    "models": [{"id": "gpt-4o", "name": "gpt-4o"}],
                    "active_model_id": null
                }
            ],
            "proxy_port": 15721,
            "default_project_path": "",
            "rectifier": {"enabled": true, "thinking_signature": true, "thinking_budget": true, "media_fallback": true},
            "optimizer": {"enabled": true, "cache_injection": true, "thinking_optimizer": true, "deepseek_normalize": true},
            "skills_dir": ""
        }"#;

        let config: AiConfig = serde_json::from_str(new_json).expect("Should deserialize new flat format");
        assert_eq!(config.providers.len(), 1);
        let p = &config.providers[0];
        // 旧格式 v3：单一 base_url + protocol 折叠为对应协议 URL
        assert_eq!(p.openai_url, "https://api.openai.com/v1");
        assert_eq!(p.anthropic_url, "");
        assert_eq!(p.google_url, "");
        assert_eq!(p.models.len(), 1);
    }

    #[test]
    fn test_api_key_plaintext_at_rest() {
        use crate::commands::ai::models::ModelEntry;
        let config = AiConfig {
            providers: vec![AiProvider {
                id: "test-prov".to_string(),
                name: "Test".to_string(),
                category: "provider".to_string(),
                api_key: "sk-plaintext-key-12345".to_string(),
                website: String::new(),
                openai_url: "https://api.example.com/v1".to_string(),
                anthropic_url: String::new(),
                google_url: String::new(),
                models: vec![ModelEntry { id: "m1".into(), name: "M1".into(), custom_params: vec![] }],
                active_model_id: None,
            }],
            proxy_port: 15721,
            default_project_path: String::new(),
            rectifier: RectifierConfig::default(),
            headroom: HeadroomConfig::default(),
            optimizer: OptimizerConfig::default(),
            skills_dir: String::new(),
            tool_symlinks: std::collections::HashMap::new(),
            route_chain: Vec::new(),
            aggregate: AggregateConfig::default(),
        };

        let raw = serde_json::to_string(&config).expect("serialize");
        // api_key 明文落盘（不再加密）
        assert!(raw.contains("sk-plaintext-key-12345"), "api_key 应明文落盘，实际: {}", raw);
        assert!(!raw.contains("ENC_V2:"), "api_key 不应再加密存储");

        let back: AiConfig = serde_json::from_str(&raw).expect("deserialize");
        assert_eq!(back.providers[0].api_key, "sk-plaintext-key-12345", "明文读取应保持原值");
    }

    #[test]
    fn test_legacy_encrypted_key_still_readable() {
        // 旧版本加密存储的 api_key（ENC_V2 前缀）读取时应能解出明文，兼容历史配置
        use crate::commands::ai::models::ModelEntry;
        let enc = crate::commands::secrets::encrypt_secret("sk-legacy-encrypted").expect("encrypt");
        let json = format!(
            r#"{{"providers":[{{"id":"p1","name":"P1","category":"provider","api_key":"{}","website":"","openai_url":"https://x/v1","anthropic_url":"","google_url":"","models":[],"active_model_id":null}}],"proxy_port":15721,"default_project_path":"","rectifier":{{}},"optimizer":{{}},"skills_dir":""}}"#,
            enc
        );
        let config: AiConfig = serde_json::from_str(&json).expect("deserialize legacy encrypted");
        assert_eq!(config.providers[0].api_key, "sk-legacy-encrypted");
    }

    #[test]
    fn test_legacy_plaintext_key_stays_readable() {
        // 旧版本明文 key（无 ENC_V2 前缀）读取后应保持原值，等待下次保存时加密
        let legacy = r#"{"providers":[{"id":"p1","name":"P1","category":"provider","api_key":"sk-legacy","website":"","openai_url":"https://x/v1","anthropic_url":"","google_url":"","models":[],"active_model_id":null}],"proxy_port":15721,"default_project_path":"","rectifier":{},"optimizer":{},"skills_dir":""}"#;
        let config: AiConfig = serde_json::from_str(legacy).expect("deserialize legacy");
        assert_eq!(config.providers[0].api_key, "sk-legacy");
    }
}
