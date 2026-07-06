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

/// 内置「官方模型名/别名 → role」解析表。
///
/// 覆盖 claude-sonnet-4-*、claude-opus-4-*、claude-haiku-4-*、claude-fable-5-*
/// 各版本，以及 Claude Code 内部别名（best/opusplan 等），统一归并为
/// `sonnet` / `opus` / `haiku` / `fable` 四个标准 role，再交由 `role_map` 映射。
fn resolve_role(model: &str) -> Option<&'static str> {
    let lower = model.to_lowercase();

    // Claude Code 内部别名（不含 sonnet/opus/haiku/fable 关键词）
    match lower.as_str() {
        "best" => return Some("opus"),     // Claude Code 的 "best" 解析为最强模型
        "opusplan" => return Some("opus"), // "plan mode" 专用 opus 别名
        _ => {}
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
        "model": resolved_model,
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
            .filter(|t| t.get("name").and_then(|v| v.as_str()) != Some("BatchTool"))
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.get("name").unwrap_or(&json!("")),
                        "description": t.get("description").unwrap_or(&json!("")),
                        "parameters": t.get("input_schema").unwrap_or(&json!({}))
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

    // content → text block
    if let Some(text) = message.and_then(|m| m.get("content").and_then(|v| v.as_str())) {
        if !text.is_empty() {
            content.push(json!({"type": "text", "text": text}));
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
    content_index: usize,
    current_tool_calls: Vec<ToolCallState>,
    /// 当前是否有打开的 thinking block
    thinking_open: bool,
    /// 当前是否有打开的 text block
    text_open: bool,
}

struct ToolCallState {
    id: String,
    name: String,
    arguments: String,
}

impl StreamConverter {
    pub fn new(model: String) -> Self {
        Self {
            message_id: format!("msg_stream_{}", chrono::Utc::now().timestamp_millis()),
            model,
            input_tokens: 0,
            output_tokens: 0,
            started: false,
            content_index: 0,
            current_tool_calls: Vec::new(),
            thinking_open: false,
            text_open: false,
        }
    }

    /// 返回累积的 token 用量
    pub fn usage(&self) -> (u64, u64) {
        (self.input_tokens, self.output_tokens)
    }
    /// 返回请求的模型名
    pub fn model_name(&self) -> &str {
        &self.model
    }

    /// 将一个 OpenAI SSE chunk 转换为零或多个 Anthropic SSE 事件
    pub fn convert_chunk(&mut self, chunk: &Value) -> Vec<String> {
        let mut events = Vec::new();

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
                if !self.thinking_open {
                    // 关闭已打开的 text block（如果有的话）
                    if self.text_open {
                        events.push(format!("event: content_block_stop\ndata: {}\n\n", json!({"type": "content_block_stop", "index": self.content_index})));
                        self.content_index += 1;
                        self.text_open = false;
                    }
                    self.thinking_open = true;
                    let ev = json!({
                        "type": "content_block_start",
                        "index": self.content_index,
                        "content_block": {"type": "thinking", "thinking": ""}
                    });
                    events.push(format!("event: content_block_start\ndata: {}\n\n", ev));
                }
                let delta_ev = json!({
                    "type": "content_block_delta",
                    "index": self.content_index,
                    "delta": {"type": "thinking_delta", "thinking": reasoning}
                });
                events.push(format!("event: content_block_delta\ndata: {}\n\n", delta_ev));
            }
        }

        // content → text delta
        if let Some(content) = delta.get("content").and_then(|v| v.as_str()) {
            if !content.is_empty() {
                if !self.text_open {
                    // 关闭已打开的 thinking block（如果有的话）
                    if self.thinking_open {
                        events.push(format!("event: content_block_stop\ndata: {}\n\n", json!({"type": "content_block_stop", "index": self.content_index})));
                        self.content_index += 1;
                        self.thinking_open = false;
                    }
                    self.text_open = true;
                    let ev = json!({
                        "type": "content_block_start",
                        "index": self.content_index,
                        "content_block": {"type": "text", "text": ""}
                    });
                    events.push(format!("event: content_block_start\ndata: {}\n\n", ev));
                }
                let delta_ev = json!({
                    "type": "content_block_delta",
                    "index": self.content_index,
                    "delta": {"type": "text_delta", "text": content}
                });
                events.push(format!("event: content_block_delta\ndata: {}\n\n", delta_ev));
            }
        }

        // tool_calls delta
        if let Some(tool_calls) = delta.get("tool_calls").and_then(|v| v.as_array()) {
            // 关闭已打开的 text/thinking block
            if self.text_open || self.thinking_open {
                events.push(format!("event: content_block_stop\ndata: {}\n\n", json!({"type": "content_block_stop", "index": self.content_index})));
                self.content_index += 1;
                self.text_open = false;
                self.thinking_open = false;
            }

            for tc in tool_calls {
                let tc_index = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

                // 确保有足够的 tool call state
                while self.current_tool_calls.len() <= tc_index {
                    self.current_tool_calls.push(ToolCallState {
                        id: String::new(),
                        name: String::new(),
                        arguments: String::new(),
                    });
                }

                let state = &mut self.current_tool_calls[tc_index];

                if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                    state.id = id.to_string();
                    let name = tc
                        .get("function")
                        .and_then(|f| f.get("name").and_then(|v| v.as_str()))
                        .unwrap_or("");
                    state.name = name.to_string();

                    let ev = json!({
                        "type": "content_block_start",
                        "index": self.content_index,
                        "content_block": {
                            "type": "tool_use",
                            "id": state.id,
                            "name": state.name,
                            "input": {}
                        }
                    });
                    events.push(format!("event: content_block_start\ndata: {}\n\n", ev));
                    self.content_index += 1;
                }

                if let Some(args_delta) = tc
                    .get("function")
                    .and_then(|f| f.get("arguments").and_then(|v| v.as_str()))
                {
                    state.arguments.push_str(args_delta);
                    let delta_ev = json!({
                        "type": "content_block_delta",
                        "index": self.content_index,
                        "delta": {
                            "type": "input_json_delta",
                            "partial_json": args_delta
                        }
                    });
                    events.push(format!("event: content_block_delta\ndata: {}\n\n", delta_ev));
                }
            }
        }

        // finish_reason → message_delta + message_stop
        if let Some(finish_reason) = choice.get("finish_reason").and_then(|v| v.as_str()) {
            // 关闭所有未关闭的 text/thinking blocks
            if self.text_open || self.thinking_open {
                events.push(format!("event: content_block_stop\ndata: {}\n\n", json!({"type": "content_block_stop", "index": self.content_index})));
                self.content_index += 1;
                self.text_open = false;
                self.thinking_open = false;
            }

            // 关闭所有未关闭的 tool call blocks
            for _ in 0..self.current_tool_calls.len() {
                let stop_ev = json!({
                    "type": "content_block_stop",
                    "index": self.content_index
                });
                events.push(format!("event: content_block_stop\ndata: {}\n\n", stop_ev));
                self.content_index += 1;
            }

            let stop_reason = match finish_reason {
                "stop" => "end_turn",
                "length" => "max_tokens",
                "tool_calls" | "function_call" => "tool_use",
                _ => "end_turn",
            };

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
        }

        events
    }
}
