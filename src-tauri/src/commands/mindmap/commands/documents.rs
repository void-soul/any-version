use super::*;

#[tauri::command]
pub fn mm_list_documents(folder_id: Option<String>) -> Result<Vec<MindmapDocument>, String> {
    crate::commands::mindmap::db::list_documents(folder_id.as_deref())
}

#[tauri::command]
pub fn mm_create_document(input: CreateDocumentInput) -> Result<MindmapDocument, String> {
    crate::commands::mindmap::db::create_document(
        &input.name,
        input.description.as_deref().unwrap_or(""),
        input.source_type.as_deref().unwrap_or("manual"),
        input.folder_id.as_deref(),
    )
}

#[tauri::command]
pub fn mm_update_document(input: UpdateDocumentInput) -> Result<(), String> {
    let fid = input.folder_id.as_deref();
    crate::commands::mindmap::db::update_document(
        &input.id,
        input.name.as_deref(),
        input.description.as_deref(),
        Some(fid),
    )
}

#[tauri::command]
pub fn mm_delete_document(id: String) -> Result<(), String> {
    crate::commands::mindmap::db::delete_document(&id)
}

#[tauri::command]
pub fn mm_update_background_texture(document_id: String, texture: String) -> Result<(), String> {
    crate::commands::mindmap::db::update_background_texture(&document_id, &texture)
}

#[tauri::command]
pub fn mm_update_layout_dir(document_id: String, dir: String) -> Result<(), String> {
    crate::commands::mindmap::db::update_layout_dir(&document_id, &dir)
}

#[tauri::command]
pub fn mm_load_document(id: String) -> Result<Option<DocumentFull>, String> {
    crate::commands::mindmap::db::load_full(&id)
}

/// 从外部 JSON 文件导入思维导图（**不需要 AI**）。
///
/// 格式就是 AI 导入内部用的那一份节点契约（见 `json_to_mindmap_nodes`），
/// 现在对外开放：别的 Agent / 脚本生成好文件后，用户点「导入」即可进导图。
/// 文件可以是：
/// - `{"nodes":[...]}` —— 只有节点，文档名取文件名
/// - `{"name":"...","description":"...","layoutDir":"lr","nodes":[...]}` —— 带文档元信息
#[tauri::command]
pub fn mm_import_nodes(input: MindmapImportInput) -> Result<MindmapImportResult, String> {
    let raw = match input
        .content
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        Some(text) => text.to_string(),
        None => {
            let path = input
                .path
                .as_deref()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .ok_or("未提供导入内容：需要 path 或 content")?;
            std::fs::read_to_string(path)
                .map_err(|e| format!("读取导入文件失败 {}: {}", path, e))?
        }
    };
    // 有些模型会给 JSON 套一层 Markdown 代码围栏，剥掉再解析
    let raw = raw.trim();
    let raw = raw
        .strip_prefix("```json")
        .or_else(|| raw.strip_prefix("```"))
        .unwrap_or(raw);
    let raw = raw.trim_end_matches("```").trim();
    let json: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("导入文件不是合法 JSON: {}", e))?;

    let nodes_value = json
        .get("nodes")
        .and_then(|v| v.as_array())
        .ok_or("导入文件缺少 nodes 数组")?;
    if nodes_value.is_empty() {
        return Err("导入文件的 nodes 是空的".to_string());
    }

    // 文档：优先导入到指定文档，否则新建（名字依次取 JSON name/title → 文件名 → 默认）
    let document_id = match input
        .document_id
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        Some(id) => {
            crate::commands::mindmap::db::load_full(id)?
                .ok_or_else(|| format!("目标文档不存在: {}", id))?
                .document
                .id
        }
        None => {
            let fallback = input
                .path
                .as_deref()
                .map(std::path::Path::new)
                .and_then(|p| p.file_stem())
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let name = json
                .get("name")
                .and_then(|v| v.as_str())
                .or_else(|| json.get("title").and_then(|v| v.as_str()))
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| {
                    if fallback.is_empty() {
                        "导入的导图".to_string()
                    } else {
                        fallback
                    }
                });
            let description = json
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            crate::commands::mindmap::db::create_document(
                &name,
                description,
                "import",
                input.folder_id.as_deref(),
            )?
            .id
        }
    };

    // 文档元信息：布局方向（非法值回退默认）
    if let Some(dir) = json
        .get("layoutDir")
        .or_else(|| json.get("layout_dir"))
        .and_then(|v| v.as_str())
    {
        if matches!(dir, "lr" | "rl" | "tb" | "bt") {
            let _ = crate::commands::mindmap::db::update_layout_dir(&document_id, dir);
        }
    }

    if input.replace_existing {
        crate::commands::mindmap::db::with_conn(|c| {
            crate::commands::mindmap::db::sql(c.execute(
                "DELETE FROM mindmap_nodes WHERE document_id=?1",
                rusqlite::params![document_id],
            ))
        })?;
    }

    let id_prefix = crate::commands::mindmap::db::new_id("imp");
    let id_prefix = format!("{}_", id_prefix);
    let mut nodes = json_to_mindmap_nodes(&json, &document_id, &id_prefix);
    let mut warnings = Vec::new();
    normalize_imported_nodes(&mut nodes, &mut warnings);
    let node_count = nodes.len();
    for n in &nodes {
        crate::commands::mindmap::db::upsert_node(n)?;
    }
    crate::commands::mindmap::db::touch_document(&document_id)?;

    Ok(MindmapImportResult {
        document_id,
        node_count,
        warnings,
    })
}

/// 导入节点的规范化：AI 内部路径容忍度高，外部文件要替用户兜住明显错误。
pub(super) fn normalize_imported_nodes(nodes: &mut Vec<MindmapNode>, warnings: &mut Vec<String>) {
    for n in nodes {
        if n.name.trim().is_empty() {
            n.name = "未命名".to_string();
            warnings.push(format!("节点 {} 缺少 name，已填「未命名」", n.id));
        }
        if !NODE_KINDS.contains(&n.kind.as_str()) {
            warnings.push(format!(
                "节点「{}」的 kind={} 不在允许列表，已改成 other",
                n.name, n.kind
            ));
            n.kind = "other".to_string();
        }
        let ok = n.color.len() == 7
            && n.color.starts_with('#')
            && n.color[1..].chars().all(|c| c.is_ascii_hexdigit());
        if !ok {
            warnings.push(format!(
                "节点「{}」的 color={} 不是 #RRGGBB，已用默认色",
                n.name, n.color
            ));
            n.color = "#94a3b8".to_string();
        }
    }
}

#[tauri::command]
pub fn mm_upsert_node(input: UpsertNodeInput) -> Result<(), String> {
    crate::commands::mindmap::db::upsert_node(&input.node)
}

#[tauri::command]
pub fn mm_delete_node(input: DeleteNodeInput) -> Result<(), String> {
    crate::commands::mindmap::db::delete_node(&input.document_id, &input.node_id)
}

#[tauri::command]
pub fn mm_update_positions(
    document_id: String,
    positions: Vec<PositionInput>,
) -> Result<(), String> {
    let mut nodes = crate::commands::mindmap::db::list_nodes(&document_id)?;
    let pm: std::collections::HashMap<&str, &PositionInput> =
        positions.iter().map(|p| (p.node_id.as_str(), p)).collect();
    for n in &mut nodes {
        if let Some(pos) = pm.get(n.id.as_str()) {
            n.position_x = pos.x;
            n.position_y = pos.y;
        }
    }
    crate::commands::mindmap::db::batch_save_nodes(&nodes)
}

#[tauri::command]
pub fn mm_upsert_sticker(input: UpsertStickerInput) -> Result<(), String> {
    crate::commands::mindmap::db::upsert_sticker(&input.sticker)
}

#[tauri::command]
pub fn mm_delete_sticker(input: DeleteStickerInput) -> Result<(), String> {
    crate::commands::mindmap::db::delete_sticker(&input.document_id, &input.sticker_id)
}

#[tauri::command]
pub fn mm_upsert_link(input: UpsertLinkInput) -> Result<(), String> {
    crate::commands::mindmap::db::upsert_link(&input.link)
}

#[tauri::command]
pub fn mm_delete_link(input: DeleteLinkInput) -> Result<(), String> {
    crate::commands::mindmap::db::delete_link(&input.document_id, &input.link_id)
}

#[tauri::command]
pub fn mm_export_markdown(document_id: String) -> Result<String, String> {
    let full = crate::commands::mindmap::db::load_full(&document_id)?.ok_or("文档不存在")?;
    use std::collections::HashMap;
    let mut ch: HashMap<Option<&str>, Vec<&MindmapNode>> = HashMap::new();
    for n in &full.nodes {
        ch.entry(n.parent_id.as_deref()).or_default().push(n);
    }
    let mut out = format!(
        "# {}\n\n> {} 更新时间: {}\n\n",
        full.document.name, full.document.source_type, full.document.updated_at
    );
    if !full.document.description.is_empty() {
        out.push_str(&format!("{}\n\n---\n\n", full.document.description));
    }
    const MAX: usize = 32;
    fn toc(
        out: &mut String,
        pid: Option<&str>,
        ch: &HashMap<Option<&str>, Vec<&MindmapNode>>,
        d: usize,
        path: &mut std::collections::HashSet<String>,
    ) {
        if d > MAX {
            return;
        }
        if let Some(l) = ch.get(&pid) {
            for n in l {
                if !path.insert(n.id.clone()) {
                    continue;
                }
                let p = "  ".repeat(d);
                out.push_str(&format!("{}- {} `{}`\n", p, n.name, n.kind));
                toc(out, Some(&n.id), ch, d + 1, path);
                path.remove(&n.id);
            }
        }
    }
    fn append_sources(out: &mut String, n: &MindmapNode) {
        if n.sources.is_empty() {
            return;
        }
        out.push_str("📎 证据文件：\n");
        for s in &n.sources {
            out.push_str(&format!("- `{}`\n", s));
        }
        out.push_str("\n");
    }
    fn nodes(
        out: &mut String,
        pid: Option<&str>,
        ch: &HashMap<Option<&str>, Vec<&MindmapNode>>,
        d: usize,
        path: &mut std::collections::HashSet<String>,
    ) {
        if d > MAX {
            return;
        }
        if let Some(l) = ch.get(&pid) {
            for n in l {
                if !path.insert(n.id.clone()) {
                    continue;
                }
                let h = "#".repeat((d + 2).min(6));
                out.push_str(&format!("{} {} ({})\n\n", h, n.name, n.kind));
                if !n.detail.is_empty() {
                    out.push_str(&format!("{}\n\n", n.detail));
                }
                append_sources(out, n);
                nodes(out, Some(&n.id), ch, d + 1, path);
                path.remove(&n.id);
            }
        }
    }
    out.push_str("## 目录\n\n");
    toc(
        &mut out,
        None,
        &ch,
        0,
        &mut std::collections::HashSet::new(),
    );
    // 损坏或历史数据中的未知父级节点也必须导出，按独立根节点处理。
    let node_ids: std::collections::HashSet<&str> =
        full.nodes.iter().map(|n| n.id.as_str()).collect();
    let known_roots: std::collections::HashSet<&str> = full
        .nodes
        .iter()
        .filter(|n| {
            n.parent_id
                .as_deref()
                .map(|parent| !node_ids.contains(parent))
                .unwrap_or(true)
        })
        .map(|n| n.id.as_str())
        .collect();
    let exported_roots: std::collections::HashSet<&str> = ch
        .get(&None)
        .into_iter()
        .flat_map(|items| items.iter().map(|n| n.id.as_str()))
        .collect();
    for n in &full.nodes {
        if known_roots.contains(n.id.as_str()) && !exported_roots.contains(n.id.as_str()) {
            out.push_str(&format!("- {} `{}`\n", n.name, n.kind));
        }
    }
    out.push_str("\n---\n\n## 详情\n\n");
    nodes(
        &mut out,
        None,
        &ch,
        0,
        &mut std::collections::HashSet::new(),
    );
    for n in &full.nodes {
        if known_roots.contains(n.id.as_str()) && !exported_roots.contains(n.id.as_str()) {
            let h = "##";
            out.push_str(&format!("{} {} ({})\n\n", h, n.name, n.kind));
            if !n.detail.is_empty() {
                out.push_str(&format!("{}\n\n", n.detail));
            }
            append_sources(&mut out, n);
        }
    }
    if !full.stickers.is_empty() {
        out.push_str("\n---\n\n## 贴纸根节点\n\n- 贴纸根节点 `sticker-root`\n\n");
        for (i, sticker) in full.stickers.iter().enumerate() {
            out.push_str(&format!("### 贴纸 {}\n\n", i + 1));
            if !sticker.image_data.is_empty() {
                out.push_str(&format!(
                    "![图片贴纸 {}]({})\n\n",
                    i + 1,
                    sticker.image_data
                ));
            }
            if !sticker.content.is_empty() {
                out.push_str(&format!("{}\n\n", sticker.content));
            }
        }
    }
    out.push_str("\n---\n*由 Kira 思维导图生成*\n");
    Ok(out)
}

#[tauri::command]
pub fn mm_init() -> Result<(), String> {
    crate::commands::mindmap::db::init_db()
}

#[tauri::command]
pub fn mm_list_folders() -> Result<Vec<MindmapFolder>, String> {
    crate::commands::mindmap::db::list_folders()
}

#[tauri::command]
pub fn mm_create_folder(input: CreateFolderInput) -> Result<MindmapFolder, String> {
    crate::commands::mindmap::db::create_folder(&input.name, input.folder_id.as_deref())
}

#[tauri::command]
pub fn mm_update_folder(input: UpdateFolderInput) -> Result<(), String> {
    crate::commands::mindmap::db::update_folder(&input.id, input.name.as_deref())
}

#[tauri::command]
pub fn mm_delete_folder(id: String) -> Result<(), String> {
    crate::commands::mindmap::db::delete_folder(&id)
}

#[tauri::command]
pub fn mm_move_folder(input: MoveFolderInput) -> Result<(), String> {
    crate::commands::mindmap::db::move_folder(&input.folder_id, input.parent_id.as_deref())
}

#[tauri::command]
pub fn mm_move_document(input: MoveDocumentInput) -> Result<(), String> {
    crate::commands::mindmap::db::move_document(&input.document_id, input.folder_id.as_deref())
}
