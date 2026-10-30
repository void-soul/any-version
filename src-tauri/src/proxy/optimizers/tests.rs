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
    for m in [
        "deepseek-chat",
        "deepseek-reasoner",
        "glm-4.6",
        "qwen-max",
        "qwq-32b",
        "moonshot-v1-8k",
        "o1-mini",
    ] {
        assert!(is_text_only_model(m), "{m} 应判定为纯文本");
    }
    // 视觉版本 / 视觉模型必须排除（名字里带 vl、vision、4v、qvq）
    for m in [
        "glm-4v",
        "qwen-vl-max",
        "qwen2-vl-72b",
        "moonshot-v1-8k-vision",
        "qvq-72b",
        "gpt-4o",
        "claude-sonnet-4-5",
        "",
    ] {
        assert!(!is_text_only_model(m), "{m} 不该被判定为纯文本");
    }
    // 大小写不敏感（供应商回填的模型名可能带大写）
    assert!(is_text_only_model("DeepSeek-V3"));
}

/// 三态判定：显式声明 > 注册表；注册表没覆盖时必须是 Unknown（fail-open）。
#[test]
fn image_capability_is_tri_state_with_declared_priority() {
    use ImageInputCapability as C;
    // 显式声明压过注册表：哪怕注册表说是纯文本，声明「支持」就是支持
    assert_eq!(
        resolve_image_input_capability("deepseek-chat", Some(true), true),
        C::Supported
    );
    // 显式声明「不支持」不受启发式开关影响（这是事实，不是预测）
    assert_eq!(
        resolve_image_input_capability("gpt-4o", Some(false), false),
        C::Unsupported
    );
    // 注册表命中
    assert_eq!(
        resolve_image_input_capability("deepseek-chat", None, true),
        C::Unsupported
    );
    // 关掉启发式 → 只剩 Unknown
    assert_eq!(
        resolve_image_input_capability("deepseek-chat", None, false),
        C::Unknown
    );
    // 注册表没覆盖且无声明 → Unknown（绝不能当成「支持」而放行，也不能当成「不支持」而丢图）
    assert_eq!(
        resolve_image_input_capability("some-unknown-model", None, true),
        C::Unknown
    );
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
    assert!(
        fixed.get("thinking").is_none(),
        "顶层 thinking 应被移除: {fixed}"
    );
    let content = fixed["messages"][0]["content"].as_array().unwrap();
    assert!(content.iter().all(|b| b["type"] != "thinking"), "{fixed}");
    assert_eq!(content[0]["text"], "hi", "非 thinking 块要保留");

    // 请求里本来就没有 thinking：剥不出东西 → 不认领，也不白重试一次
    let no_thinking =
        json!({ "model": "claude-sonnet-4-5", "messages": [{ "role": "user", "content": "hi" }] });
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
        assert!(
            !supports_reasoning_effort(unsupported),
            "不应支持: {unsupported}"
        );
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
        assert_eq!(
            resolve_reasoning_effort("gpt-5.4", &body),
            Some(expected),
            "{effort}"
        );
    }
    // `max` 只在有独立 max 档的模型上保留，其余降级为 xhigh。
    // 注意判定用的是「解析后的上游模型」，不是 body.model。
    let max_body = json!({"model": "claude-sonnet-4", "output_config": {"effort": "max"}});
    for model in [
        "gpt-5.6",
        "gpt-5.6-sol",
        "gpt-5.6-terra",
        "gpt-5.6-luna",
        "gpt-6-astra",
    ] {
        assert_eq!(
            resolve_reasoning_effort(model, &max_body),
            Some("max"),
            "{model}"
        );
    }
    assert_eq!(
        resolve_reasoning_effort("gpt-5.4", &max_body),
        Some("xhigh")
    );
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
