//! Tauri 命令：思维导图 CRUD + AI 生成 + 导出。

use crate::commands::ai;
use std::time::Duration;
use super::models::*;

// ─── 工具 ───

fn resolve_provider_model(pid: &Option<String>, mid: &Option<String>) -> Result<(ai::models::AiProvider, String), String> {
    let cfg = ai::config::load_ai_config();
    let p = if let Some(id) = pid { cfg.providers.iter().find(|x| &x.id == id).cloned().ok_or_else(|| format!("未找到供应商: {}", id))? }
    else { cfg.providers.iter().find(|x| !x.api_key.is_empty() && !x.openai_url.is_empty()).cloned().ok_or("无可用供应商")? };
    if p.openai_url.is_empty() { return Err(format!("供应商 '{}' 未配置端点", p.name)); }
    if p.api_key.is_empty() { return Err(format!("供应商 '{}' 未配置 Key", p.name)); }
    let m = mid.clone().or_else(|| p.active_model_id.clone()).or_else(|| p.models.first().map(|m| m.id.clone())).ok_or("无可用模型")?;
    Ok((p, m))
}

fn completion_url(base: &str) -> String {
    let t = base.trim_end_matches('/');
    if t.ends_with("/v1") { format!("{}/chat/completions", t) } else { format!("{}/v1/chat/completions", t) }
}

fn parse_json(text: &str) -> Result<serde_json::Value, String> {
    let t = text.trim();
    let s = t.find('{').ok_or("无 JSON")?;
    let e = t.rfind('}').ok_or("无 JSON")?;
    serde_json::from_str(&t[s..=e]).map_err(|e| format!("JSON: {}", e))
}

fn json_to_mindmap_nodes(json: &serde_json::Value, document_id: &str, id_prefix: &str) -> Vec<MindmapNode> {
    let arr = match json.get("nodes").and_then(|v| v.as_array()) { Some(a) => a, None => return vec![] };
    let ts = super::db::now_ts();
    let colors = ["#22d3ee","#34d399","#fbbf24","#60a5fa","#fb7185","#a78bfa","#f97316","#f59e0b","#f8fafc","#94a3b8"];
    let ids: Vec<String> = arr.iter().enumerate().map(|(i, v)| {
        let raw = v.get("id").and_then(|x| x.as_str()).filter(|s| !s.trim().is_empty()).unwrap_or("");
        if raw.is_empty() { format!("{}n{}", id_prefix, i + 1) } else { format!("{}{}", id_prefix, raw) }
    }).collect();
    let raw_to_id: std::collections::HashMap<&str, String> = arr.iter().enumerate().filter_map(|(i, v)| {
        v.get("id").and_then(|x| x.as_str()).filter(|s| !s.trim().is_empty()).map(|raw| (raw, ids[i].clone()))
    }).collect();
    arr.iter().enumerate().map(|(i, v)| {
        let c = colors[i % colors.len()];
        let parent_raw = v.get("parent_id").or_else(|| v.get("parentId")).and_then(|x| x.as_str()).filter(|s| !s.is_empty() && *s != "null");
        let parent_id = parent_raw.and_then(|raw| raw_to_id.get(raw).cloned());
        let is_root = parent_id.is_none();
        MindmapNode {
            id: ids[i].clone(),
            document_id: document_id.to_string(),
            // 只引用同一批导入节点，未知父级自动成为新的根节点，避免挂到旧树或丢失。
            parent_id,
            name: v.get("name").and_then(|x| x.as_str()).unwrap_or("未命名").to_string(),
            description: v.get("description").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            detail: v.get("detail").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            kind: if is_root { "root".to_string() } else { v.get("kind").and_then(|x| x.as_str()).unwrap_or("other").to_string() },
            color: v.get("color").and_then(|x| x.as_str()).unwrap_or(c).to_string(),
            progress: v.get("progress").and_then(|x| x.as_i64()).unwrap_or(0).clamp(0, 100) as i32,
            position_x: 0.0, position_y: 0.0,
            created_at: ts.clone(), updated_at: ts.clone(),
        }
    }).collect()
}

fn ensure_import_root(nodes: &mut Vec<MindmapNode>, document_id: &str, id_prefix: &str, name: &str, summary: &str) {
    if nodes.iter().any(|n| n.parent_id.is_none()) { return; }
    let ts = super::db::now_ts();
    nodes.insert(0, MindmapNode {
        id: format!("{}root", id_prefix), document_id: document_id.to_string(), parent_id: None,
        name: name.to_string(), description: "AI 导入根节点".to_string(), detail: summary.to_string(),
        kind: "root".to_string(), color: "#f8fafc".to_string(), progress: 0,
        position_x: 0.0, position_y: 0.0, created_at: ts.clone(), updated_at: ts,
    });
}

// ─── 文档 ───

#[tauri::command]
pub fn mm_list_documents(folder_id: Option<String>) -> Result<Vec<MindmapDocument>, String> {
    super::db::list_documents(folder_id.as_deref())
}

#[tauri::command]
pub fn mm_create_document(input: CreateDocumentInput) -> Result<MindmapDocument, String> {
    super::db::create_document(&input.name, input.description.as_deref().unwrap_or(""), input.source_type.as_deref().unwrap_or("manual"), input.folder_id.as_deref())
}

#[tauri::command]
pub fn mm_update_document(input: UpdateDocumentInput) -> Result<(), String> {
    let fid = input.folder_id.as_deref();
    super::db::update_document(&input.id, input.name.as_deref(), input.description.as_deref(), Some(fid))
}

#[tauri::command]
pub fn mm_delete_document(id: String) -> Result<(), String> { super::db::delete_document(&id) }

#[tauri::command]
pub fn mm_update_background_texture(document_id: String, texture: String) -> Result<(), String> {
    super::db::update_background_texture(&document_id, &texture)
}

#[tauri::command]
pub fn mm_load_document(id: String) -> Result<Option<DocumentFull>, String> { super::db::load_full(&id) }

// ─── 节点 ───

#[tauri::command]
pub fn mm_upsert_node(input: UpsertNodeInput) -> Result<(), String> { super::db::upsert_node(&input.node) }

#[tauri::command]
pub fn mm_delete_node(input: DeleteNodeInput) -> Result<(), String> { super::db::delete_node(&input.document_id, &input.node_id) }

#[tauri::command]
pub fn mm_update_positions(document_id: String, positions: Vec<PositionInput>) -> Result<(), String> {
    let mut nodes = super::db::list_nodes(&document_id)?;
    let pm: std::collections::HashMap<&str, &PositionInput> = positions.iter().map(|p| (p.node_id.as_str(), p)).collect();
    for n in &mut nodes {
        if let Some(pos) = pm.get(n.id.as_str()) { n.position_x = pos.x; n.position_y = pos.y; }
    }
    super::db::batch_save_nodes(&nodes)
}

// ─── 贴纸 ───

#[tauri::command]
pub fn mm_upsert_sticker(input: UpsertStickerInput) -> Result<(), String> { super::db::upsert_sticker(&input.sticker) }

#[tauri::command]
pub fn mm_delete_sticker(input: DeleteStickerInput) -> Result<(), String> { super::db::delete_sticker(&input.document_id, &input.sticker_id) }

// ─── 导出 ───

#[tauri::command]
pub fn mm_export_markdown(document_id: String) -> Result<String, String> {
    let full = super::db::load_full(&document_id)?.ok_or("文档不存在")?;
    use std::collections::HashMap;
    let mut ch: HashMap<Option<&str>, Vec<&MindmapNode>> = HashMap::new();
    for n in &full.nodes { ch.entry(n.parent_id.as_deref()).or_default().push(n); }
    let mut out = format!("# {}\n\n> {} 更新时间: {}\n\n", full.document.name, full.document.source_type, full.document.updated_at);
    if !full.document.description.is_empty() { out.push_str(&format!("{}\n\n---\n\n", full.document.description)); }
    const MAX: usize = 32;
    fn toc(out: &mut String, pid: Option<&str>, ch: &HashMap<Option<&str>, Vec<&MindmapNode>>, d: usize, path: &mut std::collections::HashSet<String>) {
        if d > MAX { return; }
        if let Some(l) = ch.get(&pid) {
            for n in l {
                if !path.insert(n.id.clone()) { continue; }
                let p = "  ".repeat(d);
                out.push_str(&format!("{}- {} `{}`\n", p, n.name, n.kind));
                toc(out, Some(&n.id), ch, d + 1, path);
                path.remove(&n.id);
            }
        }
    }
    fn nodes(out: &mut String, pid: Option<&str>, ch: &HashMap<Option<&str>, Vec<&MindmapNode>>, d: usize, path: &mut std::collections::HashSet<String>) {
        if d > MAX { return; }
        if let Some(l) = ch.get(&pid) {
            for n in l {
                if !path.insert(n.id.clone()) { continue; }
                let h = "#".repeat((d + 2).min(6));
                out.push_str(&format!("{} {} ({} {}%)\n\n", h, n.name, n.kind, n.progress));
                if !n.description.is_empty() { out.push_str(&format!("> {}\n\n", n.description)); }
                if !n.detail.is_empty() { out.push_str(&format!("{}\n\n", n.detail)); }
                nodes(out, Some(&n.id), ch, d + 1, path);
                path.remove(&n.id);
            }
        }
    }
    out.push_str("## 目录\n\n");
    toc(&mut out, None, &ch, 0, &mut std::collections::HashSet::new());
    // 损坏或历史数据中的未知父级节点也必须导出，按独立根节点处理。
    let node_ids: std::collections::HashSet<&str> = full.nodes.iter().map(|n| n.id.as_str()).collect();
    let known_roots: std::collections::HashSet<&str> = full.nodes.iter()
        .filter(|n| n.parent_id.as_deref().map(|parent| !node_ids.contains(parent)).unwrap_or(true))
        .map(|n| n.id.as_str())
        .collect();
    let exported_roots: std::collections::HashSet<&str> = ch.get(&None).into_iter()
        .flat_map(|items| items.iter().map(|n| n.id.as_str())).collect();
    for n in &full.nodes {
        if known_roots.contains(n.id.as_str()) && !exported_roots.contains(n.id.as_str()) {
            out.push_str(&format!("- {} `{}`\n", n.name, n.kind));
        }
    }
    out.push_str("\n---\n\n## 详情\n\n");
    nodes(&mut out, None, &ch, 0, &mut std::collections::HashSet::new());
    for n in &full.nodes {
        if known_roots.contains(n.id.as_str()) && !exported_roots.contains(n.id.as_str()) {
            let h = "##";
            out.push_str(&format!("{} {} ({} {}%)\n\n", h, n.name, n.kind, n.progress));
            if !n.description.is_empty() { out.push_str(&format!("> {}\n\n", n.description)); }
            if !n.detail.is_empty() { out.push_str(&format!("{}\n\n", n.detail)); }
        }
    }
    if !full.stickers.is_empty() {
        out.push_str("\n---\n\n## 贴纸根节点\n\n- 贴纸根节点 `sticker-root`\n\n");
        for (i, sticker) in full.stickers.iter().enumerate() {
            out.push_str(&format!("### 贴纸 {}\n\n", i + 1));
            if !sticker.image_data.is_empty() {
                out.push_str(&format!("![图片贴纸 {}]({})\n\n", i + 1, sticker.image_data));
            }
            if !sticker.content.is_empty() {
                out.push_str(&format!("{}\n\n", sticker.content));
            }
        }
    }
    out.push_str("\n---\n*由 AnyVersion 思维导图生成*\n");
    Ok(out)
}

// ─── AI 生成 ───

fn project_prompt() -> String {
    r##"你是一位资深软件架构师。请根据用户提供的项目扫描结果，生成可直接导入思维导图的结构化 JSON。
只允许输出一个 JSON 对象，不要 Markdown 代码围栏、解释文字或尾随逗号：
{
  "summary": "项目整体用途、技术栈、主要运行链路的简明概述",
  "nodes": [
    {"id":"唯一稳定短 ID","name":"节点名称","parent_id":null,"description":"一句话职责","detail":"详细说明，可使用 Markdown","kind":"root|module|component|service|route|config|file|other","color":"#RRGGBB","progress":0}
  ]
}
要求：至少一个根节点，根节点 parent_id 必须为 null；其余节点只能通过 parent_id 引用本次输出中的 id，不能引用扫描结果之外的节点；项目总览作为根节点，按模块、组件、服务、路由、配置和关键文件组织层级；保留重要入口、依赖和运行关系，忽略依赖缓存、构建产物和重复目录；progress 必须是 0 到 100 的整数，color 必须是 6 位十六进制颜色；节点总数控制在 15 到 50 个。"##.to_string()
}

fn text_prompt() -> String {
    r##"你是一位产品经理和系统分析师。请从用户需求文本中提取可执行的思维导图结构，生成可直接导入的 JSON。
只允许输出一个 JSON 对象，不要 Markdown 代码围栏、解释文字或尾随逗号：
{
  "summary": "需求目标、范围和关键约束的概述",
  "nodes": [
    {"id":"唯一稳定短 ID","name":"节点名称","parent_id":null,"description":"验收关注点或一句话说明","detail":"详细需求、流程、边界条件，可使用 Markdown","kind":"root|module|requirement|task|constraint|risk|other","color":"#RRGGBB","progress":0}
  ]
}
要求：至少一个根节点，根节点 parent_id 必须为 null；所有其他 parent_id 只能引用本次输出中的 id；以目标/范围为根，按用户故事、功能需求、非功能约束、任务、风险和验收标准组织层级；不要臆造文本中没有依据的实现细节；progress 必须是 0 到 100 的整数，color 必须是 6 位十六进制颜色；节点总数控制在 8 到 30 个。"##.to_string()
}

#[tauri::command]
pub async fn mm_ai_from_project(input: AiGenerateProjectInput) -> Result<DocumentFull, String> {
    let pp = input.project_path.trim().to_string();
    if pp.is_empty() { return Err("路径为空".into()); }
    let pname = std::path::Path::new(&pp).file_name().and_then(|n| n.to_str()).unwrap_or("项目").to_string();
    // 扫描
    let context = super::scan::scan_project(&pp)?;
    let (provider, model) = resolve_provider_model(&input.provider_id, &input.model_id)?;
    let url = completion_url(&provider.openai_url);
    let body = serde_json::json!({"model":model,"stream":false,"temperature":0.3,"messages":[{"role":"system","content":project_prompt()},{"role":"user","content":context}]});
    let client = reqwest::Client::new();
    let resp = client.post(&url).header("Authorization", format!("Bearer {}", provider.api_key)).header("Content-Type","application/json").json(&body).timeout(Duration::from_secs(180)).send().await.map_err(|e| format!("请求失败: {}", e))?;
    let st = resp.status();
    let val: serde_json::Value = resp.json().await.map_err(|e| format!("解析失败: {}", e))?;
    if !st.is_success() { let msg = val.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str()).unwrap_or("?"); return Err(format!("AI错误 {}: {}", st.as_u16(), msg)); }
    let content = val.get("choices").and_then(|c| c.get(0)).and_then(|ch| ch.get("message")).and_then(|m| m.get("content")).and_then(|c| c.as_str()).unwrap_or("").trim().to_string();
    if content.is_empty() { return Err("AI返回空".into()); }
    let parsed = parse_json(&content)?;
    let summary = parsed.get("summary").and_then(|s| s.as_str()).unwrap_or("").to_string();
    let id_prefix = format!("{}-", super::db::new_id("ai"));
    let mut nodes = json_to_mindmap_nodes(&parsed, &input.document_id, &id_prefix);
    ensure_import_root(&mut nodes, &input.document_id, &id_prefix, &pname, &summary);
    // AI 导入始终追加一棵新的根树，不覆盖画布中已有的节点。
    super::db::batch_save_nodes(&nodes)?;
    super::db::update_document(&input.document_id, None, None, None)?;
    super::db::load_full(&input.document_id)?.ok_or("加载失败".into())
}

#[tauri::command]
pub async fn mm_ai_from_text(input: AiGenerateTextInput) -> Result<DocumentFull, String> {
    let text = input.text.trim().to_string();
    if text.is_empty() { return Err("文本为空".into()); }
    let title = if input.title.trim().is_empty() { "需求分析" } else { input.title.trim() };
    let (provider, model) = resolve_provider_model(&input.provider_id, &input.model_id)?;
    let url = completion_url(&provider.openai_url);
    let body = serde_json::json!({"model":model,"stream":false,"temperature":0.3,"messages":[{"role":"system","content":text_prompt()},{"role":"user","content":format!("分析以下文字提取结构化需求：\n\n{}", text)}]});
    let client = reqwest::Client::new();
    let resp = client.post(&url).header("Authorization", format!("Bearer {}", provider.api_key)).header("Content-Type","application/json").json(&body).timeout(Duration::from_secs(180)).send().await.map_err(|e| format!("请求失败: {}", e))?;
    let st = resp.status();
    let val: serde_json::Value = resp.json().await.map_err(|e| format!("解析失败: {}", e))?;
    if !st.is_success() { let msg = val.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str()).unwrap_or("?"); return Err(format!("AI错误 {}: {}", st.as_u16(), msg)); }
    let content = val.get("choices").and_then(|c| c.get(0)).and_then(|ch| ch.get("message")).and_then(|m| m.get("content")).and_then(|c| c.as_str()).unwrap_or("").trim().to_string();
    if content.is_empty() { return Err("AI返回空".into()); }
    let parsed = parse_json(&content)?;
    let summary = parsed.get("summary").and_then(|s| s.as_str()).unwrap_or("").to_string();
    let id_prefix = format!("{}-", super::db::new_id("ai"));
    let mut nodes = json_to_mindmap_nodes(&parsed, &input.document_id, &id_prefix);
    ensure_import_root(&mut nodes, &input.document_id, &id_prefix, title, &summary);
    // AI 解析同样追加新的根树，保留当前画布的其他内容。
    super::db::batch_save_nodes(&nodes)?;
    super::db::update_document(&input.document_id, None, None, None)?;
    super::db::load_full(&input.document_id)?.ok_or("加载失败".into())
}

// ─── 子树重新分析 ───

#[tauri::command]
pub async fn mm_regenerate_node(input: RegenerateNodeInput) -> Result<DocumentFull, String> {
    let full = super::db::load_full(&input.document_id)?.ok_or("文档不存在")?;
    let target = full.nodes.iter().find(|n| n.id == input.node_id).ok_or("节点不存在")?;
    let (provider, model) = resolve_provider_model(&input.provider_id, &input.model_id)?;
    let url = completion_url(&provider.openai_url);
    let prompt = format!(r#"分析"{}"模块内部结构。输出{{"nodes":[...]}}，首节点 parent_id=null 为该模块自身。3-12个子节点。只输出JSON。"#, target.name);
    let body = serde_json::json!({"model":model,"stream":false,"temperature":0.3,"messages":[{"role":"system","content":prompt},{"role":"user","content":format!("分析 {} 的子结构", target.name)}]});
    let client = reqwest::Client::new();
    let resp = client.post(&url).header("Authorization", format!("Bearer {}", provider.api_key)).header("Content-Type","application/json").json(&body).timeout(Duration::from_secs(180)).send().await.map_err(|e| format!("请求失败: {}", e))?;
    let st = resp.status();
    let val: serde_json::Value = resp.json().await.map_err(|e| format!("解析失败: {}", e))?;
    if !st.is_success() { let msg = val.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str()).unwrap_or("?"); return Err(format!("AI错误 {}: {}", st.as_u16(), msg)); }
    let content = val.get("choices").and_then(|c| c.get(0)).and_then(|ch| ch.get("message")).and_then(|m| m.get("content")).and_then(|c| c.as_str()).unwrap_or("").trim().to_string();
    if content.is_empty() { return Err("AI返回空".into()); }
    let parsed = parse_json(&content)?;
    let id_prefix = format!("{}-", super::db::new_id("ai-child"));
    let new_children = json_to_mindmap_nodes(&parsed, &input.document_id, &id_prefix);
    // 更新 target 描述
    if let Some(r) = new_children.iter().find(|n| n.parent_id.is_none()) {
        super::db::with_conn(|c| { super::db::sql(c.execute("UPDATE mindmap_nodes SET description=?1, detail=?2, updated_at=?3 WHERE id=?4", rusqlite::params![r.description, r.detail, super::db::now_ts(), target.id]))?; Ok(()) })?;
    }
    let root_ai = new_children.iter().find(|n| n.parent_id.is_none()).map(|n| n.id.clone()).unwrap_or_default();
    // 删旧后代（带 visited 防环：AI 生成的父指针若成环，无保护会无限循环卡死）
    let mut desc_ids = vec![target.id.clone()];
    let mut visited_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    visited_ids.insert(target.id.clone());
    let mut i = 0;
    while i < desc_ids.len() {
        let nodes = &full.nodes;
        for n in nodes {
            if n.parent_id.as_deref() == Some(&desc_ids[i]) && visited_ids.insert(n.id.clone()) {
                desc_ids.push(n.id.clone());
            }
        }
        i += 1;
    }
    let _skip_root = desc_ids.remove(0); // 保留 target 自身
    let desc_set: std::collections::HashSet<_> = desc_ids.into_iter().collect();
    if !desc_set.is_empty() {
        super::db::with_conn(|c| {
            for id in &desc_set { super::db::sql(c.execute("DELETE FROM mindmap_nodes WHERE id=?1", rusqlite::params![id]))?; }
            Ok(())
        })?;
    }
    // 插入新子节点
    for child in &new_children {
        if child.id == root_ai || child.parent_id.is_none() { continue; }
        let mut n = child.clone();
        if n.parent_id.as_deref() == Some(&root_ai) { n.parent_id = Some(target.id.clone()); }
        super::db::upsert_node(&n)?;
    }
    super::db::load_full(&input.document_id)?.ok_or("加载失败".into())
}

// ─── 初始化 ───

#[tauri::command]
pub fn mm_init() -> Result<(), String> { super::db::init_db() }

// ─── 文件夹 ───

#[tauri::command]
pub fn mm_list_folders() -> Result<Vec<MindmapFolder>, String> { super::db::list_folders() }

#[tauri::command]
pub fn mm_create_folder(input: CreateFolderInput) -> Result<MindmapFolder, String> { super::db::create_folder(&input.name) }

#[tauri::command]
pub fn mm_update_folder(input: UpdateFolderInput) -> Result<(), String> { super::db::update_folder(&input.id, input.name.as_deref()) }

#[tauri::command]
pub fn mm_delete_folder(id: String) -> Result<(), String> { super::db::delete_folder(&id) }

#[tauri::command]
pub fn mm_move_document(input: MoveDocumentInput) -> Result<(), String> { super::db::move_document(&input.document_id, input.folder_id.as_deref()) }