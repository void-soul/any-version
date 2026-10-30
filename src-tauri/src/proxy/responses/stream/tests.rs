use super::super::*;
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
    assert_eq!(
        chat["messages"][0]["content"],
        json!("You are a coding agent.")
    );
    assert_eq!(
        chat["messages"][1]["content"],
        json!("hi"),
        "纯文本块要塌缩成字符串"
    );
    // function_call 与它的输出配对：assistant.tool_calls + role=tool
    assert_eq!(chat["messages"][2]["role"], json!("assistant"));
    assert_eq!(
        chat["messages"][2]["tool_calls"][0]["function"]["name"],
        json!("shell")
    );
    assert_eq!(chat["messages"][3]["role"], json!("tool"));
    assert_eq!(chat["messages"][3]["tool_call_id"], json!("call_1"));
    // 严格上游不允许 `type: null`
    assert_eq!(
        chat["tools"][0]["function"]["parameters"]["type"],
        json!("object")
    );
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
    assert_eq!(
        resp["usage"]["output_tokens_details"]["reasoning_tokens"],
        json!(0)
    );
    assert_eq!(
        resp["usage"]["input_tokens_details"]["cached_tokens"],
        json!(3)
    );
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
    assert_eq!(
        resp["incomplete_details"]["reason"],
        json!("max_output_tokens")
    );
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
    assert!(
        joined.contains(r#""text":"Hello""#),
        "收尾要带上完整文本: {joined}"
    );
    assert!(
        joined.contains(r#""input_tokens":7"#),
        "usage 要落到 completed: {joined}"
    );
    // message item id 用上游 id 去掉 resp_ 前缀
    assert!(joined.contains(r#""id":"msg_chatcmpl-abc""#), "{joined}");
}

/// 上游把工具调用写成 XML 方言时（见 `proxy::xml_tools`）：标记之前的正文照常逐字
/// 流出，XML 本身扣到流末还原成 function_call —— 否则 Codex 会判定轮次结束而停住。
#[test]
fn stream_recovers_xml_dialect_tool_calls() {
    let sample =
        "先梳理结构。]<]minimax[>[<tool_call>\n]<]minimax[>[<invoke name=\"shell_command\">\
            <workdir>E:\\x</workdir><command>dir</command></invoke>\n]<]minimax[>[</tool_call>";
    let mut conv = ResponsesStreamConverter::new("gpt-5");
    let mut out = Vec::new();
    // 切成 7 字一片喂进去：真实流里标记会被拆散在各个 delta 中
    let chars: Vec<char> = sample.chars().collect();
    for piece in chars.chunks(7) {
        let piece: String = piece.iter().collect();
        out.extend(conv.push_chunk(&json!({
            "id": "c1",
            "choices": [{ "delta": { "content": piece } }]
        })));
    }
    out.extend(conv.finish());
    let joined = out.join("");

    assert!(
        joined.contains("先梳理结构。"),
        "正文要照常发出来：{joined}"
    );
    assert!(
        !joined.contains("<invoke"),
        "原始 XML 不该透传给客户端：{joined}"
    );
    assert!(
        !joined.contains("minimax"),
        "上游塞的分隔符要剥掉：{joined}"
    );
    assert!(
        joined.contains("\"type\":\"function_call\""),
        "要还原成 function_call：{joined}"
    );
    assert!(joined.contains("\"name\":\"shell_command\""), "{joined}");
    // 参数在 SSE 里是 JSON 字符串内嵌，直接查会被转义干扰，改查转换器的结果
    assert_eq!(
        conv.tools.len(),
        1,
        "应还原出 1 个工具调用：{:?}",
        conv.tools
    );
    assert_eq!(conv.tools[0].3, "shell_command");
    assert!(
        conv.tools[0].4.contains("\"command\":\"dir\""),
        "参数要还原：{}",
        conv.tools[0].4
    );
    assert!(
        !conv.msg_text.contains("minimax"),
        "正文里不该留 XML：{}",
        conv.msg_text
    );
}

/// 反向护栏：普通正文里的尖括号不能被扣住不发（hold 只保留「可能还是标记前缀」的一小段）
#[test]
fn stream_does_not_swallow_plain_angle_brackets() {
    let mut conv = ResponsesStreamConverter::new("gpt-5");
    let out = conv.push_chunk(&json!({
        "id": "c1",
        "choices": [{ "delta": { "content": "比较 a < b 且 c > d 的大小" } }]
    }));
    let joined = out.join("");
    assert!(
        joined.contains("比较 a < b 且 c > d 的大小"),
        "正文不能被吞：{joined}"
    );
}

/// 扣住是为了还原，不是为了吞：出现工具标记但**解析不出** invoke 时，
/// 原文必须原样回到客户端（否则用户在问 `<invoke>` 语法时内容会凭空消失）
#[test]
fn stream_returns_unparsable_xml_text_to_the_client() {
    let mut conv = ResponsesStreamConverter::new("gpt-5");
    let text = "讲一下 <tool_call> 这个标签怎么用";
    let mut out = conv.push_chunk(&json!({
        "id": "c1",
        "choices": [{ "delta": { "content": text } }]
    }));
    out.extend(conv.finish());
    let joined = out.join("");
    assert!(joined.contains("<tool_call>"), "原文要还回来：{joined}");
    assert!(
        !joined.contains("function_call"),
        "没解析出工具就不该造 function_call：{joined}"
    );
}

/// 模型「调完工具又补一句话」时，那句话不能被吞
#[test]
fn stream_keeps_text_emitted_after_the_tool_call() {
    let mut conv = ResponsesStreamConverter::new("gpt-5");
    let sample = "先读。<tool_call><invoke name=\"read_file\"><path>a.txt</path></invoke></tool_call>读完继续。";
    let chars: Vec<char> = sample.chars().collect();
    let mut out = Vec::new();
    for piece in chars.chunks(9) {
        let piece: String = piece.iter().collect();
        out.extend(conv.push_chunk(&json!({
            "id": "c1",
            "choices": [{ "delta": { "content": piece } }]
        })));
    }
    out.extend(conv.finish());
    let joined = out.join("");
    assert!(
        joined.contains("读完继续。"),
        "工具调用之后的正文要发出来：{joined}"
    );
    assert_eq!(conv.tools.len(), 1);
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
