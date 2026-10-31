use super::*;

/// 用 AI 给收藏打分类标签（多标签，支持多级，如「编程语言/Rust」）。
///
/// - 默认只对「AI 还没归类过」且未被人工锁定的条目跑；
/// - `reclassify = true` 时对**所有**未锁定、未失效的条目跑：先清掉旧的 AI 分类
///   （只清 AI 加的，浏览器书签目录与人工选择保留），再写新结果；
/// - 每批 [`BATCH_SIZE`] 条，一批失败就整批放弃并报错（不写半截错误分类）；
/// - `limit` 给一个上限，方便先试 40 条看效果再决定要不要全量跑。
#[tauri::command]
pub async fn fav_classify(
    app: tauri::AppHandle,
    provider_id: Option<String>,
    model_id: Option<String>,
    limit: Option<usize>,
    reclassify: Option<bool>,
) -> Result<ClassifyResult, String> {
    crate::runtime_tasks::run_if_enabled("favorites", async move {
        let _task = TaskGuard::begin(TASK_CLASSIFY)?;
        let out = classify_inner(
            app,
            provider_id,
            model_id,
            limit,
            reclassify.unwrap_or(false),
        )
        .await;
        out
    })
    .await
}

/// 让 AI 给出「分类层级整体整理」方案（只生成、不落库）。
///
/// 输出「新建缺失分组 + 把分类移到更合适的父级下」，从整体结构出发重排，
/// 而不是只修明显放错的分类；不删除 / 不改名 / 不改已有 ID，条目关联不受影响。
/// 前端先预览勾选，再调 [`fav_apply_category_moves`]。
#[tauri::command]
pub async fn fav_plan_category_moves(
    provider_id: Option<String>,
    model_id: Option<String>,
) -> Result<organize::CategoryMovePlan, String> {
    crate::runtime_tasks::run_if_enabled("favorites", async move {
        let cfg = crate::commands::ai::config::load_ai_config();
        let (provider, model) = resolve_ai_target(&cfg, &provider_id, &model_id)?;
        let cats = db::with_conn(|conn| db::category_briefs(conn))?;
        let considered = cats.iter().filter(|c| c.kind != "follow").count();
        // 少于两个主题分类时不可能形成父子关系，直接返回空方案，不白花一次模型调用
        if considered < 2 {
            return Ok(organize::CategoryMovePlan {
                model,
                considered,
                groups: Vec::new(),
                moves: Vec::new(),
                skipped: Vec::new(),
            });
        }

        let prompt = organize::build_prompt(&cats);
        let started = std::time::Instant::now();
        // 必须走流式：400 个分类的方案输出可能要几分钟，非流式要等整段生成完才返回响应头，
        // 会直接撞上 180s 的 TTFB 上限。流式只要首块到达就拿到响应头，后续多少块都收得下。
        let outcome = crate::commands::ai::channel::stream_chat_with_resume(
            &crate::commands::ai::channel::NoHooks,
            &provider,
            &model,
            organize::SYSTEM_PROMPT,
            &prompt,
            0.2,
            |_, _| {},
        )
        .await
        .map_err(|e| format!("{}（供应商: {}，模型: {}）", e, provider.name, model))?;
        if let Some(usage) = &outcome.usage {
            crate::commands::ai::usage::log_usage_from_json_timed(
                crate::commands::ai::usage::tool_ids::FAVORITES,
                &model,
                Some(&provider.id),
                usage,
                started.elapsed().as_millis() as u64,
            );
        }
        // 个别网关不认 stream:true（回普通 JSON），SSE 会读到空文本；退回非流式再取一次
        let text = if outcome.text.trim().is_empty() {
            crate::commands::ai::channel::complete_chat(
                &crate::commands::ai::channel::NoHooks,
                &provider,
                &model,
                organize::SYSTEM_PROMPT,
                &prompt,
                0.2,
                None,
                crate::commands::ai::usage::tool_ids::FAVORITES,
            )
            .await
            .map_err(|e| format!("{}（供应商: {}，模型: {}）", e, provider.name, model))?
            .text
        } else {
            outcome.text
        };

        let (groups, raw_moves) = organize::parse_plan(&text)?;
        let plan = organize::build_plan(&cats, &groups, &raw_moves);
        crate::exit_log!(
            "[收藏] 整理类目方案: model={}, considered={}, groups={}, moves={}, skipped={}",
            model,
            considered,
            plan.groups.len(),
            plan.moves.len(),
            plan.skipped.len()
        );
        Ok(organize::CategoryMovePlan {
            model,
            considered,
            groups: plan.groups,
            moves: plan.moves,
            skipped: plan.skipped,
        })
    })
    .await
}

/// 应用整理方案：可按需新建分组，再只改已有分类的 `parent_id` / `sort_order`。
/// 已有分类 ID 与条目关联保持不变。
#[tauri::command]
pub fn fav_apply_category_moves(
    moves: Vec<db::CategoryMove>,
    groups: Option<Vec<organize::PlannedGroup>>,
) -> Result<organize::ApplyResult, String> {
    let groups = groups.unwrap_or_default();
    let result = db::with_conn(|conn| organize::apply(conn, &groups, &moves))?;
    crate::exit_log!(
        "[收藏] 应用整理类目: created={}, applied={}, skipped={}",
        result.created,
        result.applied,
        result.skipped.len()
    );
    Ok(result)
}

pub(super) async fn classify_inner(
    app: tauri::AppHandle,
    provider_id: Option<String>,
    model_id: Option<String>,
    limit: Option<usize>,
    reclassify: bool,
) -> Result<ClassifyResult, String> {
    let cfg = crate::commands::ai::config::load_ai_config();
    let (provider, model) = resolve_ai_target(&cfg, &provider_id, &model_id)?;

    reset_cancel(TASK_CLASSIFY);
    let budget = limit.unwrap_or(usize::MAX);
    let mut result = ClassifyResult {
        model: model.clone(),
        ..ClassifyResult::default()
    };
    // 重新归类：先把已归类条目的 AI 标记清空，让它们重新进「未归类」批次。
    // 旧的 AI 分类关联由 apply_tags 在逐批写入时清掉（只清 ai 来源）。
    if reclassify {
        db::with_conn(|conn| db::reset_classification(conn))?;
    }
    // 总盘子 = 开始时待处理的条数；进度条用它算百分比
    let total_pending = db::with_conn(|conn| db::select_unclassified(conn, 1_000_000))?.len();
    let mut batch_no = 0usize;

    // 先发一条 0/N：每批要等模型返回（几十秒都可能），不先点亮进度条用户会以为没反应
    emit_progress(
        &app,
        &FavoritesProgress {
            stage: "classify",
            task: TASK_CLASSIFY.to_string(),
            source: None,
            message: Some(if reclassify {
                format!("重新归类，每批 {} 条", BATCH_SIZE)
            } else {
                format!("每批 {} 条", BATCH_SIZE)
            }),
            classified: Some(0),
            tags_written: Some(0),
            remaining: Some(total_pending),
            done: false,
            ..FavoritesProgress::default()
        },
    );

    loop {
        // 停止检查放在**每批开始前**：进行中的那一批请求照跑完（已付的 token 不浪费），
        // 但不再发下一批。这样「停止」不会留下半截结果。
        if is_cancelled(TASK_CLASSIFY) {
            result.cancelled = true;
            break;
        }
        let done = result.classified;
        if done >= budget {
            break;
        }
        let batch = db::with_conn(|conn| db::select_unclassified(conn, BATCH_SIZE))?;
        if batch.is_empty() {
            break;
        }
        batch_no += 1;
        // 每批都读一次现有分类：上一批新建的分类也要进入下一批的候选，
        // 否则模型会一直按自己的命名重新发明同类分类（平级重复的根因）。
        let existing = db::with_conn(|conn| db::category_paths(conn, 200))?;
        let prompt = classify::build_prompt(&batch, &existing);
        let outcome = crate::commands::ai::channel::complete_chat(
            &crate::commands::ai::channel::NoHooks,
            &provider,
            &model,
            classify::SYSTEM_PROMPT,
            &prompt,
            0.3,
            None,
            crate::commands::ai::usage::tool_ids::FAVORITES,
        )
        .await
        .map_err(|e| format!("{}（供应商: {}，模型: {}）", e, provider.name, model))?;

        // 解析失败 → 直接返回错误：整批不落库，避免写进半截错误分类
        let groups = classify::parse_groups(&outcome.text, batch.len())?;
        // 这一批的思考内容：归类是「按模型判断该进哪个分类」，判断依据全在思考里，
        // 分类错了要能回头查它当时是怎么想的。以前完全没有上报，前端只知道进度。
        if let Some(reasoning) = outcome
            .reasoning
            .as_deref()
            .filter(|r| !r.trim().is_empty())
        {
            emit_progress(
                &app,
                &FavoritesProgress {
                    stage: "reasoning",
                    task: TASK_CLASSIFY.to_string(),
                    message: Some(reasoning.trim().to_string()),
                    classified: Some(result.classified),
                    tags_written: Some(result.tags_written),
                    remaining: Some(total_pending.saturating_sub(result.classified)),
                    done: false,
                    ..FavoritesProgress::default()
                },
            );
        }
        let written =
            db::with_conn(|conn| db::apply_tags(conn, &batch, &groups, &model, reclassify))?;
        result.tags_written += written;
        result.classified += batch.len();

        emit_progress(
            &app,
            &FavoritesProgress {
                stage: "classify",
                task: TASK_CLASSIFY.to_string(),
                message: Some(format!("第 {} 批（每批 {} 条）", batch_no, BATCH_SIZE)),
                classified: Some(result.classified),
                tags_written: Some(result.tags_written),
                remaining: Some(total_pending.saturating_sub(result.classified)),
                done: false,
                ..FavoritesProgress::default()
            },
        );

        // 用量已由 ai::channel 统一记账（tool_id=favorites），这里不再重复落库
    }

    result.remaining = db::with_conn(|conn| db::select_unclassified(conn, 1_000_000))?.len();

    // 重新归类收尾：清掉旧的 AI 关联后，被 AI 用过、现在空了的分类（含历史遗留）要收掉，
    // 否则侧栏会堆一堆 0 条目的空节点，看着乱。
    if reclassify {
        let removed = db::with_conn(|conn| db::prune_all_empty_categories(conn))?;
        if removed > 0 {
            crate::exit_log!("[收藏] 重新归类后清理空分类 {} 个", removed);
        }
    }

    crate::exit_log!(
        "[收藏] AI 归类完成: model={}, classified={}, tags={}, remaining={}",
        result.model,
        result.classified,
        result.tags_written,
        result.remaining
    );
    emit_progress(
        &app,
        &FavoritesProgress {
            stage: "classify",
            task: TASK_CLASSIFY.to_string(),
            classified: Some(result.classified),
            tags_written: Some(result.tags_written),
            remaining: Some(result.remaining),
            done: true,
            ..FavoritesProgress::default()
        },
    );
    Ok(result)
}
