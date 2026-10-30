//! agent bus responsibility.
use super::*;

/// 收尾：写入最终消息 + 推送 msg-updated
pub(super) fn finalize_message(
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
            // 释放前归档（抄 orca `worker_terminal_archives`）：取消 / 超时 / 出错时，
            // 已经产出的那部分输出只存在于这条消息里，进程一结束就没了。
            // 先存一份再覆盖，排查「跑了一半被掐掉」才有据可查。
            if status == "error" && !m.content.trim().is_empty() {
                store.dispatch_archives.push(DispatchArchive {
                    id: new_id(),
                    room_id: room_id.to_string(),
                    tool_id: m.sender.clone(),
                    message_id: msg_id.to_string(),
                    content: m.content.clone(),
                    reason: "error".to_string(),
                    created_at: now_str(),
                });
            }
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
                CollabMsgUpdatedPayload {
                    room_id: room_id.to_string(),
                    message: m.clone(),
                },
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
            avatar: registry()
                .get_tool_config(from)
                .and_then(|c| c.avatar.clone()),
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
            avatar: registry()
                .get_tool_config(&to)
                .and_then(|c| c.avatar.clone()),
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
            avatar: registry()
                .get_tool_config(from)
                .and_then(|c| c.avatar.clone()),
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
            let title = body
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
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
            let parent = body
                .get("parent")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            store.tasks.push(CollabTask {
                id: id.clone(),
                room_id: room_id.to_string(),
                title,
                description: desc,
                status: "open".to_string(),
                assignee: None,
                created_by,
                parent_task: parent,
                outcome: None,
                created_at: now_str(),
                updated_at: now_str(),
            });
            json!({ "ok": true, "id": id })
        }
        "claim" => {
            let id = body.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
            let by = body
                .get("by")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            match claim_task(&mut store, room_id, id, &by) {
                Ok(t) => json!({ "ok": true, "task": t }),
                Err(e) => json!({ "ok": false, "error": e }),
            }
        }
        "complete" => {
            let id = body.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
            // outcome 缺省按 done（历史调用方不带该字段，不能因为加了校验就把它们打回）
            let outcome = body
                .get("outcome")
                .and_then(|v| v.as_str())
                .unwrap_or("done");
            match finish_task(&mut store, room_id, id, outcome) {
                Ok(t) => json!({ "ok": true, "task": t }),
                Err(e) => json!({ "ok": false, "error": e }),
            }
        }
        "handoff" => {
            let id = body.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
            let to = body
                .get("to")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let by = body
                .get("by")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let target = resolve_mention(&to).unwrap_or(to);
            match handoff_task(&mut store, room_id, id, &target, &by) {
                Ok(t) => json!({ "ok": true, "task": t }),
                Err(e) => json!({ "ok": false, "error": e }),
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
    // 幂等（同 [`collab_send_message`]）：带 request_id 的重放直接返回上次结果，
    // 避免 UI 双击 / 崩溃重放建出两个一模一样的任务。
    let receipt_key = body
        .get("request_id")
        .and_then(|v| v.as_str())
        .map(|r| r.trim())
        .filter(|r| !r.is_empty())
        .map(|r| format!("task::{}::{}", room_id, r));
    if let Some(key) = receipt_key.as_ref() {
        if let Some(cached) = lookup_receipt(key) {
            if let Ok(v) = serde_json::from_str::<Value>(&cached) {
                return v;
            }
        }
    }
    let res = collab_task_op(app, &room_id, &body);
    if let Some(key) = receipt_key.as_ref() {
        if let Ok(payload) = serde_json::to_string(&res) {
            store_receipt(key, payload);
        }
    }
    res
}
