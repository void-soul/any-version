//! `collapse_system_messages_to_head` 的回归测试。
//! 独立成文件是因为 `convert.rs` 在「大文件只允许收缩」的白名单里。
use super::{collapse_system_messages_to_head, normalize_openai_message_roles};
use serde_json::json;

/// 插件把 notice 插在 `assistant(tool_calls)` 与 `tool` 结果之间时，
/// 上游会回 `11148 tool calls and tool results do not match`（参考侧专门打了个
/// `tool-result-pairing` 补丁来搬 notice）。
///
/// 我们没抄那个补丁：夹在工具循环里的 system 仍会并进头部，notice 自然离开 tool 组，
/// 配对恢复连续。这条用例钉住该规避效果 —— 将来谁改 collapse 都必须保证 tool 结果仍紧跟 assistant。
#[test]
fn interleaved_notice_does_not_break_tool_pairing() {
    let mut body = json!({"messages": [
        {"role": "assistant", "content": null, "tool_calls": [
            {"id": "c1", "type": "function", "function": {"name": "f", "arguments": "{}"}}
        ]},
        {"role": "developer", "content": "URI guard"},
        {"role": "tool", "tool_call_id": "c1", "content": "ok"},
        {"role": "user", "content": "next"}
    ]});
    normalize_openai_message_roles(&mut body);
    collapse_system_messages_to_head(&mut body);
    let msgs = body["messages"].as_array().unwrap();

    assert_eq!(msgs[0]["role"], json!("system"));
    assert!(msgs[0]["content"].as_str().unwrap().contains("URI guard"));
    assert_eq!(msgs[1]["role"], json!("assistant"));
    assert_eq!(msgs[2]["role"], json!("tool"));
    assert_eq!(msgs[2]["tool_call_id"], json!("c1"));
    assert_eq!(msgs[3]["role"], json!("user"));
    assert_eq!(msgs.len(), 4);
}

/// 多条散落的 system（字符串 content）收拢到头部合成一条。
#[test]
fn folds_multiple_system_messages_to_head() {
    let mut body = json!({"messages": [
        {"role": "user", "content": "q1"},
        {"role": "system", "content": "sys-a"},
        {"role": "assistant", "content": "a"},
        {"role": "system", "content": "sys-b"}
    ]});
    collapse_system_messages_to_head(&mut body);
    let msgs = body["messages"].as_array().unwrap();
    assert_eq!(msgs[0]["role"], json!("system"));
    assert_eq!(msgs[0]["content"], json!("sys-a\n\nsys-b"));
    let roles: Vec<&str> = msgs.iter().map(|m| m["role"].as_str().unwrap()).collect();
    assert_eq!(roles, vec!["system", "user", "assistant"]);
}

/// 空内容的 system 直接丢弃，不合成空 system。
#[test]
fn drops_empty_system_messages() {
    let mut body = json!({"messages": [
        {"role": "system", "content": ""},
        {"role": "system", "content": "   "},
        {"role": "user", "content": "q"}
    ]});
    collapse_system_messages_to_head(&mut body);
    let msgs = body["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1, "只剩 user 消息");
    assert_eq!(msgs[0]["role"], json!("user"));
}

/// content 为 parts 数组的 system 取各 part 的 text 拼接。
#[test]
fn reads_parts_array_content() {
    let mut body = json!({"messages": [
        {"role": "system", "content": [{"type": "text", "text": "a"}, {"type": "text", "text": "b"}]},
        {"role": "user", "content": "q"}
    ]});
    collapse_system_messages_to_head(&mut body);
    assert_eq!(body["messages"][0]["content"], json!("a\n\nb"));
}

/// 没有 system 时零改动。
#[test]
fn untouched_without_system() {
    let body = json!({"messages": [
        {"role": "user", "content": "q"},
        {"role": "assistant", "content": "a"}
    ]});
    let mut b = body.clone();
    collapse_system_messages_to_head(&mut b);
    assert_eq!(b, body);
}

/// cc-switch abf441fe：Codex 在新回合开头注入的 developer（归一化后是 system）
/// 若被并进头部，会改写已经发过的前缀 → 上游 prompt cache 从第一条断开、整段重算。
/// 判据：第二轮请求的历史部分必须与第一轮**逐字节相同**。
#[test]
fn turn_start_system_stays_in_place_to_preserve_prefix() {
    let build = |extra: Vec<serde_json::Value>| {
        let mut messages = vec![
            json!({"role": "system", "content": "You are Codex."}),
            json!({"role": "user", "content": "list files"}),
            json!({"role": "assistant", "content": "Done."}),
        ];
        messages.extend(extra);
        let mut body = json!({"messages": messages});
        normalize_openai_message_roles(&mut body);
        collapse_system_messages_to_head(&mut body);
        body["messages"].as_array().unwrap().clone()
    };
    let first = build(vec![]);
    let second = build(vec![
        json!({"role": "developer", "content": "Permissions: workspace-write"}),
        json!({"role": "user", "content": "edit a.rs"}),
    ]);
    assert_eq!(
        &second[..first.len()],
        &first[..],
        "第二轮历史必须是第一轮的精确前缀"
    );
    assert_eq!(second[first.len()]["role"], json!("user"));
    assert_eq!(
        second[first.len()]["content"],
        json!("Permissions: workspace-write")
    );
    assert_eq!(second[first.len() + 1]["role"], json!("user"));
}

/// 反面：夹在工具调用循环里的 system 后面是 tool，仍要并进头部 ——
/// 这是 D7（11148 工具配对）依赖的效果，不能被上一条修复破坏。
#[test]
fn mid_tool_loop_system_is_still_hoisted() {
    let mut body = json!({"messages": [
        {"role": "system", "content": "S1"},
        {"role": "user", "content": "U1"},
        {"role": "assistant", "content": null, "tool_calls": [
            {"id": "c0", "type": "function", "function": {"name": "f", "arguments": "{}"}}
        ]},
        {"role": "system", "content": "Notice"},
        {"role": "tool", "tool_call_id": "c0", "content": "ok"}
    ]});
    collapse_system_messages_to_head(&mut body);
    let msgs = body["messages"].as_array().unwrap();
    assert_eq!(msgs[0]["content"], json!("S1\n\nNotice"));
    let roles: Vec<&str> = msgs.iter().map(|m| m["role"].as_str().unwrap()).collect();
    assert_eq!(roles, vec!["system", "user", "assistant", "tool"]);
}
