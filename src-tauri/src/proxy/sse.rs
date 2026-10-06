//! SSE 流式解析工具

/// 从 buffer 中提取一个完整的 SSE 块（以 \n\n 或 \r\n\r\n 分隔）
pub fn take_sse_block(buffer: &str) -> Option<(String, &str)> {
    // 查找块分隔符
    if let Some(pos) = buffer.find("\n\n") {
        let block = &buffer[..pos];
        let remainder = &buffer[pos + 2..];
        return Some((block.to_string(), remainder));
    }
    if let Some(pos) = buffer.find("\r\n\r\n") {
        let block = &buffer[..pos];
        let remainder = &buffer[pos + 4..];
        return Some((block.to_string(), remainder));
    }
    None
}

/// 从 SSE 块中提取 data 字段值
pub fn extract_sse_data(block: &str) -> Option<String> {
    for line in block.lines() {
        if let Some(rest) = line.strip_prefix("data:") {
            let data = rest.strip_prefix(" ").unwrap_or(rest);
            if !data.is_empty() && data != "[DONE]" {
                return Some(data.to_string());
            }
            if data == "[DONE]" {
                return Some("[DONE]".to_string());
            }
        }
    }
    None
}

/// 从 SSE 块中提取 event 字段值
pub fn extract_sse_event(block: &str) -> Option<String> {
    for line in block.lines() {
        if let Some(rest) = line.strip_prefix("event:") {
            let event = rest.strip_prefix(" ").unwrap_or(rest);
            if !event.is_empty() {
                return Some(event.to_string());
            }
        }
    }
    None
}

/// 安全地将新字节追加到 UTF-8 buffer，处理跨 chunk 的多字节字符
pub fn append_utf8_safe(buffer: &str, new_bytes: &[u8]) -> String {
    let mut combined = buffer.to_string();
    match std::str::from_utf8(new_bytes) {
        Ok(s) => {
            combined.push_str(s);
        }
        Err(e) => {
            // 截取有效的 UTF-8 部分
            let valid_up_to = e.valid_up_to();
            if valid_up_to > 0 {
                combined.push_str(std::str::from_utf8(&new_bytes[..valid_up_to]).unwrap());
            }
            // 剩余的不完整字节留给下一个 chunk 处理
        }
    }
    combined
}

/// 把 OpenAI `chat.completion.chunk` 序列聚合成单个 `chat.completion`。
///
/// 用于「上游只收流式」的场景（`ProxyConfig::force_upstream_stream`）：客户端要的是
/// 非流式响应，但上游拒绝非流式请求（如 WorkBuddy 后端返回
/// `Non-stream chat request is currently not supported`）。于是代理强制发流式、
/// 在本地收集聚合，再交回一个普通 JSON —— 客户端完全看不出中间发生了什么。
///
/// `fallback_model`：上游没给 `model` 时用它（我们请求时用的那个），
/// 别让客户端拿到空 model。
pub fn aggregate_chat_chunks(
    chunks: &[serde_json::Value],
    fallback_model: &str,
) -> serde_json::Value {
    use serde_json::{json, Value};

    let mut id: Option<String> = None;
    let mut model: Option<String> = None;
    let mut created: Option<i64> = None;
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut finish_reason = Value::Null;
    let mut usage: Option<Value> = None;
    let mut role = String::from("assistant");
    // 工具调用：index → (id, type, name, arguments)
    //
    // **必须聚合**：这条路径（上游只收流式 → 代理聚合成非流式）原先只收 content 与
    // reasoning，工具调用被静默丢掉 —— 客户端看到的就是「模型说了段计划然后停住」，
    // 一次工具都不执行。两种来源都认：标准 `delta.tool_calls`（分段增量），
    // 以及少数上游在流中间用 `message.tool_calls` 一次给全。
    let mut tools: Vec<(u64, String, String, String, String)> = Vec::new();

    for chunk in chunks {
        if id.is_none() {
            id = chunk
                .get("id")
                .and_then(|v| v.as_str())
                .map(str::to_string);
        }
        if model.is_none() {
            model = chunk
                .get("model")
                .and_then(|v| v.as_str())
                .map(str::to_string);
        }
        if created.is_none() {
            created = chunk.get("created").and_then(|v| v.as_i64());
        }
        // usage 帧可能没有 choices，单独收
        if let Some(u) = chunk.get("usage") {
            if u.is_object() {
                usage = Some(u.clone());
            }
        }
        let Some(choices) = chunk.get("choices").and_then(|c| c.as_array()) else {
            continue;
        };
        let Some(first) = choices.first() else { continue };
        if let Some(r) = first
            .get("delta")
            .and_then(|d| d.get("role"))
            .and_then(|r| r.as_str())
        {
            if !r.is_empty() {
                role = r.to_string();
            }
        }
        if let Some(delta) = first.get("delta") {
            if let Some(c) = delta.get("content").and_then(|c| c.as_str()) {
                content.push_str(c);
            }
            // 思维链：混元等模型单独给这个字段，不聚合客户端就看不到推理内容
            for key in ["reasoning_content", "reasoning"] {
                if let Some(c) = delta.get(key).and_then(|c| c.as_str()) {
                    reasoning.push_str(c);
                    break;
                }
            }
            if let Some(calls) = delta.get("tool_calls").and_then(|c| c.as_array()) {
                for call in calls {
                    push_tool_call(&mut tools, call, true);
                }
            }
        }
        // 非标准但存在的写法：工具调用放在流中间的 message 里
        if let Some(calls) = first
            .get("message")
            .and_then(|m| m.get("tool_calls"))
            .and_then(|c| c.as_array())
        {
            for call in calls {
                push_tool_call(&mut tools, call, false);
            }
        }
        if let Some(fr) = first.get("finish_reason") {
            if !fr.is_null() {
                finish_reason = fr.clone();
            }
        }
    }

    let mut message = json!({ "role": role, "content": content });
    if !reasoning.is_empty() {
        message["reasoning_content"] = json!(reasoning);
    }
    if !tools.is_empty() {
        let calls: Vec<Value> = tools
            .iter()
            .map(|(_, id, kind, name, args)| {
                json!({
                    "id": id,
                    "type": kind,
                    "function": { "name": name, "arguments": args }
                })
            })
            .collect();
        message["tool_calls"] = json!(calls);
        // 上游没给 finish_reason 时补上：客户端（如 Codex）只认 tool_calls 才会去执行
        if finish_reason.is_null() {
            finish_reason = json!("tool_calls");
        }
    }
    let mut out = json!({
        "id": id.unwrap_or_else(|| "gen-aggregated".to_string()),
        "object": "chat.completion",
        "created": created.unwrap_or(0),
        "model": model.unwrap_or_else(|| fallback_model.to_string()),
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": finish_reason
        }]
    });
    if let Some(u) = usage {
        out["usage"] = u;
    }
    out
}

/// 把一帧里的 tool_calls 条目并进聚合表。
///
/// `incremental = true`（标准 `delta.tool_calls`）时 `arguments` 是**增量片段**，要接在
/// 已有片段之后；`false`（`message.tool_calls` 一次给全）时直接覆盖。
fn push_tool_call(
    tools: &mut Vec<(u64, String, String, String, String)>,
    call: &serde_json::Value,
    incremental: bool,
) {
    let index = call.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
    let id = call
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let kind = call
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("function")
        .to_string();
    let func = call.get("function");
    let name = func
        .and_then(|f| f.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let args = func
        .and_then(|f| f.get("arguments"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    match tools.iter_mut().find(|(i, _, _, _, _)| *i == index) {
        Some(entry) => {
            if !id.is_empty() {
                entry.1 = id;
            }
            if !name.is_empty() {
                entry.3 = name;
            }
            if incremental {
                entry.4.push_str(&args);
            } else if !args.is_empty() {
                entry.4 = args;
            }
        }
        None => tools.push((index, id, kind, name, args)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn aggregate_keeps_streamed_tool_calls() {
        // 这条路径（上游只收流式 → 聚合成非流式）原先**完全丢弃** tool_calls，
        // 客户端表现为「模型说了段计划然后停住」，一次工具都不执行
        let chunks = vec![
            json!({"id": "c1", "model": "m", "choices": [
                {"index": 0, "delta": {"role": "assistant", "content": "我先读文件。"}}]}),
            json!({"choices": [{"index": 0, "delta": {"tool_calls": [
                {"index": 0, "id": "call_1", "type": "function",
                 "function": {"name": "read_file", "arguments": ""}}]}}]}),
            json!({"choices": [{"index": 0, "delta": {"tool_calls": [
                {"index": 0, "function": {"arguments": "{\"path\":"}}]}}]}),
            json!({"choices": [{"index": 0, "delta": {"tool_calls": [
                {"index": 0, "function": {"arguments": "\"a.txt\"}"}}]}}]}),
            json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]}),
        ];
        let out = aggregate_chat_chunks(&chunks, "m");
        let msg = &out["choices"][0]["message"];
        assert_eq!(msg["content"], json!("我先读文件。"));
        assert_eq!(msg["tool_calls"].as_array().unwrap().len(), 1, "{msg}");
        assert_eq!(msg["tool_calls"][0]["id"], json!("call_1"));
        assert_eq!(msg["tool_calls"][0]["function"]["name"], json!("read_file"));
        // arguments 分三段送来，必须拼回完整 JSON
        assert_eq!(
            msg["tool_calls"][0]["function"]["arguments"],
            json!("{\"path\":\"a.txt\"}")
        );
        assert_eq!(out["choices"][0]["finish_reason"], json!("tool_calls"));
    }

    #[test]
    fn aggregate_picks_up_tool_calls_sent_as_message() {
        // 少数上游在流中间用 message.tool_calls 一次给全（arguments 不是增量）
        let chunks = vec![
            json!({"id": "c2", "choices": [{"index": 0, "delta": {"content": "改首页。"}}]}),
            json!({"choices": [{"index": 0, "delta": {},
                "message": {"tool_calls": [
                    {"index": 0, "id": "call_x", "type": "function",
                     "function": {"name": "edit", "arguments": "{\"file\":\"a\"}"}}]}}]}),
        ];
        let out = aggregate_chat_chunks(&chunks, "m");
        let msg = &out["choices"][0]["message"];
        assert_eq!(msg["tool_calls"][0]["function"]["name"], json!("edit"));
        assert_eq!(msg["tool_calls"][0]["function"]["arguments"], json!("{\"file\":\"a\"}"));
        // 上游没给 finish_reason 时补 tool_calls，客户端才会去执行
        assert_eq!(out["choices"][0]["finish_reason"], json!("tool_calls"));
    }

    #[test]
    fn aggregate_leaves_finish_reason_alone_without_tool_calls() {
        let chunks = vec![json!({"choices": [{"index": 0, "delta": {"content": "hi"}}]})];
        let out = aggregate_chat_chunks(&chunks, "m");
        assert!(out["choices"][0]["finish_reason"].is_null());
        assert!(out["choices"][0]["message"].get("tool_calls").is_none());
    }

    #[test]
    fn aggregate_concatenates_content_and_keeps_last_finish_reason() {
        // 上游只收流式时，代理要把 SSE 聚合成单个 chat.completion 交回客户端
        let chunks = vec![
            json!({"id":"gen-1","model":"hy3","created":1700000000,
                   "choices":[{"index":0,"delta":{"role":"assistant"},"finish_reason":null}]}),
            json!({"id":"gen-1","choices":[{"index":0,"delta":{"reasoning_content":"在想"},"finish_reason":null}]}),
            json!({"id":"gen-1","choices":[{"index":0,"delta":{"content":"在标准"},"finish_reason":null}]}),
            json!({"id":"gen-1","choices":[{"index":0,"delta":{"content":"模型"},"finish_reason":null}]}),
            json!({"id":"gen-1","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],
                   "usage":{"prompt_tokens":23,"completion_tokens":2,"total_tokens":25}}),
        ];
        let out = aggregate_chat_chunks(&chunks, "hy3");
        assert_eq!(out["object"], "chat.completion");
        assert_eq!(out["id"], "gen-1");
        assert_eq!(out["model"], "hy3");
        assert_eq!(out["created"], 1700000000);
        assert_eq!(out["choices"][0]["message"]["role"], "assistant");
        assert_eq!(out["choices"][0]["message"]["content"], "在标准模型", "content 要按顺序拼接");
        assert_eq!(
            out["choices"][0]["message"]["reasoning_content"], "在想",
            "思维链也要聚合（WorkBuddy 的 hy3 会单独给这个字段）"
        );
        assert_eq!(out["choices"][0]["finish_reason"], "stop", "取最后一个非空 finish_reason");
        assert_eq!(out["usage"]["total_tokens"], 25, "上游给了 usage 就沿用");
    }

    #[test]
    fn aggregate_yields_valid_shape_when_nothing_usable_arrived() {
        // 没有内容也要给出合法结构，客户端才不会解析崩
        let out = aggregate_chat_chunks(&[], "hy3");
        assert_eq!(out["object"], "chat.completion");
        assert_eq!(out["model"], "hy3");
        assert_eq!(out["choices"][0]["message"]["content"], "");
        assert!(out["choices"][0]["finish_reason"].is_null());
    }

    #[test]
    fn aggregate_uses_fallback_model_when_upstream_omits_it() {
        // 上游没给 model 时用我们请求时用的那个，别让客户端拿到空 model
        let chunks = vec![json!({"choices":[{"index":0,"delta":{"content":"x"},"finish_reason":null}]})];
        let out = aggregate_chat_chunks(&chunks, "hy4-preview");
        assert_eq!(out["model"], "hy4-preview");
        assert_eq!(out["id"], "gen-aggregated", "要有个稳定的 id");
    }

    #[test]
    fn aggregate_ignores_chunks_without_choices() {
        // 用量汇总之类的帧可能没有 choices，跳过而不是崩
        let chunks = vec![
            json!({"id":"g1"}),
            json!({"id":"g1","choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":null}]}),
        ];
        let out = aggregate_chat_chunks(&chunks, "m");
        assert_eq!(out["choices"][0]["message"]["content"], "ok");
        assert_eq!(out["id"], "g1");
    }
}
