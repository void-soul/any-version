//! projects responsibility.
use super::*;

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

pub(super) fn project_mcp_path(project_id: &str) -> Result<PathBuf, String> {
    let dir = crate::commands::ai::skills::project_dir_of(project_id)?;
    Ok(Path::new(&dir).join(".mcp.json"))
}

/// 读项目的 `.mcp.json`；不存在或不是合法对象时返回空对象。
///
/// **解析失败不静默当空**：直接返回错误。否则用户手写坏了一个逗号，UI 会显示
/// 「这个项目没装任何 MCP」，而实际文件里有 6 个 —— 看起来像我们的功能坏了。
pub(super) fn read_project_mcp_file(path: &Path) -> Result<Value, String> {
    if !path.is_file() {
        return Ok(Value::Object(Map::new()));
    }
    let text =
        fs::read_to_string(path).map_err(|e| format!("读取 {} 失败: {}", path.display(), e))?;
    if text.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    match serde_json::from_str::<Value>(&text) {
        Ok(v) if v.is_object() => Ok(v),
        Ok(_) => Err(format!("{} 不是合法的 JSON 对象", path.display())),
        Err(e) => Err(format!("解析 {} 失败: {}", path.display(), e)),
    }
}

pub(super) fn project_servers_of(file_val: &Value) -> Map<String, Value> {
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
    let managed: Vec<String> = load_store()
        .servers
        .iter()
        .map(|s| s.name.clone())
        .collect();
    let mut out: Vec<ProjectMcp> = Vec::new();
    for (name, entry) in project_servers_of(&file_val) {
        if !entry.is_object() {
            continue;
        }
        let command = entry
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let url = entry
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let transport = if url.is_empty() {
            "stdio".to_string()
        } else {
            "http".to_string()
        };
        out.push(ProjectMcp {
            managed: managed.contains(&name),
            name,
            transport,
            args: entry
                .get("args")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                })
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
pub(super) fn write_project_mcp(
    path: &Path,
    name: &str,
    entry: Option<Value>,
) -> Result<usize, String> {
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
