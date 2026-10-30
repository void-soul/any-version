use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
#[cfg(windows)]
#[cfg(windows)]
use std::os::windows::process::CommandExt; // raw_arg：原样透传参数，阻止 Rust 对 cmd /c 命令串重新加引号
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tauri::Emitter;
use tokio::sync::Notify;
mod store;
use store::{cmp_updated_at, load_store, new_id, now_str, save_store, STORE_LOCK};

use super::launch::start_tool_proxy_with_collab;
use super::models::*;
use crate::commands::ai_registry::registry;
use crate::commands::config::get_data_dir;

// ─── 持久化 ───

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
        name: if name.trim().is_empty() {
            "未命名会话".to_string()
        } else {
            name.trim().to_string()
        },
        project_path,
        input_policy: default_input_policy(),
        allowed_sources: Vec::new(),
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
pub fn collab_list_rooms(
    offset: Option<usize>,
    limit: Option<usize>,
) -> Result<CollabRoomPage, String> {
    let mut rooms = load_store().rooms;
    // 按 updated_at 降序（使用兼容比较，避免格式不一致导致的排序错位）
    rooms.sort_by(|a, b| cmp_updated_at(&a.updated_at, &b.updated_at));
    let total = rooms.len();
    let limit = limit.unwrap_or(20).max(1);
    let offset = offset.unwrap_or(0);
    let end = (offset + limit).min(total);
    let page = if offset <= total {
        rooms[offset..end].to_vec()
    } else {
        vec![]
    };
    let has_more = end < total;
    Ok(CollabRoomPage {
        rooms: page,
        has_more,
        total,
    })
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
    let all = load_store()
        .messages
        .get(&room_id)
        .cloned()
        .unwrap_or_default();
    let total = all.len();
    let limit = limit.unwrap_or(50).max(1);
    // tail=true 时返回末尾一页（聊天视口初始加载最新消息）
    let mut offset = offset.unwrap_or(0);
    if tail.unwrap_or(false) {
        offset = total.saturating_sub(limit);
    }
    let end = (offset + limit).min(total);
    let messages = if offset <= total {
        all[offset..end].to_vec()
    } else {
        vec![]
    };
    // has_more 表示在已加载的最旧消息之前是否还有更早的消息
    let has_more = offset > 0;
    Ok(CollabMessagePage {
        messages,
        has_more,
        total,
    })
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
    let keys: Vec<String> = store
        .tool_sessions
        .keys()
        .filter(|k| k.starts_with(&format!("{}::", room_id)))
        .cloned()
        .collect();
    for k in keys {
        store.tool_sessions.remove(&k);
    }
    // 清理该房间相关的上下文快照
    let snap_keys: Vec<String> = store
        .context_snapshots
        .keys()
        .filter(|k| k.starts_with(&format!("{}::", room_id)))
        .cloned()
        .collect();
    for k in snap_keys {
        store.context_snapshots.remove(&k);
    }
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
    /// Codex web_search 开关：true → 写 `web_search = "live"`（真实实时检索）。
    #[serde(default)]
    pub web_search: bool,
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
    pub rectifier_media_heuristic: Option<bool>,
    #[serde(default)]
    pub rectifier_protocol_mismatch: Option<bool>,
    /// 工具调用方言优化，None = 沿用全局配置
    #[serde(default)]
    pub rectifier_toolcall_dialect: Option<bool>,
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
    let existing_sid = store
        .tool_sessions
        .get(&session_key)
        .cloned()
        .ok_or_else(|| "没有活跃会话可压缩".to_string())?;
    let message_count = store
        .messages
        .get(&room_id)
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
        )
        .await;
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
    store
        .context_snapshots
        .insert(session_key, snapshot.clone());
    store
        .tool_sessions
        .remove(&format!("{}::{}", room_id, tool_id));
    save_store(&store)?;

    // 13. 停止常驻代理（下次派发时重建）
    stop_room_proxy(&room_id, &tool_id);

    // 14. 推送压缩完成事件
    let _ = app.emit(
        "collab:compacted",
        serde_json::json!({
            "room_id": room_id,
            "tool_id": tool_id,
            "snapshot": &snapshot,
        }),
    );

    eprintln!(
        "[collab] 上下文压缩完成: {} 条消息 → 快照 {} 字符",
        message_count,
        summary.len()
    );

    Ok(Some(snapshot))
}

/// 查询某房间+工具的上下文快照是否存在
#[tauri::command]
pub fn collab_get_snapshot(room_id: String, tool_id: String) -> Option<ContextSnapshot> {
    load_store()
        .context_snapshots
        .get(&format!("{}::{}", room_id, tool_id))
        .cloned()
}

/// 重置某工具在某会话中的续聊上下文（删除绑定的 session id）
#[tauri::command]
pub fn collab_reset_session(room_id: String, tool_id: String) -> Result<(), String> {
    eprintln!("[collab] 重置会话: room={}, tool={}", room_id, tool_id);
    let _store_lock = STORE_LOCK.get_or_init(|| Mutex::new(())).lock();
    let mut store = load_store();
    store
        .tool_sessions
        .remove(&format!("{}::{}", room_id, tool_id));
    // 同时清除上下文快照
    store
        .context_snapshots
        .remove(&format!("{}::{}", room_id, tool_id));
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

/// 设置房间的输入域策略（sealed 模式下还要给出允许进入上下文的来源）。
///
/// 抄 open-tag `authorization.md`：多 agent 互喂时，一个 agent 的输出会被另一个
/// agent 当成事实读进去。sealed 把这条链路收口到「人类 + 白名单工具」。
#[tauri::command]
pub fn collab_set_input_policy(
    room_id: String,
    policy: String,
    allowed_sources: Option<Vec<String>>,
) -> Result<(), String> {
    if policy != "open" && policy != "sealed" {
        return Err(format!("未知输入域策略：{}（可选 open / sealed）", policy));
    }
    let _store_lock = STORE_LOCK.get_or_init(|| Mutex::new(())).lock();
    let mut store = load_store();
    let Some(room) = store.rooms.iter_mut().find(|r| r.id == room_id) else {
        return Err("会话不存在".to_string());
    };
    room.input_policy = policy;
    room.allowed_sources = allowed_sources.unwrap_or_default();
    room.updated_at = now_str();
    save_store(&store)
}

/// 查询房间的派发输出归档（取消 / 超时 / 出错前留存的那半截输出）。
#[tauri::command]
pub fn collab_list_archives(room_id: String) -> Vec<DispatchArchive> {
    load_store()
        .dispatch_archives
        .into_iter()
        .filter(|a| a.room_id == room_id)
        .collect()
}

/// 用户响应工具的交互式询问
#[tauri::command]
pub fn collab_respond_prompt(msg_id: String, response: String) -> Result<(), String> {
    eprintln!(
        "[collab] 用户响应 prompt: msg_id={}, response={}",
        msg_id, response
    );
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
    fn parse_cline_json_supports_partial_and_final_text() {
        let partial = parse_cline_json(
            r#"{"type":"say","say":"text","text":"hello ","partial":true,"session_id":"s1"}"#,
        )
        .unwrap();
        assert_eq!(partial.1.as_deref(), Some("s1"));
        assert!(matches!(partial.0, StreamEvent::Delta(ref text) if text == "hello "));

        let final_event = parse_cline_json(
            r#"{"type":"say","say":"text","text":"hello world","partial":false,"session_id":"s1"}"#,
        )
        .unwrap();
        assert!(
            matches!(final_event.0, StreamEvent::Result(ref text, None) if text == "hello world")
        );
    }

    #[test]
    fn parse_cline_json_reports_activity_and_errors() {
        let activity =
            parse_cline_json(r#"{"type":"say","say":"api_req_started","text":"requesting"}"#)
                .unwrap();
        assert!(matches!(activity.0, StreamEvent::Activity(ref text) if text == "请求模型…"));

        let error = parse_cline_json(r#"{"type":"error","text":"bad request"}"#).unwrap();
        assert!(
            matches!(error.0, StreamEvent::Result(ref text, None) if text == "[error] bad request")
        );
    }

    #[test]
    fn escape_cmd_arg_wraps_value_in_double_quotes() {
        // 任何值都被整体双引号包裹，作为 cmd /c 的单个参数字符串
        assert_eq!(escape_cmd_arg("abc"), "\"abc\"");
        assert_eq!(escape_cmd_arg("a & b | c"), "\"a & b | c\"");
    }

    fn open_task(id: &str, created_by: &str) -> CollabTask {
        CollabTask {
            id: id.to_string(),
            room_id: "r1".to_string(),
            title: "t".to_string(),
            description: String::new(),
            status: "open".to_string(),
            assignee: None,
            created_by: created_by.to_string(),
            parent_task: None,
            outcome: None,
            created_at: now_str(),
            updated_at: now_str(),
        }
    }

    /// 认领是 CAS：第二次认领必须失败，且说清是谁占着（抄 open-tag core.ts:840）。
    /// 旧实现「find 到就赋值」会让两个 agent 都以为自己拿到了同一个任务。
    #[test]
    fn claim_is_compare_and_set() {
        let mut store = CollabStore {
            tasks: vec![open_task("t1", "user")],
            ..CollabStore::default()
        };
        let first = claim_task(&mut store, "r1", "t1", "claude-code");
        assert!(first.is_ok(), "首个认领应成功");
        assert_eq!(first.unwrap().assignee.as_deref(), Some("claude-code"));

        let second = claim_task(&mut store, "r1", "t1", "codex");
        let err = second.expect_err("被别人占着就不能再认领");
        assert!(err.contains("claude-code"), "错误信息要指出持有者: {err}");
        // 持有者不能被覆盖
        assert_eq!(store.tasks[0].assignee.as_deref(), Some("claude-code"));
        // 自己重复认领是幂等的（不报错）
        assert!(claim_task(&mut store, "r1", "t1", "claude-code").is_ok());
    }

    /// 终态必须带 outcome，且非法取值要被拒（orca 的 worker_done 强制带 outcome 同款）。
    #[test]
    fn finish_requires_a_known_outcome() {
        let mut store = CollabStore {
            tasks: vec![open_task("t1", "user")],
            ..CollabStore::default()
        };
        assert!(finish_task(&mut store, "r1", "t1", "maybe").is_err());
        assert!(finish_task(&mut store, "r1", "t1", "failed").is_ok());
        assert_eq!(store.tasks[0].status, "done");
        // 「完成」与「做失败了」必须分得开
        assert_eq!(store.tasks[0].outcome.as_deref(), Some("failed"));
        // done 是终态：不能再迁到 claimed
        assert!(claim_task(&mut store, "r1", "t1", "codex").is_err());
    }

    /// 交接要收口：非持有者、非创建者不能把别人的任务转走。
    #[test]
    fn handoff_only_by_holder_or_creator() {
        let mut store = CollabStore {
            tasks: vec![open_task("t1", "user")],
            ..CollabStore::default()
        };
        claim_task(&mut store, "r1", "t1", "claude-code").unwrap();
        assert!(handoff_task(&mut store, "r1", "t1", "codex", "codex").is_err());
        assert!(handoff_task(&mut store, "r1", "t1", "codex", "user").is_ok());
        assert_eq!(store.tasks[0].assignee.as_deref(), Some("codex"));
    }

    /// 输入域：open 放行一切；sealed 只放行人类与白名单工具（抄 open-tag sealed 模式）。
    #[test]
    fn sealed_input_domain_filters_agent_sources() {
        let open_room = CollabRoom {
            id: "r1".to_string(),
            name: "n".to_string(),
            project_path: String::new(),
            input_policy: "open".to_string(),
            allowed_sources: vec![],
            created_at: now_str(),
            updated_at: now_str(),
        };
        assert!(input_allowed(Some(&open_room), "some-random-agent"));

        let sealed = CollabRoom {
            input_policy: "sealed".to_string(),
            allowed_sources: vec!["claude-code".to_string()],
            ..open_room.clone()
        };
        // 人类永远放行
        assert!(input_allowed(Some(&sealed), "user"));
        assert!(input_allowed(Some(&sealed), "claude-code"));
        assert!(!input_allowed(Some(&sealed), "unknown-agent"));
    }

    /// 幂等 receipt：同一个 request_id 第二次能拿到上次的载荷（抄 orca mutation_receipts）。
    #[test]
    fn mutation_receipt_replays_the_same_payload() {
        let key = format!("send::r1::{}", new_id());
        assert!(lookup_receipt(&key).is_none());
        store_receipt(&key, "{\"ok\":true}".to_string());
        assert_eq!(lookup_receipt(&key).as_deref(), Some("{\"ok\":true}"));
        // 换一个 key 不能串味
        assert!(lookup_receipt(&format!("{key}-other")).is_none());
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
        assert!(s
            .chars()
            .all(|c| c.is_numeric() || c == '-' || c == ':' || c == 'T'));
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
        assert!(
            diff < 3600,
            "now_str 应使用 UTC 时间，与本地时间偏差: {}s",
            diff
        );
    }
}

mod models;
pub use models::*;

mod scheduler;
pub use scheduler::*;

mod messaging;
pub use messaging::*;

mod interaction;
pub use interaction::*;

mod room_proxy;
pub use room_proxy::*;

mod runner_codec;
use runner_codec::*;

mod process;
use process::*;

mod agent_bus;
pub use agent_bus::*;

mod dispatch;
use dispatch::*;
