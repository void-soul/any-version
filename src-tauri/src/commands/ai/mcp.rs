use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use serde_json::{Map, Value};
use crate::commands::ai_registry::registry;
use crate::commands::config::get_data_dir;

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
        },
    ]
}

#[tauri::command]
pub fn get_mcp_presets() -> Vec<McpPreset> {
    mcp_presets()
}

#[cfg(test)]
mod tests {
    use super::mcp_presets;
    use crate::commands::ai_registry::registry;
    use super::{project_servers_of, translate, write_project_mcp, McpServer};
    use serde_json::{json, Map, Value};
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;

    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kira-mcp-{}-{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn server(name: &str) -> McpServer {
        McpServer {
            id: name.to_string(),
            name: name.to_string(),
            transport: "stdio".to_string(),
            command: "npx".to_string(),
            args: vec!["-y".to_string(), format!("{}@latest", name)],
            env: HashMap::new(),
            cwd: None,
            url: String::new(),
            headers: HashMap::new(),
            enabled: true,
            enabled_tools: Vec::new(),
            description: None,
            install_method: "manual".to_string(),
        }
    }

    /// 装到项目 = 写 `<项目>/.mcp.json` 的 `mcpServers`，且**只动这一个键**。
    ///
    /// 整文件覆盖会把用户 `.mcp.json` 里的其它配置抹掉（inputs/outputs、各工具自己的
    /// 扩展键），所以这条是硬要求。
    #[test]
    fn install_merges_and_preserves_other_keys() {
        let dir = tmp_dir("install");
        let path = dir.join(".mcp.json");
        fs::write(
            &path,
            serde_json::to_string_pretty(&json!({
                "inputs": [{"id": "token"}],
                "mcpServers": {"hand-written": {"url": "https://example.com/mcp"}}
            }))
            .unwrap(),
        )
        .unwrap();

        let s = server("gitnexus");
        let n = write_project_mcp(&path, &s.name, Some(translate(&s, "mcp"))).unwrap();
        assert_eq!(n, 2, "装完应有2 个 server");

        let val: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let servers = project_servers_of(&val);
        assert!(servers.contains_key("gitnexus"), "应写入新 server");
        assert!(
            servers.contains_key("hand-written"),
            "**不能动**用户原有的 server 条目"
        );
        assert_eq!(servers["gitnexus"]["command"], json!("npx"));
        assert_eq!(
            servers["gitnexus"]["args"],
            json!(["-y", "gitnexus@latest"]),
            "stdio 的 args 必须完整写进去，漏一个参数就启动不了"
        );
        assert_eq!(val["inputs"][0]["id"], json!("token"), "其它键必须原样保留");

        // 移除只删自己那一条
        write_project_mcp(&path, "gitnexus", None).unwrap();
        let val: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let servers = project_servers_of(&val);
        assert_eq!(servers.len(), 1);
        assert!(servers.contains_key("hand-written"));

        let _ = fs::remove_dir_all(&dir);
    }

    /// 文件不存在时应当**新建**一个合法的空配置，而不是报错或跳过。
    #[test]
    fn install_creates_missing_file() {
        let dir = tmp_dir("create");
        let path = dir.join("nested").join(".mcp.json");
        let s = server("a");
        write_project_mcp(&path, &s.name, Some(translate(&s, "mcp"))).unwrap();
        let val: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(project_servers_of(&val).len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    /// `mcpServers` 不是对象时（用户写了数组、或文件被别的工具改成字符串）不能崩。
    #[test]
    fn malformed_mcp_servers_is_tolerated() {
        let val = json!({ "mcpServers": ["not", "an", "object"] });
        assert!(
            project_servers_of(&val).is_empty(),
            "非对象的 mcpServers 应被当成空，而不是 panic"
        );
        let empty = json!({});
        assert!(project_servers_of(&empty).is_empty());
        let good = json!({ "mcpServers": { "a": {} } });
        assert_eq!(project_servers_of(&good).len(), 1);
        let _ = Map::new();
    }

    /// http 类server：只有 url，没有 command/args。
    #[test]
    fn http_server_translation_carries_url() {
        let mut s = server("cloudflare-docs");
        s.transport = "http".to_string();
        s.command = String::new();
        s.args = Vec::new();
        s.url = "https://docs.mcp.cloudflare.com/mcp".to_string();
        let entry = translate(&s, "mcp");
        assert_eq!(entry["url"], json!("https://docs.mcp.cloudflare.com/mcp"));
        assert!(entry.get("command").is_none(), "http 不该带 command");
    }

    /// 已登记的工具（**尤其 codebuddy**）必须真能解析出它的 MCP 配置路径。
    ///
    /// 漏登记的后果特别隐蔽：用户明明在 `~/.codebuddy/mcp.json` 配了 6 个，Kira 却只看到 1 个，
    /// 而且界面上「全局配置里还有 N 个 MCP」整块**根本不渲染**（条件是 length > 0），
    /// 看起来像功能没做，而不是数据没读到。
    #[test]
    fn registered_tools_expose_their_mcp_config() {
        let reg = registry();
        let ids = reg.mcp_tool_ids();
        assert!(
            ids.iter().any(|i| i == "codebuddy"),
            "codebuddy 必须登记在 MCP 工具注册表里，否则读不到 ~/.codebuddy/mcp.json（实测漏过）"
        );
        for id in &ids {
            let (path, _f, k) = reg
                .get_tool_mcp_config(id)
                .unwrap_or_else(|| panic!("{} 应能解析出配置", id));
            assert!(
                !path.to_string_lossy().contains('~'),
                "{} 的路径应已展开 ~，实际 {}",
                id,
                path.display()
            );
            assert!(!k.is_empty(), "{} 的 mcp 键不应为空", id);
        }
        // 已登记就能真的读到条目：读不到时用户会看到「明明配了 6 个却只看到 1 个」，
        // 而界面上因为整块条件是 length > 0，看起来像功能没做
        let list = super::list_global_mcp().expect("list_global_mcp 应成功");
        assert!(!list.is_empty(), "至少应能读到本机的 MCP 条目");
        for d in &list {
            assert!(
                !d.tool_id.is_empty(),
                "每条都必须带上来源工具 id（迁移界面要靠它显示来源）"
            );
        }
    }

    #[test]
    fn test_presets_are_well_formed() {
        let presets = mcp_presets();
        assert!(!presets.is_empty(), "预设清单不应为空");
        for preset in &presets {
            assert!(!preset.id.trim().is_empty());
            assert!(!preset.name.trim().is_empty());
            assert!(!preset.description.trim().is_empty(), "{} 缺少描述", preset.id);
            match preset.transport.as_str() {
                "stdio" => assert!(
                    !preset.command.trim().is_empty(),
                    "{} 是 stdio 预设，必须给 command",
                    preset.id
                ),
                "http" | "sse" => assert!(
                    preset.url.starts_with("http://") || preset.url.starts_with("https://"),
                    "{} 的 url 非法: {}",
                    preset.id,
                    preset.url
                ),
                other => panic!("{} 的 transport 非法: {}", preset.id, other),
            }
        }
    }

    /// HTTP 预设的端口必须与 `node-projects/wigolo.json` 的服务端口一致：
    /// 服务项改端口而预设没跟上时，用户点了预设会连到一个没人监听的地址。
    #[test]
    fn test_wigolo_http_preset_matches_service_def() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../node-projects/wigolo.json");
        let raw = std::fs::read_to_string(&path).expect("wigolo.json 读取失败");
        let value: serde_json::Value = serde_json::from_str(&raw).expect("wigolo.json 解析失败");
        let port = value
            .get("defaultPort")
            .and_then(|v| v.as_u64())
            .expect("wigolo.json 缺少 defaultPort");
        assert!(
            value.get("initCmd").and_then(|v| v.as_array()).is_some(),
            "wigolo 需要配置 initCmd，否则前端不会出现「初始化」入口"
        );

        let preset = mcp_presets()
            .into_iter()
            .find(|p| p.id == "wigolo-http")
            .expect("缺少 wigolo-http 预设");
        assert!(
            preset.url.contains(&format!("127.0.0.1:{}", port)),
            "预设地址 {} 与服务端口 {} 不一致",
            preset.url,
            port
        );
        assert!(preset.url.ends_with("/mcp"));
    }
}

#[tauri::command]
pub fn save_mcp_server(server: McpServer) -> Result<(), String> {
    let name = server.name.trim();
    if name.is_empty() {
        return Err("服务器名称不能为空".to_string());
    }
    let id = normalize_id(name);
    if server.transport == "stdio" {
        if server.command.trim().is_empty() {
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
        None => return DiscoveredMcp {
            tool_id: String::new(), name: name.to_string(), transport: "stdio".into(),
            command: String::new(), args: vec![], env: HashMap::new(), cwd: None,
            url: String::new(), headers: HashMap::new(), already_managed: false,
        },
    };
    let get = |k: &str| obj.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();

    let transport = if obj.contains_key("url") || obj.contains_key("httpUrl") {
        if get("type") == "sse" { "sse".to_string() } else { "http".to_string() }
    } else {
        "stdio".to_string()
    };

    let (command, mut args) = if let Some(c) = obj.get("command") {
        if let Some(s) = c.as_str() {
            (s.to_string(), vec![])
        } else if let Some(arr) = c.as_array() {
            let cmd = arr.first().and_then(|v| v.as_str()).unwrap_or("").to_string();
            let a: Vec<String> = arr.iter().skip(1).filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
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

    let env = obj.get("env").or_else(|| obj.get("environment"))
        .and_then(|v| serde_json::from_value::<HashMap<String, String>>(v.clone()).ok())
        .unwrap_or_default();
    let headers = obj.get("headers")
        .and_then(|v| serde_json::from_value::<HashMap<String, String>>(v.clone()).ok())
        .unwrap_or_default();
    let url = get("url");
    let url = if url.is_empty() { get("httpUrl") } else { url };
    let cwd = obj.get("cwd").and_then(|v| v.as_str()).map(|s| s.to_string());

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

// ─── 项目级安装（MCP 不再全局部署） ───
//
// 早先的模型是「托管一份清单 → deploy_all 写进各开发工具的全局配置」，后果是
// **所有项目共享同一批 MCP**：为 gitnexus 付的 17 个工具的 schema，会出现在每一个项目的
// 每一次请求里（哪怕那个项目跟代码图谱毫无关系），而全局清单里躺着没连过的条目也一样占位。
//
// 现在只写项目级`<项目>/.mcp.json` —— agent 在这个项目里只看得到这个文件，
// 没写就是没连接、不spawn、不进上下文。Kira 不再往任何工具的全局配置写东西。

/// 项目 `.mcp.json` 里的一个条目（= 项目「已装」的 MCP）
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMcp {
    /// 在项目文件里的键名
    pub name: String,
    /// stdio | http | sse
    pub transport: String,
    pub command: String,
    pub args: Vec<String>,
    pub url: String,
    /// 仓库里有同名定义 = 受管（Kira 装的）；否则是用户手写的「项目自带」
    pub managed: bool,
}

fn project_mcp_path(project_id: &str) -> Result<PathBuf, String> {
    let dir = super::skills::project_dir_of(project_id)?;
    Ok(Path::new(&dir).join(".mcp.json"))
}

/// 读项目的 `.mcp.json`；不存在或不是合法对象时返回空对象。
///
/// **解析失败不静默当空**：直接返回错误。否则用户手写坏了一个逗号，UI 会显示
/// 「这个项目没装任何 MCP」，而实际文件里有 6 个 —— 看起来像我们的功能坏了。
fn read_project_mcp_file(path: &Path) -> Result<Value, String> {
    if !path.is_file() {
        return Ok(Value::Object(Map::new()));
    }
    let text = fs::read_to_string(path).map_err(|e| format!("读取 {} 失败: {}", path.display(), e))?;
    if text.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    match serde_json::from_str::<Value>(&text) {
        Ok(v) if v.is_object() => Ok(v),
        Ok(_) => Err(format!("{} 不是合法的 JSON 对象", path.display())),
        Err(e) => Err(format!("解析 {} 失败: {}", path.display(), e)),
    }
}

fn project_servers_of(file_val: &Value) -> Map<String, Value> {
    match file_val.get("mcpServers") {
        Some(Value::Object(m)) => m.clone(),
        _ => Map::new(),
    }
}

/// 列出某个项目已装的 MCP
#[tauri::command]
pub fn list_project_mcps(project_id: String) -> Result<Vec<ProjectMcp>, String> {
    let path = project_mcp_path(&project_id)?;
    let file_val = read_project_mcp_file(&path)?;
    let managed: Vec<String> = load_store().servers.iter().map(|s| s.name.clone()).collect();
    let mut out: Vec<ProjectMcp> = Vec::new();
    for (name, entry) in project_servers_of(&file_val) {
        if !entry.is_object() {
            continue;
        }
        let command = entry.get("command").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let url = entry.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let transport = if url.is_empty() { "stdio".to_string() } else { "http".to_string() };
        out.push(ProjectMcp {
            managed: managed.contains(&name),
            name,
            transport,
            args: entry
                .get("args")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
                .unwrap_or_default(),
            command,
            url,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// 写项目的 `.mcp.json`：合并写入 `mcpServers[name]`，**保留文件里其它键**。
///
/// 只动 `mcpServers` 这一个键是硬要求：用户的 `.mcp.json` 里可能还有别的配置
/// （inputs、outputs、以及各工具自己的扩展键），整文件覆盖会把它们抹掉。
fn write_project_mcp(path: &Path, name: &str, entry: Option<Value>) -> Result<usize, String> {
    let mut file_val = read_project_mcp_file(path)?;
    let mut servers = project_servers_of(&file_val);
    match entry {
        Some(v) => {
            servers.insert(name.to_string(), v);
        }
        None => {
            servers.remove(name);
        }
    }
    let count = servers.len();
    file_val
        .as_object_mut()
        .unwrap()
        .insert("mcpServers".to_string(), Value::Object(servers));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建 {} 失败: {}", parent.display(), e))?;
    }
    let data = serde_json::to_string_pretty(&file_val).map_err(|e| e.to_string())?;
    crate::commands::config::atomic_write_file(path, data.as_bytes())
        .map_err(|e| format!("写入 {} 失败: {}", path.display(), e))?;
    Ok(count)
}

/// 把仓库里的 MCP 装给某个项目
#[tauri::command]
pub fn install_mcp_to_project(project_id: String, server_id: String) -> Result<usize, String> {
    let store = load_store();
    let server = store
        .servers
        .iter()
        .find(|s| s.id == server_id)
        .ok_or_else(|| format!("MCP 服务器不存在: {}", server_id))?;
    let entry = translate(server, "mcp");
    let path = project_mcp_path(&project_id)?;
    write_project_mcp(&path, &server.name, Some(entry))
}

/// 从某个项目移除一个 MCP。
///
/// 项目自带（手写、非受管）的也能移除，但前端要先说清这是用户自己写的东西。
#[tauri::command]
pub fn remove_mcp_from_project(project_id: String, name: String) -> Result<usize, String> {
    let path = project_mcp_path(&project_id)?;
    write_project_mcp(&path, &name, None)
}

// ─── 一次性迁移：把全局配置里的 MCP 收进仓库 ───

/// 列出各开发工具**全局**配置里现存的 MCP（只读，不改任何东西）。
///
/// 用途是一次性清理：这些条目对所有项目生效，是「上下文被污染」的根源。
/// 迁移后它们从全局文件里消失，改由各项目按需引入。
#[tauri::command]
pub fn list_global_mcp() -> Result<Vec<DiscoveredMcp>, String> {
    let store = load_store();
    let reg = registry();
    let mut out: Vec<DiscoveredMcp> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for tool_id in reg.mcp_tool_ids() {
        let Some((path, _format, mcp_key)) = reg.get_tool_mcp_config(&tool_id) else { continue };
        if !path.exists() {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else { continue };
        let Ok(val) = serde_json::from_str::<Value>(&text) else { continue };
        if !val.is_object() {
            continue;
        }
        let Some(servers) = val.get(&mcp_key).and_then(|m| m.as_object()) else { continue };
        for (name, entry) in servers {
            if !entry.is_object() {
                continue;
            }
            let key = format!("{}:{}", tool_id, name);
            if !seen.insert(key) {
                continue;
            }
            let mut d = reverse_translate(name, entry);
            // tool_id 必须补上：`reverse_translate` 只负责解析条目内容，不认识它来自哪个工具。
            // 漏了它迁移界面就不知道每条来自谁（也会让 React key退化成只有 name）
            d.tool_id = tool_id.clone();
            d.already_managed = store.servers.iter().any(|s| s.name == *name);
            out.push(d);
        }
    }
    Ok(out)
}

/// 一次性迁移：把选中的全局条目**收进仓库**，并从全局配置文件里删掉它们。
///
/// 只删用户勾选的；没勾的原地保留 —— 迁移不该替用户删东西。
/// 逐个工具处理并**忽略单个失败**：某个工具的配置文件不可写时，其余仍要迁完。
#[tauri::command]
pub fn migrate_global_mcp(names: Vec<String>) -> Result<MigrateResult, String> {
    let mut store = load_store();
    let mut imported: Vec<String> = Vec::new();
    for name in &names {
        if store.servers.iter().any(|s| &s.name == name) {
            continue;
        }
        // 定义从全局文件里现读，而不是让前端回传 —— 前端回传会漏字段，
        // 而 server 定义少一个 command/args 就是启动不了
        let Some(def) = read_definition_from_globals(name) else { continue };
        store.servers.push(def);
        imported.push(name.clone());
    }
    if !imported.is_empty() {
        save_store(&store)?;
    }
    let mut removed = 0usize;
    let mut failed: Vec<String> = Vec::new();
    let reg = registry();
    for tool_id in reg.mcp_tool_ids() {
        let Some((path, _format, mcp_key)) = reg.get_tool_mcp_config(&tool_id) else { continue };
        if !path.exists() {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else { continue };
        let Ok(mut val) = serde_json::from_str::<Value>(&text) else { continue };
        if !val.is_object() {
            continue;
        }
        let Some(Value::Object(mut servers)) = val.get(&mcp_key).cloned() else { continue };
        let before = servers.len();
        for n in &imported {
            servers.remove(n);
        }
        if servers.len() == before {
            continue;
        }
        val.as_object_mut()
            .unwrap()
            .insert(mcp_key.to_string(), Value::Object(servers));
        match serde_json::to_string_pretty(&val) {
            Ok(data) => match crate::commands::config::atomic_write_file(&path, data.as_bytes()) {
                Ok(()) => removed += 1,
                Err(e) => failed.push(format!("{}: {}", tool_id, e)),
            },
            Err(e) => failed.push(format!("{}: {}", tool_id, e)),
        }
    }
    Ok(MigrateResult {
        imported,
        files_updated: removed,
        failed,
    })
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MigrateResult {
    /// 已收进仓库的条目名
    pub imported: Vec<String>,
    /// 实际改动的全局配置文件数
    pub files_updated: usize,
    /// 单个文件失败时的原因（其余仍已迁完）
    pub failed: Vec<String>,
}

/// 从各工具全局配置里读回某个 server 的完整定义（迁移时用，避免前端回传丢字段）
fn read_definition_from_globals(name: &str) -> Option<McpServer> {
    let reg = registry();
    for tool_id in reg.mcp_tool_ids() {
        let (path, _format, mcp_key) = reg.get_tool_mcp_config(&tool_id)?;
        let text = fs::read_to_string(&path).ok()?;
        let val: Value = serde_json::from_str(&text).ok()?;
        let servers = val.get(&mcp_key)?.as_object()?;
        let entry = servers.get(name)?;
        let mut d = reverse_translate(name, entry);
        d.tool_id = tool_id.clone();
        d.already_managed = false;
        let server = McpServer {
            id: normalize_id(&d.name),
            name: d.name.clone(),
            transport: d.transport.clone(),
            command: d.command.clone(),
            args: d.args.clone(),
            env: d.env.clone(),
            cwd: d.cwd.clone(),
            url: d.url.clone(),
            headers: d.headers.clone(),
            // 迁进仓库即视为可用：全局配置里存在的条目本来就是你打算用的
            enabled: true,
            enabled_tools: Vec::new(),
            description: None,
            install_method: "migrated".to_string(),
        };
        return Some(server);
    }
    None
}

/// 将规范的 McpServer 转换为各工具所需的配置片段
fn translate(server: &McpServer, format: &str) -> Value {
    let mut m: Map<String, Value> = Map::new();
    let desc = server.description.clone().filter(|d| !d.trim().is_empty());

    match server.transport.as_str() {
        "stdio" => {
            if format == "opencode" {
                let mut cmd: Vec<String> = vec![server.command.clone()];
                cmd.extend(server.args.clone());
                m.insert("type".into(), Value::String("local".into()));
                m.insert("command".into(), serde_json::to_value(&cmd).unwrap_or(Value::Null));
                if !server.env.is_empty() {
                    m.insert("environment".into(), serde_json::to_value(&server.env).unwrap_or(Value::Null));
                }
                m.insert("enabled".into(), Value::Bool(true));
            } else {
                m.insert("command".into(), Value::String(server.command.clone()));
                if !server.args.is_empty() {
                    m.insert("args".into(), serde_json::to_value(&server.args).unwrap_or(Value::Null));
                }
                if !server.env.is_empty() {
                    m.insert("env".into(), serde_json::to_value(&server.env).unwrap_or(Value::Null));
                }
                if let Some(cwd) = &server.cwd {
                    if !cwd.is_empty() {
                        m.insert("cwd".into(), Value::String(cwd.clone()));
                    }
                }
                if let Some(d) = &desc {
                    m.insert("description".into(), Value::String(d.clone()));
                }
            }
        }
        "http" | "sse" => {
            let t = server.transport.clone();
            match format {
                // 标准 `.mcp.json` 形态：**项目级文件用这一档**。
                // 必须是 `url` 而不是 gemini/qwen 风格的 `httpUrl` —— CodeBuddy / Claude Code
                // 读项目文件时只认 url，写成 httpUrl 等于配了一条永远连不上的 server。
                "mcp" => {
                    m.insert("type".into(), Value::String(t));
                    m.insert("url".into(), Value::String(server.url.clone()));
                    if !server.headers.is_empty() {
                        m.insert("headers".into(), serde_json::to_value(&server.headers).unwrap_or(Value::Null));
                    }
                }
                "opencode" => {
                    m.insert("type".into(), Value::String("remote".into()));
                    m.insert("url".into(), Value::String(server.url.clone()));
                    if !server.headers.is_empty() {
                        m.insert("headers".into(), serde_json::to_value(&server.headers).unwrap_or(Value::Null));
                    }
                    m.insert("enabled".into(), Value::Bool(true));
                }
                "claude" => {
                    m.insert("type".into(), Value::String(t));
                    m.insert("url".into(), Value::String(server.url.clone()));
                    if !server.headers.is_empty() {
                        m.insert("headers".into(), serde_json::to_value(&server.headers).unwrap_or(Value::Null));
                    }
                }
                _ => {
                    // gemini / qwen 风格
                    if t == "http" {
                        m.insert("httpUrl".into(), Value::String(server.url.clone()));
                    } else {
                        m.insert("url".into(), Value::String(server.url.clone()));
                    }
                    if !server.headers.is_empty() {
                        m.insert("headers".into(), serde_json::to_value(&server.headers).unwrap_or(Value::Null));
                    }
                }
            }
        }
        _ => {}
    }
    Value::Object(m)
}
