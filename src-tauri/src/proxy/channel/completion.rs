//! Non-stream completion, tool-call and JSON response handling.
use super::{
    ai_http_client, completion_url, error_or_status, is_recoverable_network_error,
    is_retryable_no_output, log_call, log_preview, next_call_id, read_body_text_lossy,
    send_with_retries, stream_chat_with_resume, strip_prefix_of, ChannelHooks, ChannelProvider,
    EMPTY_RETRY_DELAY, EMPTY_RETRY_MAX, READ_BODY_TIMEOUT,
};

/// 提取首个平衡 JSON 对象（跳过前后缀文本 / Markdown 围栏 / 解释性文字）。
/// 通道内用于「已拿到完整内容？」判定；调用方解析可用各自的宽容实现。
pub(super) fn extract_json(input: &str) -> Option<&str> {
    let s = input.trim();
    let start = s.find('{')?;
    let bytes = s.as_bytes();
    let mut depth = 0usize;
    let mut in_str = false;
    let mut esc = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_str {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(&s[start..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// 非流式调用结果：assistant 文本 + usage（若网关返回）。
pub struct CompleteOutcome {
    pub text: String,
    pub usage: Option<serde_json::Value>,
    /// 原始 choices[0].message（tool-call 响应的 content 为空，需从这里取 tool_calls）
    pub message: Option<serde_json::Value>,
    /// 模型的推理文本（DeepSeek/Qwen 等返回 `reasoning_content`，Claude 系为 `reasoning`）。
    /// 以前整段丢弃，导致「思考过程」无从展示；没有该字段的模型为 None。
    pub reasoning: Option<String>,
}

/// 原生 tool-calling：把 tool_calls[0].function.arguments 解析为 JSON 对象。
/// OpenAI 协议保证 arguments 是「JSON 字符串」——但个别网关会发对象而非字符串，容错处理。
pub fn parse_tool_call_arguments(message: &serde_json::Value) -> Option<serde_json::Value> {
    let args = message
        .get("tool_calls")
        .and_then(|tc| tc.get(0))
        .and_then(|tc| tc.get("function"))
        .and_then(|f| f.get("arguments"))?;
    match args {
        serde_json::Value::String(s) => {
            let v = serde_json::from_str::<serde_json::Value>(s.trim()).ok();
            if v.is_none() {
                crate::exit_log!(
                    "[ai-channel] tool_call arguments 不是合法 JSON（len={}）",
                    s.chars().count()
                );
            }
            v
        }
        v @ serde_json::Value::Object(_) => Some(v.clone()),
        _ => None,
    }
}

/// 探索点单工具的 JSON Schema（OpenAI tools 格式）。
/// 与文本协议字段一一对应（paths/dirs/done/reason），方便网关不支持 tools 时无缝降级。
/// 注意：文件数量上限由调用方通过 system prompt（explorer_prompt）告知模型，
/// 此处不写死数字——避免与可调的全局设置（explorer_files_per_round）矛盾。
pub const EXPLORER_TOOL_SPEC: &str = r##"[
  {
    "type": "function",
    "function": {
      "name": "request_files",
      "description": "请求读取项目中的一批文件以继续架构分析。每轮文件数上限以系统提示为准。",
      "parameters": {
        "type": "object",
        "properties": {
          "paths": {
            "type": "array",
            "items": { "type": "string" },
            "description": "要读取的文件（项目相对路径，必须是『目录结构』中真实存在的文件）"
          },
          "dirs": {
            "type": "array",
            "items": { "type": "string" },
            "description": "请求确认存在的目录（不读取内容）"
          },
          "done": {
            "type": "boolean",
            "description": "true 表示已读够，可以停止探索并生成导图"
          },
          "reason": {
            "type": "string",
            "description": "一句话说明为什么读这些文件"
          }
        },
        "required": ["paths", "done"]
      }
    }
  }
]"##;

/// 判断 tools 参数不被网关识别的错误（400/422 + 关键词），命中则去掉 tools 降级重试。
/// 输入为通道错误全文（形如 `AI错误 400: <网关消息>`），解析失败时保守返回 false
/// （宁可多试一次也不把业务错误当降级处理）。
pub(super) fn is_tools_unsupported_error_text(err: &str) -> bool {
    let rest = match err.strip_prefix("AI错误 ") {
        Some(r) => r,
        None => return false,
    };
    let (code, msg) = match rest.split_once(": ") {
        Some((c, m)) => (c, m),
        None => (rest, ""),
    };
    if code != "400" && code != "422" {
        return false;
    }
    let m = msg.to_lowercase();
    m.contains("tool")
        || m.contains("function calling")
        || m.contains("unknown parameter")
        || m.contains("unrecognized request argument")
}

/// 非流式 JSON 对象请求（带原生 tool-calling 与自动降级）：优先以 tools 方式强制
/// 网关输出结构化 JSON（比「文本里夹 JSON」解析成功率高得多）；网关不识别 tools
/// 参数时自动去掉降级为纯文本协议（调用方的 system prompt 已包含输出格式约定，
/// 因此降级路径无需任何额外处理）。解析成功后再校验 tools 字段是否存在，
/// 以避免每次都先吃一次 400。
///
/// `tools_json`：OpenAI tools 数组 JSON（EXPLORER_TOOL_SPEC）；空串 = 不带 tools。
pub async fn complete_chat_json(
    hooks: &dyn ChannelHooks,
    provider: &dyn ChannelProvider,
    model: &str,
    system: &str,
    user: &str,
    temperature: f32,
    tools_json: &str,
    tool_id: &str,
) -> Result<(serde_json::Value, Option<serde_json::Value>), String> {
    let should_try_tools = !tools_json.trim().is_empty();
    let mut attempt = 0usize; // 0 = 尝试 tools（若启用），1 = 降级纯文本
    loop {
        let use_tools = should_try_tools && attempt == 0;
        let outcome = complete_chat(
            hooks,
            provider,
            model,
            system,
            user,
            temperature,
            if use_tools { Some(tools_json) } else { None },
            tool_id,
        )
        .await;
        let outcome = match outcome {
            Ok(o) => o,
            Err(e) => {
                // 降级判定：只对 tools 引起的形式错误降级（400/422 + tools 关键词），
                // 业务错误（如上下文超限）原样返回，不浪费一次无意义的重试
                if use_tools && attempt == 0 && is_tools_unsupported_error_text(&e) {
                    crate::exit_log!(
                        "[ai-channel] tools 参数被网关拒绝（{}），降级为纯文本 JSON 协议重试",
                        e
                    );
                    attempt = 1;
                    continue;
                }
                return Err(e);
            }
        };
        // 从响应中提取 JSON：优先 tool_calls.arguments（原生协议），
        // 其次 choices[0].message.content（文本协议，兼容 markdown 围栏/前后缀）
        let extracted: Option<(serde_json::Value, bool)> = match outcome.message.as_ref() {
            Some(msg) if msg.get("tool_calls").is_some() => match parse_tool_call_arguments(msg) {
                Some(v) => Some((v, true)),
                None => {
                    crate::exit_log!(
                        "[ai-channel] tool_call arguments 解析失败，尝试 content 文本协议"
                    );
                    None
                }
            },
            _ => None,
        };
        let (json, used_tool_call) = match extracted {
            Some(x) => x,
            None => match parse_json(&outcome.text) {
                Ok(v) => (v, false),
                Err(e) => {
                    // 首轮（带 tools）解析失败：部分网关接受了 tools 却把输出拼进
                    // content 且加了围栏——宽容解析已处理围栏，仍失败说明网关不支持，
                    // 降级重试一次。
                    if use_tools && attempt == 0 {
                        crate::exit_log!(
                            "[ai-channel] tools 响应 content 无法解析为 JSON（{}），降级为纯文本重试",
                            e
                        );
                        attempt = 1;
                        continue;
                    }
                    return Err(e);
                }
            },
        };
        crate::exit_log!(
            "[ai-channel] JSON 提取成功（{}，{} 字符）",
            if used_tool_call {
                "原生 tool_call"
            } else {
                "文本协议"
            },
            json.to_string().chars().count()
        );
        return Ok((json, outcome.usage));
    }
}

/// 从损坏的 JSON 文本中定位错误位置：行/列 + 附近上下文（UTF-8 边界安全）。
pub fn json_error_context(slice: &str, err: &serde_json::Error) -> String {
    let line = err.line();
    let col = err.column();
    // 定位到错误所在行的起始字节
    let mut line_start = 0usize;
    for _ in 1..line.max(1) {
        match slice[line_start..].find('\n') {
            Some(i) => line_start += i + 1,
            None => break,
        }
    }
    let pos = line_start.saturating_add(col.saturating_sub(1));
    let from = pos.saturating_sub(60);
    let to = (pos + 80).min(slice.len());
    // serde 的列按字节计，需要对齐到 UTF-8 字符边界再切片
    let from = {
        let mut i = from.min(slice.len());
        while i > 0 && !slice.is_char_boundary(i) {
            i -= 1;
        }
        i
    };
    let to = {
        let mut i = to.min(slice.len());
        while i < slice.len() && !slice.is_char_boundary(i) {
            i += 1;
        }
        i
    };
    let win: String = slice[from..to].chars().collect();
    format!(
        "{}（第 {} 行第 {} 列，附近上下文：…{}…）",
        err, line, col, win
    )
}

/// 宽容修复：把字符串里的非法转义序列（如 Windows 路径 C:\Users、Markdown 残留反斜杠）
/// 转义为字面反斜杠。LLM 输出的 JSON 损坏最常见的就是这一类。
/// 仅当确实发生改动时返回 Some(修复后文本)。
pub fn repair_invalid_escapes(s: &str) -> Option<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len() + 8);
    let mut in_string = false;
    let mut escaped = false;
    let mut changed = false;
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if escaped {
            // 上一个字节是字符串内的反斜杠：当前字符必须构成合法 JSON 转义
            let valid = matches!(c, '"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't' | 'u');
            if !valid {
                out.push('\\');
                changed = true;
            }
            if c == 'u' && valid {
                // \uXXXX 需后随 4 个十六进制字符；不是则按字面转义（如 \users、\u{...}）
                let hex_ok = i + 4 < chars.len()
                    && chars[i + 1..i + 5].iter().all(|x| x.is_ascii_hexdigit());
                if !hex_ok {
                    out.push('\\');
                    changed = true;
                }
            }
            out.push(c);
            escaped = false;
            i += 1;
            continue;
        }
        match c {
            '"' => {
                in_string = !in_string;
                out.push(c);
            }
            '\\' if in_string => {
                escaped = true;
                out.push(c);
            }
            _ => out.push(c),
        }
        i += 1;
    }
    if escaped {
        out.push('\\'); // 尾部孤立反斜杠（截断/损坏），补一层转义让解析通过
        changed = true;
    }
    changed.then_some(out)
}

/// 宽容 JSON 解析：从 LLM 输出文本中提取 JSON 对象（容忍 markdown 围栏、前后缀、
/// Windows 路径类非法转义）。供 tool-call 降级路径与各调用方复用。
pub fn parse_json(text: &str) -> Result<serde_json::Value, String> {
    let t = text.trim();
    let s = t.find('{').ok_or("无 JSON")?;
    let e = t.rfind('}').ok_or("无 JSON")?;
    let slice = &t[s..=e];
    match serde_json::from_str(slice) {
        Ok(v) => Ok(v),
        Err(err) => {
            let ctx = json_error_context(slice, &err);
            // 自动修复非法转义后重试（成功时记录日志，不再当错误抛出）
            if let Some(patched) = repair_invalid_escapes(slice) {
                if let Ok(v) = serde_json::from_str(&patched) {
                    crate::exit_log!("[ai-channel] AI JSON 非法转义已自动修复，原始错误: {}", ctx);
                    return Ok(v);
                }
            }
            Err(format!("JSON: {}", ctx))
        }
    }
}

/// 一次性非流式调用：TTFB 语义的 send 超时 + send 重试仍然生效（连接阶段韧性），
/// 但拿到响应头后整体读完响应体——这里补一个读体超时防止服务端挂住连接。
///
/// 与流式不同：非流式没有断点续写能力（拿不到部分输出），重试意味着完整重新计费，
/// 因此收到 5xx/网络错误时不自动重试，由调用方决定（大多数场景一次失败重试一次足够）。
///
/// `tool_id`：谁在调用（落进 `ai_usage.tool_id`，取值见 [`usage::tool_ids`]）。
/// 必填 —— 记账下沉到通道后，漏传就编译不过，不可能再出现「某个 Agent 没被统计」。
pub async fn complete_chat(
    hooks: &dyn ChannelHooks,
    provider: &dyn ChannelProvider,
    model: &str,
    system: &str,
    user: &str,
    temperature: f32,
    tools_json: Option<&str>,
    tool_id: &str,
) -> Result<CompleteOutcome, String> {
    complete_chat_messages(
        hooks,
        provider,
        model,
        &[
            serde_json::json!({ "role": "system", "content": system }),
            serde_json::json!({ "role": "user", "content": user }),
        ],
        temperature,
        tools_json,
        tool_id,
    )
    .await
}

/// 多轮消息版非流式调用（[`complete_chat`] 的泛化形态）。
///
/// Agent 工具循环需要把 assistant(tool_calls) 与 tool(result) 消息回传给网关继续
/// 推理，单轮 (system, user) 签名表达不了这种对话形态，故抽出本函数；
/// 传输韧性（TTFB 超时 / send 重试 / 空响应与限流处理）与单轮版完全一致。
///
/// **所有直连调用的收口点**：拿到 usage 后由这里统一落库（含耗时），
/// 调用方不必、也不应再自己记一笔 —— 记在自己那边迟早会漏（收藏 Agent、
/// 安装助手 Agent 都漏过）。`tool_id` 取值见 [`usage::tool_ids`]。
pub async fn complete_chat_messages(
    hooks: &dyn ChannelHooks,
    provider: &dyn ChannelProvider,
    model: &str,
    messages: &[serde_json::Value],
    temperature: f32,
    tools_json: Option<&str>,
    tool_id: &str,
) -> Result<CompleteOutcome, String> {
    let started = std::time::Instant::now();
    let outcome =
        complete_chat_messages_inner(hooks, provider, model, messages, temperature, tools_json)
            .await?;
    // 统一记账：直连也测得出耗时，记上后这些记录才参与 t/s 与成功率聚合
    if let Some(u) = &outcome.usage {
        let input_tokens = u
            .get("prompt_tokens")
            .or_else(|| u.get("input_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let output_tokens = u
            .get("completion_tokens")
            .or_else(|| u.get("output_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let (cache_read_tokens, cache_write_tokens) =
            crate::proxy::metering::cache_tokens_from_json(u);
        if input_tokens != 0 || output_tokens != 0 || cache_read_tokens != 0 {
            crate::proxy::hooks::current().record_usage(crate::proxy::hooks::UsageRecord {
                tool_id,
                model,
                provider_id: provider.id(),
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_write_tokens,
                duration_ms: started.elapsed().as_millis() as u64,
                first_token_ms: 0,
            });
        }
    }
    Ok(outcome)
}

/// [`complete_chat_messages`] 的传输实现（不含记账）。
async fn complete_chat_messages_inner(
    hooks: &dyn ChannelHooks,
    provider: &dyn ChannelProvider,
    model: &str,
    messages: &[serde_json::Value],
    temperature: f32,
    tools_json: Option<&str>,
) -> Result<CompleteOutcome, String> {
    let client = ai_http_client();
    let url = completion_url(&provider.openai_url());
    let call_id = next_call_id();
    log_call(call_id, &format!(
        "═══ 非流式请求提交 ═══ model={} provider={}({}) url={} temperature={} messages={}\n── 末条消息 ──\n{}",
        model,
        provider.name(),
        provider.id(),
        url,
        temperature,
        messages.len(),
        log_preview(messages.last().map(|m| m.to_string()).unwrap_or_default().as_str())
    ));
    let mut body = serde_json::json!({
        "model": model,
        "messages": messages,
        "stream": false,
        "temperature": temperature,
    });
    // 原生 tool-calling：网关支持时携带 tools。tool_choice 策略：spec 只有一个工具时
    // 强制点名（探索场景每轮必须点单，防止模型「自己决定」要不要调）；多工具（Agent
    // 循环）用 auto，由模型自行决定调哪个 / 是否直接回答。
    if let Some(spec) = tools_json {
        if let Some(arr) = serde_json::from_str::<serde_json::Value>(spec)
            .ok()
            .filter(|v| v.is_array())
        {
            let single = arr.as_array().map(|a| a.len() == 1).unwrap_or(false);
            // 先取出（可能的）强制点名目标，再移交 arr 的所有权
            let forced_name = if single {
                arr.get(0)
                    .and_then(|t| t.get("function"))
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string())
            } else {
                None
            };
            if let Some(obj) = body.as_object_mut() {
                obj.insert("tools".into(), arr);
                if let Some(name) = forced_name {
                    obj.insert(
                        "tool_choice".into(),
                        serde_json::json!({ "type": "function", "function": { "name": name } }),
                    );
                } else {
                    obj.insert("tool_choice".into(), serde_json::json!("auto"));
                }
            }
        }
    }

    // 非流式「未产出内容」自动重试：网关空壳（200 + 空 choices）与 429 限流都不产生 token，
    // 重试零计费风险，而实测这类失败多为服务端瞬时状态（同一请求重发即可成功）。
    let mut empty_retry = 0usize;
    let (value, message, text, has_tool_calls) = loop {
        let resp = send_with_retries(hooks, call_id, &client, &url, provider, &body).await?;
        let status = resp.status();
        let raw = read_body_text_lossy(call_id, "非流式响应", resp).await;
        if raw.is_empty() {
            // 空响应体：错误状态码下的空体（网关限流/故障常见）按 HTTP 错误处理并允许 429 重试；
            // 只有在 2xx 时才可能是「读体超时」（状态码在消费响应体前已取出，仍然可用）。
            if !status.is_success() {
                if status.as_u16() == 429 && empty_retry < EMPTY_RETRY_MAX {
                    empty_retry += 1;
                    hooks.check_cancel()?;
                    log_call(
                        call_id,
                        &format!(
                            "服务端限流（HTTP 429 且响应体为空），第 {}/{} 次重试",
                            empty_retry, EMPTY_RETRY_MAX
                        ),
                    );
                    tokio::time::sleep(EMPTY_RETRY_DELAY * empty_retry as u32).await;
                    continue;
                }
                return Err(format!("AI错误 {}: 响应体为空", status.as_u16()));
            }
            return Err(format!(
                "AI请求无响应: 读取响应体超时（超过 {}s）",
                READ_BODY_TIMEOUT.as_secs()
            ));
        }
        log_call(
            call_id,
            &format!(
                "═══ 非流式响应（{}）共 {} 字符 ═══\n{}",
                status.as_u16(),
                raw.chars().count(),
                log_preview(&raw)
            ),
        );
        // 响应体可能不是 JSON（网关限流/故障时给空体或 HTML）——先宽容解析，
        // 不能让「解析失败」掩盖真实的 HTTP 状态与原始正文。
        let parsed = serde_json::from_str::<serde_json::Value>(&raw).ok();
        let message = parsed
            .as_ref()
            .and_then(|v| v.get("choices"))
            .and_then(|c| c.get(0))
            .and_then(|ch| ch.get("message"))
            .cloned();
        let text = message
            .as_ref()
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        // tool-call 响应的 content 合法为空：只要带了 tool_calls 就不算「AI返回空」。
        let has_tool_calls = message
            .as_ref()
            .map(|m| {
                m.get("tool_calls")
                    .and_then(|tc| tc.as_array())
                    .map(|a| !a.is_empty())
                    .unwrap_or(false)
            })
            .unwrap_or(false);

        if is_retryable_no_output(status.as_u16(), text.is_empty(), has_tool_calls)
            && empty_retry < EMPTY_RETRY_MAX
        {
            empty_retry += 1;
            hooks.check_cancel()?;
            log_call(
                call_id,
                &format!(
                    "服务端未产出内容（HTTP {}），第 {}/{} 次重试",
                    status.as_u16(),
                    empty_retry,
                    EMPTY_RETRY_MAX
                ),
            );
            tokio::time::sleep(EMPTY_RETRY_DELAY * empty_retry as u32).await;
            continue;
        }
        if !status.is_success() {
            return Err(format!(
                "AI错误 {}: {}",
                status.as_u16(),
                error_or_status(&raw, &status)
            ));
        }
        match parsed {
            Some(v) => break (v, message, text, has_tool_calls),
            None => {
                log_call(call_id, "响应 JSON 解析失败：响应体不是合法 JSON");
                return Err("解析响应失败: 响应体不是合法 JSON".to_string());
            }
        }
    };
    if text.is_empty() && !has_tool_calls {
        log_call(
            call_id,
            &format!(
                "AI返回空（连续 {} 次尝试 choices[0].message.content 均为空）",
                EMPTY_RETRY_MAX + 1
            ),
        );
        return Err(format!(
            "AI返回空：连续 {} 次请求均未返回内容（服务端 HTTP 200 但 choices 为空，通常是该模型在服务商侧无可用实例或被限流），请稍后重试或更换模型",
            EMPTY_RETRY_MAX + 1
        ));
    }
    if has_tool_calls {
        log_call(
            call_id,
            &format!(
                "═══ 非流式 tool_call 响应（{} 个调用）═══\n{}",
                message
                    .as_ref()
                    .and_then(|m| m.get("tool_calls"))
                    .and_then(|tc| tc.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0),
                log_preview(&message.as_ref().map(|m| m.to_string()).unwrap_or_default())
            ),
        );
    }
    log_call(
        call_id,
        &format!(
            "═══ 非流式最终输出（{} 字符，usage={}）═══\n{}",
            text.chars().count(),
            value.get("usage").is_some(),
            log_preview(&text)
        ),
    );
    // 推理文本在循环体内解析（每次重试都可能带），这里从最终 message 再取一次即可
    let reasoning = message
        .as_ref()
        .and_then(|m| {
            ["reasoning_content", "reasoning", "thinking"]
                .iter()
                .find_map(|k| m.get(*k).and_then(|v| v.as_str()))
        })
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    Ok(CompleteOutcome {
        text,
        usage: value.get("usage").cloned(),
        message,
        reasoning,
    })
}
