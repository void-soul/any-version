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
    extract::{OriginalUri, Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware,
    response::{sse::Event, IntoResponse, Response, Sse},
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::convert::Infallible;
use std::sync::{Arc, OnceLock};
use std::collections::HashMap;
use std::time::Instant;
use tauri::Emitter;
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

/// 是否启用代理「完整日志」。
/// 设置环境变量 ANY_VERSION_PROXY_DEBUG=1 开启后，会把入站/出站的完整请求体、
/// 上游完整响应体、流式完整文本打印到控制台，便于排查配置与模型生效问题。
/// 不设置时仅打印精简预览，避免正常使用时日志刷屏。
fn proxy_debug_enabled() -> bool {
    std::env::var("ANY_VERSION_PROXY_DEBUG")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false)
}

/// 打印完整 JSON 体（仅在 debug 开启时由调用方判断后使用）。
/// 超大响应体截断到 200KB 并注明总长，避免一次性刷屏卡死终端。
fn log_proxy_json_full(msg: &str, value: &Value) {
    match serde_json::to_string(value) {
        Ok(s) => log_proxy_raw(msg, &s),
        Err(e) => log_proxy(&format!("{}: <序列化失败: {}>", msg, e)),
    }
}

/// 打印完整原始字符串（同上，用于流式完整文本等）。
fn log_proxy_raw(msg: &str, s: &str) {
    if s.len() > 200_000 {
        log_proxy(&format!(
            "{}: {}...(truncated, total {} bytes)",
            msg,
            &s[..200_000],
            s.len()
        ));
    } else {
        log_proxy(&format!("{}: {}", msg, s));
    }
}

/// 全局请求日志中间件：记录每一个进入代理的请求（方法 + 路径）与最终响应状态。
/// 即使路径未匹配（axum 返回 404）也会被记录，便于排查「not found」等静默失败。
async fn log_requests_mw(req: Request, next: axum::middleware::Next) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    log_proxy(&format!("→ 收到请求: {} {}", method, path));
    let resp = next.run(req).await;
    log_proxy(&format!("← 响应: {} {}", resp.status().as_u16(), path));
    resp
}

/// 未匹配路由的兜底处理：明确打印 404 路径（而非 axum 默认静默 404）。
async fn catch_all_handler(OriginalUri(uri): OriginalUri) -> impl IntoResponse {
    log_proxy(&format!("✗ 404 未匹配路由: {}", uri.path()));
    (StatusCode::NOT_FOUND, "not found")
}

/// 本地代理鉴权中间件：配置了 auth_token 时，`/v1*`、`/messages`、`/chat/completions`
/// 等数据路由必须携带正确 token（Authorization: Bearer / x-api-key / x-goog-api-key 三者
/// 任一匹配即可，覆盖 Anthropic/OpenAI/Google 三种 SDK 的凭据注入习惯）。
/// `/health` 与 `/collab/*`（有独立 X-Collab-Token 校验）免鉴权。
async fn require_proxy_token_mw(
    State(state): State<ProxyState>,
    req: Request,
    next: middleware::Next,
) -> Response {
    let path = req.uri().path().to_string();
    let expected = state.config.read().await.auth_token.clone();
    if expected.is_empty() || path == "/health" || path.starts_with("/collab") {
        return next.run(req).await;
    }
    let headers = req.headers();
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .map(|v| v.strip_prefix("Bearer ").unwrap_or(v))
        .unwrap_or("");
    let x_api_key = headers.get("x-api-key").and_then(|h| h.to_str().ok()).unwrap_or("");
    let x_goog = headers.get("x-goog-api-key").and_then(|h| h.to_str().ok()).unwrap_or("");
    if bearer == expected || x_api_key == expected || x_goog == expected {
        next.run(req).await
    } else {
        log_proxy(&format!("✗ 鉴权失败: 缺少有效 token: {}", path));
        (StatusCode::UNAUTHORIZED, "unauthorized").into_response()
    }
}

/// 向前端 emit 协同代理事件（仅在协同上下文存在时发送）
fn emit_proxy_event(state: &ProxyState, event: &str, payload: Value) {
    if let (Some(app), Some(room_id)) = (&state.app_handle, &state.collab_room_id) {
        // 从全局表获取当前 msg_id（支持代理复用，动态更新）
        if let Some(msg_id) = get_collab_msg_id(room_id, &state.tool_id) {
            let mut full_payload = payload;
            if let Some(obj) = full_payload.as_object_mut() {
                obj.insert("room_id".into(), json!(room_id));
                obj.insert("msg_id".into(), json!(msg_id));
            }
            let _ = app.emit(event, full_payload);
        }
    }
}

// ─── 协同代理响应文本缓存（供 collab dispatch_to_tool 回退使用）───
/// msg_id → 代理提取的完整响应文本
static PROXY_TEXT: OnceLock<std::sync::Mutex<HashMap<String, String>>> = OnceLock::new();

// ─── 协同上下文：room_id::tool_id → 当前 msg_id（支持代理复用时动态更新）───
static COLLAB_MSG_ID: OnceLock<std::sync::Mutex<HashMap<String, String>>> = OnceLock::new();

/// collab dispatch_to_tool 开始时设置当前 msg_id
pub fn set_collab_msg_id(room_id: &str, tool_id: &str, msg_id: String) {
    let key = format!("{}::{}", room_id, tool_id);
    let map = COLLAB_MSG_ID.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    map.lock().unwrap().insert(key, msg_id);
}

/// collab dispatch_to_tool 结束时清除
pub fn clear_collab_msg_id(room_id: &str, tool_id: &str) {
    let key = format!("{}::{}", room_id, tool_id);
    if let Some(map) = COLLAB_MSG_ID.get() {
        map.lock().unwrap().remove(&key);
    }
}

/// 代理层运行时查询当前 msg_id（优先于 ProxyState 中的静态 msg_id）
fn get_collab_msg_id(room_id: &str, tool_id: &str) -> Option<String> {
    let key = format!("{}::{}", room_id, tool_id);
    COLLAB_MSG_ID.get()?.lock().unwrap().get(&key).cloned()
}

/// 代理层在响应完成时存储文本（使用当前 msg_id）
fn store_proxy_text_for(state: &ProxyState, text: &str) {
    if let Some(room_id) = &state.collab_room_id {
        store_proxy_text_with(room_id, &state.tool_id, text);
    }
}

/// 流式路径中使用（不需要 state 引用）
fn store_proxy_text_with(room_id: &str, tool_id: &str, text: &str) {
    if let Some(msg_id) = get_collab_msg_id(room_id, tool_id) {
        let map_lock = PROXY_TEXT.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
        let mut map = map_lock.lock().unwrap();
        // 限制最大缓存容量 (最多保留 100 条未提取响应，超出自动淘汰最旧项，防内存泄漏)
        if map.len() >= 100 {
            if let Some(first_key) = map.keys().next().cloned() {
                map.remove(&first_key);
            }
        }
        map.insert(msg_id.clone(), text.to_string());
        eprintln!("[proxy] ✓ 代理响应已缓存: msg={}, text_len={}", msg_id, text.len());
    }
}

/// collab dispatch_to_tool 回退取用代理文本（取后删除）
pub fn take_proxy_text(msg_id: &str) -> Option<String> {
    PROXY_TEXT.get()?.lock().unwrap().remove(msg_id)
}

/// 从入站协议形态的 SSE chunk 中提取文本增量
fn extract_inbound_delta_text(inbound: &str, cj: &Value) -> Option<String> {
    match inbound {
        "openai" => {
            // OpenAI: choices[0].delta.content
            cj.get("choices")
                .and_then(|c| c.as_array())
                .and_then(|a| a.first())
                .and_then(|c| c.get("delta"))
                .and_then(|d| d.get("content"))
                .and_then(|t| t.as_str())
                .map(|s| s.to_string())
        }
        "anthropic" => {
            // Anthropic: content_block_delta.delta.text
            match cj.get("type").and_then(|t| t.as_str()) {
                Some("content_block_delta") => {
                    cj.get("delta")
                        .and_then(|d| d.get("text"))
                        .and_then(|t| t.as_str())
                        .map(|s| s.to_string())
                }
                _ => None,
            }
        }
        "google" => {
            // Google: candidates[0].content.parts[*].text
            cj.get("candidates")
                .and_then(|c| c.as_array())
                .and_then(|a| a.first())
                .and_then(|c| c.get("content"))
                .and_then(|c| c.get("parts"))
                .and_then(|p| p.as_array())
                .and_then(|parts| {
                    let texts: Vec<String> = parts.iter()
                        .filter_map(|p| p.get("text").and_then(|t| t.as_str()).map(|s| s.to_string()))
                        .collect();
                    if texts.is_empty() { None } else { Some(texts.join("")) }
                })
        }
        _ => None,
    }
}

/// 从入站协议形态的非流式响应中提取完整文本
fn extract_inbound_full_text(inbound: &str, resp: &Value) -> String {
    match inbound {
        "openai" => {
            resp.get("choices")
                .and_then(|c| c.as_array())
                .and_then(|a| a.first())
                .and_then(|c| c.get("message"))
                .and_then(|m| m.get("content"))
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string()
        }
        "anthropic" => {
            resp.get("content")
                .and_then(|c| c.as_array())
                .map(|parts| {
                    parts.iter()
                        .filter_map(|p| {
                            if p.get("type").and_then(|t| t.as_str()) == Some("text") {
                                p.get("text").and_then(|t| t.as_str()).map(|s| s.to_string())
                            } else { None }
                        })
                        .collect::<Vec<_>>().join("")
                })
                .unwrap_or_default()
        }
        "google" => {
            resp.get("candidates")
                .and_then(|c| c.as_array())
                .and_then(|a| a.first())
                .and_then(|c| c.get("content"))
                .and_then(|c| c.get("parts"))
                .and_then(|p| p.as_array())
                .map(|parts| {
                    parts.iter()
                        .filter_map(|p| p.get("text").and_then(|t| t.as_str()).map(|s| s.to_string()))
                        .collect::<Vec<_>>().join("")
                })
                .unwrap_or_default()
        }
        _ => String::new(),
    }
}

/// 代理服务器共享状态
#[derive(Clone)]
pub struct ProxyState {
    pub config: Arc<RwLock<ProxyConfig>>,
    pub client: reqwest::Client,
    pub stats: Arc<RwLock<ProxyStats>>,
    /// 协同上下文（可选）：设置后代理会向前端 emit 事件
    pub app_handle: Option<tauri::AppHandle>,
    pub collab_room_id: Option<String>,
    pub tool_id: String,
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
        app_handle: config.app_handle.clone(),
        collab_room_id: config.collab_room_id.clone(),
        tool_id: config.tool_id.clone(),
    };

    let mut app = Router::new().route("/health", get(health_handler));
    // 注册路由时同时挂载「带 /v1 前缀」与「无 /v1 前缀」两种路径。
    // 原因：工具（如 opencode）设置的 baseUrl 可能不含 /v1，其 OpenAI 兼容客户端
    // 自己补的是 /chat/completions 而非 /v1/chat/completions，导致 404。
    // 两种前缀都注册，无论工具如何拼接路径都能命中。
    for inbound in &config.inbound_protocols {
        match inbound.as_str() {
            "anthropic" => {
                app = app
                    .route("/v1/messages", post(messages_handler))
                    .route("/messages", post(messages_handler))
                    .route("/v1/messages/count_tokens", post(count_tokens_handler))
                    .route("/messages/count_tokens", post(count_tokens_handler));
            }
            "openai" => {
                app = app
                    .route("/v1/chat/completions", post(chat_completions_handler))
                    .route("/chat/completions", post(chat_completions_handler))
                    .route("/v1/models", get(models_handler))
                    .route("/models", get(models_handler));
            }
            "google" => {
                // 注意：Google 的 RPC 风格路径形如 /v1beta/models/{model}:generateContent，
                // 把参数 {model} 和字面量 ":generateContent" 放在同一路径段，axum 不允许
                // （panic: "Only one parameter is allowed per path segment"）。
                // 因此改用通配 {*path} 捕获整段，再在 handler 内解析 model 与 action。
                app = app
                    .route("/v1beta/models/{*path}", post(google_handler))
                    .route("/models/{*path}", post(google_handler));
            }
            other => return Err(format!("未知入站协议: {}", other)),
        }
    }
    let app = app
        .route("/collab/agent/send", post(collab_agent_send_handler))
        .route("/collab/agent/message", post(collab_agent_message_handler))
        .route("/collab/agent/task", post(collab_agent_task_handler))
        .layer(middleware::from_fn_with_state(state.clone(), require_proxy_token_mw))
        .layer(middleware::from_fn(log_requests_mw))
        .fallback(catch_all_handler)
        .with_state(state.clone());

    axum::serve(tokio_listener, app)
        .await
        .map_err(|e| format!("代理服务器错误: {}", e))?;
    Ok(())
}

// ─── 协同 agent 总线端点（C/D/E：agent 主动互聊 / 消息 / 任务流） ───
// 鉴权：X-Collab-Token 必须等于该房间的 bus token（由 dispatch_to_tool 注入到被派发 agent 的环境）。

async fn collab_agent_send_handler(
    State(state): State<ProxyState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let room_id = body.get("room_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let token = headers.get("X-Collab-Token").and_then(|h| h.to_str().ok()).unwrap_or("");
    if token != crate::commands::ai::collab::room_bus_token(&room_id) {
        return (StatusCode::FORBIDDEN, "invalid token").into_response();
    }
    let app = match &state.app_handle {
        Some(a) => a.clone(),
        None => return (StatusCode::INTERNAL_SERVER_ERROR, "no app handle").into_response(),
    };
    let from = body.get("from").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let to = body.get("to").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let message = body.get("message").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let parent = body.get("parent").and_then(|v| v.as_str()).map(|s| s.to_string());
    let request_id = crate::commands::ai::collab::agent_delegate(app, &room_id, &from, &to, &message, parent.as_deref()).await;
    match crate::commands::ai::collab::await_delegation(&request_id, 1800).await {
        Some((status, result)) => Json(json!({ "status": status, "result": result })).into_response(),
        None => (StatusCode::REQUEST_TIMEOUT, "delegation not found").into_response(),
    }
}

async fn collab_agent_message_handler(
    State(state): State<ProxyState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let room_id = body.get("room_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let token = headers.get("X-Collab-Token").and_then(|h| h.to_str().ok()).unwrap_or("");
    if token != crate::commands::ai::collab::room_bus_token(&room_id) {
        return (StatusCode::FORBIDDEN, "invalid token").into_response();
    }
    let app = match &state.app_handle {
        Some(a) => a.clone(),
        None => return (StatusCode::INTERNAL_SERVER_ERROR, "no app handle").into_response(),
    };
    let from = body.get("from").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let message = body.get("message").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let parent = body.get("parent").and_then(|v| v.as_str()).map(|s| s.to_string());
    crate::commands::ai::collab::collab_post_message(app, &room_id, &from, &message, parent.as_deref());
    Json(json!({ "ok": true })).into_response()
}

async fn collab_agent_task_handler(
    State(state): State<ProxyState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let room_id = body.get("room_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let token = headers.get("X-Collab-Token").and_then(|h| h.to_str().ok()).unwrap_or("");
    if token != crate::commands::ai::collab::room_bus_token(&room_id) {
        return (StatusCode::FORBIDDEN, "invalid token").into_response();
    }
    let app = match &state.app_handle {
        Some(a) => a.clone(),
        None => return (StatusCode::INTERNAL_SERVER_ERROR, "no app handle").into_response(),
    };
    let res = crate::commands::ai::collab::collab_task_op(app, &room_id, &body);
    Json(res).into_response()
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
/// 路由用通配 {*path} 捕获（axum 不支持参数与字面量同段），在此解析 model（声明名 C）。
/// 流式由 URL 中是否含 `streamGenerateContent` 判定。
async fn google_handler(
    State(state): State<ProxyState>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let path = uri.path();
    let is_stream = path.contains("streamGenerateContent");
    // 从 "/models/" 之后取 model：形如 "gemini-2.5-pro:generateContent" 或
    // "gemini-2.5-pro:streamGenerateContent?alt=sse"，取冒号前部分。
    let model = path
        .split("/models/")
        .nth(1)
        .map(|rest| rest.split([':', '?']).next().unwrap_or("").to_string())
        .unwrap_or_default();
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

    // ─── 详细请求日志 ───
    let is_stream = if inbound == "google" {
        google_is_stream
    } else {
        body.get("stream").and_then(|v| v.as_bool()).unwrap_or(false)
    };
    let msg_count = body.get("messages").and_then(|m| m.as_array()).map(|a| a.len()).unwrap_or(0);
    let first_msg_preview = body.get("messages")
        .and_then(|m| m.as_array())
        .and_then(|a| a.last())
        .and_then(|m| {
            m.get("content").and_then(|c| {
                match c {
                    Value::String(s) => Some(s.as_str()),
                    Value::Array(parts) => parts.iter().find_map(|p| p.get("text").and_then(|t| t.as_str())),
                    _ => None,
                }
            })
        })
        .unwrap_or("");
    let preview: String = first_msg_preview.chars().take(80).collect();
    log_proxy(&format!("  请求详情: messages={}, stream={}, preview=\"{}\"", msg_count, is_stream, preview));

    // ─── emit collab:proxy-request ───
    emit_proxy_event(&state, "collab:proxy-request", json!({
        "model": claimed_model,
        "messages": msg_count,
        "stream": is_stream,
    }));

    // ② 模型伪装：C → B（替换 body.model，google 入站不写 body）
    let actual_model = apply_masquerade(inbound, &mut body, &claimed_model, aliases.as_ref());

    // ③ 协议转换：P_in → P_out（已在 body 写入 B）
    let mut out_body = convert_request(inbound, &outbound, &body, &actual_model);

    // ④ 优化器（P_out 形态）
    optimizers::apply_optimizers(&mut out_body, &outbound, &config);

    // ⑤ 预防式整流（P_out 形态）
    optimizers::apply_preventive_rectifiers(&mut out_body, &outbound, &config);

    // ⑥ 转发
    // 出站为 OpenAI / Anthropic 时，确保请求体带 stream 字段：
    // Google 入站（流式靠 URL 判定）以及跨协议转换后的 body 可能未写入 `stream`，
    // 而 OpenAI / Anthropic 上游依赖 body 中的 stream 字段决定是否流式返回。
    if is_stream && (outbound == "openai" || outbound == "anthropic") {
        if let Some(o) = out_body.as_object_mut() {
            o.insert("stream".into(), json!(true));
        }
    }
    let (upstream_url, auth_name, route_api_key) = build_upstream_url(&config, &outbound, &actual_model, is_stream);
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
    log_proxy(&format!("  出站体: model={}, stream={}", actual_model, is_stream));
    if proxy_debug_enabled() {
        log_proxy_json_full("  入站完整请求体(P_in)", &body);
        log_proxy_json_full("  出站完整请求体(P_out)", &out_body);
    }

    let req = build_upstream_request(&state.client, &config, headers, &upstream_url, &auth_name, &route_api_key, &out_body);
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

    // ─── emit collab:proxy-response-start ───
    emit_proxy_event(&state, "collab:proxy-response-start", json!({
        "status": status.as_u16(),
        "elapsed_ms": start.elapsed().as_millis(),
    }));

    if !status.is_success() {
        let error_body = upstream_resp.text().await.unwrap_or_default();
        log_proxy(&format!(
            "✗ UPSTREAM {} error: {}",
            status.as_u16(),
            error_body.chars().take(300).collect::<String>()
        ));

        // ─── emit collab:proxy-error ───
        emit_proxy_event(&state, "collab:proxy-error", json!({
            "status": status.as_u16(),
            "error": error_body.chars().take(500).collect::<String>(),
        }));

        // ⑦ 反应式整流：修正后重试一次
        if let Some(rectified) = optimizers::try_reactive_rectify(status.as_u16(), &error_body, &out_body, &config, &outbound) {
            let retry_req = build_upstream_request(&state.client, &config, headers, &upstream_url, &auth_name, &route_api_key, &rectified);
            if let Ok(retry_resp) = retry_req.send().await {
                if retry_resp.status().is_success() {
                    log_proxy(&format!("↻ retry succeeded after rectify  ({}ms)", start.elapsed().as_millis()));
                    return process_response(state, retry_resp, &config, inbound, &outbound, &claimed_model, &actual_model, is_stream).await;
                }
            }
        }

        // ⑦½ 协议回退整流（参考 cc-switch 的 with_fallback 哲学）：
        // anthropic 出站遇 401/404 且配置了 openai 回退端点时，说明该供应商的
        // anthropic_url 实际并不兼容 Anthropic 协议（仅 OpenAI 兼容，如
        // longcat/sensenova）。用"另一协议"端点以 openai 出站（a2o 转换）重发一次。
        if outbound == "anthropic"
            && (status == StatusCode::UNAUTHORIZED || status == StatusCode::NOT_FOUND)
            && !config.fallback_base_url.is_empty()
        {
            let mut fb_config = config.clone();
            fb_config.outbound_protocol = "openai".to_string();
            // 清空按模型查的 anthropic 路由，强制走 fallback_base_url（openai 端点），
            // 否则 build_upstream_url 会命中 model_routes 里仍为 anthropic 的端点。
            fb_config.model_routes.clear();
            fb_config.upstream_base_url = config.fallback_base_url.clone();
            fb_config.upstream_api_key = config.fallback_api_key.clone();
            // 用原始（已伪装但未转换的）body 重新做 a2o 转换
            let mut fb_body = convert_request(inbound, "openai", &body, &actual_model);
            optimizers::apply_optimizers(&mut fb_body, "openai", &fb_config);
            optimizers::apply_preventive_rectifiers(&mut fb_body, "openai", &fb_config);
            if is_stream && (fb_config.outbound_protocol == "openai") {
                if let Some(o) = fb_body.as_object_mut() {
                    o.insert("stream".into(), json!(true));
                }
            }
            let (fb_url, fb_auth, fb_key) = build_upstream_url(&fb_config, "openai", &actual_model, is_stream);
            if !fb_url.is_empty() {
                log_proxy(&format!(
                    "↻ protocol fallback anthropic→openai: POST {} auth={}",
                    fb_url, fb_auth
                ));
                let fb_req = build_upstream_request(&state.client, &fb_config, headers, &fb_url, &fb_auth, &fb_key, &fb_body);
                if let Ok(fb_resp) = fb_req.send().await {
                    if fb_resp.status().is_success() {
                        log_proxy(&format!("↻ fallback succeeded  ({}ms)", start.elapsed().as_millis()));
                        return process_response(state, fb_resp, &fb_config, inbound, "openai", &claimed_model, &actual_model, is_stream).await;
                    }
                    log_proxy(&format!("↻ fallback also failed: {}", fb_resp.status().as_u16()));
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
    let start = Instant::now();
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

    if proxy_debug_enabled() {
        log_proxy_json_full("  上游完整响应体(P_out)", &resp_json);
        log_proxy_json_full("  回写完整响应体(P_in)", &in_resp);
    }

    // ─── 提取并日志/emit 响应文本 ───
    let full_text = extract_inbound_full_text(inbound, &in_resp);
    let text_preview: String = full_text.chars().take(100).collect();
    log_proxy(&format!("✓ 非流式响应: text_len={}, preview=\"{}\"", full_text.len(), text_preview));
    if !full_text.is_empty() {
        emit_proxy_event(state, "collab:proxy-delta", json!({
            "delta": full_text,
        }));
        emit_proxy_event(state, "collab:proxy-complete", json!({
            "text": full_text,
            "elapsed_ms": start.elapsed().as_millis(),
        }));
        // 存入代理文本缓存（供 collab 回退）
        store_proxy_text_for(state, &full_text);
    } else {
        emit_proxy_event(state, "collab:proxy-complete", json!({
            "text": "",
            "elapsed_ms": start.elapsed().as_millis(),
        }));
        store_proxy_text_for(state, "");
    }

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
        // 协同上下文（用于 emit 事件）
        let collab_ctx = (state.app_handle.clone(), state.collab_room_id.clone());
        let collab_tool_id = state.tool_id.clone();
        let sse = async_stream::stream! {
            use futures_util::StreamExt;
            tokio::pin!(stream);
            let mut buffer = String::new();
            let mut acc_in: u64 = 0;
            let mut acc_out: u64 = 0;
            let mut acc_text = String::new();
            let mut chunk_count: u64 = 0;
            let stream_start = Instant::now();
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
                                // 提取文本增量
                                if let Some(text) = extract_inbound_delta_text(&proto, &cj) {
                                    if !text.is_empty() {
                                        chunk_count += 1;
                                        acc_text.push_str(&text);
                                        if chunk_count <= 5 {
                                            let preview: String = text.chars().take(50).collect();
                                            log_proxy(&format!("  ▸ SSE chunk #{}: text=\"{}\" (len={})", chunk_count, preview, text.len()));
                                        }
                                        // emit collab:proxy-delta
                                        if let (Some(app), Some(rid)) = &collab_ctx {
                                            if let Some(mid) = get_collab_msg_id(rid, &collab_tool_id) {
                                                let _ = app.emit("collab:proxy-delta", json!({
                                                    "room_id": rid, "msg_id": mid, "delta": text,
                                                }));
                                            }
                                        }
                                    }
                                }
                            }
                        } else {
                            log_proxy("  ▸ SSE [DONE]");
                        }
                    }
                }
                yield Ok::<_, std::io::Error>(chunk);
            }
            log_proxy(&format!("✓ 流式完成: {} chunks, {} chars text, {}ms",
                chunk_count, acc_text.len(), stream_start.elapsed().as_millis()));
            if proxy_debug_enabled() {
                log_proxy_raw("  流式完整文本", &acc_text);
            }
            // emit collab:proxy-complete
            if let (Some(app), Some(rid)) = &collab_ctx {
                if let Some(mid) = get_collab_msg_id(rid, &collab_tool_id) {
                    let _ = app.emit("collab:proxy-complete", json!({
                        "room_id": rid, "msg_id": mid,
                        "text": acc_text,
                        "elapsed_ms": stream_start.elapsed().as_millis(),
                    }));
                }
                // 存入代理文本缓存（供 collab 回退）
                store_proxy_text_with(rid, &collab_tool_id, &acc_text);
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
            // 协同上下文（用于 emit 事件）
            let collab_ctx = (state.app_handle.clone(), state.collab_room_id.clone());
            let collab_tool_id = state.tool_id.clone();
            let out_proto = outbound.to_string();
            let stream = upstream_resp.bytes_stream();
            let sse = async_stream::stream! {
                use futures_util::StreamExt;
                let mut buffer = String::new();
                tokio::pin!(stream);
                let mut acc_text = String::new();
                let mut chunk_count: u64 = 0;
                let stream_start = Instant::now();
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
                            // 从出站协议形态的 chunk 提取文本
                            if let Some(text) = extract_inbound_delta_text(&out_proto, &cj) {
                                if !text.is_empty() {
                                    chunk_count += 1;
                                    acc_text.push_str(&text);
                                    if chunk_count <= 5 {
                                        let preview: String = text.chars().take(50).collect();
                                        log_proxy(&format!("  ▸ SSE chunk #{}: text=\"{}\" (len={})", chunk_count, preview, text.len()));
                                    }
                                    // emit collab:proxy-delta
                                    if let (Some(app), Some(rid)) = &collab_ctx {
                                        if let Some(mid) = get_collab_msg_id(rid, &collab_tool_id) {
                                            let _ = app.emit("collab:proxy-delta", json!({
                                                "room_id": rid, "msg_id": mid, "delta": text,
                                            }));
                                        }
                                    }
                                }
                            }
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
                log_proxy(&format!("✓ 跨协议流式完成: {} chunks, {} chars text, {}ms",
                    chunk_count, acc_text.len(), stream_start.elapsed().as_millis()));
                if proxy_debug_enabled() {
                    log_proxy_raw("  跨协议流式完整文本", &acc_text);
                }
                // emit collab:proxy-complete
                if let (Some(app), Some(rid)) = &collab_ctx {
                    if let Some(mid) = get_collab_msg_id(rid, &collab_tool_id) {
                        let _ = app.emit("collab:proxy-complete", json!({
                            "room_id": rid, "msg_id": mid,
                            "text": acc_text,
                            "elapsed_ms": stream_start.elapsed().as_millis(),
                        }));
                    }
                    // 存入代理文本缓存（供 collab 回退）
                    store_proxy_text_with(rid, &collab_tool_id, &acc_text);
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
/// 按实际模型名 B 查 `model_routes`：命中则用该模型所属供应商的端点与 key，
/// 否则回退到全局 upstream_base_url / upstream_api_key（大模型供应商）。
/// 返回 (url, auth_header_name, api_key)。
fn build_upstream_url(config: &ProxyConfig, outbound: &str, model: &str, is_stream: bool) -> (String, String, String) {
    let (base, api_key) = config.model_routes.get(model)
        .map(|r| (r.base_url.clone(), r.api_key.clone()))
        .unwrap_or_else(|| (config.upstream_base_url.clone(), config.upstream_api_key.clone()));
    let base = base.trim_end_matches('/');
    let (url, auth_name) = match outbound {
        "anthropic" => {
            // 参照 EchoBird messages_handler.rs: 小心拼接，避免 /v1/v1/messages
            // 这类重复（baseURL 已含 /v1 时只补 /messages）。
            let url = if base.contains("/messages") {
                base.to_string()
            } else if base.ends_with("/v1") {
                format!("{}/messages", base)
            } else {
                format!("{}/v1/messages", base)
            };
            (url, "x-api-key".to_string())
        }
        "openai" => {
            // base 通常形如 https://xxx/v1（OpenAI 兼容端点）。避免重复 /v1：
            // 已含 /chat/completions 则保持；已以 /v1 结尾只补 /chat/completions；
            // 否则补 /v1/chat/completions。否则会得到 .../chat/completions 导致上游 404。
            let url = if base.ends_with("/chat/completions") {
                base.to_string()
            } else if base.ends_with("/v1") {
                format!("{}/chat/completions", base)
            } else {
                format!("{}/v1/chat/completions", base)
            };
            (url, "Authorization".to_string())
        }
        "google" => {
            // base 通常形如 https://generativelanguage.googleapis.com（不含 /v1beta）。
            // 若用户配置的 google_url 已带 /v1beta，去掉避免重复拼接导致上游解析异常。
            let gbase = if base.ends_with("/v1beta") {
                &base[..base.len() - 6]
            } else {
                base
            };
            let gbase = gbase.trim_end_matches('/');
            let url = if is_stream {
                format!("{}/v1beta/models/{}:streamGenerateContent?alt=sse", gbase, model)
            } else {
                format!("{}/v1beta/models/{}:generateContent", gbase, model)
            };
            (url, "x-goog-api-key".to_string())
        }
        _ => (String::new(), "Authorization".to_string()),
    };
    (url, auth_name, api_key)
}

/// 构建上游请求（携带对应鉴权头）。
/// `api_key`：由 `build_upstream_url` 按模型路由表解析得到的供应商 key。
fn build_upstream_request(
    client: &reqwest::Client,
    _config: &ProxyConfig,
    headers: &HeaderMap,
    upstream_url: &str,
    auth_name: &str,
    api_key: &str,
    body: &Value,
) -> reqwest::RequestBuilder {
    let mut req = client.post(upstream_url);
    match auth_name {
        "x-api-key" => {
            // 参照 EchoBird: Anthropic 兼容上游有的只认 x-api-key、有的只认
            // Bearer。两者都发，最大化兼容（真实 Anthropic 也接受 Bearer）。
            req = req
                .header("x-api-key", api_key)
                .header("Authorization", format!("Bearer {}", api_key))
                .header("anthropic-version", "2023-06-01")
                .json(body);
        }
        "x-goog-api-key" => {
            req = req.header("x-goog-api-key", api_key).json(body);
        }
        _ => {
            req = req
                .header("Authorization", format!("Bearer {}", api_key))
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
