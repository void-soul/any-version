
use super::config::load_ai_config;

/// 模型列表接口超时（秒）：列表接口应快速返回，超时即报错。
const FETCH_MODELS_TIMEOUT_SECS: u64 = 15;
/// 非 JSON 错误体的摘要上限（字符）：可能是整页 HTML 网关页或长文，只留可诊断的开头。
/// 抄自 CodexPlusPlus b498c4c 的「非 2xx 附带上游错误体摘要」。
const UPSTREAM_ERROR_SNIPPET_CHARS: usize = 200;

/// 从上游响应体里提取一句可读的错误原因。
///
/// 兼容常见形态（抄自 CodexPlusPlus b498c4c）：
/// - OpenAI 风格 `{"error": {"message": "..."}}`
/// - 业务信封 `{"code": 401, "msg": "..."}` / `{"message": "..."}`
/// - 非 JSON（HTML 网关页、纯文本）：截断原文
fn upstream_error_reason(raw: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) {
        for candidate in [
            v.pointer("/error/message"),
            v.pointer("/error/msg"),
            v.pointer("/msg"),
            v.pointer("/message"),
            v.pointer("/error"),
        ] {
            if let Some(text) = candidate
                .and_then(|x| x.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                return text.to_string();
            }
        }
    }
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    trimmed.chars().take(UPSTREAM_ERROR_SNIPPET_CHARS).collect()
}

/// 判断 HTTP 200 的响应体是不是「业务错误信封」并取出原因。
///
/// 部分网关（如智谱 Codex 专属端点）在 key 缺失/失效时返回 **200 + `{"code":401,"msg":"令牌已过期"}`**，
/// 只按状态码判断会把真因吃成「上游没有可用模型」，用户完全看不到令牌问题。
/// 仅在解析不到任何模型时调用，因此不会误伤正常返回（有数据时优先信数据）。
fn business_error_reason(body: &serde_json::Value) -> Option<String> {
    let reason = ["msg", "message"]
        .iter()
        .find_map(|key| body.get(*key).and_then(|v| v.as_str()))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let explicit_fail = body.get("success").and_then(|v| v.as_bool()) == Some(false)
        || body
            .get("code")
            .and_then(|v| v.as_i64())
            .is_some_and(|c| c != 0 && c != 200)
        || body.get("error").is_some();

    if !explicit_fail {
        return None;
    }
    Some(reason.unwrap_or_else(|| "上游返回业务错误但未提供原因".to_string()))
}

/// 解析模型列表：同时兼容 OpenAI 风格 `data[].id` 与 Gemini 风格 `models[].name`，
/// 后者剥掉 `models/` 前缀使模型 ID 与请求体的 `model` 值一致（抄自 CodexPlusPlus b498c4c）。
fn parse_model_ids(body: &serde_json::Value) -> Vec<String> {
    fn push(ids: &mut Vec<String>, raw: &str) {
        let id = raw.trim();
        if id.is_empty() {
            return;
        }
        let id = id.strip_prefix("models/").unwrap_or(id);
        if !ids.iter().any(|x| x == id) {
            ids.push(id.to_string());
        }
    }

    let mut ids: Vec<String> = Vec::new();
    if let Some(arr) = body.get("data").and_then(|v| v.as_array()) {
        for m in arr {
            if let Some(s) = m.get("id").and_then(|v| v.as_str()) {
                push(&mut ids, s);
            }
        }
    }
    if let Some(arr) = body.get("models").and_then(|v| v.as_array()) {
        for m in arr {
            if let Some(s) = m.get("name").and_then(|v| v.as_str()) {
                push(&mut ids, s);
            }
        }
    }
    ids
}

#[tauri::command]
pub async fn fetch_provider_models(base_url: String, api_key: String) -> Result<Vec<String>, String> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .timeout(std::time::Duration::from_secs(FETCH_MODELS_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    let status = resp.status();
    // 先取原文再解析：非 JSON 的错误体（HTML 网关页、纯文本）也要能看到原因；
    // 若像以前那样先 json()，解析失败会提前返回，状态码与真因一起丢失。
    let raw = resp.text().await.map_err(|e| format!("读取响应失败: {}", e))?;

    if !status.is_success() {
        let reason = upstream_error_reason(&raw);
        return Err(if reason.is_empty() {
            format!("API 返回错误 (HTTP {})", status.as_u16())
        } else {
            format!("API 返回错误 (HTTP {}): {}", status.as_u16(), reason)
        });
    }

    let body: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| format!("解析响应失败: {}（原文: {}）", e, upstream_error_reason(&raw)))?;

    let models = parse_model_ids(&body);
    if models.is_empty() {
        // 200 + 业务错误信封（鉴权失败等）→ 透传真因，而不是笼统的「未获取到模型列表」
        if let Some(reason) = business_error_reason(&body) {
            return Err(reason);
        }
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
    // 端口可被环境变量整体挪走（Windows 保留端口 / Hyper-V 排除区间场景），见 PROXY_PORT_ENV。
    let requested_port = port;
    let port = crate::proxy::resolve_proxy_port(requested_port);
    if port != requested_port {
        eprintln!(
            "[proxy] 端口被环境变量 {} 覆盖: {} -> {}",
            crate::proxy::PROXY_PORT_ENV,
            requested_port,
            port
        );
    }

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
        auth_token: String::new(),
        inbound_protocols: vec![inbound],
        outbound_protocol: outbound,
        conversion_mode,
        upstream_api_key: provider.api_key.clone(),
        upstream_base_url: provider.url_for(&outbound_protocol),
        fallback_base_url: String::new(),
        fallback_api_key: String::new(),
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
        model_routes: std::collections::HashMap::new(),
        app_handle: None,
        collab_room_id: None,
    };
    crate::proxy::server::start_proxy_server(proxy_config).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{business_error_reason, parse_model_ids, upstream_error_reason};
    use serde_json::json;

    #[test]
    fn upstream_error_reason_prefers_error_message() {
        let raw = r#"{"error":{"message":"Invalid API key provided"}}"#;
        assert_eq!(upstream_error_reason(raw), "Invalid API key provided");
    }

    #[test]
    fn upstream_error_reason_reads_business_envelope_msg() {
        assert_eq!(
            upstream_error_reason(r#"{"code":401,"msg":"令牌已过期或验证不正确"}"#),
            "令牌已过期或验证不正确"
        );
        // error 是字符串而不是对象时也要能取到
        assert_eq!(upstream_error_reason(r#"{"error":"rate limited"}"#), "rate limited");
    }

    #[test]
    fn upstream_error_reason_truncates_non_json_body() {
        let raw = format!("<html>{}</html>", "x".repeat(500));
        let reason = upstream_error_reason(&raw);
        assert_eq!(reason.chars().count(), 200);
        assert!(reason.starts_with("<html>"));
    }

    #[test]
    fn business_error_reason_detects_http_200_error_envelope() {
        // 智谱类网关：鉴权失败返回 200 + 业务错误信封
        let body = json!({"code": 401, "msg": "令牌已过期或验证不正确", "success": false});
        assert_eq!(
            business_error_reason(&body).as_deref(),
            Some("令牌已过期或验证不正确")
        );
    }

    #[test]
    fn business_error_reason_ignores_legit_empty_list() {
        // 正常返回但确实没有模型：不应误报成业务错误
        assert_eq!(business_error_reason(&json!({"data": []})), None);
        assert_eq!(business_error_reason(&json!({"object": "list", "data": []})), None);
        // code=0 / 200 表示成功，不算失败档
        assert_eq!(business_error_reason(&json!({"code": 0, "data": []})), None);
        assert_eq!(business_error_reason(&json!({"code": 200, "data": []})), None);
    }

    #[test]
    fn parse_model_ids_supports_openai_and_gemini_shapes() {
        let openai = json!({"data": [{"id": "gpt-5"}, {"id": "gpt-5"}]});
        assert_eq!(parse_model_ids(&openai), vec!["gpt-5".to_string()]);

        // Gemini 风格：name 带 models/ 前缀，输出要与请求体的 model 值一致
        let gemini = json!({"models": [{"name": "models/gemini-2.5-pro"}, {"name": "models/gemini-2.5-flash"}]});
        assert_eq!(
            parse_model_ids(&gemini),
            vec!["gemini-2.5-pro".to_string(), "gemini-2.5-flash".to_string()]
        );

        assert!(parse_model_ids(&json!({})).is_empty());
    }
}
