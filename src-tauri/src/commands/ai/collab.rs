use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Child, ChildStdin, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use serde::{Serialize, Deserialize};
use tauri::Emitter;
use crate::commands::ai_registry::registry;
use crate::commands::config::get_base_dir;
use super::models::*;
use super::launch::start_tool_proxy;

// ─── 协作线程数据模型 ───

/// 引用卡：用户引用某段内容时记录来源与原文
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CollabReference {
    pub source_message_id: String,
    pub source_sender_name: String,
    pub excerpt: String,
}

/// 文件附件：用户在输入框 @ 选择文件，派发时把内容注入提示词
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CollabFileRef {
    pub path: String,
}

/// 派发标记：本条消息触发了对某工具的派发
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CollabDispatch {
    pub tool_id: String,
    pub session_id: String,
    pub model: Option<String>,
}

/// 线程中的一条消息
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CollabMessage {
    pub id: String,
    pub room_id: String,
    /// "user" 或工具 id
    pub sender: String,
    /// 展示名："我" / "Claude Code"
    pub sender_name: String,
    pub content: String,
    pub references: Vec<CollabReference>,
    pub files: Vec<CollabFileRef>,
    pub dispatch: Option<CollabDispatch>,
    /// 工具回复回链到触发它的用户消息
    pub reply_to: Option<String>,
    /// 工具消息状态："running" | "done" | "error"
    pub status: Option<String>,
    pub created_at: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CollabRoom {
    pub id: String,
    pub name: String,
    pub project_path: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize, Deserialize, Default)]
pub struct CollabStore {
    /// Schema 版本号（未来结构变更时用于迁移判定）
    #[serde(default)]
    pub version: u32,
    pub rooms: Vec<CollabRoom>,
    pub messages: HashMap<String, Vec<CollabMessage>>,
    /// 房间+工具 → 是否已有会话（用于续聊判断）
    pub tool_sessions: HashMap<String, String>,
}

// ─── 持久化 ───

fn collab_path() -> PathBuf {
    get_base_dir().join("collab.json")
}

fn load_store() -> CollabStore {
    let path = collab_path();
    if path.exists() {
        if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(store) = serde_json::from_str::<CollabStore>(&data) {
                return store;
            }
        }
    }
    CollabStore::default()
}

fn save_store(store: &CollabStore) -> Result<(), String> {
    let path = collab_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let data = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
    fs::write(&path, data).map_err(|e| e.to_string())
}

static ID_COUNTER: AtomicU64 = AtomicU64::new(0);
fn new_id() -> String {
    let n = ID_COUNTER.fetch_add(1, Ordering::SeqCst);
    let ts = chrono::Local::now().timestamp_millis();
    format!("m{}_{}", ts, n)
}

fn now_str() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

// ─── 房间管理 ───

#[tauri::command]
pub fn collab_create_room(name: String, project_path: String) -> Result<CollabRoom, String> {
    let mut store = load_store();
    let ts = now_str();
    let room = CollabRoom {
        id: new_id(),
        name: if name.trim().is_empty() { "未命名会话".to_string() } else { name.trim().to_string() },
        project_path,
        created_at: ts.clone(),
        updated_at: ts,
    };
    store.rooms.push(room.clone());
    save_store(&store)?;
    Ok(room)
}

#[derive(Serialize)]
pub struct CollabRoomPage {
    pub rooms: Vec<CollabRoom>,
    pub has_more: bool,
    pub total: usize,
}

/// 会话列表（分页 + 按最近活跃排序），用于会话量大时的延迟加载
#[tauri::command]
pub fn collab_list_rooms(offset: Option<usize>, limit: Option<usize>) -> Result<CollabRoomPage, String> {
    let mut rooms = load_store().rooms;
    // 按 updated_at 降序（ISO 字符串可直接比较）
    rooms.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    let total = rooms.len();
    let limit = limit.unwrap_or(20).max(1);
    let offset = offset.unwrap_or(0);
    let end = (offset + limit).min(total);
    let page = if offset <= total { rooms[offset..end].to_vec() } else { vec![] };
    let has_more = end < total;
    Ok(CollabRoomPage { rooms: page, has_more, total })
}

#[tauri::command]
pub fn collab_get_messages(room_id: String) -> Result<Vec<CollabMessage>, String> {
    Ok(load_store().messages.get(&room_id).cloned().unwrap_or_default())
}

#[tauri::command]
pub fn collab_delete_room(room_id: String) -> Result<(), String> {
    let mut store = load_store();
    store.rooms.retain(|r| r.id != room_id);
    store.messages.remove(&room_id);
    // 清理该房间相关的工具会话标记
    let keys: Vec<String> = store.tool_sessions.keys()
        .filter(|k| k.starts_with(&format!("{}::", room_id)))
        .cloned()
        .collect();
    for k in keys { store.tool_sessions.remove(&k); }
    // 停止该房间相关的所有常驻代理
    stop_room_proxies(&room_id);
    save_store(&store)
}

// ─── 发送消息 + 派发 ───

/// 协作派发高级协议参数（与 LaunchAiToolRequest 对齐）
/// 使用 #[serde(default)] 使前端不传时全部回退到 None/false
#[derive(Deserialize, Clone, Debug, Default)]
pub struct CollabDispatchOptions {
    #[serde(default)]
    pub masquerade_model: Option<String>,
    #[serde(default)]
    pub fallback_model_id: Option<String>,
    #[serde(default)]
    pub fallback_provider_id: Option<String>,
    #[serde(default)]
    pub fallback_masquerade_model: Option<String>,
    #[serde(default)]
    pub one_m_context: bool,
    #[serde(default)]
    pub fallback_one_m_context: bool,
    #[serde(default)]
    pub optimizer_enabled: Option<bool>,
    #[serde(default)]
    pub rectifier_enabled: Option<bool>,
    #[serde(default)]
    pub optimizer_cache_injection: Option<bool>,
    #[serde(default)]
    pub optimizer_thinking: Option<bool>,
    #[serde(default)]
    pub optimizer_deepseek: Option<bool>,
    #[serde(default)]
    pub rectifier_thinking_signature: Option<bool>,
    #[serde(default)]
    pub rectifier_thinking_budget: Option<bool>,
    #[serde(default)]
    pub rectifier_media_fallback: Option<bool>,
    #[serde(default)]
    pub rectifier_protocol_mismatch: Option<bool>,
}

#[tauri::command]
pub async fn collab_send_message(
    app: tauri::AppHandle,
    room_id: String,
    project_path: String,
    content: String,
    references: Vec<CollabReference>,
    files: Vec<CollabFileRef>,
    tool_id: String,
    model_id: Option<String>,
    provider_id: Option<String>,
    options: Option<CollabDispatchOptions>,
) -> Result<Vec<CollabMessage>, String> {
    let mut store = load_store();
    if !store.rooms.iter().any(|r| r.id == room_id) {
        return Err("会话不存在".to_string());
    }

    let user_msg = CollabMessage {
        id: new_id(),
        room_id: room_id.clone(),
        sender: "user".to_string(),
        sender_name: "我".to_string(),
        content,
        references,
        files,
        dispatch: None,
        reply_to: None,
        status: None,
        created_at: now_str(),
    };

    let room_messages = store.messages.entry(room_id.clone()).or_default();
    room_messages.push(user_msg.clone());

    let mut result = vec![user_msg.clone()];
    // 先收集派发任务（含占位消息），统一保存后再后台执行，避免竞态
    let mut jobs: Vec<(String, String, String, String, Vec<CollabFileRef>, Option<String>, String, CollabDispatchOptions)> = Vec::new();

    if !tool_id.trim().is_empty() {
        let tool_config = match registry().get_tool_config(&tool_id) {
            Some(c) => c.clone(),
            None => return Err(format!("未知工具：{}", tool_id)),
        };
        let placeholder_id = new_id();
        let placeholder = CollabMessage {
            id: placeholder_id.clone(),
            room_id: room_id.clone(),
            sender: tool_id.clone(),
            sender_name: tool_config.display_name.clone(),
            content: String::new(),
            references: vec![],
            files: vec![],
            dispatch: Some(CollabDispatch {
                tool_id: tool_id.clone(),
                session_id: String::new(),
                model: model_id.clone(),
            }),
            reply_to: Some(user_msg.id.clone()),
            status: Some("running".to_string()),
            created_at: now_str(),
        };
        room_messages.push(placeholder.clone());
        result.push(placeholder.clone());

        jobs.push((
            tool_id.clone(),
            placeholder_id,
            project_path.clone(),
            user_msg.content.clone(),
            user_msg.files.clone(),
            model_id.clone(),
            provider_id.clone().unwrap_or_default(),
            options.clone().unwrap_or_default(),
        ));
    }

    // 更新时间戳
    if let Some(room) = store.rooms.iter_mut().find(|r| r.id == room_id) {
        room.updated_at = now_str();
    }
    // 检查是否有同房间+工具的派发正在进行（防止 TOCTOU 竞态：两个并发派发都读到空 session 导致各开新会话）
    let dispatch_key = format!("{}::{}", room_id, tool_id);
    if !tool_id.trim().is_empty() {
        let active = ACTIVE_DISPATCHES.get_or_init(|| Mutex::new(HashSet::new()));
        if active.lock().unwrap().contains(&dispatch_key) {
            return Err("该工具正在处理上一条消息，请等待完成".to_string());
        }
        active.lock().unwrap().insert(dispatch_key.clone());
    }

    // 保存（含用户消息 + 占位消息）后，再启动后台派发
    save_store(&store)?;

    for (rt_tool, rt_placeholder_id, rt_project, rt_content, rt_files, rt_model, rt_provider, rt_options) in jobs {
        let rt_room = room_id.clone();
        let rt_refs = user_msg.references.clone();
        let app_clone = app.clone();
        let cleanup_key = dispatch_key.clone();
        tauri::async_runtime::spawn(async move {
            let prompt = build_prompt(&rt_content, &rt_refs, &rt_files);
            dispatch_to_tool(
                &app_clone,
                rt_room,
                rt_tool,
                rt_project,
                prompt,
                rt_placeholder_id,
                rt_model,
                if rt_provider.is_empty() { None } else { Some(rt_provider) },
                rt_options,
            ).await;
            // 清理活动派发标记
            if let Some(active) = ACTIVE_DISPATCHES.get() {
                active.lock().unwrap().remove(&cleanup_key);
            }
        });
    }

    Ok(result)
}

/// 把用户消息 + 引用 + 文件内容拼成派发提示词
fn build_prompt(content: &str, refs: &[CollabReference], files: &[CollabFileRef]) -> String {
    let mut p = String::new();
    p.push_str(content.trim());
    if !refs.is_empty() {
        p.push_str("\n\n--- 引用内容 ---\n");
        for r in refs {
            p.push_str(&format!("【来自 {}】\n{}\n\n", r.source_sender_name, r.excerpt.trim()));
        }
    }
    if !files.is_empty() {
        p.push_str("\n\n--- 文件内容 ---\n");
        for f in files {
            // 读取文件内容（限制大小，避免超大文件卡死），失败则仅记录路径
            let body = read_file_capped(&f.path);
            p.push_str(&format!("【文件 {}】\n{}\n\n", f.path, body));
        }
    }
    p
}

/// 读取文件内容，超过 512KB 则截断并注明
fn read_file_capped(path: &str) -> String {
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) => return format!("[无法读取文件：{}]", e),
    };
    if meta.is_dir() {
        return "[跳过：这是一个目录]".to_string();
    }
    const MAX_BYTES: usize = 512 * 1024;
    match fs::read(path) {
        Ok(bytes) => {
            if bytes.len() > MAX_BYTES {
                // 按字节截断，再找 UTF-8 边界避免乱码
                let mut end = MAX_BYTES;
                while end > 0 && (bytes[end] & 0xC0) == 0x80 {
                    end -= 1;
                }
                let truncated = String::from_utf8_lossy(&bytes[..end]);
                return format!("{}…\n[文件过大，已截断至前 512KB]", truncated);
            }
            String::from_utf8_lossy(&bytes).trim().to_string()
        }
        Err(e) => format!("[无法读取文件：{}]", e),
    }
}

/// 流式派发事件（前端 listen 接收，替代轮询）
#[derive(Serialize, Clone)]
pub struct CollabDeltaPayload {
    pub room_id: String,
    pub msg_id: String,
    pub delta: String,
}

/// 活动状态推送（思考中/调用工具等），不写入消息内容，仅前端实时显示
#[derive(Serialize, Clone)]
pub struct CollabActivityPayload {
    pub room_id: String,
    pub msg_id: String,
    pub activity: String,
}

/// 工具询问用户选择时推送，前端显示交互式按钮
#[derive(Serialize, Clone)]
pub struct CollabPromptPayload {
    pub room_id: String,
    pub msg_id: String,
    pub question: String,
    pub options: Vec<String>,
}

#[derive(Serialize, Clone)]
pub struct CollabMsgUpdatedPayload {
    pub room_id: String,
    pub message: CollabMessage,
}

/// 派发生命周期控制（取消 / 超时）
#[derive(Clone)]
struct DispatchCtrl {
    child: Arc<Mutex<Option<Child>>>,
    cancel: Arc<AtomicBool>,
}

static DISPATCH_STATE: OnceLock<Mutex<HashMap<String, DispatchCtrl>>> = OnceLock::new();
const DISPATCH_TIMEOUT_SECS: u64 = 1800;

/// 活动派发追踪：防止同一房间+工具的并发派发（TOCTOU 竞态防护）
static ACTIVE_DISPATCHES: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

/// Prompt 响应控制：子进程的 stdin 句柄 + 待响应标记
struct PromptCtrl {
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    /// (问题文本, 首次检测时间) — None 表示无待响应问题
    pending: Arc<Mutex<Option<(String, Instant)>>>,
}

static PROMPT_STATE: OnceLock<Mutex<HashMap<String, PromptCtrl>>> = OnceLock::new();
const PROMPT_AUTO_RESPOND_SECS: u64 = 120;

/// 检测一行文本是否为工具的交互式询问
fn detect_prompt(line: &str) -> Option<(String, Vec<String>)> {
    let trimmed = line.trim();
    if trimmed.is_empty() { return None; }
    let lower = trimmed.to_lowercase();
    // y/n 模式
    if lower.contains("[y/n]") || lower.contains("(y/n)") || lower.contains("{y/n}") {
        let question = trimmed.trim_end_matches(|c: char| !c.is_alphanumeric()).to_string();
        return Some((question, vec!["y".to_string(), "n".to_string()]));
    }
    // yes/no 模式
    if lower.contains("(yes/no)") || lower.contains("[yes/no]") || lower.contains("(yes/no/cancel)") {
        let question = trimmed.trim_end_matches(|c: char| !c.is_alphanumeric()).to_string();
        return Some((question, vec!["yes".to_string(), "no".to_string()]));
    }
    // 常见问题模式
    let question_patterns = [
        "do you want", "would you like", "are you sure",
        "continue?", "proceed?", "confirm?", "overwrite?",
        "press enter", "press any key",
        "choose an option", "select an option", "enter your choice",
        "enter selection", "please select", "please choose",
        "please enter", "please provide",
    ];
    for pat in question_patterns {
        if lower.contains(pat) {
            return Some((trimmed.to_string(), vec!["y".to_string(), "n".to_string()]));
        }
    }
    None
}

/// 发送 prompt 事件到前端，并记录 pending 状态
fn emit_prompt(
    app: &tauri::AppHandle,
    room_id: &str,
    msg_id: &str,
    question: &str,
    options: Vec<String>,
) {
    eprintln!("[collab] prompt 检测到: {}", question);
    // 记录 pending 状态
    if let Some(map) = PROMPT_STATE.get() {
        let g = map.lock().unwrap();
        if let Some(ctrl) = g.get(msg_id) {
            let mut p = ctrl.pending.lock().unwrap();
            *p = Some((question.to_string(), Instant::now()));
        }
    }
    let _ = app.emit(
        "collab:prompt",
        CollabPromptPayload {
            room_id: room_id.to_string(),
            msg_id: msg_id.to_string(),
            question: question.to_string(),
            options,
        },
    );
}

/// 向子进程 stdin 写入响应。返回 true 表示成功写入；false 表示无待响应状态或 stdin 已关闭。
/// 无论写入是否成功，都会清除 pending 状态（避免 stdin 关闭后反复重试）。
fn write_stdin_response(msg_id: &str, response: &str) -> bool {
    if let Some(map) = PROMPT_STATE.get() {
        let g = map.lock().unwrap();
        if let Some(ctrl) = g.get(msg_id) {
            // 先检查 stdin 是否可用，不可用则清除 pending 并返回 false
            let stdin_guard = ctrl.stdin.lock().unwrap();
            if stdin_guard.is_none() {
                *ctrl.pending.lock().unwrap() = None;
                return false;
            }
            // 清除 pending 状态（无论写入结果如何，不再重试）
            *ctrl.pending.lock().unwrap() = None;
            // 重新获取 stdin 可变引用并写入
            drop(stdin_guard);
            let mut guard = ctrl.stdin.lock().unwrap();
            if let Some(ref mut stdin) = *guard {
                let _ = stdin.write_all(format!("{}\n", response).as_bytes());
                let _ = stdin.flush();
                eprintln!("[collab] prompt 已响应: {}", response);
                return true;
            }
        }
    }
    false
}

// ─── 房间级常驻代理 ───

/// 房间+工具级的常驻代理条目
struct RoomProxyEntry {
    port: u16,
    abort_handle: tokio::task::AbortHandle,
}

static ROOM_PROXIES: OnceLock<Mutex<HashMap<String, RoomProxyEntry>>> = OnceLock::new();

fn room_proxy_key(room_id: &str, tool_id: &str) -> String {
    format!("{}::{}", room_id, tool_id)
}

/// 获取或创建房间级常驻代理。返回 (端口, base_url, api_key)。
/// 首次调用时启动代理 + 写配置文件；后续调用直接复用已运行的代理。
async fn ensure_room_proxy(
    room_id: &str,
    tool_id: &str,
    tool_config: &crate::commands::ai_registry::ToolConfig,
    provider_id: Option<&str>,
    model_id: Option<&str>,
    options: &CollabDispatchOptions,
) -> (u16, String, String) {
    let key = room_proxy_key(room_id, tool_id);
    let map = ROOM_PROXIES.get_or_init(|| Mutex::new(HashMap::new()));

    // 1. 检查是否已有常驻代理 → 直接复用
    {
        let g = map.lock().unwrap();
        if let Some(entry) = g.get(&key) {
            let port = entry.port;
            let base_url = format!("http://127.0.0.1:{}", port);
            let config = super::config::load_ai_config();
            let provider = provider_id.and_then(|pid| config.providers.iter().find(|p| p.id == pid));
            let api_key = provider.map(|p| p.api_key.clone()).unwrap_or_default();
            eprintln!("[collab] 复用常驻代理 port={}", port);
            return (port, base_url, api_key);
        }
    }

    // 2. 首次：启动代理 + 写配置文件
    let config = super::config::load_ai_config();
    let provider = provider_id.and_then(|pid| config.providers.iter().find(|p| p.id == pid));

    let req = LaunchAiToolRequest {
        tool_id: tool_id.to_string(),
        model_id: model_id.map(|s| s.to_string()),
        provider_id: provider_id.map(|s| s.to_string()),
        masquerade_model: options.masquerade_model.clone(),
        fallback_model_id: options.fallback_model_id.clone(),
        fallback_provider_id: options.fallback_provider_id.clone(),
        fallback_masquerade_model: options.fallback_masquerade_model.clone(),
        one_m_context: options.one_m_context,
        fallback_one_m_context: options.fallback_one_m_context,
        optimizer_enabled: options.optimizer_enabled,
        rectifier_enabled: options.rectifier_enabled,
        rectifier_thinking_signature: options.rectifier_thinking_signature,
        rectifier_thinking_budget: options.rectifier_thinking_budget,
        rectifier_media_fallback: options.rectifier_media_fallback,
        rectifier_protocol_mismatch: options.rectifier_protocol_mismatch,
        optimizer_cache_injection: options.optimizer_cache_injection,
        optimizer_thinking: options.optimizer_thinking,
        optimizer_deepseek: options.optimizer_deepseek,
        ..Default::default()
    };

    let (port, abort_handle) = start_tool_proxy(tool_config, provider, &config, &req).await;
    let base_url = if port != 0 {
        format!("http://127.0.0.1:{}", port)
    } else {
        provider.map(|p| p.url_for(&tool_config.native_protocol())).unwrap_or_default()
    };
    let api_key = provider.map(|p| p.api_key.clone()).unwrap_or_default();

    // 写工具配置文件（仅首次，后续复用时配置文件已指向正确的代理端口）
    if port != 0 {
        if let Some(ref p) = provider {
            if !p.api_key.is_empty() {
                // 声明模型名 C：伪装优先，否则所选取模型 B
                let claimed_model = options.masquerade_model.clone()
                    .filter(|c| !c.is_empty())
                    .or_else(|| model_id.map(|s| s.to_string()));
                if let Err(e) = super::launch::write_tool_config_from_spec(
                    tool_id,
                    tool_config,
                    model_id,
                    claimed_model.as_deref(),
                    Some(&base_url),
                    &base_url,
                    &p.api_key,
                    options.fallback_model_id.as_deref(),
                    options.fallback_masquerade_model.as_deref(),
                    options.one_m_context,
                    options.fallback_one_m_context,
                    true,
                ) {
                    eprintln!("[collab] ⚠ 写入工具配置文件失败: {}", e);
                } else {
                    eprintln!("[collab] ✓ 工具配置文件已写入（baseUrl → {}）", base_url);
                }
            }
        }
    }

    // 存入全局表
    if port != 0 {
        if let Some(h) = abort_handle {
            let mut g = map.lock().unwrap();
            g.insert(key, RoomProxyEntry { port, abort_handle: h });
            eprintln!("[collab] 常驻代理已创建 port={}", port);
        }
    }

    (port, base_url, api_key)
}

/// 停止并移除房间+工具级的常驻代理
fn stop_room_proxy(room_id: &str, tool_id: &str) {
    let key = room_proxy_key(room_id, tool_id);
    if let Some(map) = ROOM_PROXIES.get() {
        let mut g = map.lock().unwrap();
        if let Some(entry) = g.remove(&key) {
            entry.abort_handle.abort();
            eprintln!("[collab] 已停止代理 port={}", entry.port);
        }
    }
}

/// 停止某房间相关的所有常驻代理
fn stop_room_proxies(room_id: &str) {
    if let Some(map) = ROOM_PROXIES.get() {
        let mut g = map.lock().unwrap();
        let prefix = format!("{}::", room_id);
        let keys: Vec<String> = g.keys().filter(|k| k.starts_with(&prefix)).cloned().collect();
        for k in keys {
            if let Some(entry) = g.remove(&k) {
                entry.abort_handle.abort();
                eprintln!("[collab] 已停止代理 port={}", entry.port);
            }
        }
    }
}

/// 停止所有常驻代理（应用退出时调用）
pub fn stop_all_room_proxies() {
    if let Some(map) = ROOM_PROXIES.get() {
        let mut g = map.lock().unwrap();
        let count = g.len();
        for (_, entry) in g.drain() {
            entry.abort_handle.abort();
        }
        if count > 0 {
            eprintln!("[collab] 已停止所有常驻代理（{} 个）", count);
        }
    }
}

// ─── 流式事件解析 ───

/// claude stream-json 的一行事件
enum StreamEvent {
    Delta(String),
    Result(String),
    /// 活动状态（思考中/调用工具等），不写入最终内容，仅前端实时显示
    Activity(String),
    Ignore,
}

/// claude `--output-format stream-json` 事件解析
fn parse_claude_json(line: &str) -> Option<(StreamEvent, Option<String>)> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let sid = v.get("session_id").and_then(|s| s.as_str()).map(|s| s.to_string());
    match v.get("type").and_then(|t| t.as_str()) {
        Some("content_block_start") => {
            // 检测 thinking / tool_use 块的开始
            if let Some(cb) = v.get("content_block") {
                match cb.get("type").and_then(|t| t.as_str()) {
                    Some("thinking") => return Some((StreamEvent::Activity("思考中…".to_string()), sid)),
                    Some("tool_use") => {
                        let name = cb.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");
                        return Some((StreamEvent::Activity(format!("调用工具: {}", name)), sid));
                    }
                    _ => {}
                }
            }
            Some((StreamEvent::Ignore, sid))
        }
        Some("content_block_delta") => {
            let delta = v.get("delta")
                .and_then(|d| d.get("text"))
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            Some((StreamEvent::Delta(delta), sid))
        }
        Some("result") => {
            let text = v.get("result").and_then(|r| r.as_str()).unwrap_or("").to_string();
            Some((StreamEvent::Result(text), sid))
        }
        _ => Some((StreamEvent::Ignore, sid)),
    }
}

/// codex `exec --json` 事件解析（JSONL）
/// 会话 id 在 thread.started.thread_id；助手文本在 item.completed(item_type=assistant_message).text
fn parse_codex_json(line: &str) -> Option<(StreamEvent, Option<String>)> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let sid = v.get("thread_id").and_then(|s| s.as_str()).map(|s| s.to_string());
    match v.get("type").and_then(|t| t.as_str()) {
        Some("thread.started") => Some((StreamEvent::Activity("初始化…".to_string()), sid)),
        Some("item.completed") => {
            let item = match v.get("item") {
                Some(i) => i,
                None => return Some((StreamEvent::Ignore, sid)),
            };
            match item.get("item_type").and_then(|t| t.as_str()) {
                Some("assistant_message") => {
                    let text = item.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string();
                    if text.is_empty() {
                        return Some((StreamEvent::Ignore, sid));
                    }
                    Some((StreamEvent::Delta(text), sid))
                }
                Some("tool_call") => {
                    let tool = item.get("tool").and_then(|t| t.as_str()).unwrap_or("unknown");
                    Some((StreamEvent::Activity(format!("使用工具: {}", tool)), sid))
                }
                _ => Some((StreamEvent::Ignore, sid)),
            }
        }
        Some("turn.failed") => {
            let err = v.get("error").and_then(|e| e.as_str()).unwrap_or("turn failed").to_string();
            Some((StreamEvent::Result(format!("[error] {}", err)), sid))
        }
        _ => Some((StreamEvent::Ignore, sid)),
    }
}

/// 从 opencode `--format json` 事件里提取助手文本
/// 实际格式（真机确认）：
///   {"type":"text",...,"sessionID":"ses_xxx","part":{"type":"text","text":"你好",...}}
///   {"type":"step_finish",...,"part":{"type":"step-finish",...}}
fn extract_opencode_text(v: &serde_json::Value) -> Option<String> {
    // 优先：part.text（opencode run --format json 的标准位置）
    if let Some(part) = v.get("part") {
        if let Some(s) = part.get("text").and_then(|x| x.as_str()) {
            return Some(s.to_string());
        }
        // part.content 也作为容错
        if let Some(s) = part.get("content").and_then(|x| x.as_str()) {
            return Some(s.to_string());
        }
    }
    // 兜底：顶层 text / content
    if let Some(s) = v.get("text").and_then(|x| x.as_str()) {
        return Some(s.to_string());
    }
    if let Some(s) = v.get("content").and_then(|x| x.as_str()) {
        return Some(s.to_string());
    }
    None
}

/// opencode `run --format json` 事件解析
/// 实际 JSONL 格式（每行一个 JSON 对象）：
///   step_start → part.type = "step-start"
///   tool_use   → part.type = "tool", part.tool = "write"/"bash"/...
///   text       → part.type = "text", part.text = "助手回复"
///   step_finish→ part.type = "step-finish"
fn parse_opencode_json(line: &str) -> Option<(StreamEvent, Option<String>)> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    // sessionID（驼峰）是 opencode 的标准字段
    let sid = v.get("sessionID")
        .or_else(|| v.get("session_id"))
        .or_else(|| v.get("session").and_then(|s| s.get("id")))
        .or_else(|| v.get("id"))
        .and_then(|s| s.as_str())
        .map(|s| s.to_string());
    let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    // 收尾事件：step_finish / done / result / completed
    if matches!(ty, "step_finish" | "done" | "result" | "completed" | "turn.completed" | "session.completed") {
        let text = extract_opencode_text(&v).unwrap_or_default();
        return Some((StreamEvent::Result(text), sid));
    }
    // step_start → 活动状态
    if ty == "step_start" {
        return Some((StreamEvent::Activity("思考中…".to_string()), sid));
    }
    // tool_use → 提取工具名和状态
    if ty == "tool_use" {
        if let Some(part) = v.get("part") {
            let tool = part.get("tool").and_then(|t| t.as_str()).unwrap_or("unknown");
            let status = part.get("state")
                .and_then(|s| s.get("status"))
                .and_then(|s| s.as_str())
                .unwrap_or("");
            // 提取工具输入摘要
            let detail = part.get("state")
                .and_then(|s| s.get("input"))
                .and_then(|i| i.get("filePath"))
                .or_else(|| part.get("state").and_then(|s| s.get("input")).and_then(|i| i.get("command")))
                .and_then(|f| f.as_str())
                .map(|s| {
                    let s = s.rsplit(['/', '\\']).next().unwrap_or(s);
                    format!(" → {}", s)
                })
                .unwrap_or_default();
            let activity = match status {
                "completed" => format!("✓ {}{}", tool, detail),
                "running" | "in_progress" => format!("执行 {}…{}", tool, detail),
                _ => format!("{} ({}){}", tool, status, detail),
            };
            return Some((StreamEvent::Activity(activity), sid));
        }
        return Some((StreamEvent::Activity("使用工具…".to_string()), sid));
    }
    // 带文本 → 增量
    if let Some(t) = extract_opencode_text(&v) {
        if !t.is_empty() {
            return Some((StreamEvent::Delta(t), sid));
        }
    }
    Some((StreamEvent::Ignore, sid))
}

/// gemini-cli / qwen-code `--output-format stream-json` 事件解析
/// 格式：
///   {"type":"system","subtype":"init","session_id":"..."}
///   {"type":"assistant","message":{"content":[{"type":"text","text":"你好"}]}}
///   {"type":"result","subtype":"success","result":"你好","session_id":"..."}
fn parse_gemini_json(line: &str) -> Option<(StreamEvent, Option<String>)> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let sid = v.get("session_id").and_then(|s| s.as_str()).map(|s| s.to_string());
    match v.get("type").and_then(|t| t.as_str()) {
        Some("system") => {
            // 初始化事件 → 活动状态
            Some((StreamEvent::Activity("初始化…".to_string()), sid))
        }
        Some("assistant") => {
            // 提取 message.content 中的文本
            let mut text = String::new();
            if let Some(content) = v.get("message").and_then(|m| m.get("content")).and_then(|c| c.as_array()) {
                for part in content {
                    if part.get("type").and_then(|t| t.as_str()) == Some("text") {
                        if let Some(t) = part.get("text").and_then(|x| x.as_str()) {
                            text.push_str(t);
                        }
                    }
                    // 检测工具调用
                    if part.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                        let name = part.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");
                        return Some((StreamEvent::Activity(format!("调用工具: {}", name)), sid));
                    }
                }
            }
            if !text.is_empty() {
                Some((StreamEvent::Delta(text), sid))
            } else {
                Some((StreamEvent::Ignore, sid))
            }
        }
        Some("result") => {
            let text = v.get("result").and_then(|r| r.as_str()).unwrap_or("").to_string();
            let is_error = v.get("is_error").and_then(|e| e.as_bool()).unwrap_or(false);
            if is_error {
                Some((StreamEvent::Result(format!("[error] {}", text)), sid))
            } else {
                Some((StreamEvent::Result(text), sid))
            }
        }
        Some("user") => {
            // 用户消息回显 → 忽略
            Some((StreamEvent::Ignore, sid))
        }
        _ => Some((StreamEvent::Ignore, sid)),
    }
}

/// 按 runner 分发到对应解析器
fn parse_runner_event(runner: &str, line: &str) -> Option<(StreamEvent, Option<String>)> {
    match runner {
        "stream-json" => parse_claude_json(line),
        "codex-json" => parse_codex_json(line),
        "opencode-json" => parse_opencode_json(line),
        "gemini-json" => parse_gemini_json(line),
        _ => None,
    }
}

/// 转义用于 cmd /c "..." 内联的参数：% 变量展开与 " 引号
fn escape_cmd_arg(s: &str) -> String {
    s.replace('%', "%%").replace('"', "\"\"")
}

/// 杀掉进程树（Windows 用 taskkill /T，否则直接 kill）
fn kill_tree(child: &Arc<Mutex<Option<Child>>>) {
    if let Some(c) = child.lock().unwrap().as_mut() {
        #[cfg(windows)]
        {
            let pid = c.id();
            let _ = Command::new("taskkill").args(["/F", "/T", "/PID", &pid.to_string()]).output();
        }
        #[cfg(not(windows))]
        {
            let _ = c.kill();
        }
    }
}

/// 收尾：写入最终消息 + 推送 msg-updated
fn finalize_message(
    app: &tauri::AppHandle,
    room_id: &str,
    msg_id: &str,
    status: &str,
    content: String,
    sid: Option<String>,
    // 若提供 (session_key, session_id)，则在同一 store 写入中保存 tool_sessions，避免额外的 load+save 循环
    session_update: Option<(String, String)>,
) {
    let mut store = load_store();
    if let Some(msgs) = store.messages.get_mut(room_id) {
        if let Some(m) = msgs.iter_mut().find(|m| m.id == msg_id) {
            m.content = content;
            m.status = Some(status.to_string());
            // 保留原始 created_at（创建时间），不覆盖为完成时间
            if let Some(s) = sid {
                if let Some(d) = m.dispatch.as_mut() {
                    d.session_id = s;
                }
            }
        }
    }
    // 同时写入 session 绑定（合并 I/O，减少重复 load_store）
    if let Some((key, sid)) = session_update {
        store.tool_sessions.insert(key, sid);
    }
    let _ = save_store(&store);
    if let Some(msgs) = store.messages.get(room_id) {
        if let Some(m) = msgs.iter().find(|m| m.id == msg_id) {
            let _ = app.emit(
                "collab:msg-updated",
                CollabMsgUpdatedPayload { room_id: room_id.to_string(), message: m.clone() },
            );
        }
    }
}

/// 派发到工具：流式读取 stdout，逐段 emit；带会话绑定 / 取消 / 超时。
/// 代理为房间+工具级常驻：首次派发时启动代理 + 写配置文件，后续消息复用同一代理。
async fn dispatch_to_tool(
    app: &tauri::AppHandle,
    room_id: String,
    tool_id: String,
    project_path: String,
    prompt: String,
    placeholder_id: String,
    model_id: Option<String>,
    provider_id: Option<String>,
    options: CollabDispatchOptions,
) {
    let tool_config = match registry().get_tool_config(&tool_id) {
        Some(c) => c.clone(),
        None => {
            finalize_message(app, &room_id, &placeholder_id, "error", "⚠ 未知工具".to_string(), None, None);
            return;
        }
    };

    // 会话绑定：已有 id 则续聊（不再开新会话）
    let session_key = format!("{}::{}", room_id, tool_id);
    let existing_sid = load_store().tool_sessions.get(&session_key).cloned();
    let has_session = existing_sid.is_some();

    let tmp_dir = get_base_dir().join("collab_tmp");
    let _ = fs::create_dir_all(&tmp_dir);
    let prompt_path = tmp_dir.join(format!(
        "{}_{}.txt",
        room_id.replace(|c: char| !c.is_alphanumeric(), ""),
        new_id()
    ));
    if let Err(e) = fs::write(&prompt_path, &prompt) {
        finalize_message(app, &room_id, &placeholder_id, "error", format!("⚠ 写入提示词失败: {}", e), None, None);
        return;
    }

    // ─── 获取或创建房间级常驻代理 + 配置文件（首次启动，后续复用） ───
    let (_port, base_url, api_key) = ensure_room_proxy(
        &room_id,
        &tool_id,
        &tool_config,
        provider_id.as_deref(),
        model_id.as_deref(),
        &options,
    ).await;

    // 加载 provider 用于 env 注入
    let config = super::config::load_ai_config();
    let provider = provider_id.as_deref().and_then(|pid| config.providers.iter().find(|p| p.id == pid));

    let runner = tool_config.runner.clone().unwrap_or_default();
    let tmpl = if has_session {
        tool_config.dispatch_resume_cmd.clone()
            .or_else(|| tool_config.dispatch_continue_cmd.clone())
    } else {
        tool_config.dispatch_cmd.clone()
    };
    let tmpl = match tmpl {
        Some(t) => t,
        None => {
            let _ = fs::remove_file(&prompt_path);
            finalize_message(app, &room_id, &placeholder_id, "error", format!("⚠ 工具 {} 未配置派发命令", tool_config.display_name), None, None);
            return;
        }
    };

    let prompt_str = prompt_path.to_string_lossy().replace('\\', "/");
    let quoted = format!("\"{}\"", prompt_str);
    let prompt_mode = tool_config.prompt_mode.clone().unwrap_or_default();
    let use_stdin = prompt_mode == "stdin";

    let mut cmd = tmpl;
    cmd = cmd.replace("{session_id}", existing_sid.as_deref().unwrap_or(""));
    if cmd.contains("{prompt_file}") {
        cmd = cmd.replace("{prompt_file}", &quoted);
    }
    if cmd.contains("{prompt}") {
        cmd = cmd.replace("{prompt}", &escape_cmd_arg(&prompt));
    }

    // ─── 环境变量注入：从 config_file.write 中的 env.* 键注入（与正常启动一致） ───
    let model_for_env = options.masquerade_model.clone()
        .filter(|c| !c.is_empty())
        .or_else(|| model_id.clone())
        .unwrap_or_default();
    let mut envs = if let Some(ref p) = provider {
        super::launch::build_env_vars(&tool_config, &p.api_key, &base_url, &model_for_env)
    } else {
        HashMap::new()
    };
    // 兜底：若工具无 config_file 定义，仍按协议注入基础 env vars
    if envs.is_empty() {
        match tool_config.native_protocol().as_str() {
            "anthropic" => {
                envs.insert("ANTHROPIC_BASE_URL".to_string(), base_url.clone());
                envs.insert("ANTHROPIC_API_KEY".to_string(), api_key.clone());
                if let Some(m) = model_id.as_deref() { envs.insert("ANTHROPIC_MODEL".to_string(), m.to_string()); }
            }
            "google" => {
                envs.insert("GOOGLE_API_BASE_URL".to_string(), base_url.clone());
                envs.insert("GOOGLE_API_KEY".to_string(), api_key.clone());
            }
            _ => {
                envs.insert("OPENAI_BASE_URL".to_string(), base_url.clone());
                envs.insert("OPENAI_API_KEY".to_string(), api_key.clone());
            }
        }
    }

    eprintln!("[collab] 派发 {} → cmd: {}", tool_config.display_name, cmd);
    let mut command = if cfg!(windows) { Command::new("cmd") } else { Command::new("sh") };
    if cfg!(windows) {
        command.args(["/c", &cmd]);
    } else {
        command.args(["-c", &cmd]);
    }
    command
        .current_dir(&project_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // 提示词传入方式：stdin 模式把临时文件作为子进程 stdin；否则用 piped（支持交互式应答）
    let mut child_stdin_opt: Option<ChildStdin> = None;
    if use_stdin {
        match fs::File::open(&prompt_path) {
            Ok(f) => { command.stdin(Stdio::from(f)); }
            Err(e) => {
                let _ = fs::remove_file(&prompt_path);
                finalize_message(app, &room_id, &placeholder_id, "error", format!("⚠ 打开提示词失败: {}", e), None, None);
                return;
            }
        }
    } else {
        command.stdin(Stdio::piped());
    }
    for (k, v) in &envs {
        command.env(k, v);
    }

    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = fs::remove_file(&prompt_path);
            finalize_message(app, &room_id, &placeholder_id, "error", format!("⚠ 启动工具失败: {}", e), None, None);
            return;
        }
    };

    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => {
            let _ = fs::remove_file(&prompt_path);
            finalize_message(app, &room_id, &placeholder_id, "error", "⚠ 无法读取工具输出".to_string(), None, None);
            return;
        }
    };
    // 取 stdin 句柄（非 stdin 模式时用于交互式应答）
    if !use_stdin {
        child_stdin_opt = child.stdin.take();
    }
    // 取 stderr 并在后台线程中持续读取，防止管道缓冲区满后子进程阻塞
    let stderr = child.stderr.take();
    let child_arc = Arc::new(Mutex::new(Some(child)));
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let map = DISPATCH_STATE.get_or_init(|| Mutex::new(HashMap::new()));
        let mut g = map.lock().unwrap();
        g.insert(placeholder_id.clone(), DispatchCtrl {
            child: child_arc.clone(),
            cancel: cancel.clone(),
        });
    }

    // stderr 后台读取线程：收集内容到 stderr_buf，供无输出时诊断；同时检测交互式 prompt
    let stderr_cancel = cancel.clone();
    let stderr_buf = Arc::new(Mutex::new(String::new()));
    let stderr_buf_clone = stderr_buf.clone();
    let stderr_room = room_id.clone();
    let stderr_msg = placeholder_id.clone();
    let stderr_app = app.clone();
    let stderr_handle = if let Some(stderr) = stderr {
        Some(std::thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut buf = String::new();
            loop {
                if stderr_cancel.load(Ordering::SeqCst) { break; }
                buf.clear();
                match reader.read_line(&mut buf) {
                    Ok(0) => break,
                    Ok(_) => {
                        let line = buf.trim_end();
                        if !line.is_empty() {
                            eprintln!("[collab] stderr: {}", line);
                            // 检测 prompt
                            if let Some((question, options)) = detect_prompt(line) {
                                emit_prompt(&stderr_app, &stderr_room, &stderr_msg, &question, options);
                            }
                            let mut g = stderr_buf_clone.lock().unwrap();
                            if g.len() < 8192 {
                                g.push_str(line);
                                g.push('\n');
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        }))
    } else {
        None
    };

    // 使用独立线程读取 stdout，主循环通过 channel + recv_timeout 实现非阻塞语义，
    // 使取消/超时检测在子进程长时间无输出时仍能生效
    let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
    let stdout_reader = std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => { let _ = tx.send(None); break; }
                Ok(_) => { let _ = tx.send(Some(line.clone())); }
                Err(_) => { let _ = tx.send(None); break; }
            }
        }
    });

    // 注册 PromptCtrl（若有 stdin 句柄）
    let prompt_stdin = Arc::new(Mutex::new(child_stdin_opt));
    let prompt_pending: Arc<Mutex<Option<(String, Instant)>>> = Arc::new(Mutex::new(None));
    if !use_stdin {
        let map = PROMPT_STATE.get_or_init(|| Mutex::new(HashMap::new()));
        let mut g = map.lock().unwrap();
        g.insert(placeholder_id.clone(), PromptCtrl {
            stdin: prompt_stdin.clone(),
            pending: prompt_pending.clone(),
        });
    }

    let start = Instant::now();
    let timeout = Duration::from_secs(DISPATCH_TIMEOUT_SECS);
    let poll_interval = Duration::from_millis(200);
    let mut accumulated = String::new();
    let mut captured_sid: Option<String> = None;
    let mut err_msg: Option<String> = None;
    let mut raw_lines: Vec<String> = Vec::new(); // 用于 JSON 解析失败时回退显示原始输出
    let is_json_runner = runner == "stream-json" || runner == "codex-json" || runner == "opencode-json" || runner == "gemini-json";

    loop {
        if cancel.load(Ordering::SeqCst) {
            err_msg = Some("⚠ 已取消".to_string());
            kill_tree(&child_arc);
            break;
        }
        if start.elapsed() > timeout {
            err_msg = Some(format!("⚠ 派发超时（>{}秒）", DISPATCH_TIMEOUT_SECS));
            kill_tree(&child_arc);
            break;
        }
        // 检查 prompt 超时自动应答
        if let Some(ref pend) = *prompt_pending.lock().unwrap() {
            if pend.1.elapsed() > Duration::from_secs(PROMPT_AUTO_RESPOND_SECS) {
                eprintln!("[collab] prompt 超时自动应答: y");
                write_stdin_response(&placeholder_id, "y");
            }
        }
        // 非阻塞接收：timeout 轮询，使取消/超时检测生效
        match rx.recv_timeout(poll_interval) {
            Ok(Some(line)) => {
                let l = line.trim_end().to_string();
                if l.is_empty() { continue; }
                if is_json_runner {
                    // 保留原始行用于回退
                    if raw_lines.len() < 50 {
                        raw_lines.push(l.clone());
                    }
                    // 检测 prompt（JSON 解析失败时也检查）
                    if let Some((question, options)) = detect_prompt(&l) {
                        emit_prompt(app, &room_id, &placeholder_id, &question, options);
                    }
                    if let Some((ev, sid)) = parse_runner_event(&runner, &l) {
                        if let Some(s) = sid { captured_sid = Some(s); }
                        match ev {
                            StreamEvent::Delta(d) => {
                                accumulated.push_str(&d);
                                let _ = app.emit(
                                    "collab:delta",
                                    CollabDeltaPayload { room_id: room_id.clone(), msg_id: placeholder_id.clone(), delta: d },
                                );
                            }
                            StreamEvent::Activity(a) => {
                                let _ = app.emit(
                                    "collab:activity",
                                    CollabActivityPayload { room_id: room_id.clone(), msg_id: placeholder_id.clone(), activity: a },
                                );
                            }
                            StreamEvent::Result(t) => {
                                // 仅当收尾文本非空才覆盖（避免空 result 抹掉已流式内容）
                                if !t.is_empty() { accumulated = t; }
                            }
                            StreamEvent::Ignore => {}
                        }
                    } else {
                        // JSON 解析失败，作为原始文本兜底
                        eprintln!("[collab] JSON 解析失败: {}", l.chars().take(200).collect::<String>());
                    }
                } else {
                    // 非 JSON runner：检测 prompt
                    if let Some((question, options)) = detect_prompt(&l) {
                        emit_prompt(app, &room_id, &placeholder_id, &question, options);
                    }
                    accumulated.push_str(&l);
                    accumulated.push('\n');
                }
            }
            Ok(None) => break, // EOF
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    if cancel.load(Ordering::SeqCst) && err_msg.is_none() {
        err_msg = Some("⚠ 已取消".to_string());
    }

    // 等待 stdout 读取线程结束
    let _ = stdout_reader.join();
    // 等待 stderr 读取线程结束
    if let Some(h) = stderr_handle { let _ = h.join(); }

    let _ = fs::remove_file(&prompt_path);
    // 代理为房间级常驻，不在每条消息结束时停止
    if let Some(map) = DISPATCH_STATE.get() {
        map.lock().unwrap().remove(&placeholder_id);
    }
    // 清理 PromptCtrl
    if let Some(map) = PROMPT_STATE.get() {
        map.lock().unwrap().remove(&placeholder_id);
    }

    // 获取 stderr 内容
    let stderr_content = stderr_buf.lock().unwrap().clone();

    match err_msg {
        Some(e) => {
            finalize_message(app, &room_id, &placeholder_id, "error", e, None, None);
        }
        None => {
            let mut content = accumulated.trim().to_string();
            // JSON runner 但无解析结果 → 回退显示原始 stdout + stderr
            if content.is_empty() && is_json_runner && !raw_lines.is_empty() {
                content = format!("⚠ 未解析到有效内容，原始输出：\n{}", raw_lines.join("\n"));
                if !stderr_content.is_empty() {
                    content.push_str(&format!("\n\n--- stderr ---\n{}", stderr_content.trim()));
                }
            } else if content.is_empty() {
                // 非 JSON runner 也无输出 → 显示 stderr
                if !stderr_content.is_empty() {
                    content = format!("⚠ 无 stdout 输出，stderr 内容：\n{}", stderr_content.trim());
                } else {
                    content = "(无输出)".to_string();
                }
            }
            // 合并 session 绑定写入 finalize_message，避免额外的 load_store + save_store 循环
            let session_update = captured_sid.as_ref().map(|sid| (session_key.clone(), sid.clone()));
            finalize_message(
                app,
                &room_id,
                &placeholder_id,
                "done",
                content,
                captured_sid.clone(),
                session_update,
            );
        }
    }
}

/// 用户取消正在进行的派发
#[tauri::command]
pub fn collab_cancel_dispatch(msg_id: String) -> Result<(), String> {
    let ctrl = if let Some(map) = DISPATCH_STATE.get() {
        map.lock().unwrap().get(&msg_id).cloned()
    } else {
        None
    };
    if let Some(ctrl) = ctrl {
        ctrl.cancel.store(true, Ordering::SeqCst);
        kill_tree(&ctrl.child);
    }
    Ok(())
}

/// 重置某工具在某会话中的续聊上下文（删除绑定的 session id）
#[tauri::command]
pub fn collab_reset_session(room_id: String, tool_id: String) -> Result<(), String> {
    let mut store = load_store();
    store.tool_sessions.remove(&format!("{}::{}", room_id, tool_id));
    save_store(&store)?;
    // 同时停止并清理常驻代理，下次派发时重建
    stop_room_proxy(&room_id, &tool_id);
    Ok(())
}

/// 用户响应工具的交互式询问
#[tauri::command]
pub fn collab_respond_prompt(msg_id: String, response: String) -> Result<(), String> {
    if !write_stdin_response(&msg_id, &response) {
        return Err("无待响应的询问或工具不支持交互".to_string());
    }
    Ok(())
}
