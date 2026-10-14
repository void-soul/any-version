//! Anthropic Messages API ↔ OpenAI Chat Completions API 格式转换
//!
//! 参考 cc-switch 的 transform.rs 实现，精简版。

use serde_json::{json, Value};
use std::collections::HashMap;

/// 模型别名映射配置
#[derive(Clone, Debug, Default)]
pub struct ModelAliases {
    /// 默认模型（当无匹配时使用）
    pub default_model: Option<String>,
    /// 角色关键词 → 实际模型 ID 的映射
    /// 例如: "sonnet" → "deepseek-v4-pro", "opus" → "claude-opus-4-8"
    pub role_map: HashMap<String, String>,
}

/// 将 Claude 模型名映射到实际上游模型名。
///
/// 映射逻辑：
/// 1. 剥离 `[1M]`/`[1m]` 后缀
/// 2. 优先：role_map 中存在与官方全名完全一致的 key（精确匹配，最高优先级，
///    允许 provider 直接以官方完整模型名为键配置映射）
/// 3. 内置解析表：官方模型系列/别名 → role（sonnet/opus/haiku/fable 及 best 等），
///    再查 role_map 中对应 role 的映射
/// 4. 无角色匹配时使用 default_model
/// 5. 无配置时原样返回
pub fn map_model_name(request_model: &str, aliases: &ModelAliases) -> String {
    // 1. 剥离 [1M]/[1m] 后缀
    let cleaned = request_model
        .replace("[1M]", "")
        .replace("[1m]", "")
        .trim()
        .to_string();
    let lower = cleaned.to_lowercase();

    // 2. 优先：role_map 中存在与官方全名完全一致的 key（精确匹配）
    //    支持 provider 以官方完整模型名（如 claude-sonnet-4-20250514）为键配置映射
    if let Some(mapped) = aliases.role_map.get(&lower).or_else(|| aliases.role_map.get(&cleaned)) {
        return mapped.clone();
    }

    // 3. 内置解析表：官方模型系列/别名 → role，再查 role_map 中对应 role
    if let Some(role) = resolve_role(&cleaned) {
        if let Some(mapped) = aliases.role_map.get(role) {
            return mapped.clone();
        }
    }

    // 4. 默认模型
    if let Some(ref default) = aliases.default_model {
        return default.clone();
    }

    // 5. 原样返回
    cleaned
}

/// Claude Code 内部别名映射表（可从外部配置文件覆盖）
/// key = Claude Code 发送的别名，value = 归并后的标准 role
const CLAUDE_INTERNAL_ALIASES: &[(&str, &str)] = &[
    ("best", "opus"),     // Claude Code 的 "best" 解析为最强模型
    ("opusplan", "opus"), // "plan mode" 专用 opus 别名
];

/// 内置「官方模型名/别名 → role」解析表。
///
/// 覆盖 claude-sonnet-4-*、claude-opus-4-*、claude-haiku-4-*、claude-fable-5-*
/// 各版本，以及 Claude Code 内部别名（best/opusplan 等），统一归并为
/// `sonnet` / `opus` / `haiku` / `fable` 四个标准 role，再交由 `role_map` 映射。
fn resolve_role(model: &str) -> Option<&'static str> {
let lower = model.to_lowercase();

// Claude Code 内部别名（不含 sonnet/opus/haiku/fable 关键词）
for (alias, role) in CLAUDE_INTERNAL_ALIASES {
    if lower == *alias {
        return Some(role);
    }
}

    // 官方模型系列（关键词归并，大小写不敏感）
    if lower.contains("sonnet") {
        return Some("sonnet");
    }
    if lower.contains("opus") {
        return Some("opus");
    }
    if lower.contains("haiku") {
        return Some("haiku");
    }
    if lower.contains("fable") {
        return Some("fable");
    }

    None
}

/// 需要拦截的工具名列表（不转发到上游的工具定义）
const BLOCKED_TOOL_NAMES: &[&str] = &["BatchTool"];

/// 检查工具名是否在拦截列表中
fn is_blocked_tool_name(name: &str) -> bool {
    BLOCKED_TOOL_NAMES.contains(&name)
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  Anthropic → OpenAI
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 将 Anthropic Messages API 请求体转换为 OpenAI Chat Completions 请求体
pub fn anthropic_to_openai(body: &Value, target_model: &str, aliases: Option<&ModelAliases>) -> Value {
    let mut messages = Vec::new();

    // 解析请求中的模型名，应用别名映射
    let request_model = body.get("model").and_then(|v| v.as_str()).unwrap_or(target_model);
    let resolved_model = if let Some(a) = aliases {
        map_model_name(request_model, a)
    } else {
        target_model.to_string()
    };

    // 1. system → system message
    if let Some(system) = body.get("system") {
        let text = match system {
            Value::String(s) => s.clone(),
            Value::Array(arr) => {
                arr.iter()
                    .filter_map(|part| {
                        if part.get("type").and_then(|v| v.as_str()) == Some("text") {
                            part.get("text").and_then(|v| v.as_str()).map(String::from)
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            _ => String::new(),
        };
        if !text.is_empty() {
            messages.push(json!({"role": "system", "content": text}));
        }
    }

    // 2. messages → chat messages
    if let Some(msgs) = body.get("messages").and_then(|v| v.as_array()) {
        for msg in msgs {
            convert_anthropic_message(msg, &mut messages);
        }
    }

    // 3. 构建 OpenAI 请求
    let mut openai = json!({
        "model": resolved_model.clone(),
        "messages": messages,
    });

    // 4. 参数映射
    if let Some(max_tokens) = body.get("max_tokens").and_then(|v| v.as_u64()) {
        openai["max_completion_tokens"] = json!(max_tokens);
    }
    if let Some(temp) = body.get("temperature").and_then(|v| v.as_f64()) {
        openai["temperature"] = json!(temp);
    }
    if let Some(top_p) = body.get("top_p").and_then(|v| v.as_f64()) {
        openai["top_p"] = json!(top_p);
    }
    if let Some(stream) = body.get("stream").and_then(|v| v.as_bool()) {
        openai["stream"] = json!(stream);
        if stream {
            openai["stream_options"] = json!({"include_usage": true});
        }
    }
    if let Some(stop) = body.get("stop_sequences") {
        openai["stop"] = stop.clone();
    }

    // 5. tools 转换
    if let Some(tools) = body.get("tools").and_then(|v| v.as_array()) {
        let openai_tools: Vec<Value> = tools
            .iter()
            .filter(|t| t.get("name").and_then(|v| v.as_str()) != Some("BatchTool") &&
              !is_blocked_tool_name(t.get("name").and_then(|v| v.as_str()).unwrap_or("")))
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.get("name").unwrap_or(&json!("")),
                        "description": t.get("description").unwrap_or(&json!("")),
                        "parameters": super::normalize_function_parameters(t.get("input_schema"))
                    }
                })
            })
            .collect();
        if !openai_tools.is_empty() {
            openai["tools"] = json!(openai_tools);
        }
    }

    // 6. tool_choice 转换
    if let Some(tc) = body.get("tool_choice") {
        let openai_tc = match tc.get("type").and_then(|v| v.as_str()) {
            Some("auto") => json!("auto"),
            Some("any") => json!("required"),
            Some("tool") => {
                if let Some(name) = tc.get("name") {
                    json!({"type": "function", "function": {"name": name}})
                } else {
                    json!("auto")
                }
            }
            _ => json!("auto"),
        };
        openai["tool_choice"] = openai_tc;
    }

    // 7. 并行工具调用：Anthropic 用 `tool_choice.disable_parallel_tool_use` 表达「不要并行」，
    //    OpenAI 是顶层 `parallel_tool_calls`。不映射时上游可能一次回多个 tool_use，
    //    客户端按单工具解析就会丢掉后续调用（参考 cc-switch「enable parallel tool calls」）。
    if body
        .get("tool_choice")
        .and_then(|tc| tc.get("disable_parallel_tool_use"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        openai["parallel_tool_calls"] = json!(false);
    } else if let Some(parallel) = body.get("parallel_tool_calls").and_then(|v| v.as_bool()) {
        // 少数客户端直接把 OpenAI 风格开关塞进 Anthropic 请求体：出现时以它为准
        openai["parallel_tool_calls"] = json!(parallel);
    }

    // 8. reasoning effort：Anthropic 的 `thinking` / `output_config.effort` 在 OpenAI 侧
    //    没有等价块，只有顶层 `reasoning_effort` 一个落点。不映射时用户的 effort 意图
    //    （含 Claude Code `/effort max`）会静默丢失，gpt-5+/grok-4.x 上表现为「档位永远默认」
    //    （抄自 cc-switch 8e478b2b / d6e05152）。
    //    仅当入站真的表达了 effort、且目标模型支持该参数时写入；
    //    未表达时留给 optimizers::optimize_thinking_openai 按家族兜底。
    if super::optimizers::supports_reasoning_effort(&resolved_model) {
        if let Some(effort) = super::optimizers::resolve_reasoning_effort(&resolved_model, body) {
            openai["reasoning_effort"] = json!(effort);
        }
    }

    openai
}

/// 转换单条 Anthropic 消息为 OpenAI 格式（可能产生多条消息）
fn convert_anthropic_message(msg: &Value, output: &mut Vec<Value>) {
    let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("user");
    let content = match msg.get("content") {
        Some(Value::String(s)) => {
            output.push(json!({"role": role, "content": s}));
            return;
        }
        Some(Value::Array(arr)) => arr,
        _ => return,
    };

    match role {
        "assistant" => {
            let mut text_parts = Vec::new();
            let mut tool_calls = Vec::new();
            let mut reasoning_parts = Vec::new();

            for part in content {
                match part.get("type").and_then(|v| v.as_str()) {
                    Some("text") => {
                        if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                            text_parts.push(text.to_string());
                        }
                    }
                    Some("thinking") => {
                        if let Some(thinking) = part.get("thinking").and_then(|v| v.as_str()) {
                            reasoning_parts.push(thinking.to_string());
                        }
                    }
                    Some("tool_use") => {
                        let id = part.get("id").cloned().unwrap_or(json!(""));
                        let name = part.get("name").cloned().unwrap_or(json!(""));
                        let input = part.get("input").cloned().unwrap_or(json!({}));
                        let args_str = serde_json::to_string(&input).unwrap_or_else(|_| "{}".to_string());
                        tool_calls.push(json!({
                            "id": id,
                            "type": "function",
                            "function": {
                                "name": name,
                                "arguments": args_str
                            }
                        }));
                    }
                    _ => {}
                }
            }

            let mut assistant_msg = json!({"role": "assistant"});
            if !text_parts.is_empty() {
                assistant_msg["content"] = json!(text_parts.join("\n"));
            }
            if !reasoning_parts.is_empty() {
                assistant_msg["reasoning_content"] = json!(reasoning_parts.join("\n"));
            }
            if !tool_calls.is_empty() {
                assistant_msg["tool_calls"] = json!(tool_calls);
            }
            output.push(assistant_msg);
        }
        "user" => {
            // 收集所有 content parts
            let mut openai_content = Vec::new();
            for part in content {
                match part.get("type").and_then(|v| v.as_str()) {
                    Some("text") => {
                        if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                            openai_content.push(json!({"type": "text", "text": text}));
                        }
                    }
                    Some("image") => {
                        if let (Some(media_type), Some(data)) = (
                            part.get("media_type").and_then(|v| v.as_str()),
                            part.get("data").and_then(|v| v.as_str()),
                        ) {
                            openai_content.push(json!({
                                "type": "image_url",
                                "image_url": {"url": format!("data:{};base64,{}", media_type, data)}
                            }));
                        }
                    }
                    Some("tool_result") => {
                        // tool_result 需要作为独立的 tool 消息
                        let tool_use_id = part.get("tool_use_id").cloned().unwrap_or(json!(""));
                        let result_content = match part.get("content") {
                            Some(Value::String(s)) => s.clone(),
                            Some(Value::Array(arr)) => {
                                arr.iter()
                                    .filter_map(|p| p.get("text").and_then(|v| v.as_str()))
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            }
                            _ => String::new(),
                        };
                        output.push(json!({
                            "role": "tool",
                            "tool_call_id": tool_use_id,
                            "content": result_content
                        }));
                        continue; // 不作为 user 消息的一部分
                    }
                    _ => {}
                }
            }

            if !openai_content.is_empty() {
                // 如果只有一个 text part，简化为纯字符串
                if openai_content.len() == 1 && openai_content[0].get("type").and_then(|v| v.as_str()) == Some("text") {
                    output.push(json!({
                        "role": "user",
                        "content": openai_content[0].get("text").unwrap_or(&json!(""))
                    }));
                } else {
                    output.push(json!({"role": "user", "content": openai_content}));
                }
            }
        }
        _ => {}
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  OpenAI → Anthropic
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 将 OpenAI Chat Completions 响应转换为 Anthropic Messages API 响应
pub fn openai_response_to_anthropic(openai_resp: &Value, request_model: &str) -> Value {
    let choice = openai_resp
        .get("choices")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first());

    let message = choice.and_then(|c| c.get("message"));
    let finish_reason = choice
        .and_then(|c| c.get("finish_reason").and_then(|v| v.as_str()))
        .unwrap_or("stop");

    let mut content = Vec::new();

    // reasoning_content → thinking block
    if let Some(reasoning) = message.and_then(|m| m.get("reasoning_content").and_then(|v| v.as_str())) {
        if !reasoning.is_empty() {
            content.push(json!({"type": "thinking", "thinking": reasoning}));
        }
    }

    // content → text block（顺带剥离内联 think 标签成 thinking block）
    if let Some(text) = message.and_then(|m| m.get("content").and_then(|v| v.as_str())) {
        if !text.is_empty() {
            if let Some((thinking, answer)) =
                crate::proxy::inline_think::split_leading_think_block(text)
            {
                if !thinking.is_empty() {
                    content.push(json!({"type": "thinking", "thinking": thinking}));
                }
                if !answer.is_empty() {
                    content.push(json!({"type": "text", "text": answer}));
                }
            } else {
                content.push(json!({"type": "text", "text": text}));
            }
        }
    }

    // tool_calls → tool_use blocks
    if let Some(tool_calls) = message.and_then(|m| m.get("tool_calls").and_then(|v| v.as_array())) {
        for tc in tool_calls {
            let id = tc.get("id").cloned().unwrap_or(json!(""));
            let name = tc
                .get("function")
                .and_then(|f| f.get("name"))
                .cloned()
                .unwrap_or(json!(""));
            let args_str = tc
                .get("function")
                .and_then(|f| f.get("arguments").and_then(|v| v.as_str()))
                .unwrap_or("{}");
            let args: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
            content.push(json!({
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": args
            }));
        }
    }

    // stop_reason 映射
    let stop_reason = match finish_reason {
        "stop" => "end_turn",
        "length" => "max_tokens",
        "tool_calls" | "function_call" => "tool_use",
        _ => "end_turn",
    };

    // usage 映射
    let usage = openai_resp.get("usage");
    let input_tokens = usage
        .and_then(|u| u.get("prompt_tokens").and_then(|v| v.as_u64()))
        .unwrap_or(0);
    let output_tokens = usage
        .and_then(|u| u.get("completion_tokens").and_then(|v| v.as_u64()))
        .unwrap_or(0);

    json!({
        "id": format!("msg_{}", openai_resp.get("id").and_then(|v| v.as_str()).unwrap_or("0")),
        "type": "message",
        "role": "assistant",
        "content": content,
        "model": request_model,
        "stop_reason": stop_reason,
        "stop_sequence": null,
        "usage": {
            "input_tokens": input_tokens,
            "output_tokens": output_tokens
        }
    })
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  SSE 流式转换：OpenAI chunk → Anthropic SSE
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 流式转换器状态
pub struct StreamConverter {
    message_id: String,
    model: String,
    input_tokens: u64,
    output_tokens: u64,
    started: bool,
    /// 已发出 `message_stop`。置位后本转换器不再产出任何事件：
    /// 流已经终止后再写 error 事件会产出畸形流（Claude Code 等客户端按事件序列解析）。
    terminated: bool,
    /// 「下一个要分配」的 content block 索引。只在真正开块时自增。
    next_index: usize,
    /// 当前打开的 text / thinking 块的索引（`None` = 没有打开）。
    /// 与 `next_index` 分开是必须的：曾把两者混为一个指针，导致开块后自增，
    /// 其后的 delta 与收尾 stop 全部错位一位，块 0 永不闭合。
    open_index: Option<usize>,
    current_tool_calls: Vec<ToolCallState>,
}

struct ToolCallState {
    id: String,
    name: String,
    arguments: String,
    /// 该 tool_use 块自己的 `content_block_start` 索引；`None` = 尚未开块。
    open_index: Option<usize>,
}

impl StreamConverter {
    pub fn new(model: String) -> Self {
        Self {
            message_id: format!("msg_stream_{}", chrono::Utc::now().timestamp_millis()),
            model,
            input_tokens: 0,
            output_tokens: 0,
            started: false,
            terminated: false,
            next_index: 0,
            open_index: None,
            current_tool_calls: Vec::new(),
        }
    }

    /// 关闭当前打开的 text / thinking 块（若有）。
    fn close_open_text_block(&mut self, events: &mut Vec<String>) {
        if let Some(idx) = self.open_index.take() {
            events.push(format!(
                "event: content_block_stop\ndata: {}\n\n",
                json!({"type": "content_block_stop", "index": idx})
            ));
        }
    }

    /// 关闭所有已开块的 tool_use（按开块顺序）。
    fn close_tool_blocks(&mut self, events: &mut Vec<String>) {
        for state in self.current_tool_calls.iter_mut() {
            if let Some(idx) = state.open_index.take() {
                events.push(format!(
                    "event: content_block_stop\ndata: {}\n\n",
                    json!({"type": "content_block_stop", "index": idx})
                ));
            }
        }
    }

    /// 异常路径用：只闭合已打开的块（text/thinking + 每个已开块的 tool_use），
    /// **不发** `message_delta` / `message_stop` —— 上游出错时不能声称「正常结束」，
    /// 闭合后由调用方发一条 error 事件，客户端在 error 处终止解析。
    pub fn close_open_blocks(&mut self) -> Vec<String> {
        let mut events = Vec::new();
        self.close_open_text_block(&mut events);
        self.close_tool_blocks(&mut events);
        events
    }

    /// 收尾：补齐所有未闭合的块，再发 `message_delta` + `message_stop`。
    /// 正常路径由 `finish_reason` 触发；异常路径（上游断流 / 无 `finish_reason` /
    /// 超时）必须由调用方显式调用本方法补收尾，否则客户端拿到未闭合的块。
    /// 幂等：已终止时什么都不做。
    pub fn finish(&mut self) -> Vec<String> {
        let mut events = Vec::new();
        if self.terminated {
            return events;
        }
        // 连 message_start 都没发过就中断：发终止事件反而会让客户端困惑，直接不产出
        if !self.started {
            self.terminated = true;
            return events;
        }
        self.close_open_text_block(&mut events);
        self.close_tool_blocks(&mut events);
        self.emit_stop_events(&mut events, "end_turn");
        events
    }

    /// 发 `message_delta` + `message_stop` 并置 `terminated`。调用方须已闭合所有块。
    fn emit_stop_events(&mut self, events: &mut Vec<String>, stop_reason: &str) {
        if self.terminated {
            return;
        }
        let delta_ev = json!({
            "type": "message_delta",
            "delta": {
                "stop_reason": stop_reason,
                "stop_sequence": null
            },
            "usage": {
                "output_tokens": self.output_tokens
            }
        });
        events.push(format!("event: message_delta\ndata: {}\n\n", delta_ev));
        events.push("event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n".to_string());
        self.terminated = true;
    }

    /// 返回累积的 token 用量
    pub fn usage(&self) -> (u64, u64) {
        (self.input_tokens, self.output_tokens)
    }
    /// 返回请求的模型名
    pub fn model_name(&self) -> &str {
        &self.model
    }
    /// 是否已发出终止事件（供调用方判断能否再写事件）
    pub fn is_terminated(&self) -> bool {
        self.terminated
    }

    /// 将一个 OpenAI SSE chunk 转换为零或多个 Anthropic SSE 事件
    pub fn convert_chunk(&mut self, chunk: &Value) -> Vec<String> {
        let mut events = Vec::new();

        // 流已终止：吞掉后续所有 chunk，绝不再产出事件（A3）
        if self.terminated {
            return events;
        }

        // 处理 usage（出现在最后一个 chunk）
        if let Some(usage) = chunk.get("usage") {
            if let Some(pt) = usage.get("prompt_tokens").and_then(|v| v.as_u64()) {
                self.input_tokens = pt;
            }
            if let Some(ct) = usage.get("completion_tokens").and_then(|v| v.as_u64()) {
                self.output_tokens = ct;
            }
        }

        let choices = match chunk.get("choices").and_then(|v| v.as_array()) {
            Some(c) => c,
            None => return events, // usage-only chunk
        };

        let choice = match choices.first() {
            Some(c) => c,
            None => return events,
        };

        let delta = match choice.get("delta") {
            Some(d) => d,
            None => return events,
        };

        // 首个 chunk：发送 message_start
        if !self.started {
            self.started = true;
            let start_event = json!({
                "type": "message_start",
                "message": {
                    "id": self.message_id,
                    "type": "message",
                    "role": "assistant",
                    "content": [],
                    "model": self.model,
                    "stop_reason": null,
                    "stop_sequence": null,
                    "usage": {"input_tokens": self.input_tokens, "output_tokens": 0}
                }
            });
            events.push(format!("event: message_start\ndata: {}\n\n", start_event));
        }

        // reasoning_content → thinking delta
        if let Some(reasoning) = delta.get("reasoning_content").and_then(|v| v.as_str()) {
            if !reasoning.is_empty() {
                let open = match self.open_index {
                    Some(i) => i,
                    None => {
                        self.close_open_text_block(&mut events);
                        let idx = self.next_index;
                        self.next_index += 1;
                        self.open_index = Some(idx);
                        let ev = json!({
                            "type": "content_block_start",
                            "index": idx,
                            "content_block": {"type": "thinking", "thinking": ""}
                        });
                        events.push(format!("event: content_block_start\ndata: {}\n\n", ev));
                        idx
                    }
                };
                let delta_ev = json!({
                    "type": "content_block_delta",
                    "index": open,
                    "delta": {"type": "thinking_delta", "thinking": reasoning}
                });
                events.push(format!("event: content_block_delta\ndata: {}\n\n", delta_ev));
            }
        }

        // content → text delta
        if let Some(content) = delta.get("content").and_then(|v| v.as_str()) {
            if !content.is_empty() {
                let open = match self.open_index {
                    Some(i) => i,
                    None => {
                        self.close_open_text_block(&mut events);
                        let idx = self.next_index;
                        self.next_index += 1;
                        self.open_index = Some(idx);
                        let ev = json!({
                            "type": "content_block_start",
                            "index": idx,
                            "content_block": {"type": "text", "text": ""}
                        });
                        events.push(format!("event: content_block_start\ndata: {}\n\n", ev));
                        idx
                    }
                };
                let delta_ev = json!({
                    "type": "content_block_delta",
                    "index": open,
                    "delta": {"type": "text_delta", "text": content}
                });
                events.push(format!("event: content_block_delta\ndata: {}\n\n", delta_ev));
            }
        }

        // tool_calls delta
        if let Some(tool_calls) = delta.get("tool_calls").and_then(|v| v.as_array()) {
            // 关闭已打开的 text/thinking block（Anthropic 同一时刻只允许一个打开）
            self.close_open_text_block(&mut events);

            for tc in tool_calls {
                let tc_index = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

                // 确保有足够的 tool call state
                while self.current_tool_calls.len() <= tc_index {
                    self.current_tool_calls.push(ToolCallState {
                        id: String::new(),
                        name: String::new(),
                        arguments: String::new(),
                        open_index: None,
                    });
                }

                let state = &mut self.current_tool_calls[tc_index];

                // 开块用「刚分配」的索引，delta 与收尾 stop 都用 state 记下的这个索引 ——
                // 绝不能靠共享的 content_index 推算，否则并行多工具时必然错位。
                if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                    state.id = id.to_string();
                    let name = tc
                        .get("function")
                        .and_then(|f| f.get("name").and_then(|v| v.as_str()))
                        .unwrap_or("");
                    state.name = name.to_string();

                    // 同一 tc_index 重复带 id（部分上游会重发）时沿用原索引，不重复开块
                    if state.open_index.is_none() {
                        let idx = self.next_index;
                        self.next_index += 1;
                        state.open_index = Some(idx);
                        let ev = json!({
                            "type": "content_block_start",
                            "index": idx,
                            "content_block": {
                                "type": "tool_use",
                                "id": state.id,
                                "name": state.name,
                                "input": {}
                            }
                        });
                        events.push(format!("event: content_block_start\ndata: {}\n\n", ev));
                    }
                }

                if let Some(args_delta) = tc
                    .get("function")
                    .and_then(|f| f.get("arguments").and_then(|v| v.as_str()))
                {
                    state.arguments.push_str(args_delta);
                    // 没有开过块的（例如 padding 出来的无 id state）不能发 delta：
                    // 那会把内容挂到一个客户端从未见过的 index 上。
                    if let Some(idx) = state.open_index {
                        let delta_ev = json!({
                            "type": "content_block_delta",
                            "index": idx,
                            "delta": {
                                "type": "input_json_delta",
                                "partial_json": args_delta
                            }
                        });
                        events.push(format!("event: content_block_delta\ndata: {}\n\n", delta_ev));
                    }
                }
            }
        }

        // finish_reason → message_delta + message_stop
        if let Some(finish_reason) = choice.get("finish_reason").and_then(|v| v.as_str()) {
            // 关闭所有未关闭的块（text/thinking + 每个已开块的 tool_use）
            self.close_open_text_block(&mut events);
            self.close_tool_blocks(&mut events);

            let stop_reason = match finish_reason {
                "stop" => "end_turn",
                "length" => "max_tokens",
                "tool_calls" | "function_call" => "tool_use",
                _ => "end_turn",
            };
            self.emit_stop_events(&mut events, stop_reason);
        }

        events
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  OpenAI → Anthropic Request & Response Translation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 将 OpenAI Chat Completions 请求体转换为 Anthropic Messages 请求体
pub fn openai_to_anthropic(body: &Value, target_model: &str, aliases: Option<&ModelAliases>) -> Value {
    let mut messages = Vec::new();
    let mut system_text = String::new();

    let request_model = body.get("model").and_then(|v| v.as_str()).unwrap_or(target_model);
    let resolved_model = if let Some(a) = aliases {
        map_model_name(request_model, a)
    } else {
        target_model.to_string()
    };

    if let Some(msgs) = body.get("messages").and_then(|v| v.as_array()) {
        for msg in msgs {
            let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("user");
            let content = msg.get("content");
            if role == "system" {
                if let Some(c) = content {
                    let text = match c {
                        Value::String(s) => s.clone(),
                        Value::Array(arr) => arr
                            .iter()
                            .filter_map(|part| {
                                if part.get("type").and_then(|v| v.as_str()) == Some("text") {
                                    part.get("text").and_then(|v| v.as_str()).map(String::from)
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n"),
                        _ => String::new(),
                    };
                    if !system_text.is_empty() {
                        system_text.push_str("\n");
                    }
                    system_text.push_str(&text);
                }
            } else {
                let anthropic_role = if role == "assistant" { "assistant" } else { "user" };
                if let Some(c) = content {
                    let converted_content = match c {
                        Value::String(s) => json!(s),
                        Value::Array(arr) => {
                            let parts: Vec<Value> = arr
                                .iter()
                                .map(|part| {
                                    let part_type = part.get("type").and_then(|v| v.as_str()).unwrap_or("text");
                                    if part_type == "image_url" {
                                        if let Some(url) = part.get("image_url").and_then(|v| v.get("url").and_then(|u| u.as_str())) {
                                            if url.starts_with("data:") {
                                                if let Some(comma_idx) = url.find(',') {
                                                    let media_type = url[5..comma_idx].split(';').next().unwrap_or("image/jpeg");
                                                    let base64_data = &url[comma_idx + 1..];
                                                    return json!({
                                                        "type": "image",
                                                        "source": {
                                                            "type": "base64",
                                                            "media_type": media_type,
                                                            "data": base64_data
                                                        }
                                                    });
                                                }
                                            }
                                        }
                                    }
                                    json!({
                                        "type": "text",
                                        "text": part.get("text").unwrap_or(&json!(""))
                                    })
                                })
                                .collect();
                            json!(parts)
                        }
                        _ => json!(""),
                    };
                    messages.push(json!({
                        "role": anthropic_role,
                        "content": converted_content
                    }));
                }
            }
        }
    }

    let mut anthropic = json!({
        "model": resolved_model,
        "messages": messages,
    });

    if !system_text.is_empty() {
        anthropic["system"] = json!(system_text);
    }

    let max_tokens = body.get("max_completion_tokens")
        .or_else(|| body.get("max_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(4096);
    anthropic["max_tokens"] = json!(max_tokens);

    if let Some(temp) = body.get("temperature").and_then(|v| v.as_f64()) {
        anthropic["temperature"] = json!(temp);
    }
    if let Some(top_p) = body.get("top_p").and_then(|v| v.as_f64()) {
        anthropic["top_p"] = json!(top_p);
    }
    if let Some(stream) = body.get("stream").and_then(|v| v.as_bool()) {
        anthropic["stream"] = json!(stream);
    }

    // tools 转换
    if let Some(tools) = body.get("tools").and_then(|v| v.as_array()) {
        let anthropic_tools: Vec<Value> = tools
            .iter()
            .filter_map(|t| {
                if t.get("type").and_then(|v| v.as_str()) == Some("function") {
                    if let Some(func) = t.get("function") {
                        return Some(json!({
                            "name": func.get("name").unwrap_or(&json!("")),
                            "description": func.get("description").unwrap_or(&json!("")),
                            "input_schema": super::normalize_function_parameters(func.get("parameters"))
                        }));
                    }
                }
                None
            })
            .collect();
        if !anthropic_tools.is_empty() {
            anthropic["tools"] = json!(anthropic_tools);
        }
    }

    // tool_choice 转换：此前**完全没有映射** —— 客户端说「必须调用工具（required）」
    // 会被静默降级成自由发挥，输出可能压根不调工具。
    // Anthropic 不接受没有 tools 的 tool_choice，因此只在有 tools 时写入。
    if anthropic.get("tools").is_some() {
        let parallel = body.get("parallel_tool_calls").and_then(|v| v.as_bool());
        let mut anthropic_tc = match body.get("tool_choice") {
            Some(Value::String(kind)) => match kind.as_str() {
                "required" => json!({"type": "any"}),
                "none" => json!({"type": "none"}),
                _ => json!({"type": "auto"}),
            },
            Some(Value::Object(obj)) => {
                if obj.get("type").and_then(|v| v.as_str()) == Some("function") {
                    let name = obj
                        .get("function")
                        .and_then(|f| f.get("name"))
                        .cloned()
                        .unwrap_or_else(|| json!(""));
                    json!({"type": "tool", "name": name})
                } else {
                    json!({"type": "auto"})
                }
            }
            _ => {
                // 没给（或给了 null）tool_choice：只有明确要求「不要并行」时才补一个承载对象，
                // 否则不写该字段，保持上游默认行为。
                if parallel == Some(false) {
                    json!({"type": "auto"})
                } else {
                    return anthropic;
                }
            }
        };
        // 并行开关：OpenAI 顶层 `parallel_tool_calls: false` → Anthropic 的
        // `tool_choice.disable_parallel_tool_use: true`（cc-switch 的
        //「enable parallel tool calls / preserve parallel tool history」同域问题）。
        if parallel == Some(false) {
            anthropic_tc["disable_parallel_tool_use"] = json!(true);
        }
        anthropic["tool_choice"] = anthropic_tc;
    }

    anthropic
}

/// 将 Anthropic Messages 响应转换为 OpenAI Chat Completions 响应
pub fn anthropic_response_to_openai(anthropic_resp: &Value, request_model: &str) -> Value {
    let content = anthropic_resp.get("content").and_then(|v| v.as_array());
    let stop_reason = anthropic_resp.get("stop_reason").and_then(|v| v.as_str()).unwrap_or("end_turn");

    let finish_reason = match stop_reason {
        "end_turn" => "stop",
        "max_tokens" => "length",
        "tool_use" => "tool_calls",
        _ => "stop",
    };

    let mut message_content = String::new();
    let mut tool_calls = Vec::new();

    if let Some(parts) = content {
        for part in parts {
            let part_type = part.get("type").and_then(|v| v.as_str()).unwrap_or("text");
            if part_type == "text" {
                if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                    message_content.push_str(text);
                }
            } else if part_type == "tool_use" {
                let id = part.get("id").cloned().unwrap_or(json!(""));
                let name = part.get("name").cloned().unwrap_or(json!(""));
                let input = part.get("input").cloned().unwrap_or(json!({}));
                let args_str = serde_json::to_string(&input).unwrap_or_else(|_| "{}".to_string());
                tool_calls.push(json!({
                    "id": id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": args_str
                    }
                }));
            }
        }
    }

    let mut message = json!({
        "role": "assistant"
    });
    if !message_content.is_empty() {
        message["content"] = json!(message_content);
    } else {
        message["content"] = Value::Null;
    }
    if !tool_calls.is_empty() {
        message["tool_calls"] = json!(tool_calls);
    }

    let usage = anthropic_resp.get("usage");
    let input_tokens = usage.and_then(|u| u.get("input_tokens").and_then(|v| v.as_u64())).unwrap_or(0);
    let output_tokens = usage.and_then(|u| u.get("output_tokens").and_then(|v| v.as_u64())).unwrap_or(0);

    json!({
        "id": format!("chatcmpl-{}", anthropic_resp.get("id").and_then(|v| v.as_str()).unwrap_or("0")),
        "object": "chat.completion",
        "created": chrono::Utc::now().timestamp(),
        "model": request_model,
        "choices": [
            {
                "index": 0,
                "message": message,
                "logprobs": null,
                "finish_reason": finish_reason
            }
        ],
        "usage": {
            "prompt_tokens": input_tokens,
            "completion_tokens": output_tokens,
            "total_tokens": input_tokens + output_tokens
        }
    })
}

/// Anthropic SSE 流式 chunk 转换为 OpenAI chunk 转换器
pub struct AnthropicToOpenaiStreamConverter {
    id: String,
    model: String,
    _started: bool,
    input_tokens: u64,
    output_tokens: u64,
    /// 已发出终止事件；置位后不再产出任何事件
    terminated: bool,
}

impl AnthropicToOpenaiStreamConverter {
    pub fn new(model: String) -> Self {
        Self {
            id: format!("chatcmpl-{}", chrono::Utc::now().timestamp_millis()),
            model,
            _started: false,
            input_tokens: 0,
            output_tokens: 0,
            terminated: false,
        }
    }

    pub fn convert_event(&mut self, event_type: &str, data: &Value) -> Option<String> {
        match event_type {
            "message_start" => {
                if let Some(msg) = data.get("message") {
                    if let Some(usage) = msg.get("usage") {
                        self.input_tokens = usage.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                    }
                }
                None
            }
            "content_block_delta" => {
                if let Some(delta) = data.get("delta") {
                    let delta_type = delta.get("type").and_then(|v| v.as_str()).unwrap_or("text_delta");
                    let mut choice_delta = json!({});

                    if delta_type == "text_delta" {
                        if let Some(text) = delta.get("text").and_then(|v| v.as_str()) {
                            choice_delta["content"] = json!(text);
                        }
                    } else if delta_type == "input_json_delta" {
                        if let Some(partial) = delta.get("partial_json").and_then(|v| v.as_str()) {
                            let idx = data.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                            choice_delta["tool_calls"] = json!([{
                                "index": idx,
                                "function": {
                                    "arguments": partial
                                }
                            }]);
                        }
                    }

                    let chunk = json!({
                        "id": self.id,
                        "object": "chat.completion.chunk",
                        "created": chrono::Utc::now().timestamp(),
                        "model": self.model,
                        "choices": [
                            {
                                "index": 0,
                                "delta": choice_delta,
                                "logprobs": null,
                                "finish_reason": null
                            }
                        ]
                    });
                    Some(format!("data: {}\n\n", serde_json::to_string(&chunk).unwrap()))
                } else {
                    None
                }
            }
            "content_block_start" => {
                if let Some(block) = data.get("content_block") {
                    let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    if block_type == "tool_use" {
                        let idx = data.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                        let tool_id = block.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        let chunk = json!({
                            "id": self.id,
                            "object": "chat.completion.chunk",
                            "created": chrono::Utc::now().timestamp(),
                            "model": self.model,
                            "choices": [
                                {
                                    "index": 0,
                                    "delta": {
                                        "tool_calls": [{
                                            "index": idx,
                                            "id": tool_id,
                                            "type": "function",
                                            "function": {
                                                "name": name,
                                                "arguments": ""
                                            }
                                        }]
                                    },
                                    "logprobs": null,
                                    "finish_reason": null
                                }
                            ]
                        });
                        return Some(format!("data: {}\n\n", serde_json::to_string(&chunk).unwrap()));
                    }
                }
                None
            }
            "message_delta" => {
                if let Some(usage) = data.get("usage") {
                    self.output_tokens = usage.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                }
                let stop_reason = data.get("delta").and_then(|d| d.get("stop_reason").and_then(|v| v.as_str())).unwrap_or("end_turn");
                let finish_reason = match stop_reason {
                    "end_turn" => "stop",
                    "max_tokens" => "length",
                    "tool_use" => "tool_calls",
                    _ => "stop",
                };
                let chunk = json!({
                    "id": self.id,
                    "object": "chat.completion.chunk",
                    "created": chrono::Utc::now().timestamp(),
                    "model": self.model,
                    "choices": [
                        {
                            "index": 0,
                            "delta": {},
                            "logprobs": null,
                            "finish_reason": finish_reason
                        }
                    ],
                    "usage": {
                        "prompt_tokens": self.input_tokens,
                        "completion_tokens": self.output_tokens,
                        "total_tokens": self.input_tokens + self.output_tokens
                    }
                });
                Some(format!("data: {}\n\n", serde_json::to_string(&chunk).unwrap()))
            }
            "message_stop" => {
                Some("data: [DONE]\n\n".to_string())
            }
            _ => None
        }
    }

    pub fn usage(&self) -> (u64, u64) {
        (self.input_tokens, self.output_tokens)
    }

    pub fn model_name(&self) -> &str {
        &self.model
    }

    /// 兼容统一管线：把上游 Anthropic SSE `data:` 块（含 `type` 字段）转换为 OpenAI SSE 行向量。
    pub fn convert_chunk(&mut self, chunk: &Value) -> Vec<String> {
        if self.terminated {
            return Vec::new();
        }
        if let Some(et) = chunk.get("type").and_then(|v| v.as_str()) {
            if et == "message_stop" {
                self.terminated = true;
            }
            if let Some(s) = self.convert_event(et, chunk) {
                return vec![s];
            }
        }
        Vec::new()
    }

    /// 是否已发出终止事件
    pub fn is_terminated(&self) -> bool {
        self.terminated
    }

    /// 异常路径用：OpenAI chunk 无 content block 概念，无内容可闭合
    pub fn close_open_blocks(&mut self) -> Vec<String> {
        Vec::new()
    }

    /// 收尾：OpenAI Chat Completions 的终止标记是 `data: [DONE]`。幂等。
    pub fn finish(&mut self) -> Vec<String> {
        if self.terminated {
            return Vec::new();
        }
        self.terminated = true;
        vec!["data: [DONE]\n\n".to_string()]
    }
}

#[cfg(test)]
mod stream_block_tests {
    use super::StreamConverter;
    use serde_json::{json, Value};

    /// 把转换器产出的一行 SSE 文本解析成 (event 名, data JSON)。
    fn parse_line(raw: &str) -> Option<(String, Value)> {
        let mut name = String::new();
        let mut data = String::new();
        for line in raw.lines() {
            if let Some(rest) = line.strip_prefix("event:") {
                name = rest.trim().to_string();
            } else if let Some(rest) = line.strip_prefix("data:") {
                data = rest.trim().to_string();
            }
        }
        if data.is_empty() {
            return None;
        }
        serde_json::from_str::<Value>(&data).ok().map(|v| (name, v))
    }

    /// 收集所有事件的 (type, index)。
    fn events_of(raw: &[String]) -> Vec<(String, i64)> {
        raw.iter()
            .filter_map(|e| parse_line(e))
            .map(|(name, v)| {
                let ty = v
                    .get("type")
                    .and_then(|x| x.as_str())
                    .unwrap_or(&name)
                    .to_string();
                let idx = v.get("index").and_then(|x| x.as_i64()).unwrap_or(-1);
                (ty, idx)
            })
            .collect()
    }

    fn chunk(delta: Value, finish: Option<&str>) -> Value {
        let mut choice = json!({"index": 0, "delta": delta, "finish_reason": finish});
        if finish.is_some() {
            choice["finish_reason"] = json!(finish.unwrap());
        }
        json!({"choices": [choice]})
    }

    /// A1 回归：tool_use 的 start / delta / stop 必须落在**同一个** index 上。
    /// 修复前：start=0、delta=1、stop=1 → 块 0 永不闭合、凭空多一个未 start 的块。
    #[test]
    fn tool_use_block_start_delta_stop_share_one_index() {
        let mut c = StreamConverter::new("deepseek-chat".into());

        let mut out = c.convert_chunk(&chunk(
            json!({"tool_calls": [{
                "index": 0,
                "id": "call_1",
                "function": {"name": "read_file", "arguments": "{\"path\":"}
            }]}),
            None,
        ));
        out.extend(c.convert_chunk(&chunk(
            json!({"tool_calls": [{
                "index": 0,
                "function": {"arguments": "\"a.rs\"}"}
            }]}),
            None,
        )));
        out.extend(c.convert_chunk(&chunk(json!({}), Some("tool_calls"))));

        let evs = events_of(&out);
        let starts: Vec<i64> = evs
            .iter()
            .filter(|(t, _)| t == "content_block_start")
            .map(|(_, i)| *i)
            .collect();
        let deltas: Vec<i64> = evs
            .iter()
            .filter(|(t, _)| t == "content_block_delta")
            .map(|(_, i)| *i)
            .collect();
        let stops: Vec<i64> = evs
            .iter()
            .filter(|(t, _)| t == "content_block_stop")
            .map(|(_, i)| *i)
            .collect();

        assert_eq!(starts, vec![0], "只应开一个块，实际 {starts:?}");
        assert_eq!(deltas, vec![0, 0], "两个 delta 都要挂在块 0，实际 {deltas:?}");
        assert_eq!(stops, vec![0], "块 0 必须闭合，实际 {stops:?}");
        assert!(evs.iter().any(|(t, _)| t == "message_stop"));
    }

    /// A1 回归：并行多个 tool_call 各自成块，互不串号，且都闭合。
    #[test]
    fn parallel_tool_calls_each_get_own_block_and_close() {
        let mut c = StreamConverter::new("deepseek-chat".into());
        let mut out = c.convert_chunk(&chunk(
            json!({"tool_calls": [
                {"index": 0, "id": "call_a", "function": {"name": "f", "arguments": "{}"}},
                {"index": 1, "id": "call_b", "function": {"name": "g", "arguments": "{}"}}
            ]}),
            None,
        ));
        out.extend(c.convert_chunk(&chunk(json!({}), Some("tool_calls"))));

        let evs = events_of(&out);
        let starts: Vec<i64> = evs.iter().filter(|(t, _)| t == "content_block_start").map(|(_, i)| *i).collect();
        let stops: Vec<i64> = evs.iter().filter(|(t, _)| t == "content_block_stop").map(|(_, i)| *i).collect();
        assert_eq!(starts, vec![0, 1], "两个工具应各开一块，实际 {starts:?}");
        assert_eq!(stops, vec![0, 1], "两个块都要闭合，实际 {stops:?}");
    }

    /// A1 回归：先出文本再出工具 —— 文本块先闭合，工具块另起 index。
    #[test]
    fn text_block_closes_before_tool_block_opens() {
        let mut c = StreamConverter::new("deepseek-chat".into());
        let mut out = c.convert_chunk(&chunk(json!({"content": "我看一下"}), None));
        out.extend(c.convert_chunk(&chunk(
            json!({"tool_calls": [{"index": 0, "id": "c1", "function": {"name": "f", "arguments": "{}"}}]}),
            None,
        )));
        out.extend(c.convert_chunk(&chunk(json!({}), Some("tool_calls"))));

        let seq: Vec<(String, i64)> = events_of(&out)
            .into_iter()
            .filter(|(t, _)| t.starts_with("content_block"))
            .collect();
        assert_eq!(
            seq,
            vec![
                ("content_block_start".into(), 0),
                ("content_block_delta".into(), 0),
                ("content_block_stop".into(), 0),
                ("content_block_start".into(), 1),
                ("content_block_delta".into(), 1),
                ("content_block_stop".into(), 1),
            ],
            "实际序列 {seq:?}"
        );
    }

    /// A2 回归：上游没发 finish_reason 就断了，`finish()` 必须补齐闭合 + 终止事件。
    #[test]
    fn finish_closes_open_blocks_after_truncated_stream() {
        let mut c = StreamConverter::new("deepseek-chat".into());
        // 只发一个带工具调用的 chunk，没有 finish_reason，也没有后续
        let _ = c.convert_chunk(&chunk(
            json!({"tool_calls": [{"index": 0, "id": "c1", "function": {"name": "f", "arguments": "{"}}]}),
            None,
        ));
        assert!(!c.is_terminated(), "尚未终止");

        let tail = c.finish();
        let ty: Vec<String> = events_of(&tail).into_iter().map(|(t, _)| t).collect();
        assert!(ty.contains(&"content_block_stop".to_string()), "缺闭合：{ty:?}");
        assert!(ty.contains(&"message_delta".to_string()), "缺终止：{ty:?}");
        assert_eq!(ty.last().map(String::as_str), Some("message_stop"), "终止事件必须是最后一个：{ty:?}");
        assert!(c.is_terminated());
        // 幂等
        assert!(c.finish().is_empty());
    }

    /// A2 回归：finish() 补收尾时也不能重复发闭合（open_index 已被取走）。
    #[test]
    fn finish_is_idempotent_and_never_double_closes() {
        let mut c = StreamConverter::new("m".into());
        let _ = c.convert_chunk(&chunk(json!({"content": "hello"}), None));
        let first = c.finish();
        let second = c.finish();
        assert!(second.is_empty(), "二次 finish 必须为空：{second:?}");
        let stops = events_of(&first).into_iter().filter(|(t, _)| t == "content_block_stop").count();
        assert_eq!(stops, 1, "只应闭合一次，实际 {stops}");
    }

    /// A3 回归：`message_stop` 之后到达的 chunk 必须被吞掉，不得再产出任何事件。
    #[test]
    fn chunks_after_termination_are_swallowed() {
        let mut c = StreamConverter::new("m".into());
        let mut out = c.convert_chunk(&chunk(json!({"content": "hi"}), None));
        out.extend(c.convert_chunk(&chunk(json!({}), Some("stop"))));
        assert!(c.is_terminated());

        let after = c.convert_chunk(&chunk(json!({"content": "不该出现"}), None));
        assert!(after.is_empty(), "终止后不得再产出事件：{after:?}");
        let ty: Vec<String> = events_of(&out).into_iter().map(|(t, _)| t).collect();
        assert_eq!(ty.last().map(String::as_str), Some("message_stop"));
    }

    /// close_open_blocks（异常路径用）只闭合、不发终止事件。
    #[test]
    fn close_open_blocks_does_not_emit_terminator() {
        let mut c = StreamConverter::new("m".into());
        let _ = c.convert_chunk(&chunk(json!({"content": "x"}), None));
        let evs = events_of(&c.close_open_blocks());
        assert_eq!(evs, vec![("content_block_stop".to_string(), 0)]);
        assert!(!c.is_terminated(), "异常路径不应声称正常结束");
    }

    /// 一个 delta 都没来就中断：不产出孤立终止事件（否则客户端更困惑）。
    #[test]
    fn finish_before_any_chunk_emits_nothing() {
        let mut c = StreamConverter::new("m".into());
        assert!(c.finish().is_empty());
        assert!(c.is_terminated());
    }

    /// 上游只发 [DONE] 不带 finish_reason 时，finish() 要补终止（否则客户端挂住）
    #[test]
    fn done_without_finish_reason_still_gets_terminated_by_finish() {
        let mut c = StreamConverter::new("m".into());
        let _ = c.convert_chunk(&chunk(json!({"content": "partial"}), None));
        let tail = c.finish();
        let ty: Vec<String> = events_of(&tail).into_iter().map(|(t, _)| t).collect();
        assert_eq!(
            ty,
            vec!["content_block_stop", "message_delta", "message_stop"],
            "实际 {ty:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{anthropic_to_openai, openai_to_anthropic};
    use serde_json::{json, Value};

    fn anthropic_body(tool_choice: Value) -> Value {
        json!({
            "model": "claude-sonnet-4",
            "max_tokens": 128,
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{
                "name": "read_file",
                "description": "read a file",
                "input_schema": {"type": "object"}
            }],
            "tool_choice": tool_choice
        })
    }

    #[test]
    fn disable_parallel_tool_use_maps_to_parallel_tool_calls_false() {
        let out = anthropic_to_openai(
            &anthropic_body(json!({"type": "auto", "disable_parallel_tool_use": true})),
            "gpt-5",
            None,
        );
        assert_eq!(out.get("parallel_tool_calls"), Some(&json!(false)));
    }

    #[test]
    fn absent_flag_leaves_parallel_tool_calls_untouched() {
        // 未声明时不要主动写这个字段，让上游保持自己的默认行为
        let out = anthropic_to_openai(&anthropic_body(json!({"type": "any"})), "gpt-5", None);
        assert!(
            out.get("parallel_tool_calls").is_none(),
            "未声明 disable_parallel_tool_use 时不应注入 parallel_tool_calls"
        );
        // 顺带回归 tool_choice 映射（本次未改动，但同一段逻辑）
        assert_eq!(out.get("tool_choice"), Some(&json!("required")));
    }

    #[test]
    fn output_config_effort_maps_to_reasoning_effort() {
        // Anthropic 的 `output_config.effort` 在 OpenAI 侧只有 `reasoning_effort` 一个落点；
        // 不映射时 Claude Code `/effort max` 这类意图会静默丢失（抄自 cc-switch d6e05152）。
        let mut body = anthropic_body(json!({"type": "auto"}));
        body["output_config"] = json!({"effort": "max"});
        let out = anthropic_to_openai(&body, "gpt-5.6", None);
        assert_eq!(out.get("reasoning_effort"), Some(&json!("max")));
    }

    #[test]
    fn thinking_budget_maps_to_reasoning_effort_for_grok() {
        let mut body = anthropic_body(json!({"type": "auto"}));
        body["thinking"] = json!({"type": "enabled", "budget_tokens": 8000});
        let out = anthropic_to_openai(&body, "grok-4.7", None);
        assert_eq!(out.get("reasoning_effort"), Some(&json!("medium")));
    }

    #[test]
    fn effort_is_not_injected_for_unsupported_model() {
        let mut body = anthropic_body(json!({"type": "auto"}));
        body["output_config"] = json!({"effort": "high"});
        let out = anthropic_to_openai(&body, "gpt-4o", None);
        assert!(out.get("reasoning_effort").is_none(), "{out}");
    }

    #[test]
    fn no_effort_intent_leaves_reasoning_effort_to_the_optimizer() {
        // 入站没有表达 effort 时不要臆造，交给 optimizers::optimize_thinking_openai 按家族兜底
        let out = anthropic_to_openai(&anthropic_body(json!({"type": "auto"})), "gpt-5.4", None);
        assert!(out.get("reasoning_effort").is_none(), "{out}");
    }

    /// OpenAI 请求体（可带 tool_choice / parallel_tool_calls），用于反向转换测试。
    fn openai_body(tool_choice: Option<Value>, parallel: Option<bool>) -> Value {
        let mut body = json!({
            "model": "gpt-5",
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{
                "type": "function",
                "function": { "name": "read_file", "parameters": {"type": "object"} }
            }]
        });
        if let Some(tc) = tool_choice {
            body["tool_choice"] = tc;
        }
        if let Some(p) = parallel {
            body["parallel_tool_calls"] = json!(p);
        }
        body
    }

    /// 反向缺口：此前 openai→anthropic 完全不转换 tool_choice，「必须调用工具」会被降级成自由发挥。
    #[test]
    fn openai_tool_choice_maps_to_anthropic() {
        let required =
            openai_to_anthropic(&openai_body(Some(json!("required")), None), "claude-x", None);
        assert_eq!(required.get("tool_choice"), Some(&json!({"type": "any"})));

        let forced = openai_to_anthropic(
            &openai_body(
                Some(json!({"type": "function", "function": {"name": "read_file"}})),
                None,
            ),
            "claude-x",
            None,
        );
        assert_eq!(
            forced.get("tool_choice"),
            Some(&json!({"type": "tool", "name": "read_file"}))
        );

        let auto = openai_to_anthropic(&openai_body(Some(json!("auto")), None), "claude-x", None);
        assert_eq!(auto.get("tool_choice"), Some(&json!({"type": "auto"})));

        // 没给 tool_choice → 不写该字段（保持上游默认行为）
        let absent = openai_to_anthropic(&openai_body(None, None), "claude-x", None);
        assert!(absent.get("tool_choice").is_none());
    }

    #[test]
    fn openai_parallel_tool_calls_false_maps_to_disable_parallel() {
        // 带 tool_choice：约束挂在同一个对象上
        let out =
            openai_to_anthropic(&openai_body(Some(json!("auto")), Some(false)), "claude-x", None);
        assert_eq!(
            out.get("tool_choice"),
            Some(&json!({"type": "auto", "disable_parallel_tool_use": true}))
        );
        // 不带 tool_choice：补一个 auto 承载该约束
        let out2 = openai_to_anthropic(&openai_body(None, Some(false)), "claude-x", None);
        assert_eq!(
            out2.get("tool_choice"),
            Some(&json!({"type": "auto", "disable_parallel_tool_use": true}))
        );
        // true 或未声明 → 不写（保持上游默认）
        let out3 = openai_to_anthropic(&openai_body(None, Some(true)), "claude-x", None);
        assert!(out3.get("tool_choice").is_none());
    }

    #[test]
    fn tool_choice_is_not_written_without_tools() {
        // Anthropic 不接受没有 tools 的 tool_choice
        let mut body = openai_body(Some(json!("required")), None);
        body.as_object_mut().unwrap().remove("tools");
        let out = openai_to_anthropic(&body, "claude-x", None);
        assert!(out.get("tool_choice").is_none());
    }
}
