//! Tauri 命令：思维导图 CRUD + AI 生成 + 导出。

use crate::commands::ai;
use tauri::Emitter;
use super::models::*;

// ─── Progreso en vivo (eventos a la UI) ───

/// Emite un evento de progreso del importador IA a la interfaz.
/// El frontend escucha "mm-ai-progress" y pinta el log paso a paso.
/// `extra` aporta los campos estructurados del paso (round/reason/files/views/view/count/detail).
fn emit_progress(app: &Option<tauri::AppHandle>, step: &str, extra: serde_json::Value) {
    if let Some(handle) = app {
        let mut payload = extra;
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("step".into(), serde_json::json!(step));
        }
        let _ = handle.emit("mm-ai-progress", payload);
    }
}

// ─── AI 运行取消（类似 IDE 中断构建） ───

/// 取消时返回的标准错误文本（前端据此识别为「用户主动取消」而非失败）。
const ERR_CANCELLED: &str = "已取消";

type CancelFlag = std::sync::Arc<std::sync::atomic::AtomicBool>;
type CancelRegistry = std::sync::Mutex<std::collections::HashMap<String, CancelFlag>>;
type AskSender = tokio::sync::oneshot::Sender<serde_json::Value>;
type AskRegistry = std::sync::Mutex<std::collections::HashMap<String, AskSender>>;

/// 进程级 AI 运行取消标志注册表：每个 run_id 一把独立 AtomicBool，避免并行运行互相误伤。
fn cancel_registry() -> &'static CancelRegistry {
    static CANCELS: std::sync::OnceLock<CancelRegistry> = std::sync::OnceLock::new();
    CANCELS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn ai_cancel_flag(run_id: &str) -> CancelFlag {
    let mut g = cancel_registry().lock().unwrap();
    g.entry(run_id.to_string())
        .or_insert_with(|| std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)))
        .clone()
}

/// 运行结束后从同一个注册表移除该 run_id 的 flag（新导入会重新创建干净标志）。
fn ai_drop_flag(run_id: &str) {
    cancel_registry().lock().unwrap().remove(run_id);
}

fn is_cancelled(flag: &std::sync::atomic::AtomicBool) -> bool {
    flag.load(std::sync::atomic::Ordering::Relaxed)
}

/// 取消检查：已取消时推送 cancel 事件并返回 Err(已取消)。
fn cancel_err(app: &Option<tauri::AppHandle>, flag: &std::sync::atomic::AtomicBool) -> Result<(), String> {
    if is_cancelled(flag) {
        emit_progress(app, "cancel", serde_json::json!({}));
        return Err(ERR_CANCELLED.into());
    }
    Ok(())
}

// ─── AI 询问通道（ask-user）───
// AI 在生成中遇到信息不足/歧义时，可输出 {"ask": {...}} 向用户提问；
// 后端注册一个 oneshot 发送端（按 run_id 索引），阻塞等待前端 mm_ai_answer 回填，
// 然后把用户回答追加进提示词继续生成。取消时（mm_ai_cancel）会向该通道发送
// Null 标记把等待解除，避免用户点「停止」后运行卡死在提问处。
fn ask_registry() -> &'static AskRegistry {
    static ASKS: std::sync::OnceLock<AskRegistry> = std::sync::OnceLock::new();
    ASKS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn ask_register(run_id: &str, tx: AskSender) {
    ask_registry().lock().unwrap().insert(run_id.to_string(), tx);
}

/// 用户回填询问答案（mm_ai_answer 调用）。无等待中的询问时返回 Err。
fn ask_send_answer(run_id: &str, answer: serde_json::Value) -> Result<(), String> {
    match ask_registry().lock().unwrap().remove(run_id) {
        Some(tx) => tx.send(answer).map_err(|_| "询问已结束".into()),
        None => Err("当前没有等待中的询问".into()),
    }
}

/// 取消时解除等待中的询问（mm_ai_cancel 调用，尽力而为）。
fn ask_send_cancel(run_id: &str) {
    if let Some(tx) = ask_registry().lock().unwrap().remove(run_id) {
        let _ = tx.send(serde_json::Value::Null);
    }
}

/// 把 AI 输出的 ask 归一化为 { question, fields:[{key,label,type,options,default}] }。
fn normalize_ask(ask: &serde_json::Value) -> serde_json::Value {
    match ask {
        serde_json::Value::String(s) => serde_json::json!({ "question": s, "fields": [] }),
        serde_json::Value::Object(_) => {
            let mut obj = ask.as_object().unwrap().clone();
            if !obj.contains_key("question") {
                obj.insert("question".into(), serde_json::Value::String("请补充以下信息".into()));
            }
            if !obj.contains_key("fields") || !obj.get("fields").map(|v| v.is_array()).unwrap_or(false) {
                obj.insert("fields".into(), serde_json::json!([]));
            }
            serde_json::Value::Object(obj)
        }
        _ => serde_json::json!({ "question": "请补充以下信息", "fields": [] }),
    }
}

/// 把「询问 + 用户回答」格式化为追加进提示词的文本人话。
fn format_ask_answer(ask: &serde_json::Value, answer: &serde_json::Value) -> String {
    let mut out = String::new();
    let q = ask.get("question").and_then(|x| x.as_str()).unwrap_or("");
    if !q.is_empty() { out.push_str(&format!("问题：{}\n", q)); }
    let fields = ask.get("fields").and_then(|x| x.as_array());
    match (fields, answer) {
        (Some(fs), a) if !fs.is_empty() => {
            out.push_str("回答：\n");
            for f in fs {
                let key = f.get("key").and_then(|x| x.as_str()).unwrap_or("");
                let label = f.get("label").and_then(|x| x.as_str()).unwrap_or(key);
                let val = a.get(key).map(|v| match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                }).unwrap_or_default();
                out.push_str(&format!("- {}：{}\n", label, val));
            }
        }
        (_, serde_json::Value::String(s)) if !s.trim().is_empty() => {
            out.push_str(&format!("回答：{}\n", s));
        }
        (_, a) if !a.is_null() => {
            out.push_str(&format!("回答：{}\n", a));
        }
        _ => { out.push_str("回答：（用户未填写，请基于现有信息合理推断并继续）\n"); }
    }
    out
}

// ─── 工具 ───

/// 一次 AI 运行（路由/探索/逐视图生成）的 token 累计器：原子计数，异步任务间安全共享。
#[derive(Default)]
struct UsageAcc {
    requests: std::sync::atomic::AtomicU64,
    input: std::sync::atomic::AtomicU64,
    output: std::sync::atomic::AtomicU64,
    total: std::sync::atomic::AtomicU64,
}

impl UsageAcc {
    fn snapshot(&self) -> UsageStats {
        use std::sync::atomic::Ordering;
        UsageStats {
            requests: self.requests.load(Ordering::Relaxed),
            input_tokens: self.input.load(Ordering::Relaxed),
            output_tokens: self.output.load(Ordering::Relaxed),
            total_tokens: self.total.load(Ordering::Relaxed),
        }
    }

    /// 相对某个基准快照的增量（用于算「单个视图」的消耗，探索/路由等公共部分不计入）。
    fn diff(&self, base: &UsageStats) -> UsageStats {
        let s = self.snapshot();
        UsageStats {
            requests: s.requests.saturating_sub(base.requests),
            input_tokens: s.input_tokens.saturating_sub(base.input_tokens),
            output_tokens: s.output_tokens.saturating_sub(base.output_tokens),
            total_tokens: s.total_tokens.saturating_sub(base.total_tokens),
        }
    }
}

/// 思维导图 AI 导入的供应商/模型解析：优先级与翻译等其它 AI 功能一致——
/// 1. 显式传入（面板当前选择）
/// 2. 全局默认 AI 模型（全局设置中选择，存于 translate_config.json）——校验可用性，
///    失效时静默回退首个可用供应商
/// 3. 首个有 api_key 且配置了 OpenAI 端点的供应商
fn resolve_provider_model(pid: &Option<String>, mid: &Option<String>) -> Result<(ai::models::AiProvider, String), String> {
    let cfg = ai::config::load_ai_config();
    let default_cfg = ai::translate::load_translate_config();
    let (p, explicit_mid) = if let Some(id) = pid {
        (cfg.providers.iter().find(|x| &x.id == id).cloned().ok_or_else(|| format!("未找到供应商: {}", id))?, mid.clone())
    } else {
        // 全局默认供应商可用（存在且配了端点与 key）才采用；否则回退首个可用供应商。
        // 默认模型只与默认供应商成对使用，避免跨供应商拼出不存在的模型 id。
        match (&default_cfg.provider_id, mid) {
            (Some(gpid), _) => match cfg.providers.iter().find(|x| &x.id == gpid) {
                Some(p) if !p.openai_url.is_empty() && !p.api_key.is_empty() =>
                    (p.clone(), mid.clone().or(default_cfg.model_id.clone())),
                _ => (first_usable_provider(&cfg)?, None),
            },
            _ => (first_usable_provider(&cfg)?, None),
        }
    };
    if p.openai_url.is_empty() { return Err(format!("供应商 '{}' 未配置端点", p.name)); }
    if p.api_key.is_empty() { return Err(format!("供应商 '{}' 未配置 Key", p.name)); }
    let m = explicit_mid.or_else(|| p.active_model_id.clone()).or_else(|| p.models.first().map(|m| m.id.clone())).ok_or("无可用模型")?;
    Ok((p, m))
}

/// 首个有 api_key 且配置了 OpenAI 端点的供应商（全局默认失效/缺省时的回退）。
fn first_usable_provider(cfg: &ai::models::AiConfig) -> Result<ai::models::AiProvider, String> {
    cfg.providers.iter().find(|x| !x.api_key.is_empty() && !x.openai_url.is_empty()).cloned().ok_or("无可用供应商".to_string())
}

/// 把 serde_json 的错误定位（行/列）换算为原文上下文窗口，便于直接看出坏在哪。
/// 宽容 JSON 解析已上提到共享通道（供 tool-call 降级等场景复用）。
use ai::channel::parse_json;

/// 探索点单专用请求：走共享通道的原生 tool-calling（request_files 工具 + tool_choice
/// 强制点名）。网关不支持 tools 时通道自动降级为纯文本 JSON 协议（system prompt 中的
/// 输出格式约定仍然生效，降级路径无需额外处理）。
/// 其余业务收尾与 [`call_ai_json`] 相同：usage 记账、失败日志。
async fn call_ai_json_for_explorer(
    app: &Option<tauri::AppHandle>,
    acc: &UsageAcc,
    cancel: &std::sync::atomic::AtomicBool,
    provider: &ai::models::AiProvider,
    model: &str,
    system: &str,
    user: &str,
) -> Result<serde_json::Value, String> {
    cancel_err(app, cancel)?;
    let hooks = MmHooks { app, cancel };
    let outcome = ai::channel::complete_chat_json(
        &hooks,
        provider,
        model,
        system,
        user,
        0.3,
        ai::channel::EXPLORER_TOOL_SPEC,
    )
    .await;
    let (json, usage) = match outcome {
        Ok(x) => x,
        Err(e) => {
            log_ai_transport_failure(app, model, &e);
            return Err(e);
        }
    };
    if let Some(u) = usage {
        record_and_emit_usage(app, acc, model, &provider.id, &u);
    }
    Ok(json)
}

/// 通用 JSON 对象请求（流式，文本协议）。

fn json_to_mindmap_nodes(json: &serde_json::Value, document_id: &str, id_prefix: &str) -> Vec<MindmapNode> {
    let arr = match json.get("nodes").and_then(|v| v.as_array()) { Some(a) => a, None => return vec![] };
    let colors =["#22d3ee","#34d399","#fbbf24","#60a5fa","#fb7185","#a78bfa","#f97316","#f59e0b","#f8fafc","#94a3b8"];
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
            detail: {
                let d = v.get("detail").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
                if d.is_empty() {
                    // 详情留空时用旧的 description 字段补全（兼容旧数据/AI 仍输出的说明），保证每个节点的详情里都有模块说明
                    let desc = v.get("description").and_then(|x| x.as_str()).unwrap_or("").trim();
                    if desc.is_empty() { String::new() } else { desc.to_string() }
                } else {
                    d
                }
            },
            kind: if is_root { "root".to_string() } else { v.get("kind").and_then(|x| x.as_str()).unwrap_or("other").to_string() },
            color: v.get("color").and_then(|x| x.as_str()).unwrap_or(c).to_string(),
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
        }
    }).collect()
}

fn ensure_import_root(nodes: &mut Vec<MindmapNode>, document_id: &str, id_prefix: &str, name: &str, summary: &str) {
    if nodes.iter().any(|n| n.parent_id.is_none()) { return; }
    nodes.insert(0, MindmapNode {
        id: format!("{}root", id_prefix), document_id: document_id.to_string(), parent_id: None,
        name: name.to_string(), detail: if summary.is_empty() { "AI 导入根节点".to_string() } else { summary.to_string() },
        kind: "root".to_string(), color: "#f8fafc".to_string(), sources: Vec::new(),
        position_x: 0.0, position_y: 0.0,
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

// ─── 贴纸 ───

#[tauri::command]
pub fn mm_upsert_sticker(input: UpsertStickerInput) -> Result<(), String> { super::db::upsert_sticker(&input.sticker) }

#[tauri::command]
pub fn mm_delete_sticker(input: DeleteStickerInput) -> Result<(), String> { super::db::delete_sticker(&input.document_id, &input.sticker_id) }

// ─── 自由关系线 ───

#[tauri::command]
pub fn mm_upsert_link(input: UpsertLinkInput) -> Result<(), String> { super::db::upsert_link(&input.link) }

#[tauri::command]
pub fn mm_delete_link(input: DeleteLinkInput) -> Result<(), String> { super::db::delete_link(&input.document_id, &input.link_id) }

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
                out.push_str(&format!("{} {} ({})\n\n", h, n.name, n.kind));
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
            out.push_str(&format!("{} {} ({})\n\n", h, n.name, n.kind));
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
    out.push_str("\n---\n*由 Kira 思维导图生成*\n");
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
            // 旧数据/AI 兼容：description 已废弃，内容并入 detail 后用于关键词
            let desc = n.get("description").and_then(|x| x.as_str()).unwrap_or("");
            let detail = n.get("detail").and_then(|x| x.as_str()).unwrap_or("");
            let kws: Vec<String> = node_keywords(name, &format!("{desc} {detail}")).into_iter().map(|k| k.to_lowercase()).collect();
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

// ─── AI 请求通道（共享实现见 crate::commands::ai::channel） ───
//
// TTFB 超时 / send 重试 / 流式断点续写 / SSE 消费 / stream_options 兼容等传输韧性
// 已收编到共享模块；本文件只保留两件事：
// 1. MmHooks：把通道进度事件接到 "mm-ai-progress"，把取消检查接到 run_id 取消标志；
// 2. call_ai_json：思维导图业务收尾（token 统计、stream done 事件、宽容 JSON 解析与日志）。

/// 思维导图 AI 通道钩子：进度 → "mm-ai-progress"；取消 → run_id AtomicBool。
struct MmHooks<'a> {
    app: &'a Option<tauri::AppHandle>,
    cancel: &'a std::sync::atomic::AtomicBool,
}

impl<'a> ai::channel::ChannelHooks for MmHooks<'a> {
    fn on_progress(&self, step: &str, extra: serde_json::Value) {
        emit_progress(self.app, step, extra);
    }
    fn check_cancel(&self) -> Result<(), String> {
        cancel_err(self.app, self.cancel)
    }
}

/// 从 usage JSON 提取 token 数：累计进本次运行计数器，并推送 step=usage 事件（前端实时统计）。
/// 同时落库到 AI 模块的全局用量统计（tool_id=mindmap）：思维导图不经代理、直连共享通道，
/// 不落库的话 AI 模块用量面板看不到这部分消耗。
fn record_and_emit_usage(app: &Option<tauri::AppHandle>, acc: &UsageAcc, model: &str, provider_id: &str, u: &serde_json::Value) {
    use std::sync::atomic::Ordering;
    let prompt_tokens = u.get("prompt_tokens").and_then(|x| x.as_u64()).unwrap_or(0);
    let completion_tokens = u.get("completion_tokens").and_then(|x| x.as_u64()).unwrap_or(0);
    let total_tokens = u
        .get("total_tokens")
        .and_then(|x| x.as_u64())
        .unwrap_or(prompt_tokens + completion_tokens);
    if prompt_tokens > 0 || completion_tokens > 0 {
        acc.requests.fetch_add(1, Ordering::Relaxed);
        acc.input.fetch_add(prompt_tokens, Ordering::Relaxed);
        acc.output.fetch_add(completion_tokens, Ordering::Relaxed);
        acc.total.fetch_add(total_tokens, Ordering::Relaxed);
        // 全局用量统计（AI 模块用量面板）：按供应商归属；落库失败不阻断导入流程
        let _ = ai::usage::log_usage_db("mindmap", model, Some(provider_id), prompt_tokens, completion_tokens);
        emit_progress(app, "usage", serde_json::json!({
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "total_tokens": total_tokens,
            "model": model,
        }));
    }
}

/// 请求一次 AI 并解析出 JSON 对象（流式）。传输韧性（TTFB 超时 / send 重试 /
/// 断点续写 / stream_options 兼容）由共享通道 ai::channel 提供；本函数只做
/// 思维导图业务收尾：token 统计（step=usage + 累计进 acc）、stream done 事件、
/// 宽容 JSON 解析（失败时把 AI 原始输出落盘到滚动日志便于定位）。
async fn call_ai_json(
    app: &Option<tauri::AppHandle>,
    acc: &UsageAcc,
    cancel: &std::sync::atomic::AtomicBool,
    provider: &ai::models::AiProvider,
    model: &str,
    system: &str,
    user: &str,
) -> Result<serde_json::Value, String> {
    cancel_err(app, cancel)?;
    let hooks = MmHooks { app, cancel };
    let outcome = ai::channel::stream_chat_with_resume(
        &hooks, provider, model, system, user,
        0.3,
        |len, tail| emit_progress(app, "stream", serde_json::json!({ "length": len, "text": tail })),
    )
    .await;
    let outcome = match outcome {
        Ok(o) => o,
        Err(e) => {
            log_ai_transport_failure(app, model, &e);
            return Err(e);
        }
    };
    if let Some(u) = outcome.usage {
        record_and_emit_usage(app, acc, model, &provider.id, &u);
    }
    let streamed = outcome.text;
    emit_progress(app, "stream", serde_json::json!({ "done": true, "length": streamed.chars().count() }));
    if streamed.trim().is_empty() {
        return Err("AI返回空".into());
    }
    match parse_json(&streamed) {
        Ok(v) => Ok(v),
        Err(e) => {
            // 解析失败时把 AI 原始输出落盘到滚动日志（any-version.log），
            // 便于在日志里定位「JSON: invalid escape」背后的真实内容问题。
            let len = streamed.chars().count();
            let head: String = streamed.chars().take(3000).collect();
            tracing::error!(
                "[mindmap] AI JSON 解析失败 (len={}): {}；
AI 原始输出开头:
{}",
                len, e, head
            );
            Err(e)
        }
    }
}

/// 网络类/传输类失败统一落盘到滚动日志（any-version.log），并推送 fail 事件
/// 让前端进度日志展示原始原因（不再只有「视图生成失败」一句话）。
fn log_ai_transport_failure(app: &Option<tauri::AppHandle>, model: &str, err: &str) {
    tracing::error!("[mindmap] AI 请求失败（model={}）：{}", model, err);
    emit_progress(app, "fail", serde_json::json!({ "detail": format!("网络错误（将按可恢复策略处理）：{}", err) }));
}

/// 带校验修复循环的 AI 生成（借鉴 Archify 的 validate→repair）：
/// 校验失败时把诊断列表反馈给 AI 重新输出，最多 max_rounds 轮，
/// 保留错误数最少的一版。返回 (最终 JSON, 剩余校验错误, 实际调用轮数)；
/// 错误为空即完全通过，轮数 = 1 表示首次即通过。
/// project：项目文件集上下文，非空时校验 sources 证据真实性（存在性 + 内容相关度）。
/// 询问机制：AI 遇到信息不足/歧义时可输出 {"ask": {...}} 向用户提问（最多 3 次），
/// 后端推送 step=ask 事件并阻塞等待前端 mm_ai_answer 回填，再把回答追加进提示词继续。
const MAX_ASK_ROUNDS: usize = 3;

async fn ai_generate_with_repair(
    app: &Option<tauri::AppHandle>,
    acc: &UsageAcc,
    cancel: &std::sync::atomic::AtomicBool,
    provider: &ai::models::AiProvider,
    model: &str,
    system: &str,
    user: &str,
    max_rounds: usize,
    project: Option<&super::scan::ProjectFiles>,
    run_id: &str,
) -> Result<(serde_json::Value, Vec<String>, usize), String> {
    let mut prompt = user.to_string();
    let mut best: Option<(serde_json::Value, Vec<String>, usize)> = None;
    let mut ask_calls = 0usize;
    let mut repair_calls = 0usize;
    loop {
        cancel_err(app, cancel)?;
        let json = call_ai_json(app, acc, cancel, provider, model, system, &prompt).await?;
        // 询问机制：AI 返回 ask（且未同时给 nodes）时，先让用户填表，再基于回答继续。
        // 询问不计入修复预算；询问次数封顶 MAX_ASK_ROUNDS，防止 AI 反复提问卡死。
        if let Some(ask_raw) = json.get("ask") {
            if ask_calls < MAX_ASK_ROUNDS && json.get("nodes").is_none() {
                ask_calls += 1;
                let ask = normalize_ask(ask_raw);
                emit_progress(app, "ask", serde_json::json!({
                    "ask": ask.clone(),
                    "round": ask_calls,
                    "max": MAX_ASK_ROUNDS,
                }));
                let (tx, rx) = tokio::sync::oneshot::channel();
                ask_register(run_id, tx);
                let answer = match rx.await {
                    Ok(v) => v,
                    // 发送端被丢弃（理论上仅出现在运行被强制终止时）：按未填写处理
                    Err(_) => serde_json::Value::Null,
                };
                if is_cancelled(cancel) {
                    emit_progress(app, "cancel", serde_json::json!({}));
                    return Err(ERR_CANCELLED.into());
                }
                let answer_txt = format_ask_answer(&ask, &answer);
                prompt = format!(
                    "{}\n\n—— 用户回答 ——\n{}\n请基于以上回答继续生成（只输出完整 JSON，不要解释或 Markdown）。",
                    user, answer_txt
                );
                continue;
            }
        }
        // 校验 / 修复
        repair_calls += 1;
        let errs = validate_ai_nodes_json(&json, project);
        let total_calls = ask_calls + repair_calls;
        if errs.is_empty() {
            return Ok((json, Vec::new(), total_calls));
        }
        let is_better = best
            .as_ref()
            .map(|(_, e, _)| errs.len() < e.len())
            .unwrap_or(true);
        if is_better {
            best = Some((json.clone(), errs.clone(), total_calls));
        }
        if repair_calls >= max_rounds {
            break;
        }
        prompt = format!(
            "{}\n\n—— 修复要求 ——\n你上一次输出的 JSON 未通过校验，请修正以下错误后重新输出完整 JSON（只输出 JSON，不要解释或 Markdown）：\n{}",
            user,
            errs.iter().map(|e| format!("- {}", e)).collect::<Vec<_>>().join("\n")
        );
    }
    let (json, errs, rounds) = best.unwrap_or_else(|| {
        (serde_json::json!({ "nodes": [] }), vec!["AI 未返回可用 JSON".into()], ask_calls + repair_calls)
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
    replace_existing: bool,
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
    // AI 导入始终追加一棵新的根树；追问修改模式只替换目标文档中的节点，不影响文档元数据。
    if replace_existing {
        super::db::with_conn(|c| {
            super::db::sql(c.execute("DELETE FROM mindmap_nodes WHERE document_id=?1", rusqlite::params![document_id]))?;
            Ok(())
        })?;
    }
    super::db::batch_save_nodes(&nodes)?;
    super::db::update_document(document_id, None, None, None)?;
    super::db::load_full(document_id)?.ok_or("加载失败".into())
}

// ─── 类型路由器（借鉴 Archify 的 type router） ───

const VIEW_LABELS: &[(&str, &str)] = &[
    ("architecture", "架构"),
    ("workflow", "业务流"),
    ("dataflow", "数据流"),
    ("sequence", "时序"),
    ("lifecycle", "生命周期"),
];

/// 产物深度等级（1 最浅 → 5 最深）对应的生成要求文案，拼进视图级 prompt。
/// 1：只列清单（项目/模块/文件各自的功能一句话）；5：完整业务流走向 + 分支判定方式。
fn depth_requirement(depth: u8) -> &'static str {
    match depth {
        1 => "深度要求（最浅）：只输出清单。每个子项目、每个模块、每个文件各占一个节点，detail 用一句话说明其功能；不要展开内部结构，不要分析业务流。节点总数宁少勿滥。",
        2 => "深度要求（较浅）：模块清单 + 每个模块的功能列表。模块下直接列它提供的功能/接口（每个节点 detail 一句话），不需要展开步骤与数据。",
        3 => "深度要求（中等）：模块清单 + 功能，且每个模块的 detail 中简述该模块的关键执行路径（输入→处理→输出）。",
        4 => "深度要求（较深）：在模块/功能之上，整理出主要业务流的完整走向（触发→步骤→分支→结果），每条业务流一个子树，分支处写明判定条件。",
        _ => "深度要求（最深）：完整还原业务流走向与判定方式。每条业务流一个子树：触发条件、逐步骤、每个分支/判断节点必须写明判定依据（什么条件下走哪条路）、异常与回退路径；模块清单退居其次，只为业务流中引用到的模块保留。",
    }
}

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

/// Wspólne wymogi tresciowe dla kazdego widoku: moduly, funkcje, przeplywy.
/// (Wymusza produkt, nie strukture katalogow.)
const VIEW_SUBSTANCE_REQ: &str = r##"
TREŚĆ (obowiązkowa, ważniejsza niż struktura katalogów):
- Każdy węzeł-moduł to PRAWDZIWY moduł, który zaistniał w przeczytanych plikach (nazwa z kodu/katalogu, nie wymyślona). W detail napisz, CO ten moduł robi — konkretna funkcjonalność, nie „zarządza danymi".
- Pod każdym modułem dodaj 2-4 węzły-funkcje/operacje biznesowe, które ten moduł realizuje (np. „rejestracja użytkownika", „generowanie faktury", „odświeżanie tokenu") — z pliku lub kodu, który o tym świadeczy.
- Jeśli z przeczytanych plików wynika przepływ biznesowy (kto → co → z czym → rezultat), dodaj podkorzeń „przepływ" lub opisz go w detail modułu: wejście, kroki, wynik. Nazwij realne funkcje/endpointy/tabele, które widziałeś.
- detail każdego modułu: co robi, na czym polega implementacja (biblioteka, wzorzec, endpoint, model danych), z czym się łączy. Czerp z treści plików, nie z domysłów.
- Zakaz pustych ogólników typu „warstwa logiki", „moduł pomocniczy" — jeśli nie wiesz, co moduł robi, powiedz to wprost w detail, zamiast zmyślać."##;

/// 询问机制说明：AI 遇到信息不足/歧义时优先向用户提问（而非臆造）。
/// 输出 {"ask": {...}} 时由后端弹出表单让用户填写，回答会回填后继续生成；
/// 每轮最多问 1 次、整个生成最多问 3 次（后端封顶），问完必须基于回答继续产出节点。
const ASK_REQ: &str = r##"

询问机制（重要）：当且仅当信息不足以可靠生成、且存在实质性歧义时（例如：无法确定模块边界/某个关键业务流走向、需求文本缺少关键约束、同一功能有多种合理解释），不要臆造，而是改输出一个询问对象（不要输出 nodes）：
{"ask":{"question":"你要问用户的问题（一句话，说清缺什么、为什么需要）","fields":[{"key":"字段键（英文蛇形）","label":"字段名","type":"text","options":[],"default":""}]}}
type 取值：text（单行）、textarea（多行）、select（单选，配 options 选项数组）。fields 最多 4 个，能一句话问清就别用字段（fields 留空 []）。
用户会填写后你再基于回答继续生成。注意：只有在真正卡住、且猜错会显著影响结果时才提问；能从上下文合理推断的就直接推断并在 detail 中说明依据，不要为了提问而提问。"##;

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
/// depth：产物深度（见 depth_requirement）；project 模式下拼进生成要求。
fn view_prompt(mode: &str, view: &str, depth: u8) -> String {
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
    // 深度要求仅项目模式生效（需求文本无“扫描产物”概念，保持原行为）
    let depth_req = if mode == "project" {
        format!("\n{}", depth_requirement(depth))
    } else {
        String::new()
    };
    let tpl = r##"{opener}
只允许输出一个 JSON 对象，不要 Markdown 代码围栏、解释文字或尾随逗号：
{{"summary":"该视角的简明概述","nodes":[{{"id":"唯一稳定短 ID","name":"节点名称","parent_id":null,"detail":"节点说明：一句话职责 + 具体功能，可使用 Markdown","kind":"{kinds}","color":"#RRGGBB","sources":["项目相对路径"]}}]}}
组织要求：{guidance}{evidence}{depth_req}
结构要求：节点字段与思维导图节点数据结构一一对应（id/name/parent_id/detail/kind/color/sources，其中 sources 可省略）；至少一个根节点，根节点 parent_id 必须为 null；其余节点只能通过 parent_id 引用本次输出中的 id；每个节点的 detail 必须写明该模块/节点的说明（职责、边界、与相邻模块的关系，可用 Markdown），不得为空；color 必须是 6 位十六进制颜色；节点总数控制在 {count} 个；只输出 JSON。{evidence_req}
{substance}{ask_req}"##;
    let evidence_req = if mode == "project" {
        "\n证据要求：关键模块/组件/服务节点用 sources 字段标注 1 到 3 个真实文件（项目相对路径，必须在『目录结构』中出现），文件/配置类节点标注自身路径；sources 最多 6 个，只填真实存在的路径，不要臆造。"
    } else {
        ""
    };
    let substance = if mode == "project" { VIEW_SUBSTANCE_REQ } else { "" };
    let ask_req = ASK_REQ;
    tpl.replace("{opener}", &opener)
        .replace("{kinds}", kinds)
        .replace("{count}", count)
        .replace("{guidance}", guidance)
        .replace("{evidence}", evidence)
        .replace("{depth_req}", &depth_req)
        .replace("{evidence_req}", evidence_req)
        .replace("{substance}", substance)
        .replace("{ask_req}", ask_req)
}

/// 多轮探索 prompt（system）：AI 请求要读取的文件批次；返回 done 表示探索结束。
fn explorer_prompt(mode: &str, max_files: usize) -> String {
    format!(
        r##"你是软件架构师，正在深入分析{subject}。每一轮你会收到：项目结构（目录树、技术栈、模块耦合等）以及上一轮请求的文件内容（已压缩）。
你的任务：决定下一步要读取哪些文件，以便理解项目真实的模块、模块内的功能与业务流程——而不仅是目录结构。每轮最多请求 {max_files} 个文件。
优先读取（按此顺序）：
1. 路由/入口/控制器 → 发现业务操作（endpoint = 业务功能）
2. 服务/用例层实现 → 每个功能的内部逻辑与步骤
3. 数据模型/迁移 → 实体与状态
4. 队列/定时任务/事件 → 异步业务流
目标：读完后你能列出真实模块清单、每个模块的具体功能、以及至少一条端到端业务流（谁→做什么→数据→结果）。
只输出一个 JSON 对象，不要 Markdown 或解释文字：
{{"paths":["src/foo.ts"],"dirs":["src/services"],"done":false,"reason":"为什么读这些文件（一句话）"}}
规则：
- paths 只能是『目录结构』中真实存在的文件（项目相对路径）；最多 {max_files} 个
- dirs 可以请求确认目录存在（不读内容）；最多 {max_files} 个
- done=true 表示探索结束（已读够，可以生成导图）
- 优先读取：入口文件、路由/服务定义、核心模块实现、配置文件
- 不要重复请求已读过的文件"##,
        subject = if mode == "project" { "一个项目" } else { "一段需求" },
        max_files = max_files,
    )
}

// 探索参数已开放为全局设置（mindmap_settings.json），见 super::settings::ExplorerSettings；
// 此处保留 use 便于后续扩展。
use super::settings::ExplorerSettings;

/// 多轮探索循环：AI 每轮请求一批文件，工具读取并压缩内容作为下一轮上下文。
/// 返回 (最终上下文, 实际探索轮数, 累计读取文件数, 每轮日志)。
async fn ai_explore_project(
    app: &Option<tauri::AppHandle>,
    acc: &UsageAcc,
    cancel: &std::sync::atomic::AtomicBool,
    provider: &ai::models::AiProvider,
    model: &str,
    initial_context: &str,
    project: &super::scan::ProjectFiles,
    cfg: &ExplorerSettings,
) -> Result<(String, usize, usize, Vec<AiExploreRound>), String> {
    let mut context = initial_context.to_string();
    let mut requested: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut rounds = 0usize;
    let mut files_read = 0usize;
    let mut round_log: Vec<AiExploreRound> = Vec::new();
    for _round in 0..cfg.explorer_rounds as usize {
        cancel_err(app, cancel)?;
        let req = call_ai_json_for_explorer(
            app,
            acc,
            cancel,
            provider,
            model,
            &explorer_prompt("project", cfg.explorer_files_per_round as usize),
            &context,
        )
        .await?;
        rounds += 1;
        let done = req.get("done").and_then(|v| v.as_bool()).unwrap_or(false);
        let reason = req.get("reason").and_then(|v| v.as_str()).unwrap_or("").to_string();
        // 解析顺序很重要：先按设置硬截断（防模型超量点单），再做去重标记——
        // 若先标记后截断，被截掉的文件会被误标为「已读」，下轮点它会遭到错误去重。
        let (files, dirs, rejected) = super::scan::parse_ai_file_request(&req, project);
        let max_n = cfg.explorer_files_per_round as usize;
        let (files, dirs) = if files.len() > max_n || dirs.len() > max_n {
            let mut f = files;
            f.truncate(max_n);
            let mut d = dirs;
            d.truncate(max_n);
            (f, d)
        } else {
            (files, dirs)
        };
        // 去重：只把「本轮真正会读」的文件记入已读集合
        let fresh: Vec<String> = files
            .into_iter()
            .filter(|f| requested.insert(f.clone()))
            .collect();
        files_read += fresh.len();
        // Progreso: qué pidió el AI esta ronda y por qué
        emit_progress(
            app,
            "explore",
            serde_json::json!({
                "round": rounds,
                "total": cfg.explorer_rounds as usize,
                "reason": reason,
                "done": done,
            }),
        );
        if !fresh.is_empty() {
            emit_progress(app, "read", serde_json::json!({ "files": fresh }));
        }
        if done || (fresh.is_empty() && dirs.is_empty()) {
            // 探索结束：done=true 时记录 AI 的收尾理由与最后一批请求（不再读取）
            if done && !fresh.is_empty() {
                round_log.push(AiExploreRound {
                    round: rounds,
                    reason: reason.clone(),
                    files: fresh.iter().cloned().collect(),
                    dirs: dirs.iter().cloned().collect(),
                    truncated: false,
                });
            }
            break;
        }
        // 读取并压缩这一批文件，作为下一轮上下文（结构 + 已读内容 + 无效路径回执）
        let mut batch: Vec<(String, Option<String>, bool)> = Vec::new();
        let mut batch_chars = 0usize;
        let mut batch_truncated = false;
        for rel in &fresh {
            let per_file = (cfg.explorer_chars_per_file as usize)
                .min((cfg.explorer_batch_chars as usize).saturating_sub(batch_chars));
            if per_file == 0 {
                break;
            }
            let (content, truncated) = super::scan::read_file_compressed(&project.root, rel, per_file);
            batch_truncated |= truncated;
            batch_chars += content.as_ref().map(|t| t.chars().count()).unwrap_or(0);
            batch.push((rel.clone(), content, truncated));
        }
        // 回执：模型点了但不存在/无效的路径明确反馈（含 done 收尾轮，否则模型可能
        // 反复点同一批不存在的文件白白烧轮次）。
        if !rejected.is_empty() {
            emit_progress(
                app,
                "reject",
                serde_json::json!({ "round": rounds, "paths": rejected }),
            );
        }
        context = format!(
            "{}\n\n{}",
            context,
            super::scan::format_file_batch(&batch, &dirs, &rejected)
        );
        // 本轮记录：理由 + 实际读取清单（供导入报告展示探索过程）
        round_log.push(AiExploreRound {
            round: rounds,
            reason: reason.clone(),
            files: fresh.iter().cloned().collect(),
            dirs: dirs.iter().cloned().collect(),
            truncated: batch_truncated,
        });
    }
    Ok((context, rounds, files_read, round_log))
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
    app: &Option<tauri::AppHandle>,
    cancel: &std::sync::atomic::AtomicBool,
    depth: u8,
    requested_views: &[String],
    run_id: &str,
    target_document_id: Option<&str>,
    replace_existing: bool,
) -> Result<AiImportResult, String> {
    // 本次运行的总 token 累计器（逐请求记录 + 完成时随报告返回、随文档留痕）
    let usage = UsageAcc::default();
    // 项目模式：文件集上下文（证据锚定校验用：存在性 + 内容相关度）
    let project_files = match project_root {
        Some(p) => match super::scan::collect_project_files(p) {
            Ok(f) => Some(f),
            Err(_) => None, // 文件集失败不阻断生成，只是跳过证据校验
        },
        None => None,
    };
    // 多轮探索（仅项目模式）：AI 每轮请求一批文件，工具读取并压缩内容追加到上下文。
    // 探索失败不阻断生成，回退为纯结构扫描上下文；但会推送 fail 事件——否则前端
    // 只看到 AI 停在「扫描完成」很久没有动静，不知道探索已失败、正在走降级路径。
    let mut context = context.to_string();
    let mut exploration: Vec<AiExploreRound> = Vec::new();
    // 探索预算来自全局设置（每轮点单上限随 prompt 一起告知 AI）
    let explorer_cfg = super::settings::load_explorer_settings();
    if let Some(pf) = &project_files {
        match ai_explore_project(
            app,
            &usage,
            cancel,
            provider,
            model,
            &context,
            pf,
            &explorer_cfg,
        )
        .await
        {
            Ok((enriched, _rounds, files_read, round_log)) => {
                context = enriched;
                exploration = round_log;
                let _ = files_read; // 统计信息目前仅用于日志/调试
            }
            Err(e) => {
                log_ai_transport_failure(app, model, &format!("探索阶段失败（回退为纯结构扫描继续）: {}", e));
            }
        }
    }
    let context = context.as_str();
    // 1. 确定要生成的视图：用户显式勾选时直接采用（跳过 AI 路由，省一次请求）；
    // 未勾选时走 AI 类型路由判断，失败回退单架构视图（推送 fail 事件告知前端降级原因）。
    let mut views: Vec<String> = Vec::new();
    if !requested_views.is_empty() {
        // 过滤为合法视图名并保持用户勾选顺序
        let mut seen = std::collections::HashSet::new();
        views = requested_views
            .iter()
            .filter(|v| VIEW_LABELS.iter().any(|(k, _)| k == *v))
            .filter(|v| seen.insert((*v).clone()))
            .cloned()
            .collect();
        emit_progress(app, "route", serde_json::json!({ "views": views }));
    }
    if views.is_empty() {
        emit_progress(app, "route", serde_json::json!({}));
        views = vec!["architecture".to_string()];
        match call_ai_json(app, &usage, cancel, provider, model, &router_prompt(mode), context).await {
            Ok(router_json) => {
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
        Err(e) => {
            // 取消不算降级（run 已被用户终止，后续 cancel 检查会统一处理）；
            // 其余失败推送 fail 并走单「架构」视图回退，前端进度面板有交代。
            if !is_cancelled(cancel) {
                log_ai_transport_failure(app, model, &format!("视图路由失败（回退为单架构视图）: {}", e));
            }
        }
        }
    }
    emit_progress(app, "route", serde_json::json!({ "views": views }));

    // 2. 逐视图生成：各建独立文档；失败清理空文档并记录原因
    let mut docs: Vec<DocumentFull> = Vec::new();
    let mut failures: Vec<AiImportFailure> = Vec::new();
    let mut reports: Vec<AiImportReport> = Vec::new();
    let mut primary: Option<String> = None;
    for view in &views {
        cancel_err(app, cancel)?;
        let doc_name = format!("{} · {}", root_name, view_label(view));
        emit_progress(app, "view", serde_json::json!({ "view": view }));
        let (doc, owns_doc) = if primary.is_none() {
            if let Some(target_id) = target_document_id {
                let target = super::db::load_full(target_id)?.ok_or("目标思维导图不存在")?;
                (target.document, false)
            } else {
                (super::db::create_document(
                    &doc_name,
                    "",
                    if mode == "project" { "ai_project" } else { "ai_text" },
                    None,
                )?, true)
            }
        } else {
            (super::db::create_document(
                &doc_name,
                "",
                if mode == "project" { "ai_project" } else { "ai_text" },
                None,
            )?, true)
        };
        let user = if mode == "project" {
            context.to_string()
        } else {
            format!("分析以下文字提取结构化需求：\n\n{}", context)
        };
        let user = if replace_existing && primary.is_none() {
            let existing = super::db::load_full(&doc.id)?.map(|f| {
                f.nodes.iter().map(|n| format!("- {} [{}]：{}", n.name, n.kind, n.detail.chars().take(300).collect::<String>())).collect::<Vec<_>>().join("\n")
            }).unwrap_or_default();
            format!("{}\n\n当前目标思维导图已有节点（请根据用户指令修改，输出修改后的完整节点树；不需要保留被删除的节点）：\n{}", user, existing)
        } else {
            user
        };
        // 项目模式：记录项目根路径到文档 source_desc，供证据文件定位
        if let Some(p) = project_root {
            let _ = super::db::update_source_desc(&doc.id, p);
        }
        // 该视图开始前的用量快照：diff 出「本视图独占」的请求/token（探索/路由等公共部分不计入）
        let view_base = usage.snapshot();
        let outcome = async {
            let (parsed, errs, rounds) = ai_generate_with_repair(
                app,
                &usage,
                cancel,
                provider,
                model,
                &view_prompt(mode, view, depth),
                &user,
                3,
                project_files.as_ref(),
                run_id,
            )
            .await?;
            Ok::<_, String>((parsed, errs, rounds))
        }
        .await;
        // 生成中途被取消：清理半成品文档并整体中止（不是单个视图失败，不继续下一个视图）
        if is_cancelled(cancel) {
            let _ = if owns_doc { super::db::delete_document(&doc.id) } else { Ok(()) };
            emit_progress(app, "cancel", serde_json::json!({}));
            return Err(ERR_CANCELLED.into());
        }
        let (parsed, errs, rounds) = match outcome {
            Ok(x) => x,
            Err(e) => {
                let _ = if owns_doc { super::db::delete_document(&doc.id) } else { Ok(()) }; // 清理空文档
                emit_progress(app, "fail", serde_json::json!({ "detail": format!("{}: {}", view_label(view), e) }));
                failures.push(AiImportFailure { view: view.clone(), reason: e });
                continue;
            }
        };
        if rounds > 1 {
            emit_progress(app, "repair", serde_json::json!({ "count": errs.len(), "rounds": rounds }));
        }
        let replacing_target = replace_existing && primary.is_none();
        match import_ai_nodes(&doc.id, parsed, errs.clone(), &doc_name, replacing_target).await {
            Ok(mut full) => {
                if primary.is_none() {
                    primary = Some(full.document.id.clone());
                }
                // 本视图 token 留痕：写入文档累计统计，并同步到返回对象让报告弹窗立即可见
                let view_usage = usage.diff(&view_base);
                if view_usage.requests > 0 {
                    let _ = super::db::add_ai_usage(
                        &full.document.id,
                        view_usage.input_tokens,
                        view_usage.output_tokens,
                    );
                    full.document.ai_imports += 1;
                    full.document.ai_input_tokens += view_usage.input_tokens as i64;
                    full.document.ai_output_tokens += view_usage.output_tokens as i64;
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
                    usage: view_usage,
                });
                docs.push(full);
                // 视图落库即广播：前端可「边生成边绘制」，不必等全部视图结束。
                // 携带文档 id + 视图名，前端按需拉取 DocumentFull 增量渲染。
                emit_progress(
                    app,
                    "view_done",
                    serde_json::json!({ "doc_id": doc.id, "view": view, "index": docs.len() - 1 }),
                );
            }
            Err(e) => {
                let _ = if owns_doc { super::db::delete_document(&doc.id) } else { Ok(()) };
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
        usage: usage.snapshot(),
        exploration,
    })
}

/// 取消指定 run_id 的 AI 导入运行（前端点「停止」时调用；各循环/流式块边界会检查标志并中断）。
/// 若运行正阻塞在「询问用户」等待上，同时向询问通道发送取消标记解除阻塞，避免卡死。
#[tauri::command]
pub fn mm_ai_cancel(run_id: String) -> Result<(), String> {
    if run_id.trim().is_empty() { return Err("run_id 为空".into()); }
    ai_cancel_flag(&run_id).store(true, std::sync::atomic::Ordering::Relaxed);
    ask_send_cancel(&run_id);
    Ok(())
}

/// 回填 AI 询问的用户答案（前端 AgentWorkbench 的询问表单提交时调用）。
/// answer 为 JSON 对象（字段键→值）或字符串；后端把回答追加进提示词后继续生成。
#[tauri::command]
pub fn mm_ai_answer(run_id: String, answer: serde_json::Value) -> Result<(), String> {
    if run_id.trim().is_empty() { return Err("run_id 为空".into()); }
    ask_send_answer(&run_id, answer)
}

#[tauri::command]
pub async fn mm_ai_from_project(app: tauri::AppHandle, input: AiGenerateProjectInput) -> Result<AiImportResult, String> {
    let pp = input.project_path.trim().to_string();
    if pp.is_empty() { return Err("路径为空".into()); }
    let pname = std::path::Path::new(&pp).file_name().and_then(|n| n.to_str()).unwrap_or("项目").to_string();
    let app_opt = Some(app);
    // 本次运行的取消标志：按 run_id 独立，前后端共用同一标识
    let run_id = if input.run_id.trim().is_empty() { "import".to_string() } else { input.run_id.clone() };
    let cancel = ai_cancel_flag(&run_id);
    let result = async {
        emit_progress(&app_opt, "scan", serde_json::json!({}));
        // 扫描（含技术栈/目录规模/标记等仓库证据）
        let context = super::scan::scan_project_with_hint(&pp, input.user_hint.as_deref())?;
        emit_progress(&app_opt, "scan", serde_json::json!({ "done": true }));
        let (provider, model) = resolve_provider_model(&input.provider_id, &input.model_id)?;
        run_ai_router(&provider, &model, "project", &context, &pname, Some(&pp), &app_opt, &cancel, input.depth.clamp(1, 5), &input.views, &run_id, Some(&input.document_id), input.replace_existing).await
    }
    .await;
    ai_drop_flag(&run_id);
    result
}

#[tauri::command]
pub async fn mm_ai_from_text(app: tauri::AppHandle, input: AiGenerateTextInput) -> Result<AiImportResult, String> {
    let text = input.text.trim().to_string();
    if text.is_empty() { return Err("文本为空".into()); }
    let title = if input.title.trim().is_empty() { "需求分析" } else { input.title.trim() };
    let (provider, model) = resolve_provider_model(&input.provider_id, &input.model_id)?;
    let app_opt = Some(app);
    let run_id = if input.run_id.trim().is_empty() { "import".to_string() } else { input.run_id.clone() };
    let cancel = ai_cancel_flag(&run_id);
    let result = run_ai_router(&provider, &model, "text", &text, title, None, &app_opt, &cancel, 3, &[], &run_id, Some(&input.document_id), input.replace_existing).await;
    ai_drop_flag(&run_id);
    result
}

// ─── 思维导图 Agent（右栏对话）───
//
// 与导入流程共用传输通道（ai::channel）与进度/取消/问答基础设施；区别在于：
// 导入是「单轮 JSON 出口」，Agent 是「多轮工具循环」——模型通过 tools 读写导图。
// 写操作后端只裁决不执行：ops 以事件发给前端，由前端走既有写路径应用
// （撤销快照 / 画布刷新 / 持久化保持单一来源），后端阻塞等待前端回填结果。

/// 单轮对话的 LLM 调用轮数上限（含读工具轮），防止循环失控。
const AGENT_MAX_ROUNDS: usize = 8;
/// 单批写 ops 的节点数上限。
const AGENT_MAX_OPS: usize = 50;
/// 会话历史的字符预算（粗略 4 字符 ≈ 1 token，只求「不会无限膨胀」，精确计数交给网关）。
const AGENT_HISTORY_CHAR_BUDGET: usize = 24_000;
/// 等待前端应用/确认 ops 的超时：确认清单可能要等人，放宽到 10 分钟。
const AGENT_OPS_WAIT_SECS: u64 = 600;

/// Agent 工具集（OpenAI tools 格式）。读工具后端直接执行；写工具构建 ops 交前端应用。
pub const AGENT_TOOLS_SPEC: &str = r##"[
  { "type": "function", "function": { "name": "get_document_overview", "description": "获取当前思维导图的大纲（每个节点的 id、父节点、名称），用于了解整体结构。", "parameters": { "type": "object", "properties": {} } } },
  { "type": "function", "function": { "name": "get_subtree", "description": "读取某个节点及其全部子孙的完整内容（名称、详情 Markdown、类型、颜色）。", "parameters": { "type": "object", "properties": { "root_id": { "type": "string", "description": "子树根节点 id" } }, "required": ["root_id"] } } },
  { "type": "function", "function": { "name": "search_nodes", "description": "按关键词搜索节点（匹配名称与详情），返回匹配节点的 id 与名称。", "parameters": { "type": "object", "properties": { "keyword": { "type": "string" } }, "required": ["keyword"] } } },
  { "type": "function", "function": { "name": "add_nodes", "description": "在指定父节点下新增一批兄弟节点。新节点不需要提供 id，系统会生成并在工具结果里返回。要建整棵子树时，按层级多次调用（先挂父，再以返回的 id 为父挂子）。分析某个文件后落地为导图时，用 sources 把来源文件锚到节点上。", "parameters": { "type": "object", "properties": { "parent_id": { "type": "string", "description": "父节点 id（必须是大纲中真实存在的 id）" }, "nodes": { "type": "array", "items": { "type": "object", "properties": { "name": { "type": "string", "description": "节点名称（必填）" }, "detail": { "type": "string", "description": "节点说明，Markdown" }, "kind": { "type": "string", "description": "节点类型" }, "color": { "type": "string", "description": "#RRGGBB" }, "sources": { "type": "array", "items": { "type": "string" }, "description": "该节点对应的真实源码文件（项目相对路径或绝对路径），用于证据锚定" } }, "required": ["name"] } } }, "required": ["parent_id", "nodes"] } } },
  { "type": "function", "function": { "name": "update_nodes", "description": "批量修改已有节点字段（只传需要修改的字段）。", "parameters": { "type": "object", "properties": { "updates": { "type": "array", "items": { "type": "object", "properties": { "id": { "type": "string" }, "name": { "type": "string" }, "detail": { "type": "string" }, "color": { "type": "string" } }, "required": ["id"] } } }, "required": ["updates"] } } },
  { "type": "function", "function": { "name": "delete_nodes", "description": "删除节点及其整棵子树。破坏性操作：用户会先看到确认清单，可能拒绝。", "parameters": { "type": "object", "properties": { "ids": { "type": "array", "items": { "type": "string" } } }, "required": ["ids"] } } },
  { "type": "function", "function": { "name": "move_nodes", "description": "把节点移动/重新挂到另一个父节点下。会改变导图结构：用户会先看到确认清单，可能拒绝。", "parameters": { "type": "object", "properties": { "ids": { "type": "array", "items": { "type": "string" } }, "new_parent_id": { "type": "string" } }, "required": ["ids", "new_parent_id"] } } }
]"##;

/// 写 op 的分级：新增/编辑直接生效（Ctrl+Z 可撤销），删除/移动必须经用户确认。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentOpClass {
    /// 直接应用
    Auto,
 /// 进入右栏待确认清单
    Confirm,
}

fn agent_op_class(action: &str) -> AgentOpClass {
    match action {
        "delete" | "move" => AgentOpClass::Confirm,
        _ => AgentOpClass::Auto,
    }
}

/// 校验 AI 给的颜色为 #RRGGBB；不合法返回 None（调用方省略该字段，画布用默认色）。
fn agent_valid_color(color: Option<&str>) -> Option<String> {
    let hex = color?.trim().strip_prefix('#')?;
    if hex.len() == 6 && hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        Some(format!("#{}", hex.to_ascii_lowercase()))
    } else {
        None
    }
}

/// Agent 系统提示词：角色 + 工具使用规则 + 字段约束。
fn agent_prompt(doc_name: &str) -> String {
    format!(
        r#"你是思维导图「{name}」的智能体助手，工作在导图应用的右侧对话栏里。用户会要求你分析、修改或重组这张导图。

规则：
1. 涉及导图内容时优先使用工具；修改前先用 get_document_overview 了解结构，必要时 get_subtree / search_nodes 确认细节。引用的 id 必须来自工具返回结果，不要臆造。
2. add_nodes 的新节点不要提供 id，系统会生成并在工具结果里返回；一次调用挂同一父节点下的一批兄弟节点，建子树时按层级多次调用。
3. kind 取值：root|module|component|service|route|config|file|task|requirement|constraint|risk|other。color 是 #RRGGBB。
4. delete_nodes / move_nodes 需要用户在界面上确认；如果被拒绝，不要原样重复提交，先询问顾虑或给出替代方案。
5. 不改图的分析（总结、找重复与缺口、回答问题）直接回答，引用节点名称。
6. 用户用 @ 引用的文件会以「用户引用的文件内容」附在消息里。要求分析某个文件的业务逻辑时：先通读给出的内容，再按「入口/流程/分支/关键数据/边界与异常」组织成节点落到图上，并用 add_nodes 的 sources 把被分析的文件锚到相关节点（项目相对路径或绝对路径均可）。
7. 全程用中文，简洁，可用 Markdown。"#
        , name = doc_name)
}

/// 从「最新往回」收集会话历史直到字符预算用尽，返回保持时间正序的 (role, content)。
/// 空内容的 assistant 行（纯 ops 载荷）不进上下文——ops 已经体现在用户可见的
/// 后续对话里，回放进提示词只会重复占预算。
fn agent_history_window(history: &[AgentMessageRow], budget: usize) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut left = budget;
    for m in history.iter().rev() {
        if (m.role != "user" && m.role != "assistant") || m.content.trim().is_empty() {
            continue;
        }
        let cost = m.content.chars().count() + 8;
        if cost > left {
            break;
        }
        left -= cost;
        out.push((m.role.clone(), m.content.clone()));
    }
    out.reverse();
    out
}

/// 节点 id 及其全部子孙 id（含自身）。带已访问去重：脏数据成环时不会死循环。
fn agent_descendant_ids(nodes: &[MindmapNode], root_id: &str) -> Vec<String> {
    let mut out = vec![root_id.to_string()];
    let mut i = 0;
    while i < out.len() {
        let cur = out[i].clone();
        i += 1;
        for n in nodes {
            if n.parent_id.as_deref() == Some(cur.as_str()) && !out.iter().any(|x| x == &n.id) {
                out.push(n.id.clone());
            }
        }
    }
    out
}

fn agent_node_brief(n: &MindmapNode, with_detail: bool) -> serde_json::Value {
    let mut v = serde_json::json!({ "id": n.id, "parentId": n.parent_id, "name": n.name, "kind": n.kind });
    if with_detail {
        v["detail"] = serde_json::json!(n.detail);
        v["color"] = serde_json::json!(n.color);
    }
    v
}

/// 读工具：文档大纲（压缩表示，一行一个节点）。
fn agent_tool_overview(full: &DocumentFull) -> serde_json::Value {
    let mut outline = String::from("id | parent_id | name\n");
    for n in &full.nodes {
        outline.push_str(&format!(
            "{} | {} | {}\n",
            n.id,
            n.parent_id.as_deref().unwrap_or("-"),
            n.name
        ));
    }
    serde_json::json!({ "document": full.document.name, "total": full.nodes.len(), "outline": outline })
}

/// 读工具：子树全文。
fn agent_tool_subtree(full: &DocumentFull, root_id: &str) -> serde_json::Value {
    if !full.nodes.iter().any(|n| n.id == root_id) {
        return serde_json::json!({ "error": "节点不存在", "rootId": root_id });
    }
    let ids = agent_descendant_ids(&full.nodes, root_id);
    let nodes: Vec<serde_json::Value> = full
        .nodes
        .iter()
        .filter(|n| ids.iter().any(|i| i == &n.id))
        .map(|n| agent_node_brief(n, true))
        .collect();
    serde_json::json!({ "rootId": root_id, "count": nodes.len(), "nodes": nodes })
}

/// 读工具：关键词搜索。
fn agent_tool_search(full: &DocumentFull, keyword: &str) -> serde_json::Value {
    let kw = keyword.trim().to_lowercase();
    if kw.is_empty() {
        return serde_json::json!({ "matches": [] });
    }
    let matches: Vec<serde_json::Value> = full
        .nodes
        .iter()
        .filter(|n| n.name.to_lowercase().contains(&kw) || n.detail.to_lowercase().contains(&kw))
        .take(30)
        .map(|n| agent_node_brief(n, false))
        .collect();
    serde_json::json!({ "matches": matches, "total": full.nodes.len() })
}

/// 把一次写工具调用归一化为 ops 数组（每个受影响节点一条，camelCase 直达前端）。
fn agent_build_ops(action: &str, args: &serde_json::Value) -> Result<Vec<serde_json::Value>, String> {
    let mut ops: Vec<serde_json::Value> = Vec::new();
    match action {
        "add_nodes" => {
            let parent = args.get("parentId").and_then(|v| v.as_str()).unwrap_or_default().trim().to_string();
            let nodes = args.get("nodes").and_then(|v| v.as_array()).ok_or("add_nodes 缺少 nodes")?;
            for n in nodes.iter().take(AGENT_MAX_OPS) {
                let name = n.get("name").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
                if name.is_empty() {
                    continue;
                }
                ops.push(serde_json::json!({
                    "action": "add",
                    "id": super::db::new_id("agent"),
                    "parentId": parent,
                    "name": name,
                    "detail": n.get("detail").and_then(|v| v.as_str()).unwrap_or(""),
                    "kind": n.get("kind").and_then(|v| v.as_str()).unwrap_or("other"),
                    "color": agent_valid_color(n.get("color").and_then(|v| v.as_str())),
                    // 证据锚定：分析文件后生成的节点带着来源文件（最多 3 个，与节点上限一致）
                    "sources": n.get("sources").and_then(|v| v.as_array())
                        .map(|arr| arr.iter().filter_map(|s| s.as_str()).filter(|s| !s.trim().is_empty()).take(3).map(|s| s.trim().to_string()).collect::<Vec<_>>())
                        .unwrap_or_default(),
                }));
            }
            if ops.is_empty() {
                return Err("add_nodes 没有有效节点（name 不能为空）".into());
            }
        }
        "update_nodes" => {
            for u in args.get("updates").and_then(|v| v.as_array()).ok_or("update_nodes 缺少 updates")?.iter().take(AGENT_MAX_OPS) {
                let id = u.get("id").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
                if id.is_empty() {
                    continue;
                }
                let mut op = serde_json::json!({ "action": "update", "id": id });
                for (key, field) in [("name", "name"), ("detail", "detail")] {
                    if let Some(v) = u.get(key).and_then(|v| v.as_str()) {
                        op[field] = serde_json::json!(v);
                    }
                }
                if let Some(c) = agent_valid_color(u.get("color").and_then(|v| v.as_str())) {
                    op["color"] = serde_json::json!(c);
                }
                if op.get("name").is_none() && op.get("detail").is_none() && op.get("color").is_none() {
                    continue;
                }
                ops.push(op);
            }
            if ops.is_empty() {
                return Err("update_nodes 没有有效更新".into());
            }
        }
        "delete_nodes" => {
            for id in args.get("ids").and_then(|v| v.as_array()).ok_or("delete_nodes 缺少 ids")?.iter().take(AGENT_MAX_OPS) {
                let id = id.as_str().unwrap_or("").trim();
                if !id.is_empty() {
                    ops.push(serde_json::json!({ "action": "delete", "id": id }));
                }
            }
            if ops.is_empty() {
                return Err("delete_nodes 没有有效 id".into());
            }
        }
        "move_nodes" => {
            let parent = args.get("newParentId").and_then(|v| v.as_str()).unwrap_or_default().trim().to_string();
            if parent.is_empty() {
                return Err("move_nodes 缺少 newParentId".into());
            }
            for id in args.get("ids").and_then(|v| v.as_array()).ok_or("move_nodes 缺少 ids")?.iter().take(AGENT_MAX_OPS) {
                let id = id.as_str().unwrap_or("").trim();
                if !id.is_empty() {
                    ops.push(serde_json::json!({ "action": "move", "id": id, "parentId": parent }));
                }
            }
            if ops.is_empty() {
                return Err("move_nodes 没有有效 id".into());
            }
        }
        other => return Err(format!("未知写工具 {}", other)),
    }
    Ok(ops)
}

/// 写工具处理：构建 ops → 防环校验 → 事件发给前端 → 阻塞等待应用/确认结果。
/// 返回 (全部 ops, 给模型的工具结果)。回填复用问答通道（mm_ai_answer）：
/// 前端把应用/确认结果作为 answer 发回，取消时通道收到 Null。
async fn agent_handle_write(
    app: &Option<tauri::AppHandle>,
    cancel: &std::sync::atomic::AtomicBool,
    full: &DocumentFull,
    run_id: &str,
    name: &str,
    args: &serde_json::Value,
) -> Result<(Vec<serde_json::Value>, serde_json::Value), String> {
    let ops = agent_build_ops(name, args)?;
    // 防环：move 的目标父节点不能是被移动节点自身或其后代
    for op in ops.iter().filter(|o| o.get("action").and_then(|v| v.as_str()) == Some("move")) {
        let id = op.get("id").and_then(|v| v.as_str()).unwrap_or_default();
        let new_parent = op.get("parentId").and_then(|v| v.as_str()).unwrap_or_default();
        if agent_descendant_ids(&full.nodes, id).iter().any(|x| x == new_parent) {
            return Err("不能把节点移动到它自身或它的子孙节点下".into());
        }
    }
    let need_confirm = ops
        .iter()
        .any(|o| o.get("action").and_then(|v| v.as_str()).map(agent_op_class) == Some(AgentOpClass::Confirm));

    let (tx, rx) = tokio::sync::oneshot::channel::<serde_json::Value>();
    emit_progress(app, "agentOps", serde_json::json!({
        "runId": run_id,
        "needConfirm": need_confirm,
        "ops": ops,
    }));
    ask_register(run_id, tx);
    let ack = match tokio::time::timeout(std::time::Duration::from_secs(AGENT_OPS_WAIT_SECS), rx).await {
        Ok(Ok(v)) => v,
        Ok(Err(_)) => serde_json::Value::Null,
        Err(_) => {
            ask_send_cancel(run_id);
            return Err("等待前端应用变更超时".into());
        }
    };
    if ack.is_null() {
        // mm_ai_cancel 会向问答通道发 Null：任务被用户停止
        cancel_err(app, cancel)?;
        return Err(ERR_CANCELLED.into());
    }
    let status = ack.get("status").and_then(|v| v.as_str()).unwrap_or("applied").to_string();
    Ok((ops.clone(), serde_json::json!({
        "status": status,
        "ops": ops,
        "note": ack.get("note").cloned().unwrap_or(serde_json::Value::Null),
    })))
}

/// 一轮 Agent 对话的产物。
struct AgentTurn {
    reply: String,
    rounds: usize,
}

/// Agent 工具循环：LLM ↔ 工具（读直执 / 写经前端），直到给出最终回答或轮数耗尽。
/// `messages[0]` 必须是 system。返回最终回复与本轮实际提交的 ops（用于落库）。
#[allow(clippy::too_many_arguments)]
async fn agent_run(
    app: &Option<tauri::AppHandle>,
    cancel: &std::sync::atomic::AtomicBool,
    acc: &UsageAcc,
    provider: &ai::models::AiProvider,
    model: &str,
    full: &DocumentFull,
    session_id: &str,
    run_id: &str,
    messages: &mut Vec<serde_json::Value>,
) -> Result<AgentTurn, String> {
    let hooks = MmHooks { app, cancel };
    let mut all_ops: Vec<serde_json::Value> = Vec::new();
    for round in 0..AGENT_MAX_ROUNDS {
        cancel_err(app, cancel)?;
        let outcome = ai::channel::complete_chat_messages(&hooks, provider, model, messages, 0.4, Some(AGENT_TOOLS_SPEC))
            .await
            .map_err(|e| {
                if e.contains("tool") && e.contains("400") {
                    format!("{}（当前模型/网关可能不支持工具调用，请更换模型）", e)
                } else {
                    e
                }
            })?;
        if let Some(u) = &outcome.usage {
            record_and_emit_usage(app, acc, model, &provider.id, u);
        }
        let message = outcome.message.clone().ok_or("AI响应缺少 message")?;
        let tool_calls = message
            .get("tool_calls")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if tool_calls.is_empty() {
            let reply = outcome.text.trim().to_string();
            if reply.is_empty() {
                return Err("AI返回空".into());
            }
            let _ = super::db::agent_append_message(session_id, "assistant", &reply, "[]");
            return Ok(AgentTurn { reply, rounds: round + 1 });
        }
        // assistant(tool_calls) 原样回传（OpenAI 协议要求带上它才能接 tool 消息）
        messages.push(message.clone());
        let mut tool_results: Vec<serde_json::Value> = Vec::new();
        for tc in &tool_calls {
            let call_id = tc.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string();
            let name = tc
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let raw_args = tc.get("function").and_then(|f| f.get("arguments")).cloned();
            let args = match raw_args {
                Some(serde_json::Value::String(s)) => {
                    serde_json::from_str::<serde_json::Value>(s.trim()).unwrap_or(serde_json::json!({}))
                }
                Some(v @ serde_json::Value::Object(_)) => v,
                _ => serde_json::json!({}),
            };
            let result: serde_json::Value = match name.as_str() {
                "get_document_overview" => agent_tool_overview(full),
                "get_subtree" => agent_tool_subtree(full, args.get("rootId").and_then(|v| v.as_str()).unwrap_or_else(|| args.get("root_id").and_then(|v| v.as_str()).unwrap_or(""))),
                "search_nodes" => agent_tool_search(full, args.get("keyword").and_then(|v| v.as_str()).unwrap_or("")),
                "add_nodes" | "update_nodes" | "delete_nodes" | "move_nodes" => {
                    match agent_handle_write(app, cancel, full, run_id, &name, &args).await {
                        Ok((ops, ack)) => {
                            all_ops.extend(ops.iter().cloned());
                            // 纯 ops 载荷落库：回放时展示，不重放执行
                            let _ = super::db::agent_append_message(session_id, "assistant", "", &serde_json::to_string(&ops).unwrap_or_else(|_| "[]".into()));
                            ack
                        }
                        Err(e) => serde_json::json!({ "status": "error", "error": e }),
                    }
                }
                other => serde_json::json!({ "status": "error", "error": format!("未知工具 {}", other) }),
            };
            tool_results.push(serde_json::json!({
                "role": "tool",
                "tool_call_id": call_id,
                "content": result.to_string(),
            }));
        }
        messages.extend(tool_results);
    }
    Err(format!("Agent 连续 {} 轮未给出最终回答，已停止", AGENT_MAX_ROUNDS))
}

/// Agent 一轮对话入口：确保会话 → 构建上下文与历史 → 跑工具循环 → 落库并返回。
#[tauri::command]
pub async fn mm_agent_chat(app: tauri::AppHandle, input: AgentChatInput) -> Result<AgentChatResult, String> {
    let app_opt = Some(app);
    let user_text = input.message.trim().to_string();
    let run_id = if input.run_id.trim().is_empty() { "agent".to_string() } else { input.run_id.clone() };
    let cancel = ai_cancel_flag(&run_id);
    let result = async {
        let full = super::db::load_full(&input.document_id)?.ok_or("文档不存在")?;
        let (provider, model) = resolve_provider_model(&input.provider_id, &input.model_id)?;
        let session_id = if input.session_id.trim().is_empty() {
            super::db::agent_ensure_session(&input.document_id)?
        } else {
            input.session_id.clone()
        };

        // 上下文：system + 历史（按预算裁剪）+ 大纲/选中子树 + 本轮用户输入
        let history = super::db::agent_list_messages(&session_id)?;
        let mut messages = vec![serde_json::json!({ "role": "system", "content": agent_prompt(&full.document.name) })];
        let mut insert_at = 1usize;
        for (role, content) in agent_history_window(&history, AGENT_HISTORY_CHAR_BUDGET) {
            messages.insert(insert_at, serde_json::json!({ "role": role, "content": content }));
            insert_at += 1;
        }
        let selected = if input.selected_node_ids.is_empty() {
            String::from("当前没有选中节点。")
        } else {
            let mut parts = Vec::new();
            for id in input.selected_node_ids.iter().take(3) {
                if full.nodes.iter().any(|n| n.id == *id) {
                    parts.push(agent_tool_subtree(&full, id).to_string());
                }
            }
            if parts.is_empty() {
                String::from("当前没有选中节点。")
            } else {
                format!("用户当前选中的节点（完整子树）：\n{}", parts.join("\n"))
            }
        };
        let user_content = if user_text.is_empty() {
            format!("（用户点击了继续）\n\n{}", selected)
        } else {
            format!("{}\n\n{}", user_text, selected)
        };
        // @ 引用的文件：内容进上下文（模型在正文里也能看到 @路径 引用）
        let attachments = agent_attachments_block(full.document.project_dir.as_deref(), &input.attached_files);
        let user_content = if attachments.is_empty() {
            user_content
        } else {
            format!("{}\n\n用户引用的文件内容：{}", user_content, attachments)
        };
        messages.push(serde_json::json!({ "role": "user", "content": user_content }));

        // 空输入（「继续」）不重复落一条空用户消息
        if !user_text.is_empty() {
            super::db::agent_append_message(&session_id, "user", &user_text, "[]")?;
        }

        let usage = UsageAcc::default();
        let turn = agent_run(&app_opt, &cancel, &usage, &provider, &model, &full, &session_id, &run_id, &mut messages).await?;
        let u = usage.snapshot();
        if u.requests > 0 {
            let _ = super::db::add_ai_usage(&input.document_id, u.input_tokens, u.output_tokens);
        }
        Ok(AgentChatResult { session_id, reply: turn.reply, rounds: turn.rounds as u32 })
    }
    .await;
    ai_drop_flag(&run_id);
    result
}

/// 取文档的 Agent 会话 id（不存在则新建）。
#[tauri::command]
pub fn mm_agent_get_session(document_id: String) -> Result<String, String> {
    super::db::agent_ensure_session(&document_id)
}

/// 列出会话消息（按时间正序；前端回放右栏对话）。
#[tauri::command]
pub fn mm_agent_list_messages(session_id: String) -> Result<Vec<AgentMessageRow>, String> {
    super::db::agent_list_messages(&session_id)
}

#[cfg(test)]
mod agent_tests {
    use super::*;

    #[test]
    fn agent_op_class_routes_destructive_ops_to_confirm() {
        // 删除/移动会丢内容或打乱结构 → 必须确认；新增/编辑可撤销 → 直接生效
        assert_eq!(agent_op_class("delete"), AgentOpClass::Confirm);
        assert_eq!(agent_op_class("move"), AgentOpClass::Confirm);
        assert_eq!(agent_op_class("add"), AgentOpClass::Auto);
        assert_eq!(agent_op_class("update"), AgentOpClass::Auto);
        assert_eq!(agent_op_class("未知动作"), AgentOpClass::Auto);
    }

    #[test]
    fn agent_valid_color_accepts_only_rrggbb() {
        assert_eq!(agent_valid_color(Some("#FF8800")).as_deref(), Some("#ff8800"));
        assert_eq!(agent_valid_color(Some(" #A1B2C3 ")).as_deref(), Some("#a1b2c3"));
        // 非法：缺 #、长度不对、非十六进制
        assert_eq!(agent_valid_color(Some("FF8800")), None);
        assert_eq!(agent_valid_color(Some("#F80")), None);
        assert_eq!(agent_valid_color(Some("#GG0000")), None);
        assert_eq!(agent_valid_color(None), None);
        assert_eq!(agent_valid_color(Some("")), None);
    }

    #[test]
    fn agent_history_window_keeps_order_and_respects_budget() {
        fn row(role: &str, content: &str) -> AgentMessageRow {
            AgentMessageRow {
                id: String::new(),
                session_id: String::new(),
                role: role.into(),
                content: content.into(),
                ops_json: "[]".into(),
                created_at: String::new(),
            }
        }
        let history = vec![
            row("user", "第一条"),
            row("assistant", "第一条回答"),
            // 纯 ops 载荷行：不进上下文
            row("assistant", ""),
            row("user", "第二条"),
        ];
        let win = agent_history_window(&history, 10_000);
        assert_eq!(win.len(), 3);
        assert_eq!(win[0], ("user".to_string(), "第一条".to_string()));
        assert_eq!(win[2], ("user".to_string(), "第二条".to_string()));

        // 预算只够两条时，从最新往回保留
        let win = agent_history_window(&history, ("第二条".chars().count() + 8) as usize);
        assert_eq!(win, vec![("user".to_string(), "第二条".to_string())]);
    }

    #[test]
    fn agent_tools_spec_is_valid_openai_tools_array() {
        let spec: serde_json::Value = serde_json::from_str(AGENT_TOOLS_SPEC).expect("工具集必须是合法 JSON");
        let arr = spec.as_array().expect("工具集必须是数组");
        assert!(arr.len() >= 7);
        for t in arr {
            let name = t.get("function").and_then(|f| f.get("name")).and_then(|v| v.as_str());
            assert!(name.is_some_and(|n| !n.is_empty()), "每个工具必须有 function.name");
        }
    }

    #[test]
    fn agent_build_ops_normalizes_add_with_generated_ids() {
        let args = serde_json::json!({
            "parentId": "root-1",
            "nodes": [
                { "name": "模块A", "detail": "职责", "kind": "module", "color": "#00FF00" },
                { "name": "  " },
                { "color": "#00FF00" }
            ]
        });
        let ops = agent_build_ops("add_nodes", &args).unwrap();
        // 空名与缺名节点被跳过
        assert_eq!(ops.len(), 1);
        let op = &ops[0];
        assert_eq!(op["action"], "add");
        assert_eq!(op["parentId"], "root-1");
        assert_eq!(op["name"], "模块A");
        assert_eq!(op["color"], "#00ff00");
        // id 由后端生成，模型引用它时有据可依
        assert!(op["id"].as_str().is_some_and(|s| s.starts_with("agent_")));
    }

    #[test]
    fn agent_build_ops_rejects_invalid_input() {
        assert!(agent_build_ops("add_nodes", &serde_json::json!({ "nodes": [] })).is_err());
        assert!(agent_build_ops("delete_nodes", &serde_json::json!({ "ids": ["  "] })).is_err());
        assert!(agent_build_ops("move_nodes", &serde_json::json!({ "ids": ["a"] })).is_err());
        assert!(agent_build_ops("unknown", &serde_json::json!({})).is_err());
    }
}

// ─── 项目目录绑定与 @ 文件引用 ───

/// 列出文件的跳过目录（依赖/构建产物/缓存，@ 引用不该出现它们）。
///
/// 这里只列**具体目录名**，不再一刀切地跳过所有 `.` 开头的目录：`.github`、`.vscode`
/// 这些恰恰是常被引用的（workflow、tasks.json），一刀切会让它们永远搜不到。
/// 真正该挡掉的隐藏目录已逐条列在下面。
const PROJECT_SKIP_DIRS: [&str; 20] = [
    "node_modules", ".git", "target", "dist", "build", "out", "coverage", "__pycache__",
    ".next", ".nuxt", ".svelte-kit", ".cache", ".turbo", ".parcel-cache", ".gradle",
    ".pytest_cache", ".mypy_cache", ".ruff_cache", ".venv", ".idea",
];
/// @ 引用候选的文件数上限。
///
/// 原为 800：大仓库遍历到上限即截断，而遍历是深度优先 —— 被截掉的是**整块子树**，
/// 用户会「明明有这个文件却搜不到」。候选清单只在绑定时拉取一次并缓存在前端，
/// 放宽到 4000 的传输代价可以接受。
const PROJECT_FILES_MAX: usize = 4000;
/// 目录遍历深度上限。
const PROJECT_WALK_MAX_DEPTH: usize = 12;
/// 单个附件文件进上下文的字符上限。
const AGENT_ATTACHMENT_CHARS: usize = 8_000;
/// 只引用了**一个**文件时的字符上限：分析单个文件的业务逻辑通常要读完整段代码，
/// 8k 会把后半段截掉；多文件时仍按 8k 控制总上下文。
const AGENT_ATTACHMENT_CHARS_SINGLE: usize = 24_000;
/// 单轮对话最多引用的文件数。
const AGENT_MAX_ATTACHMENTS: usize = 8;

/// 绑定/解绑导图的项目目录（一个导图文档至多绑定一个；重复绑定即替换）。
#[tauri::command]
pub fn mm_bind_document_dir(document_id: String, dir: Option<String>) -> Result<(), String> {
    let dir = dir.map(|d| d.trim().to_string()).filter(|d| !d.is_empty());
    if let Some(d) = &dir {
        if !std::path::Path::new(d).is_dir() {
            return Err(format!("目录不存在: {}", d));
        }
    }
    super::db::bind_document_dir(&document_id, dir.as_deref())
}

/// 列出文档绑定目录下的相对文件路径（@ 引用候选；跳过依赖与构建产物目录）。
#[tauri::command]
pub fn mm_list_project_files(document_id: String) -> Result<Vec<String>, String> {
    let dir = super::db::document_project_dir(&document_id)?.ok_or("该导图未绑定项目目录")?;
    let root = std::path::PathBuf::from(&dir);
    if !root.is_dir() {
        return Err(format!("绑定的目录不存在: {}", dir));
    }
    let mut out: Vec<String> = Vec::new();
    walk_project_files(&root, &root, 0, &mut out);
    out.sort();
    Ok(out)
}

fn walk_project_files(root: &std::path::Path, dir: &std::path::Path, depth: usize, out: &mut Vec<String>) {
    if depth > PROJECT_WALK_MAX_DEPTH || out.len() >= PROJECT_FILES_MAX {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<std::fs::DirEntry> = rd.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if out.len() >= PROJECT_FILES_MAX {
            return;
        }
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if p.is_dir() {
            // 只按跳过清单排除：隐藏目录不再整体屏蔽（见 PROJECT_SKIP_DIRS 注释）
            if PROJECT_SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            walk_project_files(root, &p, depth + 1, out);
        } else if let Ok(rel) = p.strip_prefix(root) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

/// 读取 @ 引用的附件内容（相对绑定目录解析；单文件截断、总数封顶），拼进上下文。
/// 把用户 @ 引用的文件内容拼进上下文。
///
/// 路径解析：**绝对路径直接用**（盘符 / UNC / Unix 根），相对路径按导图绑定的项目目录拼。
/// 此前只认绑定目录下的相对路径 —— 用户想分析**任意一个文件**（哪怕不在绑定目录里）时，
/// 内容会被静默丢掉，而「分析这个文件并画张图」恰恰是最常见的用法。
///
/// 截断标记只在真的截断时出现（旧实现只要读成功就标「（截断）」，会误导模型以为没读全）。
fn agent_attachments_block(document_dir: Option<&str>, attached: &[String]) -> String {
    let dir = document_dir.map(str::trim).filter(|d| !d.is_empty());
    let cap = if attached.len() <= 1 {
        AGENT_ATTACHMENT_CHARS_SINGLE
    } else {
        AGENT_ATTACHMENT_CHARS
    };
    let mut block = String::new();
    let mut used = 0usize;
    for raw in attached {
        if used >= AGENT_MAX_ATTACHMENTS {
            break;
        }
        let rel = raw.trim();
        if rel.is_empty() {
            continue;
        }
        used += 1;
        let Some(path) = resolve_attachment_path(dir, rel) else {
            block.push_str(&format!(
                "\n### 文件 {}\n（未绑定项目目录，无法解析这个相对路径）\n",
                rel
            ));
            continue;
        };
        let (body, truncated) = match std::fs::read_to_string(&path) {
            Ok(text) => {
                let total = text.chars().count();
                if total > cap {
                    (text.chars().take(cap).collect::<String>(), true)
                } else {
                    (text, false)
                }
            }
            Err(_) => ("（读取失败）".to_string(), false),
        };
        block.push_str(&format!(
            "\n### 文件 {}{}（{} 字符）\n{}\n",
            rel,
            if truncated { "（截断）" } else { "" },
            body.chars().count(),
            body
        ));
    }
    block
}

/// 附件路径解析：绝对径直接用；相对路径需要项目目录，没有则 None（由调用方提示）。
fn resolve_attachment_path(dir: Option<&str>, rel: &str) -> Option<std::path::PathBuf> {
    if is_absolute_path(rel) {
        return Some(std::path::PathBuf::from(rel));
    }
    dir.map(|d| std::path::Path::new(d).join(rel))
}

/// 绝对路径判定：Unix 根 / UNC / Windows 盘符。
fn is_absolute_path(value: &str) -> bool {
    let v = value.trim();
    if v.starts_with('/') || v.starts_with('\\') {
        return true;
    }
    let bytes = v.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

// ─── 子树重新分析 ───

/// 子树重析 prompt：结构化 JSON 输出 + 校验约束（kind 白名单/color/父引用）。
fn regenerate_prompt() -> String {
    r##"你是一位资深软件架构师。请分析指定模块的内部结构，生成可直接导入思维导图的 JSON。
只允许输出一个 JSON 对象，不要 Markdown 代码围栏、解释文字或尾随逗号：
{"nodes":[{"id":"唯一稳定短 ID","name":"节点名称","parent_id":null,"detail":"节点说明：职责、边界与相邻模块关系，可使用 Markdown","kind":"root|module|component|service|route|config|file|task|requirement|constraint|risk|other","color":"#RRGGBB"}]}
要求：节点字段与思维导图节点数据结构一一对应（id/name/parent_id/detail/kind/color）；第一个节点是该模块自身（parent_id 必须为 null，kind 用 root 或 module），其余 3 到 12 个节点是其子结构；所有子节点只能通过 parent_id 引用本批输出中的 id；每个节点的 detail 必须写明该模块/子模块的说明（职责、边界、与相邻模块的关系，可用 Markdown），不得为空；kind 必须在允许列表内；color 必须是 6 位十六进制颜色；只输出 JSON。"##.to_string()
}

#[tauri::command]
pub async fn mm_regenerate_node(app: tauri::AppHandle, input: RegenerateNodeInput) -> Result<DocumentFull, String> {
    let app_opt = Some(app);
    let usage = UsageAcc::default();
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
        "请分析「{}」模块（位于文档「{}」，类型 {}）的内部结构。\n模块详情：{}\n现有子节点：{}",
        target.name,
        full.document.name,
        target.kind,
        if detail_take.is_empty() { "（无）".to_string() } else { detail_take },
        children_txt,
    );
    // 生成 → 校验 → （失败）诊断反馈重试，最多 3 轮（子树重析不做证据校验）；
    // 用量同样累计：重析完成的 token 一并写入文档留痕。重析有自己独立的取消标志（导入的
    // 「停止」按钮不会误伤正在进行的重析）。
    let regen_run = format!("regen-{}", super::db::new_id("run"));
    let cancel = ai_cancel_flag(&regen_run);
    let (parsed, errs, _rounds) = ai_generate_with_repair(
        &app_opt, &usage, &cancel, &provider, &model, &regenerate_prompt(), &user, 3, None, &regen_run,
    )
    .await?;
    ai_drop_flag(&regen_run);
    let run_usage = usage.snapshot();
    if run_usage.requests > 0 {
        let _ = super::db::add_ai_usage(&input.document_id, run_usage.input_tokens, run_usage.output_tokens);
    }
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
        super::db::with_conn(|c| { super::db::sql(c.execute("UPDATE mindmap_nodes SET detail=?1, updated_at=?2 WHERE id=?3", rusqlite::params![detail, super::db::now_ts(), target.id]))?; Ok(()) })?;
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
            for id in &desc_set {
                super::db::sql(c.execute("DELETE FROM mindmap_nodes WHERE id=?1", rusqlite::params![id]))?;
                // 被删除的后代上挂的额外连线一并清理（来源或目标）
                super::db::sql(c.execute("DELETE FROM mindmap_links WHERE source_id=?1 OR target_id=?1", rusqlite::params![id]))?;
            }
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

    #[test]
    fn is_absolute_path_covers_unix_windows_and_unc() {
        assert!(is_absolute_path("/home/u/a.ts"));
        assert!(is_absolute_path("\\\\server\\share\\a.ts"));
        assert!(is_absolute_path("C:/code/a.ts"));
        assert!(is_absolute_path("d:\\code\\a.ts"));
        // 相对路径不能被误判成绝对（否则会拼到绑定目录后面）
        assert!(!is_absolute_path("src/a.ts"));
        assert!(!is_absolute_path("./a.ts"));
        assert!(!is_absolute_path("a.ts"));
    }

    #[test]
    fn attachments_resolve_absolute_and_relative_and_report_unbound() {
        let dir = std::env::temp_dir().join(format!("kira_attach_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("rel.ts"), "export const rel = 1;").unwrap();
        std::fs::write(dir.join("abs.ts"), "export const abs = 2;").unwrap();
        let dir_str = dir.to_string_lossy().to_string();
        let abs_path = dir.join("abs.ts").to_string_lossy().to_string();

        // 绝对路径：不依赖绑定目录（旧实现会把绝对路径当相对路径拼接 → 读不到）
        let block = agent_attachments_block(None, &[abs_path.clone()]);
        assert!(block.contains("export const abs = 2;"), "绝对路径应被读到: {}", block);

        // 相对路径：按绑定目录解析
        let block = agent_attachments_block(Some(&dir_str), &["rel.ts".to_string()]);
        assert!(block.contains("export const rel = 1;"), "相对路径应按绑定目录解析: {}", block);

        // 相对路径 + 未绑定目录：明确说明，而不是像以前那样静默返回空
        let block = agent_attachments_block(None, &["rel.ts".to_string()]);
        assert!(block.contains("未绑定项目目录"), "应提示无法解析: {}", block);

        // 截断标记只在真的截断时出现（旧实现读成功就标「（截断）」，会误导模型）
        let block = agent_attachments_block(None, &[abs_path]);
        assert!(!block.contains("（截断）"), "没超上限不该标截断: {}", block);

        let _ = std::fs::remove_dir_all(&dir);
    }

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
    fn walk_project_files_keeps_hidden_project_dirs_but_skips_junk() {
        // @ 引用候选的遍历口径：`.github/.vscode` 这类隐藏目录必须能搜到
        // （原先被 `starts_with('.')` 一刀切挡掉，workflow 永远 @ 不出来），
        // 而依赖/缓存目录仍要排除。
        let dir = tmp_project("walk");
        for sub in [".github/workflows", "node_modules/pkg", ".cache", "src"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        std::fs::write(dir.join(".github/workflows/ci.yml"), "on: push").unwrap();
        std::fs::write(dir.join("node_modules/pkg/index.js"), "").unwrap();
        std::fs::write(dir.join(".cache/blob"), "").unwrap();
        std::fs::write(dir.join("src/main.rs"), "").unwrap();

        let mut out: Vec<String> = Vec::new();
        walk_project_files(&dir, &dir, 0, &mut out);
        out.sort();

        assert!(out.contains(&".github/workflows/ci.yml".to_string()), "隐藏项目目录应可见: {:?}", out);
        assert!(out.contains(&"src/main.rs".to_string()));
        assert!(!out.iter().any(|f| f.starts_with("node_modules/")), "依赖目录应排除: {:?}", out);
        assert!(!out.iter().any(|f| f.starts_with(".cache/")), "缓存目录应排除: {:?}", out);
        let _ = std::fs::remove_dir_all(&dir);
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

    #[test]
    fn parse_json_repairs_invalid_escapes() {
        // Windows 路径的反斜杠未转义是最常见的 LLM 输出损坏（invalid escape）
        let raw = r#"{"summary":"x","nodes":[{"id":"a","name":"读取 C:\Users\me\config.ini","parent_id":null}]}"#;
        let v = parse_json(raw).expect("应自动修复非法转义并成功解析");
        assert_eq!(v["nodes"][0]["name"].as_str().unwrap(), "读取 C:\\Users\\me\\config.ini");

        // 合法 JSON（如 \\、\"、\n）不受影响，且不触发修复路径
        let ok = r#"{"nodes":[{"id":"a","name":"a\\b \"q\"","parent_id":null}]}"#;
        assert_eq!(parse_json(ok).unwrap()["nodes"][0]["name"].as_str().unwrap(), "a\\b \"q\"");
    }

    #[test]
    fn parse_json_keeps_unicode_and_reports_context() {
        // 中文内容在修复后必须保持无损
        let raw = "{\"name\":\"模块·订单\", \"path\": \"C:\\Temp\\x\"}";
        let v = parse_json(raw).expect("中文 + 非法转义都应被修复");
        assert_eq!(v["name"].as_str().unwrap(), "模块·订单");
        assert_eq!(v["path"].as_str().unwrap(), "C:\\Temp\\x");

        // 无法修复的损坏：错误信息必须带行/列与附近上下文（便于定位真实错误）
        let bad = "{\"nodes\":[{\"id\":\"a\",\"name\":\"ok\",\"parent_id\":null}],\"trailing\": }";
        let err = parse_json(bad).unwrap_err();
        assert!(err.contains("column"), "错误应含列号: {err}");
        assert!(err.contains("上下文"), "错误应含上下文窗口: {err}");
    }

    #[test]
    fn ai_cancel_flag_roundtrip() {
        // 取消注册表：独立 run_id 互不影响；cancel → 命中；drop 后再取是全新未取消标志
        let a = ai_cancel_flag("run-a");
        let b = ai_cancel_flag("run-b");
        a.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(is_cancelled(&a) && !is_cancelled(&b), "取消只影响对应 run_id");
        ai_drop_flag("run-b");
        let b2 = ai_cancel_flag("run-b");
        assert!(!is_cancelled(&b2), "drop 后重新创建应为干净标志");
        ai_drop_flag("run-a");
        ai_drop_flag("run-b");
    }

    #[tokio::test]
    async fn ask_answer_reaches_waiting_run() {
        let run_id = "ask-answer-regression";
        let (tx, rx) = tokio::sync::oneshot::channel();
        ask_register(run_id, tx);
        let answer = serde_json::json!({ "scope": "billing" });
        assert!(ask_send_answer(run_id, answer.clone()).is_ok());
        assert_eq!(rx.await.unwrap(), answer);
        assert!(ask_send_answer(run_id, serde_json::json!("late")).is_err());
    }

    #[tokio::test]
    async fn cancelling_run_wakes_waiting_ask() {
        let run_id = "ask-cancel-regression";
        let (tx, rx) = tokio::sync::oneshot::channel();
        ask_register(run_id, tx);
        ask_send_cancel(run_id);
        assert!(rx.await.unwrap().is_null());
        assert!(ask_send_answer(run_id, serde_json::json!("late")).is_err());
    }
}
