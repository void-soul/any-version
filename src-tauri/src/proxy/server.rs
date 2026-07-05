//! 代理 HTTP 服务器
//!
//! 基于 Axum 的本地代理，拦截 Anthropic 协议请求，
//! 转换为 OpenAI 格式后转发到上游，响应再转回 Anthropic 格式。

use super::{optimizers, sse, transform, types::ProxyConfig};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{sse::Event, IntoResponse, Response, Sse},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 记录使用量到文件（从代理层自动提取 token 信息）
fn record_proxy_usage(model: &str, input_tokens: u64, output_tokens: u64) {
    let usage_dir = dirs_next().unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
        .join(".any-version");
    let _ = std::fs::create_dir_all(&usage_dir);
    let usage_path = usage_dir.join("ai_usage.json");

    let mut records: Vec<Value> = Vec::new();
    if usage_path.exists() {
        if let Ok(data) = std::fs::read_to_string(&usage_path) {
            if let Ok(val) = serde_json::from_str::<Value>(&data) {
                if let Some(arr) = val.get("records").and_then(|v| v.as_array()) {
                    records = arr.clone();
                }
            }
        }
    }

    let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    records.push(json!({
        "tool_id": "proxy",
        "model": model,
        "provider": null,
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "timestamp": now,
    }));

    let data = json!({"records": records});
    let _ = std::fs::write(&usage_path, serde_json::to_string_pretty(&data).unwrap_or_default());
}

fn dirs_next() -> Option<std::path::PathBuf> {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()
        .map(std::path::PathBuf::from)
    }

/// 代理服务器共享状态
#[derive(Clone)]
pub struct ProxyState {
    pub config: Arc<RwLock<ProxyConfig>>,
    pub client: reqwest::Client,
    pub stats: Arc<RwLock<ProxyStats>>,
}

#[derive(Default)]
pub struct ProxyStats {
    pub total_requests: u64,
    pub success_requests: u64,
    pub failed_requests: u64,
}

/// 启动代理服务器（异步，非阻塞）
pub async fn start_proxy_server(config: ProxyConfig) -> Result<(), String> {
    let state = ProxyState {
        config: Arc::new(RwLock::new(config.clone())),
        client: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .map_err(|e| e.to_string())?,
        stats: Arc::new(RwLock::new(ProxyStats::default())),
    };

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/v1/messages", post(messages_handler))
        .route("/v1/messages/count_tokens", post(count_tokens_handler))
        .with_state(state.clone());

    let addr: std::net::SocketAddr = format!("{}:{}", config.listen_address, config.listen_port)
        .parse()
        .map_err(|e: std::net::AddrParseError| e.to_string())?;

    println!("[proxy] 启动代理服务器: {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("绑定代理端口失败: {}", e))?;

    axum::serve(listener, app)
        .await
        .map_err(|e| format!("代理服务器错误: {}", e))?;

    Ok(())
}

/// GET /health
async fn health_handler() -> Json<Value> {
    Json(json!({"status": "ok", "service": "any-version-proxy"}))
}

/// POST /v1/messages/count_tokens — 简单回显，不做实际计算
async fn count_tokens_handler(Json(body): Json<Value>) -> Json<Value> {
    // Claude Code 用此端点估算 token，简单返回一个估算值
    let text_len = body
        .get("messages")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    m.get("content").and_then(|c| match c {
                        Value::String(s) => Some(s.len()),
                        Value::Array(parts) => Some(
                            parts
                                .iter()
                                .filter_map(|p| p.get("text").and_then(|t| t.as_str()).map(|s| s.len()))
                                .sum(),
                        ),
                        _ => None,
                    })
                })
                .sum::<usize>()
        })
        .unwrap_or(0);

    // 粗略估算：1 token ≈ 4 字符
    Json(json!({
        "input_tokens": (text_len / 4).max(1)
    }))
}

/// 构建上游请求（复用于初始请求和重试）
fn build_upstream_request(
    client: &reqwest::Client,
    config: &ProxyConfig,
    headers: &HeaderMap,
    upstream_url: &str,
    auth_header_name: &str,
    body: &Value,
) -> reqwest::RequestBuilder {
    let mut req = client.post(upstream_url);
    if auth_header_name == "x-api-key" {
        req = req
            .header("x-api-key", &config.upstream_api_key)
            .header("anthropic-version", "2023-06-01")
            .json(body);
    } else {
        req = req
            .header("Authorization", format!("Bearer {}", config.upstream_api_key))
            .json(body);
    }
    if let Some(beta) = headers.get("anthropic-beta") {
        if let Ok(val) = beta.to_str() {
            req = req.header("anthropic-beta", val);
        }
    }
    req
}

/// 处理成功的上游响应（Anthropic 直通 / 流式 / 非流式）
async fn process_successful_response(
    upstream_resp: reqwest::Response,
    state: &ProxyState,
    config: &ProxyConfig,
    is_stream: bool,
    request_model: String,
) -> Response {
    // Anthropic 直通模式
    if !config.upstream_anthropic_url.is_empty() {
        let mut stats = state.stats.write().await;
        stats.success_requests += 1;
        let body_bytes = upstream_resp.bytes().await.unwrap_or_default();
        return (
            StatusCode::OK,
            [("content-type", "application/json")],
            body_bytes,
        )
            .into_response();
    }

    // 流式响应
    if is_stream {
        let stream = upstream_resp.bytes_stream();
        let mut converter = transform::StreamConverter::new(request_model.clone());
        let stats = state.stats.clone();

        let sse_stream = async_stream::stream! {
            let mut buffer = String::new();
            use futures_util::StreamExt;

            tokio::pin!(stream);
            while let Some(chunk_result) = stream.next().await {
                let chunk = match chunk_result {
                    Ok(c) => c,
                    Err(e) => {
                        yield Ok::<_, Infallible>(Event::default().data(format!("{{\"error\":\"{}\"}}", e)));
                        break;
                    }
                };

                buffer = sse::append_utf8_safe(&buffer, &chunk);

                while let Some((block, remainder)) = sse::take_sse_block(&buffer) {
                    buffer = remainder.to_string();

                    if let Some(data_str) = sse::extract_sse_data(&block) {
                        if data_str == "[DONE]" {
                            yield Ok::<_, Infallible>(Event::default().data("[DONE]"));
                            continue;
                        }

                        if let Ok(chunk_json) = serde_json::from_str::<Value>(&data_str) {
                            let events = converter.convert_chunk(&chunk_json);
                            for event_str in events {
                                let mut event = Event::default();
                                for line in event_str.lines() {
                                    if let Some(rest) = line.strip_prefix("event:") {
                                        event = event.event(rest.trim());
                                    } else if let Some(rest) = line.strip_prefix("data:") {
                                        event = event.data(rest.trim());
                                    }
                                }
                                yield Ok::<_, Infallible>(event);
                            }
                        }
                    }
                }
            }

            // 流结束后记录 token 用量（最后一个 chunk 的 usage 已被 converter 捕获）
            let (input_tokens, output_tokens) = converter.usage();
            if input_tokens > 0 || output_tokens > 0 {
                record_proxy_usage(converter.model_name(), input_tokens, output_tokens);
            }

            let mut s = stats.write().await;
            s.success_requests += 1;
        };

        return Sse::new(sse_stream).into_response();
    }

    // 非流式响应
    let openai_resp: Value = match upstream_resp.json().await {
        Ok(v) => v,
        Err(e) => {
            let mut stats = state.stats.write().await;
            stats.failed_requests += 1;
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "type": "error",
                    "error": {
                        "type": "api_error",
                        "message": format!("解析上游响应失败: {}", e)
                    }
                })),
            )
                .into_response();
        }
    };

    let anthropic_resp = transform::openai_response_to_anthropic(&openai_resp, &request_model);

    // 自动记录 token 使用量
    let input_tokens = anthropic_resp["usage"]["input_tokens"].as_u64().unwrap_or(0);
    let output_tokens = anthropic_resp["usage"]["output_tokens"].as_u64().unwrap_or(0);
    if input_tokens > 0 || output_tokens > 0 {
        record_proxy_usage(&request_model, input_tokens, output_tokens);
    }

    let mut stats = state.stats.write().await;
    stats.success_requests += 1;

    Json(anthropic_resp).into_response()
}

/// POST /v1/messages — 核心代理处理
async fn messages_handler(
    State(state): State<ProxyState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    {
        let mut stats = state.stats.write().await;
        stats.total_requests += 1;
    }

    let config = state.config.read().await.clone();
    let is_stream = body.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
    let request_model = body
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    // 确定上游 URL 和认证方式
    let (upstream_url, auth_header_name) = if !config.upstream_anthropic_url.is_empty() {
        let url = format!(
            "{}/messages",
            config.upstream_anthropic_url.trim_end_matches('/')
        );
        (url, "x-api-key")
    } else {
        let url = format!(
            "{}/chat/completions",
            config.upstream_base_url.trim_end_matches('/')
        );
        (url, "Authorization")
    };

    // 应用请求优化（在 Anthropic 格式上操作）
    let mut optimized_body = body.clone();
    if config.optimizer_enabled {
        if config.optimizer_cache_injection {
            optimizers::inject_cache_breakpoints(&mut optimized_body);
        }
        if config.optimizer_deepseek {
            optimizers::normalize_deepseek_thinking(&mut optimized_body, &upstream_url);
        }
        if config.optimizer_thinking {
            optimizers::optimize_thinking(&mut optimized_body);
        }
    }

    // 构建模型别名映射
    let aliases = if config.model_aliases.is_empty() {
        None
    } else {
        Some(transform::ModelAliases {
            default_model: config.default_model.clone(),
            role_map: config.model_aliases.clone(),
        })
    };

    // 转换请求体：Anthropic → OpenAI
    let openai_body = transform::anthropic_to_openai(&optimized_body, &config.target_model, aliases.as_ref());

    // Anthropic 直通模式下也需要应用模型别名映射
    let anthropic_body = if !config.model_aliases.is_empty() {
        let mut body = optimized_body.clone();
        if let Some(model) = body.get("model").and_then(|v| v.as_str()) {
            let mapped = transform::map_model_name(model, aliases.as_ref().unwrap());
            body["model"] = Value::String(mapped);
        }
        body
    } else {
        optimized_body.clone()
    };

    // 构建请求
    let send_body: &Value = if !config.upstream_anthropic_url.is_empty() {
        &anthropic_body
    } else {
        &openai_body
    };
    let req = build_upstream_request(
        &state.client,
        &config,
        &headers,
        &upstream_url,
        auth_header_name,
        send_body,
    );

    // 发送请求
    let upstream_resp = match req.send().await {
        Ok(resp) => resp,
        Err(e) => {
            let mut stats = state.stats.write().await;
            stats.failed_requests += 1;
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "type": "error",
                    "error": {
                        "type": "api_error",
                        "message": format!("上游请求失败: {}", e)
                    }
                })),
            )
                .into_response();
        }
    };

    let status = upstream_resp.status();

    if !status.is_success() {
        let error_body = upstream_resp.text().await.unwrap_or_default();

        // 尝试修正 thinking 相关错误并重试
        let retry = if config.rectifier_enabled && config.rectifier_thinking_signature
            && optimizers::is_thinking_signature_error(status.as_u16(), &error_body)
        {
            let mut rectified = optimized_body.clone();
            optimizers::strip_thinking_blocks(&mut rectified);
            Some(rectified)
        } else if config.rectifier_enabled && config.rectifier_thinking_budget
            && optimizers::is_thinking_budget_error(status.as_u16(), &error_body)
        {
            let mut rectified = optimized_body.clone();
            optimizers::fix_thinking_budget(&mut rectified);
            Some(rectified)
        } else if config.rectifier_enabled && config.rectifier_media_fallback
            && optimizers::is_unsupported_image_error(status.as_u16(), &error_body)
        {
            let mut media_body = optimized_body.clone();
            let replaced = optimizers::replace_image_blocks(&mut media_body);
            if replaced > 0 {
                eprintln!("[proxy] media sanitizer: replaced {} image blocks, retrying", replaced);
                Some(media_body)
            } else {
                None
            }
        } else {
            None
        };

        if let Some(rectified) = retry {
            let rectified_openai =
                transform::anthropic_to_openai(&rectified, &config.target_model, aliases.as_ref());
            // Anthropic 直通模式下也需要应用模型别名映射
            let rectified_anthropic = if !config.model_aliases.is_empty() {
                let mut body = rectified.clone();
                if let Some(model) = body.get("model").and_then(|v| v.as_str()) {
                    let mapped = transform::map_model_name(model, aliases.as_ref().unwrap());
                    body["model"] = Value::String(mapped);
                }
                body
            } else {
                rectified.clone()
            };
            let retry_body: &Value = if !config.upstream_anthropic_url.is_empty() {
                &rectified_anthropic
            } else {
                &rectified_openai
            };
            let retry_req = build_upstream_request(
                &state.client,
                &config,
                &headers,
                &upstream_url,
                auth_header_name,
                retry_body,
            );
            if let Ok(retry_resp) = retry_req.send().await {
                if retry_resp.status().is_success() {
                    return process_successful_response(
                        retry_resp,
                        &state,
                        &config,
                        is_stream,
                        request_model,
                    )
                    .await;
                }
            }
        }

        let mut stats = state.stats.write().await;
        stats.failed_requests += 1;
        return (
            StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
            Json(json!({
                "type": "error",
                "error": {
                    "type": "api_error",
                    "message": format!("上游返回错误 ({}): {}", status, error_body)
                }
            })),
        )
            .into_response();
    }

    process_successful_response(upstream_resp, &state, &config, is_stream, request_model).await
}
