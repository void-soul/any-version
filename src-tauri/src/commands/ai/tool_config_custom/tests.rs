use super::{read_model, write_config, ModelWrite};
use std::path::PathBuf;

use super::{
    builtin_candidates, chatgptdesktop_alias, claude_desktop_layout, claudedesktop_alias,
    effective_claimed_model, first_legal, is_legal_claudedesktop_model, is_legal_openai_model,
    resolve_claimed_model, write_claudedesktop_with, ClaudeDesktopLayout, CLAUDESKTOP_WRITER,
    CLAUDE_DESKTOP_PROFILE_ID,
};
use std::path::Path;

fn probe_path(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("anyver-toolcfg-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("models.json")
}

/// 未知写入器名必须报错（配置写错要立刻看得见），不能静默写出半成品。
#[test]
fn unknown_writer_is_rejected() {
    let path = probe_path("unknown-writer");
    let m = ModelWrite {
        model: "m",
        claimed: "m",
        base_url: "https://x/v1",
        api_key: "sk",
        upstream_url: "",
        one_m: false,
    };
    assert!(write_config("nope", &path, &m).is_err());
    assert!(read_model("nope", &path).is_none());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// **断链的重解析点不能让我们整体失败**。
///
/// 真机形态：`%LOCALAPPDATA%\Claude-3p` 被 junction 到 `D:\sim-tool\Claude-3p`，
/// 而目标目录被删了 → 裸 `create_dir_all` 报
/// `os error 183`「当文件已存在时，无法创建该文件」，用户完全看不懂，
/// 「给 Claude Desktop 设置模型」必然失败。
#[cfg(windows)]
#[test]
fn ensure_dir_repairs_dangling_junction() {
    let root = std::env::temp_dir().join(format!("anyver-junction-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let link = root.join("link");
    let target = root.join("real-target");

    // 建 junction：mklink 是 cmd 内建命令，建 junction 不需要管理员权限
    let created = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&link)
        .arg(&target)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !created {
        // 环境不允许建 junction：跳过（被测逻辑没问题，只是没法在这里复现）
        let _ = std::fs::remove_dir_all(&root);
        return;
    }
    assert!(!target.exists(), "此刻应是断链状态");

    // 记录裸调用会怎样（不同 std 版本行为可能不同，不作为断言）
    let plain = std::fs::create_dir_all(&link).is_ok();

    super::ensure_dir(&link).expect("断链 junction 应被修复");
    assert!(
        target.is_dir(),
        "链接目标应被建出来（裸调用是否成功={plain}）"
    );
    std::fs::write(link.join("probe.json"), "{}").expect("修好后应能正常写入");

    // 清理：先删链接本身，再删整个临时根
    let _ = std::fs::remove_dir(&link);
    let _ = std::fs::remove_dir_all(&root);
}

fn probe_layout(name: &str) -> ClaudeDesktopLayout {
    let mut root = std::env::temp_dir();
    root.push(format!(
        "anyver-claudedesktop-{}-{}",
        name,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    ClaudeDesktopLayout {
        official_cfg: root.join("Claude").join("claude_desktop_config.json"),
        threep_cfg: root.join("Claude-3p").join("claude_desktop_config.json"),
        lib_dir: root.join("Claude-3p").join("configLibrary"),
    }
}

/// Claude Desktop：三处落点都要对，且**不能覆盖用户自己的键**（mcpServers 等）。
#[test]
fn claudedesktop_writes_3p_profile_and_keeps_user_keys() {
    let layout = probe_layout("write");
    // 预置：官方配置里已有用户自己的 mcpServers
    std::fs::create_dir_all(layout.official_cfg.parent().unwrap()).unwrap();
    std::fs::write(
        &layout.official_cfg,
        r#"{"mcpServers":{"x":{"command":"node"}},"deploymentMode":"1p"}"#,
    )
    .unwrap();

    // 注意用 `_with(&layout)`：`write_config` 会解析**本机真实**的 Claude 目录，
    // 测试里绝不能碰用户自己的配置。
    write_claudedesktop_with(
        &layout,
        &ModelWrite {
            model: "deepseek-chat",
            claimed: "claude-opus-4",
            base_url: "http://127.0.0.1:15721/",
            api_key: "kira-token",
            upstream_url: "https://api.deepseek.com/anthropic",
            one_m: true,
        },
    )
    .expect("写入应成功");

    // 两个顶层配置都切到 3p，且用户的 mcpServers 还在
    for cfg in [&layout.official_cfg, &layout.threep_cfg] {
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(cfg).unwrap()).unwrap();
        assert_eq!(doc["deploymentMode"], "3p", "{} 未切到 3p", cfg.display());
    }
    let official: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&layout.official_cfg).unwrap()).unwrap();
    assert_eq!(official["mcpServers"]["x"]["command"], "node");

    // profile：网关地址去尾斜杠、key 落盘、模型名是声明名（代理按它映射），
    // labelOverride 用真实 id（界面显示），1M 开关跟随用户勾选
    let profile: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            layout
                .lib_dir
                .join("9f2c1d47-5b6e-4a83-9c07-2f6a1b8d3e50.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(profile["inferenceGatewayBaseUrl"], "http://127.0.0.1:15721");
    assert_eq!(profile["inferenceGatewayApiKey"], "kira-token");
    assert_eq!(profile["inferenceModels"][0]["name"], "claude-opus-4");
    assert_eq!(
        profile["inferenceModels"][0]["labelOverride"],
        "deepseek-chat"
    );
    assert_eq!(profile["inferenceModels"][0]["prefer1m"], true);
    assert_eq!(profile["inferenceProvider"], "gateway");

    let meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(layout.lib_dir.join("_meta.json")).unwrap())
            .unwrap();
    assert_eq!(meta["appliedId"], "9f2c1d47-5b6e-4a83-9c07-2f6a1b8d3e50");
    assert_eq!(meta["entries"][0]["name"], "Kira");

    // 注意：这里不测 read_claudedesktop —— 它读的是**本机真实**目录，
    // 用户一旦用过这个功能就会读到真配置，断言会随环境变。

    let _ = std::fs::remove_dir_all(layout.threep_cfg.parent().unwrap());
    let _ = std::fs::remove_dir_all(layout.official_cfg.parent().unwrap());
}

/// 回环上游允许空 key（本地推理不鉴权），公网上游必须给 key。
#[test]
fn claudedesktop_requires_key_unless_local() {
    let layout = probe_layout("keys");
    let with = |base: &'static str, key: &'static str| ModelWrite {
        model: "m",
        claimed: "",
        base_url: base,
        api_key: key,
        upstream_url: "",
        one_m: false,
    };
    assert!(write_claudedesktop_with(&layout, &with("https://api.anthropic.com", "")).is_err());
    assert!(write_claudedesktop_with(&layout, &with("http://127.0.0.1:11434", "")).is_ok());
    // 没有网关地址直接报错，不写出半份 profile
    assert!(write_claudedesktop_with(&layout, &with("", "sk")).is_err());
    let _ = std::fs::remove_dir_all(layout.threep_cfg.parent().unwrap());
    let _ = std::fs::remove_dir_all(layout.official_cfg.parent().unwrap());
}

/// 平台解析：Windows/macOS 能拿到布局，其余平台返回 None（调用方给明确报错）。
#[test]
fn claudedesktop_layout_is_platform_aware() {
    let layout = claude_desktop_layout();
    if cfg!(any(windows, target_os = "macos")) {
        let layout = layout.expect("Windows/macOS 应能解析出目录");
        assert!(layout.official_cfg.ends_with("claude_desktop_config.json"));
        assert!(layout.threep_cfg.to_string_lossy().contains("Claude-3p"));
        assert!(layout.lib_dir.to_string_lossy().contains("configLibrary"));
    } else {
        assert!(layout.is_none());
    }
}

// ═══════════════ 模型伪装：Desktop 的模型名校验（qa 规则） ═══════════════
//
// 规则来自 Claude Desktop 1.44121.2 的 app.asar（`inferenceProvider: "gateway"`
// → `e_e(name)` → `qa(name)`），原文：
//   Ka  = ["sonnet","opus","haiku","fable","mythos"]
//   Gge = ["claude", ...Ka, "anthropic"]
//   Kge = /ark-code|deepseek|gemini|glm|gpt|hy3|kimi|llama|minimax|openai|qwen|hunyuan|codex|.../
//   qa(name) = !Kge.test(name) && (Wge.test(name) || Gge.some(t => name.includes(t)))
// 下面把原文的关键片段原样搬成 Rust 用例 —— 这是 Desktop 会不会接受某个名字的**唯一判据**。

/// 清单（按「新 → 旧」维护）。测试自己给，不读注册表 —— 这样「清单顺序决定兜底」
/// 这条契约能被独立验证，不受 `ai-tools/*.json` 改动影响。
fn cands(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// 与 `ai-tools/claudedesktop/config.json` 当前清单一致（仅测试用；真实来源是那个 JSON）
const CLAUDE_LIST: &[&str] = &["claude-opus-5-5", "claude-sonnet-4-6"];

/// 合法的伪装名原样使用（不改动用户填的声明名）
#[test]
fn alias_keeps_a_legal_preferred_name() {
    let c = cands(CLAUDE_LIST);
    assert_eq!(
        claudedesktop_alias("space-bunny", "claude-sonnet-4-6", &c).unwrap(),
        "claude-sonnet-4-6"
    );
    assert_eq!(
        claudedesktop_alias("space-bunny", "anthropic/claude-opus-4", &c).unwrap(),
        "anthropic/claude-opus-4"
    );
    // Wge：纯家族名也合法（`^(sonnet|opus|haiku|fable|mythos)(-[0-9.]+)?$`）
    assert_eq!(
        claudedesktop_alias("space-bunny", "sonnet", &c).unwrap(),
        "sonnet"
    );
    assert_eq!(
        claudedesktop_alias("space-bunny", "opus-4-5", &c).unwrap(),
        "opus-4-5"
    );
}

/// 没配声明名 → 取清单里**第一个**合法项（= 按新→旧维护时的最新在售型号）
#[test]
fn alias_falls_back_to_the_newest_candidate() {
    let fallback = "claude-opus-5-5";
    let c = cands(CLAUDE_LIST);
    // 没填
    assert_eq!(
        claudedesktop_alias("space-bunny", "", &c).unwrap(),
        fallback
    );
    assert_eq!(
        claudedesktop_alias("space-bunny", "   ", &c).unwrap(),
        fallback
    );
    // 填了但 Desktop 会剔除的：真实模型名、别家模型名
    assert_eq!(
        claudedesktop_alias("space-bunny", "space-bunny", &c).unwrap(),
        fallback
    );
    assert_eq!(
        claudedesktop_alias("glm-5.3", "glm-5.3", &c).unwrap(),
        fallback
    );
    assert_eq!(claudedesktop_alias("hy3", "hy3", &c).unwrap(), fallback);
}

/// 易错反例：**黑名单先判**，所以「含 claude 但也含别家词」依然被拒
#[test]
fn alias_rejects_names_the_desktop_blacklists_even_with_anthropic_words() {
    let c = cands(CLAUDE_LIST);
    // 这三个如果实现成「只检查含不含 claude」就会误判成合法
    for illegal in ["glm-5.3-claude", "qwen-claude-sonnet", "deepseek-v4-opus"] {
        assert_eq!(
            claudedesktop_alias("x", illegal, &c).unwrap(),
            "claude-opus-5-5",
            "{illegal} 含 Anthropic 词但也命中黑名单，必须回落"
        );
    }
}

/// 真实模型名本身合法时也不该被改写（用户就是在用 Anthropic 模型）
#[test]
fn alias_is_identity_when_real_model_is_already_anthropic() {
    let c = cands(CLAUDE_LIST);
    assert_eq!(
        claudedesktop_alias("claude-sonnet-4-5", "", &c).unwrap(),
        "claude-sonnet-4-5"
    );
    assert_eq!(claudedesktop_alias("sonnet", "", &c).unwrap(), "sonnet");
}

/// **退役即失效**：兜底名只能来自清单，清单改了立刻生效 —— 代码里不许再写死一个模型名。
///
/// 这条是「伪装成已废弃模型」的唯一护栏：早先版本把 `claude-sonnet-4-6` 硬编码成
/// 兜底，官方一旦退役该型号，Desktop 就会剔除这个名字，而用户完全看不出原因。
#[test]
fn fallback_follows_the_list_instead_of_a_hardcoded_name() {
    let old = cands(&["claude-sonnet-4-6"]);
    assert_eq!(
        claudedesktop_alias("space-bunny", "", &old).unwrap(),
        "claude-sonnet-4-6"
    );
    // 清单换新 → 兜底跟着换，代码一行不改
    let new = cands(&["claude-opus-5-5", "claude-sonnet-4-6"]);
    assert_eq!(
        claudedesktop_alias("space-bunny", "", &new).unwrap(),
        "claude-opus-5-5"
    );
    // 清单里第一个不合法时**跳过**它取下一个，而不是整个失败
    let dirty = cands(&["glm-5.3", "claude-opus-5-5"]);
    assert_eq!(
        claudedesktop_alias("space-bunny", "", &dirty).unwrap(),
        "claude-opus-5-5"
    );
    // 清单空了 / 全不合法 → 报错（调用方退化成「不伪装」），绝不猜一个名字
    assert!(claudedesktop_alias("space-bunny", "", &[]).is_err());
    assert!(claudedesktop_alias("space-bunny", "", &cands(&["glm-5.3"])).is_err());
}

/// 启动路径与写盘**共用**同一个接缝：两个桌面端走别名规则，其余工具原样。
///
/// 断言写成「等于清单里第一个合法项」而不是钉死某个具体型号 —— 清单本来就是
/// 要随官方在售型号变的数据，测试钉死型号等于给「加新模型」设一个隐形地雷。
#[test]
fn effective_claimed_model_only_masquerades_for_desktop_apps() {
    for (tool, is_legal) in [
        (
            "claudedesktop",
            is_legal_claudedesktop_model as fn(&str) -> bool,
        ),
        ("chatgptdesktop", is_legal_openai_model as fn(&str) -> bool),
    ] {
        let got = effective_claimed_model(tool, "space-bunny", "");
        let want = first_legal(builtin_candidates(tool), is_legal)
            .unwrap_or_else(|| panic!("{tool} 的 builtinModels 清单里应有合法项"));
        assert_eq!(got, want, "{tool} 的兜底名必须取清单第一项");
        assert!(is_legal(&got), "{tool} 兜底名 {got} 必须合法");
        // 手填的声明名合法时优先用它
        let preferred = if tool == "claudedesktop" {
            "claude-opus-4"
        } else {
            "gpt-4o"
        };
        assert_eq!(
            effective_claimed_model(tool, "space-bunny", preferred),
            preferred
        );
    }
    // 其余工具必须**原样**（别让这次改动波及其它工具）
    assert_eq!(
        effective_claimed_model("claude", "space-bunny", ""),
        "space-bunny"
    );
    assert_eq!(
        effective_claimed_model("codebuddy", "space-bunny", ""),
        "space-bunny"
    );
    // Codex CLI 有手填伪装框，没填就不伪装（保持原行为）
    assert_eq!(
        effective_claimed_model("codex-cli", "space-bunny", ""),
        "space-bunny"
    );
    // 非桌面端工具：声明名照旧优先
    assert_eq!(
        effective_claimed_model("codex-cli", "space-bunny", "my-alias"),
        "my-alias"
    );
}

/// **防「写 A 注册 B」**：profile 里写出的 `name` 必须与启动时注册进
/// `model_aliases` 的键**完全一致**。不一致时 Desktop 发的名字无人认领 → 上游 400。
#[test]
fn profile_name_matches_the_alias_the_proxy_registers() {
    for (real, preferred) in [
        ("space-bunny", ""),       // 没配声明名 → 取清单第一项
        ("glm-5.3", ""),           // 别家模型 → 取清单第一项
        ("space-bunny", "sonnet"), // 合法声明名 → 用它
        ("claude-opus-4", ""),     // 真实名本来就合法 → 用它
    ] {
        let layout = probe_layout("alias-consistency");
        let m = ModelWrite {
            model: real,
            claimed: preferred,
            base_url: "http://127.0.0.1:15721/",
            api_key: "kira-token",
            upstream_url: "",
            one_m: false,
        };
        // 写盘这条路径会读注册表取清单；清单读不到就该在这里报出人能看懂的话，
        // 而不是后面一个莫名的 unwrap panic
        let candidates = builtin_candidates(CLAUDESKTOP_WRITER);
        assert!(
            !candidates.is_empty(),
            "读不到 claudedesktop 的 builtinModels（ai-tools 目录没找到？清单被清空了？）"
        );
        write_claudedesktop_with(&layout, &m).unwrap();

        // 直接读 profile JSON 的 `name`（不能用 read_claudedesktop —— 它读的是
        // labelOverride，那是给界面看的真实模型名，语义不同）
        let profile_path = layout
            .lib_dir
            .join(format!("{CLAUDE_DESKTOP_PROFILE_ID}.json"));
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&profile_path).unwrap()).unwrap();
        let written = doc["inferenceModels"][0]["name"]
            .as_str()
            .unwrap_or_default();
        let label = doc["inferenceModels"][0]["labelOverride"]
            .as_str()
            .unwrap_or_default();

        let registered = effective_claimed_model("claudedesktop", real, preferred);
        assert_eq!(
            written, registered,
            "real={real} preferred={preferred:?}：profile 写的与代理注册的必须一致"
        );
        assert!(
                is_legal_claudedesktop_model(written),
                "real={real} preferred={preferred:?}：写进 profile 的名字必须被 Desktop 接受（得到 {written}）"
            );
        assert_eq!(label, real, "labelOverride 必须是真实模型名，供界面显示");
        let _ = std::fs::remove_dir_all(layout.lib_dir.parent().unwrap());
    }
}

/// **闭环**：别名经 `map_model_name`（`apply_masquerade` 实际调用的函数）必须映射回真实模型。
///
/// 这条测试存在的原因：`map_model_name` 内置了一张 **role 解析表**
/// （`claude-sonnet-4-*` → role `sonnet`）。别名 `claude-sonnet-4-6` 天然会被它认成
/// sonnet 系列 —— 一旦别名没被**精确**注册进 `role_map`，就会落到 role 映射甚至
/// `default_model`，**静默路由到别的模型**（不报错，只是答非所问）。
/// 所以必须证明「精确键优先」这条路径真的成立。
#[test]
fn alias_round_trips_back_to_the_real_model_through_map_model_name() {
    use crate::proxy::transform::{map_model_name, ModelAliases};
    use std::collections::HashMap;

    for real in ["space-bunny", "glm-5.3", "hy3", "kimi-k3"] {
        let alias = claudedesktop_alias(real, "", &cands(CLAUDE_LIST)).unwrap();
        // 模拟 launch.rs 注册：model_aliases[别名] = 真实模型
        let mut role_map = HashMap::new();
        role_map.insert(alias.clone(), real.to_string());
        let aliases = ModelAliases {
            default_model: None,
            role_map,
        };
        assert_eq!(
            map_model_name(&alias, &aliases),
            real,
            "别名 {alias} 必须精确映射回 {real}，不能被内置 role 表劫持"
        );
    }
}

/// 反面：只注册 role（`sonnet`）而**没有**精确键时，别名不会命中真实模型 ——
/// 记录这个事实，说明"精确键必须注册"不是可选项。
#[test]
fn role_only_mapping_would_not_catch_the_alias() {
    use crate::proxy::transform::{map_model_name, ModelAliases};
    use std::collections::HashMap;

    let alias = claudedesktop_alias("space-bunny", "", &cands(CLAUDE_LIST)).unwrap();
    // 只有 role 键，没有别名精确键
    let mut role_map = HashMap::new();
    role_map.insert("sonnet".to_string(), "some-other-model".to_string());
    let aliases = ModelAliases {
        default_model: None,
        role_map,
    };
    let mapped = map_model_name(&alias, &aliases);
    assert_ne!(
        mapped, "space-bunny",
        "只靠 role 映射拿不到 space-bunny —— 这正是 launch.rs 必须注册精确键的原因"
    );
}

/// 后端命令 `resolve_claimed_model` 的行为：桌面端走别名，其余原样。
/// 前端用它把「实际生效的伪装名」显示到底部，所以语义必须与 profile 写入一致。
#[test]
fn resolve_claimed_model_command_matches_what_we_write_to_profile() {
    // Claude Desktop + 真实模型不合法 → 取清单第一项
    assert_eq!(
        resolve_claimed_model("claudedesktop", "space-bunny", ""),
        first_legal(
            builtin_candidates("claudedesktop"),
            is_legal_claudedesktop_model
        )
        .unwrap()
    );
    // Claude Desktop + 手填合法声明名 → 用它
    assert_eq!(
        resolve_claimed_model("claudedesktop", "space-bunny", "claude-opus-4"),
        "claude-opus-4"
    );
    // 其它工具 → 原样（前端据此判断「没发生伪装」，不显示映射行）
    assert_eq!(resolve_claimed_model("claude", "glm-5.3", ""), "glm-5.3");
    assert_eq!(
        resolve_claimed_model("codex-cli", "gpt-5", "my-alias"),
        "my-alias"
    );
}

/// 未知 / 空工具 id 也不能 panic —— 前端会把各种 id 传进来
#[test]
fn resolve_claimed_model_tolerates_unknown_tool() {
    assert_eq!(resolve_claimed_model("", "", ""), "");
    assert_eq!(resolve_claimed_model("some-unknown-tool", "m", ""), "m");
}

// ═══════════════ ChatGPT 桌面端（OpenAI 侧）═══════════════

/// 官方名判定：家族词命中且不沾别家 → 合法。
///
/// 黑名单**优先于**家族词，所以 `gpt-5.3-claude` 这种「像 OpenAI 但命中 Anthropic」
/// 必须判非法 —— 否则会把别家模型伪装成 OpenAI 名写进 App，出了错极难查。
#[test]
fn openai_legality_accepts_official_names_only() {
    for ok in [
        "gpt-5.1-codex",
        "gpt-4o",
        "GPT-5.5",
        "gpt-6-astra",
        "codex-mini-latest",
        "o3",
        "o4-mini",
    ] {
        assert!(is_legal_openai_model(ok), "{ok} 应判为官方名");
    }
    for bad in [
        "",
        "space-bunny", // 自家/自定义名
        "glm-5.3",     // 别家
        "claude-sonnet-4-6",
        "gpt-5.3-claude",  // 沾别家 → 黑名单优先
        "qwen3-max-codex", // 沾别家
        "foo3",            // o3 必须整词，foo3 不算
        "v3",
    ] {
        assert!(!is_legal_openai_model(bad), "{bad} 不该判为官方名");
    }
}

/// 兜底名的取值顺序：合法声明名 → 合法真实名 → 清单第一项 → Err。
/// 清单内容与 `ai-tools/chatgptdesktop/config.json` 一致即可，不必与它同步维护。
#[test]
fn chatgpt_alias_prefers_legal_claimed_then_legal_real_then_candidates() {
    let c = cands(&["gpt-6-astra", "gpt-5.1-codex"]);
    // 真实名不是官方名 → 取清单第一项（最新在售）
    assert_eq!(
        chatgptdesktop_alias("space-bunny", "", &c).unwrap(),
        "gpt-6-astra"
    );
    // 真实名本来就是官方名 → 不伪装（写出去的与真实一致，代理也不必注册映射）
    assert_eq!(chatgptdesktop_alias("gpt-4o", "", &c).unwrap(), "gpt-4o");
    // 手填了合法声明名 → 用它
    assert_eq!(
        chatgptdesktop_alias("space-bunny", "gpt-5.5", &c).unwrap(),
        "gpt-5.5"
    );
    // 手填了**非法**声明名 → 不能照抄（那正是模型选择器要剔掉的），回落清单第一项
    assert_eq!(
        chatgptdesktop_alias("space-bunny", "glm-5.3", &c).unwrap(),
        "gpt-6-astra"
    );
    // 清单退役后兜底跟着变：代码里没有写死任何型号
    assert_eq!(
        chatgptdesktop_alias("space-bunny", "", &cands(&["gpt-5.1-codex"])).unwrap(),
        "gpt-5.1-codex"
    );
    // 清单空 / 全不合法 → Err（调用方退化成不伪装）
    assert!(chatgptdesktop_alias("space-bunny", "", &[]).is_err());
    assert!(chatgptdesktop_alias("space-bunny", "", &cands(&["glm-5.3"])).is_err());
}

/// **闭环**：OpenAI 别名经 `map_model_name` 必须精确映射回真实模型。
///
/// `map_model_name` 内置的 role 表只认 sonnet/opus/haiku/fable，理论上劫持不了
/// `gpt-*`；但这条是「别名注册漏了就会静默路由到 default_model」的护栏，
/// 与 Claude 那条同源。
#[test]
fn openai_alias_round_trips_through_map_model_name() {
    use crate::proxy::transform::{map_model_name, ModelAliases};
    use std::collections::HashMap;

    for real in ["space-bunny", "glm-5.3", "minimax-m3"] {
        let alias = chatgptdesktop_alias(real, "", &cands(&["gpt-6-astra"])).unwrap();
        let mut role_map = HashMap::new();
        role_map.insert(alias.clone(), real.to_string());
        let aliases = ModelAliases {
            default_model: None,
            role_map,
        };
        assert_eq!(
            map_model_name(&alias, &aliases),
            real,
            "别名 {alias} 必须精确映射回 {real}"
        );
    }
}

/// 命令层（前端底部显示用的那个）必须与写盘用的是同一套规则。
#[test]
fn resolve_claimed_model_gives_chatgpt_desktop_an_official_name() {
    let got = resolve_claimed_model("chatgptdesktop", "space-bunny", "");
    assert!(
        is_legal_openai_model(&got),
        "兜底必须是自己清单里的官方名，拿到 {got}"
    );
    // 已经是官方名 → 原样，前端就不会显示「伪装」
    assert_eq!(
        resolve_claimed_model("chatgptdesktop", "gpt-5.1", ""),
        "gpt-5.1"
    );
}
