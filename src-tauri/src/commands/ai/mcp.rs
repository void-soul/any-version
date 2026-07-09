use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use serde_json::{Map, Value};
use crate::commands::ai_registry::registry;
use crate::commands::config::get_base_dir;
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

#[derive(Serialize, Deserialize, Default)]
struct McpStore {
    #[serde(default)]
    servers: Vec<McpServer>,
}

fn mcp_path() -> PathBuf {
    get_base_dir().join("mcp.json")
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
    let path = mcp_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let data = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
    fs::write(&path, data).map_err(|e| e.to_string())
}

fn normalize_id(name: &str) -> String {
    name.to_lowercase().replace(' ', "-").replace('_', "-")
}

// ─── 命令 ───

#[tauri::command]
pub fn get_mcp_servers() -> Result<Vec<McpServer>, String> {
    Ok(load_store().servers)
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
        let (path, format) = match reg.get_tool_mcp_config(&tool_id) {
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
        let mcp_key = if format == "opencode" { "mcp" } else { "mcpServers" };
        let mut servers_map: Map<String, Value> = match file_val.get(mcp_key).cloned() {
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
        fs::write(&path, data).map_err(|e| format!("写入 {} 失败: {}", path.display(), e))?;
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
