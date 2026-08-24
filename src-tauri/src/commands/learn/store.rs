//! 持久化所有分析结果，按项目路径定位。
//! 存储格式：`{data_dir}/learn/` 目录下，每份结构一个 `<hash>.json` 文件，
//! 外加 `index.json` 汇总所有条目（列表页用）。

use crate::commands::config::get_data_dir;
use super::models::{LearnGraph, LearnMeta};

fn learn_dir() -> std::path::PathBuf {
    get_data_dir().join("learn")
}

fn ensure_dir() -> Result<(), String> {
    std::fs::create_dir_all(&learn_dir()).map_err(|e| format!("创建 learn 目录失败: {}", e))
}

/// 按项目路径生成固定短 ID（取 MD5 前 12 位 hex）。
fn path_id(project_path: &str) -> String {
    let h = md5::compute(project_path.as_bytes());
    h.iter().take(6).map(|b| format!("{:02x}", b)).collect()
}

fn graph_path(project_path: &str) -> std::path::PathBuf {
    learn_dir().join(format!("{}.json", path_id(project_path)))
}

fn index_path() -> std::path::PathBuf {
    learn_dir().join("index.json")
}

fn load_index() -> Vec<LearnMeta> {
    let path = index_path();
    match std::fs::read_to_string(&path) {
        Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

fn save_index(list: &[LearnMeta]) -> Result<(), String> {
    ensure_dir()?;
    let path = index_path();
    let tmp = path.with_extension("json.tmp");
    let data = serde_json::to_string_pretty(list).map_err(|e| format!("序列化索引失败: {}", e))?;
    std::fs::write(&tmp, &data).map_err(|e| format!("写入临时索引失败: {}", e))?;
    if let Ok(f) = std::fs::File::open(&tmp) { let _ = f.sync_all(); }
    std::fs::rename(&tmp, &path).map_err(|e| format!("重命名索引文件失败: {}", e))
}

/// 保存图（原子写入：先写临时文件，fsync 后 rename，崩溃不丢旧图）。
pub fn save_graph(graph: &LearnGraph) -> Result<(), String> {
    ensure_dir()?;
    let path = graph_path(&graph.project_path);
    let tmp = path.with_extension("json.tmp");
    let data = serde_json::to_string_pretty(graph).map_err(|e| format!("序列化图失败: {}", e))?;
    std::fs::write(&tmp, &data).map_err(|e| format!("写入临时图文件失败: {}", e))?;
    // fsync 确保数据落盘后再 rename，避免半写文件残留
    if let Ok(f) = std::fs::File::open(&tmp) {
        let _ = f.sync_all();
    }
    std::fs::rename(&tmp, &path).map_err(|e| format!("重命名图文件失败: {}", e))?;
    // 更新索引
    let mut index = load_index();
    let meta = LearnMeta {
        project_path: graph.project_path.clone(),
        project_name: graph.project_name.clone(),
        generated_at: graph.generated_at.clone(),
        node_count: graph.nodes.len(),
    };
    if let Some(existing) = index.iter_mut().find(|m| m.project_path == graph.project_path) {
        *existing = meta;
    } else {
        index.push(meta);
    }
    save_index(&index)
}

/// 加载图。
pub fn load_graph(project_path: &str) -> Option<LearnGraph> {
    let path = graph_path(project_path);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|data| serde_json::from_str(&data).ok())
}

/// 列出所有分析记录（用于历史面板）。
pub fn list_metas() -> Vec<LearnMeta> {
    load_index()
}

/// 删除某项目的图与索引条目。
pub fn delete_graph(project_path: &str) -> Result<(), String> {
    let path = graph_path(project_path);
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("删除图文件失败: {}", e))?;
    }
    let mut index = load_index();
    index.retain(|m| m.project_path != project_path);
    save_index(&index)
}

// ─── Tauri 命令 ───

/// 列出所有已生成的学习结构（历史记录）。
#[tauri::command]
pub fn learn_list() -> Vec<LearnMeta> {
    list_metas()
}

/// 加载某项目的学习结构（若存在）。
#[tauri::command]
pub fn learn_load(project_path: String) -> Option<LearnGraph> {
    load_graph(&project_path)
}

/// 删除某项目的学习结构。
#[tauri::command]
pub fn learn_delete(project_path: String) -> Result<(), String> {
    delete_graph(&project_path)
}

/// 更新节点位置（拖放后持久化）。
#[tauri::command]
pub fn learn_update_positions(project_path: String, positions: Vec<super::models::NodePosition>) -> Result<(), String> {
    let mut graph = load_graph(&project_path)
        .ok_or_else(|| "未找到该项目的分析记录".to_string())?;
    let pos_map: std::collections::HashMap<&str, &super::models::NodePosition> =
        positions.iter().map(|p| (p.node_id.as_str(), p)).collect();
    for node in &mut graph.nodes {
        if let Some(pos) = pos_map.get(node.id.as_str()) {
            node.position_x = pos.x;
            node.position_y = pos.y;
        }
    }
    save_graph(&graph)
}

// ─── Markdown 导出 ───

/// 将 LearnGraph 渲染为结构化 Markdown 文档并返回字符串。
#[tauri::command]
pub fn learn_export_markdown(project_path: String) -> Result<String, String> {
    let graph = load_graph(&project_path)
        .ok_or_else(|| "未找到该项目的分析记录".to_string())?;
    render_graph_markdown(&graph)
}

/// 按层级将节点拼成 Markdown。
fn render_graph_markdown(graph: &LearnGraph) -> Result<String, String> {
    use super::models::LearnNode;
    use std::collections::HashMap;

    let mut children: HashMap<Option<&str>, Vec<&LearnNode>> = HashMap::new();
    for n in &graph.nodes {
        let pid = n.parent_id.as_deref();
        children.entry(pid).or_default().push(n);
    }

    let kind_labels: HashMap<&str, &str> = [
        ("root", "根节点"), ("module", "模块"), ("lib", "库"), ("component", "组件"),
        ("class", "类"), ("function", "函数"), ("service", "服务"), ("route", "路由"),
        ("config", "配置"), ("file", "文件"), ("entry", "入口"), ("other", "其他"),
    ].into_iter().collect();

    let mut out = String::new();

    // 标题
    out.push_str(&format!("# {}\n\n", graph.project_name));
    out.push_str(&format!("> 生成时间：{} · 共 {} 个节点\n\n", &graph.generated_at, graph.nodes.len()));

    // 概述
    if !graph.summary.is_empty() {
        out.push_str("## 项目概述\n\n");
        out.push_str(&graph.summary);
        out.push_str("\n\n---\n\n");
    }

    const MAX_GRAPH_DEPTH: usize = 32;

    // 目录
    out.push_str("## 结构目录\n\n");
    fn render_toc(
        out: &mut String,
        pid: Option<&str>,
        children: &HashMap<Option<&str>, Vec<&LearnNode>>,
        depth: usize,
    ) {
        if depth > MAX_GRAPH_DEPTH { return; }
        if let Some(list) = children.get(&pid) {
        for n in list {
            let prefix = "  ".repeat(depth);
            let anchor = n.name.to_lowercase().replace(|c: char| !c.is_alphanumeric() && c != '-', "-");
            out.push_str(&format!("{}- [{}](#{})\n", prefix, n.name, anchor));
            if !n.description.is_empty() {
                out.push_str(&format!("{}  *{}*\n", prefix, n.description));
            }
            render_toc(out, Some(&n.id), children, depth + 1);
        }
        }
    }
    render_toc(&mut out, None, &children, 0);
    out.push_str("\n---\n\n");

    // 各节点详情
    out.push_str("## 节点详情\n\n");
    fn render_nodes(
        out: &mut String,
        pid: Option<&str>,
        children: &HashMap<Option<&str>, Vec<&LearnNode>>,
        depth: usize,
        kind_labels: &HashMap<&str, &str>,
    ) {
        if depth > MAX_GRAPH_DEPTH { return; }
        if let Some(list) = children.get(&pid) {
        let level = (depth + 2).min(6);
        let hashes = "#".repeat(level);
        for n in list {
            let kind = kind_labels.get(n.kind.as_str()).copied().unwrap_or(&n.kind);
            out.push_str(&format!("{} {} `{}`\n\n", hashes, n.name, kind));
            if !n.description.is_empty() {
                out.push_str(&format!("> {}\n\n", n.description));
            }
            if !n.detail.is_empty() {
                out.push_str(&n.detail);
                if !n.detail.ends_with('\n') { out.push('\n'); }
                out.push('\n');
            }
            if n.description.is_empty() && n.detail.is_empty() {
                out.push_str("*暂无详细说明*\n\n");
            }
            render_nodes(out, Some(&n.id), children, depth + 1, kind_labels);
        }
        }
    }
    render_nodes(&mut out, None, &children, 0, &kind_labels);

    // 页脚
    out.push_str("\n---\n\n");
    out.push_str(&format!("*由 AnyVersion 项目学习模块生成于 {}*\n", &graph.generated_at));

    Ok(out)
}