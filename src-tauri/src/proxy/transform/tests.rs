//! tests implementation.
use super::*;

#[cfg(test)]
mod stream_block_tests {
    use super::super::StreamConverter;
    use serde_json::{json, Value};

    /// 把转换器产出的一行 SSE 文本解析成 (event 名, data JSON)。
    fn parse_line(raw: &str) -> Option<(String, Value)> {
        let mut name = String::new();
        let mut data = String::new();
        for line in raw.lines() {
            if let Some(rest) = line.strip_prefix("event:") {
                name = rest.trim().to_string();
            } else if let Some(rest) = line.strip_prefix("data:") {
                data = rest.trim().to_string();
            }
        }
        if data.is_empty() {
            return None;
        }
        serde_json::from_str::<Value>(&data).ok().map(|v| (name, v))
    }

    /// 收集所有事件的 (type, index)。
    fn events_of(raw: &[String]) -> Vec<(String, i64)> {
        raw.iter()
            .filter_map(|e| parse_line(e))
            .map(|(name, v)| {
                let ty = v
                    .get("type")
                    .and_then(|x| x.as_str())
                    .unwrap_or(&name)
                    .to_string();
                let idx = v.get("index").and_then(|x| x.as_i64()).unwrap_or(-1);
                (ty, idx)
            })
            .collect()
    }

    fn chunk(delta: Value, finish: Option<&str>) -> Value {
        let mut choice = json!({"index": 0, "delta": delta, "finish_reason": finish});
        if finish.is_some() {
            choice["finish_reason"] = json!(finish.unwrap());
        }
        json!({"choices": [choice]})
    }

    /// A1 回归：tool_use 的 start / delta / stop 必须落在**同一个** index 上。
    /// 修复前：start=0、delta=1、stop=1 → 块 0 永不闭合、凭空多一个未 start 的块。
    #[test]
    fn tool_use_block_start_delta_stop_share_one_index() {
        let mut c = StreamConverter::new("deepseek-chat".into());

        let mut out = c.convert_chunk(&chunk(
            json!({"tool_calls": [{
                "index": 0,
                "id": "call_1",
                "function": {"name": "read_file", "arguments": "{\"path\":"}
            }]}),
            None,
        ));
        out.extend(c.convert_chunk(&chunk(
            json!({"tool_calls": [{
                "index": 0,
                "function": {"arguments": "\"a.rs\"}"}
            }]}),
            None,
        )));
        out.extend(c.convert_chunk(&chunk(json!({}), Some("tool_calls"))));

        let evs = events_of(&out);
        let starts: Vec<i64> = evs
            .iter()
            .filter(|(t, _)| t == "content_block_start")
            .map(|(_, i)| *i)
            .collect();
        let deltas: Vec<i64> = evs
            .iter()
            .filter(|(t, _)| t == "content_block_delta")
            .map(|(_, i)| *i)
            .collect();
        let stops: Vec<i64> = evs
            .iter()
            .filter(|(t, _)| t == "content_block_stop")
            .map(|(_, i)| *i)
            .collect();

        assert_eq!(starts, vec![0], "只应开一个块，实际 {starts:?}");
        assert_eq!(
            deltas,
            vec![0, 0],
            "两个 delta 都要挂在块 0，实际 {deltas:?}"
        );
        assert_eq!(stops, vec![0], "块 0 必须闭合，实际 {stops:?}");
        assert!(evs.iter().any(|(t, _)| t == "message_stop"));
    }

    /// A1 回归：并行多个 tool_call 各自成块，互不串号，且都闭合。
    #[test]
    fn parallel_tool_calls_each_get_own_block_and_close() {
        let mut c = StreamConverter::new("deepseek-chat".into());
        let mut out = c.convert_chunk(&chunk(
            json!({"tool_calls": [
                {"index": 0, "id": "call_a", "function": {"name": "f", "arguments": "{}"}},
                {"index": 1, "id": "call_b", "function": {"name": "g", "arguments": "{}"}}
            ]}),
            None,
        ));
        out.extend(c.convert_chunk(&chunk(json!({}), Some("tool_calls"))));

        let evs = events_of(&out);
        let starts: Vec<i64> = evs
            .iter()
            .filter(|(t, _)| t == "content_block_start")
            .map(|(_, i)| *i)
            .collect();
        let stops: Vec<i64> = evs
            .iter()
            .filter(|(t, _)| t == "content_block_stop")
            .map(|(_, i)| *i)
            .collect();
        assert_eq!(starts, vec![0, 1], "两个工具应各开一块，实际 {starts:?}");
        assert_eq!(stops, vec![0, 1], "两个块都要闭合，实际 {stops:?}");
    }

    /// A1 回归：先出文本再出工具 —— 文本块先闭合，工具块另起 index。
    #[test]
    fn text_block_closes_before_tool_block_opens() {
        let mut c = StreamConverter::new("deepseek-chat".into());
        let mut out = c.convert_chunk(&chunk(json!({"content": "我看一下"}), None));
        out.extend(c.convert_chunk(&chunk(
            json!({"tool_calls": [{"index": 0, "id": "c1", "function": {"name": "f", "arguments": "{}"}}]}),
            None,
        )));
        out.extend(c.convert_chunk(&chunk(json!({}), Some("tool_calls"))));

        let seq: Vec<(String, i64)> = events_of(&out)
            .into_iter()
            .filter(|(t, _)| t.starts_with("content_block"))
            .collect();
        assert_eq!(
            seq,
            vec![
                ("content_block_start".into(), 0),
                ("content_block_delta".into(), 0),
                ("content_block_stop".into(), 0),
                ("content_block_start".into(), 1),
                ("content_block_delta".into(), 1),
                ("content_block_stop".into(), 1),
            ],
            "实际序列 {seq:?}"
        );
    }

    /// A2 回归：上游没发 finish_reason 就断了，`finish()` 必须补齐闭合 + 终止事件。
    #[test]
    fn finish_closes_open_blocks_after_truncated_stream() {
        let mut c = StreamConverter::new("deepseek-chat".into());
        // 只发一个带工具调用的 chunk，没有 finish_reason，也没有后续
        let _ = c.convert_chunk(&chunk(
            json!({"tool_calls": [{"index": 0, "id": "c1", "function": {"name": "f", "arguments": "{"}}]}),
            None,
        ));
        assert!(!c.is_terminated(), "尚未终止");

        let tail = c.finish();
        let ty: Vec<String> = events_of(&tail).into_iter().map(|(t, _)| t).collect();
        assert!(
            ty.contains(&"content_block_stop".to_string()),
            "缺闭合：{ty:?}"
        );
        assert!(ty.contains(&"message_delta".to_string()), "缺终止：{ty:?}");
        assert_eq!(
            ty.last().map(String::as_str),
            Some("message_stop"),
            "终止事件必须是最后一个：{ty:?}"
        );
        assert!(c.is_terminated());
        // 幂等
        assert!(c.finish().is_empty());
    }

    /// A2 回归：finish() 补收尾时也不能重复发闭合（open_index 已被取走）。
    #[test]
    fn finish_is_idempotent_and_never_double_closes() {
        let mut c = StreamConverter::new("m".into());
        let _ = c.convert_chunk(&chunk(json!({"content": "hello"}), None));
        let first = c.finish();
        let second = c.finish();
        assert!(second.is_empty(), "二次 finish 必须为空：{second:?}");
        let stops = events_of(&first)
            .into_iter()
            .filter(|(t, _)| t == "content_block_stop")
            .count();
        assert_eq!(stops, 1, "只应闭合一次，实际 {stops}");
    }

    /// A3 回归：`message_stop` 之后到达的 chunk 必须被吞掉，不得再产出任何事件。
    #[test]
    fn chunks_after_termination_are_swallowed() {
        let mut c = StreamConverter::new("m".into());
        let mut out = c.convert_chunk(&chunk(json!({"content": "hi"}), None));
        out.extend(c.convert_chunk(&chunk(json!({}), Some("stop"))));
        assert!(c.is_terminated());

        let after = c.convert_chunk(&chunk(json!({"content": "不该出现"}), None));
        assert!(after.is_empty(), "终止后不得再产出事件：{after:?}");
        let ty: Vec<String> = events_of(&out).into_iter().map(|(t, _)| t).collect();
        assert_eq!(ty.last().map(String::as_str), Some("message_stop"));
    }

    /// close_open_blocks（异常路径用）只闭合、不发终止事件。
    #[test]
    fn close_open_blocks_does_not_emit_terminator() {
        let mut c = StreamConverter::new("m".into());
        let _ = c.convert_chunk(&chunk(json!({"content": "x"}), None));
        let evs = events_of(&c.close_open_blocks());
        assert_eq!(evs, vec![("content_block_stop".to_string(), 0)]);
        assert!(!c.is_terminated(), "异常路径不应声称正常结束");
    }

    /// 一个 delta 都没来就中断：不产出孤立终止事件（否则客户端更困惑）。
    #[test]
    fn finish_before_any_chunk_emits_nothing() {
        let mut c = StreamConverter::new("m".into());
        assert!(c.finish().is_empty());
        assert!(c.is_terminated());
    }

    /// 上游只发 [DONE] 不带 finish_reason 时，finish() 要补终止（否则客户端挂住）
    #[test]
    fn done_without_finish_reason_still_gets_terminated_by_finish() {
        let mut c = StreamConverter::new("m".into());
        let _ = c.convert_chunk(&chunk(json!({"content": "partial"}), None));
        let tail = c.finish();
        let ty: Vec<String> = events_of(&tail).into_iter().map(|(t, _)| t).collect();
        assert_eq!(
            ty,
            vec!["content_block_stop", "message_delta", "message_stop"],
            "实际 {ty:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::{anthropic_to_openai, openai_to_anthropic};
    use serde_json::{json, Value};

    fn anthropic_body(tool_choice: Value) -> Value {
        json!({
            "model": "claude-sonnet-4",
            "max_tokens": 128,
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{
                "name": "read_file",
                "description": "read a file",
                "input_schema": {"type": "object"}
            }],
            "tool_choice": tool_choice
        })
    }

    #[test]
    fn disable_parallel_tool_use_maps_to_parallel_tool_calls_false() {
        let out = anthropic_to_openai(
            &anthropic_body(json!({"type": "auto", "disable_parallel_tool_use": true})),
            "gpt-5",
            None,
        );
        assert_eq!(out.get("parallel_tool_calls"), Some(&json!(false)));
    }

    #[test]
    fn absent_flag_leaves_parallel_tool_calls_untouched() {
        // 未声明时不要主动写这个字段，让上游保持自己的默认行为
        let out = anthropic_to_openai(&anthropic_body(json!({"type": "any"})), "gpt-5", None);
        assert!(
            out.get("parallel_tool_calls").is_none(),
            "未声明 disable_parallel_tool_use 时不应注入 parallel_tool_calls"
        );
        // 顺带回归 tool_choice 映射（本次未改动，但同一段逻辑）
        assert_eq!(out.get("tool_choice"), Some(&json!("required")));
    }

    #[test]
    fn output_config_effort_maps_to_reasoning_effort() {
        // Anthropic 的 `output_config.effort` 在 OpenAI 侧只有 `reasoning_effort` 一个落点；
        // 不映射时 Claude Code `/effort max` 这类意图会静默丢失（抄自 cc-switch d6e05152）。
        let mut body = anthropic_body(json!({"type": "auto"}));
        body["output_config"] = json!({"effort": "max"});
        let out = anthropic_to_openai(&body, "gpt-5.6", None);
        assert_eq!(out.get("reasoning_effort"), Some(&json!("max")));
    }

    #[test]
    fn thinking_budget_maps_to_reasoning_effort_for_grok() {
        let mut body = anthropic_body(json!({"type": "auto"}));
        body["thinking"] = json!({"type": "enabled", "budget_tokens": 8000});
        let out = anthropic_to_openai(&body, "grok-4.7", None);
        assert_eq!(out.get("reasoning_effort"), Some(&json!("medium")));
    }

    #[test]
    fn effort_is_not_injected_for_unsupported_model() {
        let mut body = anthropic_body(json!({"type": "auto"}));
        body["output_config"] = json!({"effort": "high"});
        let out = anthropic_to_openai(&body, "gpt-4o", None);
        assert!(out.get("reasoning_effort").is_none(), "{out}");
    }

    #[test]
    fn no_effort_intent_leaves_reasoning_effort_to_the_optimizer() {
        // 入站没有表达 effort 时不要臆造，交给 optimizers::optimize_thinking_openai 按家族兜底
        let out = anthropic_to_openai(&anthropic_body(json!({"type": "auto"})), "gpt-5.4", None);
        assert!(out.get("reasoning_effort").is_none(), "{out}");
    }

    /// OpenAI 请求体（可带 tool_choice / parallel_tool_calls），用于反向转换测试。
    fn openai_body(tool_choice: Option<Value>, parallel: Option<bool>) -> Value {
        let mut body = json!({
            "model": "gpt-5",
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{
                "type": "function",
                "function": { "name": "read_file", "parameters": {"type": "object"} }
            }]
        });
        if let Some(tc) = tool_choice {
            body["tool_choice"] = tc;
        }
        if let Some(p) = parallel {
            body["parallel_tool_calls"] = json!(p);
        }
        body
    }

    /// 反向缺口：此前 openai→anthropic 完全不转换 tool_choice，「必须调用工具」会被降级成自由发挥。
    #[test]
    fn openai_tool_choice_maps_to_anthropic() {
        let required = openai_to_anthropic(
            &openai_body(Some(json!("required")), None),
            "claude-x",
            None,
        );
        assert_eq!(required.get("tool_choice"), Some(&json!({"type": "any"})));

        let forced = openai_to_anthropic(
            &openai_body(
                Some(json!({"type": "function", "function": {"name": "read_file"}})),
                None,
            ),
            "claude-x",
            None,
        );
        assert_eq!(
            forced.get("tool_choice"),
            Some(&json!({"type": "tool", "name": "read_file"}))
        );

        let auto = openai_to_anthropic(&openai_body(Some(json!("auto")), None), "claude-x", None);
        assert_eq!(auto.get("tool_choice"), Some(&json!({"type": "auto"})));

        // 没给 tool_choice → 不写该字段（保持上游默认行为）
        let absent = openai_to_anthropic(&openai_body(None, None), "claude-x", None);
        assert!(absent.get("tool_choice").is_none());
    }

    #[test]
    fn openai_parallel_tool_calls_false_maps_to_disable_parallel() {
        // 带 tool_choice：约束挂在同一个对象上
        let out = openai_to_anthropic(
            &openai_body(Some(json!("auto")), Some(false)),
            "claude-x",
            None,
        );
        assert_eq!(
            out.get("tool_choice"),
            Some(&json!({"type": "auto", "disable_parallel_tool_use": true}))
        );
        // 不带 tool_choice：补一个 auto 承载该约束
        let out2 = openai_to_anthropic(&openai_body(None, Some(false)), "claude-x", None);
        assert_eq!(
            out2.get("tool_choice"),
            Some(&json!({"type": "auto", "disable_parallel_tool_use": true}))
        );
        // true 或未声明 → 不写（保持上游默认）
        let out3 = openai_to_anthropic(&openai_body(None, Some(true)), "claude-x", None);
        assert!(out3.get("tool_choice").is_none());
    }

    #[test]
    fn tool_choice_is_not_written_without_tools() {
        // Anthropic 不接受没有 tools 的 tool_choice
        let mut body = openai_body(Some(json!("required")), None);
        body.as_object_mut().unwrap().remove("tools");
        let out = openai_to_anthropic(&body, "claude-x", None);
        assert!(out.get("tool_choice").is_none());
    }
}
