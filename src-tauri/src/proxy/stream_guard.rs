//! 流式收尾判定：区分「模型正常说完」与「流被半截掐断」。
//!
//! 背景：上游可能返回 `200`、正常吐出若干内容帧，然后在既没有 `finish_reason`
//! 也没有 `[DONE]` 的情况下结束（连接被中间设备或后端掐断）。此时如果代理按
//! 「循环结束 = 正常完成」处理，会：
//! - 给客户端补一个「正常终止」事件（Anthropic `message_stop` / Responses
//!   `response.completed`）→ 客户端把半截回答当完整答案收下、不再续跑；
//! - 在日志里留下一条 tok 正常的「成功」→ 问题长期查不出来。
//!
//! 参考实现（Go 版 workbuddy2api 的 `truncated-stream-signal` 补丁）的判据是：
//! **转发过内容帧 + 没有 finish_reason + 没有 [DONE]** ⇒ 判截断，在 `[DONE]`
//! 之前补一条 error 帧，并把请求状态收敛为失败。
//!
//! 本模块只做「观测 + 判定」这一件纯逻辑，不碰 IO，便于单测。

use serde_json::Value;

/// 一条上游 SSE 流的收尾进度。
#[derive(Debug, Default, Clone, Copy)]
pub struct StreamProgress {
    saw_content: bool,
    saw_finish: bool,
    saw_done: bool,
}

impl StreamProgress {
    pub fn new() -> Self {
        Self::default()
    }

    /// 见到 `data: [DONE]`。
    pub fn observe_done(&mut self) {
        self.saw_done = true;
    }

    /// 观测一帧 `data:` 的原始载荷（自动忽略 `[DONE]` 与非法 JSON）。
    ///
    /// `protocol` 是**出站**（= 上游）协议：流里的帧是上游形态。
    pub fn observe_payload(&mut self, protocol: &str, payload: &str) {
        if payload.trim() == "[DONE]" {
            self.saw_done = true;
            return;
        }
        let Ok(v) = serde_json::from_str::<Value>(payload) else {
            return;
        };
        self.observe_value(protocol, &v);
    }

    /// 观测一帧已解析的 JSON。
    pub fn observe_value(&mut self, protocol: &str, v: &Value) {
        match protocol {
            "anthropic" => self.observe_anthropic(v),
            "google" => self.observe_google(v),
            _ => self.observe_openai(v),
        }
    }

    pub fn saw_content(&self) -> bool {
        self.saw_content
    }

    pub fn saw_finish(&self) -> bool {
        self.saw_finish
    }

    pub fn saw_done(&self) -> bool {
        self.saw_done
    }

    /// 有内容、但既没 `finish_reason` 也没 `[DONE]` ⇒ 截断。
    ///
    /// 一帧内容都没有的「空流」不算截断：那是另一类问题（上游 200 但 0 帧），
    /// 由聚合路径已有的 502 分支处理，别在这里重复定罪。
    pub fn is_truncated(&self) -> bool {
        self.saw_content && !self.saw_finish && !self.saw_done
    }

    fn observe_openai(&mut self, v: &Value) {
        let Some(choices) = v.get("choices").and_then(|c| c.as_array()) else {
            return;
        };
        for c in choices {
            if c.get("finish_reason")
                .and_then(|x| x.as_str())
                .is_some_and(|s| !s.is_empty())
            {
                self.saw_finish = true;
            }
            let Some(delta) = c.get("delta") else {
                continue;
            };
            let has_text = delta
                .get("content")
                .and_then(|x| x.as_str())
                .is_some_and(|s| !s.is_empty())
                || delta
                    .get("reasoning_content")
                    .and_then(|x| x.as_str())
                    .is_some_and(|s| !s.is_empty());
            // 工具调用也是「模型已经产出东西」：`tool_calls` 非空数组才算
            let has_tool = delta
                .get("tool_calls")
                .and_then(|x| x.as_array())
                .is_some_and(|a| !a.is_empty());
            if has_text || has_tool {
                self.saw_content = true;
            }
        }
    }

    fn observe_anthropic(&mut self, v: &Value) {
        match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
            "content_block_delta" => {
                let Some(d) = v.get("delta") else {
                    return;
                };
                let has_text = d
                    .get("text")
                    .and_then(|x| x.as_str())
                    .is_some_and(|s| !s.is_empty())
                    || d.get("thinking")
                        .and_then(|x| x.as_str())
                        .is_some_and(|s| !s.is_empty())
                    || d.get("partial_json").is_some();
                if has_text {
                    self.saw_content = true;
                }
            }
            "message_delta" => {
                if v.get("delta")
                    .and_then(|x| x.get("stop_reason"))
                    .and_then(|x| x.as_str())
                    .is_some()
                {
                    self.saw_finish = true;
                }
            }
            // message_stop 是 Anthropic 的终止事件：有它就不算截断
            "message_stop" => self.saw_finish = true,
            _ => {}
        }
    }

    fn observe_google(&mut self, v: &Value) {
        let Some(cands) = v.get("candidates").and_then(|c| c.as_array()) else {
            return;
        };
        for c in cands {
            if c.get("finishReason")
                .and_then(|x| x.as_str())
                .is_some_and(|s| !s.is_empty())
            {
                self.saw_finish = true;
            }
            let has_text = c
                .pointer("/content/parts")
                .and_then(|p| p.as_array())
                .is_some_and(|parts| {
                    parts.iter().any(|p| {
                        p.get("text")
                            .and_then(|t| t.as_str())
                            .is_some_and(|s| !s.is_empty())
                    })
                });
            if has_text {
                self.saw_content = true;
            }
        }
    }
}

/// 截断时补发给客户端的 OpenAI 形态 error 帧（含 `[DONE]`，保证客户端能收尾）。
///
/// 文案与参考实现一致，便于日志/客户端按同一串搜索。
pub const TRUNCATED_STREAM_FRAME: &str = concat!(
    "data: {\"error\":{\"message\":\"upstream stream ended before completion (truncated)\",",
    "\"type\":\"upstream_error\",\"code\":\"upstream_parse\"}}\n\ndata: [DONE]\n\n"
);

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn openai_chunk(text: Option<&str>, finish: Option<&str>, tools: bool) -> Value {
        let mut delta = json!({});
        if let Some(t) = text {
            delta["content"] = json!(t);
        }
        if tools {
            delta["tool_calls"] = json!([{ "index": 0, "id": "c1" }]);
        }
        json!({ "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }] })
    }

    #[test]
    fn complete_stream_with_done_is_not_truncated() {
        let mut p = StreamProgress::new();
        p.observe_payload("openai", &openai_chunk(Some("hi"), None, false).to_string());
        p.observe_payload("openai", "[DONE]");
        assert!(!p.is_truncated());
        assert!(p.saw_done());
    }

    #[test]
    fn complete_stream_with_finish_reason_is_not_truncated() {
        let mut p = StreamProgress::new();
        p.observe_payload("openai", &openai_chunk(Some("hi"), None, false).to_string());
        p.observe_payload("openai", &openai_chunk(None, Some("stop"), false).to_string());
        assert!(p.saw_finish());
        assert!(!p.is_truncated());
    }

    #[test]
    fn content_without_finish_or_done_is_truncated() {
        let mut p = StreamProgress::new();
        p.observe_payload("openai", &openai_chunk(Some("half"), None, false).to_string());
        assert!(p.saw_content());
        assert!(p.is_truncated());
    }

    #[test]
    fn role_only_frame_is_not_content() {
        // 首个 chunk 常只有 role、content 为空：不能当成「已产出内容」
        let mut p = StreamProgress::new();
        p.observe_payload(
            "openai",
            &json!({ "choices": [{ "delta": { "role": "assistant", "content": "" } }] }).to_string(),
        );
        assert!(!p.saw_content());
        assert!(!p.is_truncated());
    }

    #[test]
    fn tool_call_delta_counts_as_content() {
        let mut p = StreamProgress::new();
        p.observe_payload("openai", &openai_chunk(None, None, true).to_string());
        assert!(p.saw_content());
        assert!(p.is_truncated());
    }

    #[test]
    fn anthropic_stop_reason_and_message_stop_count_as_finish() {
        let mut a = StreamProgress::new();
        a.observe_payload(
            "anthropic",
            &json!({ "type": "content_block_delta", "delta": { "type": "text_delta", "text": "hi" } })
                .to_string(),
        );
        a.observe_payload(
            "anthropic",
            &json!({ "type": "message_delta", "delta": { "stop_reason": "end_turn" } }).to_string(),
        );
        assert!(!a.is_truncated());

        let mut b = StreamProgress::new();
        b.observe_payload(
            "anthropic",
            &json!({ "type": "content_block_delta", "delta": { "text": "hi" } }).to_string(),
        );
        b.observe_payload("anthropic", &json!({ "type": "message_stop" }).to_string());
        assert!(!b.is_truncated());
    }

    #[test]
    fn google_finish_reason_counts_as_finish() {
        let mut p = StreamProgress::new();
        p.observe_payload(
            "google",
            &json!({ "candidates": [{ "content": { "parts": [{ "text": "hi" }] } }] }).to_string(),
        );
        assert!(p.is_truncated());
        p.observe_payload(
            "google",
            &json!({ "candidates": [{ "finishReason": "STOP" }] }).to_string(),
        );
        assert!(!p.is_truncated());
    }

    #[test]
    fn empty_stream_is_not_truncation() {
        // 一帧都没有 = 另一类问题（空流），由聚合路径的 502 分支处理
        let p = StreamProgress::new();
        assert!(!p.is_truncated());
    }

    #[test]
    fn garbage_payload_is_ignored() {
        let mut p = StreamProgress::new();
        p.observe_payload("openai", "not json");
        assert!(!p.saw_content() && !p.saw_finish() && !p.saw_done());
    }

    #[test]
    fn truncated_frame_carries_error_and_done() {
        assert!(TRUNCATED_STREAM_FRAME.contains("upstream_parse"));
        assert!(TRUNCATED_STREAM_FRAME.ends_with("data: [DONE]\n\n"));
    }
}
