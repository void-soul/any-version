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
///
/// 返回「实际剥掉了多少东西」（块数 + 顶层 thinking 是否被移除）：反应式整流靠它判断
/// 该不该认领这个错误 —— 剥不出东西说明请求里本来就没有 thinking，重试也没用，
/// 应该把错误让给后面的 budget / media / unknown-field 分支。
pub fn strip_thinking_blocks(body: &mut Value) -> usize {
    let mut removed = 0usize;
    if let Some(messages) = body.get_mut("messages").and_then(|v| v.as_array_mut()) {
        for msg in messages.iter_mut() {
            if let Some(content) = msg.get_mut("content").and_then(|v| v.as_array_mut()) {
                let before = content.len();
                content.retain(|block| {
                    let btype = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    btype != "thinking" && btype != "redacted_thinking"
                });
                removed += before.saturating_sub(content.len());
                // 移除剩余块中的 signature 字段
                for block in content.iter_mut() {
                    block.as_object_mut().map(|o| o.remove("signature"));
                }
            }
        }
    }
    // 移除顶层 thinking 配置
    if let Some(o) = body.as_object_mut() {
        if o.remove("thinking").is_some() {
            removed += 1;
        }
    }
    removed
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

// ─── 纯文本模型注册表（发送前预判）────────────────────────
//
// 抄 cc-switch 的 `request_media_heuristic`：它维护一份「已确认纯文本」的模型注册表，
// 命中就在**发送前**剥掉图片块，省掉一次必然失败的往返（而不是等上游报错再降级）。
//
// 只收**已确认**的家族：误判会把用户发的图片静默丢掉，代价远大于漏判 ——
// 漏判还有「上游报错后降级（media_fallback）」那条路兜底。
const TEXT_ONLY_MODEL_PREFIXES: &[&str] = &[
    "deepseek-",    // deepseek-chat / -reasoner / -coder 全部纯文本（其 API 无视觉模型）
    "qwq-",         // QwQ 推理模型
    "glm-",         // GLM 文本系列（视觉版 glm-4v / glm-4.5v 由下面的「以 v 结尾」规则排除）
    "moonshot-v1-", // Moonshot 文本系列（视觉版 moonshot-v1-vision）
    "qwen",         // Qwen 文本系列（qwen-vl / qwen2-vl / qvq 由 VISION_HINTS 排除）
    "ernie-",       // 文心一言文本系列
    "o1-mini",      // OpenAI o1-mini（o1 / o3 支持图片，不在此列）
];

/// 名字里带这些片段 = 该模型**支持**图片，即使在上面注册表里也要跳过。
const VISION_HINTS: &[&str] = &["-vl", "vl-", "-vision", "vision-", "4v", "qvq", "-audio", "-omni"];

/// 该模型是否已确认「不接受图片输入」。
pub fn is_text_only_model(model: &str) -> bool {
    let m = model.trim().to_lowercase();
    if m.is_empty() || VISION_HINTS.iter().any(|h| m.contains(h)) {
        return false;
    }
    // 视觉版的常见后缀：glm-4v / glm-4.5v（这些名字里没有可匹配的特征片段）
    if m.ends_with("v") {
        return false;
    }
    TEXT_ONLY_MODEL_PREFIXES.iter().any(|p| m.starts_with(p))
}

/// 纯文本模型预判：命中注册表就把图片块降级为文本标记，返回降级块数。
///
/// 与 `ProxyConfig` 解耦是有意的 —— 单测里构造 ProxyConfig 会把 tauri 运行时
/// 链接进测试二进制（Windows 上直接加载失败），所以开关判断留在调用方，
/// 这里只做「模型 → 降级」这件纯事。
pub fn strip_images_for_text_only_model(body: &mut Value) -> usize {
    // 先拷出模型名再改 body：不可变借用在 mutable 借用前结束
    let text_only = body
        .get("model")
        .and_then(|v| v.as_str())
        .map(is_text_only_model)
        .unwrap_or(false);
    if !text_only {
        return 0;
    }
    let n = replace_image_blocks(body);
    if n > 0 {
        eprintln!("[rectifier] 纯文本模型预判：该模型不支持图片，发送前降级 {n} 个图片块");
    }
    n
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

    // 纯文本模型预判（与协议无关，先于协议分支执行）：已确认不接受图片的模型，
    // 发送前就把图片块降级成文本标记，省掉一次必然失败的往返。
    // 与「上游报错后降级（media_fallback）」是两条独立路径 —— 关掉这项只停用
    // 注册表预判，报错兜底仍在，且不会改动模型目录里的能力声明。
    if config.rectifier_media_heuristic {
        strip_images_for_text_only_model(body);
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

/// 签名整流：剥得动才返回 Some（剥不动说明请求里本来就没有 thinking，
/// 不该由签名整流认领这个错误 —— 让后面的 media / unknown-field 分支有机会处理）。
pub fn rectify_thinking_signature(body: &Value) -> Option<Value> {
    let mut fixed = body.clone();
    (strip_thinking_blocks(&mut fixed) > 0).then_some(fixed)
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

    // Thinking 签名整流：上游报签名不合法时剥离 thinking 后重试一次（cc-switch 同款语义）。
    // 预防式已在 anthropic 出站剥过一轮，能走到这儿说明还有残留（例如入站的
    // redacted_thinking 是转换后才暴露的）。
    if config.rectifier_thinking_signature && is_thinking_signature_error(status, error_body) {
        if let Some(fixed) = rectify_thinking_signature(body) {
            return Some(fixed);
        }
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

    fn body_with_image(model: &str) -> Value {
        json!({
            "model": model,
            "messages": [
                { "role": "user", "content": [
                    { "type": "image_url", "image_url": { "url": "data:image/png;base64,AAA" } },
                    { "type": "text", "text": "这是什么？" }
                ] }
            ]
        })
    }

    fn body_with_thinking() -> Value {
        json!({
            "model": "claude-sonnet-4-5",
            "thinking": { "type": "enabled", "budget_tokens": 1024 },
            "messages": [
                { "role": "assistant", "content": [
                    { "type": "thinking", "thinking": "...", "signature": "bad" },
                    { "type": "text", "text": "hi" }
                ] }
            ]
        })
    }

    /// 纯文本注册表：宁可漏判也不能误判 —— 误判会把用户发的图片静默丢掉。
    #[test]
    fn text_only_registry_is_conservative() {
        for m in ["deepseek-chat", "deepseek-reasoner", "glm-4.6", "qwen-max", "qwq-32b", "moonshot-v1-8k", "o1-mini"] {
            assert!(is_text_only_model(m), "{m} 应判定为纯文本");
        }
        // 视觉版本 / 视觉模型必须排除（名字里带 vl、vision、4v、qvq）
        for m in ["glm-4v", "qwen-vl-max", "qwen2-vl-72b", "moonshot-v1-8k-vision", "qvq-72b", "gpt-4o", "claude-sonnet-4-5", ""] {
            assert!(!is_text_only_model(m), "{m} 不该被判定为纯文本");
        }
        // 大小写不敏感（供应商回填的模型名可能带大写）
        assert!(is_text_only_model("DeepSeek-V3"));
    }

    /// 发送前预判：命中注册表就降级图片，未命中则原样转发。
    #[test]
    fn text_only_models_get_images_stripped_before_send() {
        let mut body = body_with_image("deepseek-chat");
        assert_eq!(strip_images_for_text_only_model(&mut body), 1);
        let first = &body["messages"][0]["content"][0];
        assert_eq!(first["type"], "text", "图片块应在发送前被降级: {body}");
        assert_eq!(first["text"], "[Unsupported Image]");
        // 相邻的文本块不能被动到
        assert_eq!(body["messages"][0]["content"][1]["text"], "这是什么？");

        // 支持图片的模型不受影响
        let mut kept = body_with_image("gpt-4o");
        assert_eq!(strip_images_for_text_only_model(&mut kept), 0);
        assert_eq!(kept["messages"][0]["content"][0]["type"], "image_url");
    }

    /// 签名整流：剥得动才认领错误（重试），剥不动就交给后面的分支。
    #[test]
    fn signature_rectify_only_claims_errors_it_can_fix() {
        let fixed = rectify_thinking_signature(&body_with_thinking())
            .expect("含 thinking 的请求应能被签名整流修正");
        assert!(fixed.get("thinking").is_none(), "顶层 thinking 应被移除: {fixed}");
        let content = fixed["messages"][0]["content"].as_array().unwrap();
        assert!(content.iter().all(|b| b["type"] != "thinking"), "{fixed}");
        assert_eq!(content[0]["text"], "hi", "非 thinking 块要保留");

        // 请求里本来就没有 thinking：剥不出东西 → 不认领，也不白重试一次
        let no_thinking = json!({ "model": "claude-sonnet-4-5", "messages": [{ "role": "user", "content": "hi" }] });
        assert!(
            rectify_thinking_signature(&no_thinking).is_none(),
            "没有 thinking 可剥就不该重试"
        );
    }

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
