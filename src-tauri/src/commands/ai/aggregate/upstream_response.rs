//! upstream response responsibility.
use super::*;

/// 从上游响应 JSON 提取 usage（按入口协议的形态），落账到 ai_usage。
pub(super) fn record_usage(
    inbound: &str,
    candidate: &AggCandidate,
    resp: &Value,
    elapsed_ms: u128,
) {
    let usage = resp.get("usage").cloned().unwrap_or(Value::Null);
    let (input, output) = match inbound {
        "anthropic" => (
            usage
                .get("input_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
            usage
                .get("output_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
        ),
        _ => (
            usage
                .get("prompt_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
            usage
                .get("completion_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
        ),
    };
    let (cache_read, cache_write) = crate::commands::ai::usage::cache_tokens_from_json(&usage);
    if input == 0 && output == 0 && cache_read == 0 {
        return;
    }
    let _ = log_usage_entry(
        &UsageEntry::success(
            "aggregate",
            &candidate.model_id,
            Some(&candidate.provider_id),
        )
        .tokens(input, output)
        .cache(cache_read, cache_write)
        .timing(elapsed_ms as u64, 0),
    );
}

/// 按入口/出站协议对响应做转换（跨协议时），并提取 usage 落账。
pub(super) fn finish_non_stream(
    inbound: &'static str,
    candidate: &AggCandidate,
    upstream: Value,
    request_model: &str,
    elapsed_ms: u128,
) -> Response {
    let converted = if inbound == candidate.outbound.as_str() {
        upstream
    } else {
        crate::proxy::convert::convert_response(
            candidate.outbound.as_str(),
            inbound,
            &upstream,
            request_model,
        )
    };
    record_usage(inbound, candidate, &converted, elapsed_ms);
    (StatusCode::OK, Json(converted)).into_response()
}

/// 同协议转发时的模型名改写（纯函数，便于单测）。
///
/// 聚合入口对外只暴露 `kiro-proxy`（AGGREGATE_MODEL_ID）；直接 `body.clone()`
/// 会把这个入口模型名原样透传给上游 → 上游报「模型不存在」。
/// 必须改写为候选配置的 `model_id` 再转发。跨协议路径由 transform 函数完成同样改写。
pub(super) fn rewrite_upstream_model(mut body: Value, model: &str) -> Value {
    body["model"] = Value::String(model.to_string());
    body
}

/// 单候选请求：成功返回响应；失败返回分类错误。
/// 空闲超时包装：流式过程中两个 chunk 之间超过 `idle` 没有数据就终止流。
///
/// 抄 cc-switch `AppProxyConfig.streamingIdleTimeout`。注意它**只能终止当前流**，
/// 不能换候选 —— 首字节之后上游已经开始产出，换一家会把这段输出重复计费。
/// 流被终止时以 `io::Error` 收尾，axum 会把它当成连接错误传给客户端，
/// 客户端至少能立刻报错而不是无限挂住。
pub(super) fn idle_timeout_stream<S>(
    inner: S,
    idle: std::time::Duration,
) -> impl futures_util::Stream<Item = Result<bytes::Bytes, std::io::Error>>
where
    S: futures_util::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send + 'static,
{
    async_stream::stream! {
        // Pin 到堆上：reqwest 的 `bytes_stream()` 只承诺 `impl Stream`，不承诺 Unpin，
        // 而 `StreamExt::next` 需要 Unpin。`Pin<Box<S>>` 本身是 Unpin。
        let mut inner = Box::pin(inner);
        loop {
            match tokio::time::timeout(idle, futures_util::StreamExt::next(&mut inner)).await {
                Ok(Some(Ok(chunk))) => yield Ok(chunk),
                Ok(Some(Err(e))) => {
                    yield Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()));
                    break;
                }
                Ok(None) => break,
                Err(_) => {
                    yield Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        format!("上游流式响应空闲超时（{}s 无数据）", idle.as_secs()),
                    ));
                    break;
                }
            }
        }
    }
}

pub(super) async fn try_candidate(
    state: &AggState,
    candidate: &AggCandidate,
    inbound: &'static str,
    body: &Value,
    stream_requested: bool,
    // 反应式整流修正后的 P_out 请求体（Some = 本次直接发它，不再跑优化器）
    outbound_override: Option<&Value>,
    // 整流用的统一代理配置（由 [`candidate_proxy_config`] 构造）
    proxy_cfg: &ProxyConfig,
    // 首字节超时：连上上游后迟迟不吐字节就判失败，让故障转移真正生效
    first_byte_timeout: std::time::Duration,
    idle_timeout: std::time::Duration,
) -> Result<Response, CandidateError> {
    let model = &candidate.model_id;
    let outbound = candidate.outbound.as_str();
    // 同协议只改模型名（入口是 kiro-proxy，上游只认自己配置的模型名）；跨协议先按非流式
    // 整体转换（流式转换仅覆盖 a↔o 主流组合）。转换分发表统一走 proxy::convert，与代理一致。
    let cross = inbound != outbound;
    let upstream_body = match outbound_override {
        Some(fixed) => fixed.clone(),
        None => {
            let mut built = if !cross {
                rewrite_upstream_model(body.clone(), model)
            } else {
                let mut b = body.clone();
                if stream_requested {
                    b["stream"] = Value::Bool(false);
                }
                crate::proxy::convert::convert_request(inbound, outbound, &b, model)
            };
            // 与单工具代理同一套整流：出站前跑角色归一化 + 优化器 + 预防式整流。
            // 此前聚合完全不走这几步，导致同一份配置在「代理」与「聚合」两条路径上
            // 协议行为不一致（thinking 参数、图片降级、cache 断点全都只在一边生效）。
            if outbound == "openai" {
                crate::proxy::convert::normalize_openai_message_roles(&mut built);
                crate::proxy::convert::collapse_system_messages_to_head(&mut built);
                crate::proxy::convert::normalize_image_data_urls_for_model(&mut built, model);
            }
            optimizers::apply_optimizers(&mut built, outbound, proxy_cfg);
            optimizers::apply_preventive_rectifiers(&mut built, outbound, proxy_cfg);
            built
        }
    };
    let upstream_stream = !cross && stream_requested;

    let (url, auth_name) = build_candidate_url(
        candidate.outbound,
        &candidate.base_url,
        model,
        upstream_stream,
        candidate.include_v1,
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| CandidateError {
            class: FailureClass::Transient,
            message: format!("HTTP 客户端构建失败: {}", e),
            retry_after: None,
            status: None,
            error_body: String::new(),
            outbound_body: upstream_body.clone(),
        })?;
    // 鉴权 + 自定义头统一走 proxy::upstream（与协议转换代理一致：anthropic 双头、显式头优先）
    let req = crate::proxy::upstream::inject_auth(
        client.post(&url),
        auth_name,
        &candidate.api_key,
        &candidate.headers,
    );
    // 首字节超时（抄 cc-switch `streamingFirstByteTimeout`）：`send()` 返回即响应头到达，
    // 也就是"第一个字节"。此前只有 300s 的整体超时，上游「连得上但挂住不响应」会把整条
    // 聚合链堵死 300s —— 故障转移在此期间完全不起作用。超时按瞬时失败处理，
    // 由调用方决定重试还是切下一个候选。
    let send_result =
        tokio::time::timeout(first_byte_timeout, req.json(&upstream_body).send()).await;
    let resp = match send_result {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            let message = if e.is_connect() {
                format!("连接被拒绝（{}）", url)
            } else if e.is_timeout() {
                format!(
                    "请求超时（{}s）",
                    std::time::Duration::from_secs(300).as_secs()
                )
            } else {
                format!("请求失败: {}", e)
            };
            return Err(CandidateError {
                class: FailureClass::Transient,
                message,
                retry_after: None,
                status: None,
                error_body: String::new(),
                outbound_body: upstream_body.clone(),
            });
        }
        Err(_) => {
            return Err(CandidateError {
                class: FailureClass::Transient,
                message: format!(
                    "首字节超时（{}s 内上游未响应，{}）",
                    first_byte_timeout.as_secs(),
                    url
                ),
                retry_after: None,
                status: None,
                error_body: String::new(),
                outbound_body: upstream_body.clone(),
            });
        }
    };
    let status = resp.status();
    let retry_after = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok());
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        let class = classify_failure(Some(status.as_u16()), &text);
        // 失败落账：按模型成功率的分母靠它（参考 ai-toolbox per-model success rate）。
        // 这里记的是「候选级失败」——同一请求换到下一个候选成功时，成功那条也会落账，
        // 因此按模型看是"该模型失败过几次"，这正是排查坏供应商需要的口径。
        let _ = log_usage_failure(
            "aggregate",
            &candidate.model_id,
            Some(&candidate.provider_id),
            class.as_str(),
        );
        return Err(CandidateError {
            class,
            message: format!("HTTP {} {}", status.as_u16(), trim_error(&text)),
            retry_after,
            status: Some(status.as_u16()),
            error_body: text,
            outbound_body: upstream_body.clone(),
        });
    }

    let started = std::time::Instant::now();

    // 同协议 + 流式：字节流透传（首字节后不再切换候选，避免重复计费）
    if upstream_stream && inbound == candidate.outbound.as_str() {
        let stream = idle_timeout_stream(resp.bytes_stream(), idle_timeout);
        let body_stream = axum::body::Body::from_stream(stream);
        let response = Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "text/event-stream")
            .body(body_stream)
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
        return Ok(response);
    }

    // 跨协议 + 流式：a↔o 用流式转换器；google 组合降级为非流式整体转换（记日志说明）
    if upstream_stream && inbound != candidate.outbound.as_str() {
        let pair_supported = matches!(
            (inbound, candidate.outbound),
            ("anthropic", Outbound::OpenAi) | ("openai", Outbound::Anthropic)
        );
        if pair_supported {
            return stream_cross_protocol(state, candidate, inbound, resp, started, idle_timeout);
        }
        emit_aggregate_log(
            &state.app,
            "route",
            "warn",
            format!(
                "候选 {} / {} 为 google 出站，跨协议流式暂不支持，已降级为非流式整体转换",
                candidate.provider_name, candidate.model_id
            ),
        );
    }

    // 非流式：整体转换 + 落账
    let text = resp.text().await.map_err(|e| CandidateError {
        class: FailureClass::Transient,
        message: format!("读取响应失败: {}", e),
        retry_after: None,
        status: Some(status.as_u16()),
        error_body: String::new(),
        outbound_body: upstream_body.clone(),
    })?;
    let upstream_json: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    Ok(finish_non_stream(
        inbound,
        candidate,
        upstream_json,
        model,
        started.elapsed().as_millis(),
    ))
}

/// a↔o 跨协议流式转换器的统一包装（两个转换器类型不同，但方法签名一致）。
pub(super) enum CrossStreamConverter {
    AnthropicToOpenai(crate::proxy::transform::StreamConverter),
    OpenaiToAnthropic(crate::proxy::transform::AnthropicToOpenaiStreamConverter),
}

impl CrossStreamConverter {
    pub(super) fn convert_chunk(&mut self, chunk: &Value) -> Vec<String> {
        match self {
            Self::AnthropicToOpenai(c) => c.convert_chunk(chunk),
            Self::OpenaiToAnthropic(c) => c.convert_chunk(chunk),
        }
    }
    pub(super) fn usage(&self) -> (u64, u64) {
        match self {
            Self::AnthropicToOpenai(c) => c.usage(),
            Self::OpenaiToAnthropic(c) => c.usage(),
        }
    }
    /// 异常路径：只闭合已打开的 content block，不发终止事件
    pub(super) fn close_open_blocks(&mut self) -> Vec<String> {
        match self {
            Self::AnthropicToOpenai(c) => c.close_open_blocks(),
            Self::OpenaiToAnthropic(c) => c.close_open_blocks(),
        }
    }
    /// 完整收尾：闭合块 + 终止事件。幂等。
    pub(super) fn finish(&mut self) -> Vec<String> {
        match self {
            Self::AnthropicToOpenai(c) => c.finish(),
            Self::OpenaiToAnthropic(c) => c.finish(),
        }
    }
    pub(super) fn is_terminated(&self) -> bool {
        match self {
            Self::AnthropicToOpenai(c) => c.is_terminated(),
            Self::OpenaiToAnthropic(c) => c.is_terminated(),
        }
    }
}

/// 把转换器产出的一行 SSE 文本（`event: x` / `data: y`）转成 axum `Event`。
pub(super) fn to_event(raw: &str) -> axum::response::sse::Event {
    use axum::response::sse::Event;
    let mut event = Event::default();
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("event:") {
            event = event.event(rest.trim());
        } else if let Some(rest) = line.strip_prefix("data:") {
            event = event.data(rest.trim());
        }
    }
    event
}

/// 跨协议流式转发（仅 a↔o）：用流式转换器把上游 chunk 转成入口协议的 SSE 事件。
pub(super) fn stream_cross_protocol(
    state: &AggState,
    candidate: &AggCandidate,
    inbound: &'static str,
    resp: reqwest::Response,
    started: std::time::Instant,
    idle_timeout: std::time::Duration,
) -> Result<Response, CandidateError> {
    use axum::response::sse::{Event, Sse};
    use futures_util::StreamExt;

    let model = candidate.model_id.clone();
    let provider_id = candidate.provider_id.clone();
    let outbound = candidate.outbound;
    let stream = resp.bytes_stream();
    // 生成出的流必须 'static，不能借用 `state`，先把 AppHandle 克隆进去
    let app = state.app.clone();
    let sse = async_stream::stream! {
        let mut conv = match (inbound, outbound) {
            ("anthropic", Outbound::OpenAi) => Some(CrossStreamConverter::AnthropicToOpenai(
                crate::proxy::transform::StreamConverter::new(model.clone()),
            )),
            ("openai", Outbound::Anthropic) => Some(CrossStreamConverter::OpenaiToAnthropic(
                crate::proxy::transform::AnthropicToOpenaiStreamConverter::new(model.clone()),
            )),
            _ => None,
        };
        let Some(conv) = conv.as_mut() else {
            yield Ok::<_, std::convert::Infallible>(Event::default().data("{\"error\":\"unsupported stream pair\"}"));
            return;
        };
        let mut buffer = String::new();
        tokio::pin!(stream);
        // 上游是否已发过 [DONE]（OpenAI 出站下它即完整终止，补收尾会重复发）
        let mut sent_done = false;
        // 空闲超时：两个 chunk 之间超过 `idle_timeout` 无数据就终止流（首字节之后不能换
        // 候选 —— 上游已在产出，换一家会重复计费，只能把错误传给客户端让它立刻失败）
        loop {
            let next = tokio::time::timeout(idle_timeout, stream.next()).await;
            let r = match next {
                Ok(Some(r)) => r,
                Ok(None) => break,
                Err(_) => {
                    let msg = format!("上游流式响应空闲超时（{}s 无数据）", idle_timeout.as_secs());
                    emit_aggregate_log(&app, "route", "warn", format!(
                        "候选 {} / {} 流式空闲超时，已终止该流: {msg}", provider_id, model
                    ));
                    // 先闭合已打开的 block 再报错；流已终止则只记日志，不再写事件
                    for ev in conv.close_open_blocks() {
                        yield Ok::<_, std::convert::Infallible>(to_event(&ev));
                    }
                    if conv.is_terminated() {
                        emit_aggregate_log(&app, "route", "warn", format!(
                            "候选 {} / {} 在终止事件之后超时，丢弃错误事件", provider_id, model
                        ));
                    } else {
                        yield Ok::<_, std::convert::Infallible>(Event::default().data(format!("{{\"error\":\"{msg}\"}}")));
                    }
                    break;
                }
            };
            let chunk = match r {
                Ok(c) => c,
                Err(e) => {
                    for ev in conv.close_open_blocks() {
                        yield Ok::<_, std::convert::Infallible>(to_event(&ev));
                    }
                    if conv.is_terminated() {
                        emit_aggregate_log(&app, "route", "warn", format!(
                            "候选 {} / {} 在终止事件之后断流，丢弃错误事件", provider_id, model
                        ));
                    } else {
                        yield Ok::<_, std::convert::Infallible>(Event::default().data(format!("{{\"error\":\"{}\"}}", e)));
                    }
                    break;
                }
            };
            buffer = crate::proxy::sse::append_utf8_safe(&buffer, &chunk);
            while let Some((block, remainder)) = crate::proxy::sse::take_sse_block(&buffer) {
                buffer = remainder.to_string();
                let data_str = match crate::proxy::sse::extract_sse_data(&block) {
                    Some(d) => d,
                    None => continue,
                };
                if data_str == "[DONE]" {
                    sent_done = true;
                    yield Ok::<_, std::convert::Infallible>(Event::default().data("[DONE]"));
                    // 只发 [DONE] 不带 finish_reason 的上游：Anthropic 出站需补收尾
                    if inbound == "anthropic" {
                        for ev in conv.finish() {
                            yield Ok::<_, std::convert::Infallible>(to_event(&ev));
                        }
                    }
                    continue;
                }
                if let Ok(cj) = serde_json::from_str::<Value>(&data_str) {
                    for ev in conv.convert_chunk(&cj) {
                        yield Ok::<_, std::convert::Infallible>(to_event(&ev));
                    }
                }
            }
        }
        // 循环自然结束（上游没发 finish_reason 就断了）：补收尾，
        // 否则客户端拿到的是未闭合的 content block（Anthropic 客户端会报错/挂住）
        if !sent_done {
            for ev in conv.finish() {
                yield Ok::<_, std::convert::Infallible>(to_event(&ev));
            }
        }
        // 跨协议流式的 usage 从转换器聚合而来，按入口协议形态落账
        let (in_t, out_t) = conv.usage();
        if in_t > 0 || out_t > 0 {
            let _ = log_usage_db_timed(
                "aggregate",
                &model,
                Some(&provider_id),
                in_t,
                out_t,
                started.elapsed().as_millis() as u64,
                0,
            );
        }
    };
    Ok(Sse::new(sse).into_response())
}

/// 上游错误体压成一行（用于日志与 `CandidateError.message`）。
/// 截断走 [`crate::commands::utils::truncate_utf8`]：上游错误文案常带中文，
/// `&cleaned[..200]` 会正好切在多字节字符中间而 panic。
pub(super) fn trim_error(text: &str) -> String {
    let cleaned = text.trim().replace('\n', " ");
    let truncated = crate::commands::utils::truncate_utf8(&cleaned, 200);
    if truncated.len() == cleaned.len() {
        cleaned
    } else {
        format!("{}…", truncated)
    }
}

/// 压缩旁路：成功返回 Some(压缩后的 body)；未启用/不可达返回 None 或 Err。
pub(super) async fn compress_body(
    hr: &HeadroomRuntime,
    body: &Value,
) -> Result<Option<Value>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(hr.timeout_ms))
        .build()
        .map_err(|e| format!("HTTP 客户端构建失败: {}", e))?;
    let url = format!("{}/v1/compress", hr.base_url);
    let resp = client.post(&url).json(body).send().await.map_err(|e| {
        if e.is_connect() {
            format!("连接被拒绝——Headroom 服务未启动（{}）", hr.base_url)
        } else if e.is_timeout() {
            "压缩超时".to_string()
        } else {
            format!("请求失败: {}", e)
        }
    })?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status().as_u16()));
    }
    let value: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析压缩响应失败: {}", e))?;
    // 超时等场景 headroom 会 fail-open：原样返回并带 compression_skipped
    if value
        .get("compression_skipped")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return Ok(None);
    }
    let messages = value.get("messages").cloned();
    match messages {
        Some(msgs) => {
            let mut next = body.clone();
            next["messages"] = msgs;
            Ok(Some(next))
        }
        None => Ok(None),
    }
}
