//! SSE consumption and bounded continuation.
use super::RESUME_MIN_PROGRESS_CHARS;
use super::{
    ai_http_client, completion_url, error_or_status, extract_json, is_recoverable_network_error,
    is_retryable_no_output, log_call, log_preview, next_call_id, parse_json, read_body_text_lossy,
    send_with_retries, strip_prefix_of, ChannelHooks, ChannelProvider, EMPTY_RETRY_DELAY,
    EMPTY_RETRY_MAX, SSE_IDLE_TIMEOUT, STREAM_RESUME_MAX,
};

/// 解析一行 SSE data 载荷：累积 delta.content、delta.reasoning*（思考过程）、记录 usage、识别 error。
pub(super) fn process_sse_line(
    line: &str,
    acc: &mut String,
    reasoning: &mut String,
    usage: &mut Option<serde_json::Value>,
) -> Result<(), String> {
    let data = line.strip_prefix("data:").map(str::trim).unwrap_or("");
    if data.is_empty() || data == "[DONE]" {
        return Ok(());
    }
    let v: serde_json::Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(_) => return Ok(()), // 非 JSON 行（ping/event 等）忽略
    };
    if let Some(err) = v.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("流式响应中的错误");
        return Err(format!("AI错误: {}", msg));
    }
    if let Some(d) = v
        .pointer("/choices/0/delta/content")
        .and_then(|c| c.as_str())
    {
        acc.push_str(d);
    }
    // 推理型模型（DeepSeek-R1 / Qwen 思考模式等）的「思考过程」：
    // 字段名不统一，OpenAI o 系用 reasoning，DeepSeek/Qwen 用 reasoning_content，
    // 个别网关用 thinking。取到哪个算哪个，累积出来供调用方展示「思维过程」。
    let delta = v.pointer("/choices/0/delta");
    if let Some(r) = delta
        .and_then(|d| d.get("reasoning_content"))
        .or_else(|| delta.and_then(|d| d.get("reasoning")))
        .or_else(|| delta.and_then(|d| d.get("thinking")))
        .and_then(|r| r.as_str())
    {
        if !r.is_empty() {
            reasoning.push_str(r);
        }
    }
    if let Some(u) = v.get("usage") {
        *usage = Some(u.clone());
    }
    Ok(())
}

/// 消费一次 SSE 流：逐块读取（带空闲超时，防止断流后永久挂起），跨包拼行、
/// 累积 content 与 reasoning；逐块检查取消（用户点「停止」后流式请求随即中断）。
/// `model` 仅用于错误信息定位。返回 (正文全文, 思考全文, usage)，读流失败返回 Err。
async fn consume_sse(
    hooks: &dyn ChannelHooks,
    call_id: u64,
    model: &str,
    resp: reqwest::Response,
    on_stream_tick: &(dyn Fn(usize, &str) + Send + Sync),
) -> Result<(String, String, Option<serde_json::Value>), String> {
    use futures_util::StreamExt;
    log_call(call_id, "开始读取 SSE 流…");
    let mut acc = String::new();
    let mut acc_reason = String::new();
    let mut usage: Option<serde_json::Value> = None;
    let mut line_buf = String::new();
    let mut last_emit = std::time::Instant::now();
    let mut last_len = 0usize;
    let mut last_reason_len = 0usize;
    let mut reason_emitted = false;
    let mut stream = resp.bytes_stream();
    loop {
        let chunk = match tokio::time::timeout(SSE_IDLE_TIMEOUT, stream.next()).await {
            Err(_) => {
                let e = format!(
                    "流式中断: 读流空闲超时（超过 {}s 未收到数据，model={}）",
                    SSE_IDLE_TIMEOUT.as_secs(),
                    model
                );
                log_call(
                    call_id,
                    &format!("SSE 流错误：{}（已累计 {} 字符）", e, acc.chars().count()),
                );
                return Err(e);
            }
            Ok(None) => break, // 正常流结束
            Ok(Some(item)) => {
                hooks.check_cancel()?;
                match item {
                    Ok(c) => c,
                    Err(e) => {
                        let e = format!("流式中断(model={}): {}", model, e);
                        log_call(
                            call_id,
                            &format!("SSE 流错误：{}（已累计 {} 字符）", e, acc.chars().count()),
                        );
                        return Err(e);
                    }
                }
            }
        };
        line_buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(pos) = line_buf.find('\n') {
            let line = line_buf[..pos].trim().to_string();
            line_buf.drain(..=pos);
            process_sse_line(&line, &mut acc, &mut acc_reason, &mut usage).map_err(|e| {
                log_call(
                    call_id,
                    &format!(
                        "SSE data 行报错：{}（行内容：{}）",
                        e,
                        line.chars().take(500).collect::<String>()
                    ),
                );
                e
            })?;
        }
        // 节流：每积累约 400 字符或 200ms 上报一次，保持实时又不刷屏
        let cur = acc.chars().count();
        if cur.saturating_sub(last_len) >= 400 || last_emit.elapsed().as_millis() >= 200 {
            last_emit = std::time::Instant::now();
            last_len = cur;
            // 传**累积全文**而不是尾片段：调用方要做增量解析（数据库设计器边收边落图）
            // 就必须拿到完整文本；只给尾部 120 字符会丢中段内容，无法累积。
            // 需要尾预览的调用方（思维导图）自己切片即可。
            on_stream_tick(cur, &acc);
        }
        // 思考过程（reasoning_content）单独节流上报：推理型模型先想几十秒才动笔，
        // 这部分不报进度，前端就像卡死。阈值（200 字符/200ms）独立于 content，
        // 保证「只思考不写字」时屏幕也在动。
        //
        // text 传**累积全文**而不是尾部 200 字符。早先只给尾片段是觉得「完整思考
        // 文本对调用方没价值」，但那样前端永远只能看到最后 200 字——用户想排查
        // 「AI 为什么给出这个结果」时，恰恰需要完整的推理链条。量级上推理文本通常
        // 几千字，按 200 字符节流也就十几帧，远比丢信息划算。
        let r_cur = acc_reason.chars().count();
        if r_cur > last_reason_len
            && (r_cur - last_reason_len >= 200 || last_emit.elapsed().as_millis() >= 200)
        {
            hooks.on_progress(
                "reasoning",
                serde_json::json!({ "length": r_cur, "text": acc_reason.clone() }),
            );
            last_reason_len = r_cur;
            reason_emitted = true;
            last_emit = std::time::Instant::now();
        }
    }
    // 收尾：缓冲区里剩余的最后一行（可能没有换行符结尾）
    if !line_buf.trim().is_empty() {
        process_sse_line(line_buf.trim(), &mut acc, &mut acc_reason, &mut usage).map_err(|e| {
            log_call(
                call_id,
                &format!(
                    "SSE data 行报错：{}（行内容：{}）",
                    e,
                    line_buf.trim().chars().take(500).collect::<String>()
                ),
            );
            e
        })?;
    }
    // 思考过程收尾：有增量就标 done（前端据此把「思考中」定格为已完成的思考块）。
    // 同样带上全文：某些供应商只在最后一帧给出思考，中途一帧都没发过。
    //
    // 触发条件不只是「比上次长」：若最后一段思考在上一帧已发完（长度没变化），
    // 只判增量就永远不发 done，前端会一直停在「思考中…」。
    let r_final = acc_reason.chars().count();
    if r_final > 0 && (r_final > last_reason_len || reason_emitted) {
        hooks.on_progress(
            "reasoning",
            serde_json::json!({ "length": r_final, "text": acc_reason.clone(), "done": true }),
        );
    }
    log_call(
        call_id,
        &format!(
            "SSE 流正常结束：累计 {} 字符，思考 {} 字符，usage={}",
            acc.chars().count(),
            acc_reason.chars().count(),
            usage.is_some()
        ),
    );
    Ok((acc, acc_reason, usage))
}

/// 流式输出「继续」prompt：AI 中途断流后，把已收到的部分发回去请求续写。
/// 像在 IDE 里「中断后发送继续请求」一样，让模型从断点接着写，不重复已有内容。
pub fn resume_prompt(system: &str, user: &str, partial: &str) -> (String, String) {
    // 截尾：只回传已输出内容的首尾片段，避免超长 partial 把上下文撑爆。
    // 断点多发生在输出后半段，因此尾部保留更多。
    let chars: Vec<char> = partial.chars().collect();
    let partial_ctx = if chars.len() > 6000 {
        let head: String = chars[..2000].iter().collect();
        let tail: String = chars[chars.len() - 4000..].iter().collect();
        format!(
            "{}\n…（中间省略 {} 字符）…\n{}",
            head,
            chars.len() - 6000,
            tail
        )
    } else {
        partial.to_string()
    };
    let sys = format!(
        "{}\n\n【继续输出模式】你上一次回答因网络中断而只输出了一部分。用户消息将包含你已输出的内容片段。\n请从断点处无缝续写剩余内容：不要重复已有内容、不要重新开始、不要输出任何解释或 Markdown 围栏，直接继续写完整个 JSON。",
        system
    );
    let usr = format!(
        "{}\n\n—— 你上一次已输出的内容（截至中断处）——\n{}\n\n—— 请从上面的断点处直接继续输出剩余部分 ——",
        user, partial_ctx
    );
    (sys, usr)
}

/// 流式调用结果：完整拼接文本 + 思考全文 + 末次 usage（流式末块，需 stream_options.include_usage）。
///
/// `reasoning` 以前**没有这个字段**——思考文本在 `consume_sse` 返回处就被丢弃了，
/// 于是「思考过程」在前端只能看到字数。现在与非流式的 `CompleteOutcome` 对齐：
/// 两边都有完整的思考文本，前端不必区分走的是哪条路径。没有思考内容的模型为 None。
pub struct StreamOutcome {
    pub text: String,
    pub reasoning: Option<String>,
    pub usage: Option<serde_json::Value>,
}

/// 发送一次流式请求并消费 SSE；断流/续写重试由此函数全权负责。
/// 调用方拿到 [`StreamOutcome`] 后自行做 JSON 解析等业务收尾。
///
/// `on_stream_tick(len, acc)`：流式节流回调（约每 400 字符 / 200ms 一次）。
/// 第二个参数是**当前累积到的完整文本**（不是增量、也不是尾片段）——
/// 调用方可以据此做增量解析；只要尾预览的自己切片。传 `|_, _| {}` 即可静默。
///
/// 个别网关不识别 `stream_options.include_usage` 时会 400/422：首次响应遇到该错误
/// 自动去掉参数重试一次（之后无 usage，仅进度）。
pub async fn stream_chat_with_resume(
    hooks: &dyn ChannelHooks,
    provider: &dyn ChannelProvider,
    model: &str,
    system: &str,
    user: &str,
    temperature: f32,
    on_stream_tick: impl Fn(usize, &str) + Send + Sync,
) -> Result<StreamOutcome, String> {
    let client = ai_http_client();
    let url = completion_url(&provider.openai_url());
    let call_id = next_call_id();
    log_call(call_id, &format!(
        "═══ 流式请求提交 ═══ model={} provider={}({}) url={} temperature={}\n── system prompt ──\n{}\n── user prompt ──\n{}",
        model,
        provider.name(),
        provider.id(),
        url,
        temperature,
        log_preview(system),
        log_preview(user)
    ));

    let mut body = serde_json::json!({
        "model": model, "stream": true, "temperature": temperature,
        "stream_options": { "include_usage": true },
        "messages": [{ "role": "system", "content": system }, { "role": "user", "content": user }]
    });
    let with_usage = body.clone();

    // 闭包返回 async block 的生命周期标注问题：直接用局部 async fn 表达「借用环境
    // 中的 client/url/provider 发送一次请求」。
    async fn send(
        hooks: &dyn ChannelHooks,
        call_id: u64,
        client: &reqwest::Client,
        url: &str,
        provider: &dyn ChannelProvider,
        body: &serde_json::Value,
    ) -> Result<reqwest::Response, String> {
        send_with_retries(hooks, call_id, client, url, provider, body).await
    }

    let tick = |len: usize, tail: &str| on_stream_tick(len, tail);

    let resp = send(hooks, call_id, &client, &url, provider, &body).await?;
    let st = resp.status();
    if !st.is_success() {
        // 诊断信息：无论正文是 JSON 还是 HTML/纯文本都尽量取出来
        let text = read_body_text_lossy(call_id, "错误响应体", resp).await;
        let msg = error_or_status(&text, &st);
        // 网关不认 stream_options/include_usage → 去掉该参数重试一次（此时无 usage，仅进度）
        let unknown_param = (st.as_u16() == 400 || st.as_u16() == 422)
            && (msg.contains("stream_options")
                || msg.contains("include_usage")
                || msg.to_lowercase().contains("unknown parameter"));
        if !unknown_param {
            log_call(call_id, &format!("非 2xx 响应体：{}", log_preview(&text)));
            return Err(format!("AI错误 {}: {}", st.as_u16(), msg));
        }
        log_call(
            call_id,
            "网关不识别 stream_options.include_usage，去掉该参数重试一次",
        );
        if let Some(obj) = body.as_object_mut() {
            obj.remove("stream_options");
        }
        let resp2 = send(hooks, call_id, &client, &url, provider, &body).await?;
        let st2 = resp2.status();
        if !st2.is_success() {
            let text2 = read_body_text_lossy(call_id, "重试错误响应体", resp2).await;
            let msg2 = error_or_status(&text2, &st2);
            log_call(
                call_id,
                &format!("重试后仍非 2xx 响应体：{}", log_preview(&text2)),
            );
            return Err(format!("AI错误 {}: {}", st2.as_u16(), msg2));
        }
        let (text, reasoning, usage) = consume_sse(hooks, call_id, model, resp2, &tick).await?;
        return finish_stream_text(call_id, text, reasoning, usage);
    }
    // 思考文本与正文**分开累计**：续写只补正文，思考不能因为断流而重复一段
    let mut reasoning = String::new();
    let (mut text, first_reasoning, mut usage) =
        match consume_sse(hooks, call_id, model, resp, &tick).await {
            Ok(x) => x,
            Err(e) => {
                let err = format!("流式中断: {}", strip_prefix_of(&e, "流式中断: "));
                if !is_recoverable_network_error(&err) {
                    return Err(err);
                }
                log_call(call_id, &format!("流中断可恢复，准备断点续写：{}", err));
                (String::new(), String::new(), None)
            }
        };
    reasoning.push_str(&first_reasoning);
    for attempt in 1..=STREAM_RESUME_MAX {
        // 已拿到完整内容则不再续写
        if !text.trim().is_empty() && extract_json(&text).is_some() {
            log_call(
                call_id,
                &format!(
                    "内容已成完整 JSON，提前返回（共 {} 字符，续写 {} 次）",
                    text.chars().count(),
                    attempt - 1
                ),
            );
            return finish_stream_text(call_id, text, reasoning, usage);
        }
        // 断流且内容为空：直接重发原请求（首次请求没收到任何有效内容）
        // 断流但已有部分内容：发「继续」请求，把已收到的片段回传给模型续写
        hooks.on_progress(
            "reconnect",
            serde_json::json!({
                "attempt": attempt,
                "max": STREAM_RESUME_MAX,
                "length": text.chars().count(),
                "resume": !text.trim().is_empty(),
            }),
        );
        let (sys, usr) = if text.trim().is_empty() {
            log_call(
                call_id,
                &format!(
                    "断流且内容为空，第 {}/{} 次直接重发原请求",
                    attempt, STREAM_RESUME_MAX
                ),
            );
            (system.to_string(), user.to_string())
        } else {
            log_call(
                call_id,
                &format!(
                "断流，第 {}/{} 次发断点续写请求（已累计 {} 字符）\n── 已收到片段（截断）──\n{}",
                attempt,
                STREAM_RESUME_MAX,
                text.chars().count(),
                log_preview(&text)
            ),
            );
            resume_prompt(system, user, &text)
        };
        let mut resume_body = with_usage.clone();
        if let Some(obj) = resume_body.as_object_mut() {
            obj.insert(
                "messages".into(),
                serde_json::json!([
                    { "role": "system", "content": sys },
                    { "role": "user", "content": usr }
                ]),
            );
        }
        let before = text.chars().count();
        let outcome = async {
            let r = send(hooks, call_id, &client, &url, provider, &resume_body).await?;
            let s = r.status();
            if !s.is_success() {
                // 非 2xx：尽量读出网关的错误正文，便于日志定位（非 JSON 也不硬报错）
                let t = read_body_text_lossy(call_id, "续写错误响应体", r).await;
                log_call(
                    call_id,
                    &format!("续写请求非 2xx 响应体：{}", log_preview(&t)),
                );
                Err(format!(
                    "AI错误 {}: {}",
                    s.as_u16(),
                    error_or_status(&t, &s)
                ))
            } else {
                consume_sse(hooks, call_id, model, r, &tick).await
            }
        }
        .await;
        match outcome {
            Ok((part, part_reasoning, u)) => {
                // 续写内容直接拼接（模型被要求从断点继续，不重复已有部分）。
                if u.is_some() {
                    usage = u;
                }
                let got = part.chars().count();
                log_call(
                    call_id,
                    &format!(
                        "续写第 {} 次返回 {} 字符\n── 续写返回内容 ──\n{}",
                        attempt,
                        got,
                        log_preview(&part)
                    ),
                );
                text.push_str(&part);
                reasoning.push_str(&part_reasoning);
                // 若本次续写几乎无新增内容，说明模型在复读而非续写——判定失败。
                if got < RESUME_MIN_PROGRESS_CHARS {
                    return Err(format!(
                        "流式中断: 续写第 {} 次仅新增 {} 字符，放弃",
                        attempt, got
                    ));
                }
            }
            Err(e) => {
                let err = format!("流式中断: {}", strip_prefix_of(&e, "流式中断: "));
                log_call(
                    call_id,
                    &format!(
                        "续写第 {} 次失败：{}（已累计 {} 字符）",
                        attempt,
                        err,
                        text.chars().count()
                    ),
                );
                // 续写期间有进展则把 partial 留着再试一次；否则终止
                if text.chars().count() > before || is_recoverable_network_error(&err) {
                    continue;
                }
                return Err(err);
            }
        }
    }
    // resume 次数用尽：把现有文本原样交回（调用方可用宽容解析，可能已经完整）
    log_call(
        call_id,
        &format!(
            "续写次数用尽，将现有内容交回调用方宽容解析（共 {} 字符）",
            text.chars().count()
        ),
    );
    finish_stream_text(call_id, text, reasoning, usage)
}

/// resume 次数用尽后的收尾：记录最终输出全量（截断预览），文本原样交回
///（可能已完整或为空，由调用方宽容解析/判定），usage 随 StreamOutcome 带回——
/// 通道只负责传输，不替调用方判断内容语义。
pub(super) fn finish_stream_text(
    call_id: u64,
    text: String,
    reasoning: String,
    usage: Option<serde_json::Value>,
) -> Result<StreamOutcome, String> {
    log_call(
        call_id,
        &format!(
            "═══ 流式最终返回（{} 字符，思考 {} 字符，usage={}）═══\n{}",
            text.chars().count(),
            reasoning.chars().count(),
            usage.is_some(),
            log_preview(&text)
        ),
    );
    Ok(StreamOutcome {
        text,
        // 空串统一成 None：调用方就不用同时判 null 和空串两种「没有思考」
        reasoning: (!reasoning.trim().is_empty()).then_some(reasoning),
        usage,
    })
}
