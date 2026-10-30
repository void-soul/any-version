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

/// Minimal request endpoint contract; provider configuration stays in the application.
pub trait ChannelProvider: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn api_key(&self) -> &str;
    fn openai_url(&self) -> &str;
}
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
/// 「未产出内容」的非流式重试次数与退避基数（第 n 次重试睡 n×基数）。
/// 触发条件见 is_retryable_no_output：这类响应不含任何 token（usage 全 0），
/// 重试无额外计费风险；退避时间短是因为空壳/限流响应都是毫秒级返回。
pub const EMPTY_RETRY_MAX: usize = 2;
pub const EMPTY_RETRY_DELAY: Duration = Duration::from_secs(1);
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
            log_call(
                call_id,
                &format!(
                    "读响应体超时（{}）：超过 {}s 未收到正文",
                    label,
                    READ_BODY_TIMEOUT.as_secs()
                ),
            );
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

/// 一次非流式响应是否属于「没产出任何内容、重试零成本」的情形：
/// - 429：限流（网关常在限流时连响应体都不给），没有任何 token 产生；
/// - 2xx 但内容为空且无 tool_calls：部分网关（实测魔搭 ModelScope + ZhipuAI/GLM-4.7-Flash）
///   在模型无可用实例/被限流时**不返回错误码**，而是 HTTP 200 + `{"choices":null,"usage":{全 0}}`
///   的 171 字节空壳（250ms 返回），既无内容也无计费。
/// 这两类都允许自动重试；5xx 不重试（可能已产生计费输出）。
fn is_retryable_no_output(status: u16, text_empty: bool, has_tool_calls: bool) -> bool {
    status == 429 || ((200..300).contains(&status) && text_empty && !has_tool_calls)
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
    provider: &dyn ChannelProvider,
    body: &serde_json::Value,
) -> Result<reqwest::Response, String> {
    let mut last_err = String::new();
    for attempt in 0..=SEND_RETRY_MAX {
        hooks.check_cancel()?;
        if attempt > 0 {
            log_call(
                call_id,
                &format!(
                    "send 重试 {}/{}：上次错误：{}",
                    attempt, SEND_RETRY_MAX, last_err
                ),
            );
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
        log_call(
            call_id,
            &format!(
                "POST {}（第 {} 次尝试）提交 body：\n{}",
                url,
                attempt + 1,
                log_preview(&serde_json::to_string_pretty(body).unwrap_or_default())
            ),
        );
        let started = std::time::Instant::now();
        let req = client
            .post(url)
            .header("Authorization", format!("Bearer {}", provider.api_key()))
            .header("Content-Type", "application/json")
            .json(body);
        match tokio::time::timeout(REQUEST_TTFB_TIMEOUT, req.send()).await {
            Ok(Ok(resp)) => {
                let st = resp.status();
                log_call(
                    call_id,
                    &format!(
                        "响应头 {}：耗时 {}ms",
                        st.as_u16(),
                        started.elapsed().as_millis()
                    ),
                );
                return Ok(resp);
            }
            Ok(Err(e)) => {
                last_err = format!("请求失败: {}", e);
                log_call(
                    call_id,
                    &format!(
                        "send 失败（{}ms）：{}",
                        started.elapsed().as_millis(),
                        last_err
                    ),
                );
            }
            Err(_) => {
                last_err = format!(
                    "请求超时: 等待响应头超过 {}s",
                    REQUEST_TTFB_TIMEOUT.as_secs()
                );
                log_call(
                    call_id,
                    &format!(
                        "send 失败（{}ms）：{}",
                        started.elapsed().as_millis(),
                        last_err
                    ),
                );
            }
        }
    }
    log_call(
        call_id,
        &format!(
            "send 最终失败（共 {} 次尝试）：{}",
            SEND_RETRY_MAX + 1,
            last_err
        ),
    );
    Err(last_err)
}

mod completion;
mod stream;
pub use completion::{
    complete_chat, complete_chat_json, complete_chat_messages, json_error_context, parse_json,
    parse_tool_call_arguments, repair_invalid_escapes, CompleteOutcome, EXPLORER_TOOL_SPEC,
};
pub use stream::{resume_prompt, stream_chat_with_resume, StreamOutcome};
#[cfg(test)]
mod tests;

use completion::extract_json;
#[cfg(test)]
use completion::is_tools_unsupported_error_text;
