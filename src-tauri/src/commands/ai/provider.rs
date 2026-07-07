
use super::config::load_ai_config;

#[tauri::command]
pub async fn fetch_provider_models(base_url: String, api_key: String) -> Result<Vec<String>, String> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    let status = resp.status();
    let body: serde_json::Value = resp.json().await.map_err(|e| format!("解析响应失败: {}", e))?;

    if !status.is_success() {
        let msg = body.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str()).unwrap_or("未知错误");
        return Err(format!("API 返回错误 ({}): {}", status.as_u16(), msg));
    }

    let models: Vec<String> = body.get("data")
        .and_then(|v| v.as_array())
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|m| m.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();

    if models.is_empty() {
        return Err("未获取到模型列表".to_string());
    }
    Ok(models)
}

// ─── 用量统计 ───

// ─── 模型连接测试 ───

#[tauri::command]
pub async fn test_model_connection(
    openai_url: Option<String>,
    anthropic_url: Option<String>,
    api_key: String,
) -> Result<serde_json::Value, String> {
    let url = openai_url
        .filter(|u| !u.is_empty())
        .or_else(|| anthropic_url.filter(|u| !u.is_empty()))
        .unwrap_or_default();

    if url.is_empty() {
        return Err("未提供 API URL".to_string());
    }

    let test_url = format!("{}/models", url.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let start = std::time::Instant::now();

    let resp = client
        .get(&test_url)
        .header("Authorization", format!("Bearer {}", api_key))
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("连接失败: {}", e))?;

    let latency_ms = start.elapsed().as_millis() as u64;
    let status = resp.status();

    Ok(serde_json::json!({
        "success": status.is_success(),
        "message": if status.is_success() { "连接成功".to_string() } else { format!("HTTP {}", status.as_u16()) },
        "latency_ms": latency_ms,
    }))
}
// ─── 代理服务器 ───

#[tauri::command]
pub async fn start_proxy(port: u16) -> Result<(), String> {
    let config = load_ai_config();
    let provider = config.providers.iter().find(|p| {
        p.protocols.values().any(|c| c.use_proxy)
    })
    .ok_or("没有配置了代理的 Provider")?;

    // Anthropic 代理（P1: port）
    if let Some(cfg) = provider.protocols.get("anthropic") {
        if cfg.use_proxy {
            let openai_url = provider.protocols.get("openai").map(|c| c.url.clone()).unwrap_or_default();
            let proxy_config = crate::proxy::types::ProxyConfig {
                listen_address: "127.0.0.1".to_string(),
                listen_port: port,
                upstream_base_url: openai_url,
                upstream_api_key: provider.api_key.clone(),
                upstream_anthropic_url: cfg.url.clone(),
                upstream_protocol: "anthropic".to_string(),
                target_model: String::new(),
                timeout_secs: 300,
                model_aliases: cfg.model_aliases.clone(),
                default_model: cfg.default_model.clone(),
                rectifier_enabled: config.rectifier.enabled,
                rectifier_thinking_signature: config.rectifier.thinking_signature,
                rectifier_thinking_budget: config.rectifier.thinking_budget,
                rectifier_media_fallback: config.rectifier.media_fallback,
                optimizer_enabled: config.optimizer.enabled,
                optimizer_cache_injection: config.optimizer.cache_injection,
                optimizer_thinking: config.optimizer.thinking_optimizer,
                optimizer_deepseek: config.optimizer.deepseek_normalize,
            };
            crate::proxy::server::start_proxy_server(proxy_config).await?;
        }
    }

    // OpenAI 代理（P2: port + 1）
    if let Some(cfg) = provider.protocols.get("openai") {
        if cfg.use_proxy {
            let proxy_config = crate::proxy::types::ProxyConfig {
                listen_address: "127.0.0.1".to_string(),
                listen_port: port + 1,
                upstream_base_url: cfg.url.clone(),
                upstream_api_key: provider.api_key.clone(),
                upstream_anthropic_url: String::new(),
                upstream_protocol: "openai".to_string(),
                target_model: String::new(),
                timeout_secs: 300,
                model_aliases: cfg.model_aliases.clone(),
                default_model: cfg.default_model.clone(),
                rectifier_enabled: false,
                rectifier_thinking_signature: false,
                rectifier_thinking_budget: false,
                rectifier_media_fallback: false,
                optimizer_enabled: false,
                optimizer_cache_injection: false,
                optimizer_thinking: false,
                optimizer_deepseek: false,
            };
            crate::proxy::server::start_proxy_server(proxy_config).await?;
        }
    }

    Ok(())
}
