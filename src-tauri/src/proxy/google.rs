//! Google Gemini（Generative Language API）协议转换
//!
//! 提供：
//! - 请求转换：anthropic→google / openai→google / google→anthropic / google→openai
//! - 响应转换：google→anthropic / google→openai / openai→google / anthropic→google
//! - 流式转换：上游 openai/anthropic 的 SSE chunk → Google SSE（`GoogleStreamConverter`）
//!
//! 依据：docs/tool-config/gemini-cli/configuration.md 字段名 + Google Generative Language API。
//! 端点：POST {base}/v1beta/models/{model}:generateContent（非流）
//!       POST {base}/v1beta/models/{model}:streamGenerateContent?alt=sse（流）

use serde_json::{json, Value};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  请求转换：Anthropic / OpenAI → Google
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn thinking_config_from_anthropic(body: &Value) -> Option<Value> {
    let thinking = body.get("thinking")?;
    let typ = thinking.get("type").and_then(|v| v.as_str())?;
    Some(if typ == "adaptive" {
        json!({"thinkingBudget": 0, "includeThoughts": true})
    } else {
        let budget = t_get_budget(thinking);
        json!({"thinkingBudget": budget, "includeThoughts": true})
    })
}

fn t_get_budget(t: &Value) -> u64 {
    t.get("budget_tokens").and_then(|v| v.as_u64()).unwrap_or(0)
}

fn tool_config_from_anthropic(tc: &Value) -> Value {
    match tc.get("type").and_then(|v| v.as_str()) {
        Some("any") => json!({"functionCallingConfig": {"mode": "ANY"}}),
        Some("tool") => {
            let name = tc.get("name").and_then(|v| v.as_str()).unwrap_or("");
            json!({"functionCallingConfig": {"mode": "ANY", "allowedFunctionNames": [name]}})
        }
        _ => json!({"functionCallingConfig": {"mode": "AUTO"}}),
    }
}

fn tool_config_from_openai(tc: &Value) -> Value {
    match tc.as_str() {
        Some("required") => json!({"functionCallingConfig": {"mode": "ANY"}}),
        Some("none") => json!({"functionCallingConfig": {"mode": "NONE"}}),
        _ => {
            if let Some(obj) = tc.as_object() {
                if obj.get("type").and_then(|v| v.as_str()) == Some("function") {
                    let name = obj.get("function").and_then(|f| f.get("name")).and_then(|v| v.as_str()).unwrap_or("");
                    return json!({"functionCallingConfig": {"mode": "ANY", "allowedFunctionNames": [name]}});
                }
            }
            json!({"functionCallingConfig": {"mode": "AUTO"}})
        }
    }
}

fn function_declarations_from_anthropic(tools: &Value) -> Vec<Value> {
    tools.as_array().map(|arr| {
        arr.iter().filter_map(|t| {
            Some(json!({
                "name": t.get("name").cloned().unwrap_or(json!("")),
                "description": t.get("description").cloned().unwrap_or(json!("")),
                "parameters": t.get("input_schema").cloned().unwrap_or(json!({}))
            }))
        }).collect()
    }).unwrap_or_default()
}

fn function_declarations_from_openai(tools: &Value) -> Vec<Value> {
    tools.as_array().map(|arr| {
        arr.iter().filter_map(|t| {
            t.get("function").map(|f| json!({
                "name": f.get("name").cloned().unwrap_or(json!("")),
                "description": f.get("description").cloned().unwrap_or(json!("")),
                "parameters": f.get("parameters").cloned().unwrap_or(json!({}))
            }))
        }).collect()
    }).unwrap_or_default()
}

/// Anthropic Messages 请求 → Google generateContent 请求
pub fn anthropic_to_google(body: &Value, model: &str) -> Value {
    let mut contents = Vec::new();
    let mut system_text = String::new();

    if let Some(system) = body.get("system") {
        match system {
            Value::String(s) => system_text.push_str(s),
            Value::Array(arr) => {
                for part in arr {
                    if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                        system_text.push_str(t);
                    }
                }
            }
            _ => {}
        }
    }

    if let Some(msgs) = body.get("messages").and_then(|v| v.as_array()) {
        for msg in msgs {
            let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("user");
            let grole = if role == "assistant" { "model" } else { "user" };
            if let Some(content) = msg.get("content") {
                match content {
                    Value::String(s) => {
                        contents.push(json!({"role": grole, "parts": [{"text": s}]}));
                    }
                    Value::Array(parts) => {
                        let mut gparts = Vec::new();
                        let mut tool_results: Vec<Value> = Vec::new();
                        for p in parts {
                            let pt = p.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            match pt {
                                "text" => {
                                    if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                                        gparts.push(json!({"text": t}));
                                    }
                                }
                                "image" => {
                                    if let (Some(mt), Some(d)) = (
                                        p.get("media_type").and_then(|v| v.as_str()),
                                        p.get("data").and_then(|v| v.as_str()),
                                    ) {
                                        gparts.push(json!({"inlineData": {"mimeType": mt, "data": d}}));
                                    }
                                }
                                "tool_use" => {
                                    gparts.push(json!({
                                        "functionCall": {
                                            "name": p.get("name").cloned().unwrap_or(json!("")),
                                            "args": p.get("input").cloned().unwrap_or(json!({}))
                                        }
                                    }));
                                }
                                "tool_result" => {
                                    tool_results.push(json!({
                                        "role": "user",
                                        "parts": [{
                                            "functionResponse": {
                                                "name": p.get("tool_use_id").cloned().unwrap_or(json!("")),
                                                "response": {"result": tool_result_text(p)}
                                            }
                                        }]
                                    }));
                                }
                                _ => {}
                            }
                        }
                        if !gparts.is_empty() {
                            contents.push(json!({"role": grole, "parts": gparts}));
                        }
                        for tr in tool_results {
                            contents.push(tr);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    let mut gen = json!({});
    if let Some(mt) = body.get("max_tokens").and_then(|v| v.as_u64()) {
        gen["maxOutputTokens"] = json!(mt);
    }
    if let Some(t) = body.get("temperature").and_then(|v| v.as_f64()) {
        gen["temperature"] = json!(t);
    }
    if let Some(tp) = body.get("top_p").and_then(|v| v.as_f64()) {
        gen["topP"] = json!(tp);
    }
    if let Some(stop) = body.get("stop_sequences").and_then(|v| v.as_array()) {
        gen["stopSequences"] = json!(stop);
    }
    if let Some(tc) = thinking_config_from_anthropic(body) {
        gen["thinkingConfig"] = tc;
    }

    let mut out = json!({
        "contents": contents,
        "generationConfig": gen,
    });
    if !system_text.is_empty() {
        out["systemInstruction"] = json!({"parts": [{"text": system_text}]});
    }
    if let Some(tools) = body.get("tools") {
        let fns = function_declarations_from_anthropic(tools);
        if !fns.is_empty() {
            out["tools"] = json!([{"functionDeclarations": fns}]);
        }
    }
    if let Some(tc) = body.get("tool_choice") {
        out["toolConfig"] = tool_config_from_anthropic(tc);
    }
    out["model"] = json!(model);
    out
}

fn tool_result_text(p: &Value) -> String {
    match p.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|x| x.get("text").and_then(|v| v.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// OpenAI Chat Completions 请求 → Google generateContent 请求
pub fn openai_to_google(body: &Value, model: &str) -> Value {
    let mut contents = Vec::new();
    let mut system_text = String::new();

    if let Some(msgs) = body.get("messages").and_then(|v| v.as_array()) {
        for msg in msgs {
            let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("user");
            let grole = if role == "assistant" { "model" } else { "user" };
            if role == "system" {
                if let Some(c) = msg.get("content") {
                    system_text.push_str(str_from_content(c).as_str());
                }
                continue;
            }
            let content = msg.get("content");
            let mut gparts = Vec::new();
            let mut tool_results: Vec<Value> = Vec::new();

            if let Some(c) = content {
                match c {
                    Value::String(s) => gparts.push(json!({"text": s})),
                    Value::Array(arr) => {
                        for part in arr {
                            let pt = part.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            if pt == "text" {
                                if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                                    gparts.push(json!({"text": t}));
                                }
                            } else if pt == "image_url" {
                                if let Some(url) = part.get("image_url").and_then(|u| u.get("url")).and_then(|v| v.as_str()) {
                                    if let Some(comma) = url.find(',') {
                                        let mt = &url[5..comma].split(';').next().unwrap_or("image/png");
                                        gparts.push(json!({"inlineData": {"mimeType": mt, "data": &url[comma + 1..]}}));
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            // tool_calls（assistant）
            if let Some(tcs) = msg.get("tool_calls").and_then(|v| v.as_array()) {
                for tc in tcs {
                    let name = tc.get("function").and_then(|f| f.get("name")).cloned().unwrap_or(json!(""));
                    let args = tc.get("function").and_then(|f| f.get("arguments"))
                        .and_then(|v| v.as_str())
                        .and_then(|s| serde_json::from_str::<Value>(s).ok())
                        .unwrap_or(json!({}));
                    gparts.push(json!({"functionCall": {"name": name, "args": args}}));
                }
            }
            // tool 角色消息（工具结果）
            if role == "tool" {
                let name = msg.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let result = match content {
                    Some(Value::String(s)) => s.clone(),
                    _ => String::new(),
                };
                tool_results.push(json!({
                    "role": "user",
                    "parts": [{"functionResponse": {"name": name, "response": {"result": result}}}]
                }));
            }
            if !gparts.is_empty() {
                contents.push(json!({"role": grole, "parts": gparts}));
            }
            for tr in tool_results {
                contents.push(tr);
            }
        }
    }

    let mut gen = json!({});
    if let Some(mt) = body.get("max_tokens").or_else(|| body.get("max_completion_tokens")).and_then(|v| v.as_u64()) {
        gen["maxOutputTokens"] = json!(mt);
    }
    if let Some(t) = body.get("temperature").and_then(|v| v.as_f64()) {
        gen["temperature"] = json!(t);
    }
    if let Some(tp) = body.get("top_p").and_then(|v| v.as_f64()) {
        gen["topP"] = json!(tp);
    }

    let mut out = json!({"contents": contents, "generationConfig": gen});
    if !system_text.is_empty() {
        out["systemInstruction"] = json!({"parts": [{"text": system_text}]});
    }
    if let Some(tools) = body.get("tools") {
        let fns = function_declarations_from_openai(tools);
        if !fns.is_empty() {
            out["tools"] = json!([{"functionDeclarations": fns}]);
        }
    }
    if let Some(tc) = body.get("tool_choice") {
        out["toolConfig"] = tool_config_from_openai(tc);
    }
    out["model"] = json!(model);
    out
}

fn str_from_content(c: &Value) -> String {
    match c {
        Value::String(s) => s.clone(),
        Value::Array(arr) => arr.iter().filter_map(|p| p.get("text").and_then(|v| v.as_str())).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  请求转换：Google → Anthropic / OpenAI
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Google generateContent 请求 → Anthropic Messages 请求
pub fn google_to_anthropic(body: &Value, model: &str) -> Value {
    let mut messages = Vec::new();
    let mut system_text = String::new();
    if let Some(si) = body.get("systemInstruction").and_then(|v| v.get("parts")).and_then(|v| v.as_array()) {
        for p in si {
            if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                system_text.push_str(t);
            }
        }
    }
    if let Some(contents) = body.get("contents").and_then(|v| v.as_array()) {
        for c in contents {
            let role = c.get("role").and_then(|v| v.as_str()).unwrap_or("user");
            let arole = if role == "model" { "assistant" } else { "user" };
            let mut blocks = Vec::new();
            if let Some(parts) = c.get("parts").and_then(|v| v.as_array()) {
                for p in parts {
                    if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                        blocks.push(json!({"type": "text", "text": t}));
                    } else if p.get("functionCall").is_some() {
                        let fc = &p["functionCall"];
                        blocks.push(json!({
                            "type": "tool_use",
                            "id": fc.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                            "name": fc.get("name").cloned().unwrap_or(json!("")),
                            "input": fc.get("args").cloned().unwrap_or(json!({}))
                        }));
                    } else if p.get("functionResponse").is_some() {
                        let fr = &p["functionResponse"];
                        blocks.push(json!({
                            "type": "tool_result",
                            "tool_use_id": fr.get("name").cloned().unwrap_or(json!("")),
                            "content": fr.get("response").and_then(|r| r.get("result")).cloned().unwrap_or(json!(""))
                        }));
                    } else if p.get("inlineData").is_some() {
                        let id = &p["inlineData"];
                        blocks.push(json!({
                            "type": "image",
                            "source": {"type": "base64", "media_type": id.get("mimeType").cloned().unwrap_or(json!("image/png")), "data": id.get("data").cloned().unwrap_or(json!(""))}
                        }));
                    }
                }
            }
            if !blocks.is_empty() {
                messages.push(json!({"role": arole, "content": blocks}));
            }
        }
    }

    let mut out = json!({"model": model, "messages": messages});
    if !system_text.is_empty() {
        out["system"] = json!(system_text);
    }
    if let Some(gc) = body.get("generationConfig") {
        if let Some(mt) = gc.get("maxOutputTokens").and_then(|v| v.as_u64()) {
            out["max_tokens"] = json!(mt);
        }
        if let Some(t) = gc.get("temperature").and_then(|v| v.as_f64()) {
            out["temperature"] = json!(t);
        }
        if let Some(tp) = gc.get("topP").and_then(|v| v.as_f64()) {
            out["top_p"] = json!(tp);
        }
        if let Some(tc) = gc.get("thinkingConfig") {
            out["thinking"] = json!({"type": "enabled", "budget_tokens": tc.get("thinkingBudget").and_then(|v| v.as_u64()).unwrap_or(0)});
        }
    }
    if let Some(tools) = body.get("tools").and_then(|v| v.get(0)).and_then(|t| t.get("functionDeclarations")) {
        let atools: Vec<Value> = tools.as_array().map(|arr| {
            arr.iter().map(|f| json!({
                "name": f.get("name").cloned().unwrap_or(json!("")),
                "description": f.get("description").cloned().unwrap_or(json!("")),
                "input_schema": f.get("parameters").cloned().unwrap_or(json!({}))
            })).collect()
        }).unwrap_or_default();
        if !atools.is_empty() {
            out["tools"] = json!(atools);
        }
    }
    out
}

/// Google generateContent 请求 → OpenAI Chat Completions 请求
pub fn google_to_openai(body: &Value, model: &str) -> Value {
    let mut messages = Vec::new();
    if let Some(si) = body.get("systemInstruction").and_then(|v| v.get("parts")).and_then(|v| v.as_array()) {
        let t: String = si.iter().filter_map(|p| p.get("text").and_then(|v| v.as_str())).collect::<Vec<_>>().join("\n");
        if !t.is_empty() {
            messages.push(json!({"role": "system", "content": t}));
        }
    }
    if let Some(contents) = body.get("contents").and_then(|v| v.as_array()) {
        for c in contents {
            let role = c.get("role").and_then(|v| v.as_str()).unwrap_or("user");
            let orole = if role == "model" { "assistant" } else { "user" };
            let mut text = String::new();
            let mut tool_calls = Vec::new();
            if let Some(parts) = c.get("parts").and_then(|v| v.as_array()) {
                for p in parts {
                    if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                        text.push_str(t);
                    } else if p.get("functionCall").is_some() {
                        let fc = &p["functionCall"];
                        tool_calls.push(json!({
                            "id": fc.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                            "type": "function",
                            "function": {"name": fc.get("name").cloned().unwrap_or(json!("")), "arguments": fc.get("args").and_then(|v| serde_json::to_string(v).ok()).unwrap_or_else(|| "{}".to_string())}
                        }));
                    } else if p.get("functionResponse").is_some() {
                        let fr = &p["functionResponse"];
                        messages.push(json!({"role": "tool", "name": fr.get("name").cloned().unwrap_or(json!("")), "content": fr.get("response").and_then(|r| r.get("result")).cloned().unwrap_or(json!(""))}));
                    }
                }
            }
            let mut m = json!({"role": orole});
            if !tool_calls.is_empty() {
                m["tool_calls"] = json!(tool_calls);
            } else if !text.is_empty() {
                m["content"] = json!(text);
            }
            messages.push(m);
        }
    }
    let mut out = json!({"model": model, "messages": messages});
    if let Some(gc) = body.get("generationConfig") {
        if let Some(mt) = gc.get("maxOutputTokens").and_then(|v| v.as_u64()) {
            out["max_completion_tokens"] = json!(mt);
        }
        if let Some(t) = gc.get("temperature").and_then(|v| v.as_f64()) {
            out["temperature"] = json!(t);
        }
        if let Some(tp) = gc.get("topP").and_then(|v| v.as_f64()) {
            out["top_p"] = json!(tp);
        }
    }
    if let Some(tools) = body.get("tools").and_then(|v| v.get(0)).and_then(|t| t.get("functionDeclarations")) {
        let ot: Vec<Value> = tools.as_array().map(|arr| {
            arr.iter().map(|f| json!({"type": "function", "function": {
                "name": f.get("name").cloned().unwrap_or(json!("")),
                "description": f.get("description").cloned().unwrap_or(json!("")),
                "parameters": f.get("parameters").cloned().unwrap_or(json!({}))
            }})).collect()
        }).unwrap_or_default();
        if !ot.is_empty() {
            out["tools"] = json!(ot);
        }
    }
    out
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  响应转换：Google → Anthropic / OpenAI
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn finish_reason_to_anthropic(r: &str) -> &'static str {
    match r {
        "STOP" => "end_turn",
        "MAX_TOKENS" => "max_tokens",
        "SAFETY" | "RECITATION" | "OTHER" => "end_turn",
        _ => "end_turn",
    }
}
fn finish_reason_to_openai(r: &str) -> &'static str {
    match r {
        "STOP" => "stop",
        "MAX_TOKENS" => "length",
        _ => "stop",
    }
}

/// Google 响应 → Anthropic 响应（request_model 用于伪装回填）
pub fn google_response_to_anthropic(resp: &Value, request_model: &str) -> Value {
    let mut content = Vec::new();
    let mut stop = "end_turn";
    let mut input_tokens = 0u64;
    let mut output_tokens = 0u64;

    if let Some(cands) = resp.get("candidates").and_then(|v| v.as_array()) {
        if let Some(cand) = cands.first() {
            if let Some(fr) = cand.get("finishReason").and_then(|v| v.as_str()) {
                stop = finish_reason_to_anthropic(fr);
            }
            if let Some(parts) = cand.get("content").and_then(|v| v.get("parts")).and_then(|v| v.as_array()) {
                for p in parts {
                    if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                        content.push(json!({"type": "text", "text": t}));
                    } else if p.get("functionCall").is_some() {
                        let fc = &p["functionCall"];
                        content.push(json!({
                            "type": "tool_use",
                            "id": fc.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                            "name": fc.get("name").cloned().unwrap_or(json!("")),
                            "input": fc.get("args").cloned().unwrap_or(json!({}))
                        }));
                    } else if p.get("thought").and_then(|v| v.as_bool()).unwrap_or(false) {
                        if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                            content.push(json!({"type": "thinking", "thinking": t}));
                        }
                    }
                }
            }
        }
    }
    if let Some(um) = resp.get("usageMetadata") {
        input_tokens = um.get("promptTokenCount").and_then(|v| v.as_u64()).unwrap_or(0);
        let cand = um.get("candidatesTokenCount").and_then(|v| v.as_u64()).unwrap_or(0);
        let thought = um.get("thoughtsTokenCount").and_then(|v| v.as_u64()).unwrap_or(0);
        output_tokens = cand + thought;
    }

    json!({
        "id": "msg_google",
        "type": "message",
        "role": "assistant",
        "content": content,
        "model": request_model,
        "stop_reason": stop,
        "stop_sequence": null,
        "usage": {"input_tokens": input_tokens, "output_tokens": output_tokens}
    })
}

/// Google 响应 → OpenAI 响应（request_model 用于伪装回填）
pub fn google_response_to_openai(resp: &Value, request_model: &str) -> Value {
    let mut message_content = String::new();
    let mut tool_calls = Vec::new();
    let mut finish = "stop";
    let mut input_tokens = 0u64;
    let mut output_tokens = 0u64;

    if let Some(cands) = resp.get("candidates").and_then(|v| v.as_array()) {
        if let Some(cand) = cands.first() {
            if let Some(fr) = cand.get("finishReason").and_then(|v| v.as_str()) {
                finish = finish_reason_to_openai(fr);
            }
            if let Some(parts) = cand.get("content").and_then(|v| v.get("parts")).and_then(|v| v.as_array()) {
                for p in parts {
                    if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                        message_content.push_str(t);
                    } else if p.get("functionCall").is_some() {
                        let fc = &p["functionCall"];
                        tool_calls.push(json!({
                            "id": fc.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                            "type": "function",
                            "function": {"name": fc.get("name").cloned().unwrap_or(json!("")), "arguments": fc.get("args").and_then(|v| serde_json::to_string(v).ok()).unwrap_or_else(|| "{}".to_string())}
                        }));
                    }
                }
            }
        }
    }
    if let Some(um) = resp.get("usageMetadata") {
        input_tokens = um.get("promptTokenCount").and_then(|v| v.as_u64()).unwrap_or(0);
        let cand = um.get("candidatesTokenCount").and_then(|v| v.as_u64()).unwrap_or(0);
        let thought = um.get("thoughtsTokenCount").and_then(|v| v.as_u64()).unwrap_or(0);
        output_tokens = cand + thought;
    }

    let mut message = json!({"role": "assistant"});
    if !tool_calls.is_empty() {
        message["tool_calls"] = json!(tool_calls);
    } else {
        message["content"] = json!(if message_content.is_empty() { Value::Null } else { Value::String(message_content) });
    }

    json!({
        "id": "chatcmpl_google",
        "object": "chat.completion",
        "created": chrono::Utc::now().timestamp(),
        "model": request_model,
        "choices": [{"index": 0, "message": message, "finish_reason": finish}],
        "usage": {"prompt_tokens": input_tokens, "completion_tokens": output_tokens, "total_tokens": input_tokens + output_tokens}
    })
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  响应转换：OpenAI / Anthropic → Google（供非流式 inbound=google 使用）
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// OpenAI 响应 → Google 响应（request_model 用于伪装回填）
pub fn openai_response_to_google(resp: &Value, request_model: &str) -> Value {
    let mut parts = Vec::new();
    let mut finish = "STOP";
    let mut in_t = 0u64;
    let mut out_t = 0u64;

    if let Some(choice) = resp.get("choices").and_then(|v| v.as_array()).and_then(|a| a.first()) {
        if let Some(fr) = choice.get("finish_reason").and_then(|v| v.as_str()) {
            finish = match fr { "stop" => "STOP", "length" => "MAX_TOKENS", _ => "STOP" };
        }
        if let Some(msg) = choice.get("message") {
            if let Some(t) = msg.get("content").and_then(|v| v.as_str()) {
                if !t.is_empty() {
                    parts.push(json!({"text": t}));
                }
            }
            if let Some(tcs) = msg.get("tool_calls").and_then(|v| v.as_array()) {
                for tc in tcs {
                    let name = tc.get("function").and_then(|f| f.get("name")).cloned().unwrap_or(json!(""));
                    let args = tc.get("function").and_then(|f| f.get("arguments")).and_then(|v| v.as_str())
                        .and_then(|s| serde_json::from_str::<Value>(s).ok()).unwrap_or(json!({}));
                    parts.push(json!({"functionCall": {"name": name, "args": args}}));
                }
            }
        }
    }
    if let Some(u) = resp.get("usage") {
        in_t = u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        out_t = u.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    }

    json!({
        "candidates": [{"content": {"role": "model", "parts": parts}, "finishReason": finish}],
        "model": request_model,
        "usageMetadata": {"promptTokenCount": in_t, "candidatesTokenCount": out_t, "totalTokenCount": in_t + out_t}
    })
}

/// Anthropic 响应 → Google 响应（request_model 用于伪装回填）
pub fn anthropic_response_to_google(resp: &Value, request_model: &str) -> Value {
    let mut parts = Vec::new();
    let mut finish = "STOP";
    let mut in_t = 0u64;
    let mut out_t = 0u64;

    if let Some(sr) = resp.get("stop_reason").and_then(|v| v.as_str()) {
        finish = match sr { "end_turn" | "stop_sequence" => "STOP", "max_tokens" => "MAX_TOKENS", "tool_use" => "STOP", _ => "STOP" };
    }
    if let Some(content) = resp.get("content").and_then(|v| v.as_array()) {
        for p in content {
            let pt = p.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if pt == "text" {
                if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                    parts.push(json!({"text": t}));
                }
            } else if pt == "tool_use" {
                parts.push(json!({"functionCall": {"name": p.get("name").cloned().unwrap_or(json!("")), "args": p.get("input").cloned().unwrap_or(json!({}))}}));
            } else if pt == "thinking" {
                if let Some(t) = p.get("thinking").and_then(|v| v.as_str()) {
                    parts.push(json!({"thought": true, "text": t}));
                }
            }
        }
    }
    if let Some(u) = resp.get("usage") {
        in_t = u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        out_t = u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    }

    json!({
        "candidates": [{"content": {"role": "model", "parts": parts}, "finishReason": finish}],
        "model": request_model,
        "usageMetadata": {"promptTokenCount": in_t, "candidatesTokenCount": out_t, "totalTokenCount": in_t + out_t}
    })
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  流式转换：上游 OpenAI / Anthropic 的 SSE chunk → Google SSE
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 把上游（openai 或 anthropic）的流式 chunk 转换为 Google SSE `data:` 行。
/// `source` 为上游协议（"openai" | "anthropic"）。
pub struct GoogleStreamConverter {
    source: String,
    model: String,
    input_tokens: u64,
    output_tokens: u64,
}

impl GoogleStreamConverter {
    pub fn new(source: &str, model: &str) -> Self {
        Self { source: source.to_string(), model: model.to_string(), input_tokens: 0, output_tokens: 0 }
    }

    pub fn usage(&self) -> (u64, u64) {
        (self.input_tokens, self.output_tokens)
    }

    /// 兼容统一管线：把上游（openai/anthropic）chunk 转换为 Google SSE 行向量。
    pub fn convert_chunk(&mut self, chunk: &Value) -> Vec<String> {
        match self.convert_source_chunk(chunk) {
            Some(s) => vec![s],
            None => Vec::new(),
        }
    }

    /// 输入一个已解析的上游 chunk JSON，返回 0~1 个 Google SSE `data:` 行（含 `\n\n`）。
    pub fn convert_source_chunk(&mut self, chunk: &Value) -> Option<String> {
        if self.source == "openai" {
            self.convert_openai_chunk(chunk)
        } else {
            self.convert_anthropic_chunk(chunk)
        }
    }

    fn emit(&self, parts: Vec<Value>, finish: Option<&str>) -> String {
        let mut cand = json!({"content": {"role": "model", "parts": parts}});
        if let Some(f) = finish {
            cand["finishReason"] = json!(f);
        }
        let mut obj = json!({"candidates": [cand]});
        obj["model"] = json!(self.model);
        if self.input_tokens > 0 || self.output_tokens > 0 {
            obj["usageMetadata"] = json!({
                "promptTokenCount": self.input_tokens,
                "candidatesTokenCount": self.output_tokens,
                "totalTokenCount": self.input_tokens + self.output_tokens
            });
        }
        format!("data: {}\n\n", serde_json::to_string(&obj).unwrap())
    }

    fn convert_openai_chunk(&mut self, chunk: &Value) -> Option<String> {
        if let Some(u) = chunk.get("usage") {
            self.input_tokens = u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
            self.output_tokens = u.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        }
        let choice = chunk.get("choices").and_then(|v| v.as_array()).and_then(|a| a.first())?;
        let delta = choice.get("delta")?;
        let mut parts = Vec::new();
        if let Some(t) = delta.get("content").and_then(|v| v.as_str()) {
            if !t.is_empty() {
                parts.push(json!({"text": t}));
            }
        }
        if let Some(tcs) = delta.get("tool_calls").and_then(|v| v.as_array()) {
            for tc in tcs {
                let name = tc.get("function").and_then(|f| f.get("name")).and_then(|v| v.as_str()).unwrap_or("");
                let args = tc.get("function").and_then(|f| f.get("arguments")).and_then(|v| v.as_str()).unwrap_or("");
                parts.push(json!({"functionCall": {"name": name, "args": serde_json::from_str::<Value>(args).unwrap_or(json!({}))}}));
            }
        }
        let finish = choice.get("finish_reason").and_then(|v| v.as_str()).map(|fr| match fr {
            "stop" => "STOP",
            "length" => "MAX_TOKENS",
            _ => "STOP",
        });
        if parts.is_empty() && finish.is_none() {
            return None;
        }
        Some(self.emit(parts, finish))
    }

    fn convert_anthropic_chunk(&mut self, chunk: &Value) -> Option<String> {
        let etype = chunk.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match etype {
            "message_start" => {
                if let Some(u) = chunk.get("message").and_then(|m| m.get("usage")) {
                    self.input_tokens = u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                }
                None
            }
            "content_block_delta" => {
                let delta = chunk.get("delta")?;
                let dt = delta.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if dt == "text_delta" {
                    if let Some(t) = delta.get("text").and_then(|v| v.as_str()) {
                        return Some(self.emit(vec![json!({"text": t})], None));
                    }
                } else if dt == "input_json_delta" {
                    let args = delta.get("partial_json").and_then(|v| v.as_str()).unwrap_or("");
                    return Some(self.emit(vec![json!({"functionCall": {"name": "", "args": serde_json::from_str::<Value>(args).unwrap_or(json!({}))}})], None));
                }
                None
            }
            "content_block_start" => {
                if let Some(b) = chunk.get("content_block") {
                    if b.get("type").and_then(|v| v.as_str()).unwrap_or("") == "tool_use" {
                        let name = b.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        return Some(self.emit(vec![json!({"functionCall": {"name": name, "args": json!({})}})], None));
                    }
                }
                None
            }
            "message_delta" => {
                if let Some(u) = chunk.get("usage") {
                    self.output_tokens = u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                }
                None
            }
            "message_stop" => Some(self.emit(vec![], Some("STOP"))),
            _ => None,
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  流式转换：Google SSE → Anthropic SSE
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 把上游 Google 的流式 chunk（完整 GenerateContentResponse）转换为 Anthropic SSE 事件。
pub struct GoogleToAnthropicStreamConverter {
    model: String,
    message_id: String,
    started: bool,
    content_index: usize,
    tool_open: bool,
    text_open: bool,
    thought_open: bool,
    input_tokens: u64,
    output_tokens: u64,
}

impl GoogleToAnthropicStreamConverter {
    pub fn new(model: &str) -> Self {
        Self {
            model: model.to_string(),
            message_id: format!("msg_google_{}", chrono::Utc::now().timestamp_millis()),
            started: false,
            content_index: 0,
            tool_open: false,
            text_open: false,
            thought_open: false,
            input_tokens: 0,
            output_tokens: 0,
        }
    }

    pub fn usage(&self) -> (u64, u64) {
        (self.input_tokens, self.output_tokens)
    }

    /// 输入一个已解析的 Google chunk JSON，返回 0~N 个 Anthropic SSE 事件字符串。
    pub fn convert_chunk(&mut self, chunk: &Value) -> Vec<String> {
        let mut events = Vec::new();

        // usageMetadata（出现在最后一个 chunk）
        if let Some(um) = chunk.get("usageMetadata") {
            self.input_tokens = um.get("promptTokenCount").and_then(|v| v.as_u64()).unwrap_or(self.input_tokens);
            let cand = um.get("candidatesTokenCount").and_then(|v| v.as_u64()).unwrap_or(0);
            let thought = um.get("thoughtsTokenCount").and_then(|v| v.as_u64()).unwrap_or(0);
            self.output_tokens = cand + thought;
        }

        if !self.started {
            self.started = true;
            let start = json!({
                "type": "message_start",
                "message": {
                    "id": self.message_id,
                    "type": "message",
                    "role": "assistant",
                    "content": [],
                    "model": self.model,
                    "stop_reason": null,
                    "stop_sequence": null,
                    "usage": {"input_tokens": self.input_tokens, "output_tokens": 0}
                }
            });
            events.push(format!("event: message_start\ndata: {}\n\n", start));
        }

        let mut finish: Option<&str> = None;
        if let Some(cands) = chunk.get("candidates").and_then(|v| v.as_array()) {
            if let Some(cand) = cands.first() {
                if let Some(fr) = cand.get("finishReason").and_then(|v| v.as_str()) {
                    finish = Some(fr);
                }
                if let Some(parts) = cand.get("content").and_then(|v| v.get("parts")).and_then(|v| v.as_array()) {
                    for p in parts {
                        let is_thought = p.get("thought").and_then(|v| v.as_bool()).unwrap_or(false);
                        if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                            if is_thought {
                                // thinking delta
                                if !self.thought_open {
                                    self.close_open_blocks(&mut events);
                                    self.thought_open = true;
                                    events.push(format!("event: content_block_start\ndata: {}\n\n", json!({
                                        "type": "content_block_start", "index": self.content_index,
                                        "content_block": {"type": "thinking", "thinking": ""}
                                    })));
                                }
                                events.push(format!("event: content_block_delta\ndata: {}\n\n", json!({
                                    "type": "content_block_delta", "index": self.content_index,
                                    "delta": {"type": "thinking_delta", "thinking": t}
                                })));
                            } else {
                                if !self.text_open {
                                    self.close_open_blocks(&mut events);
                                    self.text_open = true;
                                    events.push(format!("event: content_block_start\ndata: {}\n\n", json!({
                                        "type": "content_block_start", "index": self.content_index,
                                        "content_block": {"type": "text", "text": ""}
                                    })));
                                }
                                events.push(format!("event: content_block_delta\ndata: {}\n\n", json!({
                                    "type": "content_block_delta", "index": self.content_index,
                                    "delta": {"type": "text_delta", "text": t}
                                })));
                            }
                        } else if p.get("functionCall").is_some() {
                            let fc = &p["functionCall"];
                            let name = fc.get("name").cloned().unwrap_or(json!(""));
                            let args = fc.get("args").cloned().unwrap_or(json!({}));
                            self.close_open_blocks(&mut events);
                            // 每个 functionCall 单独成块
                            events.push(format!("event: content_block_start\ndata: {}\n\n", json!({
                                "type": "content_block_start", "index": self.content_index,
                                "content_block": {"type": "tool_use", "id": name, "name": name, "input": {}}
                            })));
                            events.push(format!("event: content_block_delta\ndata: {}\n\n", json!({
                                "type": "content_block_delta", "index": self.content_index,
                                "delta": {"type": "input_json_delta", "partial_json": serde_json::to_string(&args).unwrap_or_else(|_| "{}".to_string())}
                            })));
                            events.push(format!("event: content_block_stop\ndata: {}\n\n", json!({
                                "type": "content_block_stop", "index": self.content_index
                            })));
                            self.content_index += 1;
                            self.tool_open = false;
                        }
                    }
                }
            }
        }

        if let Some(fr) = finish {
            self.close_open_blocks(&mut events);
            let stop_reason = match fr {
                "STOP" => "end_turn",
                "MAX_TOKENS" => "max_tokens",
                _ => "end_turn",
            };
            events.push(format!("event: message_delta\ndata: {}\n\n", json!({
                "type": "message_delta",
                "delta": {"stop_reason": stop_reason, "stop_sequence": null},
                "usage": {"output_tokens": self.output_tokens}
            })));
            events.push("event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n".to_string());
        }

        events
    }

    fn close_open_blocks(&mut self, events: &mut Vec<String>) {
        if self.text_open || self.thought_open || self.tool_open {
            events.push(format!("event: content_block_stop\ndata: {}\n\n", json!({
                "type": "content_block_stop", "index": self.content_index
            })));
            self.content_index += 1;
            self.text_open = false;
            self.thought_open = false;
            self.tool_open = false;
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  流式转换：Google SSE → OpenAI SSE
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 把上游 Google 的流式 chunk 转换为 OpenAI SSE chunk。
pub struct GoogleToOpenaiStreamConverter {
    model: String,
    id: String,
    tool_index: usize,
    input_tokens: u64,
    output_tokens: u64,
}

impl GoogleToOpenaiStreamConverter {
    pub fn new(model: &str) -> Self {
        Self {
            model: model.to_string(),
            id: format!("chatcmpl_google_{}", chrono::Utc::now().timestamp_millis()),
            tool_index: 0,
            input_tokens: 0,
            output_tokens: 0,
        }
    }

    pub fn usage(&self) -> (u64, u64) {
        (self.input_tokens, self.output_tokens)
    }

    pub fn convert_chunk(&mut self, chunk: &Value) -> Vec<String> {
        if let Some(um) = chunk.get("usageMetadata") {
            self.input_tokens = um.get("promptTokenCount").and_then(|v| v.as_u64()).unwrap_or(self.input_tokens);
            let cand = um.get("candidatesTokenCount").and_then(|v| v.as_u64()).unwrap_or(0);
            let thought = um.get("thoughtsTokenCount").and_then(|v| v.as_u64()).unwrap_or(0);
            self.output_tokens = cand + thought;
        }

        let mut delta = json!({});
        let mut finish: Option<&str> = None;
        if let Some(cands) = chunk.get("candidates").and_then(|v| v.as_array()) {
            if let Some(cand) = cands.first() {
                if let Some(fr) = cand.get("finishReason").and_then(|v| v.as_str()) {
                    finish = Some(fr);
                }
                if let Some(parts) = cand.get("content").and_then(|v| v.get("parts")).and_then(|v| v.as_array()) {
                    for p in parts {
                        let is_thought = p.get("thought").and_then(|v| v.as_bool()).unwrap_or(false);
                        if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                            if !is_thought {
                                delta["content"] = json!(t);
                            }
                        } else if p.get("functionCall").is_some() {
                            let fc = &p["functionCall"];
                            let name = fc.get("name").cloned().unwrap_or(json!(""));
                            let args = serde_json::to_string(fc.get("args").unwrap_or(&json!({}))).unwrap_or_else(|_| "{}".to_string());
                            let idx = self.tool_index;
                            self.tool_index += 1;
                            delta["tool_calls"] = json!([{
                                "index": idx,
                                "id": name,
                                "type": "function",
                                "function": {"name": name, "arguments": args}
                            }]);
                        }
                    }
                }
            }
        }

        let finish_reason = finish.map(|fr| match fr {
            "STOP" => "stop",
            "MAX_TOKENS" => "length",
            _ => "stop",
        });

        let chunk_out = json!({
            "id": self.id,
            "object": "chat.completion.chunk",
            "created": chrono::Utc::now().timestamp(),
            "model": self.model,
            "choices": [{
                "index": 0,
                "delta": delta,
                "logprobs": null,
                "finish_reason": finish_reason
            }]
        });
        vec![format!("data: {}\n\n", serde_json::to_string(&chunk_out).unwrap())]
    }
}
