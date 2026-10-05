//! Codex 模型目录（`model_catalog_json`）生成：按**上游域名**匹配厂商能力模板，
//! 把用户选中的模型身份戳进模板，产出一份单条目 `{"models":[...]}`。
//!
//! ## 为什么需要
//!
//! Codex 直连第三方 Responses 端点时只会套用自己的默认目录 —— 它不知道这个模型真实
//! 有多大的窗口、支持哪些 reasoning 档位、注册了哪些工具，于是**误判上下文窗口、
//! 工具注册不上**。写一份 catalog 并让 `model_catalog_json` 指向它，才是正确的告知方式。
//!
//! ## 模板不是「模型清单」
//!
//! `assets/codex-catalogs/*.json` 每份是**一个厂商的能力模板**，装的是与具体模型无关的
//! 字段（`base_instructions` 提示框架、`apply_patch_tool_type`、`web_search_tool_type`、
//! `supported_reasoning_levels`、`truncation_policy`、`input_modalities`…）。模型身份
//! （slug / display_name / context_window / priority）由 [`build_catalog`] 现场戳上去。
//! 未知模型版本套保守的纯文本默认值仍然可用；图像输入只对厂商文档里明确支持的 id 开启。
//!
//! **匹配只看域名、不看模型品牌** —— 转售商未必实现与品牌方相同的能力。
//!
//! 未收录的厂商保持原行为：不写 `model_catalog_json`，Codex 用自己的默认目录。
//!
//! 资产与 EchoBird 同源；DeepSeek 那份的 `base_instructions` 是从 DeepSeek 官方配置
//! 脚本中原样提取的 Codex agent 提示框架。

use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// DeepSeek 的 Codex 能力模板。
const DEEPSEEK_TEMPLATE: &str = include_str!("../../../assets/codex-catalogs/deepseek.json");
/// MiniMax 的 Codex 能力模板（自适应思考、1M 窗口、文本 + 图像）。
const MINIMAX_TEMPLATE: &str = include_str!("../../../assets/codex-catalogs/minimax.json");
/// 小米 MiMo 的 Codex 能力模板（1M 窗口、**没有** web_search 工具 —— MiMo 对它硬报 400）。
const MIMO_TEMPLATE: &str = include_str!("../../../assets/codex-catalogs/mimo.json");

/// 从 URL 里取 host（小写、去尾部点、去 userinfo 与端口）。解析失败返回 None。
///
/// 不引 `url` crate：这里只需要 host，且上游地址都是普通 `scheme://host[:port]/path`。
/// IPv6 字面量（`[::1]:8080`）不是第三方厂商域名，不做特殊处理。
fn host_of(url: &str) -> Option<String> {
    let rest = url.trim();
    let rest = rest.split_once("://").map(|(_, r)| r).unwrap_or(rest);
    let authority = rest.split(['/', '?', '#']).next()?;
    let host_port = authority
        .rsplit_once('@')
        .map(|(_, h)| h)
        .unwrap_or(authority);
    let host = host_port.split(':').next()?.trim_end_matches('.').to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

/// `base_url` 是否属于 `domain`（含子域）。**只看域名**，不看路径与模型品牌。
pub fn url_matches_domain(base_url: &str, domain: &str) -> bool {
    let Some(host) = host_of(base_url) else {
        return false;
    };
    let domain = domain.trim_end_matches('.').to_ascii_lowercase();
    host == domain || host.ends_with(&format!(".{domain}"))
}

/// 按 `base_url` 域名选厂商能力模板；未收录的厂商返回 `None`（保持无 catalog 的原行为）。
pub fn template_for_url(base_url: &str) -> Option<&'static str> {
    if url_matches_domain(base_url, "deepseek.com") {
        Some(DEEPSEEK_TEMPLATE)
    } else if url_matches_domain(base_url, "minimax.cn")
        || url_matches_domain(base_url, "minimaxi.com")
        || url_matches_domain(base_url, "minimax.io")
    {
        Some(MINIMAX_TEMPLATE)
    } else if url_matches_domain(base_url, "xiaomimimo.com") {
        Some(MIMO_TEMPLATE)
    } else {
        None
    }
}

/// 用厂商模板 + 选中模型的身份，生成单条目 catalog。
///
/// 模板里与模型无关的字段（提示框架、工具类型、reasoning 档位、截断策略）原样保留，
/// 只戳入身份与窗口大小。
///
/// `slug` 与 `real_model_id` **刻意分开**：
/// - `slug` = 写进 `config.toml` 的 `model`（声明名 C）。Codex 按 `model` 找条目，
///   这里必须用声明名，否则伪装时条目匹配不上、Codex 回落默认模型；
/// - `real_model_id` = 真实模型 B 的 id。图像/搜索能力是**厂商逐模型**的，只能按它判
///   —— 拿 `gpt-5.1-codex` 这种伪装名去判，会把收图的 DeepSeek Flash 判成不收图。
pub fn build_catalog(
    template: &Value,
    slug: &str,
    real_model_id: &str,
    display_name: &str,
    context_window: u64,
) -> Value {
    let mut entry = template.clone();
    entry["slug"] = json!(slug);
    entry["display_name"] = json!(display_name);
    entry["description"] = json!(format!("{display_name} via AnyVersion"));
    entry["context_window"] = json!(context_window);
    entry["max_context_window"] = json!(context_window);
    entry["priority"] = json!(0);

    // 图像能力是**逐模型**的：DeepSeek Flash 与 MiMo v2.5 收图，DeepSeek V4 Pro 与
    // MiMo v2.5 Pro 只收文本。模板保持保守默认，只对厂商文档明确支持的 id 开启，
    // 免得给只收文本的模型声明成能收图（那会让 Codex 直接发图过去报错）。
    let supports_image = matches!(real_model_id, "deepseek-flash" | "mimo-v2.5");
    if real_model_id.starts_with("deepseek-") || real_model_id.starts_with("mimo-") {
        entry["input_modalities"] = if supports_image {
            json!(["text", "image"])
        } else {
            json!(["text"])
        };
        entry["supports_image_detail_original"] = json!(supports_image);
    }
    if real_model_id.starts_with("deepseek-") {
        entry["supports_search_tool"] = json!(real_model_id == "deepseek-flash");
    }
    // MiniMax 的 base_instructions 里带 `{model}` 占位符，替换成实际显示名，
    // 让提示词里出现的模型名是真的；DeepSeek 的提示框架与模型无关，不受影响。
    if let Some(bi) = entry["base_instructions"].as_str() {
        if bi.contains("{model}") {
            entry["base_instructions"] = json!(bi.replace("{model}", display_name));
        }
    }
    json!({ "models": [entry] })
}

/// catalog 文件的绝对路径 = **与 `config.toml` 同目录**。
///
/// 刻意**不用 `get_home_dir()`**：
/// 1. 工具的配置目录未必是 `~/.codex` —— `CODEX_HOME` 可以把它指到别处，
///    写死 home 就会把目录写到 Codex 根本不会读的地方；
/// 2. 写死 home 还会让**测试直接落到真实用户目录**（真机上验证过：一次 `cargo test`
///    就往真实的 `~/.codex/models.json` 写了一份 38KB 的目录文件）。
///
/// 跟着声明里的配置路径走，既是对的（Codex 读的就是配置旁边那份），也天然可测。
///
/// **统一用正斜杠**：这个值要写进 `config.toml` 的基本字符串，Windows 的反斜杠需要
/// TOML 转义，正斜杠不用。
pub fn models_json_path_for(config_dir: &Path) -> PathBuf {
    PathBuf::from(
        config_dir
            .join("models.json")
            .to_string_lossy()
            .replace('\\', "/"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_for_url_matches_deepseek_domain() {
        assert_eq!(
            template_for_url("https://api.deepseek.com/v1"),
            Some(DEEPSEEK_TEMPLATE)
        );
        assert_eq!(
            template_for_url("https://api.deepseek.com"),
            Some(DEEPSEEK_TEMPLATE)
        );
        // 后缀撞名不能误命中（notdeepseek.com 不是 deepseek.com 的子域）
        assert_eq!(template_for_url("https://notdeepseek.com/v1"), None);
        // 子域要命中
        assert_eq!(
            template_for_url("https://api.cn.deepseek.com/v1"),
            Some(DEEPSEEK_TEMPLATE)
        );
    }

    #[test]
    fn template_for_url_matches_minimax_domains() {
        for url in [
            "https://api.minimax.cn/v1",
            "https://api.minimaxi.com/v1",
            "https://api.minimax.io/v1",
        ] {
            assert_eq!(template_for_url(url), Some(MINIMAX_TEMPLATE), "{url}");
        }
    }

    #[test]
    fn template_for_url_matches_mimo_domains() {
        // Token 套餐的区域端点共用同一域名
        assert_eq!(
            template_for_url("https://token-plan-cn.xiaomimimo.com/v1"),
            Some(MIMO_TEMPLATE)
        );
        assert_eq!(template_for_url("https://notxiaomimimo.com/v1"), None);
    }

    #[test]
    fn template_for_url_returns_none_for_unbundled_vendors() {
        for url in [
            "https://ark.cn-beijing.volces.com/api/coding/v1",
            "https://api.openai.com/v1",
            "https://api.moonshot.cn/v1",
            // 本地代理：域名是回环地址，看不出上游是谁 → 不能瞎套模板
            "http://127.0.0.1:8080/v1",
        ] {
            assert_eq!(template_for_url(url), None, "{url}");
        }
    }

    #[test]
    fn bundled_templates_parse_and_are_not_model_lists() {
        for template in [DEEPSEEK_TEMPLATE, MINIMAX_TEMPLATE, MIMO_TEMPLATE] {
            let v: Value = serde_json::from_str(template).expect("模板必须可解析");
            assert!(
                v.get("base_instructions").and_then(|x| x.as_str()).is_some(),
                "模板必须带 base_instructions"
            );
            assert!(
                v.get("models").is_none(),
                "模板不能是模型清单 —— 身份由 build_catalog 现场戳"
            );
        }
    }

    #[test]
    fn build_catalog_stamps_identity_and_window() {
        let tpl: Value = serde_json::from_str(DEEPSEEK_TEMPLATE).unwrap();
        let catalog = build_catalog(
            &tpl,
            "deepseek-v4-pro",
            "deepseek-v4-pro",
            "DeepSeek V4 Pro",
            204_800,
        );
        let entry = &catalog["models"][0];
        assert_eq!(entry["slug"], "deepseek-v4-pro");
        assert_eq!(entry["display_name"], "DeepSeek V4 Pro");
        assert_eq!(entry["context_window"].as_u64(), Some(204_800));
        assert_eq!(entry["max_context_window"].as_u64(), Some(204_800));
        // 只收文本的 deepseek 模型不能声明图像能力
        assert_eq!(entry["input_modalities"], json!(["text"]));
        assert_eq!(entry["supports_search_tool"], json!(false));
        assert_eq!(catalog["models"].as_array().unwrap().len(), 1);
    }

    /// 伪装生效时：`slug` 是**声明名**（Codex 按 `config.toml` 的 `model` 找条目），
    /// 而图像 / 搜索能力仍按**真实模型**判。否则一个收图的 DeepSeek Flash 被伪装成
    /// `gpt-5.1-codex` 后就会被判成「只收文本」，Codex 直接把图丢掉。
    #[test]
    fn build_catalog_uses_claimed_slug_but_real_capabilities() {
        let tpl: Value = serde_json::from_str(DEEPSEEK_TEMPLATE).unwrap();
        let c = build_catalog(
            &tpl,
            "gpt-5.1-codex",
            "deepseek-flash",
            "DeepSeek Flash",
            1_000_000,
        );
        let entry = &c["models"][0];
        assert_eq!(entry["slug"], "gpt-5.1-codex", "slug 必须是声明名，否则 Codex 匹配不上");
        assert_eq!(entry["display_name"], "DeepSeek Flash");
        assert_eq!(entry["input_modalities"], json!(["text", "image"]));
        assert_eq!(entry["supports_search_tool"], json!(true));
    }

    #[test]
    fn build_catalog_opts_in_documented_image_models() {
        let tpl: Value = serde_json::from_str(DEEPSEEK_TEMPLATE).unwrap();
        let flash = build_catalog(
            &tpl,
            "deepseek-flash",
            "deepseek-flash",
            "DeepSeek Flash",
            1_000_000,
        );
        assert_eq!(flash["models"][0]["input_modalities"], json!(["text", "image"]));
        assert_eq!(flash["models"][0]["supports_search_tool"], json!(true));

        let mimo_tpl: Value = serde_json::from_str(MIMO_TEMPLATE).unwrap();
        let mimo = build_catalog(
            &mimo_tpl,
            "mimo-v2.5",
            "mimo-v2.5",
            "MiMo v2.5",
            1_000_000,
        );
        assert_eq!(mimo["models"][0]["input_modalities"], json!(["text", "image"]));
    }

    #[test]
    fn build_catalog_substitutes_model_placeholder() {
        let tpl: Value = serde_json::from_str(MINIMAX_TEMPLATE).unwrap();
        assert!(
            MINIMAX_TEMPLATE.contains("{model}"),
            "前提：MiniMax 模板带占位符"
        );
        let catalog = build_catalog(
            &tpl,
            "MiniMax-M3",
            "MiniMax-M3",
            "MiniMax M3",
            1_000_000,
        );
        let bi = catalog["models"][0]["base_instructions"].as_str().unwrap();
        assert!(!bi.contains("{model}"), "占位符必须被替换掉");
        assert!(bi.contains("MiniMax M3"), "应替换成实际显示名: {bi}");
    }

    /// catalog 必须落在**声明的配置目录**里，而不是写死的 `~/.codex`。
    ///
    /// 这条是拿真实事故换来的：早先版本用 `get_home_dir()`，结果一次 `cargo test` 就往
    /// 真实用户目录写了一份 38KB 的 catalog；而且 `CODEX_HOME` 被指到别处时会写错位置。
    #[test]
    fn catalog_sits_next_to_the_declared_config() {
        let dir = Path::new("/tmp/custom-codex-home");
        let p = models_json_path_for(dir);
        let s = p.to_string_lossy();
        assert!(s.starts_with("/tmp/custom-codex-home"), "{s}");
        assert!(!s.contains('\\'), "Windows 反斜杠必须换成正斜杠: {s}");
        assert!(s.ends_with("/models.json"), "{s}");
    }

    #[test]
    fn host_of_handles_ports_userinfo_and_case() {
        assert_eq!(host_of("https://API.DeepSeek.com:443/v1"), Some("api.deepseek.com".into()));
        assert_eq!(host_of("https://u:p@api.deepseek.com/v1"), Some("api.deepseek.com".into()));
        assert_eq!(host_of("api.deepseek.com"), Some("api.deepseek.com".into()));
        assert_eq!(host_of("https://api.deepseek.com./v1"), Some("api.deepseek.com".into()));
        assert_eq!(host_of(""), None);
    }
}
