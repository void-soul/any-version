use super::*;

#[test]
fn tool_call_arguments_parsing() {
    // 标准协议：arguments 是 JSON 字符串
    let msg: serde_json::Value = serde_json::from_str(
            r#"{"tool_calls":[{"function":{"name":"request_files","arguments":"{\"paths\":[\"src/main.rs\"],\"done\":false}"}}]}"#,
        )
        .unwrap();
    let v = parse_tool_call_arguments(&msg).expect("字符串 arguments 应解析成功");
    assert_eq!(v["paths"][0], "src/main.rs");
    assert_eq!(v["done"], false);
    // 容错：部分网关直接发对象
    let msg_obj: serde_json::Value = serde_json::from_str(
        r#"{"tool_calls":[{"function":{"arguments":{"done":true,"reason":"读完了"}}}]}"#,
    )
    .unwrap();
    let v2 = parse_tool_call_arguments(&msg_obj).expect("对象 arguments 应直接返回");
    assert_eq!(v2["done"], true);
    // 非法 JSON 字符串 → None
    let msg_bad: serde_json::Value =
        serde_json::from_str(r#"{"tool_calls":[{"function":{"arguments":"{broken"}}]}"#).unwrap();
    assert!(parse_tool_call_arguments(&msg_bad).is_none());
    // 没有 tool_calls → None
    let msg_none: serde_json::Value = serde_json::from_str(r#"{"content":"hello"}"#).unwrap();
    assert!(parse_tool_call_arguments(&msg_none).is_none());
}

#[test]
fn tools_unsupported_error_classification() {
    // 400/422 + tools 关键词 → 降级
    assert!(is_tools_unsupported_error_text(
        "AI错误 400: tools is not supported by this model"
    ));
    assert!(is_tools_unsupported_error_text(
        "AI错误 422: Unknown parameter: 'tools'"
    ));
    assert!(is_tools_unsupported_error_text(
        "AI错误 400: Unrecognized request argument supplied: functions"
    ));
    // 非 400/422 → 不降级（可能是限流/超时，等价错误不该触发换协议）
    assert!(!is_tools_unsupported_error_text(
        "AI错误 429: tools rate limited"
    ));
    // 400 但与 tools 无关 → 不降级（业务错误原样返回）
    assert!(!is_tools_unsupported_error_text(
        "AI错误 400: context_length_exceeded"
    ));
    // 非 AI 错误格式 → 不降级
    assert!(!is_tools_unsupported_error_text("网络错误"));
}

#[test]
fn tool_spec_is_valid_json_array() {
    let v: serde_json::Value =
        serde_json::from_str(EXPLORER_TOOL_SPEC).expect("工具定义必须是合法 JSON");
    assert!(v.is_array());
    assert_eq!(v[0]["type"], "function");
    assert_eq!(v[0]["function"]["name"], "request_files");
    assert!(v[0]["function"]["parameters"]["properties"]["paths"].is_object());
}

#[test]
fn completion_url_variants() {
    assert_eq!(
        completion_url("https://api.x.com"),
        "https://api.x.com/v1/chat/completions"
    );
    assert_eq!(
        completion_url("https://api.x.com/"),
        "https://api.x.com/v1/chat/completions"
    );
    assert_eq!(
        completion_url("https://api.x.com/v1/"),
        "https://api.x.com/v1/chat/completions"
    );
}

#[test]
fn recoverable_error_classification() {
    // 网络/传输类 → 可恢复
    assert!(is_recoverable_network_error(
        "流式中断: error decoding response body"
    ));
    assert!(is_recoverable_network_error(
        "流式中断: 读流空闲超时（超过 75s 未收到数据）"
    ));
    assert!(is_recoverable_network_error(
        "请求失败: connection reset by peer"
    ));
    assert!(is_recoverable_network_error("stream disconnected (eos)"));
    // send 阶段的超时/失败 → 可恢复
    assert!(is_recoverable_network_error(
        "请求超时: 等待响应头超过 180s"
    ));
    assert!(is_recoverable_network_error(
        "请求失败: error sending request for url (https://api.example.com/v1/chat/completions)"
    ));
    // 业务类错误（AI 主动返回）→ 不重试
    assert!(!is_recoverable_network_error("AI错误 401: Invalid API key"));
    assert!(!is_recoverable_network_error("AI返回空"));
}

#[test]
fn no_output_retry_classification() {
    // 魔搭空壳：200 + 无内容 + 无 tool_calls → 重试（零计费）
    assert!(is_retryable_no_output(200, true, false));
    // 限流（含响应体为空的 429）→ 重试
    assert!(is_retryable_no_output(429, true, false));
    assert!(is_retryable_no_output(429, false, false));
    // 正常内容 → 不重试
    assert!(!is_retryable_no_output(200, false, false));
    // tool-call 响应 content 合法为空 → 不重试
    assert!(!is_retryable_no_output(200, true, true));
    // 5xx 可能已产生计费输出 → 不重试
    assert!(!is_retryable_no_output(500, true, false));
}

#[test]
fn api_error_body_extraction_tolerates_html_and_plain_text() {
    // JSON 正文 → 取 error.message
    assert_eq!(
        extract_api_error_body("{\"error\":{\"message\":\"Rate limit exceeded\"}}"),
        "Rate limit exceeded"
    );
    // 非 JSON（网关 HTML/纯文本）→ 取片段而非报「解析失败」
    assert_eq!(
        extract_api_error_body("<html>502 Bad Gateway</html>"),
        "<html>502 Bad Gateway</html>"
    );
    assert_eq!(extract_api_error_body("  "), "");
    // 超长正文截断
    let long = "x".repeat(1000);
    assert_eq!(extract_api_error_body(&long).chars().count(), 300);
}

#[test]
fn resume_prompt_carries_partial_and_forbids_repetition() {
    let (sys, usr) = resume_prompt(
        "你是架构师",
        "分析项目",
        "{\"summary\":\"x\",\"nodes\":[{\"id\":\"a\"",
    );
    assert!(sys.contains("继续输出模式"), "system 提示应声明继续模式");
    assert!(sys.contains("你是架构师"), "原 system 提示应保留");
    assert!(usr.contains("分析项目"), "原 user 上下文应保留");
    assert!(usr.contains("{\"summary\":\"x\""), "已输出片段应回传");
    assert!(usr.contains("断点"), "应指明从断点继续");
}

#[test]
fn resume_prompt_truncates_long_partial() {
    // 超长 partial：首尾保留 + 中间省略标记，避免撑爆上下文
    let long = "x".repeat(20_000);
    let (_, usr) = resume_prompt("s", "u", &long);
    assert!(usr.contains("中间省略"), "超长片段应折叠中间部分");
    let len = usr.chars().count();
    assert!(len < 8_500, "折叠后长度应受限，实际 {}", len);
}

#[test]
fn extract_json_balances_strings_and_escapes() {
    assert_eq!(extract_json("前缀 {\"a\":1} 后缀"), Some("{\"a\":1}"));
    assert_eq!(
        extract_json("```json\n{\"a\":{\"b\":\"}\"}}\n```"),
        Some("{\"a\":{\"b\":\"}\"}}")
    );
    assert_eq!(
        extract_json("{\"a\":\"{\\\"x\\\"}\"}"),
        Some("{\"a\":\"{\\\"x\\\"}\"}")
    );
    assert_eq!(extract_json("没有任何 JSON"), None);
    assert_eq!(extract_json("{\"未闭合\":1"), None);
}

#[test]
fn strip_prefix_avoids_nested_stream_error() {
    assert_eq!(strip_prefix_of("流式中断: abc", "流式中断: "), "abc");
    assert_eq!(strip_prefix_of("其它错误", "流式中断: "), "其它错误");
}

#[test]
fn log_preview_keeps_head_tail_and_marks_omitted() {
    // 短文本原样返回，不做任何包装
    assert_eq!(log_preview("hello"), "hello");
    // 超长文本：保留头尾 + 标注省略量与总长
    let long = format!("{}{}", "a".repeat(45_000), "z");
    let out = log_preview(&long);
    assert!(out.contains("日志截断"), "超长文本应带截断标记");
    assert!(out.starts_with("aaaa"), "开头应保留");
    assert!(out.ends_with("z"), "结尾应保留");
    let total: usize = long.chars().count();
    assert!(out.contains(&format!("全文共 {} 字符", total)));
    // 恰好等于上限时不截断
    let edge = "x".repeat(LOG_PREVIEW_CAP);
    assert_eq!(log_preview(&edge), edge);
}
