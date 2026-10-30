#[cfg(windows)]
use super::compose_apps_folder_uri;
use super::{
    default_work_dir, is_aggregate_upstream, json_value_to_yaml, read_applied_models, registry,
    render_json_template, resolve_start_command, resolve_write_target_file, set_yaml_path,
    strip_jsonc, write_format_for, write_json_config, write_toml_config,
    write_tool_config_from_spec, write_yaml_config, WriteFormat,
};
use crate::commands::ai_registry::ConfigFileDef;
use std::collections::HashMap;
use std::path::PathBuf;

/// TOML 语义合并：只改受管键，未托管的 `mcp_servers` / `[[hooks]]` / 注释必须原样留下。
///
/// 这正是文本级替换会写坏的三类输入：嵌套表、数组表头、以及被注释隔开的表。
#[test]
fn toml_merge_keeps_unmanaged_tables_and_comments() {
    let dir = std::env::temp_dir().join(format!("av-toml-merge-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("config.toml");

    let existing = r#"# 用户自己的注释
model = "old-model"

[mcp_servers]
[mcp_servers.fs]
command = "npx"
args = ["-y", "server-fs"]

[[hooks.PreToolUse]]
matcher = "Bash"
command = "echo hi"

[model_providers.personal]
name = "personal"
"#;
    let writes = vec![
        ("model".to_string(), serde_json::json!("anyversion/gpt-5")),
        (
            "model_providers.anyversion.base_url".to_string(),
            serde_json::json!("http://127.0.0.1:15721"),
        ),
    ];
    write_toml_config(&path, existing, &writes, "codex").unwrap();

    let out = std::fs::read_to_string(&path).unwrap();
    // 受管键被改写 / 新建
    assert!(out.contains(r#"model = "anyversion/gpt-5""#), "{out}");
    assert!(out.contains("http://127.0.0.1:15721"), "{out}");
    // 未托管内容原样保留
    assert!(out.contains("# 用户自己的注释"), "注释被冲掉了: {out}");
    assert!(
        out.contains(r#"command = "npx""#),
        "mcp_servers 丢了: {out}"
    );
    assert!(out.contains("[[hooks.PreToolUse]]"), "数组表头丢了: {out}");
    assert!(out.contains(r#"name = "personal""#), "别人的表丢了: {out}");
    // 结果必须是合法 TOML
    out.parse::<toml_edit::DocumentMut>()
        .expect("合并结果应可解析");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 重复表头（非法 TOML）：整体解析会失败，必须退化成切块合并而不是放弃写入。
#[test]
fn toml_merge_survives_duplicate_table_headers() {
    let dir = std::env::temp_dir().join(format!("av-toml-dup-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("config.toml");

    let existing =
        "[mcp_servers]\nenable = true\n\n[mcp_servers]\nenable = false\nhidden = \"keep\"\n";
    let writes = vec![("model".to_string(), serde_json::json!("anyversion/gpt-5"))];
    write_toml_config(&path, existing, &writes, "codex").unwrap();

    let out = std::fs::read_to_string(&path).unwrap();
    assert!(out.contains(r#"model = "anyversion/gpt-5""#), "{out}");
    // 后写的块覆盖先写的同键，未冲突的键保留
    assert!(out.contains("enable = false"), "{out}");
    assert!(out.contains(r#"hidden = "keep""#), "{out}");
    out.parse::<toml_edit::DocumentMut>()
        .expect("合并结果应可解析");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 回显「工具配置文件里当前写定的模型」：只认 `model` / `fallbackModel` 模板，
/// 并且**跳过模型定义占位符路径** —— 否则 opencode 的
/// `provider.X.models.{model_name}.name` 会抢先命中，回显成某个模型定义的名字
/// （谁先被遍历到纯属碰运气），界面于是显示成一个用户根本没选过的模型。
#[test]
fn applied_models_reads_selection_and_skips_placeholder_paths() {
    let dir = std::env::temp_dir().join(format!("av-applied-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("opencode.jsonc");

    let cfg: ConfigFileDef = serde_json::from_value(serde_json::json!({
        "path": path.display().to_string(),
        "format": "jsonc",
        "write": {
            "provider.anyversion.models.{model_name}.name": "modelName",
            "model": "model",
            "small_model": "fallbackModel"
        }
    }))
    .unwrap();
    std::fs::write(
        &path,
        r#"{
              // 用户的注释不该影响回显
              "model": "anyversion/gpt-5",
              "small_model": "anyversion/gpt-5-mini",
              "provider": { "anyversion": { "models": { "other": { "name": "other" } } } }
            }"#,
    )
    .unwrap();

    let out = read_applied_models(&cfg, &path, "opencode");
    assert_eq!(out.model.as_deref(), Some("anyversion/gpt-5"));
    assert_eq!(out.fallback_model.as_deref(), Some("anyversion/gpt-5-mini"));

    let _ = std::fs::remove_dir_all(&dir);
}

/// TOML 工具（codex）走文本扫描兜底：`model = "x"` 要能读回来，
/// 否则重启软件后就看不到上次选的模型了。
#[test]
fn applied_models_reads_toml_via_text_scan() {
    let dir = std::env::temp_dir().join(format!("av-applied-toml-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("config.toml");

    let cfg: ConfigFileDef = serde_json::from_value(serde_json::json!({
        "path": path.display().to_string(),
        "format": "toml",
        "write": { "model": "model" }
    }))
    .unwrap();
    std::fs::write(
        &path,
        "model = \"anyversion/gpt-5.4\"\nmodel_provider = \"anyversion\"\n",
    )
    .unwrap();

    let out = read_applied_models(&cfg, &path, "chatgptdesktop");
    assert_eq!(out.model.as_deref(), Some("anyversion/gpt-5.4"));
    assert_eq!(out.fallback_model, None, "Codex 没有 fallback 概念");

    let _ = std::fs::remove_dir_all(&dir);
}

/// TOML 写入必须**保留值类型**：布尔写成 `"true"` 会让 codex 报
/// `invalid type: string "true", expected a boolean`，整份 config 加载失败、工具起不来。
#[test]
fn toml_write_preserves_bool_and_number_types() {
    let dir = std::env::temp_dir().join(format!("av-toml-bool-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("config.toml");

    let existing = "[model_providers.anyversion]\nname = \"AnyVersion\"\n";
    let writes = vec![
        (
            "model_providers.anyversion.requires_openai_auth".to_string(),
            serde_json::json!(true),
        ),
        (
            "model_context_window".to_string(),
            serde_json::json!(1_000_000),
        ),
    ];
    write_toml_config(&path, existing, &writes, "codex").unwrap();

    let out = std::fs::read_to_string(&path).unwrap();
    // 裸布尔 / 数字，不能带引号
    assert!(out.contains("requires_openai_auth = true"), "{out}");
    assert!(!out.contains("requires_openai_auth = \"true\""), "{out}");
    assert!(out.contains("model_context_window = 1000000"), "{out}");
    let doc: toml_edit::DocumentMut = out.parse().unwrap();
    assert_eq!(
        doc["model_providers"]["anyversion"]["requires_openai_auth"].as_bool(),
        Some(true)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// 兄弟文件（`文件#子路径`）的格式按**目标文件**判定：codex 的 config.toml 边上还有个
/// auth.json，一律用主文件的 toml 去写会把 auth.json 写成 TOML。
#[test]
fn write_format_follows_target_extension() {
    assert_eq!(
        write_format_for(std::path::Path::new("/x/auth.json"), "toml"),
        WriteFormat::Json
    );
    assert_eq!(
        write_format_for(std::path::Path::new("/x/settings.json"), "yaml"),
        WriteFormat::Json
    );
    assert_eq!(
        write_format_for(std::path::Path::new("/x/credentials.yaml"), "json"),
        WriteFormat::Yaml
    );
    assert_eq!(
        write_format_for(std::path::Path::new("/x/config.toml"), "jsonc"),
        WriteFormat::Toml
    );
    // 没有可辨识扩展名 → 用工具声明的格式（jsonc 归到 json 写入器，它保留注释）
    assert_eq!(
        write_format_for(std::path::Path::new("/x/opencode"), "jsonc"),
        WriteFormat::Json
    );
    assert_eq!(
        write_format_for(std::path::Path::new("/x/whatever"), "yaml"),
        WriteFormat::Yaml
    );
}

/// `文件#子路径` 的文件部分：空 → 主文件；`~` → 展开成绝对路径（跨目录的偏好文件）；
/// 相对路径 → 主文件同目录的兄弟文件（auth.json / settings.json 的常规用法）。
#[test]
fn write_target_file_supports_home_and_siblings() {
    let main = PathBuf::from("/base/dir/config.toml");
    assert_eq!(resolve_write_target_file(&main, ""), main);
    assert_eq!(
        resolve_write_target_file(&main, "auth.json"),
        PathBuf::from("/base/dir/auth.json")
    );
    let home_relative = resolve_write_target_file(&main, "~/prefs.json");
    assert!(
        home_relative.is_absolute() && home_relative.ends_with("prefs.json"),
        "`~` 应展开成绝对路径: {}",
        home_relative.display()
    );
    assert_ne!(home_relative, PathBuf::from("/base/dir/~/prefs.json"));
}

/// 聚合上游识别：只有「回环主机 + 恰好等于聚合端口」才算（端口后面必须是 `/`/`?`/结尾）。
#[test]
fn is_aggregate_upstream_matches_only_the_exact_port() {
    assert!(is_aggregate_upstream("http://127.0.0.1:15721", 15721));
    assert!(is_aggregate_upstream("http://127.0.0.1:15721/v1", 15721));
    assert!(is_aggregate_upstream("http://localhost:15721", 15721));
    // 端口前缀误匹配 / 非回环 / 端口为 0 都不能算聚合
    assert!(!is_aggregate_upstream("http://127.0.0.1:157210", 15721));
    assert!(!is_aggregate_upstream("http://api.openai.com:15721", 15721));
    assert!(!is_aggregate_upstream("http://127.0.0.1:15721", 0));
}

/// 测试用临时目录（每个用例独立，跑完删）。
fn temp_dir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("anyver-writecfg-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 取**真实声明**并把它指向临时文件：这样验证的是 ai-tools/<id>/config.json 本身，
/// 声明改错就会在这里失败（且绝不动用户自己的配置文件）。
fn tool_cfg_at(id: &str, target: &std::path::Path) -> crate::commands::ai_registry::ToolConfig {
    let mut cfg = registry()
        .get_tool_config(id)
        .unwrap_or_else(|| panic!("{id} 应在注册表里"))
        .clone();
    cfg.config_file
        .as_mut()
        .expect("该工具应声明 configFile")
        .path = target.to_string_lossy().to_string();
    cfg
}

#[allow(clippy::too_many_arguments)]
fn write_for(
    cfg: &crate::commands::ai_registry::ToolConfig,
    fallback: Option<&str>,
    protocol: &str,
    web_search: bool,
) -> Result<(), String> {
    write_tool_config_from_spec(
        cfg,
        Some("deepseek-chat"),
        Some("claude-opus-4"), // 声明模型名（伪装）
        "http://127.0.0.1:15721",
        "kira-token",
        "https://api.deepseek.com",
        fallback,
        None,
        false,
        false,
        true,
        &[],
        &HashMap::new(),
        web_search,
        protocol,
    )
}

/// Claude Code：env 块必须**落盘**（只保存模型、不起 Kira 时也得生效），
/// 并且用 ANTHROPIC_AUTH_TOKEN（EchoBird claudecode.rs 同款），旧的 ANTHROPIC_API_KEY 要被清掉。
#[test]
fn claude_code_declaration_persists_env_block() {
    let dir = temp_dir("claude-code");
    let file = dir.join("settings.json");
    // 预置一个旧键，验证「换供应商后旧值不残留」
    std::fs::write(
        &file,
        r#"{"env":{"ANTHROPIC_API_KEY":"sk-old"},"permissions":{}}"#,
    )
    .unwrap();

    let cfg = tool_cfg_at("claude-code", &file);
    write_for(&cfg, Some("glm-4.6"), "anthropic", false).expect("写入应成功");

    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(doc["env"]["ANTHROPIC_BASE_URL"], "http://127.0.0.1:15721");
    assert_eq!(doc["env"]["ANTHROPIC_AUTH_TOKEN"], "kira-token");
    assert_eq!(doc["env"]["ANTHROPIC_MODEL"], "claude-opus-4");
    assert_eq!(doc["env"]["CLAUDE_CODE_SUBAGENT_MODEL"], "claude-opus-4");
    assert_eq!(doc["env"]["ANTHROPIC_DEFAULT_HAIKU_MODEL"], "glm-4.6");
    assert_eq!(doc["env"]["API_TIMEOUT_MS"], "3000000");
    assert!(
        doc["env"].get("ANTHROPIC_API_KEY").is_none(),
        "旧 key 应被清理"
    );
    // 用户自己的键不能被覆盖
    assert!(doc["permissions"].is_object());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Codex / ChatGPT 桌面端：凭证要写进同目录的 `auth.json`，且按 JSON（不是 TOML）落盘。
#[test]
fn codex_declaration_writes_auth_json() {
    let dir = temp_dir("codex");
    let file = dir.join("config.toml");
    let cfg = tool_cfg_at("codex-cli", &file);
    write_for(&cfg, None, "openai", false).expect("写入应成功");

    let toml_text = std::fs::read_to_string(&file).unwrap();
    assert!(
        toml_text.contains("model = \"claude-opus-4\""),
        "toml: {toml_text}"
    );
    assert!(
        toml_text.contains("[model_providers.anyversion]"),
        "toml: {toml_text}"
    );

    let auth = dir.join("auth.json");
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&auth).expect("auth.json 应被创建"))
            .expect("auth.json 必须是 JSON（不能按 toml 写）");
    assert_eq!(doc["OPENAI_API_KEY"], "kira-token");
    let _ = std::fs::remove_dir_all(&dir);
}

/// ChatGPT 桌面端：**自动伪装**。真实模型不是官方 OpenAI 名时，`config.toml` 里
/// 写下的 `model` 必须是官方名，且与启动时注册进 `model_aliases` 的键**完全一致**
/// （同一个 `effective_claimed_model`）。
///
/// 这条锁的是「写 A、注册 B」这个坑：两侧只要有一侧改了来源，App 发出的名字就无人
/// 认领 → 代理回落上游 → 404 / 答非所问，而且日志里看不出任何异常。
///
/// **刻意不钉死具体型号**：兜底名来自 `builtinModels` 清单第一项，官方上新/下架时
/// 清单会变，那属于数据维护，不该让这条测试变红。
#[test]
#[allow(clippy::too_many_arguments)]
fn chatgptdesktop_declares_an_official_name_the_proxy_also_registers() {
    let dir = temp_dir("chatgptdesktop-alias");
    let file = dir.join("config.toml");
    let cfg = tool_cfg_at("chatgptdesktop", &file);

    // 真实模型不是官方 OpenAI 名（界面截图里的那个场景）
    let real = "space-bunny";
    let claimed = crate::commands::ai::tool_config_custom::effective_claimed_model(
        "chatgptdesktop",
        real,
        "",
    );
    let listed = registry()
        .get_tool_config("chatgptdesktop")
        .map(|c| c.builtin_models.clone())
        .unwrap_or_default();
    assert!(
        !listed.is_empty(),
        "chatgptdesktop 的 builtinModels 是空的，自动伪装拿不到兜底名"
    );
    assert_eq!(
        claimed, listed[0],
        "兜底名必须取清单第一项（清单按新→旧维护）"
    );
    assert!(
        crate::commands::ai::tool_config_custom::is_legal_openai_model(&claimed),
        "声明名必须是 App 认的官方名，否则伪装等于没做"
    );

    write_tool_config_from_spec(
        &cfg,
        Some(real),
        Some(&claimed),
        "http://127.0.0.1:15721",
        "kira-token",
        "https://api.deepseek.com",
        None,
        None,
        false,
        false,
        true,
        &[],
        &HashMap::new(),
        false,
        "openai",
    )
    .expect("写入应成功");

    let text = std::fs::read_to_string(&file).unwrap();
    assert!(
        text.contains(&format!("model = \"{claimed}\"")),
        "config.toml 里写的必须就是代理注册的那个声明名：{text}"
    );
    // 窗口 / 模型目录按**真实模型**判定：slug 是声明名，窗口与显示名是真实模型
    let catalog = std::fs::read_to_string(dir.join("models.json")).expect("catalog 应生成");
    assert!(
        catalog.contains(&format!("\"slug\": \"{claimed}\"")),
        "{catalog}"
    );
    assert!(
        catalog.contains(&format!("\"display_name\": \"{real}\"")),
        "catalog 显示名应是真实模型：{catalog}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Codex 的附加写入（B4/B7）：上下文窗口、压缩阈值、reasoning 档位、按域名禁用联网搜索、
/// 以及模型目录 —— 而且目录必须落在**声明的配置目录旁边**。
///
/// 这条同时锁住一个真实事故：早先 catalog 路径用 `get_home_dir()` 取，结果一次
/// `cargo test` 就往真实用户的 `~/.codex/models.json` 写了 38KB。现在路径跟着声明的
/// 配置目录走，测试天然隔离。
#[test]
fn codex_extras_write_window_websearch_and_catalog_next_to_config() {
    let dir = temp_dir("codex-extras");
    let file = dir.join("config.toml");
    let cfg = tool_cfg_at("codex-cli", &file);
    // 上游固定是 api.deepseek.com；web_search 传 true，验证「域名判断优先于开关」
    write_for(&cfg, None, "openai", true).expect("写入应成功");

    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("model_reasoning_effort = \"high\""), "{text}");
    assert!(text.contains("model_context_window = 1000000"), "{text}");
    assert!(
        text.contains("model_auto_compact_token_limit = 900000"),
        "{text}"
    );
    assert!(
        text.contains("web_search = \"disabled\""),
        "deepseek 没有可用的联网检索，必须 disabled（开关打开也一样）: {text}"
    );

    let catalog = dir.join("models.json");
    assert!(catalog.is_file(), "catalog 应写在配置同目录: {catalog:?}");
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&catalog).unwrap()).unwrap();
    // 目录里的身份是**声明模型名 C**（工具以为自己调用的那个）—— Codex 按它查表
    assert_eq!(doc["models"][0]["slug"], "claude-opus-4");
    assert_eq!(doc["models"][0]["context_window"].as_u64(), Some(1_000_000));
    let catalog_ref = catalog.to_string_lossy().replace('\\', "/");
    assert!(text.contains(&catalog_ref), "配置应引用该目录: {text}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 还原必须把附加写入的内容一起清掉：配置里的窗口 / 档位 / 联网搜索键，以及我们生成的
/// catalog 文件。否则「选了官方模型」之后 Codex 仍带着我们的设置，磁盘上还留一份目录。
#[test]
fn codex_restore_removes_extras_and_generated_catalog() {
    let dir = temp_dir("codex-restore");
    let file = dir.join("config.toml");
    let cfg = tool_cfg_at("codex-cli", &file);
    write_for(&cfg, None, "openai", false).expect("写入应成功");
    let catalog = dir.join("models.json");
    assert!(catalog.is_file(), "前提：catalog 已被生成");

    crate::commands::ai::tool_config_restore::restore_tool_config(&cfg).expect("还原应成功");

    let text = std::fs::read_to_string(&file).unwrap();
    for key in [
        "model_context_window",
        "model_auto_compact_token_limit",
        "model_reasoning_effort",
        "web_search",
        "model_catalog_json",
    ] {
        assert!(!text.contains(key), "还原后不该留下 {key}: {text}");
    }
    assert!(!catalog.exists(), "我们生成的 catalog 应被删除");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Kimi 桌面端（B5）：上下文窗口 / 能力声明 / 自适应思考是**常量**。
/// 缺了它，界面只按最基础的能力展示，用户会以为模型不支持工具调用或思考。
#[test]
fn kimidesktop_declaration_writes_capability_constants() {
    let dir = temp_dir("kimidesktop");
    let file = dir.join("config.toml");
    let cfg = tool_cfg_at("kimidesktop", &file);
    write_for(&cfg, None, "openai", false).expect("写入应成功");

    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("max_context_size = 1000000"), "{text}");
    assert!(text.contains("capabilities"), "{text}");
    assert!(text.contains("\"tool_use\""), "{text}");
    assert!(text.contains("\"thinking\""), "{text}");
    assert!(text.contains("adaptive_thinking = true"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// OMP（C8）：`modelRoles` 是**一整套**角色。换模型时所有指向我们的角色都要重新指向，
/// 并各自保留 thinking 档位后缀；用户自己的角色一个都不动。
#[test]
fn omp_retargets_all_managed_roles_keeping_thinking_suffix() {
    let dir = temp_dir("omp");
    let file = dir.join("models.yml");
    let cfg = tool_cfg_at("omp", &file);
    // 预置上一次写入留下的角色（旧模型），外加一个用户自己的角色
    std::fs::write(
            dir.join("config.yml"),
            "modelRoles:\n  default: echobird/old-model:high\n  smol: echobird/old-smol\n  plan: echobird/old-plan:xhigh\n  custom: personal/other:max\n",
        )
        .unwrap();

    write_for(&cfg, None, "openai", false).expect("写入应成功");

    let doc: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(dir.join("config.yml")).unwrap()).unwrap();
    let roles = doc.get("modelRoles").unwrap();
    assert_eq!(roles["default"], "echobird/claude-opus-4:high");
    assert_eq!(roles["smol"], "echobird/claude-opus-4");
    assert_eq!(roles["plan"], "echobird/claude-opus-4:xhigh");
    assert_eq!(
        roles["custom"], "personal/other:max",
        "用户自己的角色一个都不能改"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// ZCode（A3）：默认模型选择与 provider 规则在**另一个文件** `provider_config.json` 里。
/// 只写 `config.json` 的话，界面里的默认模型还是旧的 —— 这正是「设置成功却没生效」。
#[test]
fn zcode_writes_default_selection_into_provider_config() {
    let dir = temp_dir("zcode");
    let file = dir.join("config.json");
    let cfg = tool_cfg_at("zcode", &file);
    // 预置 ZCode 自己会生成的那份结构，并放一个别人的 provider
    std::fs::write(
        dir.join("provider_config.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "schemaVersion": 1,
            "config": {
                "providerConfigRules": { "providerRules": [
                    { "providerId": "other", "providerName": "Other" }
                ] },
                "modelConfigRules": { "providerModelRules": [], "manualProviderModelRules": [] },
                "defaultModelSelection": { "providerId": "other", "modelId": "old" },
                "providerOrder": ["other"]
            }
        }))
        .unwrap(),
    )
    .unwrap();

    write_for(&cfg, None, "openai", false).expect("写入应成功");

    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("provider_config.json")).unwrap())
            .unwrap();
    let cfgv = &doc["config"];
    assert_eq!(cfgv["defaultModelSelection"]["providerId"], "anyversion");
    assert_eq!(cfgv["defaultModelSelection"]["modelId"], "claude-opus-4");
    let rules = cfgv["providerConfigRules"]["providerRules"]
        .as_array()
        .unwrap();
    assert_eq!(rules.len(), 2, "别人的 provider 规则不能丢: {rules:?}");
    let ours = rules
        .iter()
        .find(|r| r["providerId"] == "anyversion")
        .expect("应写入我们的规则");
    assert_eq!(ours["config"]["api"]["baseUrl"], "http://127.0.0.1:15721");
    assert_eq!(ours["config"]["access"]["apiKey"], "kira-token");
    assert_eq!(
        cfgv["providerOrder"],
        serde_json::json!(["other", "anyversion"])
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Qwen Code：缺 `modelProviders[]` 时它找不到自定义端点，必须写成数组，
/// 且数组里的 envKey 与配置文件 env 块里的键对得上。
#[test]
fn qwencode_declaration_registers_model_provider() {
    let dir = temp_dir("qwencode");
    let file = dir.join("settings.json");
    let cfg = tool_cfg_at("qwencode", &file);
    write_for(&cfg, None, "openai", false).expect("写入应成功");

    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    let list = doc["modelProviders"]["openai"]
        .as_array()
        .expect("应是数组");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["id"], "claude-opus-4");
    assert_eq!(list[0]["baseUrl"], "http://127.0.0.1:15721");
    assert_eq!(list[0]["envKey"], "ANYVERSION_API_KEY");
    // envKey 指向的键要真落盘，否则 Qwen Code 拿不到 key
    assert_eq!(doc["env"]["ANYVERSION_API_KEY"], "kira-token");
    assert_eq!(doc["security"]["auth"]["selectedType"], "openai");
    let _ = std::fs::remove_dir_all(&dir);
}

/// MiMo 桌面端：模型选择写它自己的 preferences.json，**不能**动顶层 model/small_model
/// （那是 MiMo Code CLI 的选择器，改了会连带改掉 CLI 的默认模型）。
#[test]
fn mimodesktop_declaration_writes_preferences_without_touching_cli_model() {
    let dir = temp_dir("mimodesktop");
    let file = dir.join("mimocode.jsonc");
    let mut cfg = tool_cfg_at("mimodesktop", &file);
    // 真实声明写的是 %APPDATA%\Xiaomi MiMo\preferences.json，测试里换成临时目录
    let write = cfg.config_file.as_mut().unwrap().write.as_mut().unwrap();
    let key = write
        .keys()
        .find(|k| k.contains("preferences.json"))
        .expect("声明里应有 preferences.json 的写入项")
        .clone();
    let tpl = write.remove(&key).unwrap();
    write.insert(format!("prefs.json#model"), tpl);

    write_for(&cfg, None, "openai", false).expect("写入应成功");

    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap().replace("//", "")).unwrap();
    assert!(
        doc.get("model").is_none(),
        "不能写顶层 model（会改掉 CLI 默认模型）"
    );
    assert!(doc["provider"]["anyversion-desktop"].is_object());
    let prefs: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("prefs.json")).unwrap()).unwrap();
    assert_eq!(prefs["model"], "anyversion-desktop/claude-opus-4");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 写 JSONC（带注释）配置文件不能把用户已有的内容整份冲掉。
///
/// 回归：以前 `serde_json::from_str` 解析带注释的 jsonc 失败 → 被当成空文档 → 整份覆盖。
#[test]
fn write_json_config_preserves_jsonc_comments_and_user_keys() {
    let dir = temp_dir("jsonc");
    let file = dir.join("opencode.jsonc");
    let original = "{\n  // 我的注释\n  \"model\": \"official/model\",\n  \"permissions\": { \"allow\": [\"Read\"] }\n}\n";
    std::fs::write(&file, original).unwrap();
    write_json_config(
        &file,
        &original,
        &[(
            "provider.anyversion.options.baseURL".to_string(),
            serde_json::json!("http://127.0.0.1:1"),
        )],
        None,
        &crate::commands::ai_registry::registry()
            .get_tool_config("opencode")
            .unwrap(),
    )
    .unwrap();
    let out = std::fs::read_to_string(&file).unwrap();
    let doc: serde_json::Value =
        serde_json::from_str(&strip_jsonc(&out)).expect("写回后仍是合法 JSONC");
    // 用户的权限没丢，我们写的键加上了
    assert_eq!(doc["permissions"]["allow"][0], "Read");
    assert_eq!(
        doc["provider"]["anyversion"]["options"]["baseURL"],
        "http://127.0.0.1:1"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// `json:` 值模板：数组/对象/数字都能表达，占位符在字符串内转义，
/// 模板非法时**报错**（静默写半份配置比不写更难排查）。
#[test]
fn json_value_template_renders_and_escapes() {
    let v = render_json_template(
        r#"[{"id":"{model}","name":"{modelName}","baseUrl":"{baseUrl}","envKey":"X"}]"#,
        "anyversion/gpt-4o",
        "gpt-4o",
        "http://127.0.0.1:1",
        "sk",
    )
    .unwrap();
    assert_eq!(v[0]["id"], "anyversion/gpt-4o");
    // {modelName} 不能被 {model} 抢先匹配（否则会变成 "Name}"）
    assert_eq!(v[0]["name"], "gpt-4o");
    assert_eq!(v[0]["baseUrl"], "http://127.0.0.1:1");

    // 数字 / 布尔 / 嵌套对象
    assert_eq!(
        render_json_template("2", "", "", "", "").unwrap(),
        serde_json::json!(2)
    );
    assert_eq!(
        render_json_template(r#"{"a":[{"b":true}]}"#, "", "", "", "").unwrap(),
        serde_json::json!({"a": [{"b": true}]})
    );

    // 模型名里的引号要转义，不能拼出坏 JSON
    let quoted = render_json_template(r#"{"m":"{model}"}"#, "weird\"name", "", "", "").unwrap();
    assert_eq!(quoted["m"], "weird\"name");

    assert!(render_json_template("{not json", "", "", "", "").is_err());
}

/// 死路径绝不能被当成启动命令（`os error 3` 的根因）。
///
/// 回归场景（2026-10-05 实测）：Claude 桌面端从 Squirrel 版换成 MSIX/Store 版，
/// `%LOCALAPPDATA%\AnthropicClaude\Claude.exe` 整个目录被卸载器删掉，但注册表
/// 卸载项索引（进程级缓存）里还留着它 → 启动直接 `os error 3`，点启动毫无反应。
#[test]
fn live_exe_rejects_a_deleted_path() {
    let mut dir = std::env::temp_dir();
    dir.push(format!("anyver-live-exe-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("gone.exe");
    std::fs::write(&exe, "").unwrap();

    // 文件在 → 认
    assert_eq!(super::live_exe(Some(exe.clone())), Some(exe.clone()));

    // 文件被删（重装/卸载）→ 丢掉，让上层继续走下一条策略（最终落到包 URI）
    std::fs::remove_file(&exe).unwrap();
    assert!(super::live_exe(Some(exe.clone())).is_none());
    // 目录不算可执行文件：CreateProcess 同样起不来
    std::fs::create_dir_all(&dir).unwrap();
    assert!(super::live_exe(Some(dir.clone())).is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

/// 桌面应用（startCommand 为空）要靠「检测到的 exe 绝对路径」启动。
#[test]
fn start_command_falls_back_to_detected_exe() {
    use crate::commands::ai_registry::PathConfig;
    use std::collections::HashMap;

    let mut dir = std::env::temp_dir();
    dir.push(format!("anyver-launch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let exe_name = if cfg!(windows) {
        "AnyverProbe.exe"
    } else {
        "AnyverProbe"
    };
    let exe = dir.join(exe_name);
    std::fs::write(&exe, "").unwrap();

    let mk = |cmd: &str, detect: &str| PathConfig {
        name: String::new(),
        category: String::new(),
        api_protocol: vec![],
        command: String::new(),
        start_command: cmd.to_string(),
        detect_cmd: detect.to_string(),
        install_cmd: String::new(),
        uninstall_cmd: None,
        paths: {
            let mut m = HashMap::new();
            for key in ["win32", "darwin", "linux"] {
                m.insert(key.to_string(), vec![exe.to_string_lossy().to_string()]);
            }
            m
        },
        launch_uri: None,
        install_hints: None,
    };

    // 声明了 startCommand → 原样使用（命令名交给方言解析）
    assert_eq!(
        resolve_start_command("anyverprobe", &mk("probe --flag", ""), "probe --flag").as_deref(),
        Some("probe --flag")
    );
    // 没声明 → 用检测到的 exe 绝对路径
    assert_eq!(
        resolve_start_command("anyverprobe", &mk("", ""), "").as_deref(),
        Some(exe.to_string_lossy().as_ref())
    );
    // 既没命令也找不到 exe → 退回 detect_cmd；再没有就 None（由调用方报错）
    let mut no_exe = mk("", "probe --version");
    for key in ["win32", "darwin", "linux"] {
        no_exe.paths.insert(
            key.to_string(),
            vec![dir.join("nope").to_string_lossy().to_string()],
        );
    }
    assert_eq!(
        resolve_start_command("anyverprobe", &no_exe, "").as_deref(),
        Some("probe")
    );
    let mut nothing = no_exe.clone();
    nothing.detect_cmd = String::new();
    assert!(resolve_start_command("anyverprobe", &nothing, "").is_none());

    // 桌面端（Store / MSIX 安装）：磁盘上没有 exe、也没有 detect_cmd →
    // 回退 `shell:AppsFolder\…` 包 URI。此前这里直接 None，用户看到的是
    //「明明检测到已安装，点启动却报『没有可用的启动命令』」。
    #[cfg(windows)]
    {
        let mut store_app = nothing.clone();
        store_app.category = "Desktop".to_string();
        store_app.launch_uri = Some("shell:AppsFolder\\Anyver.Probe_abc123!App".to_string());
        assert_eq!(
            resolve_start_command("anyverprobe", &store_app, "").as_deref(),
            Some("shell:AppsFolder\\Anyver.Probe_abc123!App")
        );
        // CLI 工具没有 Store 形态：即使声明了 launch_uri 也不走这条回退
        let mut cli_app = store_app.clone();
        cli_app.category = "CLI".to_string();
        assert!(resolve_start_command("anyverprobe", &cli_app, "").is_none());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// 真实包族名重组 AUMID：只换族名，应用 Id 沿用声明里的。
///
/// beta 渠道（`OpenAI.CodexBeta_xxx`）装出来的族名与声明里写死的
/// `OpenAI.Codex_2p2nqsd0c76g0` 不同，组装时必须保留 `!App` 后缀，
/// 否则 explorer 打开的就不是应用。
#[cfg(windows)]
#[test]
fn compose_apps_folder_uri_keeps_app_id() {
    assert_eq!(
        compose_apps_folder_uri(
            "OpenAI.Codex_abc",
            "shell:AppsFolder\\OpenAI.Codex_2p2nqsd0c76g0!App"
        ),
        "shell:AppsFolder\\OpenAI.Codex_abc!App"
    );
    assert_eq!(
        compose_apps_folder_uri(
            "Claude_new",
            "shell:AppsFolder\\Claude_pzs8sxrjxfjjc!Claude"
        ),
        "shell:AppsFolder\\Claude_new!Claude"
    );
}

/// 工作目录：exe 存在时用它所在目录，否则退到主目录（空目录会让 start /d 失败）。
#[test]
fn work_dir_falls_back_to_home() {
    let mut dir = std::env::temp_dir();
    dir.push(format!("anyver-wd-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join(if cfg!(windows) {
        "anyver-wd.exe"
    } else {
        "anyver-wd"
    });
    std::fs::write(&exe, "").unwrap();
    assert_eq!(
        default_work_dir(&exe.to_string_lossy()),
        dir.to_string_lossy()
    );
    // 找不到文件（`probe --flag` 这种命令名）→ 主目录
    let home = crate::commands::utils::get_home_dir()
        .to_string_lossy()
        .to_string();
    assert_eq!(default_work_dir("probe --flag"), home);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn set_yaml_path_creates_nested_mapping() {
    let mut doc = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
    set_yaml_path(
        &mut doc,
        "providers.echobird.baseUrl",
        serde_yaml::Value::String("http://127.0.0.1:1234".into()),
    );
    set_yaml_path(
        &mut doc,
        "providers.echobird.api",
        serde_yaml::Value::String("openai-completions".into()),
    );
    set_yaml_path(
        &mut doc,
        "modelRoles.default",
        serde_yaml::Value::String("echobird/gpt-5".into()),
    );
    let providers = doc.get("providers").unwrap().get("echobird").unwrap();
    assert_eq!(
        providers.get("baseUrl").unwrap().as_str(),
        Some("http://127.0.0.1:1234")
    );
    assert_eq!(
        providers.get("api").unwrap().as_str(),
        Some("openai-completions")
    );
    assert_eq!(
        doc.get("modelRoles")
            .unwrap()
            .get("default")
            .unwrap()
            .as_str(),
        Some("echobird/gpt-5")
    );
}

#[test]
fn set_yaml_path_replaces_existing_scalar_with_mapping() {
    let mut doc = serde_yaml::from_str::<serde_yaml::Value>("providers: junk\n").unwrap();
    set_yaml_path(
        &mut doc,
        "providers.echobird.apiKey",
        serde_yaml::Value::String("sk-test".into()),
    );
    assert!(doc.get("providers").unwrap().is_mapping());
    assert_eq!(
        doc["providers"]["echobird"]["apiKey"].as_str(),
        Some("sk-test")
    );
}

#[test]
fn yaml_config_round_trip_preserves_unrelated_keys() {
    let dir = std::env::temp_dir().join(format!("kira_launch_yaml_test_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path: PathBuf = dir.join("models.yml");
    std::fs::write(&path, "providers:\n  personal:\n    apiKey: PERSONAL_KEY\n").unwrap();
    let writes = vec![
        (
            "providers.echobird.baseUrl".to_string(),
            serde_json::json!("http://127.0.0.1:9"),
        ),
        (
            "providers.echobird.models".to_string(),
            serde_json::json!([{"id": "gpt-5", "name": "gpt-5"}]),
        ),
    ];
    write_yaml_config(&path, &std::fs::read_to_string(&path).unwrap(), &writes).unwrap();
    let out: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    // 其它 provider 保留
    assert_eq!(
        out["providers"]["personal"]["apiKey"].as_str(),
        Some("PERSONAL_KEY")
    );
    // 新 provider 写入
    assert_eq!(
        out["providers"]["echobird"]["baseUrl"].as_str(),
        Some("http://127.0.0.1:9")
    );
    assert_eq!(
        out["providers"]["echobird"]["models"][0]["id"].as_str(),
        Some("gpt-5")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn json_value_to_yaml_converts_objects_and_arrays() {
    let jv = serde_json::json!({
        "a": "x",
        "b": [1, 2],
        "c": { "nested": true }
    });
    let yv = json_value_to_yaml(&jv);
    assert_eq!(yv["a"].as_str(), Some("x"));
    assert_eq!(yv["b"][1].as_i64(), Some(2));
    assert_eq!(yv["c"]["nested"].as_bool(), Some(true));
}

#[test]
fn strip_jsonc_keeps_comment_marks_inside_strings() {
    // 回显「工具当前用的模型」要能解析 jsonc；URL 里常含 //，不能误删
    let raw = "{\n // 注释\n \"model\": \"gpt-5\",\n \"baseUrl\": \"http://127.0.0.1:9/v1\", /* 块注释 */\n \"x\": \"a/*b*/c\"\n}";
    let cleaned = strip_jsonc(raw);
    let value: serde_json::Value = serde_json::from_str(&cleaned).unwrap();
    assert_eq!(value["model"].as_str(), Some("gpt-5"));
    assert_eq!(value["baseUrl"].as_str(), Some("http://127.0.0.1:9/v1"));
    assert_eq!(value["x"].as_str(), Some("a/*b*/c"));
}
