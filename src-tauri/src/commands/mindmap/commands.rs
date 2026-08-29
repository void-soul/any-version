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
            detail: {
                let d = v.get("detail").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
                if d.is_empty() {
                    // 详情留空时用说明补全，保证每个节点的详情里都有模块说明
                    let desc = v.get("description").and_then(|x| x.as_str()).unwrap_or("").trim();
                    if desc.is_empty() { String::new() } else { desc.to_string() }
                } else {
                    d
                }
            },
            kind: if is_root { "root".to_string() } else { v.get("kind").and_then(|x| x.as_str()).unwrap_or("other").to_string() },
            color: v.get("color").and_then(|x| x.as_str()).unwrap_or(c).to_string(),
            progress: v.get("progress").and_then(|x| x.as_i64()).unwrap_or(0).clamp(0, 100) as i32,
            plan_at: v.get("plan_at").or_else(|| v.get("planAt")).and_then(|x| x.as_str()).map(|s| s.to_string()),
            repeat: v.get("repeat").and_then(|x| x.as_str()).unwrap_or("none").to_string(),
            // 证据锚定：sources 数组（项目相对路径），去重、去空、限 6 个
            sources: {
                let mut out: Vec<String> = Vec::new();
                if let Some(arr) = v.get("sources").and_then(|x| x.as_array()) {
                    for s in arr.iter().filter_map(|x| x.as_str()) {
                        let t = s.trim().trim_start_matches("./").to_string();
                        if !t.is_empty() && !out.contains(&t) {
                            out.push(t);
                        }
                        if out.len() >= 6 {
                            break;
                        }
                    }
                }
                out
            },
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
        kind: "root".to_string(), color: "#f8fafc".to_string(), progress: 0, plan_at: None, repeat: "none".to_string(), sources: Vec::new(),
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
pub fn mm_update_layout_dir(document_id: String, dir: String) -> Result<(), String> {
    super::db::update_layout_dir(&document_id, &dir)
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

// ─── 计划日历 ───

/// 指定日期范围（YYYY-MM-DD，含端点）内的具体计划发生记录；
/// 重复计划在 SQL 中展开，前端只需按 occur_day 分组。
#[tauri::command]
pub fn mm_planned_occurrences(start: String, end: String) -> Result<Vec<PlannedOccurrence>, String> {
    super::db::list_planned_occurrences(&start, &end)
}

/// 拖拽移动某次计划发生：按 from_day → to_day 的天数差改写 plan_at
/// （保留钟点与重复规则；daily/weekly 整条顺延，none 单次移动）。
#[tauri::command]
pub fn mm_move_plan_occurrence(input: MovePlanOccurrenceInput) -> Result<(), String> {
    super::db::move_plan_occurrence(&input.node_id, &input.from_day, &input.to_day)
}

// ─── 计划提醒（系统通知 + 托盘小红点） ───

/// 计划时间 → 本地 HH:MM（用于通知正文）
fn plan_local_hm(plan_at: &str) -> String {
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(plan_at) {
        return t.with_timezone(&chrono::Local).format("%H:%M").to_string();
    }
    plan_at.split('T').nth(1).and_then(|s| s.get(..5)).unwrap_or("").to_string()
}

/// 今天发生（含每天/每周重复）的计划，按时间排序。
/// 复用范围展开查询（今天→今天），与计划日历的归日逻辑完全一致。
pub fn mindmap_today_plans() -> Vec<PlannedOccurrence> {
    let today = chrono::Local::now().date_naive().format("%Y-%m-%d").to_string();
    super::db::list_planned_occurrences(&today, &today).unwrap_or_default()
}

/// 已发送今日通知的日期（同一天不重复弹）
static LAST_PLAN_NOTIFY_DATE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Windows 系统通知（经 PowerShell WinRT 的 Toast，零新增依赖）
#[cfg(target_os = "windows")]
fn show_win_toast(title: &str, body: &str) {
    fn esc(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
            .replace('"', "&quot;").replace('\'', "&apos;")
    }
    let title = esc(title);
    let body = esc(body);
    let script = format!(
        r#"$ErrorActionPreference='SilentlyContinue';
try {{
  [Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null
  [Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime] | Out-Null
  $xml = New-Object Windows.Data.Xml.Dom.XmlDocument
  $xml.LoadXml('<toast><visual><binding template="ToastGeneric"><text>{title}</text><text>{body}</text></binding></visual></toast>')
  $toast = New-Object Windows.UI.Notifications.ToastNotification $xml
  [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('WindowsPowerShell').Show($toast)
}} catch {{ }}"#,
        title = title, body = body
    );
    let _ = crate::commands::hidden_cmd::hidden_cmd("powershell")
        .args(["-NoProfile", "-NonInteractive", "-STA", "-Command", &script])
        .output();
}

#[cfg(not(target_os = "windows"))]
fn show_win_toast(_title: &str, _body: &str) {}

/// 刷新「今日计划」托盘小红点；今天有计划且当日未提醒过则弹系统通知。
#[tauri::command]
pub fn mm_refresh_plan_badge(app: tauri::AppHandle) -> Result<(), String> {
    let plans = mindmap_today_plans();
    let has = !plans.is_empty();
    crate::tray::set_tray_badge(&app, has)?;
    if has {
        let today = chrono::Local::now().date_naive().to_string();
        let mut guard = LAST_PLAN_NOTIFY_DATE.lock().unwrap_or_else(|e| e.into_inner());
        if guard.as_deref() != Some(today.as_str()) {
            *guard = Some(today);
            drop(guard);
            let summary = if plans.len() == 1 {
                format!("「{}」计划于今天 {}", plans[0].name, plan_local_hm(&plans[0].plan_at))
            } else {
                format!("今天有 {} 项计划", plans.len())
            };
            let lines: Vec<String> = plans.iter().take(3)
                .map(|p| format!("{} {} · {}", plan_local_hm(&p.plan_at), p.name, p.document_name))
                .collect();
            show_win_toast("思维导图计划提醒", &format!("{}\n{}", summary, lines.join("\n")));
        }
    } else if let Ok(mut g) = LAST_PLAN_NOTIFY_DATE.lock() {
        *g = None; // 今日无计划，清掉标记以便下次有计划时重新提醒
    }
    Ok(())
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
    fn append_sources(out: &mut String, n: &MindmapNode) {
        if n.sources.is_empty() { return; }
        out.push_str("📎 证据文件：\n");
        for s in &n.sources {
            out.push_str(&format!("- `{}`\n", s));
        }
        out.push_str("\n");
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
                append_sources(out, n);
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
            append_sources(&mut out, n);
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

/// 从节点名称/描述提取内容关键词（中英混合）：名称整体 + 英文词 + 中文片段及其 2-gram。
fn node_keywords(name: &str, description: &str) -> Vec<String> {
    const STOP_EN: &[&str] = &[
        "the", "and", "for", "with", "this", "that", "from", "are", "was", "has", "have",
        "not", "its", "all", "will", "can", "use", "using", "used", "module", "service",
        "component", "file", "node", "config", "data", "info", "main", "api", "user", "order",
        "list", "view", "page", "src", "new", "out", "set", "get", "add", "del",
        "一个", "这个", "以及", "相关", "进行", "提供", "支持", "处理", "管理", "结构",
        "内部", "主要", "描述", "用于", "负责", "实现", "功能", "模块", "服务", "组件",
        "节点", "配置", "文件", "系统",
    ];
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: String, is_name: bool| {
        let t = s.trim();
        if t.len() >= 2 && !out.iter().any(|x| x == t) && (is_name || !STOP_EN.contains(&t)) {
            out.push(t.to_string());
        }
    };
    // 名称整体始终保留（最强信号）
    let name_t = name.trim();
    if name_t.len() >= 2 {
        push(name_t.to_string(), true);
    }
    // 名称+描述：英文词 / 中文连续片段
    let raw = format!("{} {}", name, description);
    let mut buf = String::new();
    let mut mode = 0u8; // 0=间隔 1=ascii 2=cjk
    for ch in raw.chars() {
        let seg = if ch.is_ascii_alphanumeric() || ch == '_' {
            1
        } else if ('\u{4e00}'..='\u{9fff}').contains(&ch) {
            2
        } else {
            0
        };
        if seg != mode {
            if mode != 0 {
                push(std::mem::take(&mut buf), false);
            }
            mode = seg;
        }
        if seg != 0 {
            buf.push(ch);
        }
    }
    if mode != 0 {
        push(buf, false);
    }
    // 中文长片段补充 2-gram（滑动窗口），提高与代码注释的命中
    let cjk_runs: Vec<String> = out.iter().filter(|k| k.chars().all(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)) && k.len() >= 3).cloned().collect();
    for run in cjk_runs {
        let chars: Vec<char> = run.chars().collect();
        for w in chars.windows(2) {
            let bigram: String = w.iter().collect();
            if !out.iter().any(|x| x == &bigram) {
                out.push(bigram);
            }
        }
    }
    out.truncate(12);
    out
}

/// 校验 AI 返回的 nodes JSON 结构，返回错误列表（空 = 通过）。
/// 借鉴 Archify 的「校验驱动」：结构/语义检查，为修复循环提供可反馈的诊断。
/// project 非空时额外做证据校验：sources 必须真实存在于扫描结果中，
/// 且文件内容/路径须与节点说明相关（防标注错误文件）。
fn validate_ai_nodes_json(
    json: &serde_json::Value,
    project: Option<&super::scan::ProjectFiles>,
) -> Vec<String> {
    const KINDS: &[&str] = &[
        "root", "module", "requirement", "task", "constraint", "risk", "other",
        "component", "service", "route", "config", "file",
    ];
    let mut errs: Vec<String> = Vec::new();
    let Some(arr) = json.get("nodes").and_then(|v| v.as_array()) else {
        return vec!["缺少 nodes 数组".into()];
    };
    if arr.is_empty() {
        return vec!["nodes 为空".into()];
    }
    if arr.len() > 80 {
        errs.push(format!("节点数量 {} 超过上限 80", arr.len()));
    }
    let mut ids: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut roots = 0usize;
    for (i, n) in arr.iter().enumerate() {
        let tag = format!("节点 #{}（{}）", i + 1, n.get("name").and_then(|x| x.as_str()).unwrap_or("?"));
        let name = n.get("name").and_then(|x| x.as_str()).unwrap_or("");
        if name.trim().is_empty() {
            errs.push(format!("{}：name 为空", tag));
        }
        if let Some(k) = n.get("kind").and_then(|x| x.as_str()) {
            if !KINDS.contains(&k) {
                errs.push(format!("{}：kind '{}' 不在允许列表", tag, k));
            }
        }
        if let Some(c) = n.get("color").and_then(|x| x.as_str()) {
            let ok = c.is_empty()
                || (c.len() == 7
                    && c.starts_with('#')
                    && c[1..].chars().all(|ch| ch.is_ascii_hexdigit()));
            if !ok {
                errs.push(format!("{}：color '{}' 需为 #RRGGBB", tag, c));
            }
        }
        if let Some(p) = n.get("progress").and_then(|x| x.as_i64()) {
            if !(0..=100).contains(&p) {
                errs.push(format!("{}：progress {} 超出 0-100", tag, p));
            }
        }
        let raw_id = n.get("id").and_then(|x| x.as_str()).unwrap_or("");
        if raw_id.trim().is_empty() {
            errs.push(format!("{}：id 为空", tag));
        } else if !ids.insert(raw_id) {
            errs.push(format!("{}：id '{}' 重复", tag, raw_id));
        }
        let pid = n
            .get("parent_id")
            .or_else(|| n.get("parentId"))
            .and_then(|x| x.as_str())
            .filter(|s| !s.trim().is_empty() && *s != "null");
        match pid {
            None => roots += 1,
            Some(p) if !arr.iter().any(|m| m.get("id").and_then(|x| x.as_str()) == Some(p)) => {
                errs.push(format!("{}：parent_id '{}' 未在本批节点中定义", tag, p));
            }
            _ => {}
        }
    }
    if roots == 0 {
        errs.push("缺少根节点（应至少一个节点 parent_id 为 null）".into());
    } else if roots > 5 {
        errs.push(format!("根节点数量 {} 过多", roots));
    }
    // 证据锚定校验：sources 必须真实存在于扫描结果，且路径/内容与节点说明相关（防标注错误文件）
    if let Some(project) = project {
        let files = &project.files;
        // 内容读取缓存（同一文件被多节点引用时只读一次），仅读 ≤256KB、前 16K 字符
        let mut cache: std::collections::HashMap<String, Option<String>> = std::collections::HashMap::new();
        let mut read_capped = |rel: &str| -> Option<String> {
            if let Some(v) = cache.get(rel) {
                return v.clone();
            }
            let path = project.root.join(rel);
            let v = (|| {
                let meta = std::fs::metadata(&path).ok()?;
                if meta.len() > 256 * 1024 {
                    return None;
                }
                let data = std::fs::read_to_string(&path).ok()?;
                Some(data.chars().take(16 * 1024).collect::<String>().to_lowercase())
            })();
            cache.insert(rel.to_string(), v.clone());
            v
        };
        for (i, n) in arr.iter().enumerate() {
            let Some(srcs) = n.get("sources").and_then(|x| x.as_array()) else {
                continue;
            };
            let name = n.get("name").and_then(|x| x.as_str()).unwrap_or("");
            let desc = n.get("description").and_then(|x| x.as_str()).unwrap_or("");
            let kws: Vec<String> = node_keywords(name, desc).into_iter().map(|k| k.to_lowercase()).collect();
            let tag = format!("节点 #{}（{}）", i + 1, name);
            for s in srcs.iter().filter_map(|x| x.as_str()) {
                let p = s.trim().trim_start_matches("./").to_string();
                if p.is_empty() {
                    continue;
                }
                let exists = files.contains(&p)
                    || files.iter().any(|f| f.starts_with(&format!("{}/", p)));
                if !exists {
                    errs.push(format!("{}：sources '{}' 不在扫描结果中（请只引用目录结构中真实存在的文件）", tag, s));
                    continue;
                }
                // 路径自证：相对路径含关键词即视为相关（如节点名为 tsconfig.json、文件同名）
                let p_lower = p.to_lowercase();
                if !kws.is_empty() && kws.iter().any(|k| p_lower.contains(k.as_str())) {
                    continue;
                }
                // 内容相关度：读取文件内容，需命中至少一个关键词；无法读取（二进制/超大）则跳过
                if kws.is_empty() {
                    continue;
                }
                if let Some(content) = read_capped(&p) {
                    let hit = kws.iter().any(|k| content.contains(k.as_str()));
                    if !hit {
                        errs.push(format!(
                            "{}：sources '{}' 内容与节点说明不相关（未找到关键词『{}』，请核实标注的文件确实实现该模块，或移除 sources）",
                            tag,
                            s,
                            kws.iter().take(3).map(|k| k.as_str()).collect::<Vec<_>>().join("、")
                        ));
                    }
                }
            }
        }
    }
    errs
}

/// 请求一次 AI 并解析出 JSON 对象（请求/格式失败直接返回错误）。
async fn call_ai_json(
    provider: &ai::models::AiProvider,
    model: &str,
    system: &str,
    user: &str,
) -> Result<serde_json::Value, String> {
    let url = completion_url(&provider.openai_url);
    let body = serde_json::json!({"model": model, "stream": false, "temperature": 0.3, "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}]});
    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", provider.api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .timeout(Duration::from_secs(180))
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;
    let st = resp.status();
    let val: serde_json::Value = resp.json().await.map_err(|e| format!("解析失败: {}", e))?;
    if !st.is_success() {
        let msg = val
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("?");
        return Err(format!("AI错误 {}: {}", st.as_u16(), msg));
    }
    let content = val
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|ch| ch.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if content.is_empty() {
        return Err("AI返回空".into());
    }
    parse_json(&content)
}

/// 带校验修复循环的 AI 生成（借鉴 Archify 的 validate→repair）：
/// 校验失败时把诊断列表反馈给 AI 重新输出，最多 max_rounds 轮，
/// 保留错误数最少的一版。返回 (最终 JSON, 剩余校验错误, 实际调用轮数)；
/// 错误为空即完全通过，轮数 = 1 表示首次即通过。
/// project：项目文件集上下文，非空时校验 sources 证据真实性（存在性 + 内容相关度）。
async fn ai_generate_with_repair(
    provider: &ai::models::AiProvider,
    model: &str,
    system: &str,
    user: &str,
    max_rounds: usize,
    project: Option<&super::scan::ProjectFiles>,
) -> Result<(serde_json::Value, Vec<String>, usize), String> {
    let mut prompt = user.to_string();
    let mut best: Option<(serde_json::Value, Vec<String>, usize)> = None;
    for round in 0..max_rounds {
        let json = call_ai_json(provider, model, system, &prompt).await?;
        let errs = validate_ai_nodes_json(&json, project);
        if errs.is_empty() {
            return Ok((json, Vec::new(), round + 1));
        }
        let is_better = best
            .as_ref()
            .map(|(_, e, _)| errs.len() < e.len())
            .unwrap_or(true);
        if is_better {
            best = Some((json.clone(), errs.clone(), round + 1));
        }
        if round + 1 >= max_rounds {
            break;
        }
        prompt = format!(
            "{}\n\n—— 修复要求 ——\n你上一次输出的 JSON 未通过校验，请修正以下错误后重新输出完整 JSON（只输出 JSON，不要解释或 Markdown）：\n{}",
            user,
            errs.iter().map(|e| format!("- {}", e)).collect::<Vec<_>>().join("\n")
        );
    }
    let (json, errs, rounds) = best.unwrap_or_else(|| {
        (serde_json::json!({ "nodes": [] }), vec!["AI 未返回可用 JSON".into()], max_rounds)
    });
    Ok((json, errs, rounds))
}

/// AI 输出带修复循环：生成 → 校验 → （失败）把诊断反馈重试。
/// 若仍有剩余校验错误，如实附加到根节点 detail（不隐藏质量问题）。
async fn import_ai_nodes(
    document_id: &str,
    parsed: serde_json::Value,
    errs: Vec<String>,
    root_name: &str,
) -> Result<DocumentFull, String> {
    let summary = parsed.get("summary").and_then(|s| s.as_str()).unwrap_or("").to_string();
    let id_prefix = format!("{}-", super::db::new_id("ai"));
    let mut nodes = json_to_mindmap_nodes(&parsed, document_id, &id_prefix);
    if !errs.is_empty() {
        if let Some(root) = nodes.iter_mut().find(|n| n.parent_id.is_none()) {
            root.detail.push_str(&format!(
                "\n\n> ⚠️ AI 输出校验未完全通过（{} 项）：{}",
                errs.len(),
                errs.join("；")
            ));
        }
    }
    ensure_import_root(&mut nodes, document_id, &id_prefix, root_name, &summary);
    // AI 导入始终追加一棵新的根树，不覆盖画布中已有的节点。
    super::db::batch_save_nodes(&nodes)?;
    super::db::update_document(document_id, None, None, None)?;
    super::db::load_full(document_id)?.ok_or("加载失败".into())
}

// ─── 类型路由器（借鉴 Archify 的 type router） ───

const VIEW_LABELS: &[(&str, &str)] = &[
    ("architecture", "架构"),
    ("workflow", "流程"),
    ("dataflow", "数据流"),
    ("sequence", "时序"),
    ("lifecycle", "生命周期"),
];

fn view_label(view: &str) -> String {
    VIEW_LABELS
        .iter()
        .find(|(k, _)| *k == view)
        .map(|(_, l)| l.to_string())
        .unwrap_or_else(|| view.to_string())
}

/// 各视图的生成指导（组织方式），供视图级 prompt 使用。
fn view_guidance(view: &str) -> &'static str {
    match view {
        "workflow" => "以流程视角组织：根节点为流程总览，按阶段/步骤/分支展开，标注关键节点、审批与异常分支；kind 用 module/task/constraint/risk/other。",
        "dataflow" => "以数据流视角组织：根节点为数据流总览，按数据源→处理→存储→消费方展开，标注关键管道与依赖；kind 用 module/component/task/other。",
        "sequence" => "以时序视角组织：根节点为入口请求，按调用顺序逐级展开调用链（含返回/异步），detail 中简述请求→响应；kind 用 module/service/task/other。",
        "lifecycle" => "以生命周期视角组织：根节点为业务对象，按状态转移/阶段展开（含重试、等待与终态）；kind 用 module/requirement/constraint/task/other。",
        _ => "以架构视角组织：根节点为项目总览，按模块、组件、服务、路由、配置与关键文件分层。",
    }
}

/// 类型路由分类 prompt：让 AI 先判断适用哪些视图。
fn router_prompt(mode: &str) -> String {
    let subject = if mode == "project" { "项目扫描结果" } else { "需求文本" };
    format!(
        r##"你是软件架构师。请判断{subject}最值得用哪几种视图生成思维导图。
只输出一个 JSON 对象，不要 Markdown 或解释文字：
{{"views":[{{"type":"architecture","reason":"为什么选它（一句话）"}}]}}
可选 type（按适用度从高到低）：
- architecture：组件/服务/模块拓扑
- workflow：业务流程/任务编排/CI/CD
- dataflow：数据管道/ETL/血缘
- sequence：调用链/请求生命周期
- lifecycle：状态机/生命周期
要求：选择 1 到 3 个最贴合的视图，按重要程度排序；不要选明显不适用的；type 必须来自上述列表。"##
    )
}

/// 视图级生成 prompt（system）：按指定视角组织导图。
fn view_prompt(mode: &str, view: &str) -> String {
    let (kinds, count) = if mode == "project" {
        ("root|module|component|service|route|config|file|other", "10 到 30")
    } else {
        ("root|module|requirement|task|constraint|risk|other", "6 到 20")
    };
    let evidence = if mode == "project" {
        "\n请以扫描中的『技术栈与依赖』『目录规模』『项目标记』为证据确认框架、模块边界与部署形态，不要臆造扫描中不存在的依赖或模块。"
    } else {
        "\n只提取文本中有依据的内容，不要臆造。"
    };
    let opener = if mode == "project" {
        format!("你是一位资深软件架构师。请根据用户提供的项目扫描结果，生成「{}」视角的思维导图 JSON。", view_label(view))
    } else {
        format!("你是一位产品经理和系统分析师。请从用户需求文本中提取「{}」视角的思维导图 JSON。", view_label(view))
    };
    let guidance = view_guidance(view);
    let tpl = r##"{opener}
只允许输出一个 JSON 对象，不要 Markdown 代码围栏、解释文字或尾随逗号：
{{"summary":"该视角的简明概述","nodes":[{{"id":"唯一稳定短 ID","name":"节点名称","parent_id":null,"description":"一句话职责","detail":"详细说明，可使用 Markdown","kind":"{kinds}","color":"#RRGGBB","progress":0,"sources":["项目相对路径"]}}]}}
组织要求：{guidance}{evidence}
结构要求：至少一个根节点，根节点 parent_id 必须为 null；其余节点只能通过 parent_id 引用本次输出中的 id；每个节点的 detail 必须写明该模块/节点的说明（职责、边界、与相邻模块的关系，可用 Markdown），不得为空；description 保持一句话概述；progress 必须是 0 到 100 的整数，color 必须是 6 位十六进制颜色；节点总数控制在 {count} 个；只输出 JSON。{evidence_req}"##;
    let evidence_req = if mode == "project" {
        "\n证据要求：关键模块/组件/服务节点用 sources 字段标注 1 到 3 个真实文件（项目相对路径，必须在『目录结构』中出现），文件/配置类节点标注自身路径；sources 最多 6 个，只填真实存在的路径，不要臆造。"
    } else {
        ""
    };
    tpl.replace("{opener}", &opener)
        .replace("{kinds}", kinds)
        .replace("{count}", count)
        .replace("{guidance}", guidance)
        .replace("{evidence}", evidence)
        .replace("{evidence_req}", evidence_req)
}

/// 类型路由主流程：分类 → 逐视图生成（各建独立文档），单个视图失败不影响其它视图。
/// project_root 非空（项目模式）时收集文件集用于证据校验，并把项目根写入文档 source_desc。
async fn run_ai_router(
    provider: &ai::models::AiProvider,
    model: &str,
    mode: &str,
    context: &str,
    root_name: &str,
    project_root: Option<&str>,
) -> Result<AiImportResult, String> {
    // 项目模式：文件集上下文（证据锚定校验用：存在性 + 内容相关度）
    let project_files = match project_root {
        Some(p) => match super::scan::collect_project_files(p) {
            Ok(f) => Some(f),
            Err(_) => None, // 文件集失败不阻断生成，只是跳过证据校验
        },
        None => None,
    };
    // 1. 类型路由：判断适合哪些视图（分类失败则回退为单架构视图）
    let mut views: Vec<String> = vec!["architecture".to_string()];
    if let Ok(router_json) = call_ai_json(provider, model, &router_prompt(mode), context).await {
        if let Some(arr) = router_json.get("views").and_then(|v| v.as_array()) {
            let picked: Vec<String> = arr
                .iter()
                .filter_map(|v| v.get("type").and_then(|t| t.as_str()).map(|s| s.to_string()))
                .filter(|t| VIEW_LABELS.iter().any(|(k, _)| k == t))
                .collect();
            let mut seen = std::collections::HashSet::new();
            views = picked
                .into_iter()
                .filter(|t| seen.insert(t.clone()))
                .take(3)
                .collect();
            if views.is_empty() {
                views = vec!["architecture".to_string()];
            }
        }
    }

    // 2. 逐视图生成：各建独立文档；失败清理空文档并记录原因
    let mut docs: Vec<DocumentFull> = Vec::new();
    let mut failures: Vec<AiImportFailure> = Vec::new();
    let mut reports: Vec<AiImportReport> = Vec::new();
    let mut primary: Option<String> = None;
    for view in &views {
        let doc_name = format!("{} · {}", root_name, view_label(view));
        let doc = match super::db::create_document(
            &doc_name,
            "",
            if mode == "project" { "ai_project" } else { "ai_text" },
            None,
        ) {
            Ok(d) => d,
            Err(e) => {
                failures.push(AiImportFailure { view: view.clone(), reason: e });
                continue;
            }
        };
        let user = if mode == "project" {
            context.to_string()
        } else {
            format!("分析以下文字提取结构化需求：\n\n{}", context)
        };
        // 项目模式：记录项目根路径到文档 source_desc，供证据文件定位
        if let Some(p) = project_root {
            let _ = super::db::update_source_desc(&doc.id, p);
        }
        let outcome = async {
            let (parsed, errs, rounds) = ai_generate_with_repair(
                provider,
                model,
                &view_prompt(mode, view),
                &user,
                3,
                project_files.as_ref(),
            )
            .await?;
            Ok::<_, String>((parsed, errs, rounds))
        }
        .await;
        let (parsed, errs, rounds) = match outcome {
            Ok(x) => x,
            Err(e) => {
                let _ = super::db::delete_document(&doc.id); // 清理空文档
                failures.push(AiImportFailure { view: view.clone(), reason: e });
                continue;
            }
        };
        match import_ai_nodes(&doc.id, parsed, errs.clone(), &doc_name).await {
            Ok(full) => {
                if primary.is_none() {
                    primary = Some(full.document.id.clone());
                }
                // 证据统计：引用数 / 有证据节点数 / 命中率（项目模式用文件集核验）
                let evidence_count = full.nodes.iter().map(|n| n.sources.len()).sum();
                let evidence_nodes = full.nodes.iter().filter(|n| !n.sources.is_empty()).count();
                let (evidence_hit_count, evidence_verified) = match &project_files {
                    Some(pf) => {
                        let files = &pf.files;
                        let hit = full
                            .nodes
                            .iter()
                            .flat_map(|n| &n.sources)
                            .filter(|s| {
                                files.contains(s.as_str())
                                    || files.iter().any(|f| f.starts_with(&format!("{}/", s)))
                            })
                            .count();
                        (hit, true)
                    }
                    None => (evidence_count, false),
                };
                reports.push(AiImportReport {
                    document_id: full.document.id.clone(),
                    view: view.clone(),
                    node_count: full.nodes.len(),
                    repair_rounds: rounds,
                    diagnostics: errs,
                    evidence_count,
                    evidence_hit_count,
                    evidence_verified,
                    evidence_nodes,
                });
                docs.push(full);
            }
            Err(e) => {
                let _ = super::db::delete_document(&doc.id);
                failures.push(AiImportFailure { view: view.clone(), reason: e });
            }
        }
    }
    if docs.is_empty() {
        return Err(failures
            .first()
            .map(|f| format!("「{}」视图生成失败: {}", view_label(&f.view), f.reason))
            .unwrap_or_else(|| "AI 未生成任何视图".into()));
    }
    Ok(AiImportResult {
        primary_id: primary.unwrap_or_else(|| docs[0].document.id.clone()),
        documents: docs,
        failures,
        reports,
    })
}

#[tauri::command]
pub async fn mm_ai_from_project(input: AiGenerateProjectInput) -> Result<AiImportResult, String> {
    let pp = input.project_path.trim().to_string();
    if pp.is_empty() { return Err("路径为空".into()); }
    let pname = std::path::Path::new(&pp).file_name().and_then(|n| n.to_str()).unwrap_or("项目").to_string();
    // 扫描（含技术栈/目录规模/标记等仓库证据）
    let context = super::scan::scan_project(&pp)?;
    let (provider, model) = resolve_provider_model(&input.provider_id, &input.model_id)?;
    run_ai_router(&provider, &model, "project", &context, &pname, Some(&pp)).await
}

#[tauri::command]
pub async fn mm_ai_from_text(input: AiGenerateTextInput) -> Result<AiImportResult, String> {
    let text = input.text.trim().to_string();
    if text.is_empty() { return Err("文本为空".into()); }
    let title = if input.title.trim().is_empty() { "需求分析" } else { input.title.trim() };
    let (provider, model) = resolve_provider_model(&input.provider_id, &input.model_id)?;
    run_ai_router(&provider, &model, "text", &text, title, None).await
}

// ─── 子树重新分析 ───

/// 子树重析 prompt：结构化 JSON 输出 + 校验约束（kind 白名单/color/progress/父引用）。
fn regenerate_prompt() -> String {
    r##"你是一位资深软件架构师。请分析指定模块的内部结构，生成可直接导入思维导图的 JSON。
只允许输出一个 JSON 对象，不要 Markdown 代码围栏、解释文字或尾随逗号：
{"nodes":[{"id":"唯一稳定短 ID","name":"节点名称","parent_id":null,"description":"一句话职责","detail":"详细说明，可使用 Markdown","kind":"root|module|component|service|route|config|file|task|requirement|constraint|risk|other","color":"#RRGGBB","progress":0}]}
要求：第一个节点是该模块自身（parent_id 必须为 null，kind 用 root 或 module），其余 3 到 12 个节点是其子结构；所有子节点只能通过 parent_id 引用本批输出中的 id；每个节点的 detail 必须写明该模块/子模块的说明（职责、边界、与相邻模块的关系，可用 Markdown），不得为空；description 保持一句话概述；kind 必须在允许列表内；progress 必须是 0 到 100 的整数；color 必须是 6 位十六进制颜色；只输出 JSON。"##.to_string()
}

#[tauri::command]
pub async fn mm_regenerate_node(input: RegenerateNodeInput) -> Result<DocumentFull, String> {
    let full = super::db::load_full(&input.document_id)?.ok_or("文档不存在")?;
    let target = full.nodes.iter().find(|n| n.id == input.node_id).ok_or("节点不存在")?;
    let (provider, model) = resolve_provider_model(&input.provider_id, &input.model_id)?;
    // 上下文：模块自身信息 + 现有直接子节点，让 AI 知道在重析什么
    let direct_children: Vec<&str> = full
        .nodes
        .iter()
        .filter(|n| n.parent_id.as_deref() == Some(target.id.as_str()))
        .map(|n| n.name.as_str())
        .collect();
    let detail_take = target.detail.chars().take(800).collect::<String>();
    let children_txt = if direct_children.is_empty() {
        "（无）".to_string()
    } else {
        direct_children.join("、")
    };
    let user = format!(
        "请分析「{}」模块（位于文档「{}」，类型 {}，进度 {}%）的内部结构。\n模块描述：{}\n模块详情：{}\n现有子节点：{}",
        target.name,
        full.document.name,
        target.kind,
        target.progress,
        target.description,
        if detail_take.is_empty() { "（无）".to_string() } else { detail_take },
        children_txt,
    );
    // 生成 → 校验 → （失败）诊断反馈重试，最多 3 轮（子树重析不做证据校验）
    let (parsed, errs, _rounds) =
        ai_generate_with_repair(&provider, &model, &regenerate_prompt(), &user, 3, None).await?;
    let id_prefix = format!("{}-", super::db::new_id("ai-child"));
    let new_children = json_to_mindmap_nodes(&parsed, &input.document_id, &id_prefix);
    // 更新 target 描述/详情（校验未完全通过时如实附加警告）
    if let Some(r) = new_children.iter().find(|n| n.parent_id.is_none()) {
        let mut detail = r.detail.clone();
        if !errs.is_empty() {
            detail.push_str(&format!(
                "\n\n> ⚠️ AI 输出校验未完全通过（{} 项）：{}",
                errs.len(),
                errs.join("；")
            ));
        }
        super::db::with_conn(|c| { super::db::sql(c.execute("UPDATE mindmap_nodes SET description=?1, detail=?2, updated_at=?3 WHERE id=?4", rusqlite::params![r.description, detail, super::db::now_ts(), target.id]))?; Ok(()) })?;
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
pub fn mm_create_folder(input: CreateFolderInput) -> Result<MindmapFolder, String> { super::db::create_folder(&input.name, input.folder_id.as_deref()) }

#[tauri::command]
pub fn mm_update_folder(input: UpdateFolderInput) -> Result<(), String> { super::db::update_folder(&input.id, input.name.as_deref()) }

#[tauri::command]
pub fn mm_delete_folder(id: String) -> Result<(), String> { super::db::delete_folder(&id) }

#[tauri::command]
pub fn mm_move_folder(input: MoveFolderInput) -> Result<(), String> { super::db::move_folder(&input.folder_id, input.parent_id.as_deref()) }#[tauri::command]
pub fn mm_move_document(input: MoveDocumentInput) -> Result<(), String> { super::db::move_document(&input.document_id, input.folder_id.as_deref()) }

#[cfg(test)]
mod tests {
    use super::*;

    fn project_files(tmp: &std::path::Path, v: &[&str]) -> crate::commands::mindmap::scan::ProjectFiles {
        crate::commands::mindmap::scan::ProjectFiles {
            root: tmp.to_path_buf(),
            files: v.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn tmp_project(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mm_validate_test_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn validator_rejects_hallucinated_sources() {
        let dir = tmp_project("halluc");
        // 文件内容与节点说明无关：标注错误文件应被内容相关度检查拦下
        std::fs::create_dir_all(dir.join("src/services")).unwrap();
        std::fs::create_dir_all(dir.join("src/routes")).unwrap();
        std::fs::write(dir.join("src/services/order.ts"), "export class OrderService { 订单处理 }\n").unwrap();
        std::fs::write(dir.join("src/routes/order.ts"), "export const routes = [invoice, billing, payment];\n").unwrap();
        let pf = project_files(&dir, &["src/services/order.ts", "src/routes/order.ts"]);
        let json = serde_json::json!({
            "nodes": [
                {"id": "a", "name": "订单服务", "parent_id": null, "sources": ["src/services/order.ts"]},
                {"id": "b", "name": "订单路由", "parent_id": "a", "sources": ["src/routes/order.ts", "src/routes/nonexistent.ts"]}
            ]
        });
        let errs = validate_ai_nodes_json(&json, Some(&pf));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(errs.iter().any(|e| e.contains("nonexistent.ts")), "got: {errs:?}");
        // 订单服务节点的证据 order.ts 内容含关键词，应通过
        assert!(!errs.iter().any(|e| e.contains("services/order.ts")), "got: {errs:?}");
        // 订单路由节点的证据 routes/order.ts 内容与「订单路由」不相关（无中文关键词），应被拦下
        assert!(errs.iter().any(|e| e.contains("内容与节点说明不相关")), "got: {errs:?}");
        // 无文件集时不校验证据
        let errs3 = validate_ai_nodes_json(&json, None);
        assert!(errs3.is_empty(), "got: {errs3:?}");
    }

    #[test]
    fn validator_path_self_evidence_and_dir_prefix() {
        let dir = tmp_project("pathev");
        std::fs::create_dir_all(dir.join("src/services")).unwrap();
        // 空内容 + 路径自证（节点名与文件同名）：不应报内容不相关
        std::fs::write(dir.join("src/services/order.ts"), "").unwrap();
        let pf = project_files(&dir, &["src/services/order.ts"]);
        let json = serde_json::json!({"nodes": [{"id": "a", "name": "order.ts", "parent_id": null, "sources": ["src/services/order.ts"]}]});
        let errs = validate_ai_nodes_json(&json, Some(&pf));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(errs.is_empty(), "got: {errs:?}");
        // 目录前缀视为存在（引用整个目录作证据）；节点名过短无关键词 → 跳过内容检查
        let pf2 = project_files(&dir, &["src/services/order.ts"]);
        let json2 = serde_json::json!({"nodes": [{"id": "a", "name": "x", "parent_id": null, "sources": ["src/services"]}]});
        let errs2 = validate_ai_nodes_json(&json2, Some(&pf2));
        assert!(errs2.is_empty(), "got: {errs2:?}");
    }

    #[test]
    fn node_keywords_mixed_lang() {
        let kws = node_keywords("订单服务", "负责订单查询与生成，使用 OrderService");
        assert!(kws.iter().any(|k| k == "订单服务"), "got: {kws:?}");
        assert!(kws.iter().any(|k| k == "orderservice" || k == "OrderService") || kws.iter().any(|k| k.eq_ignore_ascii_case("orderservice")), "got: {kws:?}");
        assert!(kws.iter().any(|k| k == "订单"), "got: {kws:?}");
    }

    #[test]
    fn json_nodes_maps_sources() {
        let json = serde_json::json!({
            "nodes": [
                {"id": "a", "name": "服务", "parent_id": null,
                 "sources": ["./src/a.ts", "src/b.ts", "src/b.ts", "", "src/c.ts"]}
            ]
        });
        let nodes = json_to_mindmap_nodes(&json, "doc1", "p-");
        assert_eq!(nodes[0].sources, vec!["src/a.ts", "src/b.ts", "src/c.ts"]);
        // detail 为空时用 description 兜底
        let json2 = serde_json::json!({"nodes": [{"id": "a", "name": "x", "parent_id": null, "description": "模块说明", "detail": ""}]});
        let nodes2 = json_to_mindmap_nodes(&json2, "doc1", "p-");
        assert_eq!(nodes2[0].detail, "模块说明");
    }
}
