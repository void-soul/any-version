//! messaging responsibility.
use super::*;

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
    // 幂等键：前端每次「发送」生成一个 id 并带上；UI 重发 / 崩溃重放时后端据此
    // 直接返回上次的结果，不会再真的拉起一个 agent（抄 orca `mutation_receipts`）。
    request_id: Option<String>,
) -> Result<Vec<CollabMessage>, String> {
    eprintln!("[collab] ▶ 发送消息: room={}, tool={}, model={:?}, provider={:?}, content_len={}, refs={}, files={}",
        room_id, tool_id, model_id, provider_id, content.len(), references.len(), files.len());
    // 幂等重放：同一个 request_id 再来一次就返回上次的消息，不重复派发
    let receipt_key = request_id
        .as_deref()
        .map(|r| r.trim())
        .filter(|r| !r.is_empty())
        .map(|r| format!("send::{}::{}", room_id, r));
    if let Some(key) = receipt_key.as_ref() {
        if let Some(cached) = lookup_receipt(key) {
            if let Ok(msgs) = serde_json::from_str::<Vec<CollabMessage>>(&cached) {
                eprintln!("[collab] ↺ 幂等重放（request_id 已处理过），直接返回上次结果");
                return Ok(msgs);
            }
        }
    }
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
                if t.state == "collecting"
                    && t.room_id == room_id
                    && t.sender == "user"
                    && t.owner_tool_id.as_deref() == Some(tool_id.as_str())
                    && parse_ts(&t.dispatch_after).unwrap_or(0) > now_ts
                {
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

    if let Some(key) = receipt_key.as_ref() {
        if let Ok(payload) = serde_json::to_string(&result) {
            store_receipt(key, payload);
        }
    }

    Ok(result)
}

/// 把用户消息 + 引用 + 文件内容拼成派发提示词
pub(super) fn build_prompt(
    content: &str,
    refs: &[CollabReference],
    files: &[CollabFileRef],
) -> String {
    let mut p = String::new();
    p.push_str(content.trim());
    if !refs.is_empty() {
        p.push_str("\n\n--- 引用内容 ---\n");
        for r in refs {
            p.push_str(&format!(
                "【来自 {}】\n{}\n\n",
                r.source_sender_name,
                r.excerpt.trim()
            ));
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
pub(super) fn read_file_capped(path: &str) -> String {
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
