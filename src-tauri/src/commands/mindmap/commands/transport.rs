use super::*;

/// 探索点单专用请求：走共享通道的原生 tool-calling（request_files 工具 + tool_choice
/// 强制点名）。网关不支持 tools 时通道自动降级为纯文本 JSON 协议（system prompt 中的
/// 输出格式约定仍然生效，降级路径无需额外处理）。
/// 其余业务收尾与 [`call_ai_json`] 相同：usage 记账、失败日志。
pub(super) async fn call_ai_json_for_explorer(
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
        ai::usage::tool_ids::MINDMAP,
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
        record_and_emit_usage(app, acc, model, &u);
    }
    Ok(json)
}

/// 从 usage JSON 提取 token 数：累计进本次运行计数器，并推送 step=usage 事件（前端实时统计）。
///
/// 落库不在这里：已下沉到 `ai::channel`（调用必传 tool_id，统一记账，tool_id=mindmap）。
pub(super) fn record_and_emit_usage(
    app: &Option<tauri::AppHandle>,
    acc: &UsageAcc,
    model: &str,
    u: &serde_json::Value,
) {
    use std::sync::atomic::Ordering;
    let prompt_tokens = u.get("prompt_tokens").and_then(|x| x.as_u64()).unwrap_or(0);
    let completion_tokens = u
        .get("completion_tokens")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let total_tokens = u
        .get("total_tokens")
        .and_then(|x| x.as_u64())
        .unwrap_or(prompt_tokens + completion_tokens);
    if prompt_tokens > 0 || completion_tokens > 0 {
        acc.requests.fetch_add(1, Ordering::Relaxed);
        acc.input.fetch_add(prompt_tokens, Ordering::Relaxed);
        acc.output.fetch_add(completion_tokens, Ordering::Relaxed);
        acc.total.fetch_add(total_tokens, Ordering::Relaxed);
        // 全局用量统计不再在这里落库：已下沉到 ai::channel（调用必传 tool_id，
        // 统一记账），这里只管本模块的进度事件与文档级累计。
        emit_progress(
            app,
            "usage",
            serde_json::json!({
                "prompt_tokens": prompt_tokens,
                "completion_tokens": completion_tokens,
                "total_tokens": total_tokens,
                "model": model,
            }),
        );
    }
}

/// 请求一次 AI 并解析出 JSON 对象（流式）。传输韧性（TTFB 超时 / send 重试 /
/// 断点续写 / stream_options 兼容）由共享通道 ai::channel 提供；本函数只做
/// 思维导图业务收尾：token 统计（step=usage + 累计进 acc）、stream done 事件、
/// 宽容 JSON 解析（失败时把 AI 原始输出落盘到滚动日志便于定位）。
pub(super) async fn call_ai_json(
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
        &hooks,
        provider,
        model,
        system,
        user,
        0.3,
        // 通道给的是累积全文，这里只取尾部 120 字符做预览 —— 事件里没必要每次都带上整篇
        |len, acc| {
            let tail: String = acc
                .chars()
                .rev()
                .take(120)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            emit_progress(
                app,
                "stream",
                serde_json::json!({ "length": len, "text": tail }),
            )
        },
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
        record_and_emit_usage(app, acc, model, &u);
    }
    let streamed = outcome.text;
    emit_progress(
        app,
        "stream",
        serde_json::json!({ "done": true, "length": streamed.chars().count() }),
    );
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
                len,
                e,
                head
            );
            Err(e)
        }
    }
}

/// 网络类/传输类失败统一落盘到滚动日志（any-version.log），并推送 fail 事件
/// 让前端进度日志展示原始原因（不再只有「视图生成失败」一句话）。
pub(super) fn log_ai_transport_failure(app: &Option<tauri::AppHandle>, model: &str, err: &str) {
    tracing::error!("[mindmap] AI 请求失败（model={}）：{}", model, err);
    emit_progress(
        app,
        "fail",
        serde_json::json!({ "detail": format!("网络错误（将按可恢复策略处理）：{}", err) }),
    );
}

pub(super) async fn ai_generate_with_repair(
    app: &Option<tauri::AppHandle>,
    acc: &UsageAcc,
    cancel: &std::sync::atomic::AtomicBool,
    provider: &ai::models::AiProvider,
    model: &str,
    system: &str,
    user: &str,
    max_rounds: usize,
    project: Option<&crate::commands::mindmap::scan::ProjectFiles>,
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
                emit_progress(
                    app,
                    "ask",
                    serde_json::json!({
                        "ask": ask.clone(),
                        "round": ask_calls,
                        "max": MAX_ASK_ROUNDS,
                    }),
                );
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
        (
            serde_json::json!({ "nodes": [] }),
            vec!["AI 未返回可用 JSON".into()],
            ask_calls + repair_calls,
        )
    });
    Ok((json, errs, rounds))
}

/// AI 输出带修复循环：生成 → 校验 → （失败）把诊断反馈重试。
/// 若仍有剩余校验错误，如实附加到根节点 detail（不隐藏质量问题）。
pub(super) async fn import_ai_nodes(
    document_id: &str,
    parsed: serde_json::Value,
    errs: Vec<String>,
    root_name: &str,
    replace_existing: bool,
) -> Result<DocumentFull, String> {
    let summary = parsed
        .get("summary")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let id_prefix = format!("{}-", crate::commands::mindmap::db::new_id("ai"));
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
        crate::commands::mindmap::db::with_conn(|c| {
            crate::commands::mindmap::db::sql(c.execute(
                "DELETE FROM mindmap_nodes WHERE document_id=?1",
                rusqlite::params![document_id],
            ))?;
            Ok(())
        })?;
    }
    crate::commands::mindmap::db::batch_save_nodes(&nodes)?;
    crate::commands::mindmap::db::update_document(document_id, None, None, None)?;
    crate::commands::mindmap::db::load_full(document_id)?.ok_or("加载失败".into())
}
