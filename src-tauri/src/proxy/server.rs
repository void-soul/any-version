//! 统一代理 HTTP 服务器
//!
//! 每次工具启动一个独立代理实例（监听由 OS 分配的空闲端口），按工具的
//! `api_protocol`（inbound_protocol）锁定入站路由，再根据 Provider 实际拥有的
//! URL 推导出站协议（outbound_protocol），必要时做三协议（Anthropic / OpenAI /
//! Google）互转。处理管线：
//!
//! ① 统计(强制) → ② 模型伪装 C→B → ③ 协议转换 P_in→P_out →
//! ④ 优化器(P_out) → ⑤ 预防式整流(P_out) → ⑥ 转发 →
//! ⑦ 反应式整流(失败重试) → ⑧ 协议转换响应 P_out→P_in →
//! ⑨ 模型伪装回填 C → ⑩ 统计落库(强制)

use super::{google, optimizers, sse, transform, types::ProxyConfig};
use axum::{
    body::Body,
    extract::{OriginalUri, Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{sse::Event, IntoResponse, Response, Sse},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

/// 记录使用量到 SQLite 数据库（线程安全）。携带真实 tool_id / provider_id。
fn record_proxy_usage(tool_id: &str, provider_id: &str, model: &str, input_tokens: u64, output_tokens: u64) {
    if let Err(e) = crate::commands::ai::usage::log_usage_db(tool_id, model, Some(provider_id), input_tokens, output_tokens) {
        eprintln!("[proxy] 记录用量失败: {}", e);
    }
}

/// 打印代理网络请求日志（统一前缀，不打印任何敏感头/密钥）
fn log_proxy(msg: &str) {
    let ts = chrono::Local::now().format("%H:%M:%S%.3f");
    println!("[proxy] {} {}", ts, msg);
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

/// 绑定一个由 OS 分配的空闲端口（地址 `listen_address`），返回 (端口号, std 监听器)。
/// 调用方必须持有监听器直到 `serve_proxy` 接管，避免端口被其它进程抢占。
pub fn bind_free_port(listen_address: &str) -> Result<(u16, std::net::TcpListener), String> {
    let addr = format!("{}:0", listen_address);
    let listener = std::net::TcpListener::bind(&addr).map_err(|e| format!("绑定空闲端口失败: {}", e))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    Ok((port, listener))
}

/// 在已绑定的监听器上启动代理服务（内部使用，不入站 Async 绑定）。
pub async fn serve_proxy(config: ProxyConfig, listener: std::net::TcpListener) -> Result<(), String> {
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let tokio_listener = tokio::net::TcpListener::from_std(listener).map_err(|e| e.to_string())?;

    let state = ProxyState {
        config: Arc::new(RwLock::new(config.clone())),
        client: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .map_err(|e| e.to_string())?,
        stats: Arc::new(RwLock::new(ProxyStats::default())),
    };

    let mut app = Router::new().route("/health", get(health_handler));
    for inbound in &config.inbound_protocols {
        match inbound.as_str() {
            "anthropic" => {
                app = app
                    .route("/v1/messages", post(messages_handler))
                    .route("/v1/messages/count_tokens", post(count_tokens_handler));
            }
            "openai" => {
                app = app
                    .route("/v1/chat/completions", post(chat_completions_handler))
                    .route("/v1/models", get(models_handler));
            }
            "google" => {
                app = app
                    .route("/v1beta/models/{model}:generateContent", post(google_handler))
                    .route("/v1beta/models/{model}:streamGenerateContent", post(google_handler));
            }
            other => return Err(format!("未知入站协议: {}", other)),
        }
    }
    let app = app.with_state(state.clone());

    axum::serve(tokio_listener, app)
        .await
        .map_err(|e| format!("代理服务器错误: {}", e))?;
    Ok(())
}

/// 启动代理服务器（按 config.listen_port 绑定，用于手动/独立启动场景）。
pub async fn start_proxy_server(config: ProxyConfig) -> Result<(), String> {
    let addr = format!("{}:{}", config.listen_address, config.listen_port);
    let listener = std::net::TcpListener::bind(&addr).map_err(|e| format!("绑定代理端口 {} 失败: {}", addr, e))?;
    log_proxy(&format!("启动代理服务器: {}", addr));
    serve_proxy(config, listener).await
}

/// GET /health
async fn health_handler() -> Json<Value> {
    Json(json!({"status": "ok", "service": "any-version-proxy"}))
}

/// GET /v1/models — OpenAI 入站时透传到上游 OpenAI 的 /models（用于工具列举可用模型）
async fn models_handler(State(state): State<ProxyState>, headers: HeaderMap) -> Response {
    let config = state.config.read().await.clone();
    let url = format!("{}/models", config.upstream_base_url.trim_end_matches('/'));
    log_proxy(&format!("← IN   /v1/models  → OUT GET {}", url));

    let mut req = state
        .client
        .get(&url)
        .header("Authorization", format!("Bearer {}", config.upstream_api_key));
    if let Some(ct) = headers.get(header::CONTENT_TYPE) {
        if let Ok(v) = ct.to_str() {
            req = req.header(header::CONTENT_TYPE, v);
        }
    }
    if let Some(ua) = headers.get(header::USER_AGENT) {
        if let Ok(v) = ua.to_str() {
            req = req.header(header::USER_AGENT, v);
        }
    }

    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            let ct = resp.headers().get(header::CONTENT_TYPE).cloned();
            let bytes = match resp.bytes().await {
                Ok(b) => b,
                Err(e) => {
                    return (StatusCode::BAD_GATEWAY, Json(json!({"error": {"message": format!("读取上游响应失败: {}", e)}})))
                        .into_response();
                }
            };
            let mut out = Response::new(Body::from(bytes));
            *out.status_mut() = status;
            if let Some(c) = ct {
                out.headers_mut().insert(header::CONTENT_TYPE, c);
            }
            out
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": {"message": format!("上游请求失败: {}", e)}})),
        )
            .into_response(),
    }
}

/// POST /v1/messages/count_tokens — Anthropic 入站时返回估算值
async fn count_tokens_handler(State(state): State<ProxyState>, Json(body): Json<Value>) -> Json<Value> {
    let _ = state;
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
    Json(json!({ "input_tokens": (text_len / 4).max(1) }))
}

/// POST /v1/messages — Anthropic 入站
async fn messages_handler(State(state): State<ProxyState>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    let claimed = body
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    process_request(&state, &headers, "anthropic", claimed, false, body).await
}

/// POST /v1/chat/completions — OpenAI 入站
async fn chat_completions_handler(State(state): State<ProxyState>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    let claimed = body
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    process_request(&state, &headers, "openai", claimed, false, body).await
}

/// POST /v1beta/models/{model}:generateContent | :streamGenerateContent — Google 入站
/// model 来自 URL 路径（即声明名 C）。流式由 URL 中是否含 `streamGenerateContent` 判定。
async fn google_handler(
    State(state): State<ProxyState>,
    OriginalUri(uri): OriginalUri,
    Path(model): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let is_stream = uri.path().contains("streamGenerateContent");
    process_request(&state, &headers, "google", model, is_stream, body).await
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  统一处理管线
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 请求处理统一入口。
/// - `inbound`：工具说话的协议（"anthropic" | "openai" | "google"）
/// - `claimed_model`：声明名 C（anthropic/openai 来自 body.model；google 来自 URL path）
/// - `google_is_stream`：仅 google 入站有意义
/// - `body`：P_in 形态请求体
async fn process_request(
    state: &ProxyState,
    headers: &HeaderMap,
    inbound: &str,
    claimed_model: String,
    google_is_stream: bool,
    mut body: Value,
) -> Response {
    {
        let mut stats = state.stats.write().await;
        stats.total_requests += 1;
    }
    let start = Instant::now();

    let config = state.config.read().await.clone();
    let outbound = config.outbound_protocol.clone();
    let aliases = build_aliases(&config);

    log_proxy(&format!(
        "← IN   [{}] model={}  → OUT [{}]  (转换: {})",
        inbound, claimed_model, outbound, config.conversion_mode
    ));

    // ② 模型伪装：C → B（替换 body.model，google 入站不写 body）
    let actual_model = apply_masquerade(inbound, &mut body, &claimed_model, aliases.as_ref());

    // ③ 协议转换：P_in → P_out（已在 body 写入 B）
    let mut out_body = convert_request(inbound, &outbound, &body, &actual_model);

    // ④ 优化器（P_out 形态）
    optimizers::apply_optimizers(&mut out_body, &outbound, &config);

    // ⑤ 预防式整流（P_out 形态）
    optimizers::apply_preventive_rectifiers(&mut out_body, &outbound, &config);

    // ⑥ 转发
    let is_stream = if inbound == "google" {
        google_is_stream
    } else {
        body.get("stream").and_then(|v| v.as_bool()).unwrap_or(false)
    };
    // 出站为 OpenAI / Anthropic 时，确保请求体带 stream 字段：
    // Google 入站（流式靠 URL 判定）以及跨协议转换后的 body 可能未写入 `stream`，
    // 而 OpenAI / Anthropic 上游依赖 body 中的 stream 字段决定是否流式返回。
    if is_stream && (outbound == "openai" || outbound == "anthropic") {
        if let Some(o) = out_body.as_object_mut() {
            o.insert("stream".into(), json!(true));
        }
    }
    let (upstream_url, auth_name) = build_upstream_url(&config, &outbound, &actual_model, is_stream);
    if upstream_url.is_empty() {
        let mut stats = state.stats.write().await;
        stats.failed_requests += 1;
        return (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": {"message": "未配置对应出站协议的上游 URL"}})),
        )
            .into_response();
    }

    log_proxy(&format!("→ OUT  POST {} auth={}", upstream_url, auth_name));

    let req = build_upstream_request(&state.client, &config, headers, &upstream_url, &auth_name, &out_body);
    let upstream_resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            let mut stats = state.stats.write().await;
            stats.failed_requests += 1;
            log_proxy(&format!("✗ OUT  POST {} 请求失败: {}  ({}ms)", upstream_url, e, start.elapsed().as_millis()));
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": {"message": format!("上游请求失败: {}", e)}})),
            )
                .into_response();
        }
    };

    let status = upstream_resp.status();
    log_proxy(&format!("← UPSTREAM {}  ({}ms)", status.as_u16(), start.elapsed().as_millis()));

    if !status.is_success() {
        let error_body = upstream_resp.text().await.unwrap_or_default();
        log_proxy(&format!(
            "✗ UPSTREAM {} error: {}",
            status.as_u16(),
            error_body.chars().take(300).collect::<String>()
        ));

        // ⑦ 反应式整流：修正后重试一次
        if let Some(rectified) = optimizers::try_reactive_rectify(status.as_u16(), &error_body, &out_body, &config, &outbound) {
            let retry_req = build_upstream_request(&state.client, &config, headers, &upstream_url, &auth_name, &rectified);
            if let Ok(retry_resp) = retry_req.send().await {
                if retry_resp.status().is_success() {
                    log_proxy(&format!("↻ retry succeeded after rectify  ({}ms)", start.elapsed().as_millis()));
                    return process_response(state, retry_resp, &config, inbound, &outbound, &claimed_model, &actual_model, is_stream).await;
                }
            }
        }

        let mut stats = state.stats.write().await;
        stats.failed_requests += 1;
        return (
            StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
            Json(json!({
                "error": { "message": format!("上游返回错误 ({}): {}", status, error_body) }
            })),
        )
            .into_response();
    }

    process_response(state, upstream_resp, &config, inbound, &outbound, &claimed_model, &actual_model, is_stream).await
}

/// 处理成功的上游响应（流式 / 非流式）
/// - `claimed_model`：声明名 C，用于响应转换回填（⑧⑨）
/// - `actual_model`：实际模型 B，用于用量统计落库（⑩）
async fn process_response(
    state: &ProxyState,
    upstream_resp: reqwest::Response,
    config: &ProxyConfig,
    inbound: &str,
    outbound: &str,
    claimed_model: &str,
    actual_model: &str,
    is_stream: bool,
) -> Response {
    if is_stream {
        return stream_response(state, upstream_resp, config, inbound, outbound, claimed_model, actual_model).await;
    }

    let content_type = upstream_resp.headers().get(header::CONTENT_TYPE).cloned();
    let resp_json: Value = match upstream_resp.json().await {
        Ok(v) => v,
        Err(e) => {
            let mut stats = state.stats.write().await;
            stats.failed_requests += 1;
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": {"message": format!("解析上游响应失败: {}", e)}})),
            )
                .into_response();
        }
    };

    // ⑧ 协议转换响应：P_out → P_in
    let mut in_resp = convert_response(outbound, inbound, &resp_json, claimed_model);

    // ⑨ 模型伪装回填：响应 model 字段写回声明名 C
    set_response_model(&mut in_resp, claimed_model, inbound);

    // ⑩ 统计落库（记录实际模型 B，而非声明名 C）
    record_usage_from_response(state, config, inbound, &in_resp, actual_model);

    let mut stats = state.stats.write().await;
    stats.success_requests += 1;

    // 同协议透传时保持原始 content-type；转换后的 JSON 用 application/json
    if outbound == inbound {
        let mut out = Response::new(Body::from(serde_json::to_vec(&in_resp).unwrap_or_default()));
        *out.status_mut() = StatusCode::OK;
        if let Some(ct) = content_type {
            out.headers_mut().insert(header::CONTENT_TYPE, ct);
        }
        out
    } else {
        Json(in_resp).into_response()
    }
}

/// 流式响应：把上游（出站协议）SSE 转换为入站协议 SSE。
/// - `claimed_model`：声明名 C（用于转换器回填）
/// - `actual_model`：实际模型 B（用于流式用量统计落库）
async fn stream_response(
    state: &ProxyState,
    upstream_resp: reqwest::Response,
    config: &ProxyConfig,
    inbound: &str,
    outbound: &str,
    claimed_model: &str,
    actual_model: &str,
) -> Response {
    let tool_id = config.tool_id.clone();
    let provider_id = config.provider_id.clone();
    let claimed = claimed_model.to_string();
    let actual = actual_model.to_string();

    // 同协议透传：原样转发字节，同时抽取 SSE 中的 usage 落库
    if inbound == outbound {
        let stats = state.stats.clone();
        let ct = upstream_resp.headers().get(header::CONTENT_TYPE).cloned();
        let tid = tool_id.clone();
        let pid = provider_id.clone();
        let act = actual.clone();
        let proto = inbound.to_string();
        let stream = upstream_resp.bytes_stream();
        let sse = async_stream::stream! {
            use futures_util::StreamExt;
            tokio::pin!(stream);
            let mut buffer = String::new();
            let mut acc_in: u64 = 0;
            let mut acc_out: u64 = 0;
            while let Some(r) = stream.next().await {
                let chunk = match r {
                    Ok(c) => c,
                    Err(e) => {
                        yield Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()));
                        break;
                    }
                };
                // 边转发边解析 usage（不改动原始字节）
                buffer = sse::append_utf8_safe(&buffer, &chunk);
                while let Some((block, remainder)) = sse::take_sse_block(&buffer) {
                    buffer = remainder.to_string();
                    if let Some(data_str) = sse::extract_sse_data(&block) {
                        if data_str != "[DONE]" {
                            if let Ok(cj) = serde_json::from_str::<Value>(&data_str) {
                                let (i, o) = extract_stream_usage(&proto, &cj);
                                if i > 0 { acc_in = i; }
                                if o > 0 { acc_out = o; }
                            }
                        }
                    }
                }
                yield Ok::<_, std::io::Error>(chunk);
            }
            if acc_in > 0 || acc_out > 0 {
                record_proxy_usage(&tid, &pid, &act, acc_in, acc_out);
            }
            let mut s = stats.write().await;
            s.success_requests += 1;
        };
        let mut out = Response::new(Body::from_stream(sse));
        *out.status_mut() = StatusCode::OK;
        if let Some(c) = ct {
            out.headers_mut().insert(header::CONTENT_TYPE, c);
        }
        return out;
    }

    // 不同协议：按 (inbound, outbound) 选择转换器
    macro_rules! run_stream {
        ($conv:expr) => {{
            let mut conv = $conv;
            let stats = state.stats.clone();
            let (tid, pid, cm) = (tool_id.clone(), provider_id.clone(), actual.clone());
            let stream = upstream_resp.bytes_stream();
            let sse = async_stream::stream! {
                use futures_util::StreamExt;
                let mut buffer = String::new();
                tokio::pin!(stream);
                while let Some(r) = stream.next().await {
                    let chunk = match r {
                        Ok(c) => c,
                        Err(e) => {
                            yield Ok::<_, Infallible>(Event::default().data(format!("{{\"error\":\"{}\"}}", e)));
                            break;
                        }
                    };
                    buffer = sse::append_utf8_safe(&buffer, &chunk);
                    while let Some((block, remainder)) = sse::take_sse_block(&buffer) {
                        buffer = remainder.to_string();
                        let data_str = match sse::extract_sse_data(&block) {
                            Some(d) => d,
                            None => continue,
                        };
                        if data_str == "[DONE]" {
                            yield Ok::<_, Infallible>(Event::default().data("[DONE]"));
                            continue;
                        }
                        if let Ok(cj) = serde_json::from_str::<Value>(&data_str) {
                            for ev in conv.convert_chunk(&cj) {
                                let mut event = Event::default();
                                for line in ev.lines() {
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
                let (in_t, out_t) = conv.usage();
                if in_t > 0 || out_t > 0 {
                    record_proxy_usage(&tid, &pid, &cm, in_t, out_t);
                }
                let mut s = stats.write().await;
                s.success_requests += 1;
            };
            return Sse::new(sse).into_response();
        }};
    }

    match (inbound, outbound) {
        ("anthropic", "openai") => {
            run_stream!(transform::StreamConverter::new(claimed.clone()));
        }
        ("openai", "anthropic") => {
            run_stream!(transform::AnthropicToOpenaiStreamConverter::new(claimed.clone()));
        }
        ("google", "openai") => {
            run_stream!(google::GoogleStreamConverter::new("openai", &claimed));
        }
        ("google", "anthropic") => {
            run_stream!(google::GoogleStreamConverter::new("anthropic", &claimed));
        }
        ("anthropic", "google") => {
            run_stream!(google::GoogleToAnthropicStreamConverter::new(&claimed));
        }
        ("openai", "google") => {
            run_stream!(google::GoogleToOpenaiStreamConverter::new(&claimed));
        }
        _ => {
            let mut stats = state.stats.write().await;
            stats.failed_requests += 1;
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": {"message": format!("不支持的流式转换组合: {} -> {}", inbound, outbound)}})),
            )
                .into_response()
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  转换与构建辅助
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn build_aliases(config: &ProxyConfig) -> Option<transform::ModelAliases> {
    if config.model_aliases.is_empty() {
        None
    } else {
        Some(transform::ModelAliases {
            default_model: config.default_model.clone(),
            role_map: config.model_aliases.clone(),
        })
    }
}

/// ② 模型伪装：声明名 C → 实际模型 B。anthropic/openai 入站写回 body.model。
/// 匹配时忽略 [1m]/[1M] 上下文后缀，使「声明名」与代理持有的 masquerade key 一致。
fn apply_masquerade(inbound: &str, body: &mut Value, claimed: &str, aliases: Option<&transform::ModelAliases>) -> String {
    let normalize = |s: &str| s.replace("[1m]", "").replace("[1M]", "").trim().to_string();
    let actual = match aliases {
        Some(a) => {
            // 先按原始名映射，再按去后缀名映射（兼容 1M 上下文后缀）
            let mapped = transform::map_model_name(claimed, a);
            if mapped != claimed {
                mapped
            } else {
                let n = normalize(claimed);
                if n != claimed { transform::map_model_name(&n, a) } else { claimed.to_string() }
            }
        }
        None => claimed.to_string(),
    };
    if inbound != "google" {
        if let Some(o) = body.as_object_mut() {
            o.insert("model".into(), json!(actual.clone()));
        }
    }
    actual
}

/// ③ 协议转换请求：P_in → P_out。同协议直接克隆。
fn convert_request(inbound: &str, outbound: &str, body: &Value, model: &str) -> Value {
    match (inbound, outbound) {
        (a, b) if a == b => body.clone(),
        ("anthropic", "openai") => transform::anthropic_to_openai(body, model, None),
        ("openai", "anthropic") => transform::openai_to_anthropic(body, model, None),
        ("anthropic", "google") => google::anthropic_to_google(body, model),
        ("openai", "google") => google::openai_to_google(body, model),
        ("google", "anthropic") => google::google_to_anthropic(body, model),
        ("google", "openai") => google::google_to_openai(body, model),
        _ => body.clone(),
    }
}

/// ⑧ 协议转换响应：P_out → P_in。
fn convert_response(outbound: &str, inbound: &str, resp: &Value, claimed: &str) -> Value {
    match (outbound, inbound) {
        (a, b) if a == b => resp.clone(),
        ("openai", "anthropic") => transform::openai_response_to_anthropic(resp, claimed),
        ("anthropic", "openai") => transform::anthropic_response_to_openai(resp, claimed),
        ("google", "anthropic") => google::google_response_to_anthropic(resp, claimed),
        ("google", "openai") => google::google_response_to_openai(resp, claimed),
        ("anthropic", "google") => google::anthropic_response_to_google(resp, claimed),
        ("openai", "google") => google::openai_response_to_google(resp, claimed),
        _ => resp.clone(),
    }
}

/// ⑨ 响应 model 字段回填为声明名 C。
fn set_response_model(resp: &mut Value, claimed: &str, inbound: &str) {
    match inbound {
        "anthropic" => {
            if let Some(o) = resp.as_object_mut() {
                o.insert("model".into(), json!(claimed));
            }
        }
        "openai" => {
            if let Some(choices) = resp.get_mut("choices").and_then(|v| v.as_array_mut()) {
                if let Some(first) = choices.first_mut() {
                    if let Some(msg) = first.get_mut("message") {
                        if let Some(o) = msg.as_object_mut() {
                            o.insert("model".into(), json!(claimed));
                        }
                    }
                }
            }
            if let Some(o) = resp.as_object_mut() {
                o.insert("model".into(), json!(claimed));
            }
        }
        "google" => {
            if let Some(o) = resp.as_object_mut() {
                o.insert("model".into(), json!(claimed));
            }
        }
        _ => {}
    }
}

/// ⑩ 从非流式响应解析 token 用量并落库。
/// `model` 为实际模型 B（由调用方传入，而非声明名 C）。
fn record_usage_from_response(
    state: &ProxyState,
    config: &ProxyConfig,
    inbound: &str,
    resp: &Value,
    model: &str,
) {
    let (in_t, out_t) = match inbound {
        "anthropic" => (
            resp.get("usage").and_then(|u| u.get("input_tokens")).and_then(|v| v.as_u64()).unwrap_or(0),
            resp.get("usage").and_then(|u| u.get("output_tokens")).and_then(|v| v.as_u64()).unwrap_or(0),
        ),
        "openai" => (
            resp.get("usage").and_then(|u| u.get("prompt_tokens")).and_then(|v| v.as_u64()).unwrap_or(0),
            resp.get("usage").and_then(|u| u.get("completion_tokens")).and_then(|v| v.as_u64()).unwrap_or(0),
        ),
        "google" => {
            let um = resp.get("usageMetadata");
            let i = um.and_then(|u| u.get("promptTokenCount")).and_then(|v| v.as_u64()).unwrap_or(0);
            let cand = um.and_then(|u| u.get("candidatesTokenCount")).and_then(|v| v.as_u64()).unwrap_or(0);
            let thought = um.and_then(|u| u.get("thoughtsTokenCount")).and_then(|v| v.as_u64()).unwrap_or(0);
            (i, cand + thought)
        }
        _ => (0, 0),
    };
    if in_t > 0 || out_t > 0 {
        record_proxy_usage(&config.tool_id, &config.provider_id, model, in_t, out_t);
    }
    let _ = state;
}

/// 从单个 SSE `data:` 块（已解析为 JSON）抽取 token 用量（按入站协议形态）。
/// 用于同协议流式透传时边流边累加，流结束后落库。
/// 返回 (input_tokens, output_tokens)；未包含用量时返回 (0, 0)。
fn extract_stream_usage(inbound: &str, cj: &Value) -> (u64, u64) {
    match inbound {
        "anthropic" => {
            // message_start: message.usage.input_tokens；message_delta: usage.output_tokens
            match cj.get("type").and_then(|v| v.as_str()) {
                Some("message_start") => (
                    cj.get("message")
                        .and_then(|m| m.get("usage"))
                        .and_then(|u| u.get("input_tokens"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0),
                    0,
                ),
                Some("message_delta") => (
                    0,
                    cj.get("usage")
                        .and_then(|u| u.get("output_tokens"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0),
                ),
                _ => (0, 0),
            }
        }
        "openai" => (
            cj.get("usage")
                .and_then(|u| u.get("prompt_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
            cj.get("usage")
                .and_then(|u| u.get("completion_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
        ),
        "google" => {
            let um = cj.get("usageMetadata");
            let i = um.and_then(|u| u.get("promptTokenCount")).and_then(|v| v.as_u64()).unwrap_or(0);
            let cand = um.and_then(|u| u.get("candidatesTokenCount")).and_then(|v| v.as_u64()).unwrap_or(0);
            let thought = um.and_then(|u| u.get("thoughtsTokenCount")).and_then(|v| v.as_u64()).unwrap_or(0);
            (i, cand + thought)
        }
        _ => (0, 0),
    }
}

/// ⑥ 构建上游 URL 与鉴权头名称。
fn build_upstream_url(config: &ProxyConfig, outbound: &str, model: &str, is_stream: bool) -> (String, String) {
    let base = config.upstream_base_url.trim_end_matches('/');
    match outbound {
        "anthropic" => {
            let url = if base.ends_with("/messages") {
                base.to_string()
            } else {
                format!("{}/messages", base)
            };
            (url, "x-api-key".to_string())
        }
        "openai" => (
            format!("{}/chat/completions", base),
            "Authorization".to_string(),
        ),
        "google" => {
            let url = if is_stream {
                format!("{}/v1beta/models/{}:streamGenerateContent?alt=sse", base, model)
            } else {
                format!("{}/v1beta/models/{}:generateContent", base, model)
            };
            (url, "x-goog-api-key".to_string())
        }
        _ => (String::new(), "Authorization".to_string()),
    }
}

/// 构建上游请求（携带对应鉴权头）。
fn build_upstream_request(
    client: &reqwest::Client,
    config: &ProxyConfig,
    headers: &HeaderMap,
    upstream_url: &str,
    auth_name: &str,
    body: &Value,
) -> reqwest::RequestBuilder {
    let mut req = client.post(upstream_url);
    match auth_name {
        "x-api-key" => {
            req = req
                .header("x-api-key", &config.upstream_api_key)
                .header("anthropic-version", "2023-06-01")
                .json(body);
        }
        "x-goog-api-key" => {
            req = req.header("x-goog-api-key", &config.upstream_api_key).json(body);
        }
        _ => {
            req = req
                .header("Authorization", format!("Bearer {}", config.upstream_api_key))
                .json(body);
        }
    }
    if let Some(beta) = headers.get("anthropic-beta") {
        if let Ok(val) = beta.to_str() {
            req = req.header("anthropic-beta", val);
        }
    }
    req
}
