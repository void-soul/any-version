//! OpenAI Responses API ↔ Chat Completions 互转。
//!
//! 为什么需要：新版 ChatGPT Desktop / Codex（26.901+，见 openai/codex discussion #7782）
//! **不再支持 `wire_api = "chat"`**，只会往 `{base_url}/responses` 发 Responses 请求。
//! 我们此前只注册了 `/v1/messages` 与 `/v1/chat/completions`，Codex 因此报
//! `404 not found: http://127.0.0.1:8466/responses`。
//!
//! 设计：**Responses 只作为「入口方言」与「出口方言」**。请求进来先转成 Chat Completions
//! 形态，交给现有 `process_request` 管线（模型伪装、协议转换、整流器、SSE 转换全部复用），
//! 出来再转回 Responses。这样不为一个端点复制一整套转发逻辑。
//!
//! 转换规则抄 CodexPlusPlus `protocol_proxy.rs`（Responses↔Chat 双向转换与事件序列）。

use serde_json::{json, Map, Value};

/// Responses 请求 → Chat Completions 请求。
///
/// **白名单重建**（抄 CodexPlusPlus `protocol_proxy.rs:227-329`）：只挑认识的键搬过去，
/// 没列进来的（`store`、`previous_response_id`、`include`、`text`、`truncation` …）
/// 天然丢弃 —— Codex 的会话字段对第三方上游没有意义，透传只会让上游报未知字段。
pub fn responses_to_chat(body: &Value) -> Value {
    let mut out = Map::new();

    // 同名直拷的标量
    for key in [
        "model",
        "temperature",
        "top_p",
        "seed",
        "stop",
        "user",
        "metadata",
        "frequency_penalty",
        "presence_penalty",
        "logit_bias",
        "logprobs",
        "top_logprobs",
        "n",
        "response_format",
        "service_tier",
    ] {
        if let Some(v) = body.get(key) {
            if !v.is_null() {
                out.insert(key.to_string(), v.clone());
            }
        }
    }

    let stream = body.get("stream").and_then(|s| s.as_bool()).unwrap_or(false);
    if stream {
        out.insert("stream".to_string(), json!(true));
        // 不请求 usage 的话，流末拿不到 token 统计（Codex 的用量显示会空）
        out.insert("stream_options".to_string(), json!({ "include_usage": true }));
    }

    // max_output_tokens → max_tokens / max_completion_tokens
    if let Some(limit) = body.get("max_output_tokens").and_then(|v| v.as_u64()) {
        let model = body.get("model").and_then(|m| m.as_str()).unwrap_or("");
        let key = if is_o_series(model) {
            "max_completion_tokens"
        } else {
            "max_tokens"
        };
        out.insert(key.to_string(), json!(limit));
    }

    // messages：instructions 是 system，input 是正文
    let mut messages: Vec<Value> = Vec::new();
    if let Some(system) = instructions_text(body.get("instructions")) {
        messages.push(json!({ "role": "system", "content": system }));
    }
    push_input_messages(&mut messages, body.get("input"));
    out.insert("messages".to_string(), json!(messages));

    // tools：Responses 是扁平 schema，Chat 要包一层 `function`
    let tools = convert_tools(body.get("tools"));
    if !tools.is_empty() {
        out.insert("tools".to_string(), json!(tools));
        // 没有 tools 时发 tool_choice / parallel_tool_calls，严格上游会判非法
        if let Some(v) = body.get("tool_choice") {
            out.insert("tool_choice".to_string(), v.clone());
        }
        if let Some(v) = body.get("parallel_tool_calls") {
            out.insert("parallel_tool_calls".to_string(), v.clone());
        }
    }

    // reasoning.effort → reasoning_effort（进一步的各家方言由 optimizers 处理）
    if let Some(effort) = body
        .get("reasoning")
        .and_then(|r| r.get("effort"))
        .and_then(|e| e.as_str())
    {
        let normalized = effort.trim().to_lowercase();
        if normalized != "none" && normalized != "off" && normalized != "disabled" {
            out.insert("reasoning_effort".to_string(), json!(normalized));
        }
    }

    Value::Object(out)
}

/// `instructions` 可能是字符串，也可能是 `[{type:"text", text:"…"}]` 之类的块数组。
fn instructions_text(instructions: Option<&Value>) -> Option<String> {
    let value = instructions?;
    match value {
        Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
        Value::Array(items) => {
            let parts: Vec<String> = items
                .iter()
                .filter_map(|it| {
                    it.get("text")
                        .and_then(|t| t.as_str())
                        .map(str::to_string)
                        .or_else(|| it.as_str().map(str::to_string))
                })
                .filter(|s| !s.trim().is_empty())
                .collect();
            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n\n"))
            }
        }
        _ => None,
    }
}

/// `input` → chat messages。
///
/// 元素按 `type` 分派（抄 CodexPlusPlus `protocol_proxy.rs:2890+`）：
/// `message` / `function_call` / `function_call_output` / `reasoning`。
fn push_input_messages(messages: &mut Vec<Value>, input: Option<&Value>) {
    let Some(input) = input else { return };
    match input {
        Value::String(text) => {
            messages.push(json!({ "role": "user", "content": text }));
        }
        Value::Array(items) => {
            // reasoning 块要挂到「下一条 assistant 消息」上，先暂存
            let mut pending_reasoning: Option<String> = None;
            for item in items {
                let kind = item.get("type").and_then(|t| t.as_str()).unwrap_or("message");
                match kind {
                    "reasoning" => {
                        if let Some(text) = reasoning_text(item) {
                            pending_reasoning = Some(text);
                        }
                    }
                    "function_call" => {
                        let call_id = item
                            .get("call_id")
                            .or_else(|| item.get("id"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        let arguments = normalize_arguments(item.get("arguments"));
                        let tool_call = json!({
                            "id": call_id,
                            "type": "function",
                            "function": { "name": name, "arguments": arguments }
                        });
                        // 连续的 function_call 合成同一条 assistant 消息的 tool_calls
                        match messages.last_mut() {
                            Some(last)
                                if last.get("role").and_then(|r| r.as_str()) == Some("assistant")
                                    && last.get("tool_calls").is_some() =>
                            {
                                if let Some(arr) =
                                    last.get_mut("tool_calls").and_then(|v| v.as_array_mut())
                                {
                                    arr.push(tool_call);
                                }
                            }
                            _ => {
                                let mut msg = json!({ "role": "assistant", "content": Value::Null });
                                if let Some(rc) = pending_reasoning.take() {
                                    msg["reasoning_content"] = json!(rc);
                                }
                                msg["tool_calls"] = json!([tool_call]);
                                messages.push(msg);
                            }
                        }
                    }
                    "function_call_output" => {
                        let call_id = item
                            .get("call_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let output = output_text(item.get("output"));
                        if call_id.is_empty() {
                            // 孤儿输出（没有配对的 call_id）：降级成 user 文本，
                            // 丢掉它等于把工具结果从上下文里抹掉
                            messages.push(json!({ "role": "user", "content": output }));
                        } else {
                            messages.push(json!({
                                "role": "tool",
                                "tool_call_id": call_id,
                                "content": output
                            }));
                        }
                    }
                    _ => {
                        let role = item
                            .get("role")
                            .and_then(|r| r.as_str())
                            .unwrap_or("user")
                            .to_string();
                        let content = message_content(item.get("content"));
                        let mut msg = json!({ "role": role, "content": content });
                        if role == "assistant" {
                            if let Some(rc) = pending_reasoning.take() {
                                msg["reasoning_content"] = json!(rc);
                            }
                        }
                        messages.push(msg);
                    }
                }
            }
        }
        _ => {}
    }
}

/// reasoning item 的文本：优先 `summary[].text`，回落到 `content[].text` / `reasoning_content`。
fn reasoning_text(item: &Value) -> Option<String> {
    if let Some(s) = item.get("reasoning_content").and_then(|v| v.as_str()) {
        if !s.trim().is_empty() {
            return Some(s.to_string());
        }
    }
    for key in ["summary", "content"] {
        if let Some(parts) = item.get(key).and_then(|v| v.as_array()) {
            let joined: String = parts
                .iter()
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n");
            if !joined.trim().is_empty() {
                return Some(joined);
            }
        }
    }
    None
}

/// `arguments` 在 Responses 里可能是对象，在 Chat 里必须是 JSON **字符串**。
fn normalize_arguments(arguments: Option<&Value>) -> String {
    match arguments {
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => "{}".to_string(),
    }
}

/// `function_call_output.output` 可能是字符串或块数组。
fn output_text(output: Option<&Value>) -> String {
    match output {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|it| it.get("text").and_then(|t| t.as_str()).map(str::to_string))
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

/// message 的 `content` 块数组 → chat content。
///
/// 只有文本块时**塌缩成字符串**（上游普遍更认这种形态）；含图片时保留块数组。
fn message_content(content: Option<&Value>) -> Value {
    let Some(content) = content else {
        return Value::String(String::new());
    };
    match content {
        Value::String(s) => Value::String(s.clone()),
        Value::Array(parts) => {
            let mut has_image = false;
            let mut blocks: Vec<Value> = Vec::new();
            for part in parts {
                let kind = part.get("type").and_then(|t| t.as_str()).unwrap_or("text");
                match kind {
                    "input_image" | "image_url" | "output_image" => {
                        has_image = true;
                        let url = part
                            .get("image_url")
                            .and_then(|v| v.as_str().map(str::to_string).or_else(|| {
                                v.get("url").and_then(|u| u.as_str()).map(str::to_string)
                            }))
                            .or_else(|| part.get("url").and_then(|u| u.as_str()).map(str::to_string))
                            .or_else(|| {
                                part.get("image_url")
                                    .and_then(|v| v.get("url"))
                                    .and_then(|u| u.as_str())
                                    .map(str::to_string)
                            })
                            .unwrap_or_default();
                        blocks.push(json!({ "type": "image_url", "image_url": { "url": url } }));
                    }
                    _ => {
                        if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                            blocks.push(json!({ "type": "text", "text": text }));
                        }
                    }
                }
            }
            if !has_image {
                let text = blocks
                    .iter()
                    .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join("");
                Value::String(text)
            } else {
                Value::Array(blocks)
            }
        }
        other => other.clone(),
    }
}

/// Responses 的扁平 function schema → Chat 的 `{type:"function", function:{…}}`。
///
/// `parameters` 统一过一遍 [`super::normalize_function_parameters`]：严格上游
/// （DeepSeek 等）要求 `type: "object"` + `properties`，Codex 发来的可能是裸 `$ref` 或缺字段。
fn convert_tools(tools: Option<&Value>) -> Vec<Value> {
    let Some(Value::Array(tools)) = tools else {
        return Vec::new();
    };
    tools
        .iter()
        .filter_map(|tool| {
            let kind = tool.get("type").and_then(|t| t.as_str()).unwrap_or("function");
            if kind != "function" {
                // web_search / computer_use 这类内置工具第三方上游不认，直接跳过
                return None;
            }
            let name = tool.get("name").and_then(|v| v.as_str())?;
            let mut function = Map::new();
            function.insert("name".to_string(), json!(name));
            if let Some(desc) = tool.get("description") {
                function.insert("description".to_string(), desc.clone());
            }
            function.insert(
                "parameters".to_string(),
                super::normalize_function_parameters(tool.get("parameters")),
            );
            if let Some(strict) = tool.get("strict") {
                function.insert("strict".to_string(), strict.clone());
            }
            Some(json!({ "type": "function", "function": Value::Object(function) }))
        })
        .collect()
}

/// `o` 系列（`o1`/`o3`/`o4-mini`…）用 `max_completion_tokens`，其余用 `max_tokens`。
fn is_o_series(model: &str) -> bool {
    let base = model.rsplit('/').next().unwrap_or(model).to_lowercase();
    let mut chars = base.chars();
    matches!(chars.next(), Some('o')) && chars.next().is_some_and(|c| c.is_ascii_digit())
}

/// Chat Completions 响应（非流式）→ Responses 响应。
pub fn chat_to_responses(chat: &Value, request: &Value) -> Value {
    let chat_id = chat.get("id").and_then(|v| v.as_str()).unwrap_or("chatcmpl");
    let response_id = format!("resp_{chat_id}");
    let created = chat.get("created").and_then(|v| v.as_i64()).unwrap_or(0);
    let model = chat
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| request.get("model").and_then(|m| m.as_str()).unwrap_or(""));

    let choice = chat.get("choices").and_then(|c| c.get(0));
    let message = choice.and_then(|c| c.get("message")).cloned().unwrap_or(Value::Null);
    let mut output = Vec::new();

    if let Some(rc) = message
        .get("reasoning_content")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        output.push(json!({
            "id": format!("rs_{response_id}"),
            "type": "reasoning",
            "reasoning_content": rc,
            "summary": [{ "type": "summary_text", "text": rc }]
        }));
    }

    let text = match message.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    };
    if !text.is_empty() {
        output.push(json!({
            "id": format!("msg_{}", message_item_suffix(&response_id)),
            "type": "message",
            "status": "completed",
            "role": "assistant",
            "content": [{ "type": "output_text", "text": text, "annotations": [] }]
        }));
    }

    if let Some(tool_calls) = message.get("tool_calls").and_then(|v| v.as_array()) {
        for call in tool_calls {
            let call_id = call.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let name = call
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let arguments = call
                .get("function")
                .and_then(|f| f.get("arguments"))
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .unwrap_or_else(|| "{}".to_string());
            output.push(json!({
                "id": format!("fc_{call_id}"),
                "type": "function_call",
                "status": "completed",
                "call_id": call_id,
                "name": name,
                "arguments": arguments
            }));
        }
    }

    let finish_reason = choice
        .and_then(|c| c.get("finish_reason"))
        .and_then(|v| v.as_str())
        .unwrap_or("stop");
    let status = if finish_reason == "length" {
        "incomplete"
    } else {
        "completed"
    };

    let mut resp = json!({
        "id": response_id,
        "object": "response",
        "created_at": created,
        "status": status,
        "model": model,
        "output": output,
        "usage": usage_to_responses(chat.get("usage"))
    });
    if status == "incomplete" {
        resp["incomplete_details"] = json!({ "reason": "max_output_tokens" });
    }
    // 回写请求侧字段：Codex 会读回自己发过的这些值
    for key in [
        "instructions",
        "max_output_tokens",
        "tools",
        "tool_choice",
        "parallel_tool_calls",
        "reasoning",
        "temperature",
        "top_p",
        "metadata",
        "previous_response_id",
    ] {
        if let Some(v) = request.get(key) {
            if !v.is_null() {
                resp[key] = v.clone();
            }
        }
    }
    resp
}

/// message item 的 id 必须是 `msg_` + **去掉 `resp_` 前缀**的 response id，
/// 否则 Codex 拒收（抄 CodexPlusPlus `protocol_proxy.rs:1913-1920`）。
fn message_item_suffix(response_id: &str) -> &str {
    response_id.strip_prefix("resp_").unwrap_or(response_id)
}

/// Chat usage → Responses usage。`output_tokens_details.reasoning_tokens` 是**必填**，
/// 缺 usage 时也要给零值，不然 Codex 解析失败。
pub fn usage_to_responses(usage: Option<&Value>) -> Value {
    let input = usage
        .and_then(|u| u.get("prompt_tokens").or_else(|| u.get("input_tokens")))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let output = usage
        .and_then(|u| u.get("completion_tokens").or_else(|| u.get("output_tokens")))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let reasoning = usage
        .and_then(|u| u.pointer("/completion_tokens_details/reasoning_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let cached = usage
        .and_then(|u| u.pointer("/prompt_tokens_details/cached_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    json!({
        "input_tokens": input,
        "input_tokens_details": { "cached_tokens": cached },
        "output_tokens": output,
        "output_tokens_details": { "reasoning_tokens": reasoning },
        "total_tokens": input + output
    })
}

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
    usage: Option<Value>,
    finish_reason: Option<String>,
    failed: bool,
}

impl ResponsesStreamConverter {
    pub fn new(model: &str) -> Self {
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
            usage: None,
            finish_reason: None,
            failed: false,
        }
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

    /// 处理一个 Chat SSE chunk（已解析的 JSON），返回要写回客户端的事件文本。
    pub fn push_chunk(&mut self, chunk: &Value) -> Vec<String> {
        if self.failed {
            return Vec::new();
        }
        // 流内错误：转成 response.failed 并停止后续事件
        if let Some(err) = chunk.get("error") {
            self.failed = true;
            let message = err
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("upstream stream error");
            return vec![
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
            ];
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
                events.extend(self.ensure_message_block());
                self.msg_text.push_str(text);
                events.push(sse(
                    "response.output_text.delta",
                    json!({
                        "type": "response.output_text.delta",
                        "item_id": self.msg_id,
                        "output_index": self.msg_index.unwrap_or(0),
                        "content_index": 0,
                        "delta": text
                    }),
                ));
            }
        }

        if let Some(tool_calls) = delta.get("tool_calls").and_then(|v| v.as_array()) {
            for call in tool_calls {
                let seq = call.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let call_id = call.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
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
mod tests {
    use super::*;

    fn codex_request() -> Value {
        json!({
            "model": "gpt-5",
            "instructions": "You are a coding agent.",
            "input": [
                { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "hi" }] },
                { "type": "function_call", "call_id": "call_1", "name": "shell", "arguments": "{\"cmd\":\"ls\"}" },
                { "type": "function_call_output", "call_id": "call_1", "output": "a.txt" }
            ],
            "tools": [{ "type": "function", "name": "shell", "description": "run", "parameters": { "type": null, "properties": {} } }],
            "tool_choice": "auto",
            "parallel_tool_calls": true,
            "max_output_tokens": 8192,
            "reasoning": { "effort": "high" },
            "stream": true,
            "store": false,
            "previous_response_id": "resp_prev"
        })
    }

    #[test]
    fn responses_to_chat_builds_messages_and_drops_session_fields() {
        let chat = responses_to_chat(&codex_request());
        assert_eq!(chat["model"], json!("gpt-5"));
        assert_eq!(chat["messages"][0]["role"], json!("system"));
        assert_eq!(chat["messages"][0]["content"], json!("You are a coding agent."));
        assert_eq!(chat["messages"][1]["content"], json!("hi"), "纯文本块要塌缩成字符串");
        // function_call 与它的输出配对：assistant.tool_calls + role=tool
        assert_eq!(chat["messages"][2]["role"], json!("assistant"));
        assert_eq!(chat["messages"][2]["tool_calls"][0]["function"]["name"], json!("shell"));
        assert_eq!(chat["messages"][3]["role"], json!("tool"));
        assert_eq!(chat["messages"][3]["tool_call_id"], json!("call_1"));
        // 严格上游不允许 `type: null`
        assert_eq!(chat["tools"][0]["function"]["parameters"]["type"], json!("object"));
        assert_eq!(chat["tools"][0]["type"], json!("function"));
        assert_eq!(chat["reasoning_effort"], json!("high"));
        assert_eq!(chat["max_tokens"], json!(8192));
        assert_eq!(chat["stream_options"]["include_usage"], json!(true));
        // 会话字段必须丢弃
        assert!(chat.get("store").is_none(), "{chat}");
        assert!(chat.get("previous_response_id").is_none(), "{chat}");
        assert!(chat.get("instructions").is_none(), "{chat}");
    }

    #[test]
    fn o_series_uses_max_completion_tokens() {
        let mut body = codex_request();
        body["model"] = json!("o3-mini");
        let chat = responses_to_chat(&body);
        assert_eq!(chat["max_completion_tokens"], json!(8192));
        assert!(chat.get("max_tokens").is_none(), "{chat}");
    }

    #[test]
    fn chat_to_responses_shapes_items_and_usage() {
        let chat = json!({
            "id": "chatcmpl-1",
            "created": 1730000000,
            "model": "gpt-5",
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "content": "let me check",
                    "reasoning_content": "thinking…",
                    "tool_calls": [{ "id": "call_9", "type": "function", "function": { "name": "shell", "arguments": "{\"cmd\":\"ls\"}" } }]
                }
            }],
            "usage": { "prompt_tokens": 10, "completion_tokens": 5, "prompt_tokens_details": { "cached_tokens": 3 } }
        });
        let resp = chat_to_responses(&chat, &codex_request());
        assert_eq!(resp["id"], json!("resp_chatcmpl-1"));
        assert_eq!(resp["status"], json!("completed"));
        // message item 的 id 必须是 msg_ + 去掉 resp_ 的 body（Codex 会校验）
        assert_eq!(resp["output"][1]["id"], json!("msg_chatcmpl-1"));
        assert_eq!(resp["output"][0]["type"], json!("reasoning"));
        assert_eq!(resp["output"][2]["type"], json!("function_call"));
        assert_eq!(resp["output"][2]["call_id"], json!("call_9"));
        assert_eq!(resp["usage"]["input_tokens"], json!(10));
        assert_eq!(resp["usage"]["output_tokens_details"]["reasoning_tokens"], json!(0));
        assert_eq!(resp["usage"]["input_tokens_details"]["cached_tokens"], json!(3));
        // 请求侧字段回写
        assert_eq!(resp["instructions"], json!("You are a coding agent."));
    }

    #[test]
    fn length_finish_marks_incomplete() {
        let chat = json!({
            "id": "c1",
            "choices": [{ "finish_reason": "length", "message": { "content": "x" } }]
        });
        let resp = chat_to_responses(&chat, &json!({}));
        assert_eq!(resp["status"], json!("incomplete"));
        assert_eq!(resp["incomplete_details"]["reason"], json!("max_output_tokens"));
    }

    #[test]
    fn stream_emits_created_then_text_then_completed() {
        let mut conv = ResponsesStreamConverter::new("gpt-5");
        let mut out = conv.push_chunk(&json!({
            "id": "chatcmpl-abc",
            "created": 1730000000,
            "model": "gpt-5",
            "choices": [{ "delta": { "content": "He" } }]
        }));
        out.extend(conv.push_chunk(&json!({
            "choices": [{ "delta": { "content": "llo" }, "finish_reason": "stop" }]
        })));
        out.extend(conv.push_chunk(&json!({
            "choices": [],
            "usage": { "prompt_tokens": 7, "completion_tokens": 2 }
        })));
        out.extend(conv.finish());
        let joined = out.join("");

        // 事件序列与顺序
        let order: Vec<&str> = vec![
            "event: response.created",
            "event: response.in_progress",
            "event: response.output_item.added",
            "event: response.content_part.added",
            "event: response.output_text.delta",
            "event: response.output_text.done",
            "event: response.content_part.done",
            "event: response.output_item.done",
            "event: response.completed",
        ];
        let mut cursor = 0usize;
        for name in order {
            let found = joined[cursor..]
                .find(name)
                .unwrap_or_else(|| panic!("缺少事件 {name}\n{joined}"));
            cursor += found + name.len();
        }
        assert!(joined.ends_with("data: [DONE]\n\n"), "{joined}");
        assert!(joined.contains(r#""delta":"He""#), "{joined}");
        assert!(joined.contains(r#""text":"Hello""#), "收尾要带上完整文本: {joined}");
        assert!(joined.contains(r#""input_tokens":7"#), "usage 要落到 completed: {joined}");
        // message item id 用上游 id 去掉 resp_ 前缀
        assert!(joined.contains(r#""id":"msg_chatcmpl-abc""#), "{joined}");
    }

    #[test]
    fn stream_reports_upstream_error_as_response_failed() {
        let mut conv = ResponsesStreamConverter::new("gpt-5");
        let out = conv.push_chunk(&json!({ "error": { "message": "boom" } }));
        let joined = out.join("");
        assert!(joined.contains("event: response.failed"), "{joined}");
        assert!(joined.contains("boom"), "{joined}");
        assert!(joined.ends_with("data: [DONE]\n\n"), "{joined}");
        // 失败之后不再产出任何事件
        assert!(conv.finish().is_empty());
    }
}
