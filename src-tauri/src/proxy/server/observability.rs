use tauri::Emitter;
// Observability, request history and live configuration registry.
use super::{
    get, google, header, json, middleware, Arc, HashMap, Instant, IntoResponse, Mutex, OnceLock,
    OriginalUri, ProxyConfig, ProxyState, Request, Response, RwLock, State, StatusCode, Value,
    VecDeque,
};

/// 记录使用量到 SQLite 数据库（线程安全）。携带真实 tool_id / provider_id。
///
/// `duration_ms` / `first_token_ms` 供输出速度（t/s）统计（抄自 cc-switch dc0febe5）：
/// 生成窗口 = 总耗时 − 首字延迟；非流式没有首字延迟传 0，窗口即总耗时。
/// 两个值都为 0 表示未测量，该记录不参与 t/s 聚合。
///
/// `cache_read_tokens` / `cache_write_tokens` 供缓存命中率统计：只有能从响应体
/// 读到缓存的路径才传非 0（流式透传路径目前拿不到，按 0 记，不参与命中率）。
#[allow(clippy::too_many_arguments)]
pub(super) fn record_proxy_usage(
    tool_id: &str,
    provider_id: &str,
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
    duration_ms: u64,
    first_token_ms: u64,
) {
    crate::proxy::hooks::current().record_usage(crate::proxy::hooks::UsageRecord {
        tool_id,
        provider_id,
        model,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_write_tokens,
        duration_ms,
        first_token_ms,
    });
}

/// 打印代理网络请求日志（统一前缀，不打印任何敏感头/密钥）。
///
/// **同时落一份到滚动日志文件**（`<数据目录>/logs/any-version.log.<日期>`，走 tracing）：
/// 控制台会被顶掉，排查「注入是否发生」这类问题时只能靠文件回溯
/// —— 2026-10-06 排查方言提示注入时，用户手动复制控制台只能拿到尾部，现场全丢。
pub(super) fn log_proxy(msg: &str) {
    let ts = chrono::Local::now().format("%H:%M:%S%.3f");
    println!("[proxy] {} {}", ts, msg);
    tracing::info!("[proxy] {} {}", ts, msg);
}

// ─── 请求日志（供 Buddy/2API 面板展示） ───

/// 一条请求日志。`source` 区分代理来源（`2api` = 2API 服务 / `ai` = AI 工具模型代理），
/// 面板按来源过滤展示。
#[derive(Clone, serde::Serialize)]
pub struct RequestLogEntry {
    pub ts: String,
    pub source: String,
    pub message: String,
}

/// 请求日志内存环形缓冲：UI 轮询读取；上限 1000 条，超出时丢最旧的。
pub(super) static REQUEST_LOG: Mutex<VecDeque<RequestLogEntry>> = Mutex::new(VecDeque::new());
pub(super) const REQUEST_LOG_CAP: usize = 1000;

/// 追加一条请求日志到内存缓冲。`source` 取自 `ProxyConfig.source`。
pub fn push_request_log(source: &str, message: impl Into<String>) {
    let mut q = REQUEST_LOG.lock().unwrap();
    if q.len() >= REQUEST_LOG_CAP {
        q.pop_front();
    }
    q.push_back(RequestLogEntry {
        ts: chrono::Local::now().format("%H:%M:%S%.3f").to_string(),
        source: source.to_string(),
        message: message.into(),
    });
}

/// 读取请求日志；`source_filter` 非空时只返回匹配来源的。
pub fn get_request_logs(source_filter: &str) -> Vec<RequestLogEntry> {
    REQUEST_LOG
        .lock()
        .unwrap()
        .iter()
        .filter(|e| source_filter.is_empty() || e.source == source_filter)
        .cloned()
        .collect()
}

/// 清空请求日志缓冲。
pub fn clear_request_logs() {
    REQUEST_LOG.lock().unwrap().clear();
}

/// 是否启用代理「完整日志」。
/// 设置环境变量 ANY_VERSION_PROXY_DEBUG=1 开启后，会把入站/出站的完整请求体、
/// 上游完整响应体、流式完整文本打印到控制台，便于排查配置与模型生效问题。
/// 不设置时仅打印精简预览，避免正常使用时日志刷屏。
pub(super) fn proxy_debug_enabled() -> bool {
    std::env::var("ANY_VERSION_PROXY_DEBUG")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false)
}

/// 打印完整 JSON 体（仅在 debug 开启时由调用方判断后使用）。
/// 超大响应体截断到 200KB 并注明总长，避免一次性刷屏卡死终端。
pub(super) fn log_proxy_json_full(msg: &str, value: &Value) {
    match serde_json::to_string(value) {
        Ok(s) => log_proxy_raw(msg, &s),
        Err(e) => log_proxy(&format!("{}: <序列化失败: {}>", msg, e)),
    }
}

/// 打印完整原始字符串（同上，用于流式完整文本等）。
pub(super) fn log_proxy_raw(msg: &str, s: &str) {
    if s.len() > 200_000 {
        // 必须按字符边界截断：流式完整文本常含中文，`&s[..200_000]` 会 panic。
        log_proxy(&format!(
            "{}: {}...(truncated, total {} bytes)",
            msg,
            crate::commands::foundation::truncate_utf8(s, 200_000),
            s.len()
        ));
    } else {
        log_proxy(&format!("{}: {}", msg, s));
    }
}

/// 全局请求日志中间件：记录每一个进入代理的请求（方法 + 路径）与最终响应状态。
/// 即使路径未匹配（axum 返回 404）也会被记录，便于排查「not found」等静默失败。
/// 同时写进内存缓冲（带 `ProxyConfig.source` 来源标识），供 Buddy/2API 面板展示。
pub(super) async fn log_requests_mw(
    State(state): State<ProxyState>,
    req: Request,
    next: axum::middleware::Next,
) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let source = state.config.read().await.source.clone();
    log_proxy(&format!("→ 收到请求: {} {}", method, path));
    push_request_log(&source, format!("→ {} {}", method, path));
    let start = Instant::now();
    let resp = next.run(req).await;
    let status = resp.status().as_u16();
    let ms = start.elapsed().as_millis();
    log_proxy(&format!("← 响应: {} {} ({}ms)", status, path, ms));
    push_request_log(&source, format!("← {} {} ({}ms)", status, path, ms));
    resp
}

/// 未匹配路由的兜底处理：明确打印 404 路径（而非 axum 默认静默 404）。
pub(super) async fn catch_all_handler(OriginalUri(uri): OriginalUri) -> impl IntoResponse {
    log_proxy(&format!("✗ 404 未匹配路由: {}", uri.path()));
    (StatusCode::NOT_FOUND, "not found")
}

/// 本地代理鉴权中间件：配置了 auth_token 时，`/v1*`、`/messages`、`/chat/completions`
/// 等数据路由必须携带正确 token（Authorization: Bearer / x-api-key / x-goog-api-key 三者
/// 任一匹配即可，覆盖 Anthropic/OpenAI/Google 三种 SDK 的凭据注入习惯）。
/// `/health` 与 `/collab/*`（有独立 X-Collab-Token 校验）免鉴权。
pub(super) async fn require_proxy_token_mw(
    State(state): State<ProxyState>,
    req: Request,
    next: middleware::Next,
) -> Response {
    let path = req.uri().path().to_string();
    let expected = state.config.read().await.auth_token.clone();
    if expected.is_empty()
        || path == "/health"
        || crate::proxy::hooks::current().independently_authenticated(&path)
    {
        return next.run(req).await;
    }
    let headers = req.headers();
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .map(|v| v.strip_prefix("Bearer ").unwrap_or(v))
        .unwrap_or("");
    let x_api_key = headers
        .get("x-api-key")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let x_goog = headers
        .get("x-goog-api-key")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if bearer == expected || x_api_key == expected || x_goog == expected {
        next.run(req).await
    } else {
        log_proxy(&format!("✗ 鉴权失败: 缺少有效 token: {}", path));
        (StatusCode::UNAUTHORIZED, "unauthorized").into_response()
    }
}

/// 向前端 emit 协同代理事件（仅在协同上下文存在时发送）
pub(super) fn emit_proxy_event(state: &ProxyState, event: &str, payload: Value) {
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
pub(super) static PROXY_TEXT: OnceLock<std::sync::Mutex<HashMap<String, String>>> = OnceLock::new();

// ─── 协同上下文：room_id::tool_id → 当前 msg_id（支持代理复用时动态更新）───
pub(super) static COLLAB_MSG_ID: OnceLock<std::sync::Mutex<HashMap<String, String>>> =
    OnceLock::new();

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
pub(super) fn get_collab_msg_id(room_id: &str, tool_id: &str) -> Option<String> {
    let key = format!("{}::{}", room_id, tool_id);
    COLLAB_MSG_ID.get()?.lock().unwrap().get(&key).cloned()
}

/// 代理层在响应完成时存储文本（使用当前 msg_id）
pub(super) fn store_proxy_text_for(state: &ProxyState, text: &str) {
    if let Some(room_id) = &state.collab_room_id {
        store_proxy_text_with(room_id, &state.tool_id, text);
    }
}

/// 流式路径中使用（不需要 state 引用）
pub(super) fn store_proxy_text_with(room_id: &str, tool_id: &str, text: &str) {
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
        eprintln!(
            "[proxy] ✓ 代理响应已缓存: msg={}, text_len={}",
            msg_id,
            text.len()
        );
    }
}

/// collab dispatch_to_tool 回退取用代理文本（取后删除）
pub fn take_proxy_text(msg_id: &str) -> Option<String> {
    PROXY_TEXT.get()?.lock().unwrap().remove(msg_id)
}

/// 从入站协议形态的 SSE chunk 中提取文本增量
pub(super) fn extract_inbound_delta_text(inbound: &str, cj: &Value) -> Option<String> {
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
                Some("content_block_delta") => cj
                    .get("delta")
                    .and_then(|d| d.get("text"))
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string()),
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
                    let texts: Vec<String> = parts
                        .iter()
                        .filter_map(|p| {
                            p.get("text")
                                .and_then(|t| t.as_str())
                                .map(|s| s.to_string())
                        })
                        .collect();
                    if texts.is_empty() {
                        None
                    } else {
                        Some(texts.join(""))
                    }
                })
        }
        _ => None,
    }
}

/// 从入站协议形态的非流式响应中提取完整文本
pub(super) fn extract_inbound_full_text(inbound: &str, resp: &Value) -> String {
    match inbound {
        "openai" => resp
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|a| a.first())
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string(),
        "anthropic" => resp
            .get("content")
            .and_then(|c| c.as_array())
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|p| {
                        if p.get("type").and_then(|t| t.as_str()) == Some("text") {
                            p.get("text")
                                .and_then(|t| t.as_str())
                                .map(|s| s.to_string())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default(),
        "google" => resp
            .get("candidates")
            .and_then(|c| c.as_array())
            .and_then(|a| a.first())
            .and_then(|c| c.get("content"))
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.as_array())
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|p| {
                        p.get("text")
                            .and_then(|t| t.as_str())
                            .map(|s| s.to_string())
                    })
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default(),
        _ => String::new(),
    }
}

/// 运行中代理实例的配置句柄注册表（按监听端口索引）。
///
/// `serve_proxy` 会把 `ProxyConfig` 封进内部的 `Arc<RwLock<_>>`，外部拿不到，
/// 因此**无法**在服务运行期间换配置。2API 需要这一点：Buddy 切号后要立刻把新的
/// 鉴权头换进去，而不是重启服务。这里登记一份句柄，让外部能按端口找到它。
pub(super) static RUNNING_CONFIGS: std::sync::LazyLock<
    std::sync::Mutex<Vec<(u16, Arc<RwLock<ProxyConfig>>)>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

/// 热更新某个运行中实例的配置。返回是否找到该实例。
///
/// 找不到通常意味着服务没在跑（尚未启动 / 已停止），调用方应视作"无需更新"。
///
/// 注意：注册表用 `std::sync::Mutex` 保护，但取出句柄后就释放锁，再 `.write().await`
/// —— 不能把同步锁的 guard 带过 await 点，否则会卡住整个 runtime。
pub async fn update_running_config(port: u16, f: impl FnOnce(&mut ProxyConfig)) -> bool {
    let handle = {
        let Ok(guard) = RUNNING_CONFIGS.lock() else {
            return false;
        };
        guard
            .iter()
            .find(|(p, _)| *p == port)
            .map(|(_, cfg)| cfg.clone())
    };
    match handle {
        Some(cfg) => {
            let mut w = cfg.write().await;
            f(&mut w);
            true
        }
        None => false,
    }
}

/// 注销某个端口的实例（服务停止时调用，避免句柄残留）
pub fn unregister_running(port: u16) {
    if let Ok(mut guard) = RUNNING_CONFIGS.lock() {
        guard.retain(|(p, _)| *p != port);
    }
}
