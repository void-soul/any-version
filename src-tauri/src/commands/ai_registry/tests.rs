use super::{registry, tool_kind_of, ProviderPreset, ProviderPresetDto};

/// dsh 已从 **AI 工具** 里移除（2026-10-04）。
///
/// 它仍然作为 **Node 服务**存在（`node-projects/harness.json` + `node_manager.rs`），
/// 那部分不受影响 —— 这里只保证它不会再出现在 AI 工具列表里。
/// 删掉 `ai-tools/dsh/` 就够了：工具注册表是配置驱动的，目录没了自然读不到。
#[test]
fn dsh_is_not_an_ai_tool_anymore() {
    assert!(
        registry().get_tool("dsh").is_none(),
        "dsh 不该再注册为 AI 工具（ai-tools/dsh/ 是否被加回来了？）"
    );
}

/// 回归：DTO 一旦被标上 `rename_all = "camelCase"`，URL 会序列化成
/// `openaiUrl`，前端按 `openai_url` 读取全部落空 → 添加预设只剩名称/官网。
#[test]
fn desktop_tools_are_registered_and_classified_as_desktop() {
    // 从 EchoBird 移植进来的桌面端工具（category=Desktop）：
    // 注册表读得到、且被归到 desktop —— 否则工具列表的「桌面端」分组永远是空的。
    let reg = registry();
    for id in [
        "claudedesktop",
        "chatgptdesktop",
        "mimodesktop",
        "kimidesktop",
        "opencodedesktop",
        "openscience",
        "zcode",
    ] {
        let Some((config, paths)) = reg.get_tool(id) else {
            panic!(
                "注册表里没有桌面端工具 {}（ai-tools/{}/ 是否完整？）",
                id, id
            );
        };
        assert_eq!(
            tool_kind_of(Some(&paths.category)),
            "desktop",
            "{} 的形态是 {}，没被归到桌面端",
            id,
            paths.category
        );
        // 声明了 configFile 才允许显示「设置模型」入口，两者必须一致
        assert_eq!(
            config.config_file.is_some(),
            config.support_model,
            "{} 的 configFile 与 supportModel 不一致（会成为点了没效果的假入口）",
            id
        );
    }
}

/// **数据契约**：两个桌面端的 `builtinModels` 就是它们伪装名的**唯一来源**。
///
/// 用户没填伪装名时，`tool_config_custom::effective_claimed_model` 会拿清单第一项
/// 当兜底声明名（写死的兜底名迟早变成已废弃型号，而用户看不出异常）。所以：
///
/// 1. 清单不能空 —— 空了这两个工具就退化成「不伪装」，等于功能没上线；
/// 2. 每一项都必须**通过该工具的合法性校验** —— 不合法的项会被跳过，
///    排在它后面的型号才会被选中，清单就白维护了；
/// 3. 清单按「新 → 旧」排序（第一项 = 当前在售最新），退役的型号要删掉。
#[test]
fn desktop_masquerade_lists_are_usable_as_fallback_source() {
    use crate::commands::ai::tool_config_custom::{
        is_legal_claudedesktop_model, is_legal_openai_model,
    };

    let reg = registry();
    for (id, is_legal) in [
        (
            "claudedesktop",
            is_legal_claudedesktop_model as fn(&str) -> bool,
        ),
        ("chatgptdesktop", is_legal_openai_model as fn(&str) -> bool),
    ] {
        let cfg = reg
            .get_tool_config(id)
            .unwrap_or_else(|| panic!("注册表里没有 {id}"));
        assert!(
            !cfg.builtin_models.is_empty(),
            "{id} 的 builtinModels 是空的：它在用户没填伪装名时拿不到兜底声明名，\
                 自动伪装等于没上线（ai-tools/{id}/config.json）"
        );
        for (i, name) in cfg.builtin_models.iter().enumerate() {
            assert!(
                is_legal(name),
                "{id} 的 builtinModels[{i}] = `{name}` 会被该工具的模型名校验剔除，\
                     兜底时会跳过它 —— 清单里不该留这种项"
            );
        }
    }
}

/// 回归：只要工具支持「设置模型」，就必须同时声明支持优化器 / 整流器。
///
/// 这两组能力是代理侧的协议层能力（cache 注入、thinking 自适应、报错后改写重试…），
/// 与工具是 CLI 还是桌面端无关 —— 通过 Kira 启动时写进工具配置的就是本地代理地址，
/// 流量必然过代理。早先桌面端一律 `false`（保守默认），导致「桌面端选了模型
/// 却看不到这两组开关」，看起来像能力缺失。
#[test]
fn model_capable_tools_allow_optimizer_and_rectifier() {
    let reg = registry();
    for (id, (config, _)) in reg.tool_iter() {
        if !config.support_model {
            continue;
        }
        assert!(
                config.supports_optimizer && config.supports_rectifier,
                "{} 支持设置模型（流量走本地代理），却声明 supports_optimizer={} / supports_rectifier={}\
                 → 启动页看不到「请求优化器 / 协议整流器」，请检查 ai-tools/{}/config.json",
                id,
                config.supports_optimizer,
                config.supports_rectifier,
                id,
            );
    }
}

/// 每个工具目录都必须能加载进注册表。
///
/// 回归的是「少写一个字段 → 整份 config.json 解析失败 → 工具被静默丢掉」：
/// omp 就因为没有 `cacheDirs` 一直没出现在列表里，只在 stderr 留了一行 parse 失败。
#[test]
fn every_tool_directory_loads_into_the_registry() {
    let dir = super::AiToolRegistry::find_registry_dir();
    let mut dirs: Vec<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("读不到 ai-tools 目录 {}: {e}", dir.display()))
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    dirs.sort();
    assert!(!dirs.is_empty(), "{} 下没有工具目录", dir.display());

    let reg = registry();
    let missing: Vec<String> = dirs
        .iter()
        .filter(|id| reg.get_tool_config(id).is_none())
        .cloned()
        .collect();
    assert!(
        missing.is_empty(),
        "这些工具目录没能加载（config.json 多半缺字段，解析失败会被整个丢掉）: {missing:?}"
    );
}

#[test]
fn tool_kind_normalizes_cli_and_desktop_categories() {
    // 列表要按「CLI / 桌面端」分组（抄 EchoBird 的维度）：
    // EchoBird 的 category 取值更细，这里归一层，前端只认 cli/desktop/other
    assert_eq!(tool_kind_of(Some("CLI Code")), "cli");
    assert_eq!(tool_kind_of(Some("cli code")), "cli");
    assert_eq!(tool_kind_of(Some("Desktop")), "desktop");
    assert_eq!(tool_kind_of(Some("IDE")), "desktop");
    // 认不出的分类不会让分组崩掉，只是落到 other
    assert_eq!(tool_kind_of(None), "other");
    assert_eq!(tool_kind_of(Some("")), "other");
    assert_eq!(tool_kind_of(Some("Science")), "other");
}

#[test]
fn provider_preset_dto_serializes_urls_as_snake_case() {
    let dto = ProviderPresetDto {
        id: "workbuddy2api".into(),
        name: "WorkBuddy2API 本地转换".into(),
        category: "relay".into(),
        website: "https://example.com".into(),
        openai_url: "http://127.0.0.1:8788/v1".into(),
        anthropic_url: "http://127.0.0.1:8788".into(),
        google_url: String::new(),
    };
    let value = serde_json::to_value(&dto).expect("序列化失败");
    assert!(
        value.get("openai_url").is_some(),
        "缺少 openai_url 键: {value}"
    );
    assert!(
        value.get("anthropic_url").is_some(),
        "缺少 anthropic_url 键: {value}"
    );
    assert!(
        value.get("google_url").is_some(),
        "缺少 google_url 键: {value}"
    );
    assert!(value.get("openaiUrl").is_none(), "不应输出驼峰键: {value}");
}

/// providers.json 健全性：id 唯一、name 非空、category 仅取已知值、
/// 至少一个协议端点非空。新增预设时防止手误破坏前端过滤与展示。
#[test]
fn providers_json_entries_are_valid_and_unique() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../ai-tools/providers.json");
    let raw = std::fs::read_to_string(&path).expect("providers.json 读取失败");
    let presets: Vec<ProviderPreset> = serde_json::from_str(&raw).expect("providers.json 解析失败");
    assert!(presets.len() >= 40, "预设数量异常: {}", presets.len());

    let mut seen = std::collections::HashSet::new();
    for p in &presets {
        assert!(!seen.contains(&p.id), "重复预设 id: {}", p.id);
        seen.insert(p.id.clone());
        assert!(!p.name.trim().is_empty(), "预设 {} 名称为空", p.id);
        assert!(
            matches!(p.category.as_str(), "provider" | "relay" | "local"),
            "预设 {} 分类未知: {}（前端过滤只认 provider/relay/local）",
            p.id,
            p.category
        );
        assert!(
            !p.openai_url.is_empty() || !p.anthropic_url.is_empty() || !p.google_url.is_empty(),
            "预设 {} 没有任何协议端点",
            p.id
        );
        for url in [&p.openai_url, &p.anthropic_url, &p.google_url] {
            if !url.is_empty() {
                assert!(
                    url.starts_with("http://") || url.starts_with("https://"),
                    "预设 {} 端点不是 http(s) URL: {}",
                    p.id,
                    url
                );
            }
        }
    }
}
