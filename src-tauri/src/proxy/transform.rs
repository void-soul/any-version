//! Anthropic Messages API ↔ OpenAI Chat Completions API 格式转换
//!
//! 参考 cc-switch 的 transform.rs 实现，精简版。

use serde_json::{json, Value};
use std::collections::HashMap;

/// 模型别名映射配置
#[derive(Clone, Debug, Default)]
pub struct ModelAliases {
    /// 默认模型（当无匹配时使用）
    pub default_model: Option<String>,
    /// 角色关键词 → 实际模型 ID 的映射
    /// 例如: "sonnet" → "deepseek-v4-pro", "opus" → "claude-opus-4-8"
    pub role_map: HashMap<String, String>,
}

/// 将 Claude 模型名映射到实际上游模型名。
///
/// 映射逻辑：
/// 1. 剥离 `[1M]`/`[1m]` 后缀
/// 2. 优先：role_map 中存在与官方全名完全一致的 key（精确匹配，最高优先级，
///    允许 provider 直接以官方完整模型名为键配置映射）
/// 3. 内置解析表：官方模型系列/别名 → role（sonnet/opus/haiku/fable 及 best 等），
///    再查 role_map 中对应 role 的映射
/// 4. 无角色匹配时使用 default_model
/// 5. 无配置时原样返回
pub fn map_model_name(request_model: &str, aliases: &ModelAliases) -> String {
    // 1. 剥离 [1M]/[1m] 后缀
    let cleaned = request_model
        .replace("[1M]", "")
        .replace("[1m]", "")
        .trim()
        .to_string();
    let lower = cleaned.to_lowercase();

    // 2. 优先：role_map 中存在与官方全名完全一致的 key（精确匹配）
    //    支持 provider 以官方完整模型名（如 claude-sonnet-4-20250514）为键配置映射
    if let Some(mapped) = aliases
        .role_map
        .get(&lower)
        .or_else(|| aliases.role_map.get(&cleaned))
    {
        return mapped.clone();
    }

    // 3. 内置解析表：官方模型系列/别名 → role，再查 role_map 中对应 role
    if let Some(role) = resolve_role(&cleaned) {
        if let Some(mapped) = aliases.role_map.get(role) {
            return mapped.clone();
        }
    }

    // 4. 默认模型
    if let Some(ref default) = aliases.default_model {
        return default.clone();
    }

    // 5. 原样返回
    cleaned
}

/// Claude Code 内部别名映射表（可从外部配置文件覆盖）
/// key = Claude Code 发送的别名，value = 归并后的标准 role
const CLAUDE_INTERNAL_ALIASES: &[(&str, &str)] = &[
    ("best", "opus"),     // Claude Code 的 "best" 解析为最强模型
    ("opusplan", "opus"), // "plan mode" 专用 opus 别名
];

/// 内置「官方模型名/别名 → role」解析表。
///
/// 覆盖 claude-sonnet-4-*、claude-opus-4-*、claude-haiku-4-*、claude-fable-5-*
/// 各版本，以及 Claude Code 内部别名（best/opusplan 等），统一归并为
/// `sonnet` / `opus` / `haiku` / `fable` 四个标准 role，再交由 `role_map` 映射。
fn resolve_role(model: &str) -> Option<&'static str> {
    let lower = model.to_lowercase();

    // Claude Code 内部别名（不含 sonnet/opus/haiku/fable 关键词）
    for (alias, role) in CLAUDE_INTERNAL_ALIASES {
        if lower == *alias {
            return Some(role);
        }
    }

    // 官方模型系列（关键词归并，大小写不敏感）
    if lower.contains("sonnet") {
        return Some("sonnet");
    }
    if lower.contains("opus") {
        return Some("opus");
    }
    if lower.contains("haiku") {
        return Some("haiku");
    }
    if lower.contains("fable") {
        return Some("fable");
    }

    None
}

/// 需要拦截的工具名列表（不转发到上游的工具定义）
const BLOCKED_TOOL_NAMES: &[&str] = &["BatchTool"];

/// 检查工具名是否在拦截列表中
fn is_blocked_tool_name(name: &str) -> bool {
    BLOCKED_TOOL_NAMES.contains(&name)
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  Anthropic → OpenAI
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 将 Anthropic Messages API 请求体转换为 OpenAI Chat Completions 请求体
pub fn anthropic_to_openai(
    body: &Value,
    target_model: &str,
    aliases: Option<&ModelAliases>,
) -> Value {
    let mut messages = Vec::new();

    // 解析请求中的模型名，应用别名映射
    let request_model = body
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or(target_model);
    let resolved_model = if let Some(a) = aliases {
        map_model_name(request_model, a)
    } else {
        target_model.to_string()
    };

    // 1. system → system message
    if let Some(system) = body.get("system") {
        let text = match system {
            Value::String(s) => s.clone(),
            Value::Array(arr) => arr
                .iter()
                .filter_map(|part| {
                    if part.get("type").and_then(|v| v.as_str()) == Some("text") {
                        part.get("text").and_then(|v| v.as_str()).map(String::from)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        };
        if !text.is_empty() {
            messages.push(json!({"role": "system", "content": text}));
        }
    }

    // 2. messages → chat messages
    if let Some(msgs) = body.get("messages").and_then(|v| v.as_array()) {
        for msg in msgs {
            convert_anthropic_message(msg, &mut messages);
        }
    }

    // 3. 构建 OpenAI 请求
    let mut openai = json!({
        "model": resolved_model.clone(),
        "messages": messages,
    });

    // 4. 参数映射
    if let Some(max_tokens) = body.get("max_tokens").and_then(|v| v.as_u64()) {
        openai["max_completion_tokens"] = json!(max_tokens);
    }
    if let Some(temp) = body.get("temperature").and_then(|v| v.as_f64()) {
        openai["temperature"] = json!(temp);
    }
    if let Some(top_p) = body.get("top_p").and_then(|v| v.as_f64()) {
        openai["top_p"] = json!(top_p);
    }
    if let Some(stream) = body.get("stream").and_then(|v| v.as_bool()) {
        openai["stream"] = json!(stream);
        if stream {
            openai["stream_options"] = json!({"include_usage": true});
        }
    }
    if let Some(stop) = body.get("stop_sequences") {
        openai["stop"] = stop.clone();
    }

    // 5. tools 转换
    if let Some(tools) = body.get("tools").and_then(|v| v.as_array()) {
        let openai_tools: Vec<Value> = tools
            .iter()
            .filter(|t| {
                t.get("name").and_then(|v| v.as_str()) != Some("BatchTool")
                    && !is_blocked_tool_name(t.get("name").and_then(|v| v.as_str()).unwrap_or(""))
            })
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.get("name").unwrap_or(&json!("")),
                        "description": t.get("description").unwrap_or(&json!("")),
                        "parameters": super::normalize_function_parameters(t.get("input_schema"))
                    }
                })
            })
            .collect();
        if !openai_tools.is_empty() {
            openai["tools"] = json!(openai_tools);
        }
    }

    // 6. tool_choice 转换
    if let Some(tc) = body.get("tool_choice") {
        let openai_tc = match tc.get("type").and_then(|v| v.as_str()) {
            Some("auto") => json!("auto"),
            Some("any") => json!("required"),
            Some("tool") => {
                if let Some(name) = tc.get("name") {
                    json!({"type": "function", "function": {"name": name}})
                } else {
                    json!("auto")
                }
            }
            _ => json!("auto"),
        };
        openai["tool_choice"] = openai_tc;
    }

    // 7. 并行工具调用：Anthropic 用 `tool_choice.disable_parallel_tool_use` 表达「不要并行」，
    //    OpenAI 是顶层 `parallel_tool_calls`。不映射时上游可能一次回多个 tool_use，
    //    客户端按单工具解析就会丢掉后续调用（参考 cc-switch「enable parallel tool calls」）。
    if body
        .get("tool_choice")
        .and_then(|tc| tc.get("disable_parallel_tool_use"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        openai["parallel_tool_calls"] = json!(false);
    } else if let Some(parallel) = body.get("parallel_tool_calls").and_then(|v| v.as_bool()) {
        // 少数客户端直接把 OpenAI 风格开关塞进 Anthropic 请求体：出现时以它为准
        openai["parallel_tool_calls"] = json!(parallel);
    }

    // 8. reasoning effort：Anthropic 的 `thinking` / `output_config.effort` 在 OpenAI 侧
    //    没有等价块，只有顶层 `reasoning_effort` 一个落点。不映射时用户的 effort 意图
    //    （含 Claude Code `/effort max`）会静默丢失，gpt-5+/grok-4.x 上表现为「档位永远默认」
    //    （抄自 cc-switch 8e478b2b / d6e05152）。
    //    仅当入站真的表达了 effort、且目标模型支持该参数时写入；
    //    未表达时留给 optimizers::optimize_thinking_openai 按家族兜底。
    if super::optimizers::supports_reasoning_effort(&resolved_model) {
        if let Some(effort) = super::optimizers::resolve_reasoning_effort(&resolved_model, body) {
            openai["reasoning_effort"] = json!(effort);
        }
    }

    openai
}

/// 转换单条 Anthropic 消息为 OpenAI 格式（可能产生多条消息）
fn convert_anthropic_message(msg: &Value, output: &mut Vec<Value>) {
    let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("user");
    let content = match msg.get("content") {
        Some(Value::String(s)) => {
            output.push(json!({"role": role, "content": s}));
            return;
        }
        Some(Value::Array(arr)) => arr,
        _ => return,
    };

    match role {
        "assistant" => {
            let mut text_parts = Vec::new();
            let mut tool_calls = Vec::new();
            let mut reasoning_parts = Vec::new();

            for part in content {
                match part.get("type").and_then(|v| v.as_str()) {
                    Some("text") => {
                        if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                            text_parts.push(text.to_string());
                        }
                    }
                    Some("thinking") => {
                        if let Some(thinking) = part.get("thinking").and_then(|v| v.as_str()) {
                            reasoning_parts.push(thinking.to_string());
                        }
                    }
                    Some("tool_use") => {
                        let id = part.get("id").cloned().unwrap_or(json!(""));
                        let name = part.get("name").cloned().unwrap_or(json!(""));
                        let input = part.get("input").cloned().unwrap_or(json!({}));
                        let args_str =
                            serde_json::to_string(&input).unwrap_or_else(|_| "{}".to_string());
                        tool_calls.push(json!({
                            "id": id,
                            "type": "function",
                            "function": {
                                "name": name,
                                "arguments": args_str
                            }
                        }));
                    }
                    _ => {}
                }
            }

            let mut assistant_msg = json!({"role": "assistant"});
            if !text_parts.is_empty() {
                assistant_msg["content"] = json!(text_parts.join("\n"));
            }
            if !reasoning_parts.is_empty() {
                assistant_msg["reasoning_content"] = json!(reasoning_parts.join("\n"));
            }
            if !tool_calls.is_empty() {
                assistant_msg["tool_calls"] = json!(tool_calls);
            }
            output.push(assistant_msg);
        }
        "user" => {
            // 收集所有 content parts
            let mut openai_content = Vec::new();
            for part in content {
                match part.get("type").and_then(|v| v.as_str()) {
                    Some("text") => {
                        if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                            openai_content.push(json!({"type": "text", "text": text}));
                        }
                    }
                    Some("image") => {
                        if let (Some(media_type), Some(data)) = (
                            part.get("media_type").and_then(|v| v.as_str()),
                            part.get("data").and_then(|v| v.as_str()),
                        ) {
                            openai_content.push(json!({
                                "type": "image_url",
                                "image_url": {"url": format!("data:{};base64,{}", media_type, data)}
                            }));
                        }
                    }
                    Some("tool_result") => {
                        // tool_result 需要作为独立的 tool 消息
                        let tool_use_id = part.get("tool_use_id").cloned().unwrap_or(json!(""));
                        let result_content = match part.get("content") {
                            Some(Value::String(s)) => s.clone(),
                            Some(Value::Array(arr)) => arr
                                .iter()
                                .filter_map(|p| p.get("text").and_then(|v| v.as_str()))
                                .collect::<Vec<_>>()
                                .join("\n"),
                            _ => String::new(),
                        };
                        output.push(json!({
                            "role": "tool",
                            "tool_call_id": tool_use_id,
                            "content": result_content
                        }));
                        continue; // 不作为 user 消息的一部分
                    }
                    _ => {}
                }
            }

            if !openai_content.is_empty() {
                // 如果只有一个 text part，简化为纯字符串
                if openai_content.len() == 1
                    && openai_content[0].get("type").and_then(|v| v.as_str()) == Some("text")
                {
                    output.push(json!({
                        "role": "user",
                        "content": openai_content[0].get("text").unwrap_or(&json!(""))
                    }));
                } else {
                    output.push(json!({"role": "user", "content": openai_content}));
                }
            }
        }
        _ => {}
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  OpenAI → Anthropic
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 将 OpenAI Chat Completions 响应转换为 Anthropic Messages API 响应
pub fn openai_response_to_anthropic(openai_resp: &Value, request_model: &str) -> Value {
    let choice = openai_resp
        .get("choices")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first());

    let message = choice.and_then(|c| c.get("message"));
    let finish_reason = choice
        .and_then(|c| c.get("finish_reason").and_then(|v| v.as_str()))
        .unwrap_or("stop");

    let mut content = Vec::new();

    // reasoning_content → thinking block
    if let Some(reasoning) =
        message.and_then(|m| m.get("reasoning_content").and_then(|v| v.as_str()))
    {
        if !reasoning.is_empty() {
            content.push(json!({"type": "thinking", "thinking": reasoning}));
        }
    }

    // content → text block（顺带剥离内联 think 标签成 thinking block）
    if let Some(text) = message.and_then(|m| m.get("content").and_then(|v| v.as_str())) {
        if !text.is_empty() {
            if let Some((thinking, answer)) =
                crate::proxy::inline_think::split_leading_think_block(text)
            {
                if !thinking.is_empty() {
                    content.push(json!({"type": "thinking", "thinking": thinking}));
                }
                if !answer.is_empty() {
                    content.push(json!({"type": "text", "text": answer}));
                }
            } else {
                content.push(json!({"type": "text", "text": text}));
            }
        }
    }

    // tool_calls → tool_use blocks
    if let Some(tool_calls) = message.and_then(|m| m.get("tool_calls").and_then(|v| v.as_array())) {
        for tc in tool_calls {
            let id = tc.get("id").cloned().unwrap_or(json!(""));
            let name = tc
                .get("function")
                .and_then(|f| f.get("name"))
                .cloned()
                .unwrap_or(json!(""));
            let args_str = tc
                .get("function")
                .and_then(|f| f.get("arguments").and_then(|v| v.as_str()))
                .unwrap_or("{}");
            let args: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
            content.push(json!({
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": args
            }));
        }
    }

    // stop_reason 映射
    let stop_reason = match finish_reason {
        "stop" => "end_turn",
        "length" => "max_tokens",
        "tool_calls" | "function_call" => "tool_use",
        _ => "end_turn",
    };

    // usage 映射
    let usage = openai_resp.get("usage");
    let input_tokens = usage
        .and_then(|u| u.get("prompt_tokens").and_then(|v| v.as_u64()))
        .unwrap_or(0);
    let output_tokens = usage
        .and_then(|u| u.get("completion_tokens").and_then(|v| v.as_u64()))
        .unwrap_or(0);

    json!({
        "id": format!("msg_{}", openai_resp.get("id").and_then(|v| v.as_str()).unwrap_or("0")),
        "type": "message",
        "role": "assistant",
        "content": content,
        "model": request_model,
        "stop_reason": stop_reason,
        "stop_sequence": null,
        "usage": {
            "input_tokens": input_tokens,
            "output_tokens": output_tokens
        }
    })
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  OpenAI → Anthropic Request & Response Translation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 将 OpenAI Chat Completions 请求体转换为 Anthropic Messages 请求体
pub fn openai_to_anthropic(
    body: &Value,
    target_model: &str,
    aliases: Option<&ModelAliases>,
) -> Value {
    let mut messages = Vec::new();
    let mut system_text = String::new();

    let request_model = body
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or(target_model);
    let resolved_model = if let Some(a) = aliases {
        map_model_name(request_model, a)
    } else {
        target_model.to_string()
    };

    if let Some(msgs) = body.get("messages").and_then(|v| v.as_array()) {
        for msg in msgs {
            let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("user");
            let content = msg.get("content");
            if role == "system" {
                if let Some(c) = content {
                    let text = match c {
                        Value::String(s) => s.clone(),
                        Value::Array(arr) => arr
                            .iter()
                            .filter_map(|part| {
                                if part.get("type").and_then(|v| v.as_str()) == Some("text") {
                                    part.get("text").and_then(|v| v.as_str()).map(String::from)
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n"),
                        _ => String::new(),
                    };
                    if !system_text.is_empty() {
                        system_text.push_str("\n");
                    }
                    system_text.push_str(&text);
                }
            } else {
                let anthropic_role = if role == "assistant" {
                    "assistant"
                } else {
                    "user"
                };
                if let Some(c) = content {
                    let converted_content = match c {
                        Value::String(s) => json!(s),
                        Value::Array(arr) => {
                            let parts: Vec<Value> = arr
                                .iter()
                                .map(|part| {
                                    let part_type =
                                        part.get("type").and_then(|v| v.as_str()).unwrap_or("text");
                                    if part_type == "image_url" {
                                        if let Some(url) = part
                                            .get("image_url")
                                            .and_then(|v| v.get("url").and_then(|u| u.as_str()))
                                        {
                                            if url.starts_with("data:") {
                                                if let Some(comma_idx) = url.find(',') {
                                                    let media_type = url[5..comma_idx]
                                                        .split(';')
                                                        .next()
                                                        .unwrap_or("image/jpeg");
                                                    let base64_data = &url[comma_idx + 1..];
                                                    return json!({
                                                        "type": "image",
                                                        "source": {
                                                            "type": "base64",
                                                            "media_type": media_type,
                                                            "data": base64_data
                                                        }
                                                    });
                                                }
                                            }
                                        }
                                    }
                                    json!({
                                        "type": "text",
                                        "text": part.get("text").unwrap_or(&json!(""))
                                    })
                                })
                                .collect();
                            json!(parts)
                        }
                        _ => json!(""),
                    };
                    messages.push(json!({
                        "role": anthropic_role,
                        "content": converted_content
                    }));
                }
            }
        }
    }

    let mut anthropic = json!({
        "model": resolved_model,
        "messages": messages,
    });

    if !system_text.is_empty() {
        anthropic["system"] = json!(system_text);
    }

    let max_tokens = body
        .get("max_completion_tokens")
        .or_else(|| body.get("max_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(4096);
    anthropic["max_tokens"] = json!(max_tokens);

    if let Some(temp) = body.get("temperature").and_then(|v| v.as_f64()) {
        anthropic["temperature"] = json!(temp);
    }
    if let Some(top_p) = body.get("top_p").and_then(|v| v.as_f64()) {
        anthropic["top_p"] = json!(top_p);
    }
    if let Some(stream) = body.get("stream").and_then(|v| v.as_bool()) {
        anthropic["stream"] = json!(stream);
    }

    // tools 转换
    if let Some(tools) = body.get("tools").and_then(|v| v.as_array()) {
        let anthropic_tools: Vec<Value> = tools
            .iter()
            .filter_map(|t| {
                if t.get("type").and_then(|v| v.as_str()) == Some("function") {
                    if let Some(func) = t.get("function") {
                        return Some(json!({
                            "name": func.get("name").unwrap_or(&json!("")),
                            "description": func.get("description").unwrap_or(&json!("")),
                            "input_schema": super::normalize_function_parameters(func.get("parameters"))
                        }));
                    }
                }
                None
            })
            .collect();
        if !anthropic_tools.is_empty() {
            anthropic["tools"] = json!(anthropic_tools);
        }
    }

    // tool_choice 转换：此前**完全没有映射** —— 客户端说「必须调用工具（required）」
    // 会被静默降级成自由发挥，输出可能压根不调工具。
    // Anthropic 不接受没有 tools 的 tool_choice，因此只在有 tools 时写入。
    if anthropic.get("tools").is_some() {
        let parallel = body.get("parallel_tool_calls").and_then(|v| v.as_bool());
        let mut anthropic_tc = match body.get("tool_choice") {
            Some(Value::String(kind)) => match kind.as_str() {
                "required" => json!({"type": "any"}),
                "none" => json!({"type": "none"}),
                _ => json!({"type": "auto"}),
            },
            Some(Value::Object(obj)) => {
                if obj.get("type").and_then(|v| v.as_str()) == Some("function") {
                    let name = obj
                        .get("function")
                        .and_then(|f| f.get("name"))
                        .cloned()
                        .unwrap_or_else(|| json!(""));
                    json!({"type": "tool", "name": name})
                } else {
                    json!({"type": "auto"})
                }
            }
            _ => {
                // 没给（或给了 null）tool_choice：只有明确要求「不要并行」时才补一个承载对象，
                // 否则不写该字段，保持上游默认行为。
                if parallel == Some(false) {
                    json!({"type": "auto"})
                } else {
                    return anthropic;
                }
            }
        };
        // 并行开关：OpenAI 顶层 `parallel_tool_calls: false` → Anthropic 的
        // `tool_choice.disable_parallel_tool_use: true`（cc-switch 的
        //「enable parallel tool calls / preserve parallel tool history」同域问题）。
        if parallel == Some(false) {
            anthropic_tc["disable_parallel_tool_use"] = json!(true);
        }
        anthropic["tool_choice"] = anthropic_tc;
    }

    anthropic
}

/// 将 Anthropic Messages 响应转换为 OpenAI Chat Completions 响应
pub fn anthropic_response_to_openai(anthropic_resp: &Value, request_model: &str) -> Value {
    let content = anthropic_resp.get("content").and_then(|v| v.as_array());
    let stop_reason = anthropic_resp
        .get("stop_reason")
        .and_then(|v| v.as_str())
        .unwrap_or("end_turn");

    let finish_reason = match stop_reason {
        "end_turn" => "stop",
        "max_tokens" => "length",
        "tool_use" => "tool_calls",
        _ => "stop",
    };

    let mut message_content = String::new();
    let mut tool_calls = Vec::new();

    if let Some(parts) = content {
        for part in parts {
            let part_type = part.get("type").and_then(|v| v.as_str()).unwrap_or("text");
            if part_type == "text" {
                if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                    message_content.push_str(text);
                }
            } else if part_type == "tool_use" {
                let id = part.get("id").cloned().unwrap_or(json!(""));
                let name = part.get("name").cloned().unwrap_or(json!(""));
                let input = part.get("input").cloned().unwrap_or(json!({}));
                let args_str = serde_json::to_string(&input).unwrap_or_else(|_| "{}".to_string());
                tool_calls.push(json!({
                    "id": id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": args_str
                    }
                }));
            }
        }
    }

    let mut message = json!({
        "role": "assistant"
    });
    if !message_content.is_empty() {
        message["content"] = json!(message_content);
    } else {
        message["content"] = Value::Null;
    }
    if !tool_calls.is_empty() {
        message["tool_calls"] = json!(tool_calls);
    }

    let usage = anthropic_resp.get("usage");
    let input_tokens = usage
        .and_then(|u| u.get("input_tokens").and_then(|v| v.as_u64()))
        .unwrap_or(0);
    let output_tokens = usage
        .and_then(|u| u.get("output_tokens").and_then(|v| v.as_u64()))
        .unwrap_or(0);

    json!({
        "id": format!("chatcmpl-{}", anthropic_resp.get("id").and_then(|v| v.as_str()).unwrap_or("0")),
        "object": "chat.completion",
        "created": chrono::Utc::now().timestamp(),
        "model": request_model,
        "choices": [
            {
                "index": 0,
                "message": message,
                "logprobs": null,
                "finish_reason": finish_reason
            }
        ],
        "usage": {
            "prompt_tokens": input_tokens,
            "completion_tokens": output_tokens,
            "total_tokens": input_tokens + output_tokens
        }
    })
}
mod stream;
pub use stream::{AnthropicToOpenaiStreamConverter, StreamConverter};
#[cfg(test)]
mod tests;
