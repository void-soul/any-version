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

use super::{
    convert, google, optimizers, reasoning_fallback,
    session_headers::SessionIds,
    sse, stream_guard, transform,
    types::{ProxyConfig, UpstreamHeader},
    upstream_error,
    upstream_error::UpstreamErrorKind,
};
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
use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use tokio::sync::RwLock;

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
    let listener =
        std::net::TcpListener::bind(&addr).map_err(|e| format!("绑定空闲端口失败: {}", e))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    Ok((port, listener))
}

/// 在已绑定的监听器上启动代理服务（内部使用，不入站 Async 绑定）。
pub async fn serve_proxy(
    config: ProxyConfig,
    listener: std::net::TcpListener,
) -> Result<(), String> {
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
    let (stop, stop_guard, finish_guard) = lifecycle::instance(state.config.clone());

    // 登记配置句柄：2API 在 Buddy 切号后靠它热更新鉴权头（否则只能重启服务）。
    // 同端口先清掉旧句柄，避免反复启动后残留。
    if let Ok(mut running) = RUNNING_CONFIGS.lock() {
        running.retain(|(p, _)| *p != config.listen_port);
        running.push((config.listen_port, state.config.clone()));
    }

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
                    // Responses 也挂在 openai 入站下：它是 OpenAI 家族的另一种形态，
                    // 且新版 ChatGPT Desktop / Codex（26.901+）**只会**打这个端点 ——
                    // 不给它注册路由，Codex 直接 404 not found: /responses。
                    .route("/v1/responses", post(responses_handler))
                    .route("/responses", post(responses_handler))
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
    let app = super::hooks::current()
        .extend_routes(app)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_proxy_token_mw,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            log_requests_mw,
        ))
        .fallback(catch_all_handler)
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(
            stop.clone(),
            lifecycle::cancel_requests,
        ));

    // Keep the serving future alive long enough to close accepted connections when its owner aborts.
    let server = tokio::spawn(async move {
        let _finish = finish_guard;
        let listener = connection::OwnedListener::new(tokio_listener, stop.clone());
        let result = axum::serve(listener, app)
            .with_graceful_shutdown(async move { stop.cancelled().await })
            .await
            .map_err(|e| format!("代理服务器错误: {}", e));
        _finish.clear().await;
        result
    });
    let result = server.await.map_err(|e| e.to_string())?;
    drop(stop_guard);
    result
}

/// 把 bind 失败翻译成可操作的提示（抄自 CodexPlusPlus 0e9a86a）。
///
/// - 被占用（`AddrInUse`）：给出「关掉占用者 / 换端口」的出路；
/// - 被系统保留（Windows raw os error **10013**）：Hyper-V/WSL 开机会把动态端口段
///   （49152-65535）划进排除区间，这时**不是**被占用，重启也好不了，必须换端口或调整排除区间；
/// - 其他错误：原样冒泡，不误贴标签。
fn classify_bind_error(addr: &str, error: &std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::AddrInUse {
        return format!(
            "代理端口被占用（{}）：请关闭占用该端口的程序，或在 AI 模型配置里换一个端口后重试。",
            addr
        );
    }
    #[cfg(windows)]
    if error.raw_os_error() == Some(10013) {
        return format!(
            "代理端口被系统保留，无法绑定（{}，Windows os error 10013）。\
             Hyper-V/WSL 启动时会把一段动态端口划入系统排除区间，重启不会恢复。任选其一：\
             1) 换一个端口后重试（也可用环境变量 {} 临时覆盖）；\
             2) 用 `netsh int ipv4 show excludedportrange protocol=tcp` 查看排除区间并避开；\
             3) 关闭 Hyper-V/WSL 的动态端口预留后重启系统。",
            addr,
            crate::proxy::PROXY_PORT_ENV
        );
    }
    format!("绑定代理端口 {} 失败: {}", addr, error)
}

/// 启动代理服务器（按 config.listen_port 绑定，用于手动/独立启动场景）。
pub async fn start_proxy_server(config: ProxyConfig) -> Result<(), String> {
    let addr = format!("{}:{}", config.listen_address, config.listen_port);
    let listener =
        std::net::TcpListener::bind(&addr).map_err(|e| classify_bind_error(&addr, &e))?;
    log_proxy(&format!("启动代理服务器: {}", addr));
    serve_proxy(config, listener).await
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
fn apply_masquerade(
    inbound: &str,
    body: &mut Value,
    claimed: &str,
    aliases: Option<&transform::ModelAliases>,
) -> String {
    let normalize = |s: &str| s.replace("[1m]", "").replace("[1M]", "").trim().to_string();
    let actual = match aliases {
        Some(a) => {
            // 先按原始名映射，再按去后缀名映射（兼容 1M 上下文后缀）
            let mapped = transform::map_model_name(claimed, a);
            if mapped != claimed {
                mapped
            } else {
                let n = normalize(claimed);
                if n != claimed {
                    transform::map_model_name(&n, a)
                } else {
                    claimed.to_string()
                }
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

/// ③/⑧ 协议转换分发表已下沉到 `proxy::convert`（与聚合共用），此处只留别名包装，
/// 让本文件里的调用点（`convert_request(...)` / `convert_response(...)`）不动。
fn convert_request(inbound: &str, outbound: &str, body: &Value, model: &str) -> Value {
    convert::convert_request(inbound, outbound, body, model)
}

fn convert_response(outbound: &str, inbound: &str, resp: &Value, claimed: &str) -> Value {
    convert::convert_response(outbound, inbound, resp, claimed)
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
    duration_ms: u64,
) {
    let (in_t, out_t) = match inbound {
        "anthropic" => (
            resp.get("usage")
                .and_then(|u| u.get("input_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
            resp.get("usage")
                .and_then(|u| u.get("output_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
        ),
        "openai" => (
            resp.get("usage")
                .and_then(|u| u.get("prompt_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
            resp.get("usage")
                .and_then(|u| u.get("completion_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
        ),
        "google" => {
            let um = resp.get("usageMetadata");
            let i = um
                .and_then(|u| u.get("promptTokenCount"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let cand = um
                .and_then(|u| u.get("candidatesTokenCount"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let thought = um
                .and_then(|u| u.get("thoughtsTokenCount"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            (i, cand + thought)
        }
        _ => (0, 0),
    };
    // 缓存命中：Anthropic 走 cache_read_input_tokens，OpenAI 系走 prompt_tokens_details.cached_tokens
    let (cache_read, cache_write) = resp
        .get("usage")
        .map(super::metering::cache_tokens_from_json)
        .unwrap_or((0, 0));
    if in_t > 0 || out_t > 0 || cache_read > 0 {
        record_proxy_usage(
            &config.tool_id,
            &config.provider_id,
            model,
            in_t,
            out_t,
            cache_read,
            cache_write,
            duration_ms,
            0,
        );
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
            let i = um
                .and_then(|u| u.get("promptTokenCount"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let cand = um
                .and_then(|u| u.get("candidatesTokenCount"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let thought = um
                .and_then(|u| u.get("thoughtsTokenCount"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            (i, cand + thought)
        }
        _ => (0, 0),
    }
}

/// ⑥ 构建上游 URL 与鉴权头名称。
/// 按实际模型名 B 查 `model_routes`：命中则用该模型所属供应商的端点与 key，
/// 否则回退到全局 upstream_base_url / upstream_api_key（大模型供应商）。
/// 返回 (url, auth_header_name, api_key, 自定义请求头)。
/// 拼接与鉴权统一走 `proxy::upstream`（与聚合共用，避免两处漂移）。
fn build_upstream_url(
    config: &ProxyConfig,
    outbound: &str,
    model: &str,
    is_stream: bool,
) -> (String, String, String, Vec<UpstreamHeader>) {
    let (base, api_key, headers, include_v1) = config
        .model_routes
        .get(model)
        .map(|r| {
            (
                r.base_url.clone(),
                r.api_key.clone(),
                r.headers.clone(),
                r.include_v1,
            )
        })
        .unwrap_or_else(|| {
            (
                config.upstream_base_url.clone(),
                config.upstream_api_key.clone(),
                config.upstream_headers.clone(),
                config.upstream_include_v1,
            )
        });
    let (url, auth_name) =
        super::upstream::resolve_url(outbound, &base, model, is_stream, include_v1);
    (url, auth_name.to_string(), api_key, headers)
}

/// 构建上游请求（携带对应鉴权头 + 供应商自定义头）。
/// `api_key`：由 `build_upstream_url` 按模型路由表解析得到的供应商 key。
/// `custom_headers`：同一次解析得到的自定义请求头；用户显式配置的头优先于默认注入
/// （抄自 CodexPlusPlus ea0ac5d：显式 `Authorization` 优先于供应商 API Key，不做隐式合并）。
fn build_upstream_request(
    client: &reqwest::Client,
    headers: &HeaderMap,
    upstream_url: &str,
    auth_name: &str,
    api_key: &str,
    custom_headers: &[UpstreamHeader],
    body: &Value,
    session: Option<&SessionIds>,
) -> reqwest::RequestBuilder {
    let mut req = super::upstream::inject_auth(
        client.post(upstream_url),
        auth_name,
        api_key,
        custom_headers,
    );
    if let Some(beta) = headers.get("anthropic-beta") {
        if let Ok(val) = beta.to_str() {
            req = req.header("anthropic-beta", val);
        }
    }
    // 会话头族：每次调用都重生成消息级 / trace（一次调用 = 一次出站尝试）。
    // 用户显式配置了同名头时跳过，避免 reqwest 的 append 语义把同一个头发两遍。
    if let Some(session) = session {
        for h in session.attempt_headers() {
            if !super::headers::overrides(custom_headers, &h.key) {
                req = req.header(h.key.as_str(), h.value);
            }
        }
    }
    req.json(body)
}

#[cfg(test)]
mod tests {
    use super::classify_bind_error;
    use std::io::{Error, ErrorKind};

    #[test]
    fn classify_bind_error_explains_addr_in_use() {
        let msg = classify_bind_error(
            "127.0.0.1:15721",
            &Error::new(ErrorKind::AddrInUse, "in use"),
        );
        assert!(msg.contains("被占用"));
        assert!(msg.contains("127.0.0.1:15721"));
    }

    #[test]
    fn classify_bind_error_keeps_other_errors_verbatim() {
        let msg = classify_bind_error(
            "127.0.0.1:15721",
            &Error::new(ErrorKind::PermissionDenied, "access denied"),
        );
        assert!(msg.contains("access denied"));
        assert!(!msg.contains("被占用"));
        assert!(!msg.contains("被系统保留"));
    }

    /// Windows 保留端口（Hyper-V/WSL 动态端口排除区间）必须给出对症提示，而不是一句英文 bind 失败。
    #[cfg(windows)]
    #[test]
    fn classify_bind_error_explains_windows_reserved_port() {
        let msg = classify_bind_error("127.0.0.1:57321", &Error::from_raw_os_error(10013));
        assert!(msg.contains("被系统保留"));
        assert!(msg.contains("netsh"));
        assert!(msg.contains(crate::proxy::PROXY_PORT_ENV));
    }

    #[test]
    fn resolve_proxy_port_falls_back_to_configured_value() {
        // 环境变量未设置时（测试进程内通常如此）回落配置值；非法取值同样回落。
        if std::env::var(crate::proxy::PROXY_PORT_ENV).is_err() {
            assert_eq!(crate::proxy::resolve_proxy_port(15721), 15721);
        }
    }

    #[test]
    fn request_log_buffer_filters_caps_and_clears() {
        // 全局缓冲在测试间共享（Rust 测试多线程并行），所以把三段断言放进同一个测试顺序跑，
        // 用唯一 source 前缀避免与其他测试串扰。
        const SRC: &str = "2api-unit-test";

        super::clear_request_logs();
        super::push_request_log(SRC, "→ GET /v1/models");
        super::push_request_log("ai-other", "→ POST /v1/chat/completions");
        super::push_request_log(SRC, "← 200 /v1/models");

        // 来源过滤：面板只拿自己来源的，别的来源不混入
        let mine = super::get_request_logs(SRC);
        assert!(mine.iter().all(|e| e.source == SRC));

        // 清空
        super::clear_request_logs();
        assert!(super::get_request_logs(SRC).is_empty());

        // 上限：超出后丢最旧的
        for i in 0..(super::REQUEST_LOG_CAP + 10) {
            super::push_request_log(SRC, format!("msg {i}"));
        }
        let all = super::get_request_logs(SRC);
        assert_eq!(all.len(), super::REQUEST_LOG_CAP);
        assert!(all[0].message.contains("msg 10"), "{}", all[0].message);

        super::clear_request_logs();
    }
}

mod connection;
mod lifecycle;
mod observability;
pub use lifecycle::wait_for_stopped_proxies;
use observability::{
    catch_all_handler, emit_proxy_event, extract_inbound_delta_text, extract_inbound_full_text,
    get_collab_msg_id, log_proxy, log_proxy_json_full, log_proxy_raw, log_requests_mw,
    proxy_debug_enabled, record_proxy_usage, require_proxy_token_mw, store_proxy_text_for,
    store_proxy_text_with, REQUEST_LOG_CAP, RUNNING_CONFIGS,
};
pub use observability::{
    clear_collab_msg_id, clear_request_logs, get_request_logs, push_request_log, set_collab_msg_id,
    take_proxy_text, unregister_running, update_running_config, RequestLogEntry,
};

mod inbound;
use inbound::{
    chat_completions_handler, count_tokens_handler, google_handler, health_handler,
    messages_handler, models_handler, responses_handler,
};

mod dispatch;
use dispatch::process_request;

mod response;
use response::process_response;
