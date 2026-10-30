use super::*;

/// 生成设计文档。**同步返回完整文档**（当前是短请求，不必做流式进度）。
#[tauri::command]
pub async fn dbd_ai_generate(
    app: tauri::AppHandle,
    input: AiDesignInput,
) -> Result<AiOutcome, String> {
    let run_id = input.run_id.clone().unwrap_or_default();
    let text = input.text.trim();
    if text.is_empty() {
        return Err(
            "请先描述要设计什么（例如：一个订单系统，含用户、商品、订单、订单项）".to_string(),
        );
    }
    let (provider, model) = resolve_provider_model(&input.provider_id, &input.model_id)?;

    let dialect = input.dialect.clone().unwrap_or_else(|| "mysql".to_string());
    let name_hint = input
        .name
        .clone()
        .unwrap_or_else(|| "未命名设计".to_string());

    // 画布有表时把当前设计带进提示词：AI 基于它修改（或判断出是全新设计则忽略）
    let user = build_user_prompt(
        &name_hint,
        &dialect,
        text,
        input.current_doc.as_ref(),
        &input.history,
        input.clarify_round,
    );
    // 公共字段配置跟着「当前设计」走：新建设计没有配置，修改场景才需要声明
    let common: &[DbField] = input
        .current_doc
        .as_ref()
        .map(|d| d.common_fields.as_slice())
        .unwrap_or(&[]);

    emit_progress(&Some(app.clone()), "start", serde_json::json!({}));
    let hooks = DbdHooks {
        app: Some(app.clone()),
        run_id: run_id.clone(),
    };
    // 走**流式**通道：AI 每写完一张表就往前端推一份部分文档（`dbd-ai-partial`），
    // 画布边收边画，不用等整篇到齐（100 张表要等几十秒，全程空白很难受）。
    // 通道要求 `Fn`（不可变捕获），「已推过多少个」只能用原子量记录
    let seen_nodes = std::sync::atomic::AtomicUsize::new(0);
    let seen_relations = std::sync::atomic::AtomicUsize::new(0);
    let app_opt = Some(app.clone());
    let outcome = ai::channel::stream_chat_with_resume(
        &hooks,
        &provider,
        &model,
        &system_prompt(common),
        &user,
        0.3,
        |len, acc| {
            // 附带末尾 200 字符做预览：前端控制台实时显示「正在写什么」（与思维导图同款）
            let tail: String = acc
                .chars()
                .rev()
                .take(200)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            emit_progress(
                &app_opt,
                "stream",
                serde_json::json!({ "length": len, "text": tail }),
            );
            // 只有「又多出一个完整节点/关系」才推一次 —— 天然节流，
            // 不会每来 400 字符就刷一遍画布。
            // 已经出现 clarify 就不再推部分文档：追问分支的语义是「不落图」，
            // 中途推过去的部分表会留在画布上变成「半套设计」。
            if acc.contains("\"clarify\"") {
                return;
            }
            let partial = extract_partial(acc);
            let grew = partial.nodes.len() > seen_nodes.load(std::sync::atomic::Ordering::Relaxed)
                || partial.relations.len()
                    > seen_relations.load(std::sync::atomic::Ordering::Relaxed);
            if grew {
                seen_nodes.store(partial.nodes.len(), std::sync::atomic::Ordering::Relaxed);
                seen_relations.store(
                    partial.relations.len(),
                    std::sync::atomic::Ordering::Relaxed,
                );
                emit_partial(&app_opt, &run_id, &partial);
            }
        },
    )
    .await
    .map_err(|e| format!("AI 生成失败：{}", e))?;

    // 兜底：个别网关不认 stream:true（直接回一个普通 JSON），SSE 消费会拿到空文本。
    // 这时退回非流式再取一次，不要让用户看到「AI返回空」。
    let text = if outcome.text.trim().is_empty() {
        ai::channel::complete_chat(
            &hooks,
            &provider,
            &model,
            &system_prompt(common),
            &user,
            0.3,
            None,
            ai::usage::tool_ids::DB_DESIGNER,
        )
        .await
        .map_err(|e| format!("AI 生成失败：{}", e))?
        .text
    } else {
        outcome.text
    };
    if text.trim().is_empty() {
        return Err("AI返回空".into());
    }
    // 上下文解析（容忍会包围栏、前后缀）。失败时再试「只取第一个完整对象」。
    // 都不行则**降级采纳流式里已经收到的完整表**。
    //
    // 为什么必须有这一层（用户实测触发）：模型写很长的 JSON 时会“忘了自己在写 JSON”——
    // 实测有产出在写 relations 的中途直接跳去写 decisions 的内容，中间整段跳过，
    // 结果是花括号不平、`decisions` 键整个消失。补括号救不回来（结构断在关键位置）。
    // 但**断点之前那几张表是完整的**（流式落图一直在用它）—— 为它们报错等于白费一轮还拿不到东西。
    let mut truncated = false;
    let json = match ai::channel::parse_json(&text) {
        Ok(v) => v,
        Err(e) => match extract_first_json_object(&text) {
            Some(v) => v,
            None => {
                let partial = extract_partial(&text);
                if partial.nodes.is_empty() {
                    return Err(format!("AI 返回的内容不是合法 JSON：{}", e));
                }
                truncated = true;
                tracing::warn!(
                    "[dbd] AI 产出结构损坏，降级采纳已完成的 {} 张表（关联与决策记录可能不完整）",
                    partial.nodes.len()
                );
                serde_json::to_value(partial)
                    .map_err(|_| format!("AI 返回的内容不是合法 JSON：{}", e))?
            }
        },
    };

    // 三种形状都要能认：clarify / 包装的 design / 旧的裸文档
    let parsed = parse_shape(json)?;
    // 追问分支：文档一个字都不动。
    // 流式期间可能已经推过部分表过去，所以前端要靠这个返回值把画布恢复到本轮开始前 ——
    // 语义是「这轮什么都没发生，只是我问了句话」。
    // 两个分支都直接返回
    match parsed {
        ParsedOutcome::Clarify { question, options } => {
            emit_progress(
                &Some(app.clone()),
                "clarify",
                serde_json::json!({ "question": question }),
            );
            return Ok(AiOutcome::Clarify { question, options });
        }
        ParsedOutcome::Patch { patch, decisions } => {
            // 增量：合并到**当前文档**（你可能在这一轮里手改过东西，基线必须是最新状态）。
            let Some(base) = input.current_doc.as_ref() else {
                return Err(
                    "画布还没有设计，无法应用增量修改；请先让 AI 生成一份完整设计。".into(),
                );
            };
            let (merged, applied) = crate::commands::db_designer::patch::apply_patch(base, &patch);
            tracing::info!(
                "[dbd] 增量：新增 {} 张 / 更新 {} 张 / 删除 {} 张 / 关联 +{} -{}",
                applied.added_tables.len(),
                applied.updated_tables.len(),
                applied.removed_tables.len(),
                applied.added_relations,
                applied.removed_relations
            );
            if !applied.dropped_relations.is_empty() {
                tracing::warn!(
                    "[dbd] 丢弃 {} 条指向不存在表的关联",
                    applied.dropped_relations.len()
                );
            }
            // 合并结果与全量走同一条链路：规范化 → 自校验重修
            let mut d = merged;
            crate::commands::db_designer::store::normalize_naming(&mut d);
            let (doc, repair) =
                self_repair(d, common, &app, &run_id, &hooks, &provider, &model).await?;
            return Ok(AiOutcome::Patch {
                document: doc,
                decisions,
                applied,
                repair,
            });
        }
        ParsedOutcome::Design {
            document,
            decisions,
        } => {
            // 落地前过一遍规范化：命名改合规 + 外键补索引。
            //
            // 为什么必须在这里做：命名规范是 error 级（阻断保存 / 导出），而提示词只是**概率性**
            // 约束 —— 改过示例与禁令之后模型仍会写出 type_id / title / status 这类裸名，
            // 用户拿到的就是「满屏报错且导不出来」的死锁。规范化是确定性的，且与校验共用同一套
            // 规则（singularize + GENERIC_FIELD_NAMES），不会出现两套口径漂移。
            let mut d = document;
            let normalized = crate::commands::db_designer::store::normalize_naming(&mut d);
            let touched = normalized.renamed_pk.len()
                + normalized.renamed_field.len()
                + normalized.added_index.len();
            if touched > 0 || !normalized.skipped.is_empty() {
                tracing::info!(
                    "[dbd] AI 产出已规范化：主键改名 {}、字段改名 {}、补索引 {}、跳过 {}",
                    normalized.renamed_pk.len(),
                    normalized.renamed_field.len(),
                    normalized.added_index.len(),
                    normalized.skipped.len()
                );
            }
            let (doc, repair) =
                self_repair(d, common, &app, &run_id, &hooks, &provider, &model).await?;
            return Ok(AiOutcome::Design {
                document: doc,
                decisions,
                repair,
                truncated,
            });
        }
    }
}

/// 复用 AI 模块的「供应商 + 模型」解析顺序：显式 provider → 全局默认 → 首个可用 → 活跃模型。
/// （与思维导图 / API 模块同一套逻辑，用户在哪配的模型都能用上。）
pub(super) fn resolve_provider_model(
    pid: &Option<String>,
    mid: &Option<String>,
) -> Result<(ai::models::AiProvider, String), String> {
    let cfg = ai::config::load_ai_config();
    let default_cfg = ai::translate::load_translate_config();
    let (p, explicit_mid) = if let Some(id) = pid {
        (
            cfg.providers
                .iter()
                .find(|x| &x.id == id)
                .cloned()
                .ok_or_else(|| format!("未找到供应商: {}", id))?,
            mid.clone(),
        )
    } else {
        match (&default_cfg.provider_id, mid) {
            (Some(gpid), _) => match cfg.providers.iter().find(|x| &x.id == gpid) {
                Some(p) if !p.openai_url.is_empty() && !p.api_key.is_empty() => {
                    (p.clone(), mid.clone().or(default_cfg.model_id.clone()))
                }
                _ => (first_usable_provider(&cfg)?, None),
            },
            _ => (first_usable_provider(&cfg)?, None),
        }
    };
    if p.openai_url.is_empty() {
        return Err(format!("供应商 '{}' 未配置端点", p.name));
    }
    if p.api_key.is_empty() {
        return Err(format!("供应商 '{}' 未配置 Key", p.name));
    }
    let m = explicit_mid
        .or_else(|| p.active_model_id.clone())
        .or_else(|| p.models.first().map(|m| m.id.clone()))
        .ok_or("无可用模型")?;
    Ok((p, m))
}

pub(super) fn first_usable_provider(
    cfg: &ai::models::AiConfig,
) -> Result<ai::models::AiProvider, String> {
    cfg.providers
        .iter()
        .find(|x| !x.api_key.is_empty() && !x.openai_url.is_empty())
        .cloned()
        .ok_or_else(|| "还没有可用的 AI 供应商：请到 AI 模块配置一个（端点 + Key）".to_string())
}
