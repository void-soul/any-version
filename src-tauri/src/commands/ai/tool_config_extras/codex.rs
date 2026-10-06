//! Codex（`~/.codex/config.toml`）的附加写入。
//!
//! 通用映射已经把 `model` / `model_provider` / `base_url` / 凭据写好了，这里补的是
//! 「平铺路径说不清」的那几项：
//!
//! - `model_context_window` 与 `model_auto_compact_token_limit`：**必须按模型的真实窗口**
//!   写。Codex 默认按 1M 算，直连一个只有 204,800 窗口的模型时它会在窗口早就满了之后
//!   才去压缩 —— 表现是上游直接报超长。
//! - `model_reasoning_effort`：常量 `high`。
//! - `web_search`：**按上游域名**决定。DeepSeek / MiMo 没有可用的联网检索，写 `live`
//!   只会让 Codex 发一个必然被拒的请求。
//! - `model_catalog_json`：按域名给厂家写一份模型目录（见 `codex_catalog`），
//!   否则 Codex 不知道这个模型的窗口、reasoning 档位与工具集。
//! - 清理一批**历史遗留键**（`review_model` / `disable_response_storage` 等）：写入侧是
//!   语义合并、只替换受管键，旧键会一直留在文件里继续生效，不删就会「改了配置但行为没变」。
//!
//! 对照 EchoBird `services/tool_config_manager/codex.rs::write_codex_canonical_fields`。

use std::path::Path;

use super::{read_or_empty, write_file, ExtrasCtx};
use crate::commands::ai::tool_config_restore::RestoreOutcome;

/// 未收录模型的兜底窗口 —— Codex 自己历史上的默认值，保持未知模型「和以前一样能用」。
const DEFAULT_CODEX_CONTEXT_WINDOW: u64 = 1_000_000;

/// 按模型名查真实上下文窗口（token）。
///
/// 数据驱动：这里只放厂商官方规格里查得到的值，**不在写入逻辑里按模型名分支**。
/// 未收录的一律用 [`DEFAULT_CODEX_CONTEXT_WINDOW`]。
fn model_context_window_for(model_id: &str) -> u64 {
    match model_id.trim().to_ascii_lowercase().as_str() {
        "minimax-m3" => 1_000_000,
        "minimax-m2.7" => 204_800,
        _ => DEFAULT_CODEX_CONTEXT_WINDOW,
    }
}

/// 自动压缩阈值 = 窗口的 90%。
///
/// 与窗口成比例，才不会出现「在只有 204,800 窗口的模型上等到 900k 才压缩」。
fn compact_limit_for(context_window: u64) -> u64 {
    context_window * 9 / 10
}

/// `web_search` 该写什么。
///
/// - DeepSeek / MiMo：**永远 `disabled`** —— 它们没有可用的联网检索，写 `live` 会让 Codex
///   发一个必然被拒的请求（判定只看域名，不看模型品牌：转售商未必实现同样能力）。
/// - 其余域名：跟随用户的联网搜索开关；关的时候**显式写回 Codex 自己的默认值 `cached`**，
///   顺带清掉上一次可能残留的 `live`。
fn web_search_mode(toggle_on: bool, base_url: &str) -> &'static str {
    use crate::commands::ai::codex_catalog::url_matches_domain;
    if url_matches_domain(base_url, "deepseek.com")
        || url_matches_domain(base_url, "xiaomimimo.com")
    {
        "disabled"
    } else if toggle_on {
        "live"
    } else {
        "cached"
    }
}

/// 去掉模型名尾部的上下文档位后缀（`MiniMax-M3[1m]` → `MiniMax-M3`）—— 查表要的是裸 id。
fn base_model_id(model_name: &str) -> String {
    let trimmed = model_name.trim();
    match trimmed.rfind('[') {
        Some(idx) if trimmed.ends_with(']') => trimmed[..idx].to_string(),
        _ => trimmed.to_string(),
    }
}

// ─── toml_edit 小工具 ───

/// 按路径设值，中间层级不存在就建表（递归绕开 `&mut` 借用的重借问题）。
fn set_item(cur: &mut toml_edit::Item, path: &[&str], item: toml_edit::Item) {
    let Some((head, tail)) = path.split_first() else {
        *cur = item;
        return;
    };
    if !cur.is_table() {
        *cur = toml_edit::Item::Table(toml_edit::Table::new());
    }
    let table = cur.as_table_mut().expect("just ensured a table");
    let entry = table
        .entry(head)
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
    set_item(entry, tail, item);
}

fn set_str(doc: &mut toml_edit::DocumentMut, path: &[&str], value: &str) {
    set_item(doc.as_item_mut(), path, toml_edit::value(value));
}

fn set_int(doc: &mut toml_edit::DocumentMut, path: &[&str], value: i64) {
    set_item(doc.as_item_mut(), path, toml_edit::value(value));
}

fn set_bool(doc: &mut toml_edit::DocumentMut, path: &[&str], value: bool) {
    set_item(doc.as_item_mut(), path, toml_edit::value(value));
}

/// 删顶层键；键本来不在就什么都不做。
fn delete_top(doc: &mut toml_edit::DocumentMut, key: &str) {
    doc.as_table_mut().remove(key);
}

/// 现有配置里的 `model_catalog_json` 是否指向**我们**的 catalog 文件。
///
/// 用于「离开受支持厂商」时决定要不要顺手删掉那份文件。**精确比对路径**，不是子串匹配：
/// 用户自己的 catalog（比如按厂商文档放在 `~/.codex/model-catalogs/xxx.json`）绝不能被我们删掉。
fn catalog_referenced(existing: &str, our_path: &Path) -> bool {
    let doc = crate::commands::ai::launch::parse_toml_lenient(existing.trim_start_matches('\u{feff}'));
    let Some(value) = doc.get("model_catalog_json").and_then(|item| item.as_str()) else {
        return false;
    };
    let value = value.trim();
    !value.is_empty()
        && (value == our_path.to_string_lossy() || value == "~/.codex/models.json")
}

/// catalog 该放哪：**与 `config.toml` 同目录**（见 [`codex_catalog::models_json_path_for`]）。
fn catalog_path_for(main_path: &Path) -> std::path::PathBuf {
    let dir = main_path.parent().unwrap_or(Path::new("."));
    crate::commands::ai::codex_catalog::models_json_path_for(dir)
}

pub(super) fn apply(ctx: &ExtrasCtx<'_>) -> Result<Vec<String>, String> {
    let path = ctx.main_path;
    let existing = read_or_empty(path);
    if existing.trim().is_empty() {
        // 主配置没被通用写入建出来（键为空 / 文件不存在）→ 不该在这里凭空造一个
        return Ok(Vec::new());
    }

    let mut doc =
        crate::commands::ai::launch::parse_toml_lenient(existing.trim_start_matches('\u{feff}'));

    let model_id = base_model_id(ctx.model_name);
    // 窗口与图像能力**按真实模型 B 判定**：`model_id` 是声明名 C，伪装生效时它是官方名
    // （`gpt-5.1-codex`），查表必然落空 → 204,800 的窗口被写成 1,000,000，Codex 于是
    // 永远等不到压缩，上游直接报超长。
    let real_id = base_model_id(ctx.real_model_name);
    let window = model_context_window_for(&real_id);

    // 先清遗留键，再写规范值 —— 顺序反了的话「清理」会把刚写的那行删掉
    delete_top(&mut doc, "review_model");
    delete_top(&mut doc, "model_catalog_json");
    delete_top(&mut doc, "disable_response_storage");
    // MiMo 专属的一对键先删掉，避免从 MiMo 切走后它的模型专属设置泄漏到别的厂商上
    delete_top(&mut doc, "model_supports_reasoning_summaries");
    delete_top(&mut doc, "model_reasoning_summary");

    // 用户通过「模型自定义参数」显式声明了这些 config 键时，这里**让位**：写死会覆盖
    // 用户的选择（「界面上填了、启动时被我们改回去」= 摆设）。见 `ExtrasCtx::user_configured_paths`。
    // 注意关联关系：窗口与压缩上限原本按真实模型查表成对写；用户只覆盖其中一个时，
    // 另一个仍按查表值 —— 这是用户自己的取舍，不替用户猜。
    if !ctx.user_configured_paths.contains("model_reasoning_effort") {
        set_str(&mut doc, &["model_reasoning_effort"], "high");
    }
    if !ctx.user_configured_paths.contains("model_context_window") {
        set_int(&mut doc, &["model_context_window"], window as i64);
    }
    if !ctx.user_configured_paths.contains("model_auto_compact_token_limit") {
        set_int(
            &mut doc,
            &["model_auto_compact_token_limit"],
            compact_limit_for(window) as i64,
        );
    }
    if !ctx.user_configured_paths.contains("web_search") {
        set_str(
            &mut doc,
            &["web_search"],
            web_search_mode(ctx.web_search, ctx.vendor_url()),
        );
    }

    // **刻意不动 `model_providers.<p>` 表**（`name` / `base_url` / `env_key` / `wire_api`）。
    // EchoBird 会写 `wire_api = "responses"` + `requires_openai_auth = true`（它走的是
    // Responses 直连）；我们的链路是「工具 → 本地代理 → 上游」，协议由代理转换，
    // 照抄会改掉当前可用的出站协议与鉴权方式。这几项由声明里的 write 映射负责。

    // MiMo 官方要求这两个顶层开关都在，`model_reasoning_effort` 才生效
    if crate::commands::ai::codex_catalog::url_matches_domain(ctx.vendor_url(), "xiaomimimo.com") {
        set_bool(&mut doc, &["model_supports_reasoning_summaries"], true);
        set_str(&mut doc, &["model_reasoning_summary"], "none");
    }

    let mut touched: Vec<String> = Vec::new();
    let catalog_path = catalog_path_for(path);
    let mut catalog_written = false;
    match crate::commands::ai::codex_catalog::template_for_url(ctx.vendor_url()) {
        Some(template_str) => {
            let template: serde_json::Value = serde_json::from_str(template_str)
                .map_err(|e| format!("内置 Codex catalog 模板非法（编译期资产被改坏？）: {e}"))?;
            let catalog = crate::commands::ai::codex_catalog::build_catalog(
                &template,
                &model_id,
                &real_id,
                ctx.real_model_name,
                window,
            );
            // **先确认文件写成功再加这一行**：`model_catalog_json` 指向一个不存在的文件
            // 会让 Codex 启动即报错。
            let text = serde_json::to_string_pretty(&catalog)
                .map_err(|e| format!("序列化 Codex catalog 失败: {e}"))?;
            write_file(&catalog_path, &text)?;
            catalog_written = true;
            set_str(
                &mut doc,
                &["model_catalog_json"],
                &catalog_path.to_string_lossy(),
            );
        }
        None => {
            // 从受支持厂商切到不受支持的厂商：上面的 delete 已经让配置不再引用 catalog，
            // 这里的文件也一并清掉，别在用户磁盘上留垃圾。**只删我们自己那一个路径**，
            // 且只在旧配置确实指向它时才删（用户自己的 catalog 不能碰）。
            if catalog_referenced(&existing, &catalog_path) && catalog_path.exists() {
                match std::fs::remove_file(&catalog_path) {
                    Ok(()) => touched.push(catalog_path.display().to_string()),
                    Err(e) => eprintln!(
                        "[extras] 删除过期 Codex catalog {} 失败（忽略）: {e}",
                        catalog_path.display()
                    ),
                }
            }
        }
    }

    let updated = doc.to_string();
    if updated != existing {
        // **部分写入保护**：catalog 文件是先写的，若这时配置写失败（真机上遇到过：
        // `config.toml` 被正在运行的 Codex 占住），就会留下一个没人引用的 catalog 文件。
        // 配置没写成就把刚生成的那份撤掉，磁盘状态与「什么都没做」一致。
        if let Err(e) = write_file(path, &updated) {
            if catalog_written {
                match std::fs::remove_file(&catalog_path) {
                    Ok(()) => eprintln!(
                        "[extras] 配置写入失败，已撤回刚生成的 catalog {}",
                        catalog_path.display()
                    ),
                    Err(cleanup) => eprintln!(
                        "[extras] 配置写入失败，且撤回 catalog {} 也失败（残留无害，未被引用）: {cleanup}",
                        catalog_path.display()
                    ),
                }
            }
            return Err(e);
        }
        touched.push(path.display().to_string());
    }
    Ok(touched)
}

/// 还原：删掉**我们生成的** Codex 模型目录文件。
///
/// `config.toml` 里的 `model_catalog_json` 那一行由通用还原按 `removeKeys` 删掉，
/// 但那份 JSON 文件是本模块生成的，得在这里清。
///
/// **只删我们自己那份**：认领标记是 `description` 里的 `via AnyVersion`
/// （`build_catalog` 写的）。同样是 `models.json` 这个名字，用户自己按厂商文档
/// 放了一份的话，一个字节都不能碰。
pub(super) fn restore(main_path: &Path) -> Result<RestoreOutcome, String> {
    let mut outcome = RestoreOutcome::default();
    let catalog = catalog_path_for(main_path);
    if !catalog.exists() {
        return Ok(outcome);
    }
    let text = read_or_empty(&catalog);
    if !text.contains("via AnyVersion") {
        outcome.notes.push(format!(
            "{} 不是我们生成的模型目录，保留不动",
            catalog.display()
        ));
        return Ok(outcome);
    }
    std::fs::remove_file(&catalog)
        .map_err(|e| format!("删除 {} 失败: {e}", catalog.display()))?;
    eprintln!("[extras] codex: 已删除我们生成的模型目录 {}", catalog.display());
    outcome.files.push(catalog.display().to_string());
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_window_lookup_is_case_insensitive_and_data_driven() {
        assert_eq!(model_context_window_for("MiniMax-M3"), 1_000_000);
        assert_eq!(model_context_window_for("minimax-m3"), 1_000_000);
        assert_eq!(model_context_window_for("  MiniMax-M2.7 "), 204_800);
        // 未收录 → Codex 历史默认，保持「和以前一样能用」
        assert_eq!(model_context_window_for("glm-5.2"), 1_000_000);
    }

    #[test]
    fn compact_limit_is_ninety_percent() {
        assert_eq!(compact_limit_for(1_000_000), 900_000);
        assert_eq!(compact_limit_for(204_800), 184_320);
    }

    #[test]
    fn web_search_is_disabled_for_vendors_without_real_search() {
        // 与开关无关：这两家写 live 只会换来一个必然被拒的请求
        assert_eq!(web_search_mode(true, "https://api.deepseek.com/v1"), "disabled");
        assert_eq!(web_search_mode(false, "https://api.deepseek.com/v1"), "disabled");
        assert_eq!(
            web_search_mode(true, "https://token-plan-cn.xiaomimimo.com/v1"),
            "disabled"
        );
        // MiniMax 支持联网 → 跟随开关
        assert_eq!(web_search_mode(true, "https://api.minimax.cn/v1"), "live");
        // 关的时候显式写回 Codex 默认值，顺带清掉残留的 live
        assert_eq!(web_search_mode(false, "https://api.minimax.cn/v1"), "cached");
    }

    #[test]
    fn base_model_id_strips_context_suffix() {
        assert_eq!(base_model_id("MiniMax-M3[1m]"), "MiniMax-M3");
        assert_eq!(base_model_id("MiniMax-M3"), "MiniMax-M3");
        // 不是以 `]` 结尾的方括号不当后缀处理（模型名里本来就可能带括号）
        assert_eq!(base_model_id("weird[model"), "weird[model");
    }

    #[test]
    fn catalog_referenced_matches_only_our_path() {
        let ours = Path::new("/home/u/.codex/models.json");
        assert!(catalog_referenced(
            "model_catalog_json = \"/home/u/.codex/models.json\"\n",
            ours
        ));
        // 厂商文档里常见的简写形式也算我们写的
        assert!(catalog_referenced(
            "model_catalog_json = \"~/.codex/models.json\"\n",
            ours
        ));
        // 用户自己的 catalog 不能被误判（否则会被我们删掉）
        assert!(!catalog_referenced(
            "model_catalog_json = \"~/.codex/model-catalogs/custom-catalog.json\"\n",
            ours
        ));
        assert!(!catalog_referenced("model = \"x\"\n", ours));
        assert!(!catalog_referenced("", ours));
    }

    #[test]
    fn set_item_creates_nested_tables() {
        let mut doc = crate::commands::ai::launch::parse_toml_lenient("");
        set_str(&mut doc, &["model_providers", "anyversion", "wire_api"], "responses");
        set_bool(&mut doc, &["model_providers", "anyversion", "requires_openai_auth"], true);
        set_int(&mut doc, &["model_context_window"], 204_800);
        let out = doc.to_string();
        assert!(out.contains("[model_providers.anyversion]"), "{out}");
        assert!(out.contains("wire_api = \"responses\""), "{out}");
        assert!(out.contains("requires_openai_auth = true"), "{out}");
        assert!(out.contains("model_context_window = 204800"), "{out}");
    }

    #[test]
    fn set_item_overwrites_existing_scalar_in_place() {
        let mut doc = crate::commands::ai::launch::parse_toml_lenient(
            "model_context_window = 1000000\nmodel = \"a\"\n",
        );
        set_int(&mut doc, &["model_context_window"], 204_800);
        let out = doc.to_string();
        assert!(out.contains("model_context_window = 204800"), "{out}");
        // 用户自己的键一个都不能丢
        assert!(out.contains("model = \"a\""), "{out}");
    }

    // ─── 伪装：窗口 / 能力必须按真实模型判定 ───

    fn temp_config(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("anyver-codex-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        // apply() 在文件为空时直接返回（不该凭空造主配置），先垫一行声明名
        std::fs::write(&path, "model = \"gpt-5.1-codex\"\n").unwrap();
        (dir, path)
    }

    fn ctx<'a>(
        path: &'a Path,
        real_model_name: &'a str,
        upstream_url: &'a str,
    ) -> ExtrasCtx<'a> {
        // 空集合：默认不覆盖任何键（保持原有写死行为）
        static EMPTY: std::sync::OnceLock<std::collections::HashSet<String>> =
            std::sync::OnceLock::new();
        ExtrasCtx {
            tool_id: "chatgptdesktop",
            main_path: path,
            base_url: "http://127.0.0.1:15721",
            upstream_url,
            api_key: "kira-token",
            model: "gpt-5.1-codex",
            model_name: "gpt-5.1-codex",
            real_model_name,
            provider: "anyversion",
            chosen_protocol: "openai",
            web_search: false,
            user_configured_paths: EMPTY.get_or_init(std::collections::HashSet::new),
        }
    }

    /// 伪装生效时，声明名 C 是官方名（`gpt-5.1-codex`），拿它查窗口表**必然落空**：
    /// 204,800 的模型会被写成 1,000,000，Codex 于是永远等不到压缩，上游直接报超长。
    #[test]
    fn context_window_follows_the_real_model_not_the_claimed_alias() {
        let (dir, path) = temp_config("window");
        apply(&ctx(&path, "minimax-m2.7", "https://api.deepseek.com")).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("model_context_window = 204800"),
            "窗口必须按真实模型 minimax-m2.7 写：{text}"
        );
        assert!(
            text.contains("model_auto_compact_token_limit = 184320"),
            "{text}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 图像 / 搜索能力同样按真实模型判：收图的 `deepseek-flash` 被伪装成 `gpt-5.1-codex`
    /// 之后不能变成「只收文本」—— 那会让 Codex 直接把图丢掉。
    #[test]
    fn catalog_capabilities_follow_the_real_model() {
        let (dir, path) = temp_config("caps");
        apply(&ctx(&path, "deepseek-flash", "https://api.deepseek.com")).unwrap();

        let catalog = std::fs::read_to_string(dir.join("models.json")).expect("catalog 应生成");
        let v: serde_json::Value = serde_json::from_str(&catalog).unwrap();
        let entry = &v["models"][0];
        assert_eq!(entry["slug"], "gpt-5.1-codex", "slug 用声明名，Codex 才匹配得上");
        assert_eq!(entry["display_name"], "deepseek-flash");
        assert_eq!(entry["input_modalities"], serde_json::json!(["text", "image"]));
        assert_eq!(entry["supports_search_tool"], serde_json::json!(true));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 用户通过「模型自定义参数」显式覆盖了 config 键时，apply 必须**让位**：
    /// 写死会把用户的值改回去（「界面上填了、启动时被改回去」= 摆设）。
    #[test]
    fn apply_yields_to_user_configured_keys() {
        let (dir, path) = temp_config("yield");
        let configured: std::collections::HashSet<String> = [
            "model_reasoning_effort".to_string(),
            "model_context_window".to_string(),
        ]
        .into_iter()
        .collect();
        let c = ExtrasCtx {
            tool_id: "chatgptdesktop",
            main_path: &path,
            base_url: "http://127.0.0.1:15721",
            upstream_url: "https://api.deepseek.com",
            api_key: "kira-token",
            model: "gpt-5.1-codex",
            model_name: "gpt-5.1-codex",
            real_model_name: "minimax-m2.7",
            provider: "anyversion",
            chosen_protocol: "openai",
            web_search: false,
            user_configured_paths: &configured,
        };
        apply(&c).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        // 让位的键：不再被写死值覆盖（用户值已由通用写入先落盘）
        assert!(
            !text.contains("model_reasoning_effort"),
            "用户自定义思考强度不该被写死覆盖：{text}"
        );
        assert!(
            !text.contains("model_context_window"),
            "用户自定义窗口不该被写死覆盖：{text}"
        );
        // 没让位的键：照常按查表写（minimax-m2.7 → 窗口 204800 → 压缩 184320）
        assert!(
            text.contains("model_auto_compact_token_limit = 184320"),
            "未自定义的压缩上限照常写：{text}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
