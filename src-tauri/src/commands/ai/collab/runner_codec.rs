//! runner codec responsibility.
use super::*;

// ─── 流式事件解析 ───

/// claude stream-json 的一行事件
pub(super) enum StreamEvent {
    Delta(String),
    Result(String, Option<TokenUsage>),
    /// 活动状态（思考中/调用工具等），不写入最终内容，仅前端实时显示
    Activity(String),
    Ignore,
}

/// 工具运行时的流式输出解析适配器（每 runner / 工具族一个 adapter）。
///
/// 把原本散落的 parse_claude_json / parse_codex_json / parse_opencode_json /
/// parse_gemini_json 收敛为统一的 trait 接口；dispatch_to_tool 只依赖该 trait，
/// 新增工具只需新增一个 adapter 并在 `runner_adapter` 工厂登记，无需改动编排逻辑。
pub(super) trait RunnerAdapter: Send + Sync {
    /// 该 runner 是否为 JSON 流式输出（决定读取循环是否逐行 JSON 解析）。
    /// 非 JSON runner 走“整段文本累加”兜底，不逐行解析。
    fn is_json(&self) -> bool {
        true
    }
    /// 解析一行 stdout 为内部流式事件（Delta / Result / Activity / Ignore）。
    /// 非 JSON runner 恒返回 None。
    fn parse(&self, line: &str) -> Option<(StreamEvent, Option<String>)>;
}

/// claude（`stream-json`）适配器
pub(super) struct ClaudeRunner;
impl RunnerAdapter for ClaudeRunner {
    fn parse(&self, line: &str) -> Option<(StreamEvent, Option<String>)> {
        parse_claude_json(line)
    }
}

/// codex（`codex-json`）适配器
pub(super) struct CodexRunner;
impl RunnerAdapter for CodexRunner {
    fn parse(&self, line: &str) -> Option<(StreamEvent, Option<String>)> {
        parse_codex_json(line)
    }
}

/// opencode（`opencode-json`）适配器
pub(super) struct OpenCodeRunner;
impl RunnerAdapter for OpenCodeRunner {
    fn parse(&self, line: &str) -> Option<(StreamEvent, Option<String>)> {
        parse_opencode_json(line)
    }
}

/// gemini / qwen（`gemini-json`）适配器
pub(super) struct GeminiRunner;
impl RunnerAdapter for GeminiRunner {
    fn parse(&self, line: &str) -> Option<(StreamEvent, Option<String>)> {
        parse_gemini_json(line)
    }
}

/// Cline CLI（`--json`）适配器
pub(super) struct ClineRunner;
impl RunnerAdapter for ClineRunner {
    fn parse(&self, line: &str) -> Option<(StreamEvent, Option<String>)> {
        parse_cline_json(line)
    }
}

/// reasonix（`reasonix-json`）适配器
pub(super) struct ReasonixRunner;
impl RunnerAdapter for ReasonixRunner {
    fn parse(&self, line: &str) -> Option<(StreamEvent, Option<String>)> {
        parse_reasonix_json(line)
    }
}

/// 兜底适配器：非 JSON runner（无流式协议），整段文本累加，不逐行解析
pub(super) struct GenericRunner;
impl RunnerAdapter for GenericRunner {
    fn is_json(&self) -> bool {
        false
    }
    fn parse(&self, _line: &str) -> Option<(StreamEvent, Option<String>)> {
        None
    }
}

/// 按 tool_config.runner 选取对应 adapter（每工具一个 adapter 的入口）。
/// 未知 / 空 runner 一律走 GenericRunner 兜底。
pub(super) fn runner_adapter(runner: &str) -> Box<dyn RunnerAdapter> {
    match runner {
        "stream-json" => Box::new(ClaudeRunner),
        "codex-json" => Box::new(CodexRunner),
        "opencode-json" => Box::new(OpenCodeRunner),
        "gemini-json" => Box::new(GeminiRunner),
        "cline-json" => Box::new(ClineRunner),
        "reasonix-json" => Box::new(ReasonixRunner),
        _ => Box::new(GenericRunner),
    }
}

/// claude `--output-format stream-json` 事件解析
pub(super) fn parse_claude_json(line: &str) -> Option<(StreamEvent, Option<String>)> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let sid = v
        .get("session_id")
        .and_then(|s| s.as_str())
        .map(|s| s.to_string());
    match v.get("type").and_then(|t| t.as_str()) {
        Some("content_block_start") => {
            // 检测 thinking / tool_use 块的开始
            if let Some(cb) = v.get("content_block") {
                match cb.get("type").and_then(|t| t.as_str()) {
                    Some("thinking") => {
                        return Some((StreamEvent::Activity("思考中…".to_string()), sid))
                    }
                    Some("tool_use") => {
                        let name = cb.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");
                        return Some((StreamEvent::Activity(format!("调用工具: {}", name)), sid));
                    }
                    _ => {}
                }
            }
            Some((StreamEvent::Ignore, sid))
        }
        Some("content_block_delta") => {
            let delta = v
                .get("delta")
                .and_then(|d| d.get("text"))
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            Some((StreamEvent::Delta(delta), sid))
        }
        Some("result") => {
            let text = v
                .get("result")
                .and_then(|r| r.as_str())
                .unwrap_or("")
                .to_string();
            // claude stream-json 的 result 事件含 usage（Anthropic 协议）；其余字段无则 None
            let usage = v.get("usage").and_then(|u| {
                let i = u.get("input_tokens").and_then(|x| x.as_u64());
                let o = u.get("output_tokens").and_then(|x| x.as_u64());
                match (i, o) {
                    (Some(i), Some(o)) => Some(TokenUsage {
                        input_tokens: i,
                        output_tokens: o,
                    }),
                    _ => None,
                }
            });
            Some((StreamEvent::Result(text, usage), sid))
        }
        _ => Some((StreamEvent::Ignore, sid)),
    }
}

/// codex `exec --json` 事件解析（JSONL）
/// 会话 id 在 thread.started.thread_id；助手文本在 item.completed(item_type=assistant_message).text
pub(super) fn parse_codex_json(line: &str) -> Option<(StreamEvent, Option<String>)> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let sid = v
        .get("thread_id")
        .and_then(|s| s.as_str())
        .map(|s| s.to_string());
    match v.get("type").and_then(|t| t.as_str()) {
        Some("thread.started") => Some((StreamEvent::Activity("初始化…".to_string()), sid)),
        Some("item.completed") => {
            let item = match v.get("item") {
                Some(i) => i,
                None => return Some((StreamEvent::Ignore, sid)),
            };
            match item.get("item_type").and_then(|t| t.as_str()) {
                Some("assistant_message") => {
                    let text = item
                        .get("text")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string();
                    if text.is_empty() {
                        return Some((StreamEvent::Ignore, sid));
                    }
                    Some((StreamEvent::Delta(text), sid))
                }
                Some("tool_call") => {
                    let tool = item
                        .get("tool")
                        .and_then(|t| t.as_str())
                        .unwrap_or("unknown");
                    // 保留调用身份（call_id/id），避免同工具多次调用时无法区分（参照 cc-switch #5310）
                    let call_id = item
                        .get("call_id")
                        .and_then(|c| c.as_str())
                        .or_else(|| item.get("id").and_then(|c| c.as_str()))
                        .unwrap_or("");
                    let label = if call_id.is_empty() {
                        format!("使用工具: {}", tool)
                    } else {
                        format!("使用工具: {} (#{})", tool, call_id)
                    };
                    Some((StreamEvent::Activity(label), sid))
                }
                _ => Some((StreamEvent::Ignore, sid)),
            }
        }
        Some("turn.failed") => {
            let err = v
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("turn failed")
                .to_string();
            Some((StreamEvent::Result(format!("[error] {}", err), None), sid))
        }
        _ => Some((StreamEvent::Ignore, sid)),
    }
}

/// 从 opencode `--format json` 事件里提取助手文本
/// 实际格式（真机确认）：
///   {"type":"text",...,"sessionID":"ses_xxx","part":{"type":"text","text":"你好",...}}
///   {"type":"step_finish",...,"part":{"type":"step-finish",...}}
pub(super) fn extract_opencode_text(v: &serde_json::Value) -> Option<String> {
    // 优先：part.text（opencode run --format json 的标准位置）
    if let Some(part) = v.get("part") {
        if let Some(s) = part.get("text").and_then(|x| x.as_str()) {
            return Some(s.to_string());
        }
        // part.content 也作为容错
        if let Some(s) = part.get("content").and_then(|x| x.as_str()) {
            return Some(s.to_string());
        }
    }
    // 兜底：顶层 text / content
    if let Some(s) = v.get("text").and_then(|x| x.as_str()) {
        return Some(s.to_string());
    }
    if let Some(s) = v.get("content").and_then(|x| x.as_str()) {
        return Some(s.to_string());
    }
    None
}

/// opencode `run --format json` 事件解析
/// 实际 JSONL 格式（每行一个 JSON 对象）：
///   step_start → part.type = "step-start"
///   tool_use   → part.type = "tool", part.tool = "write"/"bash"/...
///   text       → part.type = "text", part.text = "助手回复"
///   step_finish→ part.type = "step-finish"
pub(super) fn parse_opencode_json(line: &str) -> Option<(StreamEvent, Option<String>)> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    // sessionID（驼峰）是 opencode 的标准字段
    let sid = v
        .get("sessionID")
        .or_else(|| v.get("session_id"))
        .or_else(|| v.get("session").and_then(|s| s.get("id")))
        .or_else(|| v.get("id"))
        .and_then(|s| s.as_str())
        .map(|s| s.to_string());
    let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    // 收尾事件：step_finish / done / result / completed
    if matches!(
        ty,
        "step_finish" | "done" | "result" | "completed" | "turn.completed" | "session.completed"
    ) {
        let text = extract_opencode_text(&v).unwrap_or_default();
        return Some((StreamEvent::Result(text, None), sid));
    }
    // step_start → 活动状态
    if ty == "step_start" {
        return Some((StreamEvent::Activity("思考中…".to_string()), sid));
    }
    // tool_use → 提取工具名和状态
    if ty == "tool_use" {
        if let Some(part) = v.get("part") {
            let tool = part
                .get("tool")
                .and_then(|t| t.as_str())
                .unwrap_or("unknown");
            let status = part
                .get("state")
                .and_then(|s| s.get("status"))
                .and_then(|s| s.as_str())
                .unwrap_or("");
            // 提取工具输入摘要
            let detail = part
                .get("state")
                .and_then(|s| s.get("input"))
                .and_then(|i| i.get("filePath"))
                .or_else(|| {
                    part.get("state")
                        .and_then(|s| s.get("input"))
                        .and_then(|i| i.get("command"))
                })
                .and_then(|f| f.as_str())
                .map(|s| {
                    let s = s.rsplit(['/', '\\']).next().unwrap_or(s);
                    format!(" → {}", s)
                })
                .unwrap_or_default();
            let activity = match status {
                "completed" => format!("✓ {}{}", tool, detail),
                "running" | "in_progress" => format!("执行 {}…{}", tool, detail),
                _ => format!("{} ({}){}", tool, status, detail),
            };
            return Some((StreamEvent::Activity(activity), sid));
        }
        return Some((StreamEvent::Activity("使用工具…".to_string()), sid));
    }
    // 带文本 → 增量
    if let Some(t) = extract_opencode_text(&v) {
        if !t.is_empty() {
            return Some((StreamEvent::Delta(t), sid));
        }
    }
    Some((StreamEvent::Ignore, sid))
}

/// gemini-cli / qwen-code `--output-format stream-json` 事件解析
/// 格式：
///   {"type":"system","subtype":"init","session_id":"..."}
///   {"type":"assistant","message":{"content":[{"type":"text","text":"你好"}]}}
///   {"type":"result","subtype":"success","result":"你好","session_id":"..."}
pub(super) fn parse_gemini_json(line: &str) -> Option<(StreamEvent, Option<String>)> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let sid = v
        .get("session_id")
        .and_then(|s| s.as_str())
        .map(|s| s.to_string());
    match v.get("type").and_then(|t| t.as_str()) {
        Some("system") => {
            // 初始化事件 → 活动状态
            Some((StreamEvent::Activity("初始化…".to_string()), sid))
        }
        Some("assistant") => {
            // 提取 message.content 中的文本
            let mut text = String::new();
            if let Some(content) = v
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_array())
            {
                for part in content {
                    if part.get("type").and_then(|t| t.as_str()) == Some("text") {
                        if let Some(t) = part.get("text").and_then(|x| x.as_str()) {
                            text.push_str(t);
                        }
                    }
                    // 检测工具调用
                    if part.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                        let name = part
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("unknown");
                        return Some((StreamEvent::Activity(format!("调用工具: {}", name)), sid));
                    }
                }
            }
            if !text.is_empty() {
                Some((StreamEvent::Delta(text), sid))
            } else {
                Some((StreamEvent::Ignore, sid))
            }
        }
        Some("result") => {
            let text = v
                .get("result")
                .and_then(|r| r.as_str())
                .unwrap_or("")
                .to_string();
            let is_error = v.get("is_error").and_then(|e| e.as_bool()).unwrap_or(false);
            if is_error {
                Some((StreamEvent::Result(format!("[error] {}", text), None), sid))
            } else {
                Some((StreamEvent::Result(text, None), sid))
            }
        }
        Some("user") => {
            // 用户消息回显 → 忽略
            Some((StreamEvent::Ignore, sid))
        }
        _ => Some((StreamEvent::Ignore, sid)),
    }
}

/// Cline `--json` 事件解析。
/// 文本事件使用 `partial=true` 增量推送，最终非 partial 文本作为 Result，
/// 这样即使 Cline 在收尾时重复发送完整文本，也不会导致前端重复显示。
pub(super) fn parse_cline_json(line: &str) -> Option<(StreamEvent, Option<String>)> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let sid = v
        .get("session_id")
        .or_else(|| v.get("sessionId"))
        .or_else(|| v.get("session"))
        .and_then(|s| s.as_str())
        .map(|s| s.to_string());
    let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let partial = v.get("partial").and_then(|p| p.as_bool()).unwrap_or(false);

    match ty {
        "say" => {
            let subtype = v.get("say").and_then(|s| s.as_str()).unwrap_or("");
            let text = v
                .get("text")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            if text.is_empty() {
                return Some((StreamEvent::Ignore, sid));
            }
            if subtype == "text" || subtype.is_empty() {
                if partial {
                    Some((StreamEvent::Delta(text), sid))
                } else {
                    Some((StreamEvent::Result(text, None), sid))
                }
            } else if subtype == "api_req_started" || subtype == "api_req_retried" {
                Some((StreamEvent::Activity("请求模型…".to_string()), sid))
            } else {
                Some((StreamEvent::Activity(format!("{}", subtype)), sid))
            }
        }
        "ask" => {
            let question = v.get("text").and_then(|t| t.as_str()).unwrap_or("");
            if question.is_empty() {
                Some((StreamEvent::Ignore, sid))
            } else {
                Some((
                    StreamEvent::Activity(format!("等待输入: {}", question)),
                    sid,
                ))
            }
        }
        "error" => {
            let text = v
                .get("text")
                .or_else(|| v.get("message"))
                .and_then(|t| t.as_str())
                .unwrap_or("Cline 返回错误")
                .to_string();
            Some((StreamEvent::Result(format!("[error] {}", text), None), sid))
        }
        "completion_result" | "result" | "done" => {
            let text = v
                .get("text")
                .or_else(|| v.get("result"))
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            Some((StreamEvent::Result(text, None), sid))
        }
        _ => Some((StreamEvent::Ignore, sid)),
    }
}

/// reasonix `--output-format stream-json` 事件解析（eventwire JSONL 流）
/// 格式（对齐 open-tag reasonixRuntime）：
///   {"kind":"turn_started"} / {"kind":"phase",...}            → 活动状态
///   {"kind":"text","text":"切片"}                          → 增量 token 切片，丢弃（message 事件才是权威组装结果）
///   {"kind":"message","text":"完整回复"}                   → Delta
///   {"kind":"tool_dispatch","tool":{id,name,args,partial}} → 工具调用（partial 标记先到、无 args；full 后到带 args，按 id 去重）
///   {"kind":"reasoning","text":...}                        → 思考（v1.18/1.19 不发射，防御性保留）
///   {"type":"result","session_id":"...","is_error":...}  → 终结事件：Result + session_id
pub(super) fn parse_reasonix_json(line: &str) -> Option<(StreamEvent, Option<String>)> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    // 终结 result 对象（{"type":"result","result":"...","session_id":"...","is_error":...}）
    if v.get("type").and_then(|t| t.as_str()) == Some("result") {
        let sid = v
            .get("session_id")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string());
        let text = v
            .get("result")
            .and_then(|r| r.as_str())
            .unwrap_or("")
            .to_string();
        let is_error = v.get("is_error").and_then(|e| e.as_bool()).unwrap_or(false);
        let usage = v.get("usage").and_then(|u| {
            let i = u.get("input_tokens").and_then(|x| x.as_u64());
            let o = u.get("output_tokens").and_then(|x| x.as_u64());
            match (i, o) {
                (Some(i), Some(o)) => Some(TokenUsage {
                    input_tokens: i,
                    output_tokens: o,
                }),
                _ => None,
            }
        });
        if is_error {
            return Some((StreamEvent::Result(format!("[error] {}", text), usage), sid));
        }
        return Some((StreamEvent::Result(text, usage), sid));
    }
    match v.get("kind").and_then(|k| k.as_str()) {
        Some("turn_started") | Some("phase") => {
            Some((StreamEvent::Activity("工作中…".to_string()), None))
        }
        Some("text") => {
            // 增量 token 切片 → 丢弃（message 事件才是权威组装结果）
            Some((StreamEvent::Ignore, None))
        }
        Some("message") => {
            let text = v
                .get("text")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            Some((StreamEvent::Delta(text), None))
        }
        Some("reasoning") => {
            // v1.18/1.19 不发射 reasoning 事件；防御性保留（未来版本可能输出）
            Some((StreamEvent::Activity("思考中…".to_string()), None))
        }
        Some("tool_dispatch") => {
            // partial 标记先到（无 args），full 事件后到（带 args）——按工具 id 去重，只展示 full 一次
            let t = v.get("tool").unwrap_or(&serde_json::Value::Null);
            let id = t.get("id").and_then(|x| x.as_str()).unwrap_or("");
            let partial = t.get("partial").and_then(|x| x.as_bool()).unwrap_or(false);
            let has_args = t.get("args").is_some();
            if partial || !has_args || id.is_empty() {
                return Some((StreamEvent::Ignore, None));
            }
            let name = t.get("name").and_then(|n| n.as_str()).unwrap_or("tool");
            Some((StreamEvent::Activity(format!("调用工具: {}", name)), None))
        }
        // tool_result / usage / notice / approval_request / turn_done：忽略（终结以 result 对象为准）
        _ => Some((StreamEvent::Ignore, None)),
    }
}
