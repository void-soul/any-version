//! Additional Anthropic thinking mappings for non-whitelisted OpenAI-compatible models.
use serde_json::Value;

pub fn apply(
    inbound: &str,
    outbound: &str,
    source: &Value,
    target: &mut Value,
    model: &str,
    base_url: &str,
) {
    if inbound != "anthropic"
        || outbound != "openai"
        || crate::proxy::optimizers::supports_reasoning_effort(model)
    {
        return;
    }
    let Some(obj) = target.as_object_mut() else {
        return;
    };
    obj.remove("reasoning_effort");
    let Some(effort) = requested(source) else {
        return;
    };
    let name = model
        .rsplit('/')
        .next()
        .unwrap_or(model)
        .trim()
        .to_ascii_lowercase();
    let platform = url::Url::parse(base_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .unwrap_or_default();
    let has = |token: &str| platform.contains(token);
    let standard = match effort {
        "low" => "low",
        "medium" => "medium",
        _ => "high",
    };
    // Platform contract precedes model hints, as in cc-switch 2122f7da.
    let mapped = if has("githubcopilot.com")
        || has("siliconflow")
        || has("modelscope")
        || has("opencode.ai")
    {
        return;
    } else if has("openrouter") {
        match effort {
            "max" => "xhigh",
            other => other,
        }
    } else {
        if name.starts_with("gpt-3")
            || name.starts_with("gpt-4")
            || name.starts_with("chatgpt-4")
            || name.starts_with("llama")
            || name.contains("mistral")
            || has("mistral")
        {
            return;
        }
        let vendor = format!("{platform} {name}");
        if [
            "kimi",
            "moonshot",
            "glm",
            "zhipu",
            "z.ai",
            "qwen",
            "dashscope",
            "bailian",
            "minimax",
            "mimo",
        ]
        .iter()
        .any(|key| vendor.contains(key))
        {
            return;
        }
        if vendor.contains("deepseek") {
            match effort {
                "low" => "low",
                "max" => "max",
                _ => "high",
            }
        } else if vendor.contains("stepfun") || name.starts_with("step-") {
            if name.contains("2603") {
                if effort == "low" {
                    "low"
                } else {
                    "high"
                }
            } else if name.contains("step-3.7-flash") {
                standard
            } else {
                return;
            }
        } else {
            standard
        }
    };
    obj.insert("reasoning_effort".into(), serde_json::json!(mapped));
}

fn requested(body: &Value) -> Option<&'static str> {
    match body.pointer("/thinking/type").and_then(Value::as_str) {
        Some("enabled" | "adaptive") => {}
        _ => return None,
    }
    if let Some(effort) = body
        .pointer("/output_config/effort")
        .and_then(Value::as_str)
    {
        return match effort {
            "low" => Some("low"),
            "medium" => Some("medium"),
            "high" => Some("high"),
            "xhigh" => Some("xhigh"),
            "max" => Some("max"),
            _ => None,
        };
    }
    crate::proxy::optimizers::resolve_reasoning_effort("gpt-5.4", body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn mapped(model: &str, url: &str, thinking: &str, effort: &str) -> Value {
        let source = json!({"model":"claimed", "messages":[], "thinking":{"type":thinking}, "output_config":{"effort":effort}});
        let mut target =
            crate::proxy::convert::convert_request("anthropic", "openai", &source, model);
        apply("anthropic", "openai", &source, &mut target, model, url);
        target
    }
    #[test]
    fn non_whitelisted_reasoning_is_mapped() {
        assert_eq!(
            mapped("deepseek-v4", "https://api.deepseek.com", "adaptive", "max")
                ["reasoning_effort"],
            "max"
        );
        assert_eq!(
            mapped(
                "deepseek-v4",
                "https://api.deepseek.com",
                "adaptive",
                "xhigh"
            )["reasoning_effort"],
            "high"
        );
        assert_eq!(
            mapped("gpt-oss-120b", "https://api.groq.com", "adaptive", "max")["reasoning_effort"],
            "high"
        );
        assert_eq!(
            mapped(
                "vendor/new-model",
                "https://relay.example",
                "enabled",
                "medium"
            )["reasoning_effort"],
            "medium"
        );
    }
    #[test]
    fn platform_rules_precede_model_hints() {
        assert!(mapped(
            "deepseek-v4",
            "https://api.siliconflow.cn",
            "adaptive",
            "max"
        )
        .get("reasoning_effort")
        .is_none());
        assert_eq!(
            mapped(
                "deepseek-v4",
                "https://openrouter.ai/api/v1",
                "adaptive",
                "max"
            )["reasoning_effort"],
            "xhigh"
        );
    }
    #[test]
    fn budget_and_optional_effort_follow_the_reference_boundaries() {
        let source = serde_json::json!({"thinking":{"type":"enabled","budget_tokens":1000}});
        let mut target = serde_json::json!({"model":"deepseek-v4"});
        apply(
            "anthropic",
            "openai",
            &source,
            &mut target,
            "deepseek-v4",
            "https://api.deepseek.com",
        );
        assert_eq!(target["reasoning_effort"], "low");
        for url in ["https://api.modelscope.cn", "https://opencode.ai/zen"] {
            assert!(mapped("deepseek-v4", url, "adaptive", "max")
                .get("reasoning_effort")
                .is_none());
        }
        assert_eq!(
            mapped(
                "step-3.5-flash-2603",
                "https://api.stepfun.com",
                "adaptive",
                "medium"
            )["reasoning_effort"],
            "high"
        );
        assert_eq!(
            mapped(
                "step-3.7-flash",
                "https://api.stepfun.com",
                "adaptive",
                "medium"
            )["reasoning_effort"],
            "medium"
        );
        assert!(mapped(
            "step-3.5-flash",
            "https://api.stepfun.com",
            "adaptive",
            "max"
        )
        .get("reasoning_effort")
        .is_none());
        let source = serde_json::json!({"output_config":{"effort":"max"}});
        let mut target = serde_json::json!({});
        apply(
            "anthropic",
            "openai",
            &source,
            &mut target,
            "deepseek-v4",
            "https://api.deepseek.com",
        );
        assert!(target.get("reasoning_effort").is_none());
    }

    #[test]
    fn disabled_unknown_and_unsupported_are_not_injected() {
        for model in [
            "kimi-k2",
            "glm-5",
            "qwen3",
            "minimax-m2",
            "mimo-v2",
            "gpt-4o",
            "meta-llama/Llama-3",
            "mistral-large",
        ] {
            assert!(
                mapped(model, "https://relay.example", "adaptive", "max")
                    .get("reasoning_effort")
                    .is_none(),
                "{model}"
            );
        }
        assert!(
            mapped("deepseek-v4", "https://api.deepseek.com", "disabled", "max")
                .get("reasoning_effort")
                .is_none()
        );
        assert!(
            mapped("unknown", "https://relay.example", "adaptive", "turbo")
                .get("reasoning_effort")
                .is_none()
        );
        let source = json!({"thinking":{"type":"adaptive"},"output_config":{"effort":"max"}});
        let mut target = json!({"reasoning_effort":"custom", "keep":true});
        apply(
            "openai",
            "openai",
            &source,
            &mut target,
            "unknown",
            "https://relay.example",
        );
        assert_eq!(target["reasoning_effort"], "custom");
        apply(
            "anthropic",
            "openai",
            &source,
            &mut target,
            "gpt-6-sol",
            "https://relay.example",
        );
        assert_eq!(target["reasoning_effort"], "custom");
    }
}
