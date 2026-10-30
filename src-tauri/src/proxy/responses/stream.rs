//! stream implementation.
use super::{message_item_suffix, usage_to_responses};
use serde_json::{json, Value};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  流式：Chat SSE → Responses SSE
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 一个 SSE 事件（`event:` 与 `data:` 两行都要有，Codex 按 `event:` 分派）。
fn sse(event: &str, data: Value) -> String {
    format!("event: {event}\ndata: {data}\n\n")
}

/// 流式转换器：把 Chat 的 delta chunk 逐条翻译成 Responses 事件序列。
///
/// 事件顺序（抄 CodexPlusPlus `protocol_proxy.rs:2155-2411`）：
/// `response.created` → `response.in_progress` →（reasoning 块）→（message 块）→
/// `response.output_text.delta`* →（tool_calls）→ 各 `*.done` → `response.completed` → `[DONE]`。
///
/// 不生成 `sequence_number`：Codex 接受缺失（参考实现同样不生成）。
#[derive(Debug)]
pub struct ResponsesStreamConverter {
    response_id: String,
    model: String,
    created_at: i64,
    started: bool,
    next_output_index: usize,
    msg_index: Option<usize>,
    msg_id: String,
    msg_text: String,
    reasoning_index: Option<usize>,
    reasoning_id: String,
    reasoning_text: String,
    /// tool_call 序号 → (output_index, item_id, call_id, name, arguments)
    tools: Vec<(usize, String, String, String, String)>,
    /// 正文里已出现工具标记，之后的文本都进 `xml_buf` 而不发给客户端
    xml_mode: bool,
    /// 启用的工具调用方言（整流器开关决定）
    dialects: Vec<crate::proxy::xml_tools::Dialect>,
    /// 末尾可能是标记前缀、先扣住不发的一小段（见 [`Self::absorb_text`]）
    xml_hold: String,
    /// 进入标记之后累积的原始 XML，流末一次性解析
    xml_buf: String,
    usage: Option<Value>,
    finish_reason: Option<String>,
    failed: bool,
}

impl ResponsesStreamConverter {
    /// `dialects`：启用哪些工具调用方言（由 ProxyConfig 的整流器开关决定，见 `proxy::xml_tools`）
    pub fn new(model: &str) -> Self {
        Self::with_dialects(model, &crate::proxy::xml_tools::dialects_from(true))
    }

    pub fn with_dialects(model: &str, dialects: &[crate::proxy::xml_tools::Dialect]) -> Self {
        Self {
            response_id: "resp_stream".to_string(),
            model: model.to_string(),
            created_at: 0,
            started: false,
            next_output_index: 0,
            msg_index: None,
            msg_id: String::new(),
            msg_text: String::new(),
            reasoning_index: None,
            reasoning_id: String::new(),
            reasoning_text: String::new(),
            tools: Vec::new(),
            dialects: dialects.to_vec(),
            xml_mode: false,
            xml_hold: String::new(),
            xml_buf: String::new(),
            usage: None,
            finish_reason: None,
            failed: false,
        }
    }

    /// 吸收一段正文增量，返回**可以发给客户端**的那部分。
    ///
    /// 上游把工具调用写成 XML 方言时（见 `proxy::xml_tools`），整段 XML 只能等收全了才能解析，
    /// 但标记**之前**的正文必须照常逐字发出去 —— 否则用户会盯着空白等到整轮结束。
    /// 因此：
    /// - 没见到标记：正常转发，只把末尾「可能还是标记前缀」的一小段扣住（最多
    ///   `MAX_MARKER_LEN` 个字符），避免把半个 `<tool_call` 显示出来；
    /// - 见到标记：切到 xml_mode，之后的文本全部扣下，流末再解析成 function_call。
    fn absorb_text(&mut self, text: &str) -> String {
        if self.xml_mode {
            self.xml_buf.push_str(text);
            return String::new();
        }
        // 先归一化再去掉零宽字符：标记里混着 U+200B，不归一化永远匹配不上
        let (norm, _) = crate::proxy::xml_tools::normalize(text);
        let combined = format!("{}{}", self.xml_hold, norm);
        self.xml_hold.clear();
        let Some(pos) = crate::proxy::xml_tools::find_marker(&combined, &self.dialects) else {
            return self.hold_tail(&combined);
        };
        self.xml_mode = true;
        self.xml_buf.push_str(&combined[pos.0..]);
        combined[..pos.0].to_string()
    }

    /// 扣住末尾可能还是标记前缀的一段，其余原样发出
    fn hold_tail(&mut self, s: &str) -> String {
        let max = crate::proxy::xml_tools::MAX_MARKER_LEN;
        let from = s.len().saturating_sub(max);
        for k in (from..s.len()).rev() {
            if !s.is_char_boundary(k) {
                continue;
            }
            if crate::proxy::xml_tools::could_be_marker_prefix(&s[k..], &self.dialects) {
                self.xml_hold = s[k..].to_string();
                return s[..k].to_string();
            }
        }
        s.to_string()
    }

    /// 流末：把扣下的 XML 解析成标准 function_call。
    ///
    /// **解析不成也必须把原文还给客户端** —— 扣住是为了还原，不是为了吞掉内容：
    /// 误判成工具调用的普通文本（用户在问 `<invoke>` 语法之类）会原样出现，
    /// 而不是凭空消失。
    fn flush_xml_tools(&mut self) -> Vec<String> {
        let mut events = Vec::new();
        let mut push_text = |events: &mut Vec<String>, text: &str, this: &mut Self| {
            if text.is_empty() {
                return;
            }
            events.extend(this.ensure_message_block());
            this.msg_text.push_str(text);
            events.push(sse(
                "response.output_text.delta",
                json!({
                    "type": "response.output_text.delta",
                    "item_id": this.msg_id,
                    "output_index": this.msg_index.unwrap_or(0),
                    "content_index": 0,
                    "delta": text
                }),
            ));
        };

        if self.xml_mode && !self.xml_buf.is_empty() {
            let buf = std::mem::take(&mut self.xml_buf);
            self.xml_mode = false;
            match crate::proxy::xml_tools::split_tool_calls(&buf, &self.dialects) {
                Some(split) => {
                    // 工具调用之后模型可能又说了话，那段也要给客户端
                    push_text(&mut events, &split.prose_after, self);
                    for (i, call) in split.calls.iter().enumerate() {
                        let index = self.take_index();
                        let call_id = format!("call_xml_{i}");
                        let item_id = format!("fc_{call_id}");
                        events.push(sse(
                            "response.output_item.added",
                            json!({
                                "type": "response.output_item.added",
                                "output_index": index,
                                "item": {
                                    "id": item_id,
                                    "type": "function_call",
                                    "status": "in_progress",
                                    "call_id": call_id,
                                    "name": call.name,
                                    "arguments": ""
                                }
                            }),
                        ));
                        events.push(sse(
                            "response.function_call_arguments.delta",
                            json!({
                                "type": "response.function_call_arguments.delta",
                                "item_id": item_id,
                                "output_index": index,
                                "delta": call.arguments
                            }),
                        ));
                        self.tools.push((
                            index,
                            item_id,
                            call_id,
                            call.name.clone(),
                            call.arguments.clone(),
                        ));
                    }
                }
                // 没能还原（标记误判 / 上游格式又变了）：一个字都不丢，原样发回去
                None => push_text(&mut events, &buf, self),
            }
        }
        // 流结束时还扣着的那一小段：它没能长成标记，就是普通正文，补发出来
        if !self.xml_hold.is_empty() {
            let tail = std::mem::take(&mut self.xml_hold);
            push_text(&mut events, &tail, self);
        }
        events
    }

    fn base_response(&self, status: &str) -> Value {
        let mut resp = json!({
            "id": self.response_id,
            "object": "response",
            "created_at": self.created_at,
            "status": status,
            "model": self.model,
            "output": [],
            "usage": usage_to_responses(self.usage.as_ref())
        });
        if status == "incomplete" {
            resp["incomplete_details"] = json!({ "reason": "max_output_tokens" });
        }
        resp
    }

    fn take_index(&mut self) -> usize {
        let idx = self.next_output_index;
        self.next_output_index += 1;
        idx
    }

    /// 首个 chunk 到达时补发 `created` / `in_progress`，并取用上游的 id / model / created。
    fn ensure_started(&mut self, chunk: &Value) -> Vec<String> {
        if let Some(id) = chunk.get("id").and_then(|v| v.as_str()) {
            self.response_id = format!("resp_{id}");
            self.msg_id = format!("msg_{}", message_item_suffix(&self.response_id));
            self.reasoning_id = format!("rs_{}", self.response_id);
        }
        if let Some(model) = chunk.get("model").and_then(|v| v.as_str()) {
            self.model = model.to_string();
        }
        if let Some(created) = chunk.get("created").and_then(|v| v.as_i64()) {
            self.created_at = created;
        }
        if self.started {
            return Vec::new();
        }
        self.started = true;
        vec![
            sse(
                "response.created",
                json!({ "type": "response.created", "response": self.base_response("in_progress") }),
            ),
            sse(
                "response.in_progress",
                json!({ "type": "response.in_progress", "response": self.base_response("in_progress") }),
            ),
        ]
    }

    fn ensure_reasoning_block(&mut self) -> Vec<String> {
        if self.reasoning_index.is_some() {
            return Vec::new();
        }
        let index = self.take_index();
        self.reasoning_index = Some(index);
        vec![
            sse(
                "response.output_item.added",
                json!({
                    "type": "response.output_item.added",
                    "output_index": index,
                    "item": {
                        "id": self.reasoning_id,
                        "type": "reasoning",
                        "status": "in_progress",
                        "reasoning_content": "",
                        "summary": []
                    }
                }),
            ),
            sse(
                "response.reasoning_summary_part.added",
                json!({
                    "type": "response.reasoning_summary_part.added",
                    "item_id": self.reasoning_id,
                    "output_index": index,
                    "summary_index": 0,
                    "part": { "type": "summary_text", "text": "" }
                }),
            ),
        ]
    }

    fn ensure_message_block(&mut self) -> Vec<String> {
        if self.msg_index.is_some() {
            return Vec::new();
        }
        let index = self.take_index();
        self.msg_index = Some(index);
        vec![
            sse(
                "response.output_item.added",
                json!({
                    "type": "response.output_item.added",
                    "output_index": index,
                    "item": {
                        "id": self.msg_id,
                        "type": "message",
                        "status": "in_progress",
                        "role": "assistant",
                        "content": []
                    }
                }),
            ),
            sse(
                "response.content_part.added",
                json!({
                    "type": "response.content_part.added",
                    "item_id": self.msg_id,
                    "output_index": index,
                    "content_index": 0,
                    "part": { "type": "output_text", "text": "", "annotations": [] }
                }),
            ),
        ]
    }

    /// 诊断用（仅 `ANY_VERSION_PROXY_DEBUG=1` 时打印）：这一轮上游到底给了什么。
    ///
    /// 「模型只吐了一段计划就停」这类问题分不清是「上游没给工具调用」还是
    /// 「给了但被我们丢了」，没有这段日志就只能靠猜。
    pub fn upstream_debug(&self) -> String {
        let head = |s: &str| s.chars().take(400).collect::<String>();
        format!(
            "finish={:?} tools={} xml_mode={} 正文({}字)={:?} 扣留({}字)={:?}",
            self.finish_reason,
            self.tools.len(),
            self.xml_mode,
            self.msg_text.chars().count(),
            head(&self.msg_text),
            self.xml_buf.chars().count(),
            head(&self.xml_buf),
        )
    }

    /// 标记本次响应失败并产出 `response.failed` + `[DONE]`。
    /// 之后 `push_chunk` / `finish` 都不再产出任何事件（Codex 收不到「成功完成」的假象）。
    ///
    /// 两条调用路径：流内 `error` chunk（上游自己报错）、传输层断流（`body` 提前结束）。
    /// 后者以前直接 `break` 导致 `failed` 仍为 false，`finish()` 补发
    /// `response.completed` + `[DONE]` —— **上游断流被伪装成一次成功完成**，
    /// Codex 会把残缺内容当作完整答案收下。
    pub fn fail(&mut self, message: &str) -> Vec<String> {
        if self.failed {
            return Vec::new();
        }
        self.failed = true;
        vec![
            sse(
                "response.failed",
                json!({
                    "type": "response.failed",
                    "response": {
                        "id": self.response_id,
                        "object": "response",
                        "status": "failed",
                        "error": { "message": message, "type": "upstream_error" }
                    }
                }),
            ),
            "data: [DONE]\n\n".to_string(),
        ]
    }

    /// 处理一个 Chat SSE chunk（已解析的 JSON），返回要写回客户端的事件文本。
    pub fn push_chunk(&mut self, chunk: &Value) -> Vec<String> {
        if self.failed {
            return Vec::new();
        }
        // 流内错误：转成 response.failed 并停止后续事件
        if let Some(err) = chunk.get("error") {
            let message = err
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("upstream stream error")
                .to_string();
            return self.fail(&message);
        }

        let mut events = self.ensure_started(chunk);

        if let Some(usage) = chunk.get("usage").filter(|u| !u.is_null()) {
            self.usage = Some(usage.clone());
        }

        let Some(choice) = chunk.get("choices").and_then(|c| c.get(0)) else {
            return events;
        };
        let delta = choice.get("delta").cloned().unwrap_or(Value::Null);

        // reasoning 文本（DeepSeek / Moonshot 等用 reasoning_content）
        if let Some(rc) = delta
            .get("reasoning_content")
            .or_else(|| delta.get("reasoning"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            events.extend(self.ensure_reasoning_block());
            self.reasoning_text.push_str(rc);
            events.push(sse(
                "response.reasoning_summary_text.delta",
                json!({
                    "type": "response.reasoning_summary_text.delta",
                    "item_id": self.reasoning_id,
                    "output_index": self.reasoning_index.unwrap_or(0),
                    "summary_index": 0,
                    "delta": rc
                }),
            ));
        }

        if let Some(text) = delta.get("content").and_then(|v| v.as_str()) {
            if !text.is_empty() {
                // 走 absorb_text：标记之前的正文照常逐字发，标记（及其后的 XML）扣到流末解析
                let emit = self.absorb_text(text);
                if !emit.is_empty() {
                    events.extend(self.ensure_message_block());
                    self.msg_text.push_str(&emit);
                    events.push(sse(
                        "response.output_text.delta",
                        json!({
                            "type": "response.output_text.delta",
                            "item_id": self.msg_id,
                            "output_index": self.msg_index.unwrap_or(0),
                            "content_index": 0,
                            "delta": emit
                        }),
                    ));
                }
            }
        }

        if let Some(tool_calls) = delta.get("tool_calls").and_then(|v| v.as_array()) {
            for call in tool_calls {
                let seq = call.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let call_id = call
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let name = call
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let args = call
                    .get("function")
                    .and_then(|f| f.get("arguments"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let existing = self.tools.iter().position(|(i, _, _, _, _)| *i == seq);
                let index = match existing {
                    Some(pos) => {
                        let entry = &mut self.tools[pos];
                        if !call_id.is_empty() {
                            entry.2 = call_id.clone();
                        }
                        if !name.is_empty() {
                            entry.3 = name.clone();
                        }
                        entry.4.push_str(&args);
                        entry.0
                    }
                    None => {
                        let output_index = self.take_index();
                        let item_id = if call_id.is_empty() {
                            format!("fc_{output_index}")
                        } else {
                            format!("fc_{call_id}")
                        };
                        events.push(sse(
                            "response.output_item.added",
                            json!({
                                "type": "response.output_item.added",
                                "output_index": output_index,
                                "item": {
                                    "id": item_id,
                                    "type": "function_call",
                                    "status": "in_progress",
                                    "call_id": call_id,
                                    "name": name,
                                    "arguments": ""
                                }
                            }),
                        ));
                        self.tools.push((seq, item_id, call_id, name, args.clone()));
                        output_index
                    }
                };
                if !args.is_empty() {
                    let item_id = self.tools[existing.unwrap_or(self.tools.len() - 1)]
                        .1
                        .clone();
                    events.push(sse(
                        "response.function_call_arguments.delta",
                        json!({
                            "type": "response.function_call_arguments.delta",
                            "item_id": item_id,
                            "output_index": index,
                            "delta": args
                        }),
                    ));
                }
            }
        }

        if let Some(reason) = choice.get("finish_reason").and_then(|v| v.as_str()) {
            self.finish_reason = Some(reason.to_string());
        }
        events
    }

    /// 流结束：补发各块的 `*.done`、`response.completed` 与 `[DONE]`。
    pub fn finish(&mut self) -> Vec<String> {
        if self.failed {
            return Vec::new();
        }
        let mut events = Vec::new();
        if !self.started {
            // 一个 delta 都没来（上游直接空响应）：仍要给 Codex 一个完整响应
            let placeholder = json!({});
            events.extend(self.ensure_started(&placeholder));
        }

        // XML 方言的工具调用在这里还原：正文已发完，补上 function_call 与扣住的尾巴
        events.extend(self.flush_xml_tools());

        if let Some(index) = self.reasoning_index {
            events.push(sse(
                "response.reasoning_summary_text.done",
                json!({
                    "type": "response.reasoning_summary_text.done",
                    "item_id": self.reasoning_id,
                    "output_index": index,
                    "summary_index": 0,
                    "text": self.reasoning_text
                }),
            ));
            events.push(sse(
                "response.reasoning_summary_part.done",
                json!({
                    "type": "response.reasoning_summary_part.done",
                    "item_id": self.reasoning_id,
                    "output_index": index,
                    "summary_index": 0,
                    "part": { "type": "summary_text", "text": self.reasoning_text }
                }),
            ));
            events.push(sse(
                "response.output_item.done",
                json!({
                    "type": "response.output_item.done",
                    "output_index": index,
                    "item": {
                        "id": self.reasoning_id,
                        "type": "reasoning",
                        "status": "completed",
                        "reasoning_content": self.reasoning_text,
                        "summary": [{ "type": "summary_text", "text": self.reasoning_text }]
                    }
                }),
            ));
        }

        let mut items: Vec<(usize, Value)> = Vec::new();

        if let Some(index) = self.msg_index {
            events.push(sse(
                "response.output_text.done",
                json!({
                    "type": "response.output_text.done",
                    "item_id": self.msg_id,
                    "output_index": index,
                    "content_index": 0,
                    "text": self.msg_text
                }),
            ));
            events.push(sse(
                "response.content_part.done",
                json!({
                    "type": "response.content_part.done",
                    "item_id": self.msg_id,
                    "output_index": index,
                    "content_index": 0,
                    "part": { "type": "output_text", "text": self.msg_text, "annotations": [] }
                }),
            ));
            let item = json!({
                "id": self.msg_id,
                "type": "message",
                "status": "completed",
                "role": "assistant",
                "content": [{ "type": "output_text", "text": self.msg_text, "annotations": [] }]
            });
            events.push(sse(
                "response.output_item.done",
                json!({ "type": "response.output_item.done", "output_index": index, "item": item }),
            ));
            items.push((index, item));
        }

        if let Some(index) = self.reasoning_index {
            items.push((
                index,
                json!({
                    "id": self.reasoning_id,
                    "type": "reasoning",
                    "status": "completed",
                    "reasoning_content": self.reasoning_text,
                    "summary": [{ "type": "summary_text", "text": self.reasoning_text }]
                }),
            ));
        }

        for (index, item_id, call_id, name, args) in self.tools.clone() {
            events.push(sse(
                "response.function_call_arguments.done",
                json!({
                    "type": "response.function_call_arguments.done",
                    "item_id": item_id,
                    "output_index": index,
                    "arguments": args
                }),
            ));
            let item = json!({
                "id": item_id,
                "type": "function_call",
                "status": "completed",
                "call_id": call_id,
                "name": name,
                "arguments": if args.is_empty() { "{}".to_string() } else { args }
            });
            events.push(sse(
                "response.output_item.done",
                json!({ "type": "response.output_item.done", "output_index": index, "item": item }),
            ));
            items.push((index, item));
        }

        items.sort_by_key(|(index, _)| *index);
        let status = if self.finish_reason.as_deref() == Some("length") {
            "incomplete"
        } else {
            "completed"
        };
        let mut completed = self.base_response(status);
        completed["output"] = json!(items.into_iter().map(|(_, item)| item).collect::<Vec<_>>());
        events.push(sse(
            "response.completed",
            json!({ "type": "response.completed", "response": completed }),
        ));
        events.push("data: [DONE]\n\n".to_string());
        events
    }
}

#[cfg(test)]
mod tests;
