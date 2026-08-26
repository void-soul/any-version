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

fn json_to_mindmap_nodes(json: &serde_json::Value, document_id: &str) -> Vec<MindmapNode> {
    let arr = match json.get("nodes").and_then(|v| v.as_array()) { Some(a) => a, None => return vec![] };
    let ts = super::db::now_ts();
    let colors = ["#22d3ee","#34d399","#fbbf24","#60a5fa","#fb7185","#a78bfa","#f97316","#f59e0b","#f8fafc","#94a3b8"];
    arr.iter().enumerate().map(|(i, v)| {
        let c = colors[i % colors.len()];
        MindmapNode {
            id: v.get("id").and_then(|x| x.as_str()).map(|s| s.to_string()).unwrap_or_else(|| format!("n{}", i+1)),
            document_id: document_id.to_string(),
            parent_id: v.get("parent_id").and_then(|x| x.as_str()).filter(|s| !s.is_empty() && *s != "null").map(|s| s.to_string()),
            name: v.get("name").and_then(|x| x.as_str()).unwrap_or("未命名").to_string(),
            description: v.get("description").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            detail: v.get("detail").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            kind: v.get("kind").and_then(|x| x.as_str()).unwrap_or("other").to_string(),
            color: v.get("color").and_then(|x| x.as_str()).unwrap_or(c).to_string(),
            progress: v.get("progress").and_then(|x| x.as_i64()).unwrap_or(0) as i32,
            position_x: 0.0, position_y: 0.0,
            created_at: ts.clone(), updated_at: ts.clone(),
        }
    }).collect()
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
    fn toc(out: &mut String, pid: Option<&str>, ch: &HashMap<Option<&str>, Vec<&MindmapNode>>, d: usize) {
        if d > MAX { return; }
        if let Some(l) = ch.get(&pid) { for n in l { let p = "  ".repeat(d); out.push_str(&format!("{}- {} `{}`\n", p, n.name, n.kind)); toc(out, Some(&n.id), ch, d+1); } }
    }
    fn nodes(out: &mut String, pid: Option<&str>, ch: &HashMap<Option<&str>, Vec<&MindmapNode>>, d: usize) {
        if d > MAX { return; }
        if let Some(l) = ch.get(&pid) { for n in l { let h = "#".repeat((d+2).min(6)); out.push_str(&format!("{} {} ({} {}%)\n\n", h, n.name, n.kind, n.progress)); if !n.description.is_empty() { out.push_str(&format!("> {}\n\n", n.description)); } if !n.detail.is_empty() { out.push_str(&format!("{}\n\n", n.detail)); } nodes(out, Some(&n.id), ch, d+1); } }
    }
    out.push_str("## 目录\n\n"); toc(&mut out, None, &ch, 0);
    out.push_str("\n---\n\n## 详情\n\n"); nodes(&mut out, None, &ch, 0);
    out.push_str(&format!("\n---\n*由 AnyVersion 思维导图生成*\n"));
    Ok(out)
}

// ─── AI 生成 ───

fn project_prompt() -> String {
    r#"你是一位软件架构师。分析项目目录结构，输出思维导图 JSON。
格式：{ "summary":"概述", "nodes":[...] }
节点字段：id/name/parent_id/description/detail/kind/color/progress
kind: root/module/component/service/route/config/file/other
根节点 parent_id=null。总数15-50。只输出JSON。"#.to_string()
}

fn text_prompt() -> String {
    r#"你是一位产品经理。从文字中提取需求思维导图。
格式：{ "summary":"概述", "nodes":[...] }
节点字段：id/name/parent_id/description/detail/kind/color/progress
kind: root/module/requirement/task/constraint/risk/other
根节点 parent_id=null。总数8-30。只输出JSON。"#.to_string()
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
    let mut nodes = json_to_mindmap_nodes(&parsed, &input.document_id);
    let has_root = nodes.iter().any(|n| n.parent_id.is_none());
    if !has_root {
        let ts = super::db::now_ts();
        nodes.insert(0, MindmapNode { id: "root".into(), document_id: input.document_id.clone(), parent_id: None, name: pname, description: "根".into(), detail: summary, kind: "root".into(), color: "#f8fafc".into(), progress: 0, position_x: 0.0, position_y: 0.0, created_at: ts.clone(), updated_at: ts });
    }
    // 删除旧节点，批量插入新节点
    super::db::with_conn(|c| { super::db::sql(c.execute("DELETE FROM mindmap_nodes WHERE document_id=?1", rusqlite::params![input.document_id]))?; Ok(()) })?;
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
    let mut nodes = json_to_mindmap_nodes(&parsed, &input.document_id);
    let has_root = nodes.iter().any(|n| n.parent_id.is_none());
    if !has_root {
        let ts = super::db::now_ts();
        nodes.insert(0, MindmapNode { id: "root".into(), document_id: input.document_id.clone(), parent_id: None, name: title.to_string(), description: "根".into(), detail: summary, kind: "root".into(), color: "#f8fafc".into(), progress: 0, position_x: 0.0, position_y: 0.0, created_at: ts.clone(), updated_at: ts });
    }
    super::db::with_conn(|c| { super::db::sql(c.execute("DELETE FROM mindmap_nodes WHERE document_id=?1", rusqlite::params![input.document_id]))?; Ok(()) })?;
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
    let new_children = json_to_mindmap_nodes(&parsed, &input.document_id);
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