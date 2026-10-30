use axum::response::IntoResponse;
// inbound responsibility.
use super::{
    convert, header, json, log_proxy, process_request, proxy_debug_enabled, stream_guard, Body,
    HeaderMap, Json, OriginalUri, ProxyState, Response, State, StatusCode, Value,
};

/// GET /health
pub(super) async fn health_handler() -> Json<Value> {
    Json(json!({"status": "ok", "service": "any-version-proxy"}))
}

/// GET /v1/models — OpenAI 入站时透传到上游 OpenAI 的 /models（用于工具列举可用模型）
pub(super) async fn models_handler(
    State(state): State<ProxyState>,
    headers: HeaderMap,
) -> Response {
    let config = state.config.read().await.clone();
    // 模型目录路径可配：OpenAI 惯例是 /models，但有些后端（WorkBuddy）在别的路径上，
    // 配错的话客户端一个模型都枚举不到。
    let models_path = if config.models_path.is_empty() {
        "/models"
    } else {
        config.models_path.as_str()
    };
    let url = format!(
        "{}{}",
        config.upstream_base_url.trim_end_matches('/'),
        models_path
    );
    log_proxy(&format!("← IN   /v1/models  → OUT GET {}", url));

    let mut req = state.client.get(&url);
    // 与其它端点一致地注入自定义头：有些上游（如 WorkBuddy 后端）的模型目录同样要鉴权，
    // 只发 provider key 会 401/404。用户显式配了 Authorization 时不再叠加 Bearer。
    req = crate::proxy::headers::apply(req, &config.upstream_headers);
    if !crate::proxy::headers::has_authorization(&config.upstream_headers) {
        req = req.header(
            "Authorization",
            format!("Bearer {}", config.upstream_api_key),
        );
    }
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
                    return (
                        StatusCode::BAD_GATEWAY,
                        Json(json!({"error": {"message": format!("读取上游响应失败: {}", e)}})),
                    )
                        .into_response();
                }
            };
            // 上游不一定是 OpenAI 形状（WorkBuddy 返回 data.models），归一后再给客户端，
            // 否则客户端按 {object:"list",data:[…]} 解析会拿到空列表。
            let normalized_bytes: Vec<u8> = match serde_json::from_slice::<Value>(&bytes) {
                Ok(body) => {
                    let normalized =
                        convert::normalize_models_response(body, config.models_filter_non_chat);
                    serde_json::to_vec(&normalized).unwrap_or_else(|_| bytes.to_vec())
                }
                // 不是 JSON 就原样透传（例如上游返回纯文本错误）
                Err(_) => bytes.to_vec(),
            };
            let bytes = normalized_bytes;
            let mut out = Response::new(Body::from(bytes));
            *out.status_mut() = status;
            out.headers_mut().insert(
                header::CONTENT_TYPE,
                ct.unwrap_or(header::HeaderValue::from_static("application/json")),
            );
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
pub(super) async fn count_tokens_handler(
    State(state): State<ProxyState>,
    Json(body): Json<Value>,
) -> Json<Value> {
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
                                .filter_map(|p| {
                                    p.get("text").and_then(|t| t.as_str()).map(|s| s.len())
                                })
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
pub(super) async fn messages_handler(
    State(state): State<ProxyState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let claimed = body
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    process_request(&state, &headers, "anthropic", claimed, false, body).await
}

/// POST /v1/chat/completions — OpenAI 入站
pub(super) async fn chat_completions_handler(
    State(state): State<ProxyState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
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
pub(super) async fn google_handler(
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

/// POST /v1/responses — OpenAI **Responses API** 入站。
///
/// 为什么必须有：新版 ChatGPT Desktop / Codex（26.901 起，openai/codex discussion #7782）
/// 不再支持 `wire_api = "chat"`，只会往 `{base_url}/responses` 发请求。缺这条路由的表现就是
/// `unexpected status 404 Not Found: not found, url: http://127.0.0.1:<port>/responses`。
///
/// 设计：**Responses 只做「入口 / 出口方言」** —— 请求先转成 Chat Completions 形态交给
/// [`process_request`]（模型伪装、协议转换、整流器、出站 SSE 全部复用），返回时再转回
/// Responses。不为一个端点复制一整套转发管线。
pub(super) async fn responses_handler(
    State(state): State<ProxyState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let is_stream = body
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let claimed = body
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let chat_body = crate::proxy::responses::responses_to_chat(&body);
    // 工具调用方言还原的开关来自整流器配置（与「协议整流器」分组里的两个勾选对应）
    let (dialects, truncation_guard) = {
        let cfg = state.config.read().await;
        (
            crate::proxy::xml_tools::dialects_from(
                cfg.rectifier_enabled && cfg.rectifier_toolcall_dialect,
            ),
            cfg.stream_truncation_guard,
        )
    };
    let upstream = process_request(&state, &headers, "openai", claimed, false, chat_body).await;

    // 非 2xx 原样透传：Chat 的错误体 `{"error":{message,type,code}}` 正是 Codex 期望的形状，
    // 再包一层 Responses 结构反而让 Codex 读不到 message（只显示一个空错误）。
    if !upstream.status().is_success() {
        return upstream;
    }
    if is_stream {
        wrap_chat_stream_as_responses(upstream, &body, &dialects, truncation_guard)
    } else {
        wrap_chat_json_as_responses(upstream, &body, &dialects).await
    }
}

/// 非流式：Chat 响应体 → Responses 响应体。
pub(super) async fn wrap_chat_json_as_responses(
    upstream: Response,
    request: &Value,
    dialects: &[crate::proxy::xml_tools::Dialect],
) -> Response {
    let status = upstream.status();
    let bytes = match axum::body::to_bytes(upstream.into_body(), 32 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "error": { "message": format!("读取上游响应失败: {e}"), "type": "proxy_error" }
                })),
            )
                .into_response();
        }
    };
    let Ok(chat) = serde_json::from_slice::<Value>(&bytes) else {
        // 不是 JSON（畸形响应）：原样返回，让 Codex 自己把错误显出来
        return Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(bytes))
            .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response());
    };
    // 上游可能不把工具调用翻回标准 `tool_calls`（实测：WorkBuddy space-bunny 背后是
    // MiniMax），那样 content 里只有文本，Codex 会判定轮次结束而不再执行工具。
    let mut chat = chat;
    crate::proxy::xml_tools::apply_tool_calls(&mut chat, &dialects);
    let responses = crate::proxy::responses::chat_to_responses(&chat, request);
    (status, Json(responses)).into_response()
}

/// 流式：把上游的 Chat SSE 逐帧翻译成 Responses 事件流。
pub(super) fn wrap_chat_stream_as_responses(
    upstream: Response,
    request: &Value,
    dialects: &[crate::proxy::xml_tools::Dialect],
    truncation_guard: bool,
) -> Response {
    use axum::body::Bytes;
    use futures_util::StreamExt;
    let model = request
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    let stream = upstream.into_body().into_data_stream();
    // 复制一份所有权：async_stream 生成的流会 move 住捕获的量，不能借用入参
    let dialects = dialects.to_vec();
    let sse = async_stream::stream! {
        tokio::pin!(stream);
        let mut conv =
            crate::proxy::responses::ResponsesStreamConverter::with_dialects(&model, &dialects);
        let mut buffer = String::new();
        // 收尾判定：有内容但没 finish_reason / [DONE] = 截断（不能补 response.completed）
        let mut progress = stream_guard::StreamProgress::new();
        while let Some(item) = stream.next().await {
            let bytes = match item {
                Ok(b) => b,
                Err(e) => {
                    // 传输层断流：必须标记失败，否则 finish() 会补发 response.completed，
                    // 把残缺内容伪装成一次成功完成（Codex 会当作完整答案收下）
                    log_proxy(&format!("  ✗ Responses 上游流中断: {}", e));
                    for ev in conv.fail(&format!("upstream stream transport error: {e}")) {
                        yield Ok::<_, std::convert::Infallible>(Bytes::from(ev));
                    }
                    return;
                }
            };
            buffer.push_str(&String::from_utf8_lossy(&bytes));
            // SSE 帧以空行分隔；一帧里可能有 `event:` 与 `data:` 多行
            while let Some(idx) = buffer.find("\n\n") {
                let frame: String = buffer.drain(..idx + 2).collect();
                if let Some(chunk) = parse_sse_json(&frame) {
                    progress.observe_value("openai", &chunk);
                    for ev in conv.push_chunk(&chunk) {
                        yield Ok::<_, std::convert::Infallible>(Bytes::from(ev));
                    }
                }
            }
        }
        // 末帧可能没以空行收尾，别把最后一段 delta 丢掉
        if !buffer.trim().is_empty() {
            if let Some(chunk) = parse_sse_json(&buffer) {
                progress.observe_value("openai", &chunk);
                for ev in conv.push_chunk(&chunk) {
                    yield Ok::<_, std::convert::Infallible>(Bytes::from(ev));
                }
            }
        }
        if truncation_guard && progress.is_truncated() {
            // 上游 200、已转出内容，但既没有 finish_reason 也没有 [DONE]：
            // 按失败收尾（response.failed），不能补 response.completed —— 那会让
            // Codex 把半截答案当完整结果收下、不再续跑。
            log_proxy("  ✗ Responses 上游流被截断，按失败收尾");
            for ev in conv.fail("upstream stream ended before completion (truncated)") {
                yield Ok::<_, std::convert::Infallible>(Bytes::from(ev));
            }
        } else {
            for ev in conv.finish() {
                yield Ok::<_, std::convert::Infallible>(Bytes::from(ev));
            }
        }
        // 「模型只吐计划就停」这类问题要分清是上游没给工具调用、还是给了被我们丢了
        if proxy_debug_enabled() {
            log_proxy(&format!("  ↩ 上游流式汇总: {}", conv.upstream_debug()));
        }
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .body(Body::from_stream(sse))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// 从一帧 SSE 文本里取出 `data:` 的 JSON（`[DONE]` 与空行忽略）。
pub(super) fn parse_sse_json(frame: &str) -> Option<Value> {
    for line in frame.lines() {
        let Some(rest) = line.strip_prefix("data:") else {
            continue;
        };
        let payload = rest.trim();
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(payload) {
            return Some(v);
        }
    }
    None
}
