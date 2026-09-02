//! 共享 AI 请求通道：面向所有 chat/completions 调用方（思维导图 AI、翻译、API 智能导入…）。
//!
//! 把「思维导图后端」沉淀的三道防线统一收编，任何 AI 调用方零成本获得同样的韧性：
//!
//! 1. **TTFB 超时**：`tokio::time::timeout` 只包住 `send()`（拿到响应头为止），
//!    响应体（SSE 流）读取不受影响——不能使用 `RequestBuilder::timeout`，
//!    reqwest 0.12 里它是「整个请求（含响应体）」的硬上限，会把流式几分钟的
//!    大请求中途掐断（外在表现即「error sending request / error decoding response body」）。
//! 2. **send 重试**：连接/TLS/代理拒绝/等待响应头超时类失败，退避后自动重试。
//! 3. **断点续写（仅流式）**：流中途断开（网络抖动/网关断开/读空闲超时）时不直接失败，
//!    携带已收到的部分发「继续」请求让模型从断点接着写，最多 STREAM_RESUME_MAX 次且需有实际进展。
//!
//! 进度上报与取消检查通过 [`ChannelHooks`] 注入：思维导图接到 `mm-ai-progress`
//! 事件流 + run_id 取消标志；翻译/API 导入等无进度 UI 的调用方用 [`NoHooks`]。
//! 通道本身不关心事件名与业务语义，保持对调用方的零侵入。

use crate::commands::ai::models::AiProvider;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// 「拿到响应头」的最长等待时间（TTFB）：连接 + 网关转发 + 模型排队出首字节的预算。
/// 长上下文（整库扫描）+ 推理型模型首字节可能超过一分钟，因此给足余量。
pub const REQUEST_TTFB_TIMEOUT: Duration = Duration::from_secs(180);
/// TCP/TLS 连接阶段单独限时：死端点/被防火墙丢弃的地址不必等满 TTFB 才报错。
pub const REQUEST_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// send 阶段（连接/等待响应头）失败后的重试次数与退避：缓解代理/网关瞬时不稳定。
pub const SEND_RETRY_MAX: usize = 2;
pub const SEND_RETRY_DELAY: Duration = Duration::from_secs(2);
/// SSE 流读取空闲超时：超过该时长没有收到任何字节视为断流（正常流式间隔远小于此）。
pub const SSE_IDLE_TIMEOUT: Duration = Duration::from_secs(75);
/// 非流式调用读取响应正文的超时：拿到响应头后服务端可能挂住连接不发正文，
/// 必须兜底防止 complete_chat 永久挂起（非流式响应都很小，60s 足够）。
pub const READ_BODY_TIMEOUT: Duration = Duration::from_secs(60);
/// 断点续写最大重试次数（每次续写先校验已累积内容是否已成完整 JSON，成功则提前返回，
/// 因此多数情况一次续写即可完成）。
pub const STREAM_RESUME_MAX: usize = 3;
/// 续写内容最少新增字符数：低于该值视为模型在复读而非续写，判定失败。
const RESUME_MIN_PROGRESS_CHARS: usize = 32;

// ─── 请求/响应详细日志（exit.log，逐行即时落盘） ───
//
// 每次通道调用分配自增 call_id，串起「提交了什么 → 每次尝试 → 响应头耗时 →
// 返回了什么 → 续写 → 最终输出」，便于事后定位「AI 输出异常/断流/网关错误」。
// 安全约定：只记请求体与响应体（不含 Authorization 头，密钥永不落盘）。

/// 日志单条字段预览上限（字符）：超过则保留首尾 + 省略标记（坏 JSON/断流问题
/// 多出现在开头或结尾，头尾保留诊断价值最高）。
const LOG_PREVIEW_CAP: usize = 40_000;

static CALL_SEQ: AtomicU64 = AtomicU64::new(0);

fn next_call_id() -> u64 {
    CALL_SEQ.fetch_add(1, Ordering::Relaxed) + 1
}

/// 通道日志统一前缀：[ai-channel][#call_id]。
fn log_call(call_id: u64, msg: &str) {
    crate::exit_log!("[ai-channel][#{}] {}", call_id, msg);
}

/// 长文本日志预览：超长时保留首尾并标注省略量与原始长度。
pub fn log_preview(s: &str) -> String {
    let n = s.chars().count();
    if n <= LOG_PREVIEW_CAP {
        return s.to_string();
    }
    let head: String = s.chars().take(30_000).collect();
    let tail: String = s.chars().skip(n - 10_000).collect();
    format!(
        "{}\n…【日志截断：中间省略 {} 字符，全文共 {} 字符】…\n{}",
        head,
        n - 40_000,
        n,
        tail
    )
}

/// 通道钩子：进度上报 + 取消检查。由调用方注入，通道自身不绑定任何事件名/业务语义。
/// 跨 await 持有，需要 Send + Sync（Tauri 命令的 future 必须 Send）。
pub trait ChannelHooks: Send + Sync {
    /// 请求过程中的阶段性进度（step 由调用方语义决定，通道发出：
    /// `reconnect`（send 重试 / 断点续写）、`stream`（流式增量）、`cancel`）。
    fn on_progress(&self, step: &str, extra: serde_json::Value);
    /// 返回 Err 即取消运行（通道在每次发送前与每个流块后检查）。
    fn check_cancel(&self) -> Result<(), String>;
}

/// 无钩子实现：不推送进度、不可取消（翻译、API 智能导入等无进度 UI 的调用方）。
pub struct NoHooks;

impl ChannelHooks for NoHooks {
    fn on_progress(&self, _step: &str, _extra: serde_json::Value) {}
    fn check_cancel(&self) -> Result<(), String> {
        Ok(())
    }
}

/// AI 请求复用客户端：连接阶段（TCP/TLS）独立限时，避免死端点占满整个 TTFB 预算。
pub fn ai_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(REQUEST_CONNECT_TIMEOUT)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// 由供应商的 OpenAI 端点推出 chat/completions 完整地址
/// （兼容 `https://host`、`https://host/v1`、尾部斜杠等写法）。
pub fn completion_url(base: &str) -> String {
    let t = base.trim_end_matches('/');
    if t.ends_with("/v1") {
        format!("{}/chat/completions", t)
    } else {
        format!("{}/v1/chat/completions", t)
    }
}

/// 从 JSON 错误体取 error.message；「?」视为缺失。
fn api_error_message(val: &serde_json::Value) -> String {
    val.get("error")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .unwrap_or("?")
        .to_string()
}

/// 读取非 2xx 响应体里的错误信息：JSON 取 error.message，
/// 非 JSON（网关返回 HTML/纯文本）则取原文片段。「读错误正文」失败不产生误导性硬错误。
pub fn extract_api_error_body(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
        let msg = api_error_message(&v);
        if !msg.is_empty() && msg != "?" {
            return msg;
        }
    }
    trimmed.chars().take(300).collect()
}

/// 带超时读取响应体文本（拿到响应头后服务端挂住不发正文的兜底）。
/// 超时/读失败返回空串——错误分支用 error_or_status 时自然回退为状态文本，
/// 并在日志中留下记录。成功路径不要用本函数（需要区分超时与空体）。
async fn read_body_text_lossy(call_id: u64, label: &str, resp: reqwest::Response) -> String {
    match tokio::time::timeout(READ_BODY_TIMEOUT, resp.text()).await {
        Ok(t) => t.unwrap_or_default(),
        Err(_) => {
            log_call(call_id, &format!("读响应体超时（{}）：超过 {}s 未收到正文", label, READ_BODY_TIMEOUT.as_secs()));
            String::new()
        }
    }
}

/// 非空错误正文，否则回退为 HTTP 状态文本。
fn error_or_status(text: &str, status: &reqwest::StatusCode) -> String {
    let detail = extract_api_error_body(text);
    if detail.is_empty() {
        status.to_string()
    } else {
        detail
    }
}

/// 网络类可恢复错误判定：连接/解码/超时类失败允许重试（send 重试 / resume）；
/// AI 主动返回的错误（鉴权、参数、内容策略等业务错误）不重试。
pub fn is_recoverable_network_error(msg: &str) -> bool {
    let m = msg.to_lowercase();
    m.contains("流式中断")
        || m.contains("error decoding response body")
        || m.contains("请求失败")
        || m.contains("请求超时")
        || m.contains("connection")
        || m.contains("timed out")
        || m.contains("timeout")
        || m.contains("reset by peer")
        || m.contains("broken pipe")
        || m.contains("eos")
        || m.contains("incomplete message")
}

/// 错误消息若以指定前缀开头则去掉（避免「流式中断: 流式中断: …」嵌套）。
pub fn strip_prefix_of(msg: &str, prefix: &str) -> String {
    msg.strip_prefix(prefix).unwrap_or(msg).to_string()
}

/// 发送 chat/completions 请求，只限制「拿到响应头」的等待时间（TTFB）。
/// send 阶段失败退避重试（最多 SEND_RETRY_MAX 次），每次重试前检查取消并推送
/// step=reconnect（send=true）进度。响应体（SSE 流）的读取不受本函数影响。
async fn send_with_retries(
    hooks: &dyn ChannelHooks,
    call_id: u64,
    client: &reqwest::Client,
    url: &str,
    provider: &AiProvider,
    body: &serde_json::Value,
) -> Result<reqwest::Response, String> {
    let mut last_err = String::new();
    for attempt in 0..=SEND_RETRY_MAX {
        hooks.check_cancel()?;
        if attempt > 0 {
            log_call(call_id, &format!("send 重试 {}/{}：上次错误：{}", attempt, SEND_RETRY_MAX, last_err));
            tokio::time::sleep(SEND_RETRY_DELAY).await;
            hooks.on_progress(
                "reconnect",
                serde_json::json!({
                    "attempt": attempt,
                    "max": SEND_RETRY_MAX,
                    "send": true,
                    "detail": last_err,
                }),
            );
        }
        log_call(call_id, &format!(
            "POST {}（第 {} 次尝试）提交 body：\n{}",
            url,
            attempt + 1,
            log_preview(&serde_json::to_string_pretty(body).unwrap_or_default())
        ));
        let started = std::time::Instant::now();
        let req = client
            .post(url)
            .header("Authorization", format!("Bearer {}", provider.api_key))
            .header("Content-Type", "application/json")
            .json(body);
        match tokio::time::timeout(REQUEST_TTFB_TIMEOUT, req.send()).await {
            Ok(Ok(resp)) => {
                let st = resp.status();
                log_call(call_id, &format!("响应头 {}：耗时 {}ms", st.as_u16(), started.elapsed().as_millis()));
                return Ok(resp);
            }
            Ok(Err(e)) => {
                last_err = format!("请求失败: {}", e);
                log_call(call_id, &format!("send 失败（{}ms）：{}", started.elapsed().as_millis(), last_err));
            }
            Err(_) => {
                last_err = format!("请求超时: 等待响应头超过 {}s", REQUEST_TTFB_TIMEOUT.as_secs());
                log_call(call_id, &format!("send 失败（{}ms）：{}", started.elapsed().as_millis(), last_err));
            }
        }
    }
    log_call(call_id, &format!("send 最终失败（共 {} 次尝试）：{}", SEND_RETRY_MAX + 1, last_err));
    Err(last_err)
}

/// 解析一行 SSE data 载荷：累积 delta.content、记录 usage、识别 error。
fn process_sse_line(line: &str, acc: &mut String, usage: &mut Option<serde_json::Value>) -> Result<(), String> {
    let data = line.strip_prefix("data:").map(str::trim).unwrap_or("");
    if data.is_empty() || data == "[DONE]" {
        return Ok(());
    }
    let v: serde_json::Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(_) => return Ok(()), // 非 JSON 行（ping/event 等）忽略
    };
    if let Some(err) = v.get("error") {
        let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("流式响应中的错误");
        return Err(format!("AI错误: {}", msg));
    }
    if let Some(d) = v.pointer("/choices/0/delta/content").and_then(|c| c.as_str()) {
        acc.push_str(d);
    }
    if let Some(u) = v.get("usage") {
        *usage = Some(u.clone());
    }
    Ok(())
}

/// 消费一次 SSE 流：逐块读取（带空闲超时，防止断流后永久挂起），跨包拼行、
/// 累积 content；逐块检查取消（用户点「停止」后流式请求随即中断）。
/// `model` 仅用于错误信息定位。返回 (完整文本, usage)，读流失败返回 Err。
async fn consume_sse(
    hooks: &dyn ChannelHooks,
    call_id: u64,
    model: &str,
    resp: reqwest::Response,
    on_stream_tick: &(dyn Fn(usize, &str) + Send + Sync),
) -> Result<(String, Option<serde_json::Value>), String> {
    use futures_util::StreamExt;
    log_call(call_id, "开始读取 SSE 流…");
    let mut acc = String::new();
    let mut usage: Option<serde_json::Value> = None;
    let mut line_buf = String::new();
    let mut last_emit = std::time::Instant::now();
    let mut last_len = 0usize;
    let mut stream = resp.bytes_stream();
    loop {
        let chunk = match tokio::time::timeout(SSE_IDLE_TIMEOUT, stream.next()).await {
            Err(_) => {
                let e = format!(
                    "流式中断: 读流空闲超时（超过 {}s 未收到数据，model={}）",
                    SSE_IDLE_TIMEOUT.as_secs(),
                    model
                );
                log_call(call_id, &format!("SSE 流错误：{}（已累计 {} 字符）", e, acc.chars().count()));
                return Err(e);
            }
            Ok(None) => break, // 正常流结束
            Ok(Some(item)) => {
                hooks.check_cancel()?;
                match item {
                    Ok(c) => c,
                    Err(e) => {
                        let e = format!("流式中断(model={}): {}", model, e);
                        log_call(call_id, &format!("SSE 流错误：{}（已累计 {} 字符）", e, acc.chars().count()));
                        return Err(e);
                    }
                }
            }
        };
        line_buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(pos) = line_buf.find('\n') {
            let line = line_buf[..pos].trim().to_string();
            line_buf.drain(..=pos);
            process_sse_line(&line, &mut acc, &mut usage).map_err(|e| {
                log_call(call_id, &format!("SSE data 行报错：{}（行内容：{}）", e, line.chars().take(500).collect::<String>()));
                e
            })?;
        }
        // 节流：每积累约 400 字符或 200ms 上报一次，保持实时又不刷屏
        let cur = acc.chars().count();
        if cur.saturating_sub(last_len) >= 400 || last_emit.elapsed().as_millis() >= 200 {
            last_emit = std::time::Instant::now();
            last_len = cur;
            let tail: String = acc.chars().rev().take(120).collect::<Vec<_>>().into_iter().rev().collect();
            on_stream_tick(cur, &tail);
        }
    }
    // 收尾：缓冲区里剩余的最后一行（可能没有换行符结尾）
    if !line_buf.trim().is_empty() {
        process_sse_line(line_buf.trim(), &mut acc, &mut usage).map_err(|e| {
            log_call(call_id, &format!("SSE data 行报错：{}（行内容：{}）", e, line_buf.trim().chars().take(500).collect::<String>()));
            e
        })?;
    }
    log_call(call_id, &format!("SSE 流正常结束：累计 {} 字符，usage={}", acc.chars().count(), usage.is_some()));
    Ok((acc, usage))
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
        format!("{}\n…（中间省略 {} 字符）…\n{}", head, chars.len() - 6000, tail)
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

/// 流式调用结果：完整拼接文本 + 末次 usage（流式末块，需 stream_options.include_usage）。
pub struct StreamOutcome {
    pub text: String,
    pub usage: Option<serde_json::Value>,
}

/// 发送一次流式请求并消费 SSE；断流/续写重试由此函数全权负责。
/// 调用方拿到 [`StreamOutcome`] 后自行做 JSON 解析等业务收尾。
///
/// `on_stream_tick(len, tail)`：流式增量节流回调（约每 400 字符 / 200ms 一次），
/// 传 `|_, _| {}` 即可静默。
///
/// 个别网关不识别 `stream_options.include_usage` 时会 400/422：首次响应遇到该错误
/// 自动去掉参数重试一次（之后无 usage，仅进度）。
pub async fn stream_chat_with_resume(
    hooks: &dyn ChannelHooks,
    provider: &AiProvider,
    model: &str,
    system: &str,
    user: &str,
    temperature: f32,
    on_stream_tick: impl Fn(usize, &str) + Send + Sync,
) -> Result<StreamOutcome, String> {
    let client = ai_http_client();
    let url = completion_url(&provider.openai_url);
    let call_id = next_call_id();
    log_call(call_id, &format!(
        "═══ 流式请求提交 ═══ model={} provider={}({}) url={} temperature={}\n── system prompt ──\n{}\n── user prompt ──\n{}",
        model,
        provider.name,
        provider.id,
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
        provider: &AiProvider,
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
        log_call(call_id, "网关不识别 stream_options.include_usage，去掉该参数重试一次");
        if let Some(obj) = body.as_object_mut() {
            obj.remove("stream_options");
        }
        let resp2 = send(hooks, call_id, &client, &url, provider, &body).await?;
        let st2 = resp2.status();
        if !st2.is_success() {
            let text2 = read_body_text_lossy(call_id, "重试错误响应体", resp2).await;
            let msg2 = error_or_status(&text2, &st2);
            log_call(call_id, &format!("重试后仍非 2xx 响应体：{}", log_preview(&text2)));
            return Err(format!("AI错误 {}: {}", st2.as_u16(), msg2));
        }
        let (text, usage) = consume_sse(hooks, call_id, model, resp2, &tick).await?;
        return finish_stream_text(call_id, text, usage);
    }
    let (mut text, mut usage) = match consume_sse(hooks, call_id, model, resp, &tick).await {
        Ok(x) => x,
        Err(e) => {
            let err = format!("流式中断: {}", strip_prefix_of(&e, "流式中断: "));
            if !is_recoverable_network_error(&err) {
                return Err(err);
            }
            log_call(call_id, &format!("流中断可恢复，准备断点续写：{}", err));
            (String::new(), None)
        }
    };
    for attempt in 1..=STREAM_RESUME_MAX {
        // 已拿到完整内容则不再续写
        if !text.trim().is_empty() && extract_json(&text).is_some() {
            log_call(call_id, &format!("内容已成完整 JSON，提前返回（共 {} 字符，续写 {} 次）", text.chars().count(), attempt - 1));
            return Ok(StreamOutcome { text, usage });
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
            log_call(call_id, &format!("断流且内容为空，第 {}/{} 次直接重发原请求", attempt, STREAM_RESUME_MAX));
            (system.to_string(), user.to_string())
        } else {
            log_call(call_id, &format!(
                "断流，第 {}/{} 次发断点续写请求（已累计 {} 字符）\n── 已收到片段（截断）──\n{}",
                attempt,
                STREAM_RESUME_MAX,
                text.chars().count(),
                log_preview(&text)
            ));
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
                log_call(call_id, &format!("续写请求非 2xx 响应体：{}", log_preview(&t)));
                Err(format!("AI错误 {}: {}", s.as_u16(), error_or_status(&t, &s)))
            } else {
                consume_sse(hooks, call_id, model, r, &tick).await
            }
        }
        .await;
        match outcome {
            Ok((part, u)) => {
                // 续写内容直接拼接（模型被要求从断点继续，不重复已有部分）。
                if u.is_some() {
                    usage = u;
                }
                let got = part.chars().count();
                log_call(call_id, &format!(
                    "续写第 {} 次返回 {} 字符\n── 续写返回内容 ──\n{}",
                    attempt,
                    got,
                    log_preview(&part)
                ));
                text.push_str(&part);
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
                log_call(call_id, &format!("续写第 {} 次失败：{}（已累计 {} 字符）", attempt, err, text.chars().count()));
                // 续写期间有进展则把 partial 留着再试一次；否则终止
                if text.chars().count() > before || is_recoverable_network_error(&err) {
                    continue;
                }
                return Err(err);
            }
        }
    }
    // resume 次数用尽：把现有文本原样交回（调用方可用宽容解析，可能已经完整）
    log_call(call_id, &format!("续写次数用尽，将现有内容交回调用方宽容解析（共 {} 字符）", text.chars().count()));
    finish_stream_text(call_id, text, usage)
}

/// resume 次数用尽后的收尾：记录最终输出全量（截断预览），文本原样交回
///（可能已完整或为空，由调用方宽容解析/判定），usage 随 StreamOutcome 带回——
/// 通道只负责传输，不替调用方判断内容语义。
fn finish_stream_text(
    call_id: u64,
    text: String,
    usage: Option<serde_json::Value>,
) -> Result<StreamOutcome, String> {
    log_call(call_id, &format!(
        "═══ 流式最终返回（{} 字符，usage={}）═══\n{}",
        text.chars().count(),
        usage.is_some(),
        log_preview(&text)
    ));
    Ok(StreamOutcome { text, usage })
}

/// 提取首个平衡 JSON 对象（跳过前后缀文本 / Markdown 围栏 / 解释性文字）。
/// 通道内用于「已拿到完整内容？」判定；调用方解析可用各自的宽容实现。
fn extract_json(input: &str) -> Option<&str> {
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
                crate::exit_log!("[ai-channel] tool_call arguments 不是合法 JSON（len={}）", s.chars().count());
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
fn is_tools_unsupported_error_text(err: &str) -> bool {
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
    provider: &AiProvider,
    model: &str,
    system: &str,
    user: &str,
    temperature: f32,
    tools_json: &str,
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
            Some(msg) if msg.get("tool_calls").is_some() => {
                match parse_tool_call_arguments(msg) {
                    Some(v) => Some((v, true)),
                    None => {
                        crate::exit_log!("[ai-channel] tool_call arguments 解析失败，尝试 content 文本协议");
                        None
                    }
                }
            }
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
            if used_tool_call { "原生 tool_call" } else { "文本协议" },
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
    let from = { let mut i = from.min(slice.len()); while i > 0 && !slice.is_char_boundary(i) { i -= 1; } i };
    let to = { let mut i = to.min(slice.len()); while i < slice.len() && !slice.is_char_boundary(i) { i += 1; } i };
    let win: String = slice[from..to].chars().collect();
    format!("{}（第 {} 行第 {} 列，附近上下文：…{}…）", err, line, col, win)
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
            '"' => { in_string = !in_string; out.push(c); }
            '\\' if in_string => { escaped = true; out.push(c); }
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
pub async fn complete_chat(
    hooks: &dyn ChannelHooks,
    provider: &AiProvider,
    model: &str,
    system: &str,
    user: &str,
    temperature: f32,
    tools_json: Option<&str>,
) -> Result<CompleteOutcome, String> {
    let client = ai_http_client();
    let url = completion_url(&provider.openai_url);
    let call_id = next_call_id();
    log_call(call_id, &format!(
        "═══ 非流式请求提交 ═══ model={} provider={}({}) url={} temperature={}\n── system prompt ──\n{}\n── user prompt ──\n{}",
        model,
        provider.name,
        provider.id,
        url,
        temperature,
        log_preview(system),
        log_preview(user)
    ));
    let mut body = serde_json::json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user }
        ],
        "stream": false,
        "temperature": temperature,
    });
    // 原生 tool-calling：网关支持时强制结构化输出；tool_choice 强制点名工具，
    // 防止模型「自己决定」要不要调工具（探索场景每次都必须点单）。
    if let Some(spec) = tools_json {
        if let Some(arr) = serde_json::from_str::<serde_json::Value>(spec)
            .ok()
            .filter(|v| v.is_array())
        {
            if let Some(obj) = body.as_object_mut() {
                obj.insert("tools".into(), arr);
                obj.insert("tool_choice".into(), serde_json::json!({ "type": "function", "function": { "name": "request_files" } }));
            }
        }
    }

    let resp = send_with_retries(hooks, call_id, &client, &url, provider, &body).await?;
    let status = resp.status();
    let raw = read_body_text_lossy(call_id, "非流式响应", resp).await;
    if raw.is_empty() {
        // 区分「读体超时」与「空 2xx 响应」：超时时响应已被消费，状态码不可再取，
        // 用带标签的固定文案让日志与前端都能看出真实原因。
        return Err(format!("AI请求无响应: 读取响应体超时（超过 {}s）", READ_BODY_TIMEOUT.as_secs()));
    }
    log_call(call_id, &format!(
        "═══ 非流式响应（{}）共 {} 字符 ═══\n{}",
        status.as_u16(),
        raw.chars().count(),
        log_preview(&raw)
    ));
    let value: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            log_call(call_id, &format!("响应 JSON 解析失败：{}", e));
            return Err(format!("解析响应失败: {}", e));
        }
    };
    if !status.is_success() {
        let msg = value
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("未知错误");
        return Err(format!("AI错误 {}: {}", status.as_u16(), msg));
    }
    let message = value
        .get("choices")
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
        .map(|m| m.get("tool_calls").and_then(|tc| tc.as_array()).map(|a| !a.is_empty()).unwrap_or(false))
        .unwrap_or(false);
    if text.is_empty() && !has_tool_calls {
        log_call(call_id, "AI返回空（choices[0].message.content 为空）");
        return Err("AI返回空".into());
    }
    if has_tool_calls {
        log_call(call_id, &format!(
            "═══ 非流式 tool_call 响应（{} 个调用）═══\n{}",
            message.as_ref().and_then(|m| m.get("tool_calls")).and_then(|tc| tc.as_array()).map(|a| a.len()).unwrap_or(0),
            log_preview(&message.as_ref().map(|m| m.to_string()).unwrap_or_default())
        ));
    }
    log_call(call_id, &format!(
        "═══ 非流式最终输出（{} 字符，usage={}）═══\n{}",
        text.chars().count(),
        value.get("usage").is_some(),
        log_preview(&text)
    ));
    Ok(CompleteOutcome {
        text,
        usage: value.get("usage").cloned(),
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_call_arguments_parsing() {
        // 标准协议：arguments 是 JSON 字符串
        let msg: serde_json::Value = serde_json::from_str(
            r#"{"tool_calls":[{"function":{"name":"request_files","arguments":"{\"paths\":[\"src/main.rs\"],\"done\":false}"}}]}"#,
        )
        .unwrap();
        let v = parse_tool_call_arguments(&msg).expect("字符串 arguments 应解析成功");
        assert_eq!(v["paths"][0], "src/main.rs");
        assert_eq!(v["done"], false);
        // 容错：部分网关直接发对象
        let msg_obj: serde_json::Value = serde_json::from_str(
            r#"{"tool_calls":[{"function":{"arguments":{"done":true,"reason":"读完了"}}}]}"#,
        )
        .unwrap();
        let v2 = parse_tool_call_arguments(&msg_obj).expect("对象 arguments 应直接返回");
        assert_eq!(v2["done"], true);
        // 非法 JSON 字符串 → None
        let msg_bad: serde_json::Value = serde_json::from_str(
            r#"{"tool_calls":[{"function":{"arguments":"{broken"}}]}"#,
        )
        .unwrap();
        assert!(parse_tool_call_arguments(&msg_bad).is_none());
        // 没有 tool_calls → None
        let msg_none: serde_json::Value = serde_json::from_str(r#"{"content":"hello"}"#).unwrap();
        assert!(parse_tool_call_arguments(&msg_none).is_none());
    }

    #[test]
    fn tools_unsupported_error_classification() {
        // 400/422 + tools 关键词 → 降级
        assert!(is_tools_unsupported_error_text(
            "AI错误 400: tools is not supported by this model"
        ));
        assert!(is_tools_unsupported_error_text(
            "AI错误 422: Unknown parameter: 'tools'"
        ));
        assert!(is_tools_unsupported_error_text(
            "AI错误 400: Unrecognized request argument supplied: functions"
        ));
        // 非 400/422 → 不降级（可能是限流/超时，等价错误不该触发换协议）
        assert!(!is_tools_unsupported_error_text(
            "AI错误 429: tools rate limited"
        ));
        // 400 但与 tools 无关 → 不降级（业务错误原样返回）
        assert!(!is_tools_unsupported_error_text(
            "AI错误 400: context_length_exceeded"
        ));
        // 非 AI 错误格式 → 不降级
        assert!(!is_tools_unsupported_error_text("网络错误"));
    }

    #[test]
    fn tool_spec_is_valid_json_array() {
        let v: serde_json::Value = serde_json::from_str(EXPLORER_TOOL_SPEC).expect("工具定义必须是合法 JSON");
        assert!(v.is_array());
        assert_eq!(v[0]["type"], "function");
        assert_eq!(v[0]["function"]["name"], "request_files");
        assert!(v[0]["function"]["parameters"]["properties"]["paths"].is_object());
    }

    #[test]
    fn completion_url_variants() {
        assert_eq!(completion_url("https://api.x.com"), "https://api.x.com/v1/chat/completions");
        assert_eq!(completion_url("https://api.x.com/"), "https://api.x.com/v1/chat/completions");
        assert_eq!(completion_url("https://api.x.com/v1/"), "https://api.x.com/v1/chat/completions");
    }

    #[test]
    fn recoverable_error_classification() {
        // 网络/传输类 → 可恢复
        assert!(is_recoverable_network_error("流式中断: error decoding response body"));
        assert!(is_recoverable_network_error("流式中断: 读流空闲超时（超过 75s 未收到数据）"));
        assert!(is_recoverable_network_error("请求失败: connection reset by peer"));
        assert!(is_recoverable_network_error("stream disconnected (eos)"));
        // send 阶段的超时/失败 → 可恢复
        assert!(is_recoverable_network_error("请求超时: 等待响应头超过 180s"));
        assert!(is_recoverable_network_error(
            "请求失败: error sending request for url (https://api.example.com/v1/chat/completions)"
        ));
        // 业务类错误（AI 主动返回）→ 不重试
        assert!(!is_recoverable_network_error("AI错误 401: Invalid API key"));
        assert!(!is_recoverable_network_error("AI返回空"));
    }

    #[test]
    fn api_error_body_extraction_tolerates_html_and_plain_text() {
        // JSON 正文 → 取 error.message
        assert_eq!(
            extract_api_error_body("{\"error\":{\"message\":\"Rate limit exceeded\"}}"),
            "Rate limit exceeded"
        );
        // 非 JSON（网关 HTML/纯文本）→ 取片段而非报「解析失败」
        assert_eq!(extract_api_error_body("<html>502 Bad Gateway</html>"), "<html>502 Bad Gateway</html>");
        assert_eq!(extract_api_error_body("  "), "");
        // 超长正文截断
        let long = "x".repeat(1000);
        assert_eq!(extract_api_error_body(&long).chars().count(), 300);
    }

    #[test]
    fn resume_prompt_carries_partial_and_forbids_repetition() {
        let (sys, usr) = resume_prompt("你是架构师", "分析项目", "{\"summary\":\"x\",\"nodes\":[{\"id\":\"a\"");
        assert!(sys.contains("继续输出模式"), "system 提示应声明继续模式");
        assert!(sys.contains("你是架构师"), "原 system 提示应保留");
        assert!(usr.contains("分析项目"), "原 user 上下文应保留");
        assert!(usr.contains("{\"summary\":\"x\""), "已输出片段应回传");
        assert!(usr.contains("断点"), "应指明从断点继续");
    }

    #[test]
    fn resume_prompt_truncates_long_partial() {
        // 超长 partial：首尾保留 + 中间省略标记，避免撑爆上下文
        let long = "x".repeat(20_000);
        let (_, usr) = resume_prompt("s", "u", &long);
        assert!(usr.contains("中间省略"), "超长片段应折叠中间部分");
        let len = usr.chars().count();
        assert!(len < 8_500, "折叠后长度应受限，实际 {}", len);
    }

    #[test]
    fn extract_json_balances_strings_and_escapes() {
        assert_eq!(extract_json("前缀 {\"a\":1} 后缀"), Some("{\"a\":1}"));
        assert_eq!(extract_json("```json\n{\"a\":{\"b\":\"}\"}}\n```"), Some("{\"a\":{\"b\":\"}\"}}"));
        assert_eq!(extract_json("{\"a\":\"{\\\"x\\\"}\"}"), Some("{\"a\":\"{\\\"x\\\"}\"}"));
        assert_eq!(extract_json("没有任何 JSON"), None);
        assert_eq!(extract_json("{\"未闭合\":1"), None);
    }

    #[test]
    fn strip_prefix_avoids_nested_stream_error() {
        assert_eq!(strip_prefix_of("流式中断: abc", "流式中断: "), "abc");
        assert_eq!(strip_prefix_of("其它错误", "流式中断: "), "其它错误");
    }

    #[test]
    fn log_preview_keeps_head_tail_and_marks_omitted() {
        // 短文本原样返回，不做任何包装
        assert_eq!(log_preview("hello"), "hello");
        // 超长文本：保留头尾 + 标注省略量与总长
        let long = format!("{}{}", "a".repeat(45_000), "z");
        let out = log_preview(&long);
        assert!(out.contains("日志截断"), "超长文本应带截断标记");
        assert!(out.starts_with("aaaa"), "开头应保留");
        assert!(out.ends_with("z"), "结尾应保留");
        let total: usize = long.chars().count();
        assert!(out.contains(&format!("全文共 {} 字符", total)));
        // 恰好等于上限时不截断
        let edge = "x".repeat(LOG_PREVIEW_CAP);
        assert_eq!(log_preview(&edge), edge);
    }
}
