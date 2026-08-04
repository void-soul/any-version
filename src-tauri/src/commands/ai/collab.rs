use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Child, ChildStdin, Stdio};
#[cfg(windows)]
#[cfg(windows)]
use std::os::windows::process::CommandExt; // raw_arg：原样透传参数，阻止 Rust 对 cmd /c 命令串重新加引号
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use parking_lot::Mutex;
use std::sync::{Arc, OnceLock};
use tokio::sync::Notify;
use std::time::{Duration, Instant};
use serde::{Serialize, Deserialize};
use serde_json::{json, Value};
use tauri::Emitter;
use crate::commands::ai_registry::registry;
use crate::commands::config::get_base_dir;
use super::models::*;
use super::launch::start_tool_proxy_with_collab;

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

/// token 消耗（来自工具输出的 usage 字段；非所有工具都提供，故为 Option）
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// 派发标记：本条消息触发了对某工具的派发
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CollabDispatch {
    pub tool_id: String,
    pub session_id: String,
    pub model: Option<String>,
    /// 派发总耗时（毫秒），收尾时回填
    pub duration_ms: Option<u64>,
    /// token 消耗；工具输出含 usage 时回填，否则 None
    pub usage: Option<TokenUsage>,
}

/// 上下文快照：压缩旧会话后生成的摘要，用于在新会话中恢复上下文
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ContextSnapshot {
    pub id: String,
    pub room_id: String,
    pub tool_id: String,
    pub summary: String,
    pub old_session_id: String,
    pub message_count: usize,
    pub created_at: String,
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
    /// 展示头像（emoji 或单字符），来自工具 config.avatar；旧消息无此字段时按 sender 回退
    #[serde(default)]
    pub avatar: Option<String>,
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

/// 协作任务（任务流 E：open/claimed/in_progress/in_review/done），agent 可通过总线创建/认领/交接/完成
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct CollabTask {
    pub id: String,
    pub room_id: String,
    pub title: String,
    /// 任务详情 / 验收标准
    #[serde(default)]
    pub description: String,
    /// open | claimed | in_progress | in_review | done
    #[serde(default = "default_task_status")]
    pub status: String,
    /// 认领者 tool id
    #[serde(default)]
    pub assignee: Option<String>,
    /// 创建者 tool id（人或工具）
    #[serde(default)]
    pub created_by: String,
    /// 父任务（用于拆分/交接链）
    #[serde(default)]
    pub parent_task: Option<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

fn default_task_status() -> String {
    "open".to_string()
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
    /// 房间+工具 → 上下文快照（压缩后生成，新会话首次派发时注入）
    #[serde(default)]
    pub context_snapshots: HashMap<String, ContextSnapshot>,
    /// 派发轮次：同一发送者在静默窗口内的多条消息合并为一次派发
    #[serde(default)]
    pub turns: Vec<CollabTurn>,
    /// 回复授权槽位（持久化替代内存 ACTIVE_DISPATCHES，重启后可恢复防重复派发）
    #[serde(default)]
    pub reply_grants: Vec<ReplyGrant>,
    /// agent 在线状态（每个工具的当前运行状态）
    #[serde(default)]
    pub agents: Vec<CollabAgentStatus>,
    /// 任务流（E）
    #[serde(default)]
    pub tasks: Vec<CollabTask>,
}

/// 回复授权槽位（回复协调用）
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ReplyGrant {
    pub id: String,
    /// 触发消息 id（通常为 turn.trigger_message_id 或用户消息 id）
    pub message_id: String,
    pub tool_id: String,
    /// primary（主回复，唯一）| directed（被 @ 协同者）| supplemental（补充/纠正）
    pub slot: String,
    /// none | reserved | active | consumed | released
    pub status: String,
}

/// agent 在线状态
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CollabAgentStatus {
    pub tool_id: String,
    /// offline | online | thinking | working
    pub status: String,
    pub current_room: Option<String>,
    /// 最近一次心跳（UTC 时间戳字符串）
    pub last_heartbeat: String,
}

/// Turn 静默窗口（毫秒）：窗口内同一发送者的新消息合并进同一轮次
const TURN_WINDOW_MS: u64 = 800;
/// 唤醒深度上限：防止工具互 @ 形成无限循环风暴
const MAX_AGENT_WAKE_DEPTH: u32 = 4;
/// 同一根消息的最大唤醒次数
const MAX_AGENT_WAKES_PER_ROOT: u32 = 6;

// ─── agent 互聊总线（C 方案：agent 主动同步委派，对齐 open-tag 的 agent CLI） ───
/// 一次同步委派的运行时状态，server 端持有，agent 通过 HTTP 阻塞等待结果返回
struct DelegationRequest {
    msg_id: String,
    status: String,
    result: String,
    notify: Arc<Notify>,
}
static COLLAB_BUS: OnceLock<Mutex<HashMap<String, DelegationRequest>>> = OnceLock::new();
/// 房间 → 总线鉴权 token（localhost 单用户，按房间隔离，注入给被派发 agent 的环境）
static ROOM_TOKENS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

/// 取（或生成）某房间的协同总线 token
pub fn room_bus_token(room_id: &str) -> String {
    let map = ROOM_TOKENS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut g = map.lock();
    if let Some(t) = g.get(room_id) {
        return t.clone();
    }
    let t = new_id();
    g.insert(room_id.to_string(), t.clone());
    t
}

fn register_delegation(request_id: &str, msg_id: &str) -> Arc<Notify> {
    let notify = Arc::new(Notify::new());
    COLLAB_BUS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .insert(
            request_id.to_string(),
            DelegationRequest {
                msg_id: msg_id.to_string(),
                status: "running".to_string(),
                result: String::new(),
                notify: notify.clone(),
            },
        );
    notify
}

/// 被派发工具完成时回填结果并唤醒等待方（在 finalize_message 中调用）
fn complete_delegation(msg_id: &str, status: &str, result: &str) {
    let bus = COLLAB_BUS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut g = bus.lock();
    if let Some((_, req)) = g.iter_mut().find(|(_, r)| r.msg_id == msg_id) {
        req.status = status.to_string();
        req.result = result.to_string();
        req.notify.notify_one();
    }
}

/// 阻塞等待委派结果（带超时）。返回 (status, result)
pub async fn await_delegation(request_id: &str, timeout_secs: u64) -> Option<(String, String)> {
    let notify = {
        let bus = COLLAB_BUS.get_or_init(|| Mutex::new(HashMap::new()));
        bus.lock().get(request_id).map(|r| r.notify.clone())?
    };
    {
        let bus = COLLAB_BUS.get_or_init(|| Mutex::new(HashMap::new()));
        if let Some(r) = bus.lock().get(request_id) {
            if r.status != "running" {
                return Some((r.status.clone(), r.result.clone()));
            }
        }
    }
    match tokio::time::timeout(Duration::from_secs(timeout_secs), notify.notified()).await {
        Ok(_) => {
            let bus = COLLAB_BUS.get_or_init(|| Mutex::new(HashMap::new()));
            bus.lock()
                .get(request_id)
                .map(|r| (r.status.clone(), r.result.clone()))
        }
        Err(_) => Some(("timeout".to_string(), String::new())),
    }
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// 解析时间戳字符串（兼容 now_str 的 "%Y-%m-%dT%H:%M:%S" 与 RFC3339）为秒级时间戳
fn parse_ts(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|d| d.timestamp())
        .ok()
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S")
                .ok()
                .map(|d| d.and_utc().timestamp())
        })
}

/// 计算静默窗口到期时间戳（now + TURN_WINDOW_MS），到期后才派发
fn dispatch_after_ts() -> String {
    (chrono::Utc::now() + chrono::Duration::milliseconds(TURN_WINDOW_MS as i64)).to_rfc3339()
}

/// Turn 调度器是否已启动（进程级单例，避免重复 spawn 定时器）
static TURN_SCHEDULER_STARTED: OnceLock<Mutex<bool>> = OnceLock::new();

/// 确保 Turn 调度常驻任务只启动一次。每 ~200ms 扫描到期 turn 并派发。
fn ensure_turn_scheduler(app: tauri::AppHandle) {
    let started = TURN_SCHEDULER_STARTED.get_or_init(|| Mutex::new(false));
    let mut g = started.lock();
    if *g {
        return;
    }
    *g = true;
    drop(g);
    tauri::async_runtime::spawn(async move {
        eprintln!("[collab] Turn 调度器已启动");
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            tick_turns(app.clone()).await;
        }
    });
}

/// 内部派发任务参数（从到期 turn 收集，锁外 await 使用）
struct DispatchJob {
    room_id: String,
    tool_id: String,
    placeholder_id: String,
    prompt: String,
    model_id: Option<String>,
    provider_id: Option<String>,
    options: CollabDispatchOptions,
    turn_id: String,
    trigger_message_id: String,
}

/// 扫描到期 turn：锁定内收集待派发任务（含合并消息内容、路由选 owner、状态变更），
/// 锁外 await 执行实际派发。
async fn tick_turns(app: tauri::AppHandle) {
    let jobs: Vec<DispatchJob> = {
        let _lock = STORE_LOCK.get_or_init(|| Mutex::new(())).lock();
        let mut store = load_store();
        let now_ts = parse_ts(&now_rfc3339()).unwrap_or(0);
        let mut out = Vec::new();
        for turn in store.turns.iter_mut() {
            if turn.state != "collecting" {
                continue;
            }
            if parse_ts(&turn.dispatch_after).unwrap_or(0) > now_ts {
                continue; // 仍在静默窗口内
            }
            // 路由选 owner：手动 @ 已定；否则自动路由
            let owner = match &turn.owner_tool_id {
                Some(o) if !o.is_empty() => o.clone(),
                _ => route_owner(&store.messages, &turn.room_id),
            };
            if owner.is_empty() {
                // 无候选工具：保持 collecting，等待后续（理论上不应发生，因为手动 @ 必有 owner）
                continue;
            }
            // 防并发：若该 room::tool 已在派发中，跳过本次（轮询后续再处理）
            let dispatch_key = format!("{}::{}", turn.room_id, owner);
            let active = ACTIVE_DISPATCHES.get_or_init(|| tokio::sync::Mutex::new(HashSet::new()));
            if active.try_lock().map(|g| g.contains(&dispatch_key)).unwrap_or(false) {
                continue;
            }
            turn.state = "dispatching".to_string();
            turn.owner_tool_id = Some(owner.clone());
            turn.updated_at = now_rfc3339();
            // 写主回复授权槽位
            store.reply_grants.push(ReplyGrant {
                id: new_id(),
                message_id: turn.trigger_message_id.clone(),
                tool_id: owner.clone(),
                slot: "primary".to_string(),
                status: "active".to_string(),
            });
            // 合并该 turn 内所有消息内容（含引用、文件）
            let room_msgs = store.messages.get(&turn.room_id).cloned().unwrap_or_default();
            let mut content = String::new();
            let mut refs: Vec<CollabReference> = Vec::new();
            let mut files: Vec<CollabFileRef> = Vec::new();
            for mid in &turn.message_ids {
                if let Some(m) = room_msgs.iter().find(|m| m.id == *mid) {
                    if !content.is_empty() {
                        content.push_str("\n\n");
                    }
                    content.push_str(m.content.trim());
                    refs.extend(m.references.clone());
                    files.extend(m.files.clone());
                }
            }
            let placeholder_id = turn.dispatch_message_id.clone().unwrap_or_default();
            let prompt = build_prompt(&content, &refs, &files);
            let model_id = turn.model_id.clone();
            let provider_id = turn.provider_id.clone();
            let options = turn.options.clone();
            let turn_id = turn.id.clone();
            out.push(DispatchJob {
                room_id: turn.room_id.clone(),
                tool_id: owner.clone(),
                placeholder_id,
                prompt,
                model_id,
                provider_id,
                options,
                turn_id: turn_id.clone(),
                trigger_message_id: turn.trigger_message_id.clone(),
            });
            // 更新 agent 在线状态为 working（字段级借用，避免与 turn 可变借冲突）
            if let Some(a) = store.agents.iter_mut().find(|a| a.tool_id == owner) {
                a.status = "working".to_string();
                a.current_room = Some(turn.room_id.clone());
                a.last_heartbeat = now_rfc3339();
            } else {
                store.agents.push(CollabAgentStatus {
                    tool_id: owner.clone(),
                    status: "working".to_string(),
                    current_room: Some(turn.room_id.clone()),
                    last_heartbeat: now_rfc3339(),
                });
            }
        }
        save_store(&store).ok();
        out
    }; // 释放 STORE 锁

    for job in jobs {
        spawn_turn_dispatch(&app, job).await;
    }
}

/// 锁外执行实际派发，避免 std Mutex 跨 await
async fn spawn_turn_dispatch(app: &tauri::AppHandle, job: DispatchJob) {
    let dispatch_key = format!("{}::{}", job.room_id, job.tool_id);
    let active = ACTIVE_DISPATCHES.get_or_init(|| tokio::sync::Mutex::new(HashSet::new()));
    let mut active_guard = active.lock().await;
    if active_guard.contains(&dispatch_key) {
        // 已被占用：标记 turn 回 collecting 以便后续重试
        let _lock = STORE_LOCK.get_or_init(|| Mutex::new(())).lock();
        let mut store = load_store();
        if let Some(t) = store.turns.iter_mut().find(|t| t.id == job.turn_id) {
            t.state = "collecting".to_string();
        }
        save_store(&store).ok();
        return;
    }
    active_guard.insert(dispatch_key.clone());
    drop(active_guard);

    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        dispatch_to_tool(
            &app_clone,
            job.room_id.clone(),
            job.tool_id.clone(),
            load_room_project_path(&job.room_id),
            job.prompt,
            job.placeholder_id.clone(),
            job.model_id.clone(),
            job.provider_id.clone(),
            job.options.clone(),
        ).await;
        if let Some(active) = ACTIVE_DISPATCHES.get() {
            active.lock().await.remove(&dispatch_key);
        }
        // 派发完成：turn 置 completed，更新 agent 状态为 online
        let _lock = STORE_LOCK.get_or_init(|| Mutex::new(())).lock();
        let mut store = load_store();
        if let Some(t) = store.turns.iter_mut().find(|t| t.id == job.turn_id) {
            t.state = "completed".to_string();
        }
        set_agent_status(&mut store, &job.tool_id, "online", Some(&job.room_id));
        // 清理该 turn 的 reply_grants（active → consumed）
        for g in store.reply_grants.iter_mut() {
            if g.message_id == job.trigger_message_id {
                g.status = "consumed".to_string();
            }
        }
        save_store(&store).ok();
    });
}

/// 自动路由：无显式 @ 时选择 owner（sticky owner + 负载均衡）。
/// 当前前端为单选模式（手动 @ 必有 owner），此函数为后续多 agent 协同预留。
fn route_owner(messages: &HashMap<String, Vec<CollabMessage>>, room_id: &str) -> String {
    // 1. sticky owner：该房间最近一次由谁回复过 → 优先
    if let Some(recent) = messages.get(room_id) {
        for m in recent.iter().rev() {
            if m.sender != "user" && m.dispatch.is_some() {
                return m.sender.clone();
            }
        }
    }
    String::new()
}

/// 更新（或插入）某工具的在线状态
fn set_agent_status(store: &mut CollabStore, tool_id: &str, status: &str, room: Option<&str>) {
    if let Some(a) = store.agents.iter_mut().find(|a| a.tool_id == tool_id) {
        a.status = status.to_string();
        a.current_room = room.map(|r| r.to_string());
        a.last_heartbeat = now_rfc3339();
    } else {
        store.agents.push(CollabAgentStatus {
            tool_id: tool_id.to_string(),
            status: status.to_string(),
            current_room: room.map(|r| r.to_string()),
            last_heartbeat: now_rfc3339(),
        });
    }
}

/// 获取房间绑定的项目路径（用于派发工作目录）
fn load_room_project_path(room_id: &str) -> String {
    load_store()
        .rooms
        .iter()
        .find(|r| r.id == room_id)
        .map(|r| r.project_path.clone())
        .unwrap_or_default()
}

/// B 层：工具回复完成后，解析内容中的 @tool 提及，触发因果唤醒（新 turn）。
/// 带深度/次数限制防循环风暴。在 dispatch_to_tool 收尾后调用。
/// parent_msg_id 为触发唤醒的父消息，用于在被唤醒消息上设置 reply_to 形成对话链。
pub fn wake_tools_from_reply(room_id: &str, from_tool: &str, content: &str, root_turn_id: &str, depth: u32, parent_msg_id: &str) {
    if depth >= MAX_AGENT_WAKE_DEPTH {
        eprintln!("[collab] 唤醒深度已达上限({})，停止唤醒", MAX_AGENT_WAKE_DEPTH);
        return;
    }
    let mentions = parse_mentions(content);
    if mentions.is_empty() {
        return;
    }
    let _lock = STORE_LOCK.get_or_init(|| Mutex::new(())).lock();
    let mut store = load_store();
    // 统计该 root 已唤醒次数
    let root_wakes: u32 = store.turns.iter()
        .filter(|t| t.causal_root_id == root_turn_id)
        .map(|t| t.woken_tools.len() as u32)
        .sum();
    let mut slot = 0u32;
    for (raw, instr) in mentions {
        // 别名解析：id / 大小写 / nickname 均可；解析不到则跳过，避免垃圾 turn
        let tool = match resolve_mention(&raw) {
            Some(t) => t,
            None => {
                eprintln!("[collab] 提及 @{} 无法解析为已知工具，跳过", raw);
                continue;
            }
        };
        if tool == from_tool {
            continue; // 不自我唤醒
        }
        // 已被本 root 唤醒过的工具不再重复唤醒
        if store.turns.iter().any(|t| t.causal_root_id == root_turn_id
            && t.woken_tools.contains(&tool)) {
            continue;
        }
        if root_wakes + slot >= MAX_AGENT_WAKES_PER_ROOT {
            eprintln!("[collab] 根 {} 唤醒次数达上限({})，跳过 {}", root_turn_id, MAX_AGENT_WAKES_PER_ROOT, tool);
            continue;
        }
        slot += 1;
        // 被唤醒工具的委派指令：提取 @tool 后的文本；为空则回退到整段父回复作为上下文
        let req = if instr.is_empty() {
            content.to_string()
        } else {
            instr.clone()
        };
        let req = format!("[来自 @{} 的协作委派请求]\n\n{}", from_tool, req);
        // 创建唤醒 turn（directed slot），立即到期派发
        let turn_id = new_id();
        let placeholder_id = new_id();
        if let Some(room_msgs) = store.messages.get_mut(room_id) {
            room_msgs.push(CollabMessage {
                id: placeholder_id.clone(),
                room_id: room_id.to_string(),
                sender: tool.clone(),
                sender_name: registry().get_tool_config(&tool)
                    .and_then(|c| c.nickname.clone().filter(|n| !n.trim().is_empty()))
                    .unwrap_or_else(|| tool.clone()),
                avatar: registry().get_tool_config(&tool).and_then(|c| c.avatar.clone()),
                content: req,
                references: vec![],
                files: vec![],
                dispatch: Some(CollabDispatch {
                    tool_id: tool.clone(),
                    session_id: String::new(),
                    model: None,
                    duration_ms: None,
                    usage: None,
                }),
                reply_to: Some(parent_msg_id.to_string()),
                status: Some("running".to_string()),
                created_at: now_str(),
            });
        }
        store.turns.push(CollabTurn {
            id: turn_id.clone(),
            room_id: room_id.to_string(),
            sender: from_tool.to_string(),
            state: "collecting".to_string(),
            message_ids: vec![placeholder_id.clone()],
            trigger_message_id: placeholder_id.clone(),
            latest_message_id: placeholder_id.clone(),
            dispatch_after: now_rfc3339(), // 立即到期
            owner_tool_id: Some(tool.clone()),
            project_path: load_room_project_path(room_id),
            model_id: None,
            provider_id: None,
            options: CollabDispatchOptions::default(),
            dispatch_message_id: Some(placeholder_id.clone()),
            causal_root_id: root_turn_id.to_string(),
            causal_depth: depth + 1,
            woken_tools: vec![tool.clone()],
            created_at: now_rfc3339(),
            updated_at: now_rfc3339(),
        });
        // 记录唤醒到根 turn 的 woken_tools（累加）
        if let Some(root) = store.turns.iter_mut().find(|t| t.id == root_turn_id) {
            if !root.woken_tools.contains(&tool) {
                root.woken_tools.push(tool.clone());
            }
        }
        store.reply_grants.push(ReplyGrant {
            id: new_id(),
            message_id: placeholder_id.clone(),
            tool_id: tool.clone(),
            slot: "directed".to_string(),
            status: "reserved".to_string(),
        });
        eprintln!("[collab] 因果唤醒: {} @{} (depth={}, root={})", from_tool, tool, depth + 1, root_turn_id);
    }
    save_store(&store).ok();
}

/// 解析内容中的 @toolId 提及，返回 (原始提及, 该提及后的指令文本)。
/// @ 后跟非空白非 @ 字符序列（至空白/@ 结束）为 toolId；其后直到下一个 @ 或文末为该工具的委派指令。
fn parse_mentions(content: &str) -> Vec<(String, String)> {
    let bytes = content.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'@' {
            let mut j = i + 1;
            while j < bytes.len()
                && bytes[j] != b' '
                && bytes[j] != b'\t'
                && bytes[j] != b'\n'
                && bytes[j] != b'\r'
                && bytes[j] != b'@'
            {
                j += 1;
            }
            if j > i + 1 {
                let raw = content[i + 1..j].to_string();
                // 指令：从提及结束处取到下一个 @ 或文末
                let mut k = j;
                while k < bytes.len() && bytes[k] != b'@' {
                    k += 1;
                }
                let instr = content[j..k].trim().to_string();
                out.push((raw, instr));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// 将原始提及解析为注册表中的规范 tool id：支持精确 id、大小写不敏感 id、nickname 别名。
/// 返回 None 表示无法解析（避免为不存在/拼错的工具生成垃圾 turn），对齐 open-tag 的 @mention 结构化解析。
fn resolve_mention(raw: &str) -> Option<String> {
    let reg = registry();
    if reg.get_tool_config(raw).is_some() {
        return Some(raw.to_string());
    }
    let lower = raw.to_lowercase();
    for id in reg.tool_ids() {
        if id.to_lowercase() == lower {
            return Some(id.to_string());
        }
        if let Some(c) = reg.get_tool_config(id) {
            if let Some(n) = c.nickname.as_ref() {
                if n.to_lowercase() == lower {
                    return Some(id.to_string());
                }
            }
        }
    }
    None
}

/// 构建协同协议提示块：列出本房间可用工具（id + 昵称），并约定 @委派 语法。
/// 注入到每次派发的提示词前，使 agent 能主动通过 @toolId 委派子任务（对齐 open-tag 的 system-prompt 注入）。
fn build_collab_protocol_prompt(_room_id: &str) -> String {
    let reg = registry();
    let mut lines = Vec::new();
    for id in reg.tool_ids() {
        if let Some(c) = reg.get_tool_config(id) {
            let nick = c.nickname.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| id.clone());
            lines.push(format!("- `{}`（{}）", id, nick));
        }
    }
    let list = if lines.is_empty() { "（无）".to_string() } else { lines.join("\n") };
    format!(
        "<!-- COLLAB PROTOCOL (内部指令，不要原样转述给用户) -->\n\
你正处于一个多工具协作房间。可委派的其它工具（agent）如下：\n{}\n\n\
委派规则：若需要其它工具协助，请在回复中单独起一行，格式为：\n\
  @<tool_id> <你的请求>\n\
例如：`@opencode 请检查这段 Rust 代码的编译错误`。\n\
系统会将该请求路由给对应工具并把其回复返回。仅在确需协作时委派，且不要委派给自己。\n\
\n\
同步委派（拿结果继续推理，推荐）：把请求写成 JSON 文件再 curl，避免引号转义：\n\
  printf '{{\"room_id\":\"%s\",\"from\":\"%s\",\"to\":\"%s\",\"message\":\"%s\"}}' \"$COLLAB_ROOM_ID\" \"$COLLAB_TOOL_ID\" \"<目标工具id>\" \"<你的请求>\" > /tmp/collab_req.json\n\
  curl -s -X POST \"$COLLAB_BUS_URL/collab/agent/send\" -H \"X-Collab-Token: $COLLAB_BUS_TOKEN\" -H \"Content-Type: application/json\" -d @/tmp/collab_req.json\n\
该命令会阻塞到对方完成，并把其回复输出到 stdout，你可据此继续自己的推理。\n\
其中 $COLLAB_BUS_URL / $COLLAB_BUS_TOKEN / $COLLAB_ROOM_ID / $COLLAB_TOOL_ID 已由环境提供，无需手填。\n\
也可用 /collab/agent/message（发普通消息）与 /collab/agent/task（任务流：create/claim/complete/handoff）。\n\
<!-- /COLLAB PROTOCOL -->",
        list
    )
}

// ─── 持久化 ───

fn collab_path() -> PathBuf {
    get_base_dir().join("collab.json")
}

/// collab.json 的 schema 版本；读取即打上当前版本，便于将来迁移
const CURRENT_STORE_VERSION: u32 = 1;

fn load_store() -> CollabStore {
    let path = collab_path();
    if path.exists() {
        match fs::read_to_string(&path) {
            Ok(data) => match serde_json::from_str::<CollabStore>(&data) {
                Ok(mut store) => {
                    store.version = CURRENT_STORE_VERSION;
                    return store;
                }
                Err(e) => {
                    // 解析失败：保留损坏文件并备份，避免静默丢弃数据（运维可据此恢复）
                    eprintln!("[collab] ⚠ collab.json 解析失败，已备份原文件为 .corrupt: {}", e);
                    let _ = fs::copy(&path, path.with_extension("json.corrupt"));
                }
            },
            Err(e) => eprintln!("[collab] ⚠ 读取 collab.json 失败: {}", e),
        }
    }
    let mut s = CollabStore::default();
    s.version = CURRENT_STORE_VERSION;
    s
}

fn save_store(store: &CollabStore) -> Result<(), String> {
    let path = collab_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let data = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
    // 原子写：先写临时文件再 rename，避免写入中途崩溃导致 collab.json 损坏/数据丢失
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, &data).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

/// 保护 collab.json 的读-改-写临界区：所有 load_store + 修改 + save_store 必须在该锁内完成，
/// 避免并发派发（不同房间）互相覆盖导致丢失更新。
static STORE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

static ID_COUNTER: AtomicU64 = AtomicU64::new(0);
fn new_id() -> String {
    let n = ID_COUNTER.fetch_add(1, Ordering::SeqCst);
    let ts = chrono::Local::now().timestamp_millis();
    format!("m{}_{}", ts, n)
}

fn now_str() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

/// 兼容的 updated_at 比较：约定由 now_str() 生成（"%Y-%m-%dT%H:%M:%S"，字典序即时间序），
/// 同时兼容 RFC3339（含时区偏移）解析比较，任一解析失败时回退字典序。返回降序（新→旧）。
fn cmp_updated_at(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |s: &str| chrono::DateTime::parse_from_rfc3339(s).map(|d| d.timestamp()).ok();
    match (parse(a), parse(b)) {
        (Some(x), Some(y)) => y.cmp(&x),
        _ => b.cmp(a),
    }
}

// ─── 房间管理 ───

#[tauri::command]
pub fn collab_create_room(name: String, project_path: String) -> Result<CollabRoom, String> {
    eprintln!("[collab] 创建房间: name={}, project={}", name, project_path);
    // 进入读-改-写临界区，避免并发丢失更新
    let _store_lock = STORE_LOCK.get_or_init(|| Mutex::new(())).lock();
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
    eprintln!("[collab] 房间已创建: id={}, name={}", room.id, room.name);
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
    // 按 updated_at 降序（使用兼容比较，避免格式不一致导致的排序错位）
    rooms.sort_by(|a, b| cmp_updated_at(&a.updated_at, &b.updated_at));
    let total = rooms.len();
    let limit = limit.unwrap_or(20).max(1);
    let offset = offset.unwrap_or(0);
    let end = (offset + limit).min(total);
    let page = if offset <= total { rooms[offset..end].to_vec() } else { vec![] };
    let has_more = end < total;
    Ok(CollabRoomPage { rooms: page, has_more, total })
}

/// 会话消息（分页；按时间正序 oldest→newest）
#[derive(Serialize)]
pub struct CollabMessagePage {
    pub messages: Vec<CollabMessage>,
    pub has_more: bool,
    pub total: usize,
}

#[tauri::command]
pub fn collab_get_messages(
    room_id: String,
    offset: Option<usize>,
    limit: Option<usize>,
    tail: Option<bool>,
) -> Result<CollabMessagePage, String> {
    let all = load_store().messages.get(&room_id).cloned().unwrap_or_default();
    let total = all.len();
    let limit = limit.unwrap_or(50).max(1);
    // tail=true 时返回末尾一页（聊天视口初始加载最新消息）
    let mut offset = offset.unwrap_or(0);
    if tail.unwrap_or(false) {
        offset = total.saturating_sub(limit);
    }
    let end = (offset + limit).min(total);
    let messages = if offset <= total { all[offset..end].to_vec() } else { vec![] };
    // has_more 表示在已加载的最旧消息之前是否还有更早的消息
    let has_more = offset > 0;
    Ok(CollabMessagePage { messages, has_more, total })
}

#[tauri::command]
pub fn collab_delete_room(room_id: String) -> Result<(), String> {
    eprintln!("[collab] 删除房间: id={}", room_id);
    let _store_lock = STORE_LOCK.get_or_init(|| Mutex::new(())).lock();
    let mut store = load_store();
    let msg_count = store.messages.get(&room_id).map(|m| m.len()).unwrap_or(0);
    store.rooms.retain(|r| r.id != room_id);
    store.messages.remove(&room_id);
    eprintln!("[collab] 房间已删除: id={}, 消息数={}", room_id, msg_count);
    // 清理该房间相关的工具会话标记
    let keys: Vec<String> = store.tool_sessions.keys()
        .filter(|k| k.starts_with(&format!("{}::", room_id)))
        .cloned()
        .collect();
    for k in keys { store.tool_sessions.remove(&k); }
    // 清理该房间相关的上下文快照
    let snap_keys: Vec<String> = store.context_snapshots.keys()
        .filter(|k| k.starts_with(&format!("{}::", room_id)))
        .cloned()
        .collect();
    for k in snap_keys { store.context_snapshots.remove(&k); }
    // 停止该房间相关的所有常驻代理
    stop_room_proxies(&room_id);
    save_store(&store)
}

// ─── 发送消息 + 派发 ───

/// 协同派发高级协议参数（与 LaunchAiToolRequest 对齐）
/// 使用 #[serde(default)] 使前端不传时全部回退到 None/false
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
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
    /// 模型自定义启动参数模板（决定如何传参）
    #[serde(default)]
    pub custom_params: Vec<ModelCustomParam>,
    /// 用户为模型自定义参数选中的取值（key → 值）
    #[serde(default)]
    pub custom_param_values: HashMap<String, String>,
}

/// 派发轮次：同一发送者在静默窗口内的多条消息合并为一次派发。
///
/// 状态机：collecting → ready → dispatching → completed / blocked
/// - collecting：窗口期内持续合并新消息
/// - ready：窗口到期，等待路由选 owner
/// - dispatching：已 spawn 派发
/// - completed / blocked：终态
///
/// 注：本结构体放在 `CollabDispatchOptions` 之后，以便 `options` 字段直接复用该类型。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CollabTurn {
    pub id: String,
    pub room_id: String,
    /// "user" 或工具 id（谁发起本轮）
    pub sender: String,
    pub state: String,
    /// 合并的消息 id 列表（按时间正序）
    pub message_ids: Vec<String>,
    /// 触发本轮的第一条消息 id
    pub trigger_message_id: String,
    /// 最近一条消息 id（用于延长窗口）
    pub latest_message_id: String,
    /// 静默窗口到期时间（UTC 时间戳字符串），到期才派发
    pub dispatch_after: String,
    /// 负责回复的工具 id（路由分配结果；手动 @ 时直接赋值）
    pub owner_tool_id: Option<String>,
    /// 派发工作目录（项目路径）
    pub project_path: String,
    /// 选中的模型 id
    pub model_id: Option<String>,
    /// 选中的 provider id
    pub provider_id: Option<String>,
    /// 派发选项（模型伪装、回退、优化器等）
    pub options: CollabDispatchOptions,
    /// 本轮派发对应的占位消息 id（predictive placeholder，窗口期内复用同一占位）
    pub dispatch_message_id: Option<String>,
    /// 因果链根 turn id（用于唤醒深度限制，防循环风暴）
    pub causal_root_id: String,
    /// 因果深度：0=用户直接发；1=工具回复触发的唤醒；依次 +1
    pub causal_depth: u32,
    /// 本轮已唤醒的工具 id 列表（防重复唤醒同一工具）
    pub woken_tools: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
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
    eprintln!("[collab] ▶ 发送消息: room={}, tool={}, model={:?}, provider={:?}, content_len={}, refs={}, files={}",
        room_id, tool_id, model_id, provider_id, content.len(), references.len(), files.len());
    // 读-改-写临界区：仅覆盖“加载→修改→保存”的同步段，持锁不跨越任何 await，
    // 保证 Tauri 命令的 future: Send（避免 std MutexGuard 跨 await）。
    let user_msg = CollabMessage {
        id: new_id(),
        room_id: room_id.clone(),
        sender: "user".to_string(),
        sender_name: "我".to_string(),
        avatar: None,
        content,
        references,
        files,
        dispatch: None,
        reply_to: None,
        status: None,
        created_at: now_str(),
    };
    let mut result = vec![user_msg.clone()];
    {
        let _store_lock = STORE_LOCK.get_or_init(|| Mutex::new(())).lock();
        let mut store = load_store();
        if !store.rooms.iter().any(|r| r.id == room_id) {
            return Err("会话不存在".to_string());
        }
        let room_messages = store.messages.entry(room_id.clone()).or_default();
        room_messages.push(user_msg.clone());

        if !tool_id.trim().is_empty() {
            let tool_config = match registry().get_tool_config(&tool_id) {
                Some(c) => c.clone(),
                None => return Err(format!("未知工具：{}", tool_id)),
            };
            let now_ts = parse_ts(&now_rfc3339()).unwrap_or(0);
            // 查找窗口期内的 collecting turn：同一用户、同房间、手动指定同一工具
            let mut turn_idx: Option<usize> = None;
            for (i, t) in store.turns.iter().enumerate() {
                if t.state == "collecting" && t.room_id == room_id
                    && t.sender == "user"
                    && t.owner_tool_id.as_deref() == Some(tool_id.as_str())
                    && parse_ts(&t.dispatch_after).unwrap_or(0) > now_ts {
                    turn_idx = Some(i);
                    break;
                }
            }
            let turn_id: String;
            if let Some(idx) = turn_idx {
                // 复用现有窗口期 turn（快速连发合并）
                turn_id = store.turns[idx].id.clone();
            } else {
                // 新建 turn + predictive 占位消息（立即返回给前端渲染“思考中”）
                turn_id = new_id();
                let placeholder_id = new_id();
                let placeholder = CollabMessage {
                    id: placeholder_id.clone(),
                    room_id: room_id.clone(),
                    sender: tool_id.clone(),
                    sender_name: tool_config.nickname.clone()
                        .filter(|n| !n.trim().is_empty())
                        .unwrap_or_else(|| tool_config.display_name.clone()),
                    avatar: tool_config.avatar.clone(),
                    content: String::new(),
                    references: vec![],
                    files: vec![],
                    dispatch: Some(CollabDispatch {
                        tool_id: tool_id.clone(),
                        session_id: String::new(),
                        model: model_id.clone(),
                        duration_ms: None,
                        usage: None,
                    }),
                    reply_to: Some(user_msg.id.clone()),
                    status: Some("running".to_string()),
                    created_at: now_str(),
                };
                room_messages.push(placeholder.clone());
                result.push(placeholder.clone());
                store.turns.push(CollabTurn {
                    id: turn_id.clone(),
                    room_id: room_id.clone(),
                    sender: "user".to_string(),
                    state: "collecting".to_string(),
                    message_ids: Vec::new(),
                    trigger_message_id: user_msg.id.clone(),
                    latest_message_id: user_msg.id.clone(),
                    dispatch_after: dispatch_after_ts(),
                    owner_tool_id: Some(tool_id.clone()),
                    project_path: project_path.clone(),
                    model_id: model_id.clone(),
                    provider_id: provider_id.clone(),
                    options: options.clone().unwrap_or_default(),
                    dispatch_message_id: Some(placeholder_id.clone()),
                    causal_root_id: turn_id.clone(), // 用户发起：自身即为根
                    causal_depth: 0,
                    woken_tools: Vec::new(),
                    created_at: now_rfc3339(),
                    updated_at: now_rfc3339(),
                });
            }
            // 追加消息到 turn 并延长静默窗口
            if let Some(t) = store.turns.iter_mut().find(|t| t.id == turn_id) {
                t.message_ids.push(user_msg.id.clone());
                t.latest_message_id = user_msg.id.clone();
                t.dispatch_after = dispatch_after_ts();
                t.updated_at = now_rfc3339();
            }
        }
        if let Some(room) = store.rooms.iter_mut().find(|r| r.id == room_id) {
            room.updated_at = now_str();
        }
        save_store(&store)?;
    } // _store_lock 在此释放，早于首个 await

    // 触发 Turn 调度器：窗口到期后自动路由 + 派发（替代直接 spawn）
    ensure_turn_scheduler(app.clone());

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
// 用 tokio::sync::Mutex：其守卫是 Send，可在 async 命令中安全跨 await 持有，
// 避免 std MutexGuard 导致 Tauri 命令的 Future 非 Send 而无法编译。
static ACTIVE_DISPATCHES: OnceLock<tokio::sync::Mutex<HashSet<String>>> = OnceLock::new();

/// 压缩回调：dispatch_to_tool 完成后通过 oneshot 通知 collab_compact_session
static COMPACT_CALLBACKS: OnceLock<Mutex<HashMap<String, tokio::sync::oneshot::Sender<String>>>> = OnceLock::new();

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
    eprintln!("[collab] ⚠ Prompt 检测到: question='{}', options={:?}, msg={}", question, options, msg_id);
    // 记录 pending 状态
    if let Some(map) = PROMPT_STATE.get() {
        let g = map.lock();
        if let Some(ctrl) = g.get(msg_id) {
            let mut p = ctrl.pending.lock();
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
        let g = map.lock();
        if let Some(ctrl) = g.get(msg_id) {
            // 清除 pending 状态（无论写入结果如何，不再重试）
            *ctrl.pending.lock() = None;
            // 在同一把锁内检查并写入 stdin，避免 TOCTOU 竞态
            let mut guard = ctrl.stdin.lock();
            if let Some(ref mut stdin) = *guard {
                let _ = stdin.write_all(format!("{}\n", response).as_bytes());
                let _ = stdin.flush();
                eprintln!("[collab] ✓ prompt 已响应: response='{}', msg={}", response, msg_id);
                return true;
            }
            eprintln!("[collab] ⚠ stdin 已关闭，无法响应: msg={}", msg_id);
            return false;
        }
    }
    eprintln!("[collab] ⚠ 未找到 prompt 状态: msg={}", msg_id);
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
    app_handle: &tauri::AppHandle,
) -> (u16, String, String) {
    let key = room_proxy_key(room_id, tool_id);
    let map = ROOM_PROXIES.get_or_init(|| Mutex::new(HashMap::new()));
    let config = super::config::load_ai_config();
    let provider = provider_id.and_then(|pid| config.providers.iter().find(|p| p.id == pid));

    // 1. 检查是否已有常驻代理 → 健康检查通过才复用；失联则清除重建
    //（旧逻辑无脑复用，代理任务若已死，子进程连不上 base_url → gemini "fetch failed" 之类）
    let existing_port = { map.lock().get(&key).map(|e| e.port) };
    let mut reused_port: Option<u16> = None;
    if let Some(p0) = existing_port {
        if room_proxy_alive(p0).await {
            eprintln!("[collab] 复用常驻代理: key={}, port={}", key, p0);
            reused_port = Some(p0);
        } else {
            eprintln!("[collab] ⚠ 常驻代理失联(port={})，清除并重建: key={}", p0, key);
            if let Some(entry) = map.lock().remove(&key) {
                entry.abort_handle.abort();
            }
        }
    }

    let mut port: u16 = reused_port.unwrap_or(0);
    let mut abort_handle: Option<tokio::task::AbortHandle> = None;
    if reused_port.is_none() {
        eprintln!("[collab] 首次创建代理: key={}, tool={}, provider={:?}, model={:?}", key, tool_id, provider_id, model_id);
        if provider.is_none() {
            eprintln!("[collab] ⚠ 未找到 provider: {:?}", provider_id);
        }

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
            custom_params: options.custom_params.clone(),
            custom_param_values: options.custom_param_values.clone(),
            ..Default::default()
        };

        let (p1, h1) = start_tool_proxy_with_collab(
            tool_config, provider, &config, &req,
            Some(app_handle.clone()),
            Some(room_id.to_string()),
        ).await;
        port = p1;
        abort_handle = h1;
        eprintln!("[collab] 代理启动结果: port={}, has_abort={}", port, abort_handle.is_some());
    }

    let base_url = if port != 0 {
        format!("http://127.0.0.1:{}", port)
    } else {
        provider.map(|p| p.url_for(&tool_config.native_protocol())).unwrap_or_default()
    };
    let api_key = provider.map(|p| p.api_key.clone()).unwrap_or_default();

    // 每次派发都重写工具配置文件：普通启动（AI-工具页）可能在两次派发之间
    // 把共享配置（如 ~/.codex/config.toml）改写指向已关闭的启动代理端口。
    // 即使本地代理未启动（port==0），也写入 provider 上游 URL，避免 stale config
    // 导致工具始终使用旧模型（如 open-code 忽略自定义 env var，只读配置文件 model 字段）。
    if let Some(ref p) = provider {
        if !base_url.is_empty() {
            // 声明模型名 C：伪装优先，否则所选取模型 B
            let claimed_model = options.masquerade_model.clone()
                .filter(|c| !c.is_empty())
                .or_else(|| model_id.map(|s| s.to_string()));
            if let Err(e) = super::launch::write_tool_config_from_spec(
                tool_config,
                model_id,
                claimed_model.as_deref(),
                &base_url,
                &p.api_key,
                options.fallback_model_id.as_deref(),
                options.fallback_masquerade_model.as_deref(),
                options.one_m_context,
                options.fallback_one_m_context,
                true,
                &options.custom_params,
                &options.custom_param_values,
            ) {
                eprintln!("[collab] ⚠ 写入工具配置文件失败: {}", e);
            } else {
                eprintln!("[collab] ✓ 工具配置文件已写入（baseUrl → {}）", base_url);
            }
        }
    }

    // 存入全局表（仅新建时；复用路径 abort_handle 为 None 不会重复插入）
    if port != 0 {
        if let Some(h) = abort_handle {
            let mut g = map.lock();
            g.insert(key, RoomProxyEntry { port, abort_handle: h });
            eprintln!("[collab] 常驻代理已创建 port={}", port);
        }
    }

    (port, base_url, api_key)
}

/// 探测常驻代理是否存活（600ms 超时，直连不走系统代理）
async fn room_proxy_alive(port: u16) -> bool {
    let url = format!("http://127.0.0.1:{}/health", port);
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(600))
        .no_proxy()
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    match client.get(&url).send().await {
        Ok(r) => r.status().is_success(),
        Err(_) => false,
    }
}

/// 停止并移除房间+工具级的常驻代理
fn stop_room_proxy(room_id: &str, tool_id: &str) {
    let key = room_proxy_key(room_id, tool_id);
    if let Some(map) = ROOM_PROXIES.get() {
        let mut g = map.lock();
        if let Some(entry) = g.remove(&key) {
            entry.abort_handle.abort();
            eprintln!("[collab] 已停止代理 port={}", entry.port);
        }
    }
}

/// 停止某房间相关的所有常驻代理
fn stop_room_proxies(room_id: &str) {
    if let Some(map) = ROOM_PROXIES.get() {
        let mut g = map.lock();
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
    crate::exit_log::exit_log("cleanup: stop_all_room_proxies 进入");
    if let Some(map) = ROOM_PROXIES.get() {
        let mut g = map.lock();
        crate::exit_log::exit_log("cleanup: ROOM_PROXIES 锁已获取");
        let count = g.len();
        for (_, entry) in g.drain() {
            entry.abort_handle.abort();
        }
        if count > 0 {
            eprintln!("[collab] 已停止所有常驻代理（{} 个）", count);
        }
        crate::exit_log::exit_log(&format!("cleanup: 已 abort {} 个常驻代理", count));
    } else {
        crate::exit_log::exit_log("cleanup: ROOM_PROXIES 未初始化，跳过");
    }
}

// ─── 流式事件解析 ───

/// claude stream-json 的一行事件
enum StreamEvent {
    Delta(String),
    Result(String, Option<TokenUsage>),
    /// 活动状态（思考中/调用工具等），不写入最终内容，仅前端实时显示
    Activity(String),
    Ignore,
}

/// 工具运行时的流式输出解析适配器（每 runner / 工具族一个 adapter）。
///
/// 把原本散落的 parse_claude_json / parse_codex_json / parse_opencode_json /
/// parse_gemini_json 收敛为统一的 trait 接口；dispatch_to_tool 只依赖该 trait，
/// 新增工具只需新增一个 adapter 并在 `runner_adapter` 工厂登记，无需改动编排逻辑。
trait RunnerAdapter: Send + Sync {
    /// 该 runner 是否为 JSON 流式输出（决定读取循环是否逐行 JSON 解析）。
    /// 非 JSON runner 走“整段文本累加”兜底，不逐行解析。
    fn is_json(&self) -> bool {
        true
    }
    /// 解析一行 stdout 为内部流式事件（Delta / Result / Activity / Ignore）。
    /// 非 JSON runner 恒返回 None。
    fn parse(&self, line: &str) -> Option<(StreamEvent, Option<String>)>;
}

/// claude（`stream-json`）适配器
struct ClaudeRunner;
impl RunnerAdapter for ClaudeRunner {
    fn parse(&self, line: &str) -> Option<(StreamEvent, Option<String>)> {
        parse_claude_json(line)
    }
}

/// codex（`codex-json`）适配器
struct CodexRunner;
impl RunnerAdapter for CodexRunner {
    fn parse(&self, line: &str) -> Option<(StreamEvent, Option<String>)> {
        parse_codex_json(line)
    }
}

/// opencode（`opencode-json`）适配器
struct OpenCodeRunner;
impl RunnerAdapter for OpenCodeRunner {
    fn parse(&self, line: &str) -> Option<(StreamEvent, Option<String>)> {
        parse_opencode_json(line)
    }
}

/// gemini / qwen（`gemini-json`）适配器
struct GeminiRunner;
impl RunnerAdapter for GeminiRunner {
    fn parse(&self, line: &str) -> Option<(StreamEvent, Option<String>)> {
        parse_gemini_json(line)
    }
}

/// 兜底适配器：非 JSON runner（无流式协议），整段文本累加，不逐行解析
struct GenericRunner;
impl RunnerAdapter for GenericRunner {
    fn is_json(&self) -> bool {
        false
    }
    fn parse(&self, _line: &str) -> Option<(StreamEvent, Option<String>)> {
        None
    }
}

/// 按 tool_config.runner 选取对应 adapter（每工具一个 adapter 的入口）。
/// 未知 / 空 runner 一律走 GenericRunner 兜底。
fn runner_adapter(runner: &str) -> Box<dyn RunnerAdapter> {
    match runner {
        "stream-json" => Box::new(ClaudeRunner),
        "codex-json" => Box::new(CodexRunner),
        "opencode-json" => Box::new(OpenCodeRunner),
        "gemini-json" => Box::new(GeminiRunner),
        _ => Box::new(GenericRunner),
    }
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
            // claude stream-json 的 result 事件含 usage（Anthropic 协议）；其余字段无则 None
            let usage = v.get("usage").and_then(|u| {
                let i = u.get("input_tokens").and_then(|x| x.as_u64());
                let o = u.get("output_tokens").and_then(|x| x.as_u64());
                match (i, o) {
                    (Some(i), Some(o)) => Some(TokenUsage { input_tokens: i, output_tokens: o }),
                    _ => None,
                }
            });
            Some((StreamEvent::Result(text, usage), sid))
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
                    // 保留调用身份（call_id/id），避免同工具多次调用时无法区分（参照 cc-switch #5310）
                    let call_id = item
                        .get("call_id")
                        .and_then(|c| c.as_str())
                        .or_else(|| item.get("id").and_then(|c| c.as_str()))
                        .unwrap_or("");
                    let label = if call_id.is_empty() {
                        format!("使用工具: {}", tool)
                    } else {
                        format!("使用工具: {} (#{})", tool, call_id)
                    };
                    Some((StreamEvent::Activity(label), sid))
                }
                _ => Some((StreamEvent::Ignore, sid)),
            }
        }
        Some("turn.failed") => {
            let err = v.get("error").and_then(|e| e.as_str()).unwrap_or("turn failed").to_string();
            Some((StreamEvent::Result(format!("[error] {}", err), None), sid))
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
        return Some((StreamEvent::Result(text, None), sid));
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
                Some((StreamEvent::Result(format!("[error] {}", text), None), sid))
            } else {
                Some((StreamEvent::Result(text, None), sid))
            }
        }
        Some("user") => {
            // 用户消息回显 → 忽略
            Some((StreamEvent::Ignore, sid))
        }
        _ => Some((StreamEvent::Ignore, sid)),
    }
}

/// 转义用于 cmd /c 字符串拼接的参数：用双引号整体包裹，内部 " 转义为 ""。
/// 双引号内的 & | < > ^ 以及 % 均为字面量，可防止工具返回的 session_id 逃逸执行命令（RCE）。
fn escape_cmd_arg(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        if c == '"' {
            out.push_str("\"\"");
        } else {
            out.push(c);
        }
    }
    out.push('"');
    out
}

/// 对齐 cross-spawn 的 Windows 参数引号规则（用于 cmd /d /s /c "<full>" 的逐元素转义）：
/// - 空串 → ""
/// - 含空格 / 制表符 / 引号 → 用双引号包裹，内部 " 转义为 \"
/// - 否则原样返回
/// 与 escape_cmd_arg（无条件加引号、内部 "" 双写）不同：此函数仅在必要时加引号，
/// 配合 /s 标志使 cmd 正确解析含空格的程序路径/参数（cross-spawn 的成熟做法）。
#[cfg(windows)]
fn windows_quote(arg: &str) -> String {
    if arg.is_empty() {
        return "\"\"".to_string();
    }
    if arg.contains(' ') || arg.contains('\t') || arg.contains('"') {
        // 内部 " 必须转义为 ""（双写），这是 Windows cmd 的转义约定；
        // 不能用 \"（那是 CreateProcess argv 的约定），否则 /s 剥掉外层引号后，
        // 内部 \" 退化为字面引号，导致 prompt 中的 < > & | 暴露成重定向/管道符，命令被拆坏。
        let escaped: String = arg.replace('"', "\"\"");
        format!("\"{}\"", escaped)
    } else {
        arg.to_string()
    }
}

/// 把工具命令模板拆成参数数组（按空白分词，双引号仅作为分组、不被保留）。
/// 模板中的 {prompt}/{prompt_file}/{session_id} 占位符保留为独立 token，
/// 交由调用方替换为实际参数，避免整体经 shell 解释。
fn tokenize_template(tmpl: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    for ch in tmpl.chars() {
        if ch == '"' {
            in_q = !in_q;
        } else if ch.is_whitespace() && !in_q {
            if !cur.is_empty() {
                tokens.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(ch);
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

/// 像 cmd.exe 一样，按 PATH + PATHEXT 把无扩展名的程序名解析为真实可执行文件路径。
/// Windows 上 Rust 的 Command::new 不会自动补全扩展名（bare "claude" 即使 claude.exe 在 PATH 中
/// 也会报 "program not found"），这里在 PATH 中查找 .exe/.cmd 等候选并返回完整路径；
/// 找不到则原样返回（交给后续 spawn 报错，错误信息更准确）。
/// 通过 `cmd /c where <program>` 让系统 shell 自行解析。
/// 用于 PATH / 额外目录都搜不到的情况（如用户自定义 npm prefix 导致全局 bin 不在 PATH 中，
/// 但 shell 自己能 `where` 到）。
/// 优先返回带 Windows 可执行扩展名（.exe/.cmd/.bat/.com）的匹配；npm 全局在 Windows 下会同时
/// 生成无扩展名的 sh 垫片与 .cmd 垫片，`where` 可能先给出无扩展名那一个，而它无法直接 CreateProcess
/// （os error 193: 不是有效的 Win32 应用程序），因此必须优先选 .cmd/.exe 等可执行的。
#[cfg(windows)]
fn resolve_via_where(program: &str) -> Option<String> {
    let mut c = std::process::Command::new("cmd");
    c.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let out = c
        .args(["/c", "where", program])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let mut first: Option<String> = None;
    for line in s.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if first.is_none() {
            first = Some(line.to_string());
        }
        if let Some(ext) = std::path::Path::new(line).extension().and_then(|e| e.to_str()) {
            let e = ext.to_ascii_lowercase();
            if e == "exe" || e == "cmd" || e == "bat" || e == "com" || e == "btm" {
                return Some(line.to_string());
            }
        }
    }
    first
}

/// 额外需要纳入搜索的目录（GUI 进程常缺少这些 PATH）：
/// 主要是 npm/pnpm/yarn 全局 bin，以及 nvm/fnm 等 node 版本管理器的安装位置。
#[cfg(windows)]
fn extra_program_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    let add = |dirs: &mut Vec<std::path::PathBuf>, base: Option<String>, rel: &[&str]| {
        if let Some(b) = base {
            let base = std::path::Path::new(&b);
            let mut p = base.to_path_buf();
            for r in rel {
                p.push(r);
            }
            dirs.push(p);
        }
    };
    if let Ok(appdata) = std::env::var("APPDATA") {
        add(&mut dirs, Some(appdata.clone()), &["npm"]);
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        add(&mut dirs, Some(local.clone()), &["npm"]);
    }
    if let Ok(profile) = std::env::var("USERPROFILE") {
        add(&mut dirs, Some(profile.clone()), &[".cargo", "bin"]);
        add(&mut dirs, Some(profile.clone()), &["AppData", "Roaming", "npm"]);
        add(&mut dirs, Some(profile.clone()), &["AppData", "Local", "npm"]);
        add(&mut dirs, Some(profile.clone()), &["AppData", "Roaming", "nvm"]);
        add(&mut dirs, Some(profile.clone()), &["scoop", "shims"]);
    }
    dirs
}

fn resolve_program(program: &str) -> String {
    if program.is_empty() {
        return program.to_string();
    }
    let p = std::path::Path::new(program);
    // 带路径分隔符/盘符视为绝对或相对路径，不解析
    if p.is_absolute() || program.contains('\\') || program.contains('/') {
        return program.to_string();
    }
    #[cfg(not(windows))]
    {
        // 非 Windows 下扩展名解析由 shell 处理，原样返回即可
        let _ = p;
        return program.to_string();
    }
    #[cfg(windows)]
    {
        let has_ext = p.extension().is_some();
        let pathext =
            std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
        let exts: Vec<String> = if has_ext {
            Vec::new()
        } else {
            pathext
                .split(';')
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty())
                .collect()
        };
        // PATH 目录 + 额外常见全局安装目录一起搜索
        let mut dirs: Vec<String> = std::env::var("PATH")
            .unwrap_or_default()
            .split(';')
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
            .collect();
        for d in extra_program_dirs() {
            if let Some(s) = d.to_str() {
                dirs.push(s.to_string());
            }
        }
        for dir in &dirs {
            if dir.is_empty() {
                continue;
            }
            let dir = std::path::Path::new(dir);
            if has_ext {
                let cand = dir.join(program);
                if cand.is_file() {
                    return cand.to_string_lossy().to_string();
                }
            } else {
                for e in &exts {
                    let name = format!("{}.{}", program, e);
                    let cand = dir.join(&name);
                    if cand.is_file() {
                        return cand.to_string_lossy().to_string();
                    }
                }
            }
        }
        // 兜底：PATH / 额外目录都搜不到时，交给系统 shell 的 where 解析
        // （应对用户自定义 npm prefix 等导致全局 bin 不在任何已知目录的情况）
        if let Some(p) = resolve_via_where(program) {
            return p;
        }
        program.to_string()
    }
}

/// Windows 上 npm 全局工具通常是 .cmd/.bat 垫片，CreateProcess 无法直接执行，
/// 必须退回 cmd /c。其余（含 Unix 下带 shebang 的符号链接）可直接 spawn。
fn is_windows_shell_shim(program: &str) -> bool {
    #[cfg(windows)] {
        if program.is_empty() {
            return false;
        }
        if let Some(ext) = std::path::Path::new(program).extension().and_then(|e| e.to_str()) {
            let e = ext.to_ascii_lowercase();
            if e == "cmd" || e == "bat" || e == "btm" || e == "ps1" {
                return true;
            }
        }
        let pathext =
            std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
        let exts: Vec<String> = pathext
            .split(';')
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        let path = std::env::var("PATH").unwrap_or_default();
        for dir in path.split(';') {
            if dir.is_empty() {
                continue;
            }
            let base = std::path::Path::new(dir).join(program);
            let candidates = if base.extension().is_some() {
                vec![base.clone()]
            } else {
                exts.iter()
                    .map(|e| {
                        let mut name = program.to_string();
                        if !e.starts_with('.') {
                            name.push('.');
                        }
                        name.push_str(e.trim_start_matches('.'));
                        std::path::Path::new(dir).join(name)
                    })
                    .collect()
            };
            for c in candidates {
                if c.is_file() {
                    if let Some(ext) = c.extension().and_then(|e| e.to_str()) {
                        let e = ext.to_ascii_lowercase();
                        return e == "cmd" || e == "bat" || e == "btm" || e == "ps1";
                    }
                    return false;
                }
            }
        }
        false
    }
    #[cfg(not(windows))] {
        let _ = program;
        false
    }
}

/// 杀掉进程树（Windows 用 taskkill /T，否则直接 kill）
fn kill_tree(child: &Arc<Mutex<Option<Child>>>) {
    // 先取出子进程句柄并立即释放锁，再执行可能阻塞的 kill / taskkill，
    // 避免持锁期间阻塞（taskkill 是外部进程调用）导致其他写操作饿死。
    let taken = child.lock().take();
    if let Some(c) = taken {
        #[cfg(windows)]
        {
            let pid = c.id();
            let mut tcmd = Command::new("taskkill");
            tcmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
            let _ = tcmd.args(["/F", "/T", "/PID", &pid.to_string()]).output();
        }
        #[cfg(not(windows))]
        {
            let mut c = c;
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
    duration_ms: Option<u64>,
    usage: Option<TokenUsage>,
    // 若提供 (session_key, session_id)，则在同一 store 写入中保存 tool_sessions，避免额外的 load+save 循环
    session_update: Option<(String, String)>,
) {
    let _store_lock = STORE_LOCK.get_or_init(|| Mutex::new(())).lock();
    let mut store = load_store();
    // 回填委派总线：若本条消息是一次同步委派的被唤起方，唤醒等待的调用 agent
    complete_delegation(msg_id, status, &content);
    if let Some(msgs) = store.messages.get_mut(room_id) {
        if let Some(m) = msgs.iter_mut().find(|m| m.id == msg_id) {
            m.content = content;
            m.status = Some(status.to_string());
            // 保留原始 created_at（创建时间），不覆盖为完成时间
            if let Some(d) = m.dispatch.as_mut() {
                if let Some(s) = sid {
                    d.session_id = s;
                }
                d.duration_ms = duration_ms;
                d.usage = usage;
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

/// C 方案：agent 主动同步委派。由 HTTP 总线（见 proxy/server.rs 的 /collab/agent/send）调用。
/// 创建可见的「委派」消息 + 被委派工具的 turn，并注册到委派总线；调用方阻塞等待被委派工具完成取回结果。
pub async fn agent_delegate(
    app: tauri::AppHandle,
    room_id: &str,
    from: &str,
    to_raw: &str,
    message: &str,
    parent: Option<&str>,
) -> String {
    let to = match resolve_mention(to_raw) {
        Some(t) => t,
        None => return format!("{{\"error\":\"unknown tool: {}\"}}", to_raw),
    };
    if to == from {
        return "{\"error\":\"cannot delegate to self\"}".to_string();
    }
    let _lock = STORE_LOCK.get_or_init(|| Mutex::new(())).lock();
    let mut store = load_store();
    let from_msg_id = new_id();
    let to_msg_id = new_id();
    let request_id = new_id();
    if let Some(msgs) = store.messages.get_mut(room_id) {
        msgs.push(CollabMessage {
            id: from_msg_id.clone(),
            room_id: room_id.to_string(),
            sender: from.to_string(),
            sender_name: registry()
                .get_tool_config(from)
                .and_then(|c| c.nickname.clone().filter(|n| !n.trim().is_empty()))
                .unwrap_or_else(|| from.to_string()),
            avatar: registry().get_tool_config(from).and_then(|c| c.avatar.clone()),
            content: format!("[委派] @{} {}", to, message),
            references: vec![],
            files: vec![],
            dispatch: None,
            reply_to: parent.map(|p| p.to_string()),
            status: None,
            created_at: now_str(),
        });
        msgs.push(CollabMessage {
            id: to_msg_id.clone(),
            room_id: room_id.to_string(),
            sender: to.clone(),
            sender_name: registry()
                .get_tool_config(&to)
                .and_then(|c| c.nickname.clone().filter(|n| !n.trim().is_empty()))
                .unwrap_or_else(|| to.clone()),
            avatar: registry().get_tool_config(&to).and_then(|c| c.avatar.clone()),
            content: message.to_string(),
            references: vec![],
            files: vec![],
            dispatch: Some(CollabDispatch {
                tool_id: to.clone(),
                session_id: String::new(),
                model: None,
                duration_ms: None,
                usage: None,
            }),
            reply_to: Some(from_msg_id.clone()),
            status: Some("running".to_string()),
            created_at: now_str(),
        });
    }
    store.turns.push(CollabTurn {
        id: new_id(),
        room_id: room_id.to_string(),
        sender: from.to_string(),
        state: "collecting".to_string(),
        message_ids: vec![to_msg_id.clone()],
        trigger_message_id: to_msg_id.clone(),
        latest_message_id: to_msg_id.clone(),
        dispatch_after: now_rfc3339(),
        owner_tool_id: Some(to.clone()),
        project_path: load_room_project_path(room_id),
        model_id: None,
        provider_id: None,
        options: CollabDispatchOptions::default(),
        dispatch_message_id: Some(to_msg_id.clone()),
        causal_root_id: request_id.clone(),
        causal_depth: 1,
        woken_tools: vec![to.clone()],
        created_at: now_rfc3339(),
        updated_at: now_rfc3339(),
    });
    register_delegation(&request_id, &to_msg_id);
    let _ = save_store(&store);
    drop(_lock);
    ensure_turn_scheduler(app.clone());
    request_id
}

/// D 方案：agent 发一条普通消息（如状态播报），不触发派发
pub fn collab_post_message(
    app: tauri::AppHandle,
    room_id: &str,
    from: &str,
    message: &str,
    parent: Option<&str>,
) {
    let _lock = STORE_LOCK.get_or_init(|| Mutex::new(())).lock();
    let mut store = load_store();
    if let Some(msgs) = store.messages.get_mut(room_id) {
        msgs.push(CollabMessage {
            id: new_id(),
            room_id: room_id.to_string(),
            sender: from.to_string(),
            sender_name: registry()
                .get_tool_config(from)
                .and_then(|c| c.nickname.clone().filter(|n| !n.trim().is_empty()))
                .unwrap_or_else(|| from.to_string()),
            avatar: registry().get_tool_config(from).and_then(|c| c.avatar.clone()),
            content: message.to_string(),
            references: vec![],
            files: vec![],
            dispatch: None,
            reply_to: parent.map(|p| p.to_string()),
            status: None,
            created_at: now_str(),
        });
    }
    let _ = save_store(&store);
    if let Some(msgs) = store.messages.get(room_id) {
        if let Some(m) = msgs.last() {
            let _ = app.emit(
                "collab:msg-updated",
                CollabMsgUpdatedPayload {
                    room_id: room_id.to_string(),
                    message: m.clone(),
                },
            );
        }
    }
}

/// E 方案：任务流操作（agent 通过 HTTP 总线调用）。op: create/claim/complete/handoff
pub fn collab_task_op(app: tauri::AppHandle, room_id: &str, body: &Value) -> Value {
    let op = body.get("op").and_then(|v| v.as_str()).unwrap_or("");
    let _lock = STORE_LOCK.get_or_init(|| Mutex::new(())).lock();
    let mut store = load_store();
    let res = match op {
        "create" => {
            let id = new_id();
            let title = body.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let desc = body
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let created_by = body
                .get("created_by")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let parent = body.get("parent").and_then(|v| v.as_str()).map(|s| s.to_string());
            store.tasks.push(CollabTask {
                id: id.clone(),
                room_id: room_id.to_string(),
                title,
                description: desc,
                status: "open".to_string(),
                assignee: None,
                created_by,
                parent_task: parent,
                created_at: now_str(),
                updated_at: now_str(),
            });
            json!({ "ok": true, "id": id })
        }
        "claim" => {
            let id = body.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
            let by = body.get("by").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if let Some(t) = store.tasks.iter_mut().find(|t| t.id == id && t.room_id == room_id) {
                t.assignee = Some(by);
                t.status = "claimed".to_string();
                t.updated_at = now_str();
                json!({ "ok": true, "task": t })
            } else {
                json!({ "ok": false, "error": "not found" })
            }
        }
        "complete" => {
            let id = body.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
            if let Some(t) = store.tasks.iter_mut().find(|t| t.id == id && t.room_id == room_id) {
                t.status = "done".to_string();
                t.updated_at = now_str();
                json!({ "ok": true, "task": t })
            } else {
                json!({ "ok": false, "error": "not found" })
            }
        }
        "handoff" => {
            let id = body.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
            let to = body.get("to").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let target = resolve_mention(&to).unwrap_or(to);
            if let Some(t) = store.tasks.iter_mut().find(|t| t.id == id && t.room_id == room_id) {
                t.assignee = Some(target);
                t.status = "claimed".to_string();
                t.updated_at = now_str();
                json!({ "ok": true, "task": t })
            } else {
                json!({ "ok": false, "error": "not found" })
            }
        }
        _ => json!({ "ok": false, "error": format!("unknown op: {}", op) }),
    };
    let _ = save_store(&store);
    let _ = app.emit("collab:tasks-updated", json!({ "room_id": room_id }));
    res
}

/// E 方案：列出房间的任务（前端任务面板用）
#[tauri::command]
pub fn collab_list_tasks(room_id: String) -> Vec<CollabTask> {
    let _lock = STORE_LOCK.get_or_init(|| Mutex::new(())).lock();
    let store = load_store();
    store
        .tasks
        .iter()
        .filter(|t| t.room_id == room_id)
        .cloned()
        .collect()
}

/// E 方案：任务操作（前端按钮调用）：op: create/claim/complete/handoff
#[tauri::command]
pub fn collab_task_action(room_id: String, body: Value, app: tauri::AppHandle) -> Value {
    collab_task_op(app, &room_id, &body)
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
    eprintln!("[collab] ═══ 派发开始 ═══ tool={}, room={}, placeholder={}, prompt_len={}, model={:?}, provider={:?}",
        tool_id, room_id, placeholder_id, prompt.len(), model_id, provider_id);
    let tool_config = match registry().get_tool_config(&tool_id) {
        Some(c) => c.clone(),
        None => {
            eprintln!("[collab] ✗ 未知工具: {}", tool_id);
            finalize_message(app, &room_id, &placeholder_id, "error", "⚠ 未知工具".to_string(), None, None, None, None);
            return;
        }
    };

    // 会话绑定：已有 id 则续聊（不再开新会话）
    let session_key = format!("{}::{}", room_id, tool_id);
    // 单次 load_store 同时取出会话 id 与上下文快照，避免重复全量解析 collab.json
    let store = load_store();
    let existing_sid = store.tool_sessions.get(&session_key).cloned();
    let has_session = existing_sid.is_some();
    let snapshot = if !has_session {
        store.context_snapshots.get(&session_key).cloned()
    } else {
        None
    };
    eprintln!("[collab] 会话状态: has_session={}, existing_sid={:?}", has_session, existing_sid);

    // ─── 上下文快照注入：新会话首次派发时，将压缩摘要注入提示词 ───
    let prompt = if let Some(s) = snapshot {
        format!(
            "--- 上下文快照（来自上一会话的压缩摘要）---\n\n{}\n\n--- 以上为上下文快照，请基于此继续工作 ---\n\n--- 当前任务 ---\n\n{}",
            s.summary, prompt
        )
    } else {
        prompt
    };

    // ─── 协同协议注入：告诉 agent 本房间可用工具与 @委派 规则（实现 agent 主动互聊，对齐 open-tag system-prompt 注入） ───
    // 仅在「新会话首次派发」注入：协议在首轮已进入上下文；续聊（has_session）时重复注入只会污染对话、浪费 token。
    let prompt = if has_session {
        prompt
    } else {
        format!("{}\n\n{}", build_collab_protocol_prompt(&room_id), prompt)
    };

    let tmp_dir = get_base_dir().join("collab_tmp");
    let _ = fs::create_dir_all(&tmp_dir);
    let prompt_path = tmp_dir.join(format!(
        "{}_{}.txt",
        room_id.replace(|c: char| !c.is_alphanumeric(), ""),
        new_id()
    ));
    if let Err(e) = fs::write(&prompt_path, &prompt) {
        finalize_message(app, &room_id, &placeholder_id, "error", format!("⚠ 写入提示词失败: {}", e), None, None, None, None);
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
        app,
    ).await;

    // 注册当前 msg_id 到全局表（供常驻代理 emit 事件和缓存文本时查询）
    crate::proxy::server::set_collab_msg_id(&room_id, &tool_id, placeholder_id.clone());

    // 代理未启动（port==0）时明确报错并中止：否则 opencode 会经 env(baseUrl→OPENAI_BASE_URL)
    // 直连真实上游 https 端点，报出误导性的 "unknown certificate verification error"。
    if _port == 0 {
        eprintln!("[collab] ✗ 代理未启动（port==0），中止派发，避免绕过代理直连上游");
        finalize_message(
            app, &room_id, &placeholder_id, "error",
            "⚠ 代理未启动，无法派发。请检查「设置」中该供应商的密钥与协议 URL 是否已正确配置（OpenCode 需 openai 协议出站）。".to_string(),
            None, None, None, None,
        );
        return;
    }

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
            eprintln!("[collab] ✗ 未配置派发命令: tool={}, has_session={}", tool_config.display_name, has_session);
            let _ = fs::remove_file(&prompt_path);
            crate::proxy::server::clear_collab_msg_id(&room_id, &tool_id);
            finalize_message(app, &room_id, &placeholder_id, "error", format!("⚠ 工具 {} 未配置派发命令", tool_config.display_name), None, None, None, None);
            return;
        }
    };
    eprintln!("[collab] 模板选择: has_session={}, tmpl_len={}", has_session, tmpl.len());

    let prompt_str = prompt_path.to_string_lossy().replace('\\', "/");
    let quoted = format!("\"{}\"", prompt_str);
    let prompt_mode = tool_config.prompt_mode.clone().unwrap_or_default();
    let use_stdin = prompt_mode == "stdin";

    // argv 直启：把模板拆成参数数组，{prompt}/{prompt_file}/{session_id} 作为独立参数，
    // 不再整体经 shell 解释，规避命令注入（参考 skills 仓库：never spawn through a shell）
    // 必须在 tmpl 被 move 进 cmd 之前完成分词。
    // 模型名：伪装优先，否则取所选模型
    let model_for_placeholders = options.masquerade_model.clone()
        .filter(|c| !c.is_empty())
        .or_else(|| model_id.clone())
        .unwrap_or_default();

    let argv: Vec<String> = {
        let mut out = Vec::new();
        for tok in tokenize_template(&tmpl) {
            match tok.as_str() {
                "{prompt}" => {
                    if !use_stdin {
                        out.push(prompt.clone());
                    }
                }
                "{prompt_file}" => {
                    out.push(prompt_str.clone());
                }
                "{session_id}" => {
                    out.push(existing_sid.clone().unwrap_or_default());
                }
                "{model}" => {
                    out.push(model_for_placeholders.clone());
                }
                other => out.push(other.to_string()),
            }
        }
        out
    };

    // 抄 open-tag spawn：占位符解析为空（如未选模型 / 无 session）时，移除其前的 flag
    //（-m/--model/-s 等），避免残留 `--model ` 把后续 token 误当成模型名（opencode/cursor
    // 此类 node 系 CLI 会因此无法正常启动）。
    let argv: Vec<String> = {
        let mut f: Vec<String> = Vec::with_capacity(argv.len());
        for tok in argv.iter() {
            if tok.is_empty() {
                if let Some(prev) = f.last() {
                    if prev.starts_with('-') {
                        f.pop();
                        continue;
                    }
                }
                continue;
            }
            f.push(tok.clone());
        }
        f
    };

    // shell 命令串（仅用于 Windows .cmd 垫片回退与日志；提示词经 cmd 转义）
    let mut cmd = tmpl;
    cmd = cmd.replace("{session_id}", &escape_cmd_arg(existing_sid.as_deref().unwrap_or("")));
    cmd = cmd.replace("{model}", &escape_cmd_arg(&model_for_placeholders));
    if cmd.contains("{prompt_file}") {
        cmd = cmd.replace("{prompt_file}", &quoted);
    }
    if cmd.contains("{prompt}") {
        cmd = cmd.replace("{prompt}", &escape_cmd_arg(&prompt));
    }
    // ─── 环境变量注入：从 config_file.write 中的 env.* 键注入（与正常启动一致） ───
    let mut envs = if let Some(ref p) = provider {
        super::launch::build_env_vars(&tool_config, &p.api_key, &base_url, &model_for_placeholders)
    } else {
        HashMap::new()
    };
    eprintln!("[collab] 命令构建: use_stdin={}, prompt_mode={:?}, env_vars={}", use_stdin, prompt_mode, envs.len());
    eprintln!("[collab] 环境变量: {:?}", envs.keys().collect::<Vec<_>>());
    // 兜底：若工具无 config_file 定义，仍按协议注入基础 env vars
    if envs.is_empty() {
        match tool_config.native_protocol().as_str() {
            "anthropic" => {
                envs.insert("ANTHROPIC_BASE_URL".to_string(), base_url.clone());
                envs.insert("ANTHROPIC_API_KEY".to_string(), api_key.clone());
                // 与 build_env_vars 保持一致：伪装优先，否则所选取模型，避免两条路径注入的 model 名不同
                if !model_for_placeholders.is_empty() { envs.insert("ANTHROPIC_MODEL".to_string(), model_for_placeholders.clone()); }
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

    eprintln!("[collab] ▶ 派发 {} → cmd: {}", tool_config.display_name, cmd);
    eprintln!("[collab] ▶ argv: {:?}", argv);
    eprintln!("[collab] ▶ 工作目录: {}", project_path);
    // 优先直接 spawn 工具二进制（argv 数组，不经 shell，规避注入）。
    // 仅当 Windows 上工具是 .cmd/.bat 垫片（CreateProcess 无法直接执行）时才退回 cmd /c。
    let program_raw = argv.first().cloned().unwrap_or_default();
    let program = resolve_program(&program_raw);
    let needs_shell = is_windows_shell_shim(&program);
    eprintln!("[collab] ▶ 解析程序: raw={:?} → resolved={:?} shim={}", program_raw, program, needs_shell);
    // 用解析后的完整路径替换 argv[0]，后续统一以 argv 数组派发（对齐 open-tag / cross-spawn：
    // 直接 spawn 二进制，仅 .cmd/.bat 垫片退回 cmd /d /s /c）。
    let mut argv_resolved = argv.clone();
    if !argv_resolved.is_empty() {
        argv_resolved[0] = program.clone();
    }
    let mut command = if needs_shell {
        // Windows .cmd/.bat 垫片无法被 CreateProcess 直接执行，必须退回 cmd。
        // 对齐 cross-spawn：构造 `cmd /d /s /c "<windows_quote 逐元素转义并 join 的 argv>"`。
        //   /s 是关键——它修正默认 /c 的“删除首尾引号”规则，使含空格的程序路径/参数被正确识别；
        //   /d 禁止自动执行 AutoRun 注册表项。各 argv 元素用 windows_quote 加引号（含空格/引号时），
        //   整体再用一对引号括住作为 /c 的单个参数。raw_arg 原样透传，避免 Rust 重新加引号破坏 /s 解析。
        // 这是 cross-spawn 在 Windows 上经实战验证的成熟写法（opencode/gemini 等 .cmd 垫片均走此路径），
        // 取代此前易踩坑的 `cmd /c` 字符串拼接与 @ 前缀 trick。
        #[cfg(windows)] {
            let full = argv_resolved.iter().map(|a| windows_quote(a)).collect::<Vec<_>>().join(" ");
            let quoted = format!("\"{}\"", full);
            let mut c = Command::new("cmd");
            c.creation_flags(0x08000000); // CREATE_NO_WINDOW：禁止弹出命令提示符黑框
            c.raw_arg("/d").raw_arg("/s").raw_arg("/c").raw_arg(quoted.as_str());
            c
        }
        #[cfg(not(windows))] {
            // 非 Windows 的 shell 脚本（.sh 等）回退：sh -c "<join 后的字符串>"
            let full = argv_resolved.join(" ");
            let mut c = Command::new("sh");
            c.arg("-c").arg(full);
            c
        }
    } else {
        // 直接 spawn 工具二进制（argv 数组，不经 shell，规避注入）——对齐 cross-spawn 对可执行文件的路径
        let mut c = Command::new(&program);
        c.args(&argv_resolved[1..]);
        #[cfg(windows)]
        c.creation_flags(0x08000000); // CREATE_NO_WINDOW：禁止弹出命令提示符黑框
        c
    };
    command
        .current_dir(&project_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // 抄 open-tag spawn：剥离从父进程继承的 NODE_OPTIONS。若其中含代理 require hook，
    // 会导致 opencode/cursor 等 node 系 CLI 拒绝启动（open-tag 在子进程 env 中主动 strip）。
    // 我们从不依赖 NODE_OPTIONS，移除只消除该隐患。
    command.env_remove("NODE_OPTIONS");
    // ─── 协同总线 env：让被派发的 agent 能主动同步委派其它工具（C 方案，对齐 open-tag 的 agent CLI） ───
    // agent 在推理中可运行 curl 调 $COLLAB_BUS_URL/collab/agent/send 并阻塞拿到对方回复。
    command.env("COLLAB_BUS_URL", format!("http://127.0.0.1:{}", _port));
    command.env("COLLAB_BUS_TOKEN", room_bus_token(&room_id));
    command.env("COLLAB_ROOM_ID", &room_id);
    command.env("COLLAB_TOOL_ID", &tool_id);
    // 提示词传入方式：stdin 模式把临时文件作为子进程 stdin；否则用 null（避免工具检测到管道 stdin 后阻塞读取）
    let mut child_stdin_opt: Option<ChildStdin> = None;
    if use_stdin {
        match fs::File::open(&prompt_path) {
            Ok(f) => { command.stdin(Stdio::from(f)); }
            Err(e) => {
                let _ = fs::remove_file(&prompt_path);
                finalize_message(app, &room_id, &placeholder_id, "error", format!("⚠ 打开提示词失败: {}", e), None, None, None, None);
                return;
            }
        }
    } else {
        command.stdin(Stdio::null());
    }
    for (k, v) in &envs {
        command.env(k, v);
    }

    let mut child = match command.spawn() {
        Ok(c) => {
            eprintln!("[collab] ✓ 子进程已启动: pid={}", c.id());
            c
        }
        Err(e) => {
            eprintln!("[collab] ✗ 启动失败: {}", e);
            let _ = fs::remove_file(&prompt_path);
            crate::proxy::server::clear_collab_msg_id(&room_id, &tool_id);
            finalize_message(app, &room_id, &placeholder_id, "error", format!("⚠ 启动工具失败: {}", e), None, None, None, None);
            return;
        }
    };

    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => {
            let _ = fs::remove_file(&prompt_path);
            crate::proxy::server::clear_collab_msg_id(&room_id, &tool_id);
            finalize_message(app, &room_id, &placeholder_id, "error", "⚠ 无法读取工具输出".to_string(), None, None, None, None);
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
        let mut g = map.lock();
        g.insert(placeholder_id.clone(), DispatchCtrl {
            child: child_arc.clone(),
            cancel: cancel.clone(),
        });
    }

    // stderr 后台读取线程：收集内容到 stderr_buf，供无输出时诊断；同时检测交互式 prompt
    // 使用 timeout 读取，防止子进程挂起导致线程泄漏
    let stderr_cancel = cancel.clone();
    let stderr_buf = Arc::new(Mutex::new(String::new()));
    let stderr_buf_clone = stderr_buf.clone();
    let stderr_room = room_id.clone();
    let stderr_msg = placeholder_id.clone();
    let stderr_app = app.clone();
    let stderr_handle = if let Some(stderr) = stderr {
        Some(std::thread::spawn(move || {
            use std::io::Read;
            let mut reader = stderr;
            let mut buf = [0u8; 1024];
            loop {
                if stderr_cancel.load(Ordering::SeqCst) { break; }
                // 使用非阻塞读取 + sleep 轮询，使 cancel 标志能及时生效
                match reader.read(&mut buf) {
                    Ok(0) => break, // EOF
                    Ok(n) => {
                        let text = String::from_utf8_lossy(&buf[..n]);
                        for line in text.lines() {
                            let line = line.trim_end();
                            if !line.is_empty() {
                                eprintln!("[collab] stderr: {}", line);
                                if let Some((question, options)) = detect_prompt(line) {
                                    emit_prompt(&stderr_app, &stderr_room, &stderr_msg, &question, options);
                                }
                                let mut g = stderr_buf_clone.lock();
                                if g.len() < 8192 {
                                    g.push_str(line);
                                    g.push('\n');
                                }
                            }
                        }
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        // 无数据可读，短暂休眠后重试
                        std::thread::sleep(std::time::Duration::from_millis(50));
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
    let stdout_cancel = cancel.clone();
    let stdout_reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut reader = stdout;
        let mut buf = [0u8; 1024];
        loop {
            if stdout_cancel.load(Ordering::SeqCst) { let _ = tx.send(None); break; }
            match reader.read(&mut buf) {
                Ok(0) => { let _ = tx.send(None); break; }
                Ok(n) => {
                    let text = String::from_utf8_lossy(&buf[..n]);
                    for line in text.lines() {
                        let _ = tx.send(Some(line.to_string()));
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(_) => { let _ = tx.send(None); break; }
            }
        }
    });

    // 注册 PromptCtrl（若有 stdin 句柄）
    let prompt_stdin = Arc::new(Mutex::new(child_stdin_opt));
    let prompt_pending: Arc<Mutex<Option<(String, Instant)>>> = Arc::new(Mutex::new(None));
    if !use_stdin {
        let map = PROMPT_STATE.get_or_init(|| Mutex::new(HashMap::new()));
        let mut g = map.lock();
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
    let mut captured_usage: Option<TokenUsage> = None;
    let dispatch_start = std::time::Instant::now();
    let mut err_msg: Option<String> = None;
    let mut raw_lines: Vec<String> = Vec::new(); // 用于 JSON 解析失败时回退显示原始输出
    let adapter = runner_adapter(&runner);
    let is_json_runner = adapter.is_json();
    eprintln!("[collab] ▶ 读取循环开始: runner={}, is_json={}, timeout={}s", runner, is_json_runner, DISPATCH_TIMEOUT_SECS);

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
        if let Some(ref pend) = *prompt_pending.lock() {
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
                // 检测工具错误事件（如 opencode 的 {"type":"error",...}）：直接上报，避免被误判为"无输出"
                if let Ok(v) = serde_json::from_str::<Value>(&l) {
                    if v.get("type").and_then(|t| t.as_str()) == Some("error") {
                        let msg = v.get("error")
                            .and_then(|e| e.get("data").and_then(|d| d.get("message")).or_else(|| e.get("message")))
                            .and_then(|m| m.as_str())
                            .unwrap_or("未知错误")
                            .to_string();
                        err_msg = Some(format!("⚠ 工具返回错误: {}", msg));
                        break;
                    }
                }
                if is_json_runner {
                    // 保留原始行用于回退
                    if raw_lines.len() < 50 {
                        raw_lines.push(l.clone());
                    }
                    // 检测 prompt（JSON 解析失败时也检查）
                    if let Some((question, options)) = detect_prompt(&l) {
                        emit_prompt(app, &room_id, &placeholder_id, &question, options);
                    }
                    if let Some((ev, sid)) = adapter.parse(&l) {
                        if let Some(s) = sid { captured_sid = Some(s); }
                        match ev {
                            StreamEvent::Delta(d) => {
                                eprintln!("[collab] ▸ Delta: len={}", d.len());
                                accumulated.push_str(&d);
                                let _ = app.emit(
                                    "collab:delta",
                                    CollabDeltaPayload { room_id: room_id.clone(), msg_id: placeholder_id.clone(), delta: d },
                                );
                            }
                            StreamEvent::Activity(a) => {
                                eprintln!("[collab] ▸ Activity: {}", a);
                                let _ = app.emit(
                                    "collab:activity",
                                    CollabActivityPayload { room_id: room_id.clone(), msg_id: placeholder_id.clone(), activity: a },
                                );
                            }
                            StreamEvent::Result(t, u) => {
                                eprintln!("[collab] ▸ Result: len={}, non_empty={}", t.len(), !t.is_empty());
                                // 仅当收尾文本非空才覆盖（避免空 result 抹掉已流式内容）
                                if !t.is_empty() { accumulated = t; }
                                // 捕获 token 消耗（claude 的 result 事件提供；其他工具为 None）
                                if let Some(us) = u { captured_usage = Some(us); }
                            }
                            StreamEvent::Ignore => {}
                        }
                    } else {
                        // JSON 解析失败，作为原始文本兜底
                        eprintln!("[collab] ⚠ JSON 解析失败 (runner={}): {}", runner, l.chars().take(200).collect::<String>());
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
    eprintln!("[collab] 派发循环结束: accumulated_len={}, captured_sid={:?}, err={:?}, raw_lines={}",
        accumulated.len(), captured_sid, err_msg.is_some(), raw_lines.len());
    // 代理为房间级常驻，不在每条消息结束时停止
    if let Some(map) = DISPATCH_STATE.get() {
        map.lock().remove(&placeholder_id);
    }
    // 清理 PromptCtrl
    if let Some(map) = PROMPT_STATE.get() {
        map.lock().remove(&placeholder_id);
    }

    // 获取 stderr 内容
    let stderr_content = stderr_buf.lock().clone();
    if !stderr_content.is_empty() {
        eprintln!("[collab] stderr 总量: {} 字节", stderr_content.len());
    }

    let had_error = err_msg.is_some();
    let mut compact_result: Option<String> = None;

    match err_msg {
        Some(e) => {
            eprintln!("[collab] ═══ 派发结束(错误) ═══ placeholder={}, err={}", placeholder_id, e);
            finalize_message(app, &room_id, &placeholder_id, "error", e, None, None, None, None);
        }
        None => {
            let mut content = accumulated.trim().to_string();
            eprintln!("[collab] 派发完成: content_len={}, is_json={}, raw_lines={}, stderr_len={}",
                content.len(), is_json_runner, raw_lines.len(), stderr_content.len());
            // ── 代理回退：stdout 无内容时，使用代理缓存的响应文本 ──
            if content.is_empty() || content == "(无输出)" {
                if let Some(proxy_text) = crate::proxy::server::take_proxy_text(&placeholder_id) {
                    eprintln!("[collab] ✓ 代理回退: 使用代理响应文本 (len={})", proxy_text.len());
                    if !proxy_text.is_empty() {
                        content = proxy_text;
                    }
                }
            }
            // JSON runner 但无解析结果 → 回退显示原始 stdout + stderr
            if content.is_empty() && is_json_runner && !raw_lines.is_empty() {
                eprintln!("[collab] ⚠ JSON runner 无解析结果，回退原始输出 ({} 行)", raw_lines.len());
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
            eprintln!("[collab] ═══ 派发结束(成功) ═══ placeholder={}, content_len={}, sid={:?}, session_update={}",
                placeholder_id, content.len(), captured_sid, session_update.is_some());
            compact_result = Some(content.clone());
            finalize_message(
                app,
                &room_id,
                &placeholder_id,
                "done",
                content.clone(),
                captured_sid.clone(),
                Some(dispatch_start.elapsed().as_millis() as u64),
                captured_usage.clone(),
                session_update,
            );
            // B 层：工具回复后解析 @ 提及，触发因果唤醒（多 agent 协同）
            {
                let s = load_store();
                if let Some(turn) = s.turns.iter().find(|t| t.dispatch_message_id.as_deref() == Some(&placeholder_id)) {
                    let root = turn.causal_root_id.clone();
                    let depth = turn.causal_depth;
                    drop(s);
                    wake_tools_from_reply(&room_id, &tool_id, &content, &root, depth, &placeholder_id);
                }
            }
        }
    }

    // ─── 压缩回调：通知 collab_compact_session 命令 ───
    if let Some(map) = COMPACT_CALLBACKS.get() {
        if let Some(sender) = map.lock().remove(&placeholder_id) {
            let _ = sender.send(compact_result.unwrap_or_default());
        }
    }
    // ─── 快照清理：新会话成功创建后清除已注入的快照（避免重复注入） ───
    if !had_error && captured_sid.is_some() {
        let mut store = load_store();
        if store.context_snapshots.remove(&session_key).is_some() {
            let _ = save_store(&store);
        }
    }
    // 清理全局 msg_id 注册
    crate::proxy::server::clear_collab_msg_id(&room_id, &tool_id);
    // 清理代理文本缓存（防止内存泄漏：工具有 stdout 输出时代理文本不会被消费）
    let _ = crate::proxy::server::take_proxy_text(&placeholder_id);
}

/// 用户取消正在进行的派发
#[tauri::command]
pub fn collab_cancel_dispatch(msg_id: String) -> Result<(), String> {
    eprintln!("[collab] 用户取消派发: msg_id={}", msg_id);
    let ctrl = if let Some(map) = DISPATCH_STATE.get() {
        map.lock().get(&msg_id).cloned()
    } else {
        None
    };
    if let Some(ctrl) = ctrl {
        ctrl.cancel.store(true, Ordering::SeqCst);
        kill_tree(&ctrl.child);
        eprintln!("[collab] ✓ 已发送取消信号并杀进程树");
    } else {
        eprintln!("[collab] ⚠ 未找到派发状态: {}", msg_id);
    }
    Ok(())
}

/// 压缩上下文：向当前工具会话发送摘要请求，获取总结后保存为快照并重置会话
#[tauri::command]
pub async fn collab_compact_session(
    app: tauri::AppHandle,
    room_id: String,
    tool_id: String,
    project_path: String,
    model_id: Option<String>,
    provider_id: Option<String>,
    options: Option<CollabDispatchOptions>,
) -> Result<Option<ContextSnapshot>, String> {
    let session_key = format!("{}::{}", room_id, tool_id);

    // 1. 检查是否有活跃会话
    let store = load_store();
    let existing_sid = store.tool_sessions.get(&session_key)
        .cloned()
        .ok_or_else(|| "没有活跃会话可压缩".to_string())?;
    let message_count = store.messages.get(&room_id)
        .map(|msgs| msgs.len())
        .unwrap_or(0);

    // 2. 防止并发派发
    let dispatch_key = session_key.clone();
    let active = ACTIVE_DISPATCHES.get_or_init(|| tokio::sync::Mutex::new(HashSet::new()));
    // 单锁内完成“检查+插入”，消除 TOCTOU 竞态
    let mut active_guard = active.lock().await;
    if active_guard.contains(&dispatch_key) {
        return Err("该工具正在处理上一条消息，请等待完成".to_string());
    }
    active_guard.insert(dispatch_key.clone());
    // 立即释放锁，避免跨 await 持有非 Send 的 MutexGuard（Tauri 命令要求 Future: Send）
    drop(active_guard);

    // 3. 获取工具配置
    let tool_config = match registry().get_tool_config(&tool_id) {
        Some(c) => c.clone(),
        None => {
            active.lock().await.remove(&dispatch_key);
            return Err(format!("未知工具：{}", tool_id));
        }
    };

    // 4. 构建压缩提示词
    let compact_prompt = r"请总结当前会话的全部关键信息，生成一份上下文快照。这份快照将用于在新会话中无缝继续当前工作。

请务必包含以下内容（用 Markdown 格式）：

## 已完成的工作
- 已修改/创建的文件清单
- 已实现的功能和修复的问题

## 当前任务状态
- 正在进行的任务及其进度
- 遇到的阻碍和解决方案

## 关键决策
- 重要的技术选择和架构决定
- 项目约定和编码规范

## 待办事项
- 尚未完成的工作
- 下一步计划和优先级

## 重要上下文
- 任何对新会话继续工作有必要的信息（环境配置、特殊参数等）

请直接输出总结内容，不要询问确认。";

    // 5. 创建占位消息（可见的“压缩中”消息）
    let placeholder_id = new_id();
    let placeholder = CollabMessage {
        id: placeholder_id.clone(),
        room_id: room_id.clone(),
            sender: tool_id.clone(),
            sender_name: tool_config
                .nickname
                .clone()
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| tool_config.display_name.clone()),
            avatar: tool_config.avatar.clone(),
            content: String::new(),
            references: vec![],
            files: vec![],
            dispatch: Some(CollabDispatch {
                tool_id: tool_id.clone(),
                session_id: existing_sid.clone(),
                model: model_id.clone(),
                duration_ms: None,
                usage: None,
            }),
        reply_to: None,
        status: Some("running".to_string()),
        created_at: now_str(),
    };

    // 6. 保存占位消息（读-改-写临界区）
    {
        let _store_lock = STORE_LOCK.get_or_init(|| Mutex::new(())).lock();
        let mut store = load_store();
        if let Some(msgs) = store.messages.get_mut(&room_id) {
            msgs.push(placeholder.clone());
        }
        if let Some(room) = store.rooms.iter_mut().find(|r| r.id == room_id) {
            room.updated_at = now_str();
        }
        save_store(&store)?;
    }

    // 7. 推送占位消息到前端（实时显示“压缩中”状态）
    let _ = app.emit("collab:compact-started", &placeholder);

    // 8. 注册压缩回调
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    {
        let map = COMPACT_CALLBACKS.get_or_init(|| Mutex::new(HashMap::new()));
        map.lock().insert(placeholder_id.clone(), tx);
    }

    // 9. 后台派发压缩请求
    let app_clone = app.clone();
    let rt_room = room_id.clone();
    let rt_tool = tool_id.clone();
    let rt_project = project_path.clone();
    let rt_options = options.clone().unwrap_or_default();
    let rt_model = model_id.clone();
    let rt_provider = provider_id.clone();
    let rt_placeholder_id = placeholder_id.clone();

    tauri::async_runtime::spawn(async move {
        dispatch_to_tool(
            &app_clone,
            rt_room,
            rt_tool,
            rt_project,
            compact_prompt.to_string(),
            rt_placeholder_id,
            rt_model,
            rt_provider,
            rt_options,
        ).await;
    });

    // 10. 等待压缩完成
    let summary_result = rx.await;

    // 无论成功与否，清理活动派发标记
    if let Some(active) = ACTIVE_DISPATCHES.get() {
        active.lock().await.remove(&dispatch_key);
    }

    let summary = match summary_result {
        Ok(s) => s,
        Err(_) => return Err("压缩过程异常中断".to_string()),
    };

    // 11. 检查压缩结果
    if summary.is_empty() || summary.starts_with("⚠") {
        return Ok(None);
    }

    // 12. 保存快照 + 重置会话
    let snapshot = ContextSnapshot {
        id: new_id(),
        room_id: room_id.clone(),
        tool_id: tool_id.clone(),
        summary: summary.clone(),
        old_session_id: existing_sid,
        message_count,
        created_at: now_str(),
    };

    let mut store = load_store();
    store.context_snapshots.insert(session_key, snapshot.clone());
    store.tool_sessions.remove(&format!("{}::{}", room_id, tool_id));
    save_store(&store)?;

    // 13. 停止常驻代理（下次派发时重建）
    stop_room_proxy(&room_id, &tool_id);

    // 14. 推送压缩完成事件
    let _ = app.emit("collab:compacted", serde_json::json!({
        "room_id": room_id,
        "tool_id": tool_id,
        "snapshot": &snapshot,
    }));

    eprintln!("[collab] 上下文压缩完成: {} 条消息 → 快照 {} 字符",
        message_count, summary.len());

    Ok(Some(snapshot))
}

/// 查询某房间+工具的上下文快照是否存在
#[tauri::command]
pub fn collab_get_snapshot(room_id: String, tool_id: String) -> Option<ContextSnapshot> {
    load_store().context_snapshots.get(&format!("{}::{}", room_id, tool_id)).cloned()
}

/// 重置某工具在某会话中的续聊上下文（删除绑定的 session id）
#[tauri::command]
pub fn collab_reset_session(room_id: String, tool_id: String) -> Result<(), String> {
    eprintln!("[collab] 重置会话: room={}, tool={}", room_id, tool_id);
    let _store_lock = STORE_LOCK.get_or_init(|| Mutex::new(())).lock();
    let mut store = load_store();
    store.tool_sessions.remove(&format!("{}::{}", room_id, tool_id));
    // 同时清除上下文快照
    store.context_snapshots.remove(&format!("{}::{}", room_id, tool_id));
    save_store(&store)?;
    // 同时停止并清理常驻代理，下次派发时重建
    stop_room_proxy(&room_id, &tool_id);
    eprintln!("[collab] ✓ 会话已重置: room={}, tool={}", room_id, tool_id);
    Ok(())
}

/// 查询派发轮次（Turn 轮次合并状态 + 因果唤醒链），前端用于展示协同脉络
#[tauri::command]
pub fn collab_get_turns(room_id: Option<String>) -> Vec<CollabTurn> {
    let store = load_store();
    match room_id {
        Some(r) => store.turns.into_iter().filter(|t| t.room_id == r).collect(),
        None => store.turns,
    }
}

/// 查询所有工具的在线状态（agent 心跳）
#[tauri::command]
pub fn collab_get_agents() -> Vec<CollabAgentStatus> {
    load_store().agents
}

/// 用户响应工具的交互式询问
#[tauri::command]
pub fn collab_respond_prompt(msg_id: String, response: String) -> Result<(), String> {
    eprintln!("[collab] 用户响应 prompt: msg_id={}, response={}", msg_id, response);
    if !write_stdin_response(&msg_id, &response) {
        eprintln!("[collab] ⚠ 响应失败: msg_id={}", msg_id);
        return Err("无待响应的询问或工具不支持交互".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_cmd_arg_wraps_value_in_double_quotes() {
        // 任何值都被整体双引号包裹，作为 cmd /c 的单个参数字符串
        assert_eq!(escape_cmd_arg("abc"), "\"abc\"");
        assert_eq!(escape_cmd_arg("a & b | c"), "\"a & b | c\"");
    }

    #[test]
    fn escape_cmd_arg_neutralizes_injection() {
        // 恶意 session_id：整体被双引号包裹，内部引号被转义为 ""，无法提前闭合外层引号
        let evil = "x\" & calc & \"";
        let out = escape_cmd_arg(evil);
        assert!(out.starts_with('"'));
        assert!(out.ends_with('"'));
        // 作为 cmd /c 的单个参数字符串，& 被关在引号内，不会被执行
        assert!(out.contains("& calc &"));
        // 内部引号均为成对的 ""，不存在可提前闭合外层引号的裸引号
        let inner = &out[1..out.len() - 1];
        let mut chars = inner.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '"' {
                // 紧跟的必须是另一个 "（转义），否则就是裸引号（逃逸）
                assert_eq!(chars.peek(), Some(&'"'), "发现未转义的裸引号");
                chars.next();
            }
        }
    }

    #[test]
    fn escape_cmd_arg_escapes_embedded_quote() {
        // 内部双引号转义为 ""，整体被包裹
        assert_eq!(escape_cmd_arg("a%b\"c"), "\"a%b\"\"c\"");
    }

    #[test]
    fn cmp_updated_at_is_descending() {
        // cmp_updated_at 返回 b.cmp(a)（降序）：更新时间更晚的应排在前面
        assert_eq!(
            cmp_updated_at("2026-07-23T10:00:00", "2026-07-23T09:00:00"),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            cmp_updated_at("2026-07-23T09:00:00", "2026-07-23T10:00:00"),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            cmp_updated_at("2026-07-23T10:00:00", "2026-07-23T10:00:00"),
            std::cmp::Ordering::Equal
        );
    }

    #[test]
    fn cmp_updated_at_handles_rfc3339() {
        // RFC3339（含时区）也能正确比较：10:00Z 晚于 09:00+08:00(=01:00Z)，降序返回 Less
        assert_eq!(
            cmp_updated_at("2026-07-23T10:00:00Z", "2026-07-23T09:00:00+08:00"),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn now_str_format_is_sortable() {
        let s = now_str();
        assert_eq!(s.len(), 19);
        assert!(s.chars().all(|c| c.is_numeric() || c == '-' || c == ':' || c == 'T'));
    }

    #[test]
    fn tokenize_template_keeps_placeholders() {
        let tokens = tokenize_template("claude -p --resume {session_id}");
        assert!(tokens.contains(&"{session_id}".to_string()));
        assert!(tokens.contains(&"claude".to_string()));
    }

    #[test]
    fn now_str_uses_utc() {
        // 验证 now_str() 使用 UTC 时间（与 Local 相比，差异应在 ±1 小时内）
        let s = now_str();
        let parsed = chrono::NaiveDateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M:%S").unwrap();
        let utc_now = chrono::Utc::now().naive_utc();
        let diff = (utc_now - parsed).num_seconds().abs();
        assert!(diff < 3600, "now_str 应使用 UTC 时间，与本地时间偏差: {}s", diff);
    }
}

