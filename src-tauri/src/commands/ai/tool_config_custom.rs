//! 自定义「写模型」：schema 复杂到没法用「路径 → 值」声明的工具。
//!
//! 复刻自 EchoBird（`E:\pro\other-sdk\ai-tools\EchoBird`）：
//! `tools/*/config.json` 用 `"custom": true` 标记，真正的写入在
//! `src-tauri/src/services/tool_config_manager/<tool>.rs` 里整份生成配置。
//! 我们这里放同一套东西（目前只有 WorkBuddy 一个，后续按需加）。
//!
//! 为什么需要它：WorkBuddy 的模型配置是
//! `{ models: [{ id, name, vendor, url, apiKey, maxInputTokens, maxOutputTokens,
//!   supportsToolCall, supportsImages }], availableModels: [id] }` ——
//! 数组套对象、还要求 URL 是**完整的 /chat/completions**，通用的 `set_json_path` 表达不了。

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

/// WorkBuddy 默认的上下文/输出上限（EchoBird workbuddy.rs 同值）。
/// 它没有「留空让应用自己判断」的写法，字段缺了会被前端当 0 处理。
const WORKBUDDY_MAX_INPUT_TOKENS: u64 = 200_000;
const WORKBUDDY_MAX_OUTPUT_TOKENS: u64 = 8_192;

/// 按写入器名分派。`None` 之外的未知名字直接报错（配置写错要立刻看得见）。
pub fn write_config(writer: &str, path: &Path, m: &ModelWrite<'_>) -> Result<(), String> {
    match writer {
        "workbuddy" => write_workbuddy(path, m),
        "claudedesktop" => write_claudedesktop(path, m),
        other => Err(format!("未知的自定义配置写入器: {other}")),
    }
}

/// 读回「当前写定的模型」（界面回显用）。
pub fn read_model(writer: &str, path: &Path) -> Option<String> {
    match writer {
        "workbuddy" => read_workbuddy(path),
        "claudedesktop" => read_claudedesktop(path),
        _ => None,
    }
}

/// 把 base 补成 WorkBuddy 要求的完整 `/chat/completions` 端点。
///
/// 它不做 OpenAI SDK 那种「自己拼路径」的事：给了 `https://x/v1` 它会去请求
/// `https://x/v1` 本身，所以必须写全。规则与项目里其它拼 URL 的地方一致，避免 `/v1/v1`。
pub fn ensure_chat_completions(base: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    if base.is_empty() {
        return String::new();
    }
    if base.ends_with("/chat/completions") {
        return base.to_string();
    }
    if base.ends_with("/v1") {
        return format!("{base}/chat/completions");
    }
    format!("{base}/v1/chat/completions")
}

/// 从 URL 里取一个可展示的「厂商名」（WorkBuddy 界面会显示它）。
///
/// 回环地址（走本地代理时）看不出上游是谁，统一叫 `local`，别显示成 127.0.0.1。
pub fn vendor_of(url: &str) -> String {
    let rest = url
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let host_port = rest.split('/').next().unwrap_or("");
    // 去掉端口与 userinfo
    let host = host_port.rsplit('@').next().unwrap_or(host_port);
    let host = host.split(':').next().unwrap_or(host).trim();
    let host = host.strip_prefix("www.").unwrap_or(host);
    if host.is_empty() {
        return "custom".to_string();
    }
    if host == "127.0.0.1" || host == "localhost" || host == "::1" || host == "[::1]" {
        return "local".to_string();
    }
    host.to_string()
}

/// WorkBuddy（`~/.workbuddy/models.json`）与 WorkBuddy AI（`~/.workbuddy-ai/models.json`）。
///
/// 语义是**单条覆盖**（「切换模型」，不累积），与 EchoBird 一致；原有条目会被替换掉，
/// 所以替换前数一下旧条目并写进日志，用户至少能在日志里看到发生了什么。
fn write_workbuddy(path: &Path, m: &ModelWrite<'_>) -> Result<(), String> {
    if m.base_url.trim().is_empty() {
        return Err("WorkBuddy 需要上游地址（base URL），请先选择供应商与模型".to_string());
    }
    if m.api_key.trim().is_empty() {
        return Err("WorkBuddy 需要 API Key，请先给供应商填 Key".to_string());
    }
    let model_id = if m.model.trim().is_empty() {
        m.claimed.trim()
    } else {
        m.model.trim()
    };
    if model_id.is_empty() {
        return Err("WorkBuddy 需要一个模型 id".to_string());
    }
    let display = {
        let claimed = m.claimed.trim();
        if claimed.is_empty() {
            model_id
        } else {
            claimed
        }
    };

    if let Some(old) = read_workbuddy_raw(path) {
        let replaced = old
            .get("models")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        if replaced > 1 {
            eprintln!("[config_file] WorkBuddy: 覆盖原有 {replaced} 个自定义模型（单条覆盖语义）");
        }
    }

    // 厂商优先取真实上游；没给上游（如「只保存模型」直连模式）就用请求地址的 host
    let vendor = {
        let upstream = vendor_of(m.upstream_url);
        if upstream == "custom" {
            vendor_of(m.base_url)
        } else {
            upstream
        }
    };
    let doc = serde_json::json!({
        "models": [{
            "id": model_id,
            "name": display,
            "vendor": vendor,
            "url": ensure_chat_completions(m.base_url),
            "apiKey": m.api_key,
            "maxInputTokens": WORKBUDDY_MAX_INPUT_TOKENS,
            "maxOutputTokens": WORKBUDDY_MAX_OUTPUT_TOKENS,
            "supportsToolCall": true,
            "supportsImages": true
        }],
        "availableModels": [model_id]
    });

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建 WorkBuddy 配置目录失败: {e}（{}）", parent.display()))?;
    }
    // 必须 UTF-8 **不带 BOM**：个别桌面构建解析带 BOM 的 models.json 会直接失败
    // （EchoBird workbuddy.rs 里踩过）。serde_json 写出来天然无 BOM。
    let text = serde_json::to_string_pretty(&doc)
        .map_err(|e| format!("序列化 WorkBuddy 配置失败: {e}"))?;
    std::fs::write(path, text)
        .map_err(|e| format!("写入 WorkBuddy 配置失败: {e}（{}）", path.display()))?;
    eprintln!(
        "[config_file] WorkBuddy 模型已写入 {}：{}（{}）",
        path.display(),
        display,
        ensure_chat_completions(m.base_url)
    );
    Ok(())
}

fn read_workbuddy_raw(path: &Path) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<serde_json::Value>(&text).ok()
}

/// 读回第一条模型的 id（WorkBuddy 界面上「当前模型」就是它）。
fn read_workbuddy(path: &Path) -> Option<String> {
    let doc = read_workbuddy_raw(path)?;
    let id = doc
        .get("models")?
        .as_array()?
        .first()?
        .get("id")?
        .as_str()?
        .trim()
        .to_string();
    if id.is_empty() {
        None
    } else {
        Some(id)
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
    let sent_model = if m.claimed.trim().is_empty() {
        real_model
    } else {
        m.claimed.trim()
    };

    set_deployment_mode(&layout.official_cfg, "3p")?;
    set_deployment_mode(&layout.threep_cfg, "3p")?;
    std::fs::create_dir_all(&layout.lib_dir)
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

/// 写 JSON（serde_json 输出天然 UTF-8 无 BOM），父目录不存在则创建。
fn write_json(path: &Path, doc: &serde_json::Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录失败: {e}（{}）", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(doc).map_err(|e| format!("序列化失败: {e}"))?;
    std::fs::write(path, text).map_err(|e| format!("写入失败: {e}（{}）", path.display()))
}

#[cfg(test)]
mod tests {
    use super::{ensure_chat_completions, read_model, vendor_of, write_config, ModelWrite};
    use std::path::PathBuf;

    use super::{claude_desktop_layout, write_claudedesktop_with, ClaudeDesktopLayout};

    fn probe_path(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!("anyver-toolcfg-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("models.json")
    }

    /// WorkBuddy 要求完整端点：给的 base 不论带不带 /v1，都要补成 .../chat/completions。
    #[test]
    fn chat_completions_url_is_always_complete() {
        assert_eq!(
            ensure_chat_completions("https://api.x.com"),
            "https://api.x.com/v1/chat/completions"
        );
        assert_eq!(
            ensure_chat_completions("https://api.x.com/v1/"),
            "https://api.x.com/v1/chat/completions"
        );
        // 已经是完整端点 → 不重复补（否则会得到 /chat/completions/chat/completions）
        assert_eq!(
            ensure_chat_completions("https://api.x.com/v1/chat/completions"),
            "https://api.x.com/v1/chat/completions"
        );
        // 网关自带版本段：只补 chat/completions
        assert_eq!(
            ensure_chat_completions("https://gw.x.com/openai/v2"),
            "https://gw.x.com/openai/v2/v1/chat/completions"
        );
        // 走本地代理时 base 就是 127.0.0.1:port
        assert_eq!(
            ensure_chat_completions("http://127.0.0.1:15721"),
            "http://127.0.0.1:15721/v1/chat/completions"
        );
        assert_eq!(ensure_chat_completions(""), "");
    }

    #[test]
    fn vendor_is_a_readable_hostname() {
        assert_eq!(vendor_of("https://api.deepseek.com/v1"), "api.deepseek.com");
        assert_eq!(vendor_of("https://www.x.com:8443/v1"), "x.com");
        assert_eq!(vendor_of("http://127.0.0.1:15721"), "local");
        assert_eq!(vendor_of(""), "custom");
    }

    /// 写入 → 读回：字段齐全、URL 是完整端点、availableModels 与 models[0] 对得上。
    #[test]
    fn workbuddy_write_and_read_round_trip() {
        let path = probe_path("wb");
        write_config(
            "workbuddy",
            &path,
            &ModelWrite {
                model: "deepseek-chat",
                claimed: "gpt-4.1",
                base_url: "http://127.0.0.1:15721",
                api_key: "sk-test",
                upstream_url: "https://api.deepseek.com/v1",
                one_m: false,
            },
        )
        .expect("写入应成功");

        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let first = &doc["models"][0];
        assert_eq!(first["id"], "deepseek-chat");
        // name 用声明名（伪装），id 用真实模型
        assert_eq!(first["name"], "gpt-4.1");
        assert_eq!(first["url"], "http://127.0.0.1:15721/v1/chat/completions");
        assert_eq!(first["apiKey"], "sk-test");
        // 厂商取真实上游，不是回环地址
        assert_eq!(first["vendor"], "api.deepseek.com");
        assert_eq!(first["supportsToolCall"], true);
        assert_eq!(doc["availableModels"][0], "deepseek-chat");

        assert_eq!(
            read_model("workbuddy", &path).as_deref(),
            Some("deepseek-chat")
        );

        // 单条覆盖：再写一次不会累积成两条
        write_config(
            "workbuddy",
            &path,
            &ModelWrite {
                model: "glm-4.6",
                claimed: "",
                base_url: "https://open.bigmodel.cn/api/paas/v4",
                api_key: "sk-2",
                upstream_url: "",
                one_m: false,
            },
        )
        .unwrap();
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(doc["models"].as_array().unwrap().len(), 1);
        assert_eq!(read_model("workbuddy", &path).as_deref(), Some("glm-4.6"));
        // claimed 为空 → name 回退成模型 id；vendor 回退成 base 的 host
        assert_eq!(doc["models"][0]["name"], "glm-4.6");
        assert_eq!(doc["models"][0]["vendor"], "open.bigmodel.cn");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// 缺 key / base 时明确报错，别写出一份工具读不了的半成品。
    #[test]
    fn workbuddy_requires_url_and_key() {
        let path = probe_path("wb-fail");
        let with = |base_url: &'static str, api_key: &'static str| ModelWrite {
            model: "m",
            claimed: "m",
            base_url,
            api_key,
            upstream_url: "",
            one_m: false,
        };
        // 缺 base / 缺 key 都要明确报错，不能写出半成品
        assert!(write_config("workbuddy", &path, &with("", "sk")).is_err());
        assert!(write_config("workbuddy", &path, &with("https://x/v1", " ")).is_err());
        // 未知写入器名也要报错（配置写错立刻可见）
        assert!(write_config("nope", &path, &with("https://x/v1", "sk")).is_err());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
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
}
