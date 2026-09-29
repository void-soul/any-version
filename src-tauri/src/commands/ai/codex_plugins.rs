//! Codex 官方**插件市场**支持。
//!
//! ChatGPT Desktop / Codex 的插件市场是 Codex 客户端**自带**的能力，约定是：
//! 市场内容放在一个目录里（`.agents/plugins/marketplace.json` + `plugins/<name>/`），
//! 再在 `~/.codex/config.toml` 里用 `[marketplaces.<name>]` 注册。客户端启动时才去列市场、
//! 装插件 —— **单个插件的安装由客户端自己做**，这里只负责「把市场放到磁盘 + 注册进配置」。
//!
//! 抄自 CodexPlusPlus `plugin_marketplace.rs`。它踩出来的两个坑必须避开：
//!
//! 1. **市场名不能用 `openai-*`**：那是 Codex 的保留名，注册在它下面的本地市场会被
//!    **静默忽略**（`codex plugin marketplace list` 都不列，用户在市场里一个插件都看不到，
//!    issue #1974 / #1968）。所以用 `anyversion-curated`。
//! 2. **改名要连磁盘一起改**：`marketplace.json` 里的 `name` 与 `config.toml` 里的表名不一致
//!    时，整个市场同样不被识别 —— 只改一边等于没改。

use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};

/// 市场名。见模块注释：**不能**用 `openai-*` 前缀。
const MARKETPLACE_NAME: &str = "anyversion-curated";

/// 官方插件仓库（codeload 直出 zip，比 clone 快且不需要 git）。
const OPENAI_PLUGINS_ZIP_URL: &str =
    "https://codeload.github.com/openai/plugins/zip/refs/heads/main";

/// 下载体积上限：市场包约 7MB，给足余量但不允许无限流。
const DOWNLOAD_LIMIT_BYTES: usize = 128 * 1024 * 1024;

/// Codex 主目录（`CODEX_HOME` 优先，其次 `~/.codex`）。
fn codex_home() -> Result<PathBuf, String> {
    if let Ok(dir) = std::env::var("CODEX_HOME") {
        let dir = dir.trim();
        if !dir.is_empty() {
            return Ok(PathBuf::from(dir));
        }
    }
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map_err(|_| "无法确定用户主目录".to_string())?;
    Ok(PathBuf::from(home).join(".codex"))
}

/// 市场落盘根目录。用 `.tmp` 子目录与 Codex 自己的数据分开（抄 CodexPlusPlus）。
fn marketplace_root() -> Result<PathBuf, String> {
    Ok(codex_home()?.join(".tmp").join("plugins-remote"))
}

fn config_path() -> Result<PathBuf, String> {
    Ok(codex_home()?.join("config.toml"))
}

/// 市场清单文件：`<root>/.agents/plugins/marketplace.json`。
fn marketplace_json_path(root: &Path) -> PathBuf {
    root.join(".agents").join("plugins").join("marketplace.json")
}

/// 写进 `config.toml` 的 `source` 值。
///
/// Windows 上要加 `\\?\` 长路径前缀：与 CodexPlusPlus 写出的值保持一致，
/// 否则同一个市场在两边被当成两个（它做「是否已注册」判定时会比对字符串）。
fn marketplace_source_value(root: &Path) -> String {
    let value = root.to_string_lossy();
    if !cfg!(windows) || value.starts_with(r"\\?\") {
        value.into_owned()
    } else {
        format!(r"\\?\{value}")
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexPluginEntry {
    pub name: String,
    /// 清单里声明的分类（`Productivity` / `Design` / …），界面按它分组
    pub category: String,
    /// marketplace.json 里声明的相对路径
    pub path: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexPluginMarketplaceStatus {
    pub marketplace_name: String,
    /// 市场内容是否已落盘且清单可解析
    pub installed: bool,
    /// `config.toml` 里是否已注册本市场
    pub registered: bool,
    pub plugin_count: usize,
    /// 本市场里已启用的插件数（只算 `<插件>@<本市场>`）
    pub enabled_count: usize,
    pub root: String,
    pub config_path: String,
    /// Codex CLI 是否可用 —— 单个插件的安装/卸载要经它落盘，不可用时界面直接置灰
    pub cli_available: bool,
}

/// 读市场清单里的插件列表（清单缺失 / 解析失败一律返回空）。
fn read_marketplace_plugins(root: &Path) -> Vec<CodexPluginEntry> {
    let Ok(text) = std::fs::read_to_string(marketplace_json_path(root)) else {
        return Vec::new();
    };
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    doc.get("plugins")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|it| {
                    // 插件的相对路径在 `source.path` 里（顶层没有 `path`，
                    // 早期实现只读顶层导致界面上路径全是空串）
                    let path = it
                        .get("source")
                        .and_then(|s| s.get("path"))
                        .and_then(|p| p.as_str())
                        .or_else(|| it.get("path").and_then(|p| p.as_str()))
                        .unwrap_or("")
                        .to_string();
                    Some(CodexPluginEntry {
                        name: it.get("name")?.as_str()?.to_string(),
                        category: it
                            .get("category")
                            .and_then(|c| c.as_str())
                            .unwrap_or("")
                            .to_string(),
                        path,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 把市场清单里的 `name` 重写成 [`MARKETPLACE_NAME`]。
///
/// **这一步不能省**：官方仓库的 `marketplace.json` 里 `name` 是 `openai-curated`，
/// 而那是 Codex 的保留名。落盘后若原样保留，磁盘上的市场名与 `config.toml` 里的表名
/// 不一致 —— 整个市场会被客户端**静默忽略**：表现为「安装成功、插件列表空空的」，
/// 且没有任何报错可查。模块注释早就写了「改名要连磁盘一起改」，但代码里一直漏了这一步。
fn rewrite_marketplace_name(root: &Path) -> Result<(), String> {
    let path = marketplace_json_path(root);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("读取插件市场清单失败（{}）: {e}", path.display()))?;
    let mut doc: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("插件市场清单不是合法 JSON: {e}"))?;
    let obj = doc
        .as_object_mut()
        .ok_or("插件市场清单顶层不是对象，无法改名")?;
    if obj.get("name").and_then(|v| v.as_str()) == Some(MARKETPLACE_NAME) {
        return Ok(());
    }
    obj.insert(
        "name".to_string(),
        serde_json::Value::String(MARKETPLACE_NAME.to_string()),
    );
    let out = serde_json::to_string_pretty(&doc)
        .map_err(|e| format!("序列化插件市场清单失败: {e}"))?;
    std::fs::write(&path, out).map_err(|e| format!("写入插件市场清单失败: {e}"))
}

/// `config.toml` 里本市场是否已注册（表名 + source 都要对上）。
fn is_registered(config: &str, root: &Path) -> bool {
    // 容错解析：Codex 自己写出来的 config.toml 里**确实存在重复表头**
    // （实测这台机器上 `[marketplaces.openai-primary-runtime]` 出现了 3 次），
    // 整体解析必然失败 —— 用严格解析会让状态永远显示「未注册」。
    let doc = crate::commands::ai::launch::parse_toml_lenient(config);
    let Some(table) = doc
        .get("marketplaces")
        .and_then(|m| m.get(MARKETPLACE_NAME))
        .and_then(|t| t.as_table())
    else {
        return false;
    };
    let source_type = table.get("source_type").and_then(|v| v.as_str()).unwrap_or("");
    let source = table.get("source").and_then(|v| v.as_str()).unwrap_or("");
    source_type == "local" && source == marketplace_source_value(root)
}

/// 本市场已启用的插件数（**只认 `<插件>@<本市场>`**）。
///
/// 不能统计「所有 enabled 项」：用户还装着 `openai-bundled` 等市场的插件，
/// 那些也会被算进来，界面上写「已启用 N 个」就跟本市场毫无关系了。
fn enabled_plugin_count(config: &str) -> usize {
    let doc = crate::commands::ai::launch::parse_toml_lenient(config);
    let Some(plugins) = doc.get("plugins").and_then(|p| p.as_table()) else {
        return 0;
    };
    let suffix = format!("@{MARKETPLACE_NAME}");
    plugins
        .iter()
        .filter(|(key, item)| {
            key.contains(&suffix)
                && item
                    .as_table()
                    .and_then(|t| t.get("enabled"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
        })
        .count()
}

fn status_now() -> Result<CodexPluginMarketplaceStatus, String> {
    let root = marketplace_root()?;
    let config_file = config_path()?;
    let config = std::fs::read_to_string(&config_file).unwrap_or_default();
    let plugins = read_marketplace_plugins(&root);
    Ok(CodexPluginMarketplaceStatus {
        marketplace_name: MARKETPLACE_NAME.to_string(),
        installed: marketplace_json_path(&root).exists(),
        registered: is_registered(&config, &root),
        plugin_count: plugins.len(),
        enabled_count: enabled_plugin_count(&config),
        root: root.to_string_lossy().to_string(),
        config_path: config_file.to_string_lossy().to_string(),
        cli_available: codex_cli_available(),
    })
}

/// 查询插件市场状态。
#[tauri::command]
pub fn codex_plugin_marketplace_status() -> Result<CodexPluginMarketplaceStatus, String> {
    status_now()
}

/// 下载并把官方插件市场落到磁盘，然后注册进 `config.toml`。
///
/// 幂等：已装过也能再跑（覆盖更新，旧目录先备份，成功后才删）。
#[tauri::command]
pub async fn codex_install_plugin_marketplace() -> Result<CodexPluginMarketplaceStatus, String> {
    let root = marketplace_root()?;

    // 1) 下载
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| format!("HTTP 客户端构建失败: {e}"))?;
    let resp = client
        .get(OPENAI_PLUGINS_ZIP_URL)
        .send()
        .await
        .map_err(|e| format!("下载插件市场失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("下载插件市场失败: HTTP {}", resp.status()));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("读取插件市场数据失败: {e}"))?;
    if bytes.len() > DOWNLOAD_LIMIT_BYTES {
        return Err(format!(
            "插件市场包过大（{} MB，上限 {} MB），已中止",
            bytes.len() / 1024 / 1024,
            DOWNLOAD_LIMIT_BYTES / 1024 / 1024
        ));
    }

    // 2) 先解到临时目录，**校验通过再换上去**：直接往目标目录写，一旦中途失败
    //    就会留下半个市场 —— 客户端列不出插件，用户也不知道是坏了还是没装。
    let staging = root.with_extension("staging");
    let _ = std::fs::remove_dir_all(&staging);
    extract_zip_stripping_root(&bytes, &staging)?;
    if !marketplace_json_path(&staging).exists() {
        let _ = std::fs::remove_dir_all(&staging);
        return Err("插件市场包结构异常：找不到 .agents/plugins/marketplace.json".to_string());
    }
    // **改名（缺了这一步整个市场都白装）**：官方清单里 `name` 是保留名 `openai-curated`，
    // 与我们要写进 `config.toml` 的表名不一致 → 客户端直接忽略整个市场。
    if let Err(e) = rewrite_marketplace_name(&staging) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }

    // 3) 换上去（旧目录改名备份，成功后再删）
    let backup = root.with_extension("previous");
    let _ = std::fs::remove_dir_all(&backup);
    if root.exists() {
        std::fs::rename(&root, &backup)
            .map_err(|e| format!("备份旧插件市场失败: {e}"))?;
    }
    if let Some(parent) = root.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    if let Err(e) = std::fs::rename(&staging, &root) {
        // 换失败就把旧的原样放回去，别让用户两头空
        if backup.exists() {
            let _ = std::fs::rename(&backup, &root);
        }
        return Err(format!("替换插件市场失败: {e}"));
    }
    let _ = std::fs::remove_dir_all(&backup);

    // 4) 注册进 config.toml
    register_marketplace(&root)?;
    status_now()
}

/// 撤销托管：从 `config.toml` 摘掉本市场，并删除落盘目录。
#[tauri::command]
pub fn codex_remove_plugin_marketplace() -> Result<CodexPluginMarketplaceStatus, String> {
    let root = marketplace_root()?;
    let config_file = config_path()?;
    if config_file.exists() {
        let text = std::fs::read_to_string(&config_file).unwrap_or_default();
        let mut doc = crate::commands::ai::launch::parse_toml_lenient(text.trim_start_matches('\u{feff}'));
        let removed = doc
            .get_mut("marketplaces")
            .and_then(|m| m.as_table_mut())
            .map(|t| t.remove(MARKETPLACE_NAME).is_some())
            .unwrap_or(false);
        if removed {
            crate::commands::config::atomic_write_file(&config_file, doc.to_string().as_bytes())
                .map_err(|e| format!("写回 config.toml 失败: {e}"))?;
        }
    }
    if root.exists() {
        std::fs::remove_dir_all(&root).map_err(|e| format!("删除插件市场目录失败: {e}"))?;
    }
    status_now()
}

/// 把市场注册进 `config.toml`。
///
/// 用 `toml_edit` 语义合并：只加 `[marketplaces.<name>]` 这一段，
/// 用户自己的 `mcp_servers` / `[[hooks]]` / 注释一律原样保留（与启动写配置同一套机制）。
fn register_marketplace(root: &Path) -> Result<(), String> {
    let config_file = config_path()?;
    let existing = std::fs::read_to_string(&config_file).unwrap_or_default();
    let content = merge_marketplace_registration(&existing, root);
    // 写回前自校验：解析不过说明我们改坏了，宁可报错也不要把配置写成废纸
    content
        .parse::<toml_edit::DocumentMut>()
        .map_err(|e| format!("写入后的 config.toml 无法解析，已放弃写入: {e}"))?;
    if let Some(parent) = config_file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    crate::commands::config::atomic_write_file(&config_file, content.as_bytes())
        .map_err(|e| format!("写入 config.toml 失败: {e}"))
}

/// 把本市场注册进 `config.toml` 文本（纯函数，便于回归测试）。
///
/// **必须用容错解析**：Codex 自己写出来的 config.toml 里确实存在重复表头（实测
/// `[marketplaces.openai-primary-runtime]` ×3、`[plugins."…"]` 多次），严格解析会直接
/// 报错，整个注册流程静默失效 —— 用户点了「安装」却发现什么都没发生，且不知道原因。
///
/// 代价：文件整体不可解析时，结果会经过一次「切块 → 合并 → 重新序列化」，
/// 排版可能被规整（注释仍在）。这是无法两全的选择：要么写不进去，要么规整一次。
fn merge_marketplace_registration(existing: &str, root: &Path) -> String {
    let mut doc = crate::commands::ai::launch::parse_toml_lenient(existing.trim_start_matches('\u{feff}'));
    // 既没这个键、或它被用户手改成了标量 / 数组，都换成表 —— 否则永远注册不进去。
    // （注意短路求值：`doc["marketplaces"]` 在键不存在时会 panic，必须先判断存在性。）
    if !doc.as_table().contains_key("marketplaces") || doc["marketplaces"].as_table().is_none() {
        doc["marketplaces"] = toml_edit::Item::Table(toml_edit::Table::new());
    }
    let marketplaces = doc["marketplaces"]
        .as_table_mut()
        .expect("just ensured marketplaces is a table");
    // 市场名与磁盘上的 marketplace.json 必须一致：只改一边等于没注册
    let entry = marketplaces
        .entry(MARKETPLACE_NAME)
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
    if !entry.is_table() {
        *entry = toml_edit::Item::Table(toml_edit::Table::new());
    }
    let table = entry.as_table_mut().expect("just ensured it is a table");
    table["source_type"] = toml_edit::value("local");
    table["source"] = toml_edit::value(marketplace_source_value(root));
    doc.to_string()
}

/// 解压并**剥掉压缩包的第一层目录**（GitHub codeload 的 zip 顶层是 `plugins-main/`）。
///
/// 每条路径都过 [`safe_zip_entry`]：拒绝绝对路径与 `..`，防 zip slip
/// （恶意压缩包把文件写到目标目录之外）。
fn extract_zip_stripping_root(bytes: &[u8], destination: &Path) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes.to_vec()))
        .map_err(|e| format!("插件市场包不是合法 zip: {e}"))?;
    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .map_err(|e| format!("读取压缩包条目 {index} 失败: {e}"))?;
        let Some(relative) = safe_zip_entry(file.name()) else {
            continue;
        };
        let output = destination.join(&relative);
        if file.is_dir() {
            std::fs::create_dir_all(&output).map_err(|e| format!("创建目录失败: {e}"))?;
            continue;
        }
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
        }
        let mut contents = Vec::new();
        file.read_to_end(&mut contents)
            .map_err(|e| format!("读取压缩包条目失败: {e}"))?;
        std::fs::write(&output, contents).map_err(|e| format!("写入失败: {e}"))?;
    }
    Ok(())
}

/// 剥掉 zip 第一层目录并校验路径安全；返回 `None` 表示该条目应被跳过。
fn safe_zip_entry(name: &str) -> Option<PathBuf> {
    let mut components = Path::new(name).components();
    match components.next()? {
        Component::Normal(_) => {}
        _ => return None,
    }
    let mut relative = PathBuf::new();
    for component in components {
        match component {
            Component::Normal(value) => relative.push(value),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!relative.as_os_str().is_empty()).then_some(relative)
}

// ─── 插件级管理（复用 Codex 官方 CLI）───
//
// 为什么走 CLI 而不是自己写 `config.toml`：`codex plugin add` 除了写配置，还会把插件
// 落到 `~/.codex/plugins/cache/<市场>/<插件>/<版本>/` 并处理清单里的 `authPolicy`。
// 少做这一步，插件在客户端里就是「看得见、用不了」。

use std::collections::HashMap;

/// 界面用的插件条目：清单声明 + 当前安装状态。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexPluginInfo {
    pub name: String,
    pub category: String,
    pub installed: bool,
    pub enabled: bool,
    pub version: Option<String>,
}

/// 单个插件的当前状态。
#[derive(Debug, Clone, Default)]
struct PluginState {
    installed: bool,
    enabled: bool,
    version: Option<String>,
}

/// 定位 Codex CLI（`codex.exe`）。
///
/// 按可靠性排序：
/// 1. 注册表声明的路径（`ai-tools/codex-cli` / `chatgptdesktop` 指向同一份 codex）；
/// 2. Codex 自己写进 `config.toml` 的 `CODEX_CLI_PATH`（藏在 `mcp_servers.*.env` 里）——
///    那是客户端此刻实际在用的那个 exe，最权威；
/// 3. 客户端自带的 `~/.codex/plugins/.plugin-appserver/codex.exe`。
fn resolve_codex_cli() -> Option<PathBuf> {
    fn looks_like_codex(path: &Path) -> bool {
        path.file_name()
            .map(|name| {
                let name = name.to_string_lossy().to_ascii_lowercase();
                name == "codex" || name == "codex.exe"
            })
            .unwrap_or(false)
    }

    // 1) 注册表声明
    for tool_id in ["codex-cli", "chatgptdesktop"] {
        if let Some((_, paths)) = crate::commands::ai_registry::registry().get_tool(tool_id) {
            if let Some(exe) =
                super::tool_paths::find_declared_exe(tool_id, &paths.paths, &paths.command)
            {
                // 桌面端工具声明的是 ChatGPT.exe，不能拿来当 CLI 用
                if looks_like_codex(&exe) {
                    return Some(exe);
                }
            }
        }
    }

    // 2) config.toml 里 Codex 自己写的 CODEX_CLI_PATH
    if let Ok(config_file) = config_path() {
        if let Ok(config) = std::fs::read_to_string(&config_file) {
            let doc = crate::commands::ai::launch::parse_toml_lenient(&config);
            if let Some(servers) = doc.get("mcp_servers").and_then(|s| s.as_table()) {
                for (_, server) in servers.iter() {
                    let Some(env) = server
                        .as_table()
                        .and_then(|t| t.get("env"))
                        .and_then(|e| e.as_table())
                    else {
                        continue;
                    };
                    if let Some(raw) = env.get("CODEX_CLI_PATH").and_then(|v| v.as_str()) {
                        let candidate = PathBuf::from(raw);
                        if candidate.is_file() {
                            return Some(candidate);
                        }
                    }
                }
            }
        }
    }

    // 3) 客户端自带的 app-server 目录
    if let Ok(home) = codex_home() {
        let bundled = home
            .join("plugins")
            .join(".plugin-appserver")
            .join("codex.exe");
        if bundled.is_file() {
            return Some(bundled);
        }
    }

    None
}

/// CLI 是否可用（界面据此提示「插件安装不可用」而不是让用户点了报错）。
pub(crate) fn codex_cli_available() -> bool {
    resolve_codex_cli().is_some()
}

/// 跑一次 `codex …`，成功时返回 stdout。
fn run_codex_cli(args: &[&str], timeout_secs: u64) -> Result<String, String> {
    let exe = resolve_codex_cli().ok_or_else(|| {
        "找不到 Codex CLI（codex.exe）—— 插件安装依赖它把插件落到客户端缓存里。".to_string()
    })?;
    let mut cmd = std::process::Command::new(&exe);
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let output = super::detect::run_command_with_timeout(&mut cmd, timeout_secs)
        .ok_or_else(|| format!("`codex {}` 执行失败或超时（{timeout_secs}s）", args.join(" ")))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            stdout.trim().to_string()
        } else {
            stderr
        };
        return Err(format!("`codex {}` 失败：{detail}", args.join(" ")));
    }
    Ok(stdout)
}

/// 解析 `codex plugin list --json`：`{ "installed": [...], "available": [...] }`。
fn parse_plugin_list_json(text: &str) -> HashMap<String, PluginState> {
    let mut out = HashMap::new();
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(text) else {
        return out;
    };
    for section in ["installed", "available"] {
        let Some(items) = doc.get(section).and_then(|v| v.as_array()) else {
            continue;
        };
        for item in items {
            let Some(name) = item.get("name").and_then(|v| v.as_str()) else {
                continue;
            };
            out.insert(
                name.to_string(),
                PluginState {
                    installed: item
                        .get("installed")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                    enabled: item.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false),
                    version: item
                        .get("version")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                },
            );
        }
    }
    out
}

/// 从 `config.toml` 读本市场的插件状态（CLI 不可用时的回退路径）。
fn config_plugin_states(config: &str) -> Vec<(String, PluginState)> {
    let doc = crate::commands::ai::launch::parse_toml_lenient(config);
    let Some(plugins) = doc.get("plugins").and_then(|p| p.as_table()) else {
        return Vec::new();
    };
    let suffix = format!("@{MARKETPLACE_NAME}");
    plugins
        .iter()
        .filter_map(|(key, item)| {
            let name = key.strip_suffix(&suffix)?.to_string();
            let enabled = item
                .as_table()
                .and_then(|t| t.get("enabled"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Some((
                name,
                PluginState {
                    installed: true,
                    enabled,
                    version: None,
                },
            ))
        })
        .collect()
}

/// 全部插件状态：优先问官方 CLI（有 version / enabled），失败则回退到 config.toml。
fn plugin_states() -> HashMap<String, PluginState> {
    let mut states = match run_codex_cli(
        &[
            "plugin",
            "list",
            "--marketplace",
            MARKETPLACE_NAME,
            "--available",
            "--json",
        ],
        60,
    ) {
        Ok(text) => parse_plugin_list_json(&text),
        Err(e) => {
            // 只是拿不到 version / enabled 细节，不该让整个列表打不开
            eprintln!("[codex_plugins] 读取插件状态失败，回退到 config.toml：{e}");
            HashMap::new()
        }
    };
    if let Ok(config_file) = config_path() {
        let config = std::fs::read_to_string(&config_file).unwrap_or_default();
        for (name, state) in config_plugin_states(&config) {
            states.entry(name).or_insert(state);
        }
    }
    states
}

/// 列出本市场的插件（清单声明 + 当前安装状态）。
#[tauri::command]
pub fn codex_list_marketplace_plugins() -> Result<Vec<CodexPluginInfo>, String> {
    let root = marketplace_root()?;
    let states = plugin_states();
    Ok(read_marketplace_plugins(&root)
        .into_iter()
        .map(|entry| {
            let state = states.get(&entry.name).cloned().unwrap_or_default();
            CodexPluginInfo {
                name: entry.name,
                category: entry.category,
                installed: state.installed,
                enabled: state.enabled,
                version: state.version,
            }
        })
        .collect())
}

/// 安装单个插件：`codex plugin add <插件>@<市场> --json`。
///
/// 走 `spawn_blocking`：CLI 要落盘插件目录（几十 MB 级）并可能联网，不能占住主线程。
#[tauri::command]
pub async fn codex_install_plugin(name: String) -> Result<Vec<CodexPluginInfo>, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("插件名不能为空".to_string());
    }
    let selector = format!("{name}@{MARKETPLACE_NAME}");
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        run_codex_cli(&["plugin", "add", selector.as_str(), "--json"], 300)?;
        Ok(())
    })
    .await
    .map_err(|e| format!("插件安装任务异常退出: {e}"))??;
    codex_list_marketplace_plugins()
}

/// 卸载单个插件：`codex plugin remove <插件>@<市场> --json`。
#[tauri::command]
pub async fn codex_uninstall_plugin(name: String) -> Result<Vec<CodexPluginInfo>, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("插件名不能为空".to_string());
    }
    let selector = format!("{name}@{MARKETPLACE_NAME}");
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        run_codex_cli(&["plugin", "remove", selector.as_str(), "--json"], 120)?;
        Ok(())
    })
    .await
    .map_err(|e| format!("插件卸载任务异常退出: {e}"))??;
    codex_list_marketplace_plugins()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 市场名不能是 Codex 的保留名：注册在 `openai-*` 下的本地市场会被**静默忽略**，
    /// 表现为「装好了但插件列表一个都不显示」，极难排查（issue #1974 / #1968）。
    #[test]
    fn marketplace_name_is_not_a_reserved_openai_name() {
        assert!(
            !MARKETPLACE_NAME.starts_with("openai-"),
            "保留名会让整个市场被 Codex 忽略：{MARKETPLACE_NAME}"
        );
    }

    /// 注册判定的两个条件必须同时成立：表名对上、且 source 与磁盘目录一致。
    /// 只改一边（例如改名后没重写 source）会导致整个市场不被识别。
    #[test]
    fn registration_requires_matching_name_and_source() {
        let root = PathBuf::from("/tmp/mkt");
        let source = marketplace_source_value(&root);
        // 用 TOML **字面量字符串**（单引号）承载路径：Windows 的 `\\?\` 前缀在基本字符串里
        // 是转义序列，拼进去会把路径改掉 —— 真实写入由 `toml_edit::value` 负责转义，
        // 测试这里只需构造出「解析后等于 source」的文本。
        let registered = format!(
            "[marketplaces.{MARKETPLACE_NAME}]\nsource_type = \"local\"\nsource = '{source}'\n"
        );
        assert!(is_registered(&registered, &root));

        // source 指向别处 → 不算注册
        let elsewhere = "[marketplaces.anyversion-curated]\nsource_type = \"local\"\nsource = \"/tmp/other\"\n";
        assert!(!is_registered(elsewhere, &root));
        // 表名不对 → 不算注册（source 完全一致，只有名字不同，才能证明名字这一项被检查了）
        let wrong_name = format!(
            "[marketplaces.someone-else]\nsource_type = \"local\"\nsource = '{source}'\n"
        );
        assert!(!is_registered(&wrong_name, &root));
    }

    /// 解压要剥掉 `plugins-main/` 顶层目录，否则市场清单会落在
    /// `<root>/plugins-main/.agents/...`，客户端按约定路径找不到。
    #[test]
    fn zip_entries_strip_archive_root_and_reject_escapes() {
        assert_eq!(
            safe_zip_entry("plugins-main/plugins/gmail/file.txt"),
            Some(PathBuf::from("plugins").join("gmail").join("file.txt"))
        );
        assert_eq!(safe_zip_entry("plugins-main/../evil.txt"), None);
        assert_eq!(safe_zip_entry("../evil.txt"), None);
        assert_eq!(safe_zip_entry("plugins-main/"), None, "纯目录根没有相对路径");
    }

    /// 端到端：解一份最小 zip，清单要落在约定位置，插件数要读得出来。
    #[test]
    fn extract_places_manifest_at_the_conventional_path() {
        // 用**官方仓库的真实形状**（`source.path` 才是路径所在，顶层没有 `path`；
        // 且清单里的 name 是保留名 `openai-curated`，落盘时必须改名）
        let manifest = r#"{"name":"openai-curated","plugins":[{"name":"chronograph","source":{"source":"local","path":"./plugins/chronograph"},"policy":{"installation":"AVAILABLE"},"category":"Finance"}]}"#;
        let mut bytes = Cursor::new(Vec::<u8>::new());
        {
            let mut writer = zip::ZipWriter::new(&mut bytes);
            let options =
                zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            writer
                .start_file("plugins-main/.agents/plugins/marketplace.json", options)
                .unwrap();
            std::io::Write::write_all(&mut writer, manifest.as_bytes()).unwrap();
            writer.start_file("plugins-main/plugins/chronograph/.app.json", options).unwrap();
            std::io::Write::write_all(&mut writer, b"{}").unwrap();
            writer.finish().unwrap();
        }
        let payload = bytes.into_inner();

        let dir = std::env::temp_dir().join(format!("av-mkt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        extract_zip_stripping_root(&payload, &dir).unwrap();

        assert!(marketplace_json_path(&dir).exists(), "清单必须落在约定路径");
        let plugins = read_marketplace_plugins(&dir);
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].name, "chronograph");
        assert_eq!(plugins[0].category, "Finance", "分类要给界面分组用");
        assert_eq!(plugins[0].path, "./plugins/chronograph", "路径读的是 source.path");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Codex 自己写的 config.toml 里有重复表头**（实测这台机器上
    /// `[marketplaces.openai-primary-runtime]` 出现 3 次、`[plugins."…"]` 多次）。
    /// 严格解析会失败 → 注册静默失效，用户点了「安装」却什么都没发生。
    /// 这里用真实形态的输入锁住行为：注册成功、原有市场与插件一个不丢、结果仍是合法 TOML。
    #[test]
    fn registration_survives_duplicate_table_headers() {
        let root = PathBuf::from("/tmp/mkt");
        let existing = r#"
[marketplaces.openai-bundled]
source_type = "local"
source = '\\?\C:\Users\x\.codex\.tmp\bundled-marketplaces\openai-bundled'

[marketplaces.openai-primary-runtime]
source = '\\?\C:\a'

[marketplaces.openai-primary-runtime]
source_type = "local"
source = '\\?\C:\b'

[plugins."pdf@openai-primary-runtime"]
enabled = true
"#;
        let out = merge_marketplace_registration(existing, &root);
        assert!(out.contains("[marketplaces.anyversion-curated]"), "{out}");
        // Codex 原有的市场与已启用插件不能被我们弄丢
        assert!(out.contains("openai-bundled"), "{out}");
        assert!(out.contains("openai-primary-runtime"), "{out}");
        assert!(out.contains("pdf@openai-primary-runtime"), "{out}");
        // 结果必须合法，否则 Codex 直接读不了配置文件
        out.parse::<toml_edit::DocumentMut>().expect("合并结果应可解析");
        assert!(is_registered(&out, &root), "注册后状态应为已注册");
        // 别家市场的启用项要保留，但**不能**被算进本市场的启用数
        assert_eq!(enabled_plugin_count(&out), 0, "只数本市场的启用项");
    }

    /// 启用计数只认本市场：别的市场的 `enabled = true` 不能算进来
    /// （用户同时还装着 openai-bundled 的插件，界面上「已启用 N 个」必须是本市场的数）。
    #[test]
    fn enabled_count_only_counts_our_marketplace() {
        let config = format!(
            "[plugins.\"a@{m}\"]\nenabled = true\n\n[plugins.\"b@{m}\"]\nenabled = false\n\n\
             [plugins.\"c@{m}\"]\n\n[plugins.\"pdf@openai-primary-runtime\"]\nenabled = true\n",
            m = MARKETPLACE_NAME
        );
        assert_eq!(enabled_plugin_count(&config), 1, "只数本市场的 enabled");
        assert_eq!(enabled_plugin_count(""), 0);
    }

    /// `name` 是保留名时必须被替换：磁盘上的市场名与 `config.toml` 里的表名不一致时，
    /// 客户端会把整个市场静默忽略 —— 用户看到的是「安装成功但一个插件都没有」。
    #[test]
    fn rewrite_marketplace_name_replaces_reserved_name() {
        let dir = std::env::temp_dir().join(format!("av-mkt-name-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".agents").join("plugins")).unwrap();
        std::fs::write(
            marketplace_json_path(&dir),
            r#"{"name":"openai-curated","interface":{"displayName":"Codex official"},"plugins":[]}"#,
        )
        .unwrap();

        rewrite_marketplace_name(&dir).unwrap();
        let text = std::fs::read_to_string(marketplace_json_path(&dir)).unwrap();
        assert!(text.contains(MARKETPLACE_NAME), "{text}");
        assert!(!text.contains("openai-curated"), "保留名必须被换掉: {text}");
        // 其余字段不能被我们弄丢
        assert!(text.contains("Codex official"), "{text}");
        // 幂等
        rewrite_marketplace_name(&dir).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// CLI 的 JSON 结构是 `{installed:[…], available:[…]}`，两段都要读：
    /// 已装的给 enabled/version，未装的给「可安装」态。
    #[test]
    fn parse_plugin_list_json_reads_both_sections() {
        let json = r#"{
            "installed": [{"name":"figma","installed":true,"enabled":true,"version":"2.0.7"}],
            "available": [{"name":"notion","installed":false,"enabled":false,"version":"1.2.3"}]
        }"#;
        let states = parse_plugin_list_json(json);
        assert!(states["figma"].installed && states["figma"].enabled);
        assert_eq!(states["figma"].version.as_deref(), Some("2.0.7"));
        assert!(!states["notion"].installed);
        assert_eq!(states["notion"].version.as_deref(), Some("1.2.3"));
        // 非法 JSON 不能 panic，返回空表即可
        assert!(parse_plugin_list_json("not json").is_empty());
    }

    /// CLI 不可用时的回退路径：从 config.toml 认出本市场已启用的插件，
    /// 且**不能**把其它市场的插件混进来。
    #[test]
    fn config_plugin_states_scopes_to_our_marketplace() {
        let config = format!(
            "[plugins.\"figma@{m}\"]\nenabled = true\n\n[plugins.\"pdf@openai-primary-runtime\"]\nenabled = true\n",
            m = MARKETPLACE_NAME
        );
        let states = config_plugin_states(&config);
        assert_eq!(states.len(), 1, "只应认出本市场的插件");
        assert_eq!(states[0].0, "figma");
        assert!(states[0].1.installed && states[0].1.enabled);
    }
}
