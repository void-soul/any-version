//! stream implementation.
use serde_json::{json, Value};

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
                events.push(format!(
                    "event: content_block_delta\ndata: {}\n\n",
                    delta_ev
                ));
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
                events.push(format!(
                    "event: content_block_delta\ndata: {}\n\n",
                    delta_ev
                ));
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
                        events.push(format!(
                            "event: content_block_delta\ndata: {}\n\n",
                            delta_ev
                        ));
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
                        self.input_tokens = usage
                            .get("input_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                    }
                }
                None
            }
            "content_block_delta" => {
                if let Some(delta) = data.get("delta") {
                    let delta_type = delta
                        .get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("text_delta");
                    let mut choice_delta = json!({});

                    if delta_type == "text_delta" {
                        if let Some(text) = delta.get("text").and_then(|v| v.as_str()) {
                            choice_delta["content"] = json!(text);
                        }
                    } else if delta_type == "input_json_delta" {
                        if let Some(partial) = delta.get("partial_json").and_then(|v| v.as_str()) {
                            let idx =
                                data.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
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
                    Some(format!(
                        "data: {}\n\n",
                        serde_json::to_string(&chunk).unwrap()
                    ))
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
                        return Some(format!(
                            "data: {}\n\n",
                            serde_json::to_string(&chunk).unwrap()
                        ));
                    }
                }
                None
            }
            "message_delta" => {
                if let Some(usage) = data.get("usage") {
                    self.output_tokens = usage
                        .get("output_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                }
                let stop_reason = data
                    .get("delta")
                    .and_then(|d| d.get("stop_reason").and_then(|v| v.as_str()))
                    .unwrap_or("end_turn");
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
                Some(format!(
                    "data: {}\n\n",
                    serde_json::to_string(&chunk).unwrap()
                ))
            }
            "message_stop" => Some("data: [DONE]\n\n".to_string()),
            _ => None,
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
