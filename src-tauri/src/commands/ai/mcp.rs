use crate::commands::ai_registry::registry;
use crate::commands::config::get_data_dir;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

// ─── 数据模型 ───

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct McpServer {
    pub id: String,
    pub name: String,
    /// stdio | http | sse
    pub transport: String,
    /// stdio 启动命令（如 npx）
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub cwd: Option<String>,
    /// http / sse 的地址
    pub url: String,
    pub headers: HashMap<String, String>,
    /// 全局启用：false 时不部署到任何工具
    pub enabled: bool,
    /// 已部署到的工具 id 列表
    pub enabled_tools: Vec<String>,
    pub description: Option<String>,
    pub install_method: String,
    /// 托管安装来源：有值时这台服务器由本应用从 GitHub Release 下载安装，
    /// `command` 自动指向解压出来的可执行文件（`None` = 用户自己填命令）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed: Option<McpManagedSource>,
}

/// 托管安装来源（GitHub Release）。
///
/// 为什么要单独存这组字段：MCP 服务器里有相当一部分不是 npm 包，而是「下载发行包 →
/// 解压 → 跑里面的 bin」，这套动作原来只在 SDK 工具注册表（`projects/*/config.json`）里有。
/// 但 MCP 服务器不是 SDK：它不该出现在语言/CLI 的工具列表里，也不该为了能被装而伪装成
/// 一个 SDK 条目。所以下载、解压、版本目录都放在 MCP 这一侧，与 SDK 完全解耦。
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpManagedSource {
    /// GitHub 仓库 `owner/repo`
    pub repo: String,
    /// release tag 模板，如 `v{version}`
    pub tag_template: String,
    /// 资产名前缀模板，如 `jadx-headless-mcp-{version}`
    pub asset_prefix: String,
    /// 资产名后缀，如 `.zip`
    pub asset_suffix: String,
    /// 解压后要进入的子目录模板（Gradle 发行包的 `<name>-<version>/`）；
    /// 留空表示自动剥掉唯一的顶层目录
    #[serde(default)]
    pub extract_subdir: String,
    /// 可执行文件相对安装根的路径，如 `bin/jadx-headless-mcp.bat`
    pub executable: String,
}

/// 从工具配置中发现、但尚未由 vex 托管的 MCP 服务器
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredMcp {
    pub tool_id: String,
    pub name: String,
    pub transport: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub cwd: Option<String>,
    pub url: String,
    pub headers: HashMap<String, String>,
    /// 同名服务器是否已在 vex 托管（已托管则无需纳入）
    pub already_managed: bool,
}

#[derive(Serialize, Deserialize, Default)]
struct McpStore {
    #[serde(default)]
    servers: Vec<McpServer>,
}

fn mcp_path() -> PathBuf {
    get_data_dir().join("mcp.json")
}

fn load_store() -> McpStore {
    let path = mcp_path();
    if path.exists() {
        if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(s) = serde_json::from_str::<McpStore>(&data) {
                return s;
            }
        }
    }
    McpStore::default()
}

fn save_store(store: &McpStore) -> Result<(), String> {
    let data = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
    crate::commands::config::atomic_write_file(&mcp_path(), data.as_bytes())
}

fn normalize_id(name: &str) -> String {
    name.to_lowercase().replace(' ', "-").replace('_', "-")
}

// ─── 命令 ───

#[tauri::command]
pub fn get_mcp_servers() -> Result<Vec<McpServer>, String> {
    Ok(load_store().servers)
}

// ─── MCP 预设 ───

/// MCP 服务器预设：新增服务器表单里的「从预设添加」。
///
/// 只做**预填表单**，不直接落库 —— 预设给的命令 / 地址用户通常还要改（换端口、加 token、
/// 换成本地已装的路径），一键写死反而更难改。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpPreset {
    pub id: String,
    pub name: String,
    pub description: String,
    /// stdio | http | sse
    pub transport: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub url: String,
    /// 依赖的本地服务 id（如 wigolo 的服务项 id）；空表示无依赖。
    /// 前端可用它提示「先在服务页启动该服务」。
    #[serde(default)]
    pub service_id: String,
    /// 托管安装来源：有值时添加后可直接点「安装」（从 GitHub Release 下载并解压）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed: Option<McpManagedSource>,
}

/// 内置预设清单。
///
/// wigolo 给两条，因为两种接法取舍不同：stdio 由工具自己拉起进程（不必开服务），
/// HTTP 复用已在服务页启动的常驻服务（多个工具共用一个缓存与模型）。
pub fn mcp_presets() -> Vec<McpPreset> {
    vec![
        McpPreset {
            id: "wigolo-stdio".to_string(),
            name: "wigolo（stdio）".to_string(),
            description: "由工具自己拉起 wigolo 进程（npx -y wigolo），无需先在「服务」页启动；每个工具各占一份进程与缓存。首次使用前建议先在「服务」页对 wigolo 点一次「初始化」（约 1.5 GB）。".to_string(),
            transport: "stdio".to_string(),
            command: "npx".to_string(),
            args: vec!["-y".to_string(), "wigolo".to_string()],
            url: String::new(),
            service_id: "wigolo".to_string(),
            managed: None,
        },
        McpPreset {
            id: "wigolo-http".to_string(),
            name: "wigolo（HTTP）".to_string(),
            description: "复用「服务」页里已启动的 wigolo 服务（127.0.0.1:3333）上的 /mcp 端点：多个工具共用一个缓存与本地模型。须先启动该服务，否则连接失败。".to_string(),
            transport: "http".to_string(),
            command: String::new(),
            args: Vec::new(),
            url: "http://127.0.0.1:3333/mcp".to_string(),
            service_id: "wigolo".to_string(),
            managed: None,
        },
        McpPreset {
            id: "jadx-headless-mcp".to_string(),
            name: "jadx-headless-mcp".to_string(),
            description: "Android APK 静态分析（内嵌 jadx-core，不需要另装 jadx）。点「安装」会从 GitHub Release 下载发行包并解压到本应用数据目录，命令自动指向 bin/jadx-headless-mcp.bat；运行时需要 JDK 17+ 可用（装了 SDK 里的 Java 即可）。".to_string(),
            transport: "stdio".to_string(),
            command: String::new(),
            args: Vec::new(),
            url: String::new(),
            service_id: String::new(),
            managed: Some(McpManagedSource {
                repo: "1013503897/jadx-headless-mcp".to_string(),
                tag_template: "v{version}".to_string(),
                asset_prefix: "jadx-headless-mcp-{version}".to_string(),
                asset_suffix: ".zip".to_string(),
                extract_subdir: "jadx-headless-mcp-{version}".to_string(),
                executable: "bin/jadx-headless-mcp.bat".to_string(),
            }),
        },
    ]
}

#[tauri::command]
pub fn get_mcp_presets() -> Vec<McpPreset> {
    mcp_presets()
}

#[tauri::command]
pub fn save_mcp_server(server: McpServer) -> Result<(), String> {
    let name = server.name.trim();
    if name.is_empty() {
        return Err("服务器名称不能为空".to_string());
    }
    let id = normalize_id(name);
    if server.transport == "stdio" {
        // 托管条目允许「已登记但还没安装」：command 由安装命令写入，
        // 这里拦下来用户就没法先把条目存下来再点安装。
        let pending_managed = server.managed.is_some() && server.command.trim().is_empty();
        if server.command.trim().is_empty() && !pending_managed {
            return Err("stdio 类型必须填写启动命令".to_string());
        }
    } else if server.transport == "http" || server.transport == "sse" {
        if server.url.trim().is_empty() {
            return Err("http/sse 类型必须填写 URL".to_string());
        }
    } else {
        return Err(format!("不支持的传输类型: {}", server.transport));
    }

    let mut store = load_store();
    // 若改名导致 id 变化，先移除旧 id（按传入 id 匹配）
    store.servers.retain(|s| s.id != server.id && s.id != id);
    store.servers.push(McpServer {
        id: id.clone(),
        name: name.to_string(),
        transport: server.transport.clone(),
        command: server.command.clone(),
        args: server.args.clone(),
        env: server.env.clone(),
        cwd: server.cwd.clone(),
        url: server.url.clone(),
        headers: server.headers.clone(),
        enabled: server.enabled,
        enabled_tools: server.enabled_tools.clone(),
        description: server.description.clone(),
        install_method: server.install_method.clone(),
        managed: server.managed.clone(),
    });
    save_store(&store)
}

#[tauri::command]
pub fn delete_mcp_server(id: String) -> Result<(), String> {
    let mut store = load_store();
    store.servers.retain(|s| s.id != id);
    save_store(&store)
}

// 注：原先的 `adopt_mcp_server`（把发现的服务器纳入托管并立刻部署回全局）已删除——
// 它和「全局部署」是一套的：纳入托管的目的就是写进各工具的全局配置。
// 现在唯一会把全局条目收进仓库的入口是 `migrate_global_mcp`，而且它同时**删掉**
// 全局文件里的那份（这才是「不污染全局」的落点）。

/// 把各工具配置里的服务器片段反解析为规范化字段（覆盖 claude / opencode / gemini / qwen 等格式）
fn reverse_translate(name: &str, entry: &Value) -> DiscoveredMcp {
    let obj = match entry.as_object() {
        Some(o) => o,
        None => {
            return DiscoveredMcp {
                tool_id: String::new(),
                name: name.to_string(),
                transport: "stdio".into(),
                command: String::new(),
                args: vec![],
                env: HashMap::new(),
                cwd: None,
                url: String::new(),
                headers: HashMap::new(),
                already_managed: false,
            }
        }
    };
    let get = |k: &str| {
        obj.get(k)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };

    let transport = if obj.contains_key("url") || obj.contains_key("httpUrl") {
        if get("type") == "sse" {
            "sse".to_string()
        } else {
            "http".to_string()
        }
    } else {
        "stdio".to_string()
    };

    let (command, mut args) = if let Some(c) = obj.get("command") {
        if let Some(s) = c.as_str() {
            (s.to_string(), vec![])
        } else if let Some(arr) = c.as_array() {
            let cmd = arr
                .first()
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let a: Vec<String> = arr
                .iter()
                .skip(1)
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
            (cmd, a)
        } else {
            (String::new(), vec![])
        }
    } else {
        (String::new(), vec![])
    };
    if args.is_empty() {
        if let Some(v) = obj.get("args") {
            if let Ok(a) = serde_json::from_value::<Vec<String>>(v.clone()) {
                args = a;
            }
        }
    }

    let env = obj
        .get("env")
        .or_else(|| obj.get("environment"))
        .and_then(|v| serde_json::from_value::<HashMap<String, String>>(v.clone()).ok())
        .unwrap_or_default();
    let headers = obj
        .get("headers")
        .and_then(|v| serde_json::from_value::<HashMap<String, String>>(v.clone()).ok())
        .unwrap_or_default();
    let url = get("url");
    let url = if url.is_empty() { get("httpUrl") } else { url };
    let cwd = obj
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    DiscoveredMcp {
        tool_id: String::new(),
        name: name.to_string(),
        transport,
        command,
        args,
        env,
        cwd,
        url,
        headers,
        already_managed: false,
    }
}

#[cfg(test)]
mod tests;

mod managed;
pub use managed::*;

mod projects;
pub use projects::*;

mod source;
pub use source::*;

mod global;
pub use global::*;
