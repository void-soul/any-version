use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use serde_json::{Map, Value};
use crate::commands::ai_registry::registry;
use crate::commands::config::get_data_dir;
use super::skills::SkillToolInfo;

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
    save_store(&store)?;
    deploy_all()
}

#[tauri::command]
pub fn delete_mcp_server(id: String) -> Result<(), String> {
    let mut store = load_store();
    store.servers.retain(|s| s.id != id);
    save_store(&store)?;
    deploy_all()
}

#[tauri::command]
pub fn toggle_mcp_tool(id: String, tool_id: String, enabled: bool) -> Result<(), String> {
    let mut store = load_store();
    let Some(server) = store.servers.iter_mut().find(|s| s.id == id) else {
        return Err("MCP 服务器不存在".to_string());
    };
    if enabled {
        if !server.enabled_tools.contains(&tool_id) {
            server.enabled_tools.push(tool_id);
        }
    } else {
        server.enabled_tools.retain(|t| t != &tool_id);
    }
    save_store(&store)?;
    deploy_all()
}

/// 从各工具中心配置中发现已配置、但 vex 尚未托管的 MCP 服务器。
/// 类比技能管理的「问题检测 / 纳入管理」：先把散落在工具里的服务器找出来，再统一纳管。
#[tauri::command]
pub fn get_discovered_mcp() -> Result<Vec<DiscoveredMcp>, String> {
    let store = load_store();
    let reg = registry();
    let mut out: Vec<DiscoveredMcp> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for tool_id in reg.mcp_tool_ids() {
        let Some((path, _format, mcp_key)) = reg.get_tool_mcp_config(&tool_id) else { continue };
        if !path.exists() { continue; }
        let Ok(text) = fs::read_to_string(&path) else { continue };
        let Ok(val) = serde_json::from_str::<Value>(&text) else { continue };
        if !val.is_object() { continue; }
        let Some(servers) = val.get(&mcp_key).and_then(|m| m.as_object()) else { continue };
        for (name, entry) in servers {
            if !entry.is_object() { continue; }
            let raw = reverse_translate(name, entry);
            let already = store.servers.iter().any(|s| s.name == *name);
            let key = format!("{}:{}", tool_id, name);
            if seen.contains(&key) { continue; }
            seen.insert(key.clone());
            out.push(DiscoveredMcp {
                tool_id: tool_id.clone(),
                name: raw.name,
                transport: raw.transport,
                command: raw.command,
                args: raw.args,
                env: raw.env,
                cwd: raw.cwd,
                url: raw.url,
                headers: raw.headers,
                already_managed: already,
            });
        }
    }
    Ok(out)
}

/// 将一个在工具配置中发现的服务器纳入 vex 托管：
/// 写入托管仓库（默认启用、仅部署到发现它的工具），并触发重新部署。
#[tauri::command]
pub fn adopt_mcp_server(tool_id: String, name: String) -> Result<(), String> {
    let reg = registry();
    let (path, _format, mcp_key) = reg.get_tool_mcp_config(&tool_id).ok_or("找不到工具配置")?;
    let text = fs::read_to_string(&path).map_err(|e| format!("读取工具配置失败: {}", e))?;
    let val = serde_json::from_str::<Value>(&text).map_err(|e| format!("解析工具配置失败: {}", e))?;
    let entry = val
        .get(&mcp_key)
        .and_then(|m| m.get(&name))
        .cloned()
        .ok_or("未在工具配置中找到该服务器")?;
    let raw = reverse_translate(&name, &entry);
    let id = normalize_id(&name);
    let mut store = load_store();
    store.servers.retain(|s| s.id != id);
    store.servers.push(McpServer {
        id: id.clone(),
        name: name.clone(),
        transport: raw.transport,
        command: raw.command,
        args: raw.args,
        env: raw.env,
        cwd: raw.cwd,
        url: raw.url,
        headers: raw.headers,
        enabled: true,
        enabled_tools: vec![tool_id.clone()],
        description: None,
        install_method: "adopted".to_string(),
    });
    save_store(&store)?;
    deploy_all()
}

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

/// 可部署 MCP 的工具列表（由 mcp-config.json 驱动），复用技能工具信息结构
#[tauri::command]
pub fn get_mcp_tools() -> Result<Vec<SkillToolInfo>, String> {
    let reg = registry();
    let mut tools: Vec<SkillToolInfo> = Vec::new();
    for id in reg.mcp_tool_ids() {
        let label = reg.get_tool_config(&id)
            .map(|c| c.display_name.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| id.to_string());
        tools.push(SkillToolInfo { id: id.clone(), label });
    }
    tools.sort_by(|a, b| a.label.cmp(&b.label));
    Ok(tools)
}

// ─── 部署 ───

/// 将托管服务器合并写入各工具的中心配置文件。
/// 仅修改 mcpServers / mcp 键，保留文件其它内容；对未启用该服务器的工具移除托管条目。
fn deploy_all() -> Result<(), String> {
    let store = load_store();
    let reg = registry();
    for tool_id in reg.mcp_tool_ids() {
        let (path, format, mcp_key) = match reg.get_tool_mcp_config(&tool_id) {
            Some(x) => x,
            None => continue,
        };
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let mut file_val: Value = if path.exists() {
            fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or(Value::Object(Map::new()))
        } else {
            Value::Object(Map::new())
        };
        if !file_val.is_object() {
            file_val = Value::Object(Map::new());
        }
        let mut servers_map: Map<String, Value> = match file_val.get(&mcp_key).cloned() {
            Some(Value::Object(m)) => m,
            _ => Map::new(),
        };
        for s in &store.servers {
            if !s.enabled {
                continue;
            }
            if s.enabled_tools.contains(&tool_id) {
                servers_map.insert(s.name.clone(), translate(s, &format));
            } else {
                servers_map.remove(&s.name);
            }
        }
        let count = servers_map.len();
        file_val.as_object_mut().unwrap().insert(mcp_key.to_string(), Value::Object(servers_map));
        let data = serde_json::to_string_pretty(&file_val).map_err(|e| e.to_string())?;
        crate::commands::config::atomic_write_file(&path, data.as_bytes())
            .map_err(|e| format!("写入 {} 失败: {}", path.display(), e))?;
        eprintln!("[mcp] 已部署 {} 个服务器到 {} ({})", count, tool_id, format);
    }
    Ok(())
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
