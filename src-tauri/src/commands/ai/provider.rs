
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
    base_url: String,
    protocol: String,
    api_key: String,
) -> Result<serde_json::Value, String> {
    let url = base_url.trim().to_string();
    if url.is_empty() {
        return Err("未提供 API URL".to_string());
    }

    let test_url = format!("{}/models", url.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let start = std::time::Instant::now();

    // Google 端点用 x-goog-api-key 鉴权
    let resp = if protocol == "google" {
        client
            .get(&test_url)
            .header("x-goog-api-key", &api_key)
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
    } else {
        client
            .get(&test_url)
            .header("Authorization", format!("Bearer {}", api_key))
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
    }
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
        !p.api_key.is_empty() && !p.supported_protocols().is_empty()
    })
    .ok_or("没有配置了 API URL 的 Provider")?;

    // 手动启动（无工具上下文）：入站 = 出站 = 供应商首个支持的协议（同协议直连）
    let outbound_protocol = provider.primary_protocol();
    let inbound = outbound_protocol.clone();
    let outbound = outbound_protocol.clone();
    let conversion_mode = crate::proxy::types::derive_conversion_mode(&inbound, &outbound);

    let proxy_config = crate::proxy::types::ProxyConfig {
        listen_address: "127.0.0.1".to_string(),
        listen_port: port,
        inbound_protocols: vec![inbound],
        outbound_protocol: outbound,
        conversion_mode,
        upstream_api_key: provider.api_key.clone(),
        upstream_base_url: provider.url_for(&outbound_protocol),
        target_model: String::new(),
        timeout_secs: 300,
        model_aliases: std::collections::HashMap::new(),
        default_model: None,
        tool_id: String::new(),
        provider_id: provider.id.clone(),
        rectifier_enabled: config.rectifier.enabled,
        rectifier_thinking_signature: config.rectifier.thinking_signature,
        rectifier_thinking_budget: config.rectifier.thinking_budget,
        rectifier_media_fallback: config.rectifier.media_fallback,
        rectifier_protocol_mismatch: config.rectifier.protocol_mismatch,
        optimizer_enabled: config.optimizer.enabled,
        optimizer_cache_injection: config.optimizer.cache_injection,
        optimizer_thinking: config.optimizer.thinking_optimizer,
        optimizer_deepseek: config.optimizer.deepseek_normalize,
    };
    crate::proxy::server::start_proxy_server(proxy_config).await?;
    Ok(())
}
