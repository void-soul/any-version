//! 纯思考轮次的兜底：整轮只有思考、没有正文也没有工具调用时，
//! 把思考原文**同时**作为正文下发。
//!
//! 为什么需要：思考型模型（`deepseek-*` / `*-thinking`）在「只想不答」的轮次里
//! 会只输出 `reasoning_content`。OpenAI 形态的客户端把 `content` 当正文渲染，
//! 于是用户看到一个**完全空白**的回答（像卡住了），实际上模型已经想完了。
//!
//! 三条边界（照参考实现）：
//! 1. 只在「正文为空 **且** 无工具调用 **且** 思考非空」时补 —— 有正文或有工具调用的
//!    轮次一个字节都不改（工具轮回填会让客户端把文字当普通回答）；
//! 2. `reasoning_content` **原样保留**，两个字段同值（想同时渲染两栏的客户端会显示两遍，
//!    这是已知代价，参考侧明确接受）；
//! 3. 流式补帧必须插在 `finish_reason` **之前** —— 很多客户端看到 finish 就停止累积，
//!    补在它后面等于没发。所以调用方要先把收尾字节缓冲起来。
//!
//! 契约来源：Go 版 workbuddy2api 的 `reasoning-as-content` + `reasoning-frame-order` 补丁。

use serde_json::Value;

/// 已聚合的 `chat.completion`：把纯思考轮次的思考原文写进 `content`。
///
/// 逐 choice 判定，返回是否有改动。`reasoning_content` 不动。
pub fn patch_chat_response(body: &mut Value) -> usize {
    let Some(choices) = body.get_mut("choices").and_then(|c| c.as_array_mut()) else {
        return 0;
    };
    let mut changed = 0;
    for choice in choices.iter_mut() {
        let Some(message) = choice.get_mut("message").and_then(|m| m.as_object_mut()) else {
            continue;
        };
        if has_content(message) || has_tool_calls(message) {
            continue;
        }
        let Some(reasoning) = reasoning_text(message) else {
            continue;
        };
        message.insert("content".to_string(), Value::String(reasoning));
        changed += 1;
    }
    changed
}

fn has_content(message: &serde_json::Map<String, Value>) -> bool {
    match message.get("content") {
        // 缺失 / null / 空串 / 空数组 都算「没有正文」
        None | Some(Value::Null) => false,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(parts)) => !parts.is_empty(),
        Some(_) => true,
    }
}

fn has_tool_calls(message: &serde_json::Map<String, Value>) -> bool {
    message
        .get("tool_calls")
        .and_then(|t| t.as_array())
        .is_some_and(|a| !a.is_empty())
}

fn reasoning_text(message: &serde_json::Map<String, Value>) -> Option<String> {
    for key in ["reasoning_content", "reasoning"] {
        if let Some(Value::String(s)) = message.get(key) {
            if !s.trim().is_empty() {
                return Some(s.clone());
            }
        }
    }
    None
}

/// 流式观测器：边转发边判断「整轮是否只有思考」。
#[derive(Default)]
pub struct ReasoningTracker {
    buf: String,
    saw_content: bool,
    saw_tools: bool,
}

impl ReasoningTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// 观测一帧上游 chunk（OpenAI 形态）。
    pub fn observe(&mut self, chunk: &Value) {
        let Some(choices) = chunk.get("choices").and_then(|c| c.as_array()) else {
            return;
        };
        for choice in choices {
            let Some(delta) = choice.get("delta") else {
                continue;
            };
            if delta
                .get("content")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty())
            {
                self.saw_content = true;
            }
            if delta
                .get("tool_calls")
                .and_then(|v| v.as_array())
                .is_some_and(|a| !a.is_empty())
            {
                self.saw_tools = true;
            }
            for key in ["reasoning_content", "reasoning"] {
                if let Some(s) = delta.get(key).and_then(|v| v.as_str()) {
                    if !s.is_empty() {
                        self.buf.push_str(s);
                    }
                }
            }
        }
    }

    pub fn saw_content(&self) -> bool {
        self.saw_content
    }

    pub fn saw_tools(&self) -> bool {
        self.saw_tools
    }

    /// 是否仍然「可能」需要补正文（用于决定要不要开始缓冲收尾字节）。
    ///
    /// 一旦出现过正文或工具调用，就永远不需要补 —— 调用方据此停止缓冲，
    /// 让正常轮次回到零开销的字节透传。
    pub fn may_backfill(&self) -> bool {
        !self.saw_content && !self.saw_tools
    }

    /// 最终判定 + 取走思考原文。调用一次即清空缓冲。
    pub fn take_backfill_text(&mut self) -> Option<String> {
        if self.saw_content || self.saw_tools {
            return None;
        }
        let text = std::mem::take(&mut self.buf);
        if text.trim().is_empty() {
            None
        } else {
            Some(text)
        }
    }
}

/// 构造补发的 SSE 帧（OpenAI 形态，`finish_reason: null`）。
pub fn build_frame(id: &str, created: i64, text: &str) -> String {
    let frame = serde_json::json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": created,
        "choices": [{
            "index": 0,
            "delta": { "content": text },
            "finish_reason": null
        }]
    });
    format!("data: {frame}\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn response(message: Value) -> Value {
        json!({"choices": [{"index": 0, "message": message, "finish_reason": "stop"}]})
    }

    #[test]
    fn pure_reasoning_turn_gets_content() {
        let mut body = response(json!({"role": "assistant", "content": "", "reasoning_content": "思考原文"}));
        assert_eq!(patch_chat_response(&mut body), 1);
        assert_eq!(body["choices"][0]["message"]["content"], "思考原文");
        // 思考字段原样保留（两栏客户端会显示两遍，已知代价）
        assert_eq!(
            body["choices"][0]["message"]["reasoning_content"],
            "思考原文"
        );
    }

    #[test]
    fn normal_turn_is_untouched() {
        let mut body = response(json!({"content": "正文", "reasoning_content": "思考"}));
        assert_eq!(patch_chat_response(&mut body), 0);
        assert_eq!(body["choices"][0]["message"]["content"], "正文");
    }

    #[test]
    fn tool_turn_is_untouched() {
        let mut body = response(json!({
            "content": "",
            "reasoning_content": "思考",
            "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "f", "arguments": "{}"}}]
        }));
        assert_eq!(patch_chat_response(&mut body), 0);
        assert_eq!(body["choices"][0]["message"]["content"], "");
    }

    #[test]
    fn nothing_to_backfill_when_reasoning_empty() {
        let mut body = response(json!({"content": "", "reasoning_content": "  "}));
        assert_eq!(patch_chat_response(&mut body), 0);
        let mut body = response(json!({"content": ""}));
        assert_eq!(patch_chat_response(&mut body), 0);
    }

    #[test]
    fn malformed_responses_are_ignored() {
        assert_eq!(patch_chat_response(&mut json!({})), 0);
        assert_eq!(patch_chat_response(&mut json!({"choices": "nope"})), 0);
        assert_eq!(patch_chat_response(&mut json!({"choices": [{"message": 1}]})), 0);
    }

    #[test]
    fn tracker_accumulates_reasoning_across_frames() {
        let mut t = ReasoningTracker::new();
        t.observe(&json!({"choices": [{"delta": {"reasoning_content": "思考A"}}]}));
        t.observe(&json!({"choices": [{"delta": {"reasoning_content": "思考B"}}]}));
        assert!(t.may_backfill());
        assert_eq!(t.take_backfill_text().as_deref(), Some("思考A思考B"));
        // 取走后再判为空
        assert_eq!(t.take_backfill_text(), None);
    }

    #[test]
    fn tracker_stops_when_content_or_tools_appear() {
        let mut with_content = ReasoningTracker::new();
        with_content.observe(&json!({"choices": [{"delta": {"reasoning_content": "想"}}]}));
        with_content.observe(&json!({"choices": [{"delta": {"content": "答"}}]}));
        assert!(!with_content.may_backfill());
        assert_eq!(with_content.take_backfill_text(), None);

        let mut with_tools = ReasoningTracker::new();
        with_tools.observe(&json!({"choices": [{"delta": {"reasoning_content": "想"}}]}));
        with_tools.observe(&json!({"choices": [{"delta": {"tool_calls": [{"index": 0}]}}]}));
        assert!(!with_tools.may_backfill());
        assert_eq!(with_tools.take_backfill_text(), None);
    }

    #[test]
    fn frame_is_a_content_delta_before_finish() {
        let frame = build_frame("chatcmpl-x", 1_700_000_000, "思考");
        assert!(frame.starts_with("data: "));
        assert!(frame.ends_with("\n\n"));
        assert!(frame.contains(r#""delta":{"content":"思考"}"#));
        assert!(frame.contains(r#""finish_reason":null"#));
    }
}
