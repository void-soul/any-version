//! 代理请求优化器
//!
//! 提供 6 种优化策略：
//! 1. Cache Injector — 注入 Anthropic cache_control 断点以启用 prompt caching
//! 2. Thinking Signature Rectifier — 遇到 thinking 签名错误时剥离 thinking 块并重试
//! 3. Thinking Budget Rectifier — 遇到 budget_tokens 过小错误时修正并重试
//! 4. DeepSeek Thinking Normalization — 为 DeepSeek 兼容端点规范化 thinking 块
//! 5. Media Sanitizer — 遇到不支持图片的错误时替换图片块并重试
//! 6. Thinking Optimizer — 根据模型类型主动优化 thinking 参数

use serde_json::{json, Value};

const MAX_CACHE_BREAKPOINTS: usize = 4;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  1. Cache Injector
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 向 Anthropic 请求体注入 cache_control 断点以启用 prompt caching。
/// 目标：最后一个 tool、最后一个 system 块、最后一条 assistant 消息中的最后一个非 thinking 块。
pub fn inject_cache_breakpoints(body: &mut Value) {
    let mut budget = MAX_CACHE_BREAKPOINTS;

    // 1. Last tool in tools array
    if let Some(tools) = body.get_mut("tools").and_then(|v| v.as_array_mut()) {
        if let Some(last) = tools.last_mut() {
            if budget > 0 {
                last.as_object_mut()
                    .map(|o| o.insert("cache_control".into(), json!({"type": "ephemeral"})));
                budget -= 1;
            }
        }
    }

    // 2. Last system block
    if budget > 0 {
        if let Some(system) = body.get_mut("system") {
            if let Some(arr) = system.as_array_mut() {
                if let Some(last) = arr.last_mut() {
                    last.as_object_mut()
                        .map(|o| o.insert("cache_control".into(), json!({"type": "ephemeral"})));
                    budget -= 1;
                }
            } else if system.is_string() {
                // 将字符串 system 转换为带 cache_control 的数组格式
                let text = system.as_str().unwrap_or("").to_string();
                *system = json!([{"type": "text", "text": text, "cache_control": {"type": "ephemeral"}}]);
                budget -= 1;
            }
        }
    }

    // 3. Last non-thinking block in last assistant message
    if budget > 0 {
        if let Some(messages) = body.get_mut("messages").and_then(|v| v.as_array_mut()) {
            for msg in messages.iter_mut().rev() {
                if msg.get("role").and_then(|v| v.as_str()) == Some("assistant") {
                    // 将 string content 转为 array 格式
                    if let Some(content_val) = msg.get_mut("content") {
                        if content_val.is_string() {
                            let text = content_val.as_str().unwrap_or("").to_string();
                            *content_val = json!([{"type": "text", "text": text}]);
                        }
                    }
                    if let Some(content) = msg.get_mut("content").and_then(|v| v.as_array_mut()) {
                        for block in content.iter_mut().rev() {
                            let btype =
                                block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            if btype != "thinking" && btype != "redacted_thinking" {
                                block.as_object_mut().map(|o| {
                                    o.insert(
                                        "cache_control".into(),
                                        json!({"type": "ephemeral"}),
                                    )
                                });
                                break;
                            }
                        }
                    }
                    break;
                }
            }
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  2. Thinking Signature Rectifier
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 检查错误消息是否指示 thinking 签名问题。
/// 匹配 cc-switch 的 7 种模式。
pub fn is_thinking_signature_error(status: u16, body: &str) -> bool {
    if status != 400 && status != 422 {
        return false;
    }
    let lower = body.to_lowercase();
    // Pattern 1: "signature" + "thinking" + "block" + "invalid"
    (lower.contains("signature") && lower.contains("thinking") && lower.contains("block") && lower.contains("invalid"))
    // Pattern 2: "thought signature" + ("not valid" | "invalid")
    || (lower.contains("thought signature") && (lower.contains("not valid") || lower.contains("invalid")))
    // Pattern 3: "must start with a thinking block"
    || lower.contains("must start with a thinking block")
    // Pattern 4: "expected" + ("thinking" | "redacted_thinking") + "found" + "tool_use"
    || (lower.contains("expected") && (lower.contains("thinking") || lower.contains("redacted_thinking")) && lower.contains("found") && lower.contains("tool_use"))
    // Pattern 5: "signature" + "field required"
    || (lower.contains("signature") && lower.contains("field required"))
    // Pattern 6: "signature" + "extra inputs are not permitted"
    || (lower.contains("signature") && lower.contains("extra inputs are not permitted"))
    // Pattern 7: ("thinking" | "redacted_thinking") + "cannot be modified"
    || ((lower.contains("thinking") || lower.contains("redacted_thinking")) && lower.contains("cannot be modified"))
    // Pattern 8: Chinese/i18n error messages
    || lower.contains("非法请求") || lower.contains("illegal request") || lower.contains("invalid request")
}

/// 从消息历史中剥离所有 thinking/redacted_thinking 块。
/// 同时移除非 thinking 块中的 signature 字段。
pub fn strip_thinking_blocks(body: &mut Value) {
    if let Some(messages) = body.get_mut("messages").and_then(|v| v.as_array_mut()) {
        for msg in messages.iter_mut() {
            if let Some(content) = msg.get_mut("content").and_then(|v| v.as_array_mut()) {
                content.retain(|block| {
                    let btype = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    btype != "thinking" && btype != "redacted_thinking"
                });
                // 移除剩余块中的 signature 字段
                for block in content.iter_mut() {
                    block.as_object_mut().map(|o| o.remove("signature"));
                }
            }
        }
    }
    // 移除顶层 thinking 配置
    body.as_object_mut().map(|o| o.remove("thinking"));
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  3. Thinking Budget Rectifier
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 检查错误是否指示 budget_tokens 过小。
pub fn is_thinking_budget_error(status: u16, body: &str) -> bool {
    if status != 400 && status != 422 {
        return false;
    }
    let lower = body.to_lowercase();
    lower.contains("budget_tokens")
        && (lower.contains("too small")
            || lower.contains("less than")
            || lower.contains("minimum"))
}

/// 修正 thinking 预算：设置 budget_tokens 为 32000，确保 max_tokens 足够。
pub fn fix_thinking_budget(body: &mut Value) {
    if let Some(thinking) = body.get_mut("thinking") {
        if thinking.get("type").and_then(|v| v.as_str()) == Some("adaptive") {
            return; // adaptive 没有固定预算，跳过
        }
        if let Some(o) = thinking.as_object_mut() {
            o.insert("type".into(), json!("enabled"));
            o.insert("budget_tokens".into(), json!(32000));
        }
    }
    // 确保 max_tokens 足够大
    if let Some(max_tokens) = body.get("max_tokens").and_then(|v| v.as_u64()) {
        if max_tokens < 32001 {
            if let Some(o) = body.as_object_mut() {
                o.insert("max_tokens".into(), json!(64000));
            }
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  4. DeepSeek Thinking Normalization
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 检查目标 URL 是否为 DeepSeek 兼容端点。
pub fn is_deepseek_url(url: &str) -> bool {
    let lower = url.to_lowercase();
    lower.contains("deepseek")
        || lower.contains("moonshot")
        || lower.contains("kimi")
        || lower.contains("mimo")
        || lower.contains("xiaomimimo")
}

/// 为 DeepSeek 兼容端点规范化 thinking 块。
/// - 如果 assistant 有 tool_use 但没有 thinking，注入占位 thinking
/// - 从 thinking 块中剥离签名
/// - 当 thinking 被禁用时移除 effort 参数
pub fn normalize_deepseek_thinking(body: &mut Value, upstream_url: &str) {
    if !is_deepseek_url(upstream_url) {
        return;
    }

    // 从 thinking 块中剥离签名
    if let Some(messages) = body.get_mut("messages").and_then(|v| v.as_array_mut()) {
        for msg in messages.iter_mut() {
            if msg.get("role").and_then(|v| v.as_str()) != Some("assistant") {
                continue;
            }
            if let Some(content) = msg.get_mut("content").and_then(|v| v.as_array_mut()) {
                let has_tool_use = content
                    .iter()
                    .any(|b| b.get("type").and_then(|v| v.as_str()) == Some("tool_use"));
                let has_thinking = content.iter().any(|b| {
                    let t = b.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    t == "thinking" || t == "redacted_thinking"
                });

                // 从所有 thinking 块中剥离签名
                for block in content.iter_mut() {
                    if let Some(o) = block.as_object_mut() {
                        o.remove("signature");
                    }
                    // 将 redacted_thinking 转换为普通 thinking
                    if block.get("type").and_then(|v| v.as_str()) == Some("redacted_thinking")
                    {
                        if let Some(o) = block.as_object_mut() {
                            o.insert("type".into(), json!("thinking"));
                            o.insert("thinking".into(), json!("[redacted]"));
                        }
                    }
                }

                // 如果有 tool_use 但没有 thinking，注入占位 thinking
                if has_tool_use && !has_thinking {
                    content.insert(0, json!({"type": "thinking", "thinking": "tool call"}));
                }
            }
        }
    }

    // 当 thinking 被禁用时移除 effort 参数
    if let Some(thinking) = body.get("thinking") {
        if thinking.get("type").and_then(|v| v.as_str()) == Some("disabled") {
            if let Some(o) = body.as_object_mut() {
                o.remove("output_config");
            }
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  5. Media Sanitizer
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 检查上游错误是否关于不支持的图片。
pub fn is_unsupported_image_error(status: u16, body: &str) -> bool {
    if status != 400 && status != 415 && status != 422 && status != 501 {
        return false;
    }
    let lower = body.to_lowercase();
    // Must mention image/modality AND unsupported
    let mentions_image = lower.contains("image")
        || lower.contains("vision")
        || lower.contains("multimodal")
        || lower.contains("modality")
        || lower.contains("media")
        || lower.contains("attachment");
    let mentions_unsupported = lower.contains("unsupported")
        || lower.contains("not supported")
        || lower.contains("does not support")
        || lower.contains("doesn't support")
        || lower.contains("only supports text")
        || lower.contains("text only")
        || lower.contains("text-only")
        || lower.contains("invalid content type")
        || lower.contains("unknown variant")
        || lower.contains("cannot process")
        || lower.contains("cannot handle");
    mentions_image && mentions_unsupported
}

/// 替换所有图片内容块为文本标记。
/// 保留 cache_control 字段以维持 prompt cache 连续性。
/// 返回被替换的块数。
pub fn replace_image_blocks(body: &mut Value) -> usize {
    let mut count = 0;
    if let Some(messages) = body.get_mut("messages").and_then(|v| v.as_array_mut()) {
        for msg in messages.iter_mut() {
            if let Some(content) = msg.get_mut("content").and_then(|v| v.as_array_mut()) {
                for block in content.iter_mut() {
                    let btype = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    if btype == "image" || btype == "image_url" || btype == "input_image" {
                        let cache_control = block.get("cache_control").cloned();
                        let mut replacement = json!({"type": "text", "text": "[Unsupported Image]"});
                        if let Some(cc) = cache_control {
                            replacement
                                .as_object_mut()
                                .map(|o| o.insert("cache_control".into(), cc));
                        }
                        *block = replacement;
                        count += 1;
                    }
                }
            }
        }
    }
    count
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  6. Thinking Optimizer
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 主动优化 thinking 参数（基于模型名称）。
/// - Haiku 模型：跳过（无 thinking）
/// - Opus 4.6+/Sonnet 4.6+：使用 adaptive thinking + max effort
/// - 旧模型：强制 enabled thinking，budget = max_tokens - 1
pub fn optimize_thinking(body: &mut Value) {
    let model = body
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_lowercase();
    let normalized = model.replace('.', "-");

    // Haiku: skip
    if normalized.contains("haiku") {
        return;
    }

    // Adaptive path: opus-4-6+, sonnet-4-6+
    let is_adaptive = normalized.contains("opus-4-8")
        || normalized.contains("opus-4-7")
        || normalized.contains("opus-4-6")
        || normalized.contains("sonnet-4-6")
        || normalized.contains("fable");

    if is_adaptive {
        if let Some(o) = body.as_object_mut() {
            o.insert("thinking".into(), json!({"type": "adaptive"}));
            o.insert("output_config".into(), json!({"effort": "max"}));
        }
        // Append beta header
        if let Some(betas) = body
            .get_mut("anthropic_beta")
            .and_then(|v| v.as_array_mut())
        {
            if !betas
                .iter()
                .any(|b| b.as_str() == Some("context-1m-2025-08-07"))
            {
                betas.push(json!("context-1m-2025-08-07"));
            }
        } else {
            if let Some(o) = body.as_object_mut() {
                o.insert("anthropic_beta".into(), json!(["context-1m-2025-08-07"]));
            }
        }
        return;
    }

    // Legacy path: force enabled thinking with max budget
    let max_tokens = body
        .get("max_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(16384);
    let current_budget = body
        .get("thinking")
        .and_then(|t| t.get("budget_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    if current_budget < max_tokens.saturating_sub(1) {
        if let Some(o) = body.as_object_mut() {
            o.insert(
                "thinking".into(),
                json!({
                    "type": "enabled",
                    "budget_tokens": max_tokens - 1
                }),
            );
        }
    }

    // Append interleaved-thinking beta for legacy models
    if let Some(betas) = body.get_mut("anthropic_beta").and_then(|v| v.as_array_mut()) {
        if !betas.iter().any(|b| b.as_str() == Some("interleaved-thinking-2025-05-14")) {
            betas.push(json!("interleaved-thinking-2025-05-14"));
        }
    } else {
        if let Some(o) = body.as_object_mut() {
            o.insert("anthropic_beta".into(), json!(["interleaved-thinking-2025-05-14"]));
        }
    }
}
