use super::*;

/// Agent 工具循环：LLM ↔ 工具（读直执 / 写经前端），直到给出最终回答或轮数耗尽。
/// `messages[0]` 必须是 system。返回最终回复与本轮实际提交的 ops（用于落库）。
#[allow(clippy::too_many_arguments)]
pub(super) async fn agent_run(
    app: &Option<tauri::AppHandle>,
    cancel: &std::sync::atomic::AtomicBool,
    acc: &UsageAcc,
    provider: &ai::models::AiProvider,
    model: &str,
    full: &mut DocumentFull,
    session_id: &str,
    run_id: &str,
    messages: &mut Vec<serde_json::Value>,
) -> Result<AgentTurn, String> {
    let hooks = MmHooks { app, cancel };
    let mut all_ops: Vec<serde_json::Value> = Vec::new();
    // 轮数上限来自设置（「AI 探索参数」里的 Agent 轮数），不再硬编码
    let max_rounds =
        crate::commands::mindmap::settings::load_explorer_settings().agent_rounds as usize;
    for round in 0..max_rounds {
        cancel_err(app, cancel)?;
        let outcome = ai::channel::complete_chat_messages(
            &hooks,
            provider,
            model,
            messages,
            0.4,
            Some(AGENT_TOOLS_SPEC),
            ai::usage::tool_ids::MINDMAP,
        )
        .await
        .map_err(|e| {
            if e.contains("tool") && e.contains("400") {
                format!("{}（当前模型/网关可能不支持工具调用，请更换模型）", e)
            } else {
                e
            }
        })?;
        if let Some(u) = &outcome.usage {
            record_and_emit_usage(app, acc, model, u);
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
            let _ = crate::commands::mindmap::db::agent_append_message(
                session_id,
                "assistant",
                &reply,
                "[]",
            );
            return Ok(AgentTurn {
                reply,
                rounds: round + 1,
            });
        }
        // 「思考过程」外显：模型在发起工具调用前写在 content 里的想法，以及
        // 支持 reasoning 的模型给出的 reasoning_content。以前两者都被直接丢掉，
        // 用户只看到工具结果，看不到 Agent 是怎么想的。
        let thought = message
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .trim()
            .to_string();
        if !thought.is_empty() {
            emit_progress(
                app,
                "agentThought",
                serde_json::json!({ "round": round + 1, "text": truncate_for_log(&thought, 800) }),
            );
        }
        if let Some(reasoning) = outcome.reasoning.as_deref() {
            let reasoning = reasoning.trim();
            if !reasoning.is_empty() {
                emit_progress(
                    app,
                    "agentReasoning",
                    serde_json::json!({ "round": round + 1, "text": truncate_for_log(reasoning, 1200) }),
                );
            }
        }
        // assistant(tool_calls) 原样回传（OpenAI 协议要求带上它才能接 tool 消息）
        messages.push(message.clone());
        let mut tool_results: Vec<serde_json::Value> = Vec::new();
        for tc in &tool_calls {
            let call_id = tc
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let name = tc
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let raw_args = tc.get("function").and_then(|f| f.get("arguments")).cloned();
            let args = match raw_args {
                Some(serde_json::Value::String(s)) => {
                    serde_json::from_str::<serde_json::Value>(s.trim())
                        .unwrap_or(serde_json::json!({}))
                }
                Some(v @ serde_json::Value::Object(_)) => v,
                _ => serde_json::json!({}),
            };
            // 工具调用外显：读工具（查大纲/找节点）以前一个事件都不发，用户只看到
            // 「AI 在想…」却不知道它翻了什么；现在连同参数一起回传。
            emit_progress(
                app,
                "agentTool",
                serde_json::json!({
                    "round": round + 1,
                    "tool": name,
                    "args": truncate_for_log(&args.to_string(), 400),
                }),
            );
            let result: serde_json::Value = match name.as_str() {
                "get_document_overview" => agent_tool_overview(full),
                "get_subtree" => agent_tool_subtree(
                    full,
                    args.get("rootId")
                        .and_then(|v| v.as_str())
                        .unwrap_or_else(|| {
                            args.get("root_id").and_then(|v| v.as_str()).unwrap_or("")
                        }),
                ),
                "find_nodes" => agent_tool_find(
                    full,
                    args.get("keyword").and_then(|v| v.as_str()).unwrap_or(""),
                    args.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                ),
                "create_root_node" | "add_child_nodes" | "add_subtree" | "update_nodes"
                | "delete_nodes" | "move_nodes" => {
                    match agent_handle_write(app, cancel, full, run_id, &name, &args) {
                        Ok((ops, ack)) => {
                            all_ops.extend(ops.iter().cloned());
                            // 纯 ops 载荷落库：回放时展示，不重放执行
                            let _ = crate::commands::mindmap::db::agent_append_message(
                                session_id,
                                "assistant",
                                "",
                                &serde_json::to_string(&ops).unwrap_or_else(|_| "[]".into()),
                            );
                            ack
                        }
                        Err(e) => serde_json::json!({ "status": "error", "error": e }),
                    }
                }
                other => {
                    serde_json::json!({ "status": "error", "error": format!("未知工具 {}", other) })
                }
            };
            tool_results.push(serde_json::json!({
                "role": "tool",
                "tool_call_id": call_id,
                "content": result.to_string(),
            }));
        }
        messages.extend(tool_results);
    }
    Err(format!(
        "Agent 连续 {} 轮未给出最终回答，已停止。可在「思维导图设置 → AI 探索参数」调大 Agent 轮数后重试",
        max_rounds
    ))
}

/// Agent 一轮对话入口：确保会话 → 构建上下文与历史 → 跑工具循环 → 落库并返回。
#[tauri::command]
pub async fn mm_agent_chat(
    app: tauri::AppHandle,
    input: AgentChatInput,
) -> Result<AgentChatResult, String> {
    let app_opt = Some(app);
    let user_text = input.message.trim().to_string();
    let run_id = if input.run_id.trim().is_empty() {
        "agent".to_string()
    } else {
        input.run_id.clone()
    };
    let cancel = ai_cancel_flag(&run_id);
    let result = async {
        // mut：Agent 的写工具会把变更同步进这份快照，同一轮里后续读工具才看得到
        let mut full =
            crate::commands::mindmap::db::load_full(&input.document_id)?.ok_or("文档不存在")?;
        let (provider, model) = resolve_provider_model(&input.provider_id, &input.model_id)?;
        let session_id = if input.session_id.trim().is_empty() {
            crate::commands::mindmap::db::agent_ensure_session(&input.document_id)?
        } else {
            input.session_id.clone()
        };

        // 上下文：system + 历史（按预算裁剪）+ 大纲/选中子树 + 本轮用户输入
        let history = crate::commands::mindmap::db::agent_list_messages(&session_id)?;
        let mut messages = vec![
            serde_json::json!({ "role": "system", "content": agent_prompt(&full.document.name) }),
        ];
        let mut insert_at = 1usize;
        for (role, content) in agent_history_window(&history, AGENT_HISTORY_CHAR_BUDGET) {
            messages.insert(
                insert_at,
                serde_json::json!({ "role": role, "content": content }),
            );
            insert_at += 1;
        }
        let selected = if input.selected_node_ids.is_empty() {
            String::from("当前没有选中节点。")
        } else {
            let mut parts = Vec::new();
            for id in input.selected_node_ids.iter().take(3) {
                let Some(node) = full.nodes.iter().find(|n| n.id == *id) else {
                    continue;
                };
                // 明确给出「选中节点的 id」而不只是子树 JSON：图里同名节点很多，
                // 模型要能一眼知道「用户选中的那个『扩展』的 id 是 X」
                parts.push(format!(
                    "选中节点：id = {}，名称 = {}，路径 = {}\n完整子树：{}",
                    node.id,
                    node.name,
                    agent_node_path(&full.nodes, &node.id),
                    agent_tool_subtree(&full, id)
                ));
            }
            if parts.is_empty() {
                String::from("当前没有选中节点。")
            } else {
                format!("用户当前选中的节点：\n{}", parts.join("\n"))
            }
        };
        let user_content = if user_text.is_empty() {
            format!("（用户点击了继续）\n\n{}", selected)
        } else {
            format!("{}\n\n{}", user_text, selected)
        };
        // @ 引用的文件：内容进上下文（模型在正文里也能看到 @路径 引用）
        let attachments =
            agent_attachments_block(full.document.project_dir.as_deref(), &input.attached_files);
        let user_content = if attachments.is_empty() {
            user_content
        } else {
            format!("{}\n\n用户引用的文件内容：{}", user_content, attachments)
        };
        messages.push(serde_json::json!({ "role": "user", "content": user_content }));

        // 空输入（「继续」）不重复落一条空用户消息
        if !user_text.is_empty() {
            crate::commands::mindmap::db::agent_append_message(
                &session_id,
                "user",
                &user_text,
                "[]",
            )?;
        }

        let usage = UsageAcc::default();
        let turn = agent_run(
            &app_opt,
            &cancel,
            &usage,
            &provider,
            &model,
            &mut full,
            &session_id,
            &run_id,
            &mut messages,
        )
        .await?;
        let u = usage.snapshot();
        if u.requests > 0 {
            let _ = crate::commands::mindmap::db::add_ai_usage(
                &input.document_id,
                u.input_tokens,
                u.output_tokens,
            );
        }
        Ok(AgentChatResult {
            session_id,
            reply: turn.reply,
            rounds: turn.rounds as u32,
        })
    }
    .await;
    ai_drop_flag(&run_id);
    result
}

/// 取文档的 Agent 会话 id（不存在则新建）。
#[tauri::command]
pub fn mm_agent_get_session(document_id: String) -> Result<String, String> {
    crate::commands::mindmap::db::agent_ensure_session(&document_id)
}

/// 列出会话消息（按时间正序；前端回放右栏对话）。
#[tauri::command]
pub fn mm_agent_list_messages(session_id: String) -> Result<Vec<AgentMessageRow>, String> {
    crate::commands::mindmap::db::agent_list_messages(&session_id)
}

/// 列出文档的 Agent 会话（一个文档可有多个；最近使用在前）。
#[tauri::command]
pub fn mm_agent_list_sessions(document_id: String) -> Result<Vec<AgentSessionRow>, String> {
    crate::commands::mindmap::db::agent_list_sessions(&document_id)
}

/// 新建会话：同一文档开一条新的独立历史（不再复用「文档唯一会话」）。
#[tauri::command]
pub fn mm_agent_new_session(document_id: String) -> Result<String, String> {
    crate::commands::mindmap::db::agent_create_session(&document_id, "")
}

/// 删除会话（连带消息；不影响该文档的其它会话与画布内容）。
#[tauri::command]
pub fn mm_agent_delete_session(session_id: String) -> Result<(), String> {
    crate::commands::mindmap::db::agent_delete_session(&session_id)
}

/// 重命名会话（空标题 = 取消标题，回退展示首条用户消息）。
#[tauri::command]
pub fn mm_agent_rename_session(session_id: String, title: String) -> Result<(), String> {
    crate::commands::mindmap::db::agent_rename_session(&session_id, &title)
}

/// 分叉会话：复制截至 message_id（空 = 全部）的历史到新会话，原会话保持不变。
///
/// 只复制「上下文」，不重放写图操作——画布改动已在图上，分叉只改变后续对话能看到的历史。
#[tauri::command]
pub fn mm_agent_fork_session(session_id: String, message_id: String) -> Result<String, String> {
    crate::commands::mindmap::db::agent_fork_session(&session_id, &message_id)
}
