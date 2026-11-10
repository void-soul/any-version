//! 非流式「正常结束却零答案」的判定（抄 magpie `627fd1a3b` 的语义，按我们的协议形态实现）。
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use serde_json::{json, Value};

use crate::proxy::server::ProxyState;

/// 非流式成功响应前的闸门：命中就记失败并返回 502，让上游/聚合能按失败处理。
/// 单独成函数是为了让白名单里的 `server/response.rs` 只留一行接线。
pub async fn guard_non_stream(
    state: &ProxyState,
    outbound: &str,
    body: Value,
) -> Result<Value, Response> {
    if outbound == "openai" && is_empty_chat_completion(&body) {
        state.stats.write().await.failed_requests += 1;
        return Err((
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": {
                "message": "upstream returned an empty completion",
                "type": "upstream_empty_answer"
            }})),
        )
            .into_response());
    }
    Ok(body)
}

/// 上游回了 200、`finish_reason=stop`，却没有任何正文/工具调用/思考，且 usage 为 0。
///
/// 这类响应既不是截断（截断由 `stream_guard` / 流路径处理），也不是请求错误，
/// 所以旧链路会把它当成功下发：客户端收到一个空回答，聚合/故障转移也拿不到失败信号。
/// 只认 **OpenAI 出站形态**；其它协议形态暂不判，避免凭猜测误杀。
pub fn is_empty_chat_completion(body: &Value) -> bool {
    let Some(choice) = body.get("choices").and_then(|c| c.get(0)) else {
        return false;
    };
    // 没有明确结束原因 = 异常收尾，交给截断/错误分支处理，不在这里定罪。
    if choice
        .get("finish_reason")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .is_empty()
    {
        return false;
    }
    let null = Value::Null;
    let message = choice.get("message").unwrap_or(&null);
    let has_content = match message.get("content") {
        Some(Value::String(s)) => !s.trim().is_empty(),
        Some(Value::Array(parts)) => !parts.is_empty(),
        Some(Value::Null) | None => false,
        Some(_) => true,
    };
    let has_tools = message
        .get("tool_calls")
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty());
    let has_reasoning = message
        .get("reasoning_content")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty());
    let usage = body.get("usage");
    let zero_usage = usage
        .and_then(|u| u.get("total_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        == 0
        && usage
            .and_then(|u| u.get("completion_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            == 0;
    !has_content && !has_tools && !has_reasoning && zero_usage
}

#[cfg(test)]
mod tests {
    use super::is_empty_chat_completion;
    use serde_json::json;

    fn base(message: serde_json::Value) -> serde_json::Value {
        json!({
            "choices": [{"index": 0, "message": message, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0}
        })
    }

    #[test]
    fn empty_stop_with_zero_usage_is_empty() {
        assert!(is_empty_chat_completion(&base(json!({"role": "assistant", "content": null}))));
        assert!(is_empty_chat_completion(&base(json!({"role": "assistant", "content": "  "}))));
    }

    #[test]
    fn real_answer_is_not_empty() {
        assert!(!is_empty_chat_completion(&base(json!({"role": "assistant", "content": "hi"}))));
    }

    #[test]
    fn tool_calls_and_reasoning_are_content_too() {
        assert!(!is_empty_chat_completion(&base(
            json!({"role": "assistant", "content": null, "tool_calls": [{"id": "c0"}]})
        )));
        assert!(!is_empty_chat_completion(&base(
            json!({"role": "assistant", "content": null, "reasoning_content": "thinking"})
        )));
    }

    #[test]
    fn missing_finish_reason_is_not_here() {
        let mut body = base(json!({"role": "assistant", "content": null}));
        body["choices"][0]["finish_reason"] = json!(null);
        assert!(!is_empty_chat_completion(&body));
    }

    #[test]
    fn non_zero_usage_is_not_empty() {
        let mut body = base(json!({"role": "assistant", "content": null}));
        body["usage"]["completion_tokens"] = json!(3);
        assert!(!is_empty_chat_completion(&body), "有产出就不该误杀");
    }

    #[test]
    fn other_shapes_are_ignored() {
        assert!(!is_empty_chat_completion(&json!({"content": []})));
        assert!(!is_empty_chat_completion(&json!({"choices": []})));
    }
}
