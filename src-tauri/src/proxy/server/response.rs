use axum::response::IntoResponse;
use tauri::Emitter;
// response responsibility.
use super::{
    convert_response, emit_proxy_event, extract_inbound_delta_text, extract_inbound_full_text,
    extract_stream_usage, get_collab_msg_id, google, header, json, log_proxy, log_proxy_json_full,
    log_proxy_raw, proxy_debug_enabled, push_request_log, reasoning_fallback, record_proxy_usage,
    record_usage_from_response, set_response_model, sse, store_proxy_text_for,
    store_proxy_text_with, stream_guard, transform, Body, Event, Infallible, Instant, Json,
    ProxyConfig, ProxyState, Response, Sse, StatusCode, Value,
};

/// 处理成功的上游响应（流式 / 非流式）
/// - `claimed_model`：声明名 C，用于响应转换回填（⑧⑨）
/// - `actual_model`：实际模型 B，用于用量统计落库（⑩）
pub(super) async fn process_response(
    state: &ProxyState,
    upstream_resp: reqwest::Response,
    config: &ProxyConfig,
    inbound: &str,
    outbound: &str,
    claimed_model: &str,
    actual_model: &str,
    is_stream: bool,
    upstream_stream: bool,
) -> Response {
    let start = Instant::now();
    if is_stream {
        return stream_response(
            state,
            upstream_resp,
            config,
            inbound,
            outbound,
            claimed_model,
            actual_model,
        )
        .await;
    }
    if upstream_stream {
        // 客户端要非流式、上游却只给流式：收集 SSE 聚合成单个 chat.completion
        return aggregate_upstream_stream(
            state,
            upstream_resp,
            config,
            inbound,
            outbound,
            claimed_model,
            actual_model,
        )
        .await;
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
    // 先把上游可能写进 content 的方言工具调用还原成 tool_calls（见 xml_tools 文档）
    let mut resp_json = resp_json;
    if outbound == "openai" {
        let dialects = crate::proxy::xml_tools::dialects_from(
            config.rectifier_enabled && config.rectifier_toolcall_dialect,
        );
        if crate::proxy::xml_tools::apply_tool_calls(&mut resp_json, &dialects)
            && proxy_debug_enabled()
        {
            log_proxy("  ↩ 已把正文里的工具调用还原成 tool_calls（方言适配层）");
        }
    }
    // 纯思考轮次：把思考原文补成正文（`reasoning_content` 原样保留）。
    // 放在 xml_tools **之后** —— 先把正文里的方言工具调用还原成 tool_calls，
    // 才能正确判断「这轮到底有没有工具调用」。转换器会把补出的 content 变成对应协议形态。
    if config.reasoning_as_content && reasoning_fallback::patch_chat_response(&mut resp_json) > 0 {
        log_proxy("  ↩ 纯思考轮次：已把思考原文作为正文补发");
    }
    let mut in_resp = convert_response(outbound, inbound, &resp_json, claimed_model);

    // ⑨ 模型伪装回填：响应 model 字段写回声明名 C
    set_response_model(&mut in_resp, claimed_model, inbound);

    // ⑩ 统计落库（记录实际模型 B，而非声明名 C）；非流式无首字延迟，传 0
    record_usage_from_response(
        state,
        config,
        inbound,
        &in_resp,
        actual_model,
        start.elapsed().as_millis() as u64,
    );

    if proxy_debug_enabled() {
        log_proxy_json_full("  上游完整响应体(P_out)", &resp_json);
        log_proxy_json_full("  回写完整响应体(P_in)", &in_resp);
    }

    // ─── 提取并日志/emit 响应文本 ───
    let full_text = extract_inbound_full_text(inbound, &in_resp);
    let text_preview: String = full_text.chars().take(100).collect();
    log_proxy(&format!(
        "✓ 非流式响应: text_len={}, preview=\"{}\"",
        full_text.len(),
        text_preview
    ));
    if !full_text.is_empty() {
        emit_proxy_event(
            state,
            "collab:proxy-delta",
            json!({
                "delta": full_text,
            }),
        );
        emit_proxy_event(
            state,
            "collab:proxy-complete",
            json!({
                "text": full_text,
                "elapsed_ms": start.elapsed().as_millis(),
            }),
        );
        // 存入代理文本缓存（供 collab 回退）
        store_proxy_text_for(state, &full_text);
    } else {
        emit_proxy_event(
            state,
            "collab:proxy-complete",
            json!({
                "text": "",
                "elapsed_ms": start.elapsed().as_millis(),
            }),
        );
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

/// 强制上游流式、但客户端要非流式时的响应路径：把上游 SSE 收集聚合成普通 JSON。
///
/// 上游只收流式时（如 WorkBuddy 后端对非流式直接返回
/// `Non-stream chat request is currently not supported`），而客户端发了非流式请求，
/// 代理就把上游的 SSE 收集起来聚合成一个 `chat.completion` —— 客户端看不出中间发生过什么。
///
/// 只有 `force_upstream_stream` 打开且入站非流式时才会走到这里；其余情况走原来的两条路。
pub(super) async fn aggregate_upstream_stream(
    state: &ProxyState,
    upstream_resp: reqwest::Response,
    config: &ProxyConfig,
    inbound: &str,
    outbound: &str,
    claimed_model: &str,
    actual_model: &str,
) -> Response {
    use futures_util::StreamExt;
    let start = Instant::now();

    // 逐帧收集上游的 chat.completion.chunk
    let mut chunks: Vec<Value> = Vec::new();
    let mut buffer = String::new();
    let mut byte_stream = upstream_resp.bytes_stream();
    while let Some(item) = byte_stream.next().await {
        let Ok(bytes) = item else { break };
        buffer = sse::append_utf8_safe(&buffer, &bytes);
        loop {
            // take_sse_block 返回的 rest 借用 buffer，这里立刻转成 owned 再回写 buffer
            let Some((block, rest_owned)) =
                sse::take_sse_block(&buffer).map(|(b, r)| (b, r.to_string()))
            else {
                break;
            };
            buffer = rest_owned;
            let Some(data) = sse::extract_sse_data(&block) else {
                continue;
            };
            let trimmed = data.trim();
            if trimmed.is_empty() || trimmed == "[DONE]" {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
                chunks.push(v);
            }
        }
    }
    // 流末尾可能还挂着没分隔符的半帧
    if let Some(data) = sse::extract_sse_data(&buffer) {
        let trimmed = data.trim();
        if !trimmed.is_empty() && trimmed != "[DONE]" {
            if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
                chunks.push(v);
            }
        }
    }

    if chunks.is_empty() {
        let mut stats = state.stats.write().await;
        stats.failed_requests += 1;
        return (
            StatusCode::BAD_GATEWAY,
            Json(json!({
                "error": {"message": "上游流式响应为空（已强制流式但没收到任何数据）"}
            })),
        )
            .into_response();
    }

    // 聚合成单个 chat.completion，再按入站协议回填
    let mut aggregated = sse::aggregate_chat_chunks(&chunks, actual_model);
    if outbound == "openai" {
        let dialects = crate::proxy::xml_tools::dialects_from(
            config.rectifier_enabled && config.rectifier_toolcall_dialect,
        );
        crate::proxy::xml_tools::apply_tool_calls(&mut aggregated, &dialects);
    }
    // 纯思考轮次兜底（聚合路径同非流式）
    if config.reasoning_as_content && reasoning_fallback::patch_chat_response(&mut aggregated) > 0 {
        log_proxy("  ↩ 纯思考轮次：已把思考原文作为正文补发");
    }
    let mut in_resp = convert_response(outbound, inbound, &aggregated, claimed_model);
    set_response_model(&mut in_resp, claimed_model, inbound);

    // 统计落库（与流式分支不同：没有首字延迟，整体耗时记为延迟）
    record_usage_from_response(
        state,
        config,
        inbound,
        &in_resp,
        actual_model,
        start.elapsed().as_millis() as u64,
    );

    let full_text = extract_inbound_full_text(inbound, &in_resp);
    log_proxy(&format!(
        "✓ 聚合流式响应（非流式入站）: {} 帧, text_len={}, {}ms",
        chunks.len(),
        full_text.len(),
        start.elapsed().as_millis()
    ));
    if !full_text.is_empty() {
        store_proxy_text_for(state, &full_text);
    }
    emit_proxy_event(
        state,
        "collab:proxy-complete",
        json!({
            "text": full_text,
            "elapsed_ms": start.elapsed().as_millis(),
        }),
    );

    let mut stats = state.stats.write().await;
    stats.success_requests += 1;

    // 聚合后是普通 JSON，与转换后的非流式响应同样用 application/json
    Json(in_resp).into_response()
}

/// 流式响应：把上游（出站协议）SSE 转换为入站协议 SSE。
/// - `claimed_model`：声明名 C（用于转换器回填）
/// - `actual_model`：实际模型 B（用于流式用量统计落库）
pub(super) async fn stream_response(
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
        let guard = config.stream_truncation_guard;
        let reasoning_guard = config.reasoning_as_content;
        let ls = config.source.clone();
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
            // 收尾判定：有内容 / 有 finish_reason / 有 [DONE]
            let mut progress = stream_guard::StreamProgress::new();
            // 传输层是否出过错（`yield Err` 之后 break 的情况）
            let mut transport_failed = false;
            // 纯思考轮次兜底：需要把补帧插到 finish 之前，所以见到 finish 后先把原始
            // 字节缓冲起来，流结束时按需「补帧 → 原样吐出」。
            let mut fb = reasoning_fallback::ReasoningTracker::new();
            let mut tailing = false;
            let mut tail: Vec<axum::body::Bytes> = Vec::new();
            let mut first_id: Option<String> = None;
            // 首个文本增量到达的时刻：t/s 统计要从生成窗口中扣掉首字等待（抄自 cc-switch dc0febe5）
            let mut first_token_ms: u64 = 0;
            let stream_start = Instant::now();
            while let Some(r) = stream.next().await {
                let chunk = match r {
                    Ok(c) => c,
                    Err(e) => {
                        transport_failed = true;
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
                                progress.observe_value(&proto, &cj);
                                fb.observe(&cj);
                                if first_id.is_none() {
                                    first_id = cj
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .map(str::to_string);
                                }
                                let (i, o) = extract_stream_usage(&proto, &cj);
                                if i > 0 { acc_in = i; }
                                if o > 0 { acc_out = o; }
                                // 提取文本增量
                                if let Some(text) = extract_inbound_delta_text(&proto, &cj) {
                                    if !text.is_empty() {
                                        if first_token_ms == 0 {
                                            first_token_ms = stream_start.elapsed().as_millis() as u64;
                                        }
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
                            progress.observe_done();
                            log_proxy("  ▸ SSE [DONE]");
                        }
                    }
                }
                // 纯思考轮次兜底：只要还有可能补正文（没出现过正文/工具），
                // 一旦见到 finish_reason 就把后续原始字节攒住，等确认后再插帧。
                // 不需要补时这些字节会**原样**吐出，客户端看到的字节与上游一致。
                if reasoning_guard && fb.may_backfill() && (tailing || progress.saw_finish()) {
                    tailing = true;
                    tail.push(chunk);
                    continue;
                }
                yield Ok::<_, std::io::Error>(chunk);
            }
            // 截断判定：已转发内容、但既无 finish_reason 也无 [DONE]。
            // 这是「上游 200 之后被掐断」，不是「模型说完」——见 stream_guard 模块头。
            let truncated = guard && !transport_failed && progress.is_truncated();
            if truncated {
                log_proxy(&format!(
                    "✗ 流被截断（{} chunks 后无 finish_reason/[DONE]），已补 error 帧",
                    chunk_count
                ));
                if !ls.is_empty() {
                    push_request_log(&ls, "✗ 上游流被截断，已补 error 帧".to_string());
                }
                yield Ok::<_, std::io::Error>(axum::body::Bytes::from_static(
                    stream_guard::TRUNCATED_STREAM_FRAME.as_bytes(),
                ));
            } else {
                // 纯思考轮次：把思考原文补成一帧正文。
                // **必须在这里**（缓冲的 finish 帧吐出之前）—— 客户端见到 finish_reason
                // 就停止累积，补在它后面等于没发。
                if tailing {
                    if let Some(text) = fb.take_backfill_text() {
                        log_proxy(&format!(
                            "  ↩ 纯思考轮次：已把思考作为正文补发（{} 字）",
                            text.chars().count()
                        ));
                        if !ls.is_empty() {
                            push_request_log(&ls, "↩ 纯思考轮次：已把思考作为正文补发".to_string());
                        }
                        let id = first_id.take().unwrap_or_else(|| "chatcmpl-kira".to_string());
                        yield Ok::<_, std::io::Error>(axum::body::Bytes::from(
                            reasoning_fallback::build_frame(
                                &id,
                                chrono::Local::now().timestamp(),
                                &text,
                            ),
                        ));
                    }
                }
                log_proxy(&format!("✓ 流式完成: {} chunks, {} chars text, {}ms",
                    chunk_count, acc_text.len(), stream_start.elapsed().as_millis()));
            }
            // 缓冲的收尾字节原样吐出（截断时同样要吐：里面有 finish_reason）
            for buffered in tail.drain(..) {
                yield Ok::<_, std::io::Error>(buffered);
            }
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
                record_proxy_usage(
                    &tid,
                    &pid,
                    &act,
                    acc_in,
                    acc_out,
                    0,
                    0,
                    stream_start.elapsed().as_millis() as u64,
                    first_token_ms,
                );
            }
            let mut s = stats.write().await;
            if transport_failed || truncated {
                // 截断/断流不计成功：否则日志里只剩一条 tok 正常的假成功
                //（参考侧原话：这正是这类问题长期查不出来的原因）
                s.failed_requests += 1;
            } else {
                s.success_requests += 1;
            }
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
            let guard = config.stream_truncation_guard;
            let ls = config.source.clone();
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
                // 上游是否已发过 [DONE]：OpenAI/Google 出站下它就是完整终止标记，
                // 补收尾会重复发一次；Anthropic 出站下它不算终止，仍需补 message_stop
                let mut sent_done = false;
                // 收尾判定：有内容 / 有终止标记（finish_reason、message_stop…）/ 有 [DONE]
                let mut progress = stream_guard::StreamProgress::new();
                let mut transport_failed = false;
                // 首个文本增量到达的时刻：t/s 统计要从生成窗口中扣掉首字等待
                let mut first_token_ms: u64 = 0;
                let stream_start = Instant::now();
                while let Some(r) = stream.next().await {
                    let chunk = match r {
                        Ok(c) => c,
                        Err(e) => {
                            transport_failed = true;
                            // 上游断流：先把已打开的 content block 闭合，**不要**发
                            // message_stop（那等于声称正常结束），随后补一条 error 事件。
                            // 若流已终止（终止事件已发出），只记日志不再写任何事件 ——
                            // 否则会产出「message_stop 之后又来一个 error」的畸形流。
                            for ev in conv.close_open_blocks() {
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
                            if conv.is_terminated() {
                                log_proxy(&format!("  ✗ 上游在终止事件之后断流，丢弃错误事件: {}", e));
                            } else {
                                log_proxy(&format!("  ✗ 上游流中断: {}", e));
                                yield Ok::<_, Infallible>(Event::default().data(format!("{{\"error\":\"{}\"}}", e)));
                            }
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
                            sent_done = true;
                            progress.observe_done();
                            yield Ok::<_, Infallible>(Event::default().data("[DONE]"));
                            // 部分上游只发 [DONE] 而不带 finish_reason：Anthropic 出站
                            // 在这里补收尾，否则客户端永远等不到 message_stop（会挂住）
                            if out_proto == "anthropic" {
                                for ev in conv.finish() {
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
                            continue;
                        }
                        if let Ok(cj) = serde_json::from_str::<Value>(&data_str) {
                            progress.observe_value(&out_proto, &cj);
                            // 从出站协议形态的 chunk 提取文本
                            if let Some(text) = extract_inbound_delta_text(&out_proto, &cj) {
                                if !text.is_empty() {
                                    if first_token_ms == 0 {
                                        first_token_ms = stream_start.elapsed().as_millis() as u64;
                                    }
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
                // 循环结束。两种可能必须分开：
                // ① 上游已经发了终止标记 → 正常收尾；
                // ② 转发过内容、却没有任何终止标记 → **截断**：只能闭合块 + 补 error 事件，
                //    绝不能调 finish()（那是 end_turn + message_stop，等于声称「模型说完了」）。
                let truncated = guard && !transport_failed && progress.is_truncated();
                if truncated {
                    for ev in conv.close_open_blocks() {
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
                    log_proxy(&format!(
                        "  ✗ 跨协议流被截断（{} chunks 后无终止标记），已补 error 事件",
                        chunk_count
                    ));
                    if !ls.is_empty() {
                        push_request_log(&ls, "✗ 上游流被截断，已补 error 事件".to_string());
                    }
                    yield Ok::<_, Infallible>(Event::default().data(
                        "{\"error\":{\"message\":\"upstream stream ended before completion (truncated)\",\"type\":\"upstream_error\",\"code\":\"upstream_parse\"}}",
                    ));
                } else if !sent_done {
                    // 已终止但没见到 [DONE]（部分上游如此）：补收尾，
                    // 否则客户端拿到的是未闭合的 content block
                    for ev in conv.finish() {
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
                if !truncated {
                    log_proxy(&format!("✓ 跨协议流式完成: {} chunks, {} chars text, {}ms",
                        chunk_count, acc_text.len(), stream_start.elapsed().as_millis()));
                }
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
                    record_proxy_usage(
                        &tid,
                        &pid,
                        &cm,
                        in_t,
                        out_t,
                        0,
                        0,
                        stream_start.elapsed().as_millis() as u64,
                        first_token_ms,
                    );
                }
                let mut s = stats.write().await;
                if truncated || transport_failed {
                    s.failed_requests += 1;
                } else {
                    s.success_requests += 1;
                }
            };
            return Sse::new(sse).into_response();
        }};
    }

    match (inbound, outbound) {
        ("anthropic", "openai") => {
            run_stream!(transform::StreamConverter::new(claimed.clone()));
        }
        ("openai", "anthropic") => {
            run_stream!(transform::AnthropicToOpenaiStreamConverter::new(
                claimed.clone()
            ));
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
