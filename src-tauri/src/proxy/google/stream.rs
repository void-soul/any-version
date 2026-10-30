//! stream implementation.
use serde_json::{json, Value};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  流式转换：上游 OpenAI / Anthropic 的 SSE chunk → Google SSE
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 把上游（openai 或 anthropic）的流式 chunk 转换为 Google SSE `data:` 行。
/// `source` 为上游协议（"openai" | "anthropic"）。
pub struct GoogleStreamConverter {
    source: String,
    model: String,
    input_tokens: u64,
    output_tokens: u64,
    /// 已发出终止标记；置位后不再产出任何事件
    terminated: bool,
}

impl GoogleStreamConverter {
    pub fn new(source: &str, model: &str) -> Self {
        Self {
            source: source.to_string(),
            model: model.to_string(),
            input_tokens: 0,
            output_tokens: 0,
            terminated: false,
        }
    }

    pub fn usage(&self) -> (u64, u64) {
        (self.input_tokens, self.output_tokens)
    }

    /// 是否已发出终止事件
    pub fn is_terminated(&self) -> bool {
        self.terminated
    }

    /// 异常路径用：本转换器无 content block 概念，无内容可闭合
    pub fn close_open_blocks(&mut self) -> Vec<String> {
        Vec::new()
    }

    /// 收尾：Google SSE 的终止标记是 `[DONE]`。幂等。
    pub fn finish(&mut self) -> Vec<String> {
        if self.terminated {
            return Vec::new();
        }
        self.terminated = true;
        vec![format!("data: [DONE]\n\n")]
    }

    /// 兼容统一管线：把上游（openai/anthropic）chunk 转换为 Google SSE 行向量。
    pub fn convert_chunk(&mut self, chunk: &Value) -> Vec<String> {
        if self.terminated {
            return Vec::new();
        }
        match self.convert_source_chunk(chunk) {
            Some(s) => {
                if s.contains("[DONE]") {
                    self.terminated = true;
                }
                vec![s]
            }
            None => Vec::new(),
        }
    }

    /// 输入一个已解析的上游 chunk JSON，返回 0~1 个 Google SSE `data:` 行（含 `\n\n`）。
    pub fn convert_source_chunk(&mut self, chunk: &Value) -> Option<String> {
        if self.source == "openai" {
            self.convert_openai_chunk(chunk)
        } else {
            self.convert_anthropic_chunk(chunk)
        }
    }

    fn emit(&self, parts: Vec<Value>, finish: Option<&str>) -> String {
        let mut cand = json!({"content": {"role": "model", "parts": parts}});
        if let Some(f) = finish {
            cand["finishReason"] = json!(f);
        }
        let mut obj = json!({"candidates": [cand]});
        obj["model"] = json!(self.model);
        if self.input_tokens > 0 || self.output_tokens > 0 {
            obj["usageMetadata"] = json!({
                "promptTokenCount": self.input_tokens,
                "candidatesTokenCount": self.output_tokens,
                "totalTokenCount": self.input_tokens + self.output_tokens
            });
        }
        format!("data: {}\n\n", serde_json::to_string(&obj).unwrap())
    }

    fn convert_openai_chunk(&mut self, chunk: &Value) -> Option<String> {
        if let Some(u) = chunk.get("usage") {
            self.input_tokens = u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
            self.output_tokens = u
                .get("completion_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
        }
        let choice = chunk
            .get("choices")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())?;
        let delta = choice.get("delta")?;
        let mut parts = Vec::new();
        if let Some(t) = delta.get("content").and_then(|v| v.as_str()) {
            if !t.is_empty() {
                parts.push(json!({"text": t}));
            }
        }
        if let Some(tcs) = delta.get("tool_calls").and_then(|v| v.as_array()) {
            for tc in tcs {
                let name = tc
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let args = tc
                    .get("function")
                    .and_then(|f| f.get("arguments"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                parts.push(json!({"functionCall": {"name": name, "args": serde_json::from_str::<Value>(args).unwrap_or(json!({}))}}));
            }
        }
        let finish = choice
            .get("finish_reason")
            .and_then(|v| v.as_str())
            .map(|fr| match fr {
                "stop" => "STOP",
                "length" => "MAX_TOKENS",
                _ => "STOP",
            });
        if parts.is_empty() && finish.is_none() {
            return None;
        }
        Some(self.emit(parts, finish))
    }

    fn convert_anthropic_chunk(&mut self, chunk: &Value) -> Option<String> {
        let etype = chunk.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match etype {
            "message_start" => {
                if let Some(u) = chunk.get("message").and_then(|m| m.get("usage")) {
                    self.input_tokens = u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                }
                None
            }
            "content_block_delta" => {
                let delta = chunk.get("delta")?;
                let dt = delta.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if dt == "text_delta" {
                    if let Some(t) = delta.get("text").and_then(|v| v.as_str()) {
                        return Some(self.emit(vec![json!({"text": t})], None));
                    }
                } else if dt == "input_json_delta" {
                    let args = delta
                        .get("partial_json")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    return Some(self.emit(vec![json!({"functionCall": {"name": "", "args": serde_json::from_str::<Value>(args).unwrap_or(json!({}))}})], None));
                }
                None
            }
            "content_block_start" => {
                if let Some(b) = chunk.get("content_block") {
                    if b.get("type").and_then(|v| v.as_str()).unwrap_or("") == "tool_use" {
                        let name = b.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        return Some(self.emit(
                            vec![json!({"functionCall": {"name": name, "args": json!({})}})],
                            None,
                        ));
                    }
                }
                None
            }
            "message_delta" => {
                if let Some(u) = chunk.get("usage") {
                    self.output_tokens =
                        u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                }
                None
            }
            "message_stop" => Some(self.emit(vec![], Some("STOP"))),
            _ => None,
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  流式转换：Google SSE → Anthropic SSE
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 把上游 Google 的流式 chunk（完整 GenerateContentResponse）转换为 Anthropic SSE 事件。
pub struct GoogleToAnthropicStreamConverter {
    model: String,
    message_id: String,
    started: bool,
    /// 已发出 `message_stop`；置位后不再产出任何事件
    terminated: bool,
    content_index: usize,
    tool_open: bool,
    text_open: bool,
    thought_open: bool,
    input_tokens: u64,
    output_tokens: u64,
}

impl GoogleToAnthropicStreamConverter {
    pub fn new(model: &str) -> Self {
        Self {
            model: model.to_string(),
            message_id: format!("msg_google_{}", chrono::Utc::now().timestamp_millis()),
            started: false,
            terminated: false,
            content_index: 0,
            tool_open: false,
            text_open: false,
            thought_open: false,
            input_tokens: 0,
            output_tokens: 0,
        }
    }

    pub fn usage(&self) -> (u64, u64) {
        (self.input_tokens, self.output_tokens)
    }

    /// 是否已发出终止事件
    pub fn is_terminated(&self) -> bool {
        self.terminated
    }

    /// 异常路径用：只闭合已打开的块，不发终止事件
    pub fn close_open_blocks(&mut self) -> Vec<String> {
        let mut events = Vec::new();
        self.close_open_blocks_into(&mut events);
        events
    }

    /// 收尾：闭合所有块 + `message_delta` + `message_stop`。幂等。
    pub fn finish(&mut self) -> Vec<String> {
        let mut events = Vec::new();
        if self.terminated {
            return events;
        }
        if !self.started {
            self.terminated = true;
            return events;
        }
        self.close_open_blocks_into(&mut events);
        events.push(format!(
            "event: message_delta\ndata: {}\n\n",
            json!({
                "type": "message_delta",
                "delta": {"stop_reason": "end_turn", "stop_sequence": null},
                "usage": {"output_tokens": self.output_tokens}
            })
        ));
        events.push("event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n".to_string());
        self.terminated = true;
        events
    }

    /// 输入一个已解析的 Google chunk JSON，返回 0~N 个 Anthropic SSE 事件字符串。
    pub fn convert_chunk(&mut self, chunk: &Value) -> Vec<String> {
        let mut events = Vec::new();

        // 流已终止：吞掉后续 chunk，绝不再产出事件
        if self.terminated {
            return events;
        }

        // usageMetadata（出现在最后一个 chunk）
        if let Some(um) = chunk.get("usageMetadata") {
            self.input_tokens = um
                .get("promptTokenCount")
                .and_then(|v| v.as_u64())
                .unwrap_or(self.input_tokens);
            let cand = um
                .get("candidatesTokenCount")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let thought = um
                .get("thoughtsTokenCount")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            self.output_tokens = cand + thought;
        }

        if !self.started {
            self.started = true;
            let start = json!({
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
            events.push(format!("event: message_start\ndata: {}\n\n", start));
        }

        let mut finish: Option<&str> = None;
        if let Some(cands) = chunk.get("candidates").and_then(|v| v.as_array()) {
            if let Some(cand) = cands.first() {
                if let Some(fr) = cand.get("finishReason").and_then(|v| v.as_str()) {
                    finish = Some(fr);
                }
                if let Some(parts) = cand
                    .get("content")
                    .and_then(|v| v.get("parts"))
                    .and_then(|v| v.as_array())
                {
                    for p in parts {
                        let is_thought =
                            p.get("thought").and_then(|v| v.as_bool()).unwrap_or(false);
                        if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                            if is_thought {
                                // thinking delta
                                if !self.thought_open {
                                    self.close_open_blocks_into(&mut events);
                                    self.thought_open = true;
                                    events.push(format!("event: content_block_start\ndata: {}\n\n", json!({
                                        "type": "content_block_start", "index": self.content_index,
                                        "content_block": {"type": "thinking", "thinking": ""}
                                    })));
                                }
                                events.push(format!(
                                    "event: content_block_delta\ndata: {}\n\n",
                                    json!({
                                        "type": "content_block_delta", "index": self.content_index,
                                        "delta": {"type": "thinking_delta", "thinking": t}
                                    })
                                ));
                            } else {
                                if !self.text_open {
                                    self.close_open_blocks_into(&mut events);
                                    self.text_open = true;
                                    events.push(format!("event: content_block_start\ndata: {}\n\n", json!({
                                        "type": "content_block_start", "index": self.content_index,
                                        "content_block": {"type": "text", "text": ""}
                                    })));
                                }
                                events.push(format!(
                                    "event: content_block_delta\ndata: {}\n\n",
                                    json!({
                                        "type": "content_block_delta", "index": self.content_index,
                                        "delta": {"type": "text_delta", "text": t}
                                    })
                                ));
                            }
                        } else if p.get("functionCall").is_some() {
                            let fc = &p["functionCall"];
                            let name = fc.get("name").cloned().unwrap_or(json!(""));
                            let args = fc.get("args").cloned().unwrap_or(json!({}));
                            self.close_open_blocks_into(&mut events);
                            // 每个 functionCall 单独成块
                            events.push(format!("event: content_block_start\ndata: {}\n\n", json!({
                                "type": "content_block_start", "index": self.content_index,
                                "content_block": {"type": "tool_use", "id": name, "name": name, "input": {}}
                            })));
                            events.push(format!("event: content_block_delta\ndata: {}\n\n", json!({
                                "type": "content_block_delta", "index": self.content_index,
                                "delta": {"type": "input_json_delta", "partial_json": serde_json::to_string(&args).unwrap_or_else(|_| "{}".to_string())}
                            })));
                            events.push(format!(
                                "event: content_block_stop\ndata: {}\n\n",
                                json!({
                                    "type": "content_block_stop", "index": self.content_index
                                })
                            ));
                            self.content_index += 1;
                            self.tool_open = false;
                        }
                    }
                }
            }
        }

        if let Some(fr) = finish {
            self.close_open_blocks_into(&mut events);
            let stop_reason = match fr {
                "STOP" => "end_turn",
                "MAX_TOKENS" => "max_tokens",
                _ => "end_turn",
            };
            events.push(format!(
                "event: message_delta\ndata: {}\n\n",
                json!({
                    "type": "message_delta",
                    "delta": {"stop_reason": stop_reason, "stop_sequence": null},
                    "usage": {"output_tokens": self.output_tokens}
                })
            ));
            events.push("event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n".to_string());
            self.terminated = true;
        }

        events
    }

    fn close_open_blocks_into(&mut self, events: &mut Vec<String>) {
        if self.text_open || self.thought_open || self.tool_open {
            events.push(format!(
                "event: content_block_stop\ndata: {}\n\n",
                json!({
                    "type": "content_block_stop", "index": self.content_index
                })
            ));
            self.content_index += 1;
            self.text_open = false;
            self.thought_open = false;
            self.tool_open = false;
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  流式转换：Google SSE → OpenAI SSE
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 把上游 Google 的流式 chunk 转换为 OpenAI SSE chunk。
pub struct GoogleToOpenaiStreamConverter {
    model: String,
    id: String,
    tool_index: usize,
    input_tokens: u64,
    output_tokens: u64,
    /// 已发出终止标记；置位后不再产出任何事件
    terminated: bool,
}

impl GoogleToOpenaiStreamConverter {
    pub fn new(model: &str) -> Self {
        Self {
            model: model.to_string(),
            id: format!("chatcmpl_google_{}", chrono::Utc::now().timestamp_millis()),
            tool_index: 0,
            input_tokens: 0,
            output_tokens: 0,
            terminated: false,
        }
    }

    pub fn usage(&self) -> (u64, u64) {
        (self.input_tokens, self.output_tokens)
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

    pub fn convert_chunk(&mut self, chunk: &Value) -> Vec<String> {
        if self.terminated {
            return Vec::new();
        }
        if let Some(um) = chunk.get("usageMetadata") {
            self.input_tokens = um
                .get("promptTokenCount")
                .and_then(|v| v.as_u64())
                .unwrap_or(self.input_tokens);
            let cand = um
                .get("candidatesTokenCount")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let thought = um
                .get("thoughtsTokenCount")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            self.output_tokens = cand + thought;
        }

        let mut delta = json!({});
        let mut finish: Option<&str> = None;
        if let Some(cands) = chunk.get("candidates").and_then(|v| v.as_array()) {
            if let Some(cand) = cands.first() {
                if let Some(fr) = cand.get("finishReason").and_then(|v| v.as_str()) {
                    finish = Some(fr);
                }
                if let Some(parts) = cand
                    .get("content")
                    .and_then(|v| v.get("parts"))
                    .and_then(|v| v.as_array())
                {
                    for p in parts {
                        let is_thought =
                            p.get("thought").and_then(|v| v.as_bool()).unwrap_or(false);
                        if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                            if !is_thought {
                                delta["content"] = json!(t);
                            }
                        } else if p.get("functionCall").is_some() {
                            let fc = &p["functionCall"];
                            let name = fc.get("name").cloned().unwrap_or(json!(""));
                            let args = serde_json::to_string(fc.get("args").unwrap_or(&json!({})))
                                .unwrap_or_else(|_| "{}".to_string());
                            let idx = self.tool_index;
                            self.tool_index += 1;
                            delta["tool_calls"] = json!([{
                                "index": idx,
                                "id": name,
                                "type": "function",
                                "function": {"name": name, "arguments": args}
                            }]);
                        }
                    }
                }
            }
        }

        let finish_reason = finish.map(|fr| match fr {
            "STOP" => "stop",
            "MAX_TOKENS" => "length",
            _ => "stop",
        });

        let chunk_out = json!({
            "id": self.id,
            "object": "chat.completion.chunk",
            "created": chrono::Utc::now().timestamp(),
            "model": self.model,
            "choices": [{
                "index": 0,
                "delta": delta,
                "logprobs": null,
                "finish_reason": finish_reason
            }]
        });
        vec![format!(
            "data: {}\n\n",
            serde_json::to_string(&chunk_out).unwrap()
        )]
    }
}
