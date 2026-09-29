//! Claude Code 官方**插件市场**支持（走 `claude plugin` CLI）。
//!
//! 与 [`super::codex_plugins`] 同构（同一套「市场注册 + 逐插件装/卸」），差异只有三处：
//!
//! | | Codex | Claude Code |
//! |---|---|---|
//! | CLI | `codex plugin …`（app 自带 `.exe`） | `claude plugin …`（npm 全局包，Windows 上是 `.cmd`） |
//! | 市场声明 | `~/.codex/config.toml` 的 `[marketplaces.<name>]` | `~/.claude/settings.json` 的 `extraKnownMarketplaces` |
//! | 列表字段 | `name/version/installed/enabled` | `pluginId`(installed 段叫 `id`)/`version/scope/enabled` |
//!
//! 为什么仍然复用官方 CLI 而不是自己写文件：`claude plugin install` 会同时维护
//! `enabledPlugins`、`~/.claude/plugins/cache/<市场>/<插件>/<版本>/`、
//! `installed_plugins.json`、`known_marketplaces.json` 与 `plugin-catalog-cache.json`。
//! 少写任何一个，插件在客户端里就是「看得见、用不了」。
//!
//! **安全**：`claude plugin install` 带 `--accept-command <sha256>`，用它等于替用户批准
//! 「市场声明的命令」。我们**绝不自动传**这个参数 —— 命令源插件会安装失败并让用户去官方
//! CLI 里自己确认。宁可功能少一点，也不替用户批准执行命令。

use std::path::PathBuf;

/// Claude Code 的官方技能/插件市场（`anthropics/skills`，2026-09-29 实测可添加）。
///
/// 注意它**不是** ChatGPT 那种保留名机制：Claude 的市场名由仓库里的
/// `.claude-plugin/marketplace.json` 自己声明（这个仓库声明的名字是 `anthropic-agent-skills`），
/// 所以这里给出的是**来源**，不是我们要写的市场名。
pub const CLAUDE_OFFICIAL_MARKETPLACE_SOURCE: &str = "anthropics/skills";

/// 一次 CLI 调用的超时（安装要 clone + 落盘，给足）。
const CLI_TIMEOUT_SECS: u64 = 300;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudePluginInfo {
    /// `插件@市场`
    pub id: String,
    pub name: String,
    pub description: String,
    pub marketplace: String,
    pub installed: bool,
    pub enabled: bool,
    pub version: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudePluginStatus {
    /// 找不到 `claude` CLI 时界面直接置灰
    pub cli_available: bool,
    /// 已配置的市场名
    pub marketplaces: Vec<String>,
    pub installed_count: usize,
    pub available_count: usize,
    /// 官方市场的来源（界面「添加官方市场」用）
    pub official_marketplace_source: String,
}

/// 定位 `claude` CLI。
///
/// npm 全局包在 Windows 上装出来的是 `<prefix>\claude.cmd`（外加 `.ps1` 与无扩展名的 shell
/// 脚本），**不是 `.exe`** —— 所以不能只照 Codex 那样找 `.exe`。三个来源：
///
/// 1. 注册表声明的路径（`ai-tools/claude-code/paths.json`）；
/// 2. **npm 的实际全局前缀**（`npm prefix -g`）—— 用户常把 prefix 改到别的盘，
///    声明里的 `%APPDATA%\npm` 会整体落空（本机实测：prefix 是 `D:\any-versions\sdk\nodejs`，
///    三条声明路径全部不命中）；
/// 3. PATH。
fn resolve_claude_cli() -> Option<PathBuf> {
    const NAMES: [&str; 3] = ["claude.cmd", "claude.exe", "claude"];

    // 1) 注册表声明的路径
    if let Some((_, paths)) = crate::commands::ai_registry::registry().get_tool("claude-code") {
        if let Some(exe) =
            super::tool_paths::find_declared_exe("claude-code", &paths.paths, &paths.command)
        {
            if exe.is_file() {
                return Some(exe);
            }
        }
    }

    // 2) npm 实际全局前缀（Windows 上 npm prefix -g 返回的目录里就有 claude.cmd）
    //    这条与检测层共用同一份实现（tool_paths::npm_global_prefix，进程内缓存）。
    if let Some(prefix) = super::tool_paths::npm_global_prefix() {
        for name in NAMES {
            let candidate = prefix.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    // 3) PATH
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        for name in NAMES {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// 跑一次 `claude …`，成功时返回 stdout。
///
/// 统一经 `cmd /c`：npm 装出来的 `claude.cmd` 是批处理，`CreateProcess` 不能直接执行它
/// （`.exe` 走这条路也无副作用）。
fn run_claude_cli(args: &[&str], timeout_secs: u64) -> Result<String, String> {
    let exe = resolve_claude_cli().ok_or_else(|| {
        "找不到 Claude Code CLI（claude）—— 插件管理依赖它。可执行 `npm install -g @anthropic-ai/claude-code` 安装。"
            .to_string()
    })?;
    let mut cmd = std::process::Command::new("cmd");
    cmd.arg("/c").arg(&exe).args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let output = super::detect::run_command_with_timeout(&mut cmd, timeout_secs).ok_or_else(|| {
        format!(
            "`claude {}` 执行失败或超时（{timeout_secs}s）",
            args.join(" ")
        )
    })?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            stdout.trim().to_string()
        } else {
            stderr
        };
        return Err(format!("`claude {}` 失败：{detail}", args.join(" ")));
    }
    Ok(stdout)
}

pub(crate) fn claude_cli_available() -> bool {
    resolve_claude_cli().is_some()
}

/// 解析 `claude plugin marketplace list` 的人读输出：
///
/// ```text
/// Configured marketplaces:
///
///   ❯ anthropic-agent-skills
///     Source: GitHub (anthropics/skills)
/// ```
fn parse_marketplace_list(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("Configured marketplaces")
            || trimmed.starts_with("No marketplaces")
            || (trimmed.starts_with("Source:") || trimmed.starts_with("- Source:"))
        {
            continue;
        }
        if let Some(name) = trimmed.strip_prefix('❯').map(str::trim) {
            out.push(name.to_string());
        }
    }
    out
}

/// 解析 `claude plugin list --available --json`：
/// `{"installed":[{id,version,scope,enabled,installPath}],
///   "available":[{pluginId,name,description,marketplaceName,source}]}`。
///
/// 两段的字段名**不一样**（installed 用 `id` 且没有 `name`/`description`），
/// 所以要先按 `pluginId`→`id`、`name`→从 id 里切出来统一。
fn parse_plugin_list_json(text: &str) -> Vec<ClaudePluginInfo> {
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let mut by_id: std::collections::HashMap<String, ClaudePluginInfo> =
        std::collections::HashMap::new();

    // available 段信息最全，先放进去
    if let Some(items) = doc.get("available").and_then(|v| v.as_array()) {
        for item in items {
            let Some(id) = item.get("pluginId").and_then(|v| v.as_str()) else {
                continue;
            };
            by_id.insert(
                id.to_string(),
                ClaudePluginInfo {
                    id: id.to_string(),
                    name: item
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or_else(|| id.split('@').next().unwrap_or(id))
                        .to_string(),
                    description: item
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    marketplace: item
                        .get("marketplaceName")
                        .and_then(|v| v.as_str())
                        .unwrap_or_else(|| id.split('@').nth(1).unwrap_or(""))
                        .to_string(),
                    installed: false,
                    enabled: false,
                    version: item.get("version").and_then(|v| v.as_str()).map(String::from),
                },
            );
        }
    }

    // installed 段补齐状态（字段叫 id，没有 name/description）
    if let Some(items) = doc.get("installed").and_then(|v| v.as_array()) {
        for item in items {
            let Some(id) = item.get("id").and_then(|v| v.as_str()) else {
                continue;
            };
            let entry = by_id.entry(id.to_string()).or_insert_with(|| ClaudePluginInfo {
                id: id.to_string(),
                name: id.split('@').next().unwrap_or(id).to_string(),
                description: String::new(),
                marketplace: id.split('@').nth(1).unwrap_or("").to_string(),
                installed: true,
                enabled: false,
                version: None,
            });
            entry.installed = true;
            entry.enabled = item.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
            if let Some(version) = item.get("version").and_then(|v| v.as_str()) {
                entry.version = Some(version.to_string());
            }
        }
    }

    let mut out: Vec<ClaudePluginInfo> = by_id.into_values().collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// 列出插件（已装 + 可装）。
#[tauri::command]
pub fn claude_list_plugins() -> Result<Vec<ClaudePluginInfo>, String> {
    let text = run_claude_cli(&["plugin", "list", "--available", "--json"], 120)?;
    Ok(parse_plugin_list_json(&text))
}

/// 查询 Claude 插件市场状态（CLI 是否可用、已配置的市场、插件计数）。
#[tauri::command]
pub fn claude_plugin_status() -> ClaudePluginStatus {
    let cli_available = claude_cli_available();
    let marketplaces = if cli_available {
        run_claude_cli(&["plugin", "marketplace", "list"], 60)
            .map(|text| parse_marketplace_list(&text))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let (installed_count, available_count) = if cli_available && !marketplaces.is_empty() {
        match run_claude_cli(&["plugin", "list", "--available", "--json"], 120) {
            Ok(text) => {
                let plugins = parse_plugin_list_json(&text);
                let installed = plugins.iter().filter(|p| p.installed).count();
                (installed, plugins.len().saturating_sub(installed))
            }
            Err(e) => {
                eprintln!("[claude_plugins] 读取插件列表失败：{e}");
                (0, 0)
            }
        }
    } else {
        (0, 0)
    };
    ClaudePluginStatus {
        cli_available,
        marketplaces,
        installed_count,
        available_count,
        official_marketplace_source: CLAUDE_OFFICIAL_MARKETPLACE_SOURCE.to_string(),
    }
}

/// 添加市场（`claude plugin marketplace add <URL|路径|owner/repo>`）。
///
/// 走 `spawn_blocking`：要 git clone 整个市场仓库。
#[tauri::command]
pub async fn claude_plugin_marketplace_add(source: String) -> Result<Vec<String>, String> {
    let source = source.trim().to_string();
    if source.is_empty() {
        return Err("市场来源不能为空（可填 owner/repo、Git URL 或本地路径）".to_string());
    }
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        run_claude_cli(
            &["plugin", "marketplace", "add", source.as_str()],
            CLI_TIMEOUT_SECS,
        )?;
        Ok(())
    })
    .await
    .map_err(|e| format!("添加市场任务异常退出: {e}"))??;
    Ok(run_claude_cli(&["plugin", "marketplace", "list"], 60)
        .map(|text| parse_marketplace_list(&text))
        .unwrap_or_default())
}

/// 移除市场（`claude plugin marketplace remove <name>`）。
#[tauri::command]
pub async fn claude_plugin_marketplace_remove(name: String) -> Result<Vec<String>, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("市场名不能为空".to_string());
    }
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        run_claude_cli(
            &["plugin", "marketplace", "remove", name.as_str()],
            120,
        )?;
        Ok(())
    })
    .await
    .map_err(|e| format!("移除市场任务异常退出: {e}"))??;
    Ok(run_claude_cli(&["plugin", "marketplace", "list"], 60)
        .map(|text| parse_marketplace_list(&text))
        .unwrap_or_default())
}

/// 安装插件（`claude plugin install <插件>@<市场>`）。
///
/// **故意不传 `--accept-command`**：那个参数是替用户批准「市场声明的命令」。
/// 命令源插件会在这里失败，用户需要去官方 CLI 里自己确认 —— 这是有意的取舍。
#[tauri::command]
pub async fn claude_install_plugin(plugin: String) -> Result<Vec<ClaudePluginInfo>, String> {
    let plugin = plugin.trim().to_string();
    if plugin.is_empty() {
        return Err("插件 id 不能为空（形如 插件@市场）".to_string());
    }
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        run_claude_cli(&["plugin", "install", plugin.as_str()], CLI_TIMEOUT_SECS)?;
        Ok(())
    })
    .await
    .map_err(|e| format!("插件安装任务异常退出: {e}"))??;
    claude_list_plugins()
}

/// 卸载插件（`claude plugin uninstall <插件>@<市场>`）。
#[tauri::command]
pub async fn claude_uninstall_plugin(plugin: String) -> Result<Vec<ClaudePluginInfo>, String> {
    let plugin = plugin.trim().to_string();
    if plugin.is_empty() {
        return Err("插件 id 不能为空".to_string());
    }
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        run_claude_cli(&["plugin", "uninstall", plugin.as_str()], 120)?;
        Ok(())
    })
    .await
    .map_err(|e| format!("插件卸载任务异常退出: {e}"))??;
    claude_list_plugins()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 官方市场来源不能被写成「保留名」式的猜测：它必须是可用的 `owner/repo`。
    #[test]
    fn official_marketplace_source_is_a_repo_slug() {
        assert!(
            CLAUDE_OFFICIAL_MARKETPLACE_SOURCE.contains('/'),
            "应是 owner/repo 形式：{CLAUDE_OFFICIAL_MARKETPLACE_SOURCE}"
        );
    }

    /// 市场列表的解析要能跳过标题行与 Source 行，只取名字（`❯` 前缀要去掉）。
    #[test]
    fn marketplace_list_parsing_skips_headers() {
        let text = "Configured marketplaces:\n\n  ❯ anthropic-agent-skills\n    Source: GitHub (anthropics/skills)\n";
        assert_eq!(
            parse_marketplace_list(text),
            vec!["anthropic-agent-skills".to_string()]
        );
        assert!(parse_marketplace_list("No marketplaces configured").is_empty());
    }

    /// `installed` 与 `available` 两段的字段名不同（`id` vs `pluginId`+`name`），
    /// 必须合并成同一份、且已装状态要能标出来。
    #[test]
    fn plugin_list_json_merges_both_sections() {
        let json = r#"{
            "installed": [{"id":"document-skills@anthropic-agent-skills","version":"8a1541c4a3ff","scope":"user","enabled":true}],
            "available": [
                {"pluginId":"document-skills@anthropic-agent-skills","name":"document-skills","description":"Excel/Word/PPT/PDF","marketplaceName":"anthropic-agent-skills","source":"./"},
                {"pluginId":"claude-api@anthropic-agent-skills","name":"claude-api","description":"API docs","marketplaceName":"anthropic-agent-skills","source":"./"}
            ]
        }"#;
        let plugins = parse_plugin_list_json(json);
        assert_eq!(plugins.len(), 2);

        let installed = plugins.iter().find(|p| p.name == "document-skills").unwrap();
        assert!(installed.installed && installed.enabled, "已装且启用");
        assert_eq!(installed.version.as_deref(), Some("8a1541c4a3ff"));
        // 描述来自 available 段 —— 两段必须真的合并了
        assert_eq!(installed.description, "Excel/Word/PPT/PDF");

        let not_installed = plugins.iter().find(|p| p.name == "claude-api").unwrap();
        assert!(!not_installed.installed);
        assert_eq!(not_installed.marketplace, "anthropic-agent-skills");

        assert!(parse_plugin_list_json("not json").is_empty());
    }
}
