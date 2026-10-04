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
    // 所以这里统一走 `claudedesktop_alias`：声明名合法就用它，否则自动回落默认别名；
    // 连兜底都不合法（内置常量被改坏）时**报错不写盘** —— 写一个 Desktop 必然剔除的
    // profile 进去，只会把问题拖到启动时才发现。
    let sent_model = claudedesktop_alias(&real_model, m.claimed)?;

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
    write_json(&layout.lib_dir.join(format!("{CLAUDE_DESKTOP_PROFILE_ID}.json")), &profile)?;

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
        outcome
            .notes
            .push("已切回官方模式（deploymentMode=1p）并删除 3P profile，需完全退出再打开 Claude Desktop".to_string());
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
    "claude", "sonnet", "opus", "haiku", "fable", "mythos", "anthropic",
];

/// 别家模型黑名单（`Kge` 的字面量部分，原样转录）
const CLAUDE_DESKTOP_BLACKLIST: &[&str] = &[
    "ark-code", "astron", "command-r", "deepseek", "doubao", "gemini", "gemma", "glm", "gpt",
    "grok", "hermes", "hy3", "kimi", "lfm", "llama", "longcat", "mimo", "minimax", "mistral",
    "mixtral", "moonshot", "nemotron", "openai", "phi-", "qianfan", "qwen", "tc-code", "yi-",
    "stepfun", "step-3", "seed-", "bytedance", "hunyuan", "granite", "amazon.nova", "nova-",
    "devstral", "ministral", "ernie", "codex", "arcee", "trinity", "abab", "k2.", "m2.",
    "jamba", "arctic", "solar", "mercury", "zamba", "kat-coder", "dpsk",
];

/// 黑名单里带 `\b` 词边界的项：必须**整词**匹配，否则 "streaming" 里的 "ling" 会误伤
const CLAUDE_DESKTOP_BLACKLIST_WORDS: &[&str] = &["ling", "unic"];

/// 无伪装时的默认伪装名。它自身必须是合法名字（有测试钉住）。
const DEFAULT_CLAUDESKTOP_ALIAS: &str = "claude-sonnet-4-6";

/// 默认伪装名（`claude-sonnet-4-6`；这个版本里 Desktop 的定价示例就用 4-6）
pub fn default_claudedesktop_alias() -> &'static str {
    DEFAULT_CLAUDESKTOP_ALIAS
}

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
        let after_ok = haystack[after..].chars().next().is_none_or(|c| !c.is_alphanumeric());
        before_ok && after_ok
    })
}

/// 算出写进 Claude Desktop profile 的 `inferenceModels[].name`。
///
/// **这是唯一的别名来源** —— profile 写入与代理的 `model_aliases` 注册都必须调它。
/// 各算一次会出现「profile 写 A、代理注册 B」，Desktop 发的名字无人认领 → 400。
///
/// 优先级：合法声明名 → 合法真实模型名 → 内置兜底 → 报错（兜底自身不合法属代码被改坏）。
pub fn claudedesktop_alias(real_model: &str, preferred: &str) -> Result<String, String> {
    let p = preferred.trim();
    if !p.is_empty() && is_legal_claudedesktop_model(p) {
        return Ok(p.to_string());
    }
    let real = real_model.trim();
    if is_legal_claudedesktop_model(real) {
        return Ok(real.to_string());
    }
    let fallback = default_claudedesktop_alias();
    if is_legal_claudedesktop_model(fallback) {
        return Ok(fallback.to_string());
    }
    Err(format!(
        "无法为 Claude Desktop 生成合法的模型名：真实模型 `{real_model}` 与声明名 `{preferred}` \
         都不被 Desktop 接受，且内置兜底 `{fallback}` 自身也不合法 —— 这是内置常量被改坏了，请反馈"
    ))
}

/// Claude Desktop 的自定义写入器名（`ai-tools/claudedesktop/config.json` 的
/// `configFile.custom`）。也是"这个工具需要模型伪装"的判据。
pub const CLAUDESKTOP_WRITER: &str = "claudedesktop";

/// **工具实际会发出的模型名**（声明名 C）—— 启动路径注册 `model_aliases` 与写工具配置
/// 都必须调它，避免"profile 写 A、代理注册 B"导致 Desktop 发的名字无人认领。
///
/// 只有 Claude Desktop 需要伪装（它的选择器会剔除非 Anthropic 模型名）；其余写入器
/// **原样**返回，别让这个改动波及其它工具。
pub fn effective_claimed_model(writer: Option<&str>, real_model: &str, preferred: &str) -> String {
    if writer == Some(CLAUDESKTOP_WRITER) {
        if let Ok(alias) = claudedesktop_alias(real_model, preferred) {
            return alias;
        }
        // 生成不出来只可能是内置兜底常量被改坏了；这里不让启动直接失败，
        // 退回原逻辑（写盘那条路径会照实报错，见 write_claudedesktop_with）
    }
    let p = preferred.trim();
    if !p.is_empty() {
        return p.to_string();
    }
    real_model.trim().to_string()
}

fn read_claudedesktop(_declared_path: &Path) -> Option<String> {
    let layout = claude_desktop_layout()?;
    read_json_or_empty(&layout.lib_dir.join(format!("{CLAUDE_DESKTOP_PROFILE_ID}.json")))
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
mod tests {
    use super::{read_model, write_config, ModelWrite};
    use std::path::PathBuf;

    use super::{
        claude_desktop_layout, claudedesktop_alias, default_claudedesktop_alias,
        effective_claimed_model, is_legal_claudedesktop_model, write_claudedesktop_with,
        ClaudeDesktopLayout, CLAUDE_DESKTOP_PROFILE_ID,
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
        assert!(target.is_dir(), "链接目标应被建出来（裸调用是否成功={plain}）");
        std::fs::write(link.join("probe.json"), "{}").expect("修好后应能正常写入");

        // 清理：先删链接本身，再删整个临时根
        let _ = std::fs::remove_dir(&link);
        let _ = std::fs::remove_dir_all(&root);
    }

    fn probe_layout(name: &str) -> ClaudeDesktopLayout {
        let mut root = std::env::temp_dir();
        root.push(format!("anyver-claudedesktop-{}-{}", name, std::process::id()));
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
                layout.lib_dir.join("9f2c1d47-5b6e-4a83-9c07-2f6a1b8d3e50.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(profile["inferenceGatewayBaseUrl"], "http://127.0.0.1:15721");
        assert_eq!(profile["inferenceGatewayApiKey"], "kira-token");
        assert_eq!(profile["inferenceModels"][0]["name"], "claude-opus-4");
        assert_eq!(profile["inferenceModels"][0]["labelOverride"], "deepseek-chat");
        assert_eq!(profile["inferenceModels"][0]["prefer1m"], true);
        assert_eq!(profile["inferenceProvider"], "gateway");

        let meta: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(layout.lib_dir.join("_meta.json")).unwrap(),
        )
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

    /// 合法的伪装名原样使用（不改动用户填的声明名）
    #[test]
    fn alias_keeps_a_legal_preferred_name() {
        assert_eq!(claudedesktop_alias("space-bunny", "claude-sonnet-4-6").unwrap(), "claude-sonnet-4-6");
        assert_eq!(claudedesktop_alias("space-bunny", "anthropic/claude-opus-4").unwrap(), "anthropic/claude-opus-4");
        // Wge：纯家族名也合法（`^(sonnet|opus|haiku|fable|mythos)(-[0-9.]+)?$`）
        assert_eq!(claudedesktop_alias("space-bunny", "sonnet").unwrap(), "sonnet");
        assert_eq!(claudedesktop_alias("space-bunny", "opus-4-5").unwrap(), "opus-4-5");
    }

    /// 没配声明名 → 回落默认别名，且**不依赖真实模型名长什么样**
    #[test]
    fn alias_falls_back_when_preferred_is_absent_or_illegal() {
        let fallback = "claude-sonnet-4-6";
        // 没填
        assert_eq!(claudedesktop_alias("space-bunny", "").unwrap(), fallback);
        assert_eq!(claudedesktop_alias("space-bunny", "   ").unwrap(), fallback);
        // 填了但 Desktop 会剔除的：真实模型名、别家模型名
        assert_eq!(claudedesktop_alias("space-bunny", "space-bunny").unwrap(), fallback);
        assert_eq!(claudedesktop_alias("glm-5.3", "glm-5.3").unwrap(), fallback);
        assert_eq!(claudedesktop_alias("hy3", "hy3").unwrap(), fallback);
    }

    /// 易错反例：**黑名单先判**，所以「含 claude 但也含别家词」依然被拒
    #[test]
    fn alias_rejects_names_the_desktop_blacklists_even_with_anthropic_words() {
        // 这三个如果实现成「只检查含不含 claude」就会误判成合法
        for illegal in ["glm-5.3-claude", "qwen-claude-sonnet", "deepseek-v4-opus"] {
            assert_eq!(
                claudedesktop_alias("x", illegal).unwrap(),
                "claude-sonnet-4-6",
                "{illegal} 含 Anthropic 词但也命中黑名单，必须回落"
            );
        }
    }

    /// 真实模型名本身合法时也不该被改写（用户就是在用 Anthropic 模型）
    #[test]
    fn alias_is_identity_when_real_model_is_already_anthropic() {
        assert_eq!(claudedesktop_alias("claude-sonnet-4-5", "").unwrap(), "claude-sonnet-4-5");
        assert_eq!(claudedesktop_alias("sonnet", "").unwrap(), "sonnet");
    }

    /// 自身是 `claude-sonnet-4-6` 之类的默认别名 → 直接用（自指不能算成需要回落）
    #[test]
    fn alias_default_is_itself_a_legal_name() {
        let d = default_claudedesktop_alias();
        assert!(is_legal_claudedesktop_model(d), "默认别名自身必须合法，否则兜底也是空的");
    }

    /// 启动路径与 profile 写入**共用**同一个接缝：Claude Desktop 走别名规则，其余工具原样
    #[test]
    fn effective_claimed_model_only_masquerades_for_claudedesktop() {
        // Claude Desktop：space-bunny 会被换成默认别名
        assert_eq!(
            effective_claimed_model(Some("claudedesktop"), "space-bunny", ""),
            "claude-sonnet-4-6"
        );
        // 其余写入器必须**原样**（别让这次改动波及其它工具）
        assert_eq!(effective_claimed_model(None, "space-bunny", ""), "space-bunny");
        assert_eq!(
            effective_claimed_model(Some("codebuddy"), "space-bunny", ""),
            "space-bunny"
        );
        // 非 Claude Desktop 工具：声明名照旧优先
        assert_eq!(
            effective_claimed_model(None, "space-bunny", "my-alias"),
            "my-alias"
        );
    }

    /// **防「写 A 注册 B」**：profile 里写出的 `name` 必须与启动时注册进
    /// `model_aliases` 的键**完全一致**。不一致时 Desktop 发的名字无人认领 → 上游 400。
    #[test]
    fn profile_name_matches_the_alias_the_proxy_registers() {
        for (real, preferred) in [
            ("space-bunny", ""),   // 没配声明名 → 走默认别名
            ("glm-5.3", ""),       // 别家模型 → 走默认别名
            ("space-bunny", "sonnet"),          // 合法声明名 → 用它
            ("claude-opus-4", ""), // 真实名本来就合法 → 用它
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
            write_claudedesktop_with(&layout, &m).unwrap();

            // 直接读 profile JSON 的 `name`（不能用 read_claudedesktop —— 它读的是
            // labelOverride，那是给界面看的真实模型名，语义不同）
            let profile_path = layout
                .lib_dir
                .join(format!("{CLAUDE_DESKTOP_PROFILE_ID}.json"));
            let doc: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&profile_path).unwrap()).unwrap();
            let written = doc["inferenceModels"][0]["name"].as_str().unwrap_or_default();
            let label = doc["inferenceModels"][0]["labelOverride"]
                .as_str()
                .unwrap_or_default();

            let registered = effective_claimed_model(Some("claudedesktop"), real, preferred);
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
            let alias = claudedesktop_alias(real, "").unwrap();
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

        let alias = claudedesktop_alias("space-bunny", "").unwrap();
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
}
