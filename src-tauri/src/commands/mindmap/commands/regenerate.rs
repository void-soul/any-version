use super::*;

/// 子树重析 prompt：结构化 JSON 输出 + 校验约束（kind 白名单/color/父引用）。
pub(super) fn regenerate_prompt() -> String {
    r##"你是一位资深软件架构师。请分析指定模块的内部结构，生成可直接导入思维导图的 JSON。
只允许输出一个 JSON 对象，不要 Markdown 代码围栏、解释文字或尾随逗号：
{"nodes":[{"id":"唯一稳定短 ID","name":"节点名称","parent_id":null,"detail":"节点说明：职责、边界与相邻模块关系，可使用 Markdown","kind":"root|module|component|service|route|config|file|task|requirement|constraint|risk|other","color":"#RRGGBB"}]}
要求：节点字段与思维导图节点数据结构一一对应（id/name/parent_id/detail/kind/color）；第一个节点是该模块自身（parent_id 必须为 null，kind 用 root 或 module），其余 3 到 12 个节点是其子结构；所有子节点只能通过 parent_id 引用本批输出中的 id；每个节点的 detail 必须写明该模块/子模块的说明（职责、边界、与相邻模块的关系，可用 Markdown），不得为空；kind 必须在允许列表内；color 必须是 6 位十六进制颜色；只输出 JSON。"##.to_string()
}

#[tauri::command]
pub async fn mm_regenerate_node(
    app: tauri::AppHandle,
    input: RegenerateNodeInput,
) -> Result<DocumentFull, String> {
    let app_opt = Some(app);
    let usage = UsageAcc::default();
    let full = crate::commands::mindmap::db::load_full(&input.document_id)?.ok_or("文档不存在")?;
    let target = full
        .nodes
        .iter()
        .find(|n| n.id == input.node_id)
        .ok_or("节点不存在")?;
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
        if detail_take.is_empty() {
            "（无）".to_string()
        } else {
            detail_take
        },
        children_txt,
    );
    // 生成 → 校验 → （失败）诊断反馈重试，最多 3 轮（子树重析不做证据校验）；
    // 用量同样累计：重析完成的 token 一并写入文档留痕。重析有自己独立的取消标志（导入的
    // 「停止」按钮不会误伤正在进行的重析）。
    let regen_run = format!("regen-{}", crate::commands::mindmap::db::new_id("run"));
    let cancel = ai_cancel_flag(&regen_run);
    let (parsed, errs, _rounds) = ai_generate_with_repair(
        &app_opt,
        &usage,
        &cancel,
        &provider,
        &model,
        &regenerate_prompt(),
        &user,
        3,
        None,
        &regen_run,
    )
    .await?;
    ai_drop_flag(&regen_run);
    let run_usage = usage.snapshot();
    if run_usage.requests > 0 {
        let _ = crate::commands::mindmap::db::add_ai_usage(
            &input.document_id,
            run_usage.input_tokens,
            run_usage.output_tokens,
        );
    }
    let id_prefix = format!("{}-", crate::commands::mindmap::db::new_id("ai-child"));
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
        crate::commands::mindmap::db::with_conn(|c| {
            crate::commands::mindmap::db::sql(c.execute(
                "UPDATE mindmap_nodes SET detail=?1, updated_at=?2 WHERE id=?3",
                rusqlite::params![detail, crate::commands::mindmap::db::now_ts(), target.id],
            ))?;
            Ok(())
        })?;
    }
    let root_ai = new_children
        .iter()
        .find(|n| n.parent_id.is_none())
        .map(|n| n.id.clone())
        .unwrap_or_default();
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
        crate::commands::mindmap::db::with_conn(|c| {
            for id in &desc_set {
                crate::commands::mindmap::db::sql(c.execute(
                    "DELETE FROM mindmap_nodes WHERE id=?1",
                    rusqlite::params![id],
                ))?;
                // 被删除的后代上挂的额外连线一并清理（来源或目标）
                crate::commands::mindmap::db::sql(c.execute(
                    "DELETE FROM mindmap_links WHERE source_id=?1 OR target_id=?1",
                    rusqlite::params![id],
                ))?;
            }
            Ok(())
        })?;
    }
    // 插入新子节点
    for child in &new_children {
        if child.id == root_ai || child.parent_id.is_none() {
            continue;
        }
        let mut n = child.clone();
        if n.parent_id.as_deref() == Some(&root_ai) {
            n.parent_id = Some(target.id.clone());
        }
        crate::commands::mindmap::db::upsert_node(&n)?;
    }
    crate::commands::mindmap::db::load_full(&input.document_id)?.ok_or("加载失败".into())
}
