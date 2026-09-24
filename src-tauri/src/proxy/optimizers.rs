//! 代理请求优化器
//!
//! 提供 6 种优化策略：
//! 1. Cache Injector — 注入 Anthropic cache_control 断点以启用 prompt caching
//! 2. Thinking Signature Rectifier — 遇到 thinking 签名错误时剥离 thinking 块并重试
//! 3. Thinking Budget Rectifier — 遇到 budget_tokens 过小错误时修正并重试
//! 4. DeepSeek Thinking Normalization — 为 DeepSeek 兼容端点规范化 thinking 块
//! 5. Media Sanitizer — 遇到不支持图片的错误时替换图片块并重试
//! 6. Thinking Optimizer — 根据模型类型主动优化 thinking 参数
//!
//! 整改后：优化器与整流器都在「出站协议 P_out 形态」上操作（按 outbound_protocol 分派），
//! 而非仅在 Anthropic 形态上操作，从而实现协议无关的能力复用。

use serde_json::{json, Value};
use super::types::ProxyConfig;

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

/// OpenAI 形态的 DeepSeek 规范化（对应 Anthropic 形态的 `normalize_deepseek_thinking`）。
/// 在 P_out=openai 时调用：Anthropic→OpenAI 转换后 thinking 已表达为 `reasoning_content`，
/// 这里处理 DeepSeek 兼容端点的特有约束：
/// - 防御性移除 Anthropic 残留的 `signature` 字段
/// - 若 assistant 消息带 `tool_calls` 但缺少 `reasoning_content`，注入占位，
///   避免 DeepSeek 报「tool_use 前必须存在 thinking」类错误
pub fn normalize_deepseek_thinking_openai(body: &mut Value, upstream_url: &str) {
    if !is_deepseek_url(upstream_url) {
        return;
    }
    if let Some(messages) = body.get_mut("messages").and_then(|v| v.as_array_mut()) {
        for msg in messages.iter_mut() {
            if msg.get("role").and_then(|v| v.as_str()) != Some("assistant") {
                continue;
            }
            if let Some(o) = msg.as_object_mut() {
                // OpenAI 形态无 signature 概念，但转换可能残留，防御性移除
                o.remove("signature");
                let has_tool_calls = o
                    .get("tool_calls")
                    .map(|v| v.as_array().map(|a| !a.is_empty()).unwrap_or(false))
                    .unwrap_or(false);
                let has_reasoning = o.contains_key("reasoning_content");
                if has_tool_calls && !has_reasoning {
                    o.insert("reasoning_content".into(), json!("tool call"));
                }
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

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  7. Thinking Optimizer（OpenAI / Google 形态）
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 取模型名的最后一段（去掉 `provider/model` 前缀）并转小写，供家族判定使用。
fn model_basename(model: &str) -> String {
    let lower = model.trim().to_ascii_lowercase();
    lower.rsplit('/').next().unwrap_or(&lower).to_string()
}

/// 是否为 OpenAI o 系列推理模型（o1 / o3 / o4-mini 等）。
fn is_openai_o_series(model: &str) -> bool {
    model
        .strip_prefix('o')
        .and_then(|rest| rest.chars().next())
        .is_some_and(|c| c.is_ascii_digit())
}

/// 模型是否支持 OpenAI 的 `reasoning_effort` 参数。
///
/// 家族与 cc-switch `supports_reasoning_effort` 对齐（抄自 8e478b2b / d6e05152）：
/// - o 系列：o1 / o3 / o4-mini 等
/// - GPT-5+：`gpt-` 后首个字符是数字且 ≥ 5（gpt-5 / gpt-5.1 / gpt-5-codex / gpt-6-astra）
/// - xAI Grok 4.5+：`grok-4.x`，x 为数字次版本且 ≥ 5 —— **解析而非枚举**，
///   未来版本（如 grok-4.10）无需再改白名单
/// - 保留 `grok-build-*` 家族
/// - 沿袭旧实现的模糊判定：模型名里含 `reasoning` 的也视作支持
pub fn supports_reasoning_effort(model: &str) -> bool {
    let base = model_basename(model);
    is_openai_o_series(&base)
        || (base.starts_with("gpt-")
            && base
                .strip_prefix("gpt-")
                .and_then(|rest| rest.chars().next())
                .is_some_and(|c| c.is_ascii_digit() && c >= '5'))
        || base
            .strip_prefix("grok-4.")
            .and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
            .and_then(|minor| minor.parse::<u32>().ok())
            .is_some_and(|minor| minor >= 5)
        || base.starts_with("grok-build-")
        || base.contains("reasoning")
}

/// 拥有独立 `max` 推理档位的模型；其余模型把 `max` 降级为 `xhigh`。
///
/// 抄自 cc-switch d6e05152：无差别把 `max` 压成 `xhigh` 会丢掉这些模型的最强档。
fn supports_max_reasoning_effort(model: &str) -> bool {
    matches!(
        model_basename(model).as_str(),
        "gpt-5.6" | "gpt-5.6-sol" | "gpt-5.6-terra" | "gpt-5.6-luna" | "gpt-6-astra"
    )
}

/// 从 Anthropic 形态请求体解析出应发给 OpenAI 上游的 `reasoning_effort`。
///
/// `model` 必须是**解析后的上游模型名**（不能直接读 body.model：跨协议转换时
/// body.model 可能还停留在入站「声称的模型」，判定 `max` 档会判错）。
///
/// 优先级（抄自 cc-switch d6e05152）：
/// 1. 显式 `output_config.effort` —— 直接保留用户意图；`max` 仅对有独立 max 档的
///    模型保留，否则降级 `xhigh`；未知取值返回 `None`（不注入）。
/// 2. 回落 `thinking.type` + `budget_tokens`：`adaptive` → `xhigh`；
///    `enabled` 按预算 `<4000` → `low`、`<16000` → `medium`、其余及无预算 → `high`；
///    `disabled` / 缺失 → `None`。
pub fn resolve_reasoning_effort(model: &str, body: &Value) -> Option<&'static str> {
    if let Some(effort) = body
        .pointer("/output_config/effort")
        .and_then(|v| v.as_str())
    {
        let normalized = effort.trim().to_ascii_lowercase();
        return match normalized.as_str() {
            "low" => Some("low"),
            "medium" => Some("medium"),
            "high" => Some("high"),
            "xhigh" => Some("xhigh"),
            "max" if supports_max_reasoning_effort(model) => Some("max"),
            "max" => Some("xhigh"),
            _ => None,
        };
    }

    match body.pointer("/thinking/type").and_then(|v| v.as_str()) {
        Some("adaptive") => Some("xhigh"),
        Some("enabled") => {
            let budget = body
                .pointer("/thinking/budget_tokens")
                .and_then(|v| v.as_u64());
            Some(match budget {
                Some(b) if b < 4_000 => "low",
                Some(b) if b < 16_000 => "medium",
                _ => "high",
            })
        }
        _ => None,
    }
}

/// OpenAI 形态 thinking 优化：对支持 reasoning 的模型设置 `reasoning_effort`。
///
/// 已经带 `reasoning_effort` 时直接放行：转换阶段（`transform::anthropic_to_openai` 第 8 步）
/// 会按入站意图写好该字段，或客户端直接使用 OpenAI 协议自己给出，都不能被兜底值覆盖。
pub fn optimize_thinking_openai(body: &mut Value) {
    let model = body.get("model").and_then(|v| v.as_str()).unwrap_or("");
    if !supports_reasoning_effort(model) {
        return;
    }
    if body
        .get("reasoning_effort")
        .and_then(|v| v.as_str())
        .is_some()
    {
        return;
    }
    // 未表达意图时维持旧行为：保守地按 high 请求
    let effort = resolve_reasoning_effort(model, body).unwrap_or("high");
    if let Some(o) = body.as_object_mut() {
        o.insert("reasoning_effort".into(), json!(effort));
    }
}

/// Google 形态 thinking 优化：在 `generationConfig.thinkingConfig` 中开启动态思考。
pub fn optimize_thinking_google(body: &mut Value) {
    if let Some(gc) = body.get_mut("generationConfig") {
        if let Some(o) = gc.as_object_mut() {
            // thinkingBudget=0 表示由模型动态决定（Gemini 2.5+）
            o.insert(
                "thinkingConfig".into(),
                json!({"thinkingBudget": 0, "includeThoughts": true}),
            );
        }
    } else if let Some(o) = body.as_object_mut() {
        o.insert(
            "generationConfig".into(),
            json!({"thinkingConfig": {"thinkingBudget": 0, "includeThoughts": true}}),
        );
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  协议感知分派（在 P_out 形态上操作）
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 按出站协议应用优化器（在 P_out 形态上操作）。
pub fn apply_optimizers(body: &mut Value, outbound_protocol: &str, config: &ProxyConfig) {
    if !config.optimizer_enabled {
        return;
    }
    match outbound_protocol {
        "anthropic" => {
            if config.optimizer_cache_injection {
                inject_cache_breakpoints(body);
            }
            if config.optimizer_deepseek {
                normalize_deepseek_thinking(body, &config.upstream_base_url);
            }
            if config.optimizer_thinking {
                optimize_thinking(body);
            }
        }
        "openai" => {
            // OpenAI 无 cache_control 概念，跳过 cache injection
            if config.optimizer_deepseek {
                normalize_deepseek_thinking_openai(body, &config.upstream_base_url);
            }
            if config.optimizer_thinking {
                optimize_thinking_openai(body);
            }
        }
        "google" => {
            if config.optimizer_thinking {
                optimize_thinking_google(body);
            }
        }
        _ => {}
    }
}

/// 预防式整流：在转发前对 P_out 形态做高概率修正。
pub fn apply_preventive_rectifiers(body: &mut Value, outbound_protocol: &str, config: &ProxyConfig) {
    if !config.rectifier_enabled {
        return;
    }
    match outbound_protocol {
        "anthropic" => {
            // thinking signature 几乎必然触发的场景：预防式剥离历史 thinking + signature
            if config.rectifier_thinking_signature {
                strip_thinking_blocks(body);
            }
        }
        "openai" | "google" => {
            // 这两个协议没有 Anthropic 的 thinking signature 概念，无需预防式剥离
        }
        _ => {}
    }
}

/// 反应式整流：上游报错后尝试修正一次。
/// 返回 Some(修正后 body) 表示可以重试，None 表示无法修正。
pub fn try_reactive_rectify(
    status: u16,
    error_body: &str,
    body: &Value,
    config: &ProxyConfig,
    outbound_protocol: &str,
) -> Option<Value> {
    if !config.rectifier_enabled {
        return None;
    }

    if config.rectifier_thinking_budget && is_thinking_budget_error(status, error_body) {
        let mut fixed = body.clone();
        fix_thinking_budget(&mut fixed);
        return Some(fixed);
    }

    if config.rectifier_media_fallback && is_unsupported_image_error(status, error_body) {
        let mut media_body = body.clone();
        if replace_image_blocks(&mut media_body) > 0 {
            return Some(media_body);
        }
    }

    // 协议不匹配整流：转换后仍残留的协议专有字段被上游拒绝
    if config.rectifier_protocol_mismatch && is_unknown_field_error(status, error_body) {
        let mut cleaned = body.clone();
        strip_unknown_fields(&mut cleaned, outbound_protocol);
        return Some(cleaned);
    }

    None
}

/// 检测上游错误是否指示「未知字段 / 不被支持的字段」。
pub fn is_unknown_field_error(status: u16, body: &str) -> bool {
    if status != 400 && status != 422 && status != 404 {
        return false;
    }
    let lower = body.to_lowercase();
    lower.contains("unknown field")
        || lower.contains("additional properties")
        || lower.contains("additionalproperties")
        || lower.contains("extraneous")
        || lower.contains("unexpected field")
        || lower.contains("not allowed")
        || lower.contains("unknown parameter")
        || lower.contains("unsupported parameter")
        || lower.contains("schema")
}

/// 协议专有字段集合（Anthropic 形态中常见、但 OpenAI/Google 上游不认识的字段）。
/// 仅在出站协议 ≠ anthropic 时由 `strip_unknown_fields` 移除。
const ANTHROPIC_ONLY_FIELDS: &[&str] = &[
    "thinking",
    "container",
    "mcp_servers",
    "service_tier",
    "anthropic_version",
    "anthropic-beta",
    "metadata",
    "top_k",
    "cache_control",
];

/// 剥离协议专有字段：当出站协议不是 anthropic 时，移除 `ANTHROPIC_ONLY_FIELDS`
/// 中转换后仍残留的 Anthropic 专有字段（被 OpenAI/Google 上游拒绝）。
/// 多入站协议下无法在反应式整流阶段确定具体入站，直接以出站协议判定即可
/// （若出站为 anthropic 则不剥离；其余情况剥离无害，因非 anthropic 字段本就不存在）。
pub fn strip_unknown_fields(body: &mut Value, outbound_protocol: &str) {
    if outbound_protocol == "anthropic" {
        return;
    }
    if let Some(o) = body.as_object_mut() {
        for f in ANTHROPIC_ONLY_FIELDS {
            o.remove(*f);
        }
    }
    // 同时清理 messages 内 content 块残留的 cache_control
    if let Some(messages) = body.get_mut("messages").and_then(|v| v.as_array_mut()) {
        for msg in messages.iter_mut() {
            if let Some(content) = msg.get_mut("content").and_then(|v| v.as_array_mut()) {
                for block in content.iter_mut() {
                    block.as_object_mut().map(|o| o.remove("cache_control"));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn supports_reasoning_effort_covers_modern_families() {
        // 白名单按家族而非枚举，未来版本无需再改（抄自 cc-switch 8e478b2b）
        for supported in [
            "o1",
            "o3",
            "o4-mini",
            "gpt-5",
            "gpt-5.1",
            "gpt-5-codex",
            "gpt-6-astra",
            "grok-4.5",
            "grok-4.6",
            "grok-4.7",
            "grok-4.10",
            "grok-4.7-build",
            "grok-build-0.1",
            "openai/o3",
            "GPT-5.6",
        ] {
            assert!(supports_reasoning_effort(supported), "应支持: {supported}");
        }
        for unsupported in [
            "gpt-4o",
            "gpt-4.1",
            "claude-sonnet-4-6",
            "deepseek-v4",
            // grok-4.x 只接 x >= 5
            "grok-4",
            "grok-4.4",
            "grok-4.",
            "grok-4.build",
        ] {
            assert!(!supports_reasoning_effort(unsupported), "不应支持: {unsupported}");
        }
    }

    #[test]
    fn resolve_reasoning_effort_prefers_output_config() {
        for (effort, expected) in [
            ("low", "low"),
            ("medium", "medium"),
            ("high", "high"),
            ("xhigh", "xhigh"),
        ] {
            let body = json!({"model": "claude-sonnet-4", "output_config": {"effort": effort}});
            assert_eq!(resolve_reasoning_effort("gpt-5.4", &body), Some(expected), "{effort}");
        }
        // `max` 只在有独立 max 档的模型上保留，其余降级为 xhigh。
        // 注意判定用的是「解析后的上游模型」，不是 body.model。
        let max_body = json!({"model": "claude-sonnet-4", "output_config": {"effort": "max"}});
        for model in ["gpt-5.6", "gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna", "gpt-6-astra"] {
            assert_eq!(resolve_reasoning_effort(model, &max_body), Some("max"), "{model}");
        }
        assert_eq!(resolve_reasoning_effort("gpt-5.4", &max_body), Some("xhigh"));
        // 未知取值不注入
        let unknown = json!({"output_config": {"effort": "turbo"}});
        assert_eq!(resolve_reasoning_effort("gpt-5.4", &unknown), None);
    }

    #[test]
    fn resolve_reasoning_effort_falls_back_to_thinking_budget() {
        let with_thinking = |t: Value| json!({"thinking": t});
        assert_eq!(
            resolve_reasoning_effort("gpt-5.4", &with_thinking(json!({"type": "adaptive"}))),
            Some("xhigh")
        );
        assert_eq!(
            resolve_reasoning_effort(
                "gpt-5.4",
                &with_thinking(json!({"type": "enabled", "budget_tokens": 1000}))
            ),
            Some("low")
        );
        assert_eq!(
            resolve_reasoning_effort(
                "gpt-5.4",
                &with_thinking(json!({"type": "enabled", "budget_tokens": 8000}))
            ),
            Some("medium")
        );
        assert_eq!(
            resolve_reasoning_effort(
                "gpt-5.4",
                &with_thinking(json!({"type": "enabled", "budget_tokens": 32000}))
            ),
            Some("high")
        );
        // enabled 但没给预算：保守取 high
        assert_eq!(
            resolve_reasoning_effort("gpt-5.4", &with_thinking(json!({"type": "enabled"}))),
            Some("high")
        );
        // disabled / 未表达：不注入
        assert_eq!(
            resolve_reasoning_effort("gpt-5.4", &with_thinking(json!({"type": "disabled"}))),
            None
        );
        assert_eq!(resolve_reasoning_effort("gpt-5.4", &json!({})), None);
    }

    #[test]
    fn optimize_thinking_openai_does_not_clobber_explicit_effort() {
        // 转换阶段已按入站意图写好 effort，优化器的兜底值不能覆盖它
        let mut body = json!({"model": "gpt-5.6", "reasoning_effort": "max"});
        optimize_thinking_openai(&mut body);
        assert_eq!(body["reasoning_effort"], json!("max"));
    }

    #[test]
    fn optimize_thinking_openai_defaults_to_high_for_supported_model() {
        let mut body = json!({"model": "gpt-5.4"});
        optimize_thinking_openai(&mut body);
        assert_eq!(body["reasoning_effort"], json!("high"));
    }

    #[test]
    fn optimize_thinking_openai_injects_nothing_for_unsupported_model() {
        let mut body = json!({"model": "gpt-4o"});
        optimize_thinking_openai(&mut body);
        assert!(body.get("reasoning_effort").is_none(), "{body}");
    }
}
