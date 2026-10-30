//! global responsibility.
use super::*;

// ─── 一次性迁移：把全局配置里的 MCP 收进仓库 ───

/// 列出各开发工具**全局**配置里现存的 MCP（只读，不改任何东西）。
///
/// 用途是一次性清理：这些条目对所有项目生效，是「上下文被污染」的根源。
/// 迁移后它们从全局文件里消失，改由各项目按需引入。
/// 从单个工具的全局配置里解析 MCP 条目。**纯函数**：不碰磁盘、不查注册表。
///
/// 与 `list_global_mcp` 拆开是为了能确定性地测「配了 N 个就该拿到 N 个」。
/// 直接测 `list_global_mcp` 只能读用户 home 目录下的真实文件，有没有条目全看机器状态——
/// 于是想在测试里守这个不变量，就只能断言「结果非空」，而这条在没配过全局 MCP 的
/// 机器上必然失败（既不能证明代码对，也不能证明代码错）。
pub(super) fn parse_tool_mcp_entries(
    tool_id: &str,
    val: &Value,
    mcp_key: &str,
) -> Vec<DiscoveredMcp> {
    if !val.is_object() {
        return Vec::new();
    }
    let Some(servers) = val.get(mcp_key).and_then(|m| m.as_object()) else {
        return Vec::new();
    };
    let mut out: Vec<DiscoveredMcp> = Vec::new();
    for (name, entry) in servers {
        if !entry.is_object() {
            continue;
        }
        let mut d = reverse_translate(name, entry);
        // tool_id 必须补上：`reverse_translate` 只负责解析条目内容，不认识它来自哪个工具。
        // 漏了它迁移界面就不知道每条来自谁（也会让 React key 退化成只有 name）
        d.tool_id = tool_id.to_string();
        out.push(d);
    }
    out
}

#[tauri::command]
pub fn list_global_mcp() -> Result<Vec<DiscoveredMcp>, String> {
    let store = load_store();
    let reg = registry();
    let mut out: Vec<DiscoveredMcp> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for tool_id in reg.mcp_tool_ids() {
        let Some((path, _format, mcp_key)) = reg.get_tool_mcp_config(&tool_id) else {
            continue;
        };
        if !path.exists() {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(val) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        for mut d in parse_tool_mcp_entries(&tool_id, &val, &mcp_key) {
            let key = format!("{}:{}", tool_id, d.name);
            if !seen.insert(key) {
                continue;
            }
            d.already_managed = store.servers.iter().any(|s| s.name == d.name);
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
        let Some(def) = read_definition_from_globals(name) else {
            continue;
        };
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
        let Some((path, _format, mcp_key)) = reg.get_tool_mcp_config(&tool_id) else {
            continue;
        };
        if !path.exists() {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(mut val) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        if !val.is_object() {
            continue;
        }
        let Some(Value::Object(mut servers)) = val.get(&mcp_key).cloned() else {
            continue;
        };
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
pub(super) fn read_definition_from_globals(name: &str) -> Option<McpServer> {
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
            managed: None,
        };
        return Some(server);
    }
    None
}

/// 将规范的 McpServer 转换为各工具所需的配置片段
pub(super) fn translate(server: &McpServer, format: &str) -> Value {
    let mut m: Map<String, Value> = Map::new();
    let desc = server.description.clone().filter(|d| !d.trim().is_empty());

    match server.transport.as_str() {
        "stdio" => {
            if format == "opencode" {
                let mut cmd: Vec<String> = vec![server.command.clone()];
                cmd.extend(server.args.clone());
                m.insert("type".into(), Value::String("local".into()));
                m.insert(
                    "command".into(),
                    serde_json::to_value(&cmd).unwrap_or(Value::Null),
                );
                if !server.env.is_empty() {
                    m.insert(
                        "environment".into(),
                        serde_json::to_value(&server.env).unwrap_or(Value::Null),
                    );
                }
                m.insert("enabled".into(), Value::Bool(true));
            } else {
                m.insert("command".into(), Value::String(server.command.clone()));
                if !server.args.is_empty() {
                    m.insert(
                        "args".into(),
                        serde_json::to_value(&server.args).unwrap_or(Value::Null),
                    );
                }
                if !server.env.is_empty() {
                    m.insert(
                        "env".into(),
                        serde_json::to_value(&server.env).unwrap_or(Value::Null),
                    );
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
                        m.insert(
                            "headers".into(),
                            serde_json::to_value(&server.headers).unwrap_or(Value::Null),
                        );
                    }
                }
                "opencode" => {
                    m.insert("type".into(), Value::String("remote".into()));
                    m.insert("url".into(), Value::String(server.url.clone()));
                    if !server.headers.is_empty() {
                        m.insert(
                            "headers".into(),
                            serde_json::to_value(&server.headers).unwrap_or(Value::Null),
                        );
                    }
                    m.insert("enabled".into(), Value::Bool(true));
                }
                "claude" => {
                    m.insert("type".into(), Value::String(t));
                    m.insert("url".into(), Value::String(server.url.clone()));
                    if !server.headers.is_empty() {
                        m.insert(
                            "headers".into(),
                            serde_json::to_value(&server.headers).unwrap_or(Value::Null),
                        );
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
                        m.insert(
                            "headers".into(),
                            serde_json::to_value(&server.headers).unwrap_or(Value::Null),
                        );
                    }
                }
            }
        }
        _ => {}
    }
    Value::Object(m)
}
