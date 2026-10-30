//! 自定义「写模型」：schema 复杂到没法用「路径 → 值」声明的工具。
//!
//! 复刻自 EchoBird（`E:\pro\other-sdk\ai-tools\EchoBird`）：
//! `tools/*/config.json` 用 `"custom": true` 标记，真正的写入在
//! `src-tauri/src/services/tool_config_manager/<tool>.rs` 里整份生成配置。
//! 我们这里放同一套东西（目前只剩 Claude Desktop 一个，后续按需加）。
//!
//! 为什么需要它：Claude Desktop 走官方的「第三方推理网关（3P）」机制 ——
//! 要同时维护 `Claude-3p/configLibrary/<profileId>.json`、`_meta.json` 的
//! `appliedId`/`entries[]`，以及两个顶层配置里的 `deploymentMode`，
//! 通用的「路径 → 值」声明表达不了这种**跨文件互相引用**的结构。

use std::path::{Path, PathBuf};

/// 一次「把模型写进工具配置」的输入（两条调用链共用：启动时写 / 只保存模型）。
pub struct ModelWrite<'a> {
    /// 实际要发给上游的模型 id
    pub model: &'a str,
    /// 声明给工具的模型名（配了伪装就是伪装名，否则等于 model）
    pub claimed: &'a str,
    /// 工具实际请求的 base URL（启动时代理会接管 → 指向 127.0.0.1）
    pub base_url: &'a str,
    pub api_key: &'a str,
    /// 真实上游端点（仅用于展示「厂商」这类元信息；代理模式下 base_url 是本机回环，
    /// 拿它当厂商名会显示成 127.0.0.1）
    pub upstream_url: &'a str,
    /// 用户是否为该模型勾了 1M 上下文（Claude Desktop 的 1M 变体开关）
    pub one_m: bool,
}

/// 按写入器名分派。`None` 之外的未知名字直接报错（配置写错要立刻看得见）。
pub fn write_config(writer: &str, path: &Path, m: &ModelWrite<'_>) -> Result<(), String> {
    match writer {
        "claudedesktop" => write_claudedesktop(path, m),
        other => Err(format!("未知的自定义配置写入器: {other}")),
    }
}

/// 读回「当前写定的模型」（界面回显用）。
pub fn read_model(writer: &str, path: &Path) -> Option<String> {
    match writer {
        "claudedesktop" => read_claudedesktop(path),
        _ => None,
    }
}

// ════════════════════════════════════════════════════════════════
//  Claude Desktop → 官方支持的「第三方推理网关（3P）」机制
//
//  抄 EchoBird `tool_config_manager/claudedesktop.rs`（它反过来又参考 cc-switch）：
//  Claude Desktop 不读 ~/.claude/settings.json（那是 Claude Code 的），它的自定义
//  端点走另一套：
//   1. 两个顶层配置里 `deploymentMode = "3p"`（官方配置目录 + `Claude-3p` 目录）；
//   2. `Claude-3p/configLibrary/<profileId>.json` 描述网关地址、凭据与模型清单；
//   3. 同目录 `_meta.json` 的 `appliedId` 告诉它哪份 profile 生效、`entries[]` 供选择器显示名字。
//
//  与 EchoBird 的差别：它的默认「bridge 模式」指向自家的常驻代理（靠代理把 Desktop 写死的
//  claude-* 模型名改写成真实模型）。我们没有常驻代理，所以只走「relay 模式」——直接写
//  Kira 本次给的 base_url / api_key（启动 Kira 代理时就是本机代理，只保存模型时就是真实上游）。
// ════════════════════════════════════════════════════════════════

/// 我们写的 profile id（EchoBird 用自己的 UUID，各写各的，互不覆盖）。
const CLAUDE_DESKTOP_PROFILE_ID: &str = "9f2c1d47-5b6e-4a83-9c07-2f6a1b8d3e50";
const CLAUDE_DESKTOP_PROFILE_NAME: &str = "Kira";

/// Claude Desktop 的三个落点（按平台解析，测试可注入临时目录）。
pub struct ClaudeDesktopLayout {
    /// 官方配置目录里的 `claude_desktop_config.json`
    pub official_cfg: PathBuf,
    /// `Claude-3p` 目录里的 `claude_desktop_config.json`
    pub threep_cfg: PathBuf,
    /// `Claude-3p/configLibrary`
    pub lib_dir: PathBuf,
}

/// 按平台解析 Claude Desktop 的目录：
/// - Windows：`%LOCALAPPDATA%\Claude` 与 `%LOCALAPPDATA%\Claude-3p`
/// - macOS：`~/Library/Application Support/Claude` 与 `.../Claude-3p`
pub fn claude_desktop_layout() -> Option<ClaudeDesktopLayout> {
    let home = crate::commands::utils::get_home_dir();

    #[cfg(windows)]
    let (official_dir, threep_dir) = {
        let local = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData").join("Local"));
        (local.join("Claude"), local.join("Claude-3p"))
    };

    #[cfg(target_os = "macos")]
    let (official_dir, threep_dir) = {
        let app_support = home.join("Library").join("Application Support");
        (app_support.join("Claude"), app_support.join("Claude-3p"))
    };

    #[cfg(not(any(windows, target_os = "macos")))]
    let (official_dir, threep_dir) = {
        let _ = home;
        return None;
    };

    Some(ClaudeDesktopLayout {
        official_cfg: official_dir.join("claude_desktop_config.json"),
        threep_cfg: threep_dir.join("claude_desktop_config.json"),
        lib_dir: threep_dir.join("configLibrary"),
    })
}

/// 把某个顶层配置文件的 `deploymentMode` 设成 `3p`，其余键原样保留
/// （`mcpServers`、窗口尺寸、遥测开关都是用户自己的东西，不能整份覆盖）。
fn set_deployment_mode(path: &Path, mode: &str) -> Result<(), String> {
    let mut doc = read_json_or_empty(path);
    if !doc.is_object() {
        doc = serde_json::json!({});
    }
    if let Some(obj) = doc.as_object_mut() {
        obj.insert("deploymentMode".into(), serde_json::json!(mode));
    }
    write_json(path, &doc)
}

fn write_claudedesktop(_declared_path: &Path, m: &ModelWrite<'_>) -> Result<(), String> {
    let Some(layout) = claude_desktop_layout() else {
        return Err("Claude Desktop 只支持 Windows 与 macOS".to_string());
    };
    write_claudedesktop_with(&layout, m)
}

/// 落盘逻辑（与平台解耦，便于测试）。
pub fn write_claudedesktop_with(
    layout: &ClaudeDesktopLayout,
    m: &ModelWrite<'_>,
) -> Result<(), String> {
    let base = m.base_url.trim().trim_end_matches('/');
    if base.is_empty() {
        return Err("Claude Desktop 需要网关地址（base URL），请先选择供应商与模型".to_string());
    }
    // 本机上游（llama.cpp / vllm 这类）没有鉴权，Desktop 又不接受空 key：给个非空占位。
    let is_local = base.contains("127.0.0.1") || base.contains("localhost");
    let api_key = if m.api_key.trim().is_empty() {
        if is_local {
            "local-no-auth"
        } else {
            return Err("Claude Desktop 需要 API Key，请先给供应商填 Key".to_string());
        }
    } else {
        m.api_key.trim()
    };

    let real_model = if m.model.trim().is_empty() {
        m.claimed.trim()
    } else {
        m.model.trim()
    };
    if real_model.is_empty() {
        return Err("Claude Desktop 需要一个模型 id".to_string());
    }
    // Desktop 发出的 model 就是这里的 `name`；Kira 的代理按声明模型做映射，
    // 所以写声明名（有伪装就是伪装名），真实 id 放 labelOverride 供界面显示。
    //
    // 声明名必须**过 Desktop 的校验**：它会拿 `qa(name)` 剔掉非 Anthropic 模型名，
    // 剔完选择器就空了，而提示只有一句含糊的 warning（实测见 design 文档）。
    // 所以这里统一走 `claudedesktop_alias`：声明名合法就用它，否则取 builtinModels
    // 清单里第一个合法项；清单里也没有合法项时**报错不写盘** —— 写一个 Desktop 必然剔除的
    // profile 进去，只会把问题拖到启动时才发现。
    let sent_model = claudedesktop_alias(
        &real_model,
        m.claimed,
        builtin_candidates(CLAUDESKTOP_WRITER),
    )?;

    set_deployment_mode(&layout.official_cfg, "3p")?;
    set_deployment_mode(&layout.threep_cfg, "3p")?;
    ensure_dir(&layout.lib_dir)
        .map_err(|e| format!("创建 Claude Desktop profile 目录失败: {e}"))?;

    // `inferenceModels` 直接填充 Desktop 的模型选择器，并**跳过**它对网关的
    // `/v1/models` 探测（我们不实现那个接口，缺了它 Desktop 会每次弹网关错误）。
    let profile = serde_json::json!({
        "disableDeploymentModeChooser": true,
        "inferenceProvider": "gateway",
        "inferenceGatewayBaseUrl": base,
        "inferenceGatewayApiKey": api_key,
        "inferenceGatewayAuthScheme": "bearer",
        "inferenceModels": [{
            "name": sent_model,
            "labelOverride": real_model,
            "supports1m": true,
            "prefer1m": m.one_m,
        }],
        // 让内置 web_fetch 能出网：不给这个字段，出网策略回落到「仅网关主机」。
        "coworkEgressAllowedHosts": ["*"],
    });
    write_json(
        &layout
            .lib_dir
            .join(format!("{CLAUDE_DESKTOP_PROFILE_ID}.json")),
        &profile,
    )?;

    // `_meta.json`：Desktop 靠 appliedId 知道哪份 profile 生效，靠 entries 在选择器里显示名字。
    let meta = serde_json::json!({
        "appliedId": CLAUDE_DESKTOP_PROFILE_ID,
        "entries": [{ "id": CLAUDE_DESKTOP_PROFILE_ID, "name": CLAUDE_DESKTOP_PROFILE_NAME }],
    });
    write_json(&layout.lib_dir.join("_meta.json"), &meta)?;

    eprintln!(
        "[config_file] Claude Desktop 3P profile 已写入 {}（模型 {} → {}）",
        layout.lib_dir.display(),
        real_model,
        base
    );
    Ok(())
}

/// 把 Claude Desktop 还原成官方模式：两份配置的 `deploymentMode` 由 `3p` 回到 `1p`，
/// 删掉我们写的 profile 与 `_meta.json`（对应 EchoBird 的 `restore_claudedesktop_to_official`）。
pub fn restore_claudedesktop() -> Result<super::tool_config_restore::RestoreOutcome, String> {
    let Some(layout) = claude_desktop_layout() else {
        return Err("Claude Desktop 只支持 Windows 与 macOS".to_string());
    };
    let mut outcome = super::tool_config_restore::RestoreOutcome::default();
    for cfg in [&layout.official_cfg, &layout.threep_cfg] {
        if !cfg.exists() {
            continue;
        }
        set_deployment_mode(cfg, "1p")?;
        outcome.files.push(cfg.display().to_string());
    }
    for file in [
        layout
            .lib_dir
            .join(format!("{CLAUDE_DESKTOP_PROFILE_ID}.json")),
        layout.lib_dir.join("_meta.json"),
    ] {
        if file.exists() {
            std::fs::remove_file(&file)
                .map_err(|e| format!("删除 {} 失败: {e}", file.display()))?;
            outcome.files.push(file.display().to_string());
        }
    }
    if outcome.files.is_empty() {
        outcome
            .notes
            .push("没有发现 Kira 写入的 3P profile，无需还原".to_string());
    } else {
        outcome.notes.push(
            "已切回官方模式（deploymentMode=1p）并删除 3P profile，需完全退出再打开 Claude Desktop"
                .to_string(),
        );
    }
    Ok(outcome)
}

// ═══════════════ 模型伪装：Desktop 的模型名校验 ═══════════════
//
// 全部规则来自 Claude Desktop 1.44121.2 的 `app.asar`（`inferenceProvider: "gateway"`
// 走 `e_e(name)` → `qa(name)`）。原文：
//   Ka  = ["sonnet","opus","haiku","fable","mythos"]
//   Gge = ["claude", ...Ka, "anthropic"]
//   Kge = /ark-code|deepseek|glm|gpt|hy3|kimi|openai|qwen|hunyuan|codex|…/
//   qa(name) = !Kge.test(name) && (Wge.test(name) || Gge.some(t => name.includes(t)))
// 其中 Wge = `^(sonnet|opus|haiku|fable|mythos)(-[\d.]+)?$`。
//
// **Desktop 拿不到模型名时的行为**（`xCe`）：把它从 `inferenceModels` 里**静默剔除**
// 并弹一句 warning —— 不会告诉你填错了什么。所以我们必须在**写盘前**就自己判 legality。

/// Desktop 认的 Anthropic 家族词（`Gge` = claude + Ka + anthropic）
const CLAUDE_DESKTOP_FAMILY_WORDS: &[&str] = &[
    "claude",
    "sonnet",
    "opus",
    "haiku",
    "fable",
    "mythos",
    "anthropic",
];

/// 别家模型黑名单（`Kge` 的字面量部分，原样转录）
const CLAUDE_DESKTOP_BLACKLIST: &[&str] = &[
    "ark-code",
    "astron",
    "command-r",
    "deepseek",
    "doubao",
    "gemini",
    "gemma",
    "glm",
    "gpt",
    "grok",
    "hermes",
    "hy3",
    "kimi",
    "lfm",
    "llama",
    "longcat",
    "mimo",
    "minimax",
    "mistral",
    "mixtral",
    "moonshot",
    "nemotron",
    "openai",
    "phi-",
    "qianfan",
    "qwen",
    "tc-code",
    "yi-",
    "stepfun",
    "step-3",
    "seed-",
    "bytedance",
    "hunyuan",
    "granite",
    "amazon.nova",
    "nova-",
    "devstral",
    "ministral",
    "ernie",
    "codex",
    "arcee",
    "trinity",
    "abab",
    "k2.",
    "m2.",
    "jamba",
    "arctic",
    "solar",
    "mercury",
    "zamba",
    "kat-coder",
    "dpsk",
];

/// 黑名单里带 `\b` 词边界的项：必须**整词**匹配，否则 "streaming" 里的 "ling" 会误伤
const CLAUDE_DESKTOP_BLACKLIST_WORDS: &[&str] = &["ling", "unic"];

/// 判断 Claude Desktop 会不会接受这个模型名。
///
/// 判定顺序**必须**与 `qa()` 一致：先黑名单、再要求含 Anthropic 家族词。
/// 反过来就会误放行 `glm-5.3-claude`、`qwen-claude-sonnet` 这类「含 Anthropic 词
/// 但也命中别家」的名字。
pub fn is_legal_claudedesktop_model(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return false;
    }
    if is_claudedesktop_blacklisted(&lower) {
        return false;
    }
    // 原文的 `Wge.test(name) ||` 分支是**冗余**的：能匹配
    // `^(sonnet|opus|haiku|fable|mythos)(-[0-9.]+)?$` 的名字必然含其中某个家族词，
    // 而这些词都在 `Gge` 里。所以只留 contains 判断（`sonnet` / `opus-4-5` 仍合法）。
    CLAUDE_DESKTOP_FAMILY_WORDS
        .iter()
        .any(|w| lower.contains(w))
}

fn is_claudedesktop_blacklisted(lower: &str) -> bool {
    if CLAUDE_DESKTOP_BLACKLIST.iter().any(|k| lower.contains(k)) {
        return true;
    }
    CLAUDE_DESKTOP_BLACKLIST_WORDS
        .iter()
        .any(|w| contains_whole_word(lower, w))
}

/// 整词包含（对应正则的 `\b…\b`）
fn contains_whole_word(haystack: &str, word: &str) -> bool {
    haystack.match_indices(word).any(|(i, _)| {
        let before_ok = haystack[..i]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());
        let after = i + word.len();
        let after_ok = haystack[after..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric());
        before_ok && after_ok
    })
}

/// 算出写进 Claude Desktop profile 的 `inferenceModels[].name`。
///
/// **这是唯一的别名来源** —— profile 写入与代理的 `model_aliases` 注册都必须调它。
/// 各算一次会出现「profile 写 A、代理注册 B」，Desktop 发的名字无人认领 → 400。
///
/// `candidates` = 该工具 `builtinModels` 清单（按**新 → 旧**维护，退役的删掉、新款加前面）。
/// 兜底取清单里第一个合法项 —— **刻意不在代码里写死模型名**：写死的那个迟早会变成
/// 已废弃的名字，而用户看不出任何异常（只会发现「怎么突然开始用回官方模型了」）。
/// 清单为空或全不合法时返回 `Err`，由 [`effective_claimed_model`] 退化成「不伪装」。
///
/// 优先级：合法声明名 → 合法真实模型名 → 清单里第一个合法项 → Err（退化成不伪装）。
pub fn claudedesktop_alias(
    real_model: &str,
    preferred: &str,
    candidates: &[String],
) -> Result<String, String> {
    let p = preferred.trim();
    if !p.is_empty() && is_legal_claudedesktop_model(p) {
        return Ok(p.to_string());
    }
    let real = real_model.trim();
    if is_legal_claudedesktop_model(real) {
        return Ok(real.to_string());
    }
    if let Some(ok) = first_legal(candidates, is_legal_claudedesktop_model) {
        return Ok(ok);
    }
    Err(format!(
        "无法为 Claude Desktop 生成合法的模型名：真实模型 `{real_model}` 与声明名 `{preferred}` \
         都不被 Desktop 接受，且 builtinModels 清单里也没有合法项（当前 {} 项）—— \
         清单按「新 → 旧」维护，退役的型号要删掉、补上在售的新型号",
        candidates.len()
    ))
}

/// Claude Desktop 的自定义写入器名（`ai-tools/claudedesktop/config.json` 的
/// `configFile.custom`）。也正好等于它的工具 id —— [`effective_claimed_model`] 按工具 id
/// 分派，两者一致才不会分叉。
pub const CLAUDESKTOP_WRITER: &str = "claudedesktop";

// ═══════════════ 模型伪装：ChatGPT 桌面端（Codex 内核）的模型名校验 ═══════════════
//
// 与 Claude Desktop 同一个道理，只是家族词换成 OpenAI 的：桌面端的模型选择器只认官方
// 模型名，`~/.codex/config.toml` 里写 `space-bunny` 这类第三方 id 时，App 界面上显示的
// 就是那个第三方名字；用户在 App 里改一次模型，代理这边没人认领 → 回落上游 → 404 /
// 答非所问。所以真实模型名不是官方 OpenAI 名时，自动声明成一个官方名，由代理映射回去。
//
// **范围只限 ChatGPT 桌面端**：Codex CLI 那边 `builtinModels` 非空、用户可以手填，
// 保持「不填就不伪装」的现状（`effective_claimed_model` 里没有它的分支）。

/// ChatGPT/Codex 认的家族词。
const OPENAI_FAMILY_WORDS: &[&str] = &["gpt", "codex", "chatgpt", "openai"];

/// 别家模型黑名单：Claude 那张表的镜像（去掉 gpt / openai / codex 这些自家的）。
const OPENAI_BLACKLIST: &[&str] = &[
    "anthropic",
    "claude",
    "sonnet",
    "opus",
    "haiku",
    "fable",
    "mythos",
    "ark-code",
    "astron",
    "command-r",
    "deepseek",
    "doubao",
    "gemini",
    "gemma",
    "glm",
    "grok",
    "hermes",
    "hy3",
    "kimi",
    "lfm",
    "llama",
    "longcat",
    "mimo",
    "minimax",
    "mistral",
    "mixtral",
    "moonshot",
    "nemotron",
    "phi-",
    "qianfan",
    "qwen",
    "tc-code",
    "yi-",
    "stepfun",
    "step-3",
    "seed-",
    "bytedance",
    "hunyuan",
    "granite",
    "amazon.nova",
    "nova-",
    "devstral",
    "ministral",
    "ernie",
    "arcee",
    "trinity",
    "abab",
    "k2.",
    "m2.",
    "jamba",
    "arctic",
    "solar",
    "mercury",
    "zamba",
    "kat-coder",
    "dpsk",
];

/// 黑名单里带 `\b` 词边界的项（与 Claude 那份同源：`streaming` 里的 `ling` 不算命中）。
const OPENAI_BLACKLIST_WORDS: &[&str] = &["ling", "unic"];

/// ChatGPT 桌面端的工具 id。也是「这个工具需要 OpenAI 侧伪装」的判据。
pub const CHATGPTDESKTOP_TOOL: &str = "chatgptdesktop";

/// 判断 ChatGPT 桌面端会不会认这个模型名。
///
/// 判定顺序与 [`is_legal_claudedesktop_model`] 一致：先黑名单、再要求含 OpenAI 家族词。
/// 顺序反过来会误放行 `gpt-5.3-claude` 这种「像 OpenAI 但命中别家」的名字。
pub fn is_legal_openai_model(name: &str) -> bool {
    let lower = name.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return false;
    }
    if is_openai_blacklisted(&lower) {
        return false;
    }
    if OPENAI_FAMILY_WORDS.iter().any(|w| lower.contains(w)) {
        return true;
    }
    // 推理系列（o1 / o3 / o4…）没有 `gpt` 字样，但同样是官方名。
    // 按整词判：`foo3`、`v3` 这类不能被当成 o3。
    lower
        .split(|c: char| !c.is_alphanumeric())
        .any(is_openai_reasoning_token)
}

fn is_openai_blacklisted(lower: &str) -> bool {
    if OPENAI_BLACKLIST.iter().any(|k| lower.contains(k)) {
        return true;
    }
    OPENAI_BLACKLIST_WORDS
        .iter()
        .any(|w| contains_whole_word(lower, w))
}

/// `o` + 至少一位数字的整词（`o1` / `o3` / `o4-mini` 的首段）。
fn is_openai_reasoning_token(token: &str) -> bool {
    match token.strip_prefix('o') {
        Some(rest) => !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()),
        None => false,
    }
}

/// 清单里第一个合法项（清单按「新 → 旧」维护，所以第一项就是当前在售最新的那个）。
///
/// 跳过非法项而不是只看第一项：清单是手写的，保不齐每一项都合法（复制粘贴、手抖）。
fn first_legal(candidates: &[String], is_legal: fn(&str) -> bool) -> Option<String> {
    candidates
        .iter()
        .map(|c| c.trim())
        .find(|c| !c.is_empty() && is_legal(c))
        .map(str::to_string)
}

/// 该工具声明的官方名候选（`ai-tools/<id>/config.json` 的 `builtinModels`）。
///
/// 读不到工具声明时返回空切片 → 别名函数返回 `Err` → 退化成「不伪装」。
/// **刻意不兜一个硬编码模型名**：那正是这次要消灭的东西。
fn builtin_candidates(tool_id: &str) -> &[String] {
    crate::commands::ai_registry::registry()
        .get_tool_config(tool_id)
        .map(|c| c.builtin_models.as_slice())
        .unwrap_or(&[])
}

/// 算出写进 `~/.codex/config.toml` 的 `model`（声明名 C）。
///
/// 与 [`claudedesktop_alias`] 同构，**并且同样必须是唯一来源** —— 写盘与代理注册
/// `model_aliases` 都调 [`effective_claimed_model`]，各算一次就会「写 A、注册 B」。
///
/// `candidates` / 兜底取值的理由见 [`claudedesktop_alias`]：模型名来自清单，不写死在代码里。
pub fn chatgptdesktop_alias(
    real_model: &str,
    preferred: &str,
    candidates: &[String],
) -> Result<String, String> {
    let p = preferred.trim();
    if !p.is_empty() && is_legal_openai_model(p) {
        return Ok(p.to_string());
    }
    let real = real_model.trim();
    if is_legal_openai_model(real) {
        return Ok(real.to_string());
    }
    if let Some(ok) = first_legal(candidates, is_legal_openai_model) {
        return Ok(ok);
    }
    Err(format!(
        "无法为 ChatGPT 桌面端生成合法的模型名：真实模型 `{real_model}` 与声明名 `{preferred}` \
         都不是官方 OpenAI 模型名，且 builtinModels 清单里也没有合法项（当前 {} 项）—— \
         清单按「新 → 旧」维护，退役的型号要删掉、补上在售的新型号",
        candidates.len()
    ))
}

/// **工具实际会发出的模型名**（声明名 C）—— 启动路径注册 `model_aliases` 与写工具配置
/// 都必须调它，避免"配置写 A、代理注册 B"导致工具发的名字无人认领。
///
/// 只有两个桌面端需要伪装（它们的选择器只认自家官方模型名）：Claude Desktop 与
/// ChatGPT 桌面端；其余工具（Codex CLI / Claude Code / opencode …）**原样**返回，
/// 别让这个改动波及其它工具。
pub fn effective_claimed_model(tool_id: &str, real_model: &str, preferred: &str) -> String {
    let candidates = builtin_candidates(tool_id);
    match tool_id {
        CLAUDESKTOP_WRITER => {
            if let Ok(alias) = claudedesktop_alias(real_model, preferred, candidates) {
                return alias;
            }
            // 清单里没有合法项（工具声明缺失 / 清单被清空）→ 退化成「不伪装」，
            // 宁可让 Desktop 看到真实名字，也不要伪装成一个可能已废弃的官方名。
        }
        CHATGPTDESKTOP_TOOL => {
            if let Ok(alias) = chatgptdesktop_alias(real_model, preferred, candidates) {
                return alias;
            }
        }
        _ => {}
    }
    let p = preferred.trim();
    if !p.is_empty() {
        return p.to_string();
    }
    real_model.trim().to_string()
}

/// Tauri 命令：解析某个工具**实际会发出的**模型名（声明名 C）。
///
/// 前端底部要显示「用什么模型 / 伪装什么模型」，但别名规则（两套黑名单）只在 Rust 一处
/// 实现 —— 前端复制一份就会出现第二个来源，必然漂移。所以由前端调这个拿结果。
#[tauri::command]
pub fn resolve_claimed_model(tool_id: &str, real_model: &str, preferred: &str) -> String {
    effective_claimed_model(tool_id, real_model, preferred)
}

fn read_claudedesktop(_declared_path: &Path) -> Option<String> {
    let layout = claude_desktop_layout()?;
    read_json_or_empty(
        &layout
            .lib_dir
            .join(format!("{CLAUDE_DESKTOP_PROFILE_ID}.json")),
    )
    .get("inferenceModels")?
    .as_array()?
    .first()?
    .get("labelOverride")
    .and_then(|v| v.as_str())
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty())
}

/// 读 JSON 文件；不存在 / 解析失败一律当空对象（写回时只动我们的键）。
fn read_json_or_empty(path: &Path) -> serde_json::Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .unwrap_or_else(|| serde_json::json!({}))
}

/// 确保目录可用，**能处理「断链的重解析点」**（Windows 上 junction 指向的目标被删掉）。
///
/// 为什么不能直接用 `std::fs::create_dir_all`：junction 本身还在、目标没了时，
/// `mkdir` 返回 `ERROR_ALREADY_EXISTS(183)`，紧接着 std 会用跟随链接的 `is_dir()` 判断，
/// 链接断了就得到 false → 把 183 当成真错误抛出来，报的还是「当文件已存在时，无法创建该文件」，
/// 用户完全看不懂，也想不到是链接的问题。
///
/// 2026-09-29 真机实测到：`%LOCALAPPDATA%\Claude-3p` 被 junction 到 `D:\sim-tool\Claude-3p`，
/// 后者被删 → 给 Claude Desktop 设置模型**必然失败**（就是上面那条报错）。
///
/// 这里的做法：正常目录直接放行；失败时按链接把**目标目录补出来**（用户的意图就是数据放那边，
/// 不该因为目标被删就整体失败），补完仍不行才报错，并且说明是链接的问题。
pub(crate) fn ensure_dir(path: &Path) -> Result<(), String> {
    if path.is_dir() {
        return Ok(());
    }
    if std::fs::create_dir_all(path).is_ok() {
        return Ok(());
    }
    // 走到这里通常是断链的重解析点：把链接目标建出来
    if let Ok(raw) = std::fs::read_link(path) {
        let target = normalize_link_target(path, &raw);
        if let Err(e) = std::fs::create_dir_all(&target) {
            return Err(format!(
                "无法创建目录 {}：它是一个链接，指向 {}，而该目标也建不出来（{e}）",
                path.display(),
                target.display()
            ));
        }
        if path.is_dir() {
            return Ok(());
        }
    }
    Err(format!(
        "无法创建目录 {}（它可能是指向已删除目标的链接/联结，删除后重试即可）",
        path.display()
    ))
}

/// 把 `read_link` 给出的原始目标规整成可用路径。
///
/// Windows 上 junction 的目标形如 `\??\D:\sim-tool\Claude-3p`（NT 对象管理器前缀），
/// 必须去掉该前缀才能当普通路径用；相对目标按链接所在目录展开。
fn normalize_link_target(link: &Path, raw: &Path) -> PathBuf {
    let text = raw.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\??\") {
        return PathBuf::from(rest);
    }
    if raw.is_absolute() {
        return raw.to_path_buf();
    }
    link.parent().unwrap_or(Path::new(".")).join(raw)
}

/// 写 JSON（serde_json 输出天然 UTF-8 无 BOM），父目录不存在则创建。
fn write_json(path: &Path, doc: &serde_json::Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        ensure_dir(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    let text = serde_json::to_string_pretty(doc).map_err(|e| format!("序列化失败: {e}"))?;
    std::fs::write(path, text).map_err(|e| format!("写入失败: {e}（{}）", path.display()))
}

#[cfg(test)]
mod tests;
