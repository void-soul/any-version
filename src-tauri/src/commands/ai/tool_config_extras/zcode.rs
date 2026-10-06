//! ZCode（`~/.zcode/v2/`）的附加写入。
//!
//! 通用映射写的是 `config.json`（`~/.zcode/v2/config.json`）—— 那只是**provider 定义**。
//! 3.14+ 的默认模型选择与 provider 启用状态在**另一个文件**里：
//! `~/.zcode/v2/provider_config.json`（`schemaVersion: 1` + `config.*`）。只写 `config.json`
//! 的话，ZCode 界面上的「默认模型」仍是旧的 —— 用户点了设置、文件也确实变了，但双击打开
//! 跑的却是上一个模型。
//!
//! 对照 EchoBird `services/tool_config_manager/zcode.rs`。两条硬约束：
//!
//! 1. **文件不存在就不创建** —— 它的 schema 由 ZCode 自己维护，凭空造一个可能被判非法；
//! 2. `schemaVersion != 1` 或结构不符时**报错中止**，绝不猜着写。这里刻意不静默跳过：
//!    跳过等于「看着成功、实际还是旧模型」，比报错更糟。

use std::path::Path;

use super::{read_or_empty, sibling, write_file, ExtrasCtx};
use crate::commands::ai::tool_config_restore::RestoreOutcome;

/// 我们在 ZCode 里注册的规则/选择项需要被替换掉，不能累积出重复条目。
fn rules_for_us(rule: &serde_json::Value, provider: &str) -> bool {
    rule.get("providerId").and_then(|v| v.as_str()) == Some(provider)
}

pub(super) fn apply(ctx: &ExtrasCtx<'_>) -> Result<Vec<String>, String> {
    let path = sibling(ctx.main_path, "provider_config.json");
    if !path.exists() {
        eprintln!(
            "[extras] {} 不存在 → 只写 config.json（不替 ZCode 造它的内部配置文件）",
            path.display()
        );
        return Ok(Vec::new());
    }
    let provider = ctx.provider;
    let model_id = ctx.model_name;
    let existing = read_or_empty(&path);
    let mut root: serde_json::Value = serde_json::from_str(&existing).map_err(|e| {
        format!(
            "{} 不是合法 JSON，已放弃写入以免写坏 ZCode 配置: {e}",
            path.display()
        )
    })?;

    // schemaVersion 变了说明结构不是我们认识的那套 —— 报错，而不是猜着改
    if root.get("schemaVersion").and_then(|v| v.as_u64()) != Some(1) {
        return Err(format!(
            "{} 的 schemaVersion 不是 1（ZCode 换了内部结构），已放弃写入以免写坏配置",
            path.display()
        ));
    }
    let config = root
        .get_mut("config")
        .and_then(|v| v.as_object_mut())
        .ok_or_else(|| format!("{} 缺少 config 对象，已放弃写入", path.display()))?;

    // providerRules：先摘掉我们自己的旧规则再追加，避免同一个 provider 出现多条
    let provider_rules = config
        .get_mut("providerConfigRules")
        .and_then(|v| v.get_mut("providerRules"))
        .and_then(|v| v.as_array_mut())
        .ok_or_else(|| {
            format!(
                "{} 的 config.providerConfigRules.providerRules 不是数组，已放弃写入",
                path.display()
            )
        })?;
    provider_rules.retain(|rule| !rules_for_us(rule, provider));

    let api_type = if ctx.chosen_protocol == "anthropic" {
        "anthropic-messages"
    } else {
        "openai-chat-completions"
    };
    provider_rules.push(serde_json::json!({
        "providerId": provider,
        "providerName": "AnyVersion",
        "enabled": true,
        "config": {
            "group": "standard-personal",
            "access": { "type": "api-key", "apiKey": ctx.api_key },
            "api": { "type": api_type, "baseUrl": ctx.base_url },
            "personalModelIds": [model_id],
            "modelOrder": [model_id]
        }
    }));

    // modelConfigRules 下两张表同样要摘掉我们的旧条目
    let model_rules = config
        .get_mut("modelConfigRules")
        .and_then(|v| v.as_object_mut())
        .ok_or_else(|| format!("{} 缺少 config.modelConfigRules，已放弃写入", path.display()))?;
    for key in ["providerModelRules", "manualProviderModelRules"] {
        let rules = model_rules
            .get_mut(key)
            .and_then(|v| v.as_array_mut())
            .ok_or_else(|| {
                format!(
                    "{} 的 config.modelConfigRules.{key} 不是数组，已放弃写入",
                    path.display()
                )
            })?;
        rules.retain(|rule| !rules_for_us(rule, provider));
    }

    // 默认选择整体覆盖（这是「当前用哪个模型」的唯一真值）
    config.insert(
        "defaultModelSelection".to_string(),
        serde_json::json!({ "providerId": provider, "modelId": model_id }),
    );
    // providerOrder 去重后把我们挪到末尾（最近使用）
    if let Some(order) = config.get_mut("providerOrder").and_then(|v| v.as_array_mut()) {
        order.retain(|id| id.as_str() != Some(provider));
        order.push(serde_json::Value::String(provider.to_string()));
    }

    let updated = serde_json::to_string_pretty(&root)
        .map_err(|e| format!("序列化 {} 失败: {e}", path.display()))?;
    if updated == existing {
        return Ok(Vec::new());
    }
    Ok(vec![write_file(&path, &updated)?])
}

/// 还原：把 `provider_config.json` 里属于我们的规则/顺序/默认选择摘掉。
///
/// 只删**认得出是我们的**那些条目（`providerId == 我们的 provider 名`），别人的 provider
/// 原样保留。默认选择若指着我们，直接删掉这个键 —— 我们并不知道用户原本选的是哪个，
/// 瞎猜一个比留空更容易让人误以为设置生效了。
pub(super) fn restore(main_path: &Path) -> Result<RestoreOutcome, String> {
    let mut outcome = RestoreOutcome::default();
    let path = sibling(main_path, "provider_config.json");
    if !path.exists() {
        return Ok(outcome);
    }
    let existing = read_or_empty(&path);
    let Ok(mut root) = serde_json::from_str::<serde_json::Value>(&existing) else {
        outcome
            .notes
            .push(format!("{} 不是合法 JSON，未做还原", path.display()));
        return Ok(outcome);
    };
    let provider = super::provider_for("zcode");
    let Some(config) = root.get_mut("config").and_then(|v| v.as_object_mut()) else {
        return Ok(outcome);
    };

    let mut changed = false;
    if let Some(rules) = config
        .get_mut("providerConfigRules")
        .and_then(|v| v.get_mut("providerRules"))
        .and_then(|v| v.as_array_mut())
    {
        let before = rules.len();
        rules.retain(|rule| !rules_for_us(rule, provider));
        changed |= rules.len() != before;
    }
    if let Some(model_rules) = config.get_mut("modelConfigRules").and_then(|v| v.as_object_mut()) {
        for key in ["providerModelRules", "manualProviderModelRules"] {
            if let Some(rules) = model_rules.get_mut(key).and_then(|v| v.as_array_mut()) {
                let before = rules.len();
                rules.retain(|rule| !rules_for_us(rule, provider));
                changed |= rules.len() != before;
            }
        }
    }
    if let Some(order) = config.get_mut("providerOrder").and_then(|v| v.as_array_mut()) {
        let before = order.len();
        order.retain(|id| id.as_str() != Some(provider));
        changed |= order.len() != before;
    }
    let selection_is_ours = config
        .get("defaultModelSelection")
        .and_then(|v| v.get("providerId"))
        .and_then(|v| v.as_str())
        == Some(provider);
    if selection_is_ours {
        config.remove("defaultModelSelection");
        changed = true;
    }

    if changed {
        let updated = serde_json::to_string_pretty(&root)
            .map_err(|e| format!("序列化 {} 失败: {e}", path.display()))?;
        write_file(&path, &updated)?;
        eprintln!("[extras] zcode: 已从 {} 摘掉我们的 provider 规则", path.display());
        outcome.files.push(path.display().to_string());
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("anyver-zcode-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn ctx<'a>(main: &'a Path, protocol: &'a str) -> ExtrasCtx<'a> {
        static EMPTY: std::sync::OnceLock<std::collections::HashSet<String>> =
            std::sync::OnceLock::new();
        ExtrasCtx {
            tool_id: "zcode",
            main_path: main,
            base_url: "https://api.example.com/v1",
            upstream_url: "https://api.example.com/v1",
            api_key: "sk-test",
            model: "anyversion/glm-5.2",
            model_name: "glm-5.2",
            real_model_name: "glm-5.2",
            provider: "anyversion",
            chosen_protocol: protocol,
            web_search: false,
            user_configured_paths: EMPTY.get_or_init(std::collections::HashSet::new),
        }
    }

    fn valid_skeleton() -> serde_json::Value {
        serde_json::json!({
            "schemaVersion": 1,
            "config": {
                "providerConfigRules": { "providerRules": [
                    { "providerId": "other", "providerName": "Other" }
                ] },
                "modelConfigRules": { "providerModelRules": [], "manualProviderModelRules": [] },
                "defaultModelSelection": { "providerId": "other", "modelId": "old" },
                "providerOrder": ["other"]
            }
        })
    }

    #[test]
    fn missing_file_is_never_created() {
        let dir = temp_dir("missing");
        let main = dir.join("config.json");
        let out = apply(&ctx(&main, "openai")).unwrap();
        assert!(out.is_empty());
        assert!(
            !dir.join("provider_config.json").exists(),
            "不能替 ZCode 造内部配置文件"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn writes_rules_and_default_selection_keeping_other_providers() {
        let dir = temp_dir("write");
        let main = dir.join("config.json");
        std::fs::write(
            dir.join("provider_config.json"),
            serde_json::to_string(&valid_skeleton()).unwrap(),
        )
        .unwrap();

        let out = apply(&ctx(&main, "openai")).unwrap();
        assert_eq!(out.len(), 1, "应写入 provider_config.json");

        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("provider_config.json")).unwrap())
                .unwrap();
        let cfg = &doc["config"];
        // 别人的 provider 规则一条都不能丢
        let rules = cfg["providerConfigRules"]["providerRules"].as_array().unwrap();
        assert_eq!(rules.len(), 2, "{rules:?}");
        assert!(rules.iter().any(|r| r["providerId"] == "other"));
        let ours = rules
            .iter()
            .find(|r| r["providerId"] == "anyversion")
            .expect("应写入我们的规则");
        assert_eq!(ours["enabled"], serde_json::json!(true));
        assert_eq!(ours["config"]["api"]["type"], "openai-chat-completions");
        assert_eq!(ours["config"]["api"]["baseUrl"], "https://api.example.com/v1");
        assert_eq!(ours["config"]["access"]["apiKey"], "sk-test");
        assert_eq!(ours["config"]["personalModelIds"], serde_json::json!(["glm-5.2"]));
        // 默认选择必须被改成我们
        assert_eq!(cfg["defaultModelSelection"]["providerId"], "anyversion");
        assert_eq!(cfg["defaultModelSelection"]["modelId"], "glm-5.2");
        assert_eq!(cfg["providerOrder"], serde_json::json!(["other", "anyversion"]));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn repeated_apply_does_not_accumulate_rules() {
        let dir = temp_dir("idem");
        let main = dir.join("config.json");
        std::fs::write(
            dir.join("provider_config.json"),
            serde_json::to_string(&valid_skeleton()).unwrap(),
        )
        .unwrap();
        apply(&ctx(&main, "openai")).unwrap();
        apply(&ctx(&main, "openai")).unwrap();
        apply(&ctx(&main, "openai")).unwrap();

        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("provider_config.json")).unwrap())
                .unwrap();
        let rules = doc["config"]["providerConfigRules"]["providerRules"]
            .as_array()
            .unwrap();
        assert_eq!(
            rules.iter().filter(|r| r["providerId"] == "anyversion").count(),
            1,
            "重复写入不能累积出多条规则: {rules:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn anthropic_protocol_maps_to_anthropic_messages() {
        let dir = temp_dir("proto");
        let main = dir.join("config.json");
        std::fs::write(
            dir.join("provider_config.json"),
            serde_json::to_string(&valid_skeleton()).unwrap(),
        )
        .unwrap();
        apply(&ctx(&main, "anthropic")).unwrap();
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("provider_config.json")).unwrap())
                .unwrap();
        let rules = doc["config"]["providerConfigRules"]["providerRules"]
            .as_array()
            .unwrap();
        let ours = rules.iter().find(|r| r["providerId"] == "anyversion").unwrap();
        assert_eq!(ours["config"]["api"]["type"], "anthropic-messages");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_schema_version_errors_instead_of_writing() {
        let dir = temp_dir("schema");
        let main = dir.join("config.json");
        let mut skeleton = valid_skeleton();
        skeleton["schemaVersion"] = serde_json::json!(2);
        let original = serde_json::to_string(&skeleton).unwrap();
        std::fs::write(dir.join("provider_config.json"), &original).unwrap();

        let err = apply(&ctx(&main, "openai")).unwrap_err();
        assert!(err.contains("schemaVersion"), "{err}");
        // 文件必须原样保留
        assert_eq!(
            std::fs::read_to_string(dir.join("provider_config.json")).unwrap(),
            original
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_json_errors_and_leaves_file_untouched() {
        let dir = temp_dir("badjson");
        let main = dir.join("config.json");
        std::fs::write(dir.join("provider_config.json"), "{ not json").unwrap();
        let err = apply(&ctx(&main, "openai")).unwrap_err();
        assert!(err.contains("不是合法 JSON"), "{err}");
        assert_eq!(
            std::fs::read_to_string(dir.join("provider_config.json")).unwrap(),
            "{ not json"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
