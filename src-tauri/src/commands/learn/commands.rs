//! Tauri 命令：需求模块的项目/模块/图谱 CRUD + AI 生成 + 导出。
//! 保留旧命令兼容，新命令使用 project → module → graph 三级结构。

use crate::commands::ai;
use std::time::Duration;
use super::models::*;

// ─── 工具函数 ───

fn resolve_provider_model(provider_id: &Option<String>, model_id: &Option<String>) -> Result<(ai::models::AiProvider, String), String> {
    let cfg = ai::config::load_ai_config();
    let provider = if let Some(pid) = provider_id {
        cfg.providers.iter().find(|p| &p.id == pid).cloned()
            .ok_or_else(|| format!("未找到供应商: {}", pid))?
    } else {
        cfg.providers.iter().find(|p| !p.api_key.is_empty() && !p.openai_url.is_empty()).cloned()
            .ok_or_else(|| "没有配置了 OpenAI 端点和 API Key 的供应商".to_string())?
    };
    if provider.openai_url.is_empty() { return Err(format!("供应商「{}」未配置 OpenAI 兼容端点", provider.name)); }
    if provider.api_key.is_empty() { return Err(format!("供应商「{}」未配置 API Key", provider.name)); }
    let model = model_id.clone()
        .or_else(|| provider.active_model_id.clone())
        .or_else(|| provider.models.first().map(|m| m.id.clone()))
        .ok_or_else(|| format!("供应商「{}」未配置任何模型", provider.name))?;
    Ok((provider, model))
}

fn completion_url(base: &str) -> String {
    let t = base.trim_end_matches('/');
    if t.ends_with("/v1") { format!("{}/chat/completions", t) } else { format!("{}/v1/chat/completions", t) }
}

fn parse_ai_json(text: &str) -> Result<serde_json::Value, String> {
    let t = text.trim();
    let start = t.find('{').ok_or("响应中没有 JSON 对象")?;
    let end = t.rfind('}').ok_or("响应中没有 JSON 对象")?;
    serde_json::from_str(&t[start..=end]).map_err(|e| format!("JSON 解析失败: {}", e))
}

fn json_to_nodes(json: &serde_json::Value) -> Vec<LearnNode> {
    let arr = match json.get("nodes").and_then(|v| v.as_array()) { Some(a) => a, None => return vec![] };
    arr.iter().enumerate().map(|(i, v)| LearnNode {
        id: v.get("id").and_then(|id| id.as_str()).map(|s| s.to_string()).unwrap_or_else(|| format!("n{}", i + 1)),
        name: v.get("name").and_then(|n| n.as_str()).map(|s| s.to_string()).unwrap_or_else(|| "未知".to_string()),
        parent_id: v.get("parent_id").and_then(|p| p.as_str()).filter(|s| !s.is_empty() && *s != "null").map(|s| s.to_string()),
        description: v.get("description").and_then(|d| d.as_str()).unwrap_or("").to_string(),
        detail: v.get("detail").and_then(|d| d.as_str()).unwrap_or("").to_string(),
        kind: v.get("kind").and_then(|k| k.as_str()).unwrap_or("other").to_string(),
        position_x: 0.0,
        position_y: 0.0,
    }).collect()
}

// ─── 项目 ───

#[tauri::command]
pub fn req_list_projects() -> Result<Vec<ReqProject>, String> {
    super::db::list_projects()
}

#[tauri::command]
pub fn req_create_project(input: CreateReqProjectInput) -> Result<ReqProject, String> {
    super::db::create_project(&input.name, input.description.as_deref().unwrap_or(""))
}

#[tauri::command]
pub fn req_update_project(input: UpdateReqProjectInput) -> Result<(), String> {
    super::db::update_project(&input.id, input.name.as_deref(), input.description.as_deref())
}

#[tauri::command]
pub fn req_delete_project(id: String) -> Result<(), String> {
    super::db::delete_project(&id)
}

// ─── 模块 ───

#[tauri::command]
pub fn req_list_modules(project_id: String) -> Result<Vec<ReqModule>, String> {
    super::db::list_modules(&project_id)
}

#[tauri::command]
pub fn req_create_module(input: CreateReqModuleInput) -> Result<ReqModule, String> {
    super::db::create_module(&input.project_id, &input.name, input.description.as_deref().unwrap_or(""))
}

#[tauri::command]
pub fn req_update_module(input: UpdateReqModuleInput) -> Result<(), String> {
    super::db::update_module(&input.id, input.name.as_deref(), input.description.as_deref())
}

#[tauri::command]
pub fn req_delete_module(id: String) -> Result<(), String> {
    super::db::delete_module(&id)
}

// ─── 图谱 ───

#[tauri::command]
pub fn req_get_graph(module_id: String) -> Result<Option<LearnGraph>, String> {
    super::db::load_graph(&module_id)
}

#[tauri::command]
pub fn req_create_empty_graph(module_id: String) -> Result<LearnGraph, String> {
    super::db::create_empty_graph(&module_id, "思维导图")
}

#[tauri::command]
pub fn req_update_graph(module_id: String, graph_json: String) -> Result<(), String> {
    let mut graph: LearnGraph = serde_json::from_str(&graph_json).map_err(|e| format!("JSON 解析失败: {}", e))?;
    graph.module_id = module_id;
    super::db::save_graph(&graph)
}

#[tauri::command]
pub fn req_update_positions(module_id: String, positions: Vec<NodePosition>) -> Result<(), String> {
    let mut graph = super::db::load_graph(&module_id)?.ok_or("未找到图谱")?;
    let pm: std::collections::HashMap<&str, &NodePosition> = positions.iter().map(|p| (p.node_id.as_str(), p)).collect();
    for n in &mut graph.nodes {
        if let Some(pos) = pm.get(n.id.as_str()) { n.position_x = pos.x; n.position_y = pos.y; }
    }
    super::db::save_graph(&graph)
}

#[tauri::command]
pub fn req_export_markdown(module_id: String) -> Result<String, String> {
    let graph = super::db::load_graph(&module_id)?.ok_or("未找到该模块的图谱")?;
    render_markdown(&graph)
}

// ─── 从项目生成 ───

fn project_system_prompt() -> String {
    r#"你是一位资深软件架构分析师。下面是一个项目的目录结构树和关键文件内容。
请分析该项目，生成一个「父-子」树形结构 JSON，用于帮助新开发者快速理解项目。

要求：
1. 输出一个合法 JSON 对象，包含：
   - "summary": 一段 markdown 文字（项目的整体介绍，300 字以内）
   - "nodes": 数组，每个节点包含：
     - "id": 唯一字符串（如 "n1","n2",...）
     - "name": 节点显示名称（简短，<20 字）
     - "parent_id": 父节点 id 或 null（根节点为 null）
     - "description": 一句话概述（<80 字）
     - "detail": 详细说明（markdown，分析建议、阅读顺序、注意，200 字以内）
     - "kind": 类型（module/lib/component/class/function/service/route/config/file/entry/other 之一）
2. 根节点 (parent_id=null) 应该是项目名。
3. 节点总数控制在 15-50 个，按目录/模块/组件划分，不要每个文件一个节点。
4. 只输出 JSON，不要代码块标记，不要任何额外文字。"#.to_string()
}

#[tauri::command]
pub async fn req_generate_from_project(input: GenerateGraphFromProjectInput) -> Result<LearnGraph, String> {
    let pp = input.project_path.trim().to_string();
    if pp.is_empty() { return Err("项目路径不能为空".to_string()); }

    let project_name = super::scan::project_name(&pp);
    let context = super::scan::scan_project(&pp)?;
    let (provider, model) = resolve_provider_model(&input.provider_id, &input.model_id)?;
    let url = completion_url(&provider.openai_url);
    let body = serde_json::json!({
        "model": model, "stream": false, "temperature": 0.3,
        "messages": [
            { "role": "system", "content": project_system_prompt() },
            { "role": "user", "content": context }
        ]
    });

    eprintln!("[req] 从项目生成图谱 (模块: {}, 项目: {})", input.module_id, project_name);

    let client = reqwest::Client::new();
    let resp = client.post(&url).header("Authorization", format!("Bearer {}", provider.api_key))
        .header("Content-Type", "application/json").json(&body).timeout(Duration::from_secs(180)).send().await
        .map_err(|e| format!("请求失败: {}", e))?;
    let status = resp.status();
    let value: serde_json::Value = resp.json().await.map_err(|e| format!("解析响应失败: {}", e))?;
    if !status.is_success() {
        let msg = value.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str()).unwrap_or("未知错误");
        return Err(format!("AI 接口错误 ({}): {}", status.as_u16(), msg));
    }

    let content = value.get("choices").and_then(|c| c.get(0)).and_then(|ch| ch.get("message")).and_then(|m| m.get("content"))
        .and_then(|c| c.as_str()).unwrap_or("").trim().to_string();
    if content.is_empty() { return Err("AI 返回空结果".to_string()); }

    let parsed = parse_ai_json(&content).map_err(|e| format!("JSON 解析失败: {}", e))?;
    let summary = parsed.get("summary").and_then(|s| s.as_str()).unwrap_or("").to_string();
    let mut nodes = json_to_nodes(&parsed);
    let has_root = nodes.iter().any(|n| n.parent_id.is_none());
    if !has_root {
        nodes.insert(0, LearnNode { id: "root".to_string(), name: project_name.clone(), parent_id: None,
            description: "项目根节点".to_string(), detail: summary.clone(), kind: "root".to_string(),
            position_x: 0.0, position_y: 0.0 });
    }

    let ts = super::db::now_ts();
    let graph = LearnGraph { module_id: input.module_id, project_name, summary, generated_at: ts,
        source_type: "ai_project".to_string(), source_desc: pp, nodes };
    super::db::save_graph(&graph)?;
    Ok(graph)
}

// ─── 从文本生成 ───

fn text_requirements_system_prompt() -> String {
    r#"你是一位资深产品经理兼软件架构师。用户会提供一段文字（可能来自会议纪要、需求文档、聊天记录、头脑风暴等）。
请从中提取并组织为结构化的需求树形图。

要求：
1. 输出一个合法 JSON 对象，包含：
   - "summary": 一段 markdown 文字（整体概述，200 字以内）
   - "nodes": 数组，每个节点包含：
     - "id": 唯一字符串（如 "n1","n2",...）
     - "name": 节点显示名称（简短，<20 字）
     - "parent_id": 父节点 id 或 null（根节点为 null）
     - "description": 一句话概述（<80 字）
     - "detail": 详细说明（markdown，含需求细节、验收标准等，200 字以内）
     - "kind": 类型（root/module/requirement/task/constraint/risk/other）
2. 根节点 (parent_id=null) 应该是项目名或主题标题。
3. 节点总数控制在 8-30 个。把用户提到的所有明确需求、隐含需求、约束条件都组织进去。
4. 只输出 JSON，不要代码块标记，不要任何额外文字。"#.to_string()
}

#[tauri::command]
pub async fn req_generate_from_text(input: GenerateGraphFromTextInput) -> Result<LearnGraph, String> {
    let text = input.text.trim().to_string();
    if text.is_empty() { return Err("文本内容不能为空".to_string()); }
    let title = if input.title.trim().is_empty() { "需求分析" } else { input.title.trim() };

    let (provider, model) = resolve_provider_model(&input.provider_id, &input.model_id)?;
    let url = completion_url(&provider.openai_url);
    let body = serde_json::json!({
        "model": model, "stream": false, "temperature": 0.3,
        "messages": [
            { "role": "system", "content": text_requirements_system_prompt() },
            { "role": "user", "content": format!("请分析以下文字，提取结构化需求：\n\n{}", text) }
        ]
    });

    eprintln!("[req] 从文本生成图谱 (模块: {}, 长度: {})", input.module_id, text.len());

    let client = reqwest::Client::new();
    let resp = client.post(&url).header("Authorization", format!("Bearer {}", provider.api_key))
        .header("Content-Type", "application/json").json(&body).timeout(Duration::from_secs(180)).send().await
        .map_err(|e| format!("请求失败: {}", e))?;
    let status = resp.status();
    let value: serde_json::Value = resp.json().await.map_err(|e| format!("解析响应失败: {}", e))?;
    if !status.is_success() {
        let msg = value.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str()).unwrap_or("未知错误");
        return Err(format!("AI 接口错误 ({}): {}", status.as_u16(), msg));
    }

    let content = value.get("choices").and_then(|c| c.get(0)).and_then(|ch| ch.get("message")).and_then(|m| m.get("content"))
        .and_then(|c| c.as_str()).unwrap_or("").trim().to_string();
    if content.is_empty() { return Err("AI 返回空结果".to_string()); }

    let parsed = parse_ai_json(&content).map_err(|e| format!("JSON 解析失败: {}", e))?;
    let summary = parsed.get("summary").and_then(|s| s.as_str()).unwrap_or("").to_string();
    let mut nodes = json_to_nodes(&parsed);
    let has_root = nodes.iter().any(|n| n.parent_id.is_none());
    if !has_root {
        nodes.insert(0, LearnNode { id: "root".to_string(), name: title.to_string(), parent_id: None,
            description: "需求根节点".to_string(), detail: summary.clone(), kind: "root".to_string(),
            position_x: 0.0, position_y: 0.0 });
    }

    let ts = super::db::now_ts();
    let graph = LearnGraph { module_id: input.module_id, project_name: title.to_string(), summary, generated_at: ts,
        source_type: "ai_text".to_string(), source_desc: format!("文本提取 ({} 字符)", text.len()), nodes };
    super::db::save_graph(&graph)?;
    Ok(graph)
}

// ─── 子树重新生成 ───

fn subtree_system_prompt(node_name: &str, node_kind: &str, node_desc: &str) -> String {
    format!(r#"你是一位资深软件架构分析师。需要深入分析一个模块的内部结构。

目标节点："{node_name}" 类型：{node_kind} 描述：{node_desc}

请分析该模块的内部结构，生成「父-子」树形 JSON。
要求：
1. 输出 {{"nodes": [...]}}，每个子节点含 id/name/parent_id/description/detail/kind。
2. 第一个节点 (parent_id=null) 必须是当前节点本身（名称保持 "{node_name}"）。
3. 子节点总数 3-12 个。
4. 只输出 JSON，不要代码块标记。"#)
}

#[tauri::command]
pub async fn req_regenerate_node(module_id: String, node_id: String, provider_id: Option<String>, model_id: Option<String>) -> Result<LearnGraph, String> {
    let mut graph = super::db::load_graph(&module_id)?.ok_or("未找到该模块的图谱")?;
    let node_idx = graph.nodes.iter().position(|n| n.id == node_id).ok_or_else(|| format!("未找到节点: {}", node_id))?;
    let node_name = graph.nodes[node_idx].clone().name;
    let node_kind = graph.nodes[node_idx].clone().kind;
    let node_desc = graph.nodes[node_idx].clone().description;

    let (provider, model) = resolve_provider_model(&provider_id, &model_id)?;
    let url = completion_url(&provider.openai_url);
    let body = serde_json::json!({
        "model": model, "stream": false, "temperature": 0.3,
        "messages": [
            { "role": "system", "content": subtree_system_prompt(&node_name, &node_kind, &node_desc) },
            { "role": "user", "content": format!("请只分析「{node_name}」这个模块的内部子结构。") }
        ]
    });

    eprintln!("[req] 重新分析子树「{}」(模块: {})", node_name, module_id);

    let client = reqwest::Client::new();
    let resp = client.post(&url).header("Authorization", format!("Bearer {}", provider.api_key))
        .header("Content-Type", "application/json").json(&body).timeout(Duration::from_secs(180)).send().await
        .map_err(|e| format!("请求失败: {}", e))?;
    let status = resp.status();
    let value: serde_json::Value = resp.json().await.map_err(|e| format!("解析响应失败: {}", e))?;
    if !status.is_success() {
        let msg = value.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str()).unwrap_or("未知错误");
        return Err(format!("AI 接口错误 ({}): {}", status.as_u16(), msg));
    }

    let content = value.get("choices").and_then(|c| c.get(0)).and_then(|ch| ch.get("message")).and_then(|m| m.get("content"))
        .and_then(|c| c.as_str()).unwrap_or("").trim().to_string();
    if content.is_empty() { return Err("AI 返回空结果".to_string()); }

    let parsed = parse_ai_json(&content).map_err(|e| format!("JSON 解析失败: {}", e))?;
    let new_children = json_to_nodes(&parsed);
    if new_children.is_empty() { return Err("AI 未生成任何子节点".to_string()); }

    // 更新目标节点描述
    let root_new = new_children.iter().find(|n| n.parent_id.is_none());
    if let Some(rn) = root_new {
        if !rn.description.is_empty() { graph.nodes[node_idx].description = rn.description.clone(); }
        if !rn.detail.is_empty() { graph.nodes[node_idx].detail = rn.detail.clone(); }
    }

    let root_ai_id = new_children.iter().find(|n| n.parent_id.is_none()).map(|n| n.id.clone()).unwrap_or_else(|| new_children[0].id.clone());
    // 删除旧后代
    let target_id = graph.nodes[node_idx].id.clone();
    let mut desc: Vec<String> = vec![];
    fn collect_desc(nodes: &[LearnNode], pid: &str, out: &mut Vec<String>) {
        for n in nodes { if n.parent_id.as_deref() == Some(pid) { out.push(n.id.clone()); collect_desc(nodes, &n.id, out); } }
    }
    collect_desc(&graph.nodes, &target_id, &mut desc);
    let desc_set: std::collections::HashSet<String> = desc.into_iter().collect();
    graph.nodes.retain(|n| !desc_set.contains(&n.id));

    // 插入新子节点
    let existing: std::collections::HashSet<String> = graph.nodes.iter().map(|n| n.id.clone()).collect();
    let mut counter = 0u64;
    for child in &new_children {
        if child.id == root_ai_id || child.parent_id.is_none() { continue; }
        let mut new_node = child.clone();
        let mut new_id;
        loop { new_id = format!("n{}", counter); counter += 1; if !existing.contains(&new_id) { break; } }
        new_node.id = new_id;
        if new_node.parent_id.as_deref() == Some(&root_ai_id) { new_node.parent_id = Some(target_id.clone()); }
        graph.nodes.push(new_node);
    }

    graph.generated_at = super::db::now_ts();
    super::db::save_graph(&graph)?;
    Ok(graph)
}

// ─── Markdown 导出 ───

fn render_markdown(graph: &LearnGraph) -> Result<String, String> {
    use std::collections::HashMap;
    let mut children: HashMap<Option<&str>, Vec<&LearnNode>> = HashMap::new();
    for n in &graph.nodes { children.entry(n.parent_id.as_deref()).or_default().push(n); }

    let kind_labels: HashMap<&str, &str> = [
        ("root", "根节点"), ("module", "模块"), ("lib", "库"), ("component", "组件"),
        ("class", "类"), ("function", "函数"), ("service", "服务"), ("route", "路由"),
        ("config", "配置"), ("file", "文件"), ("entry", "入口"), ("other", "其他"),
        ("requirement", "需求"), ("task", "任务"), ("constraint", "约束"), ("risk", "风险"),
    ].into_iter().collect();

    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", graph.project_name));
    out.push_str(&format!("> {}\n\n", graph.generated_at));
    if !graph.summary.is_empty() { out.push_str(&format!("## 概述\n\n{}\n\n---\n\n", graph.summary)); }

    out.push_str("## 结构目录\n\n");
    const MAX_DEPTH: usize = 32;
    fn render_toc(out: &mut String, pid: Option<&str>, ch: &HashMap<Option<&str>, Vec<&LearnNode>>, depth: usize) {
        if depth > MAX_DEPTH { return; }
        if let Some(list) = ch.get(&pid) {
            for n in list {
                let pfx = "  ".repeat(depth);
                let anchor = n.name.to_lowercase().chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' }).collect::<String>();
                out.push_str(&format!("{}- [{}](#{})\n", pfx, n.name, anchor));
                if !n.description.is_empty() { out.push_str(&format!("{}  *{}*\n", pfx, n.description)); }
                render_toc(out, Some(&n.id), ch, depth + 1);
            }
        }
    }
    render_toc(&mut out, None, &children, 0);
    out.push_str("\n---\n\n## 节点详情\n\n");

    fn render_nodes(out: &mut String, pid: Option<&str>, ch: &HashMap<Option<&str>, Vec<&LearnNode>>, depth: usize, kl: &HashMap<&str, &str>) {
        if depth > MAX_DEPTH { return; }
        if let Some(list) = ch.get(&pid) {
            let lv = (depth + 2).min(6);
            let h = "#".repeat(lv);
            for n in list {
                let k = kl.get(n.kind.as_str()).copied().unwrap_or(&n.kind);
                out.push_str(&format!("{} {} `{}`\n\n", h, n.name, k));
                if !n.description.is_empty() { out.push_str(&format!("> {}\n\n", n.description)); }
                if !n.detail.is_empty() { out.push_str(&n.detail); if !n.detail.ends_with('\n') { out.push('\n'); } out.push('\n'); }
                if n.description.is_empty() && n.detail.is_empty() { out.push_str("*暂无详细说明*\n\n"); }
                render_nodes(out, Some(&n.id), ch, depth + 1, kl);
            }
        }
    }
    render_nodes(&mut out, None, &children, 0, &kind_labels);
    out.push_str(&format!("\n---\n\n*由 AnyVersion 需求模块生成于 {}*\n", graph.generated_at));
    Ok(out)
}

// ─── 保留旧命令（兼容） ───

#[tauri::command]
pub fn learn_list() -> Vec<LearnMeta> { vec![] }

#[tauri::command]
pub fn learn_load(_project_path: String) -> Option<LearnGraph> { None }

#[tauri::command]
pub fn learn_delete(_project_path: String) -> Result<(), String> { Ok(()) }

#[tauri::command]
pub fn learn_update_positions(_project_path: String, _positions: Vec<NodePosition>) -> Result<(), String> { Ok(()) }

#[tauri::command]
pub fn learn_export_markdown(module_id: String) -> Result<String, String> {
    req_export_markdown(module_id)
}

#[tauri::command]
pub async fn learn_generate(_input: GenerateLearnInput) -> Result<LearnGraph, String> {
    Err("请使用新的需求模块：先创建项目 → 创建模块 → 从项目 AI 生成".to_string())
}

#[tauri::command]
pub async fn learn_generate_from_text(_input: GenerateFromTextInput) -> Result<LearnGraph, String> {
    Err("请使用新的需求模块：先创建项目 → 创建模块 → 从文本 AI 生成".to_string())
}

#[tauri::command]
pub async fn learn_regenerate_node(_input: RegenerateNodeInput) -> Result<LearnGraph, String> {
    Err("请使用 req_regenerate_node 命令".to_string())
}