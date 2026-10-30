use super::*;

/// 产物深度等级（1 最浅 → 5 最深）对应的生成要求文案，拼进视图级 prompt。
/// 1：只列清单（项目/模块/文件各自的功能一句话）；5：完整业务流走向 + 分支判定方式。
pub(super) fn depth_requirement(depth: u8) -> &'static str {
    match depth {
        1 => "深度要求（最浅）：只输出清单。每个子项目、每个模块、每个文件各占一个节点，detail 用一句话说明其功能；不要展开内部结构，不要分析业务流。节点总数宁少勿滥。",
        2 => "深度要求（较浅）：模块清单 + 每个模块的功能列表。模块下直接列它提供的功能/接口（每个节点 detail 一句话），不需要展开步骤与数据。",
        3 => "深度要求（中等）：模块清单 + 功能，且每个模块的 detail 中简述该模块的关键执行路径（输入→处理→输出）。",
        4 => "深度要求（较深）：在模块/功能之上，整理出主要业务流的完整走向（触发→步骤→分支→结果），每条业务流一个子树，分支处写明判定条件。",
        _ => "深度要求（最深）：完整还原业务流走向与判定方式。每条业务流一个子树：触发条件、逐步骤、每个分支/判断节点必须写明判定依据（什么条件下走哪条路）、异常与回退路径；模块清单退居其次，只为业务流中引用到的模块保留。",
    }
}

pub(super) fn view_label(view: &str) -> String {
    VIEW_LABELS
        .iter()
        .find(|(k, _)| *k == view)
        .map(|(_, l)| l.to_string())
        .unwrap_or_else(|| view.to_string())
}

/// 各视图的生成指导（组织方式），供视图级 prompt 使用。
pub(super) fn view_guidance(view: &str) -> &'static str {
    match view {
        "workflow" => "以流程视角组织：根节点为流程总览，按阶段/步骤/分支展开，标注关键节点、审批与异常分支；kind 用 module/task/constraint/risk/other。",
        "dataflow" => "以数据流视角组织：根节点为数据流总览，按数据源→处理→存储→消费方展开，标注关键管道与依赖；kind 用 module/component/task/other。",
        "sequence" => "以时序视角组织：根节点为入口请求，按调用顺序逐级展开调用链（含返回/异步），detail 中简述请求→响应；kind 用 module/service/task/other。",
        "lifecycle" => "以生命周期视角组织：根节点为业务对象，按状态转移/阶段展开（含重试、等待与终态）；kind 用 module/requirement/constraint/task/other。",
        _ => "以架构视角组织：根节点为项目总览，按模块、组件、服务、路由、配置与关键文件分层。",
    }
}

/// 类型路由分类 prompt：让 AI 先判断适用哪些视图。
pub(super) fn router_prompt(mode: &str) -> String {
    let subject = if mode == "project" {
        "项目扫描结果"
    } else {
        "需求文本"
    };
    format!(
        r##"你是软件架构师。请判断{subject}最值得用哪几种视图生成思维导图。
只输出一个 JSON 对象，不要 Markdown 或解释文字：
{{"views":[{{"type":"architecture","reason":"为什么选它（一句话）"}}]}}
可选 type（按适用度从高到低）：
- architecture：组件/服务/模块拓扑
- workflow：业务流程/任务编排/CI/CD
- dataflow：数据管道/ETL/血缘
- sequence：调用链/请求生命周期
- lifecycle：状态机/生命周期
要求：选择 1 到 3 个最贴合的视图，按重要程度排序；不要选明显不适用的；type 必须来自上述列表。"##
    )
}

/// 视图级生成 prompt（system）：按指定视角组织导图。
/// depth：产物深度（见 depth_requirement）；project 模式下拼进生成要求。
pub(super) fn view_prompt(mode: &str, view: &str, depth: u8) -> String {
    let (kinds, count) = if mode == "project" {
        (
            "root|module|component|service|route|config|file|other",
            "10 到 30",
        )
    } else {
        (
            "root|module|requirement|task|constraint|risk|other",
            "6 到 20",
        )
    };
    let evidence = if mode == "project" {
        "\n请以扫描中的『技术栈与依赖』『目录规模』『项目标记』为证据确认框架、模块边界与部署形态，不要臆造扫描中不存在的依赖或模块。"
    } else {
        "\n只提取文本中有依据的内容，不要臆造。"
    };
    let opener = if mode == "project" {
        format!(
            "你是一位资深软件架构师。请根据用户提供的项目扫描结果，生成「{}」视角的思维导图 JSON。",
            view_label(view)
        )
    } else {
        format!(
            "你是一位产品经理和系统分析师。请从用户需求文本中提取「{}」视角的思维导图 JSON。",
            view_label(view)
        )
    };
    let guidance = view_guidance(view);
    // 深度要求仅项目模式生效（需求文本无“扫描产物”概念，保持原行为）
    let depth_req = if mode == "project" {
        format!("\n{}", depth_requirement(depth))
    } else {
        String::new()
    };
    let tpl = r##"{opener}
只允许输出一个 JSON 对象，不要 Markdown 代码围栏、解释文字或尾随逗号：
{{"summary":"该视角的简明概述","nodes":[{{"id":"唯一稳定短 ID","name":"节点名称","parent_id":null,"detail":"节点说明：一句话职责 + 具体功能，可使用 Markdown","kind":"{kinds}","color":"#RRGGBB","sources":["项目相对路径"]}}]}}
组织要求：{guidance}{evidence}{depth_req}
结构要求：节点字段与思维导图节点数据结构一一对应（id/name/parent_id/detail/kind/color/sources，其中 sources 可省略）；至少一个根节点，根节点 parent_id 必须为 null；其余节点只能通过 parent_id 引用本次输出中的 id；每个节点的 detail 必须写明该模块/节点的说明（职责、边界、与相邻模块的关系，可用 Markdown），不得为空；color 必须是 6 位十六进制颜色；节点总数控制在 {count} 个；只输出 JSON。{evidence_req}
{substance}{ask_req}"##;
    let evidence_req = if mode == "project" {
        "\n证据要求：关键模块/组件/服务节点用 sources 字段标注 1 到 3 个真实文件（项目相对路径，必须在『目录结构』中出现），文件/配置类节点标注自身路径；sources 最多 6 个，只填真实存在的路径，不要臆造。"
    } else {
        ""
    };
    let substance = if mode == "project" {
        VIEW_SUBSTANCE_REQ
    } else {
        ""
    };
    let ask_req = ASK_REQ;
    tpl.replace("{opener}", &opener)
        .replace("{kinds}", kinds)
        .replace("{count}", count)
        .replace("{guidance}", guidance)
        .replace("{evidence}", evidence)
        .replace("{depth_req}", &depth_req)
        .replace("{evidence_req}", evidence_req)
        .replace("{substance}", substance)
        .replace("{ask_req}", ask_req)
}

/// 多轮探索 prompt（system）：AI 请求要读取的文件批次；返回 done 表示探索结束。
pub(super) fn explorer_prompt(mode: &str, max_files: usize) -> String {
    format!(
        r##"你是软件架构师，正在深入分析{subject}。每一轮你会收到：项目结构（目录树、技术栈、模块耦合等）以及上一轮请求的文件内容（已压缩）。
你的任务：决定下一步要读取哪些文件，以便理解项目真实的模块、模块内的功能与业务流程——而不仅是目录结构。每轮最多请求 {max_files} 个文件。
优先读取（按此顺序）：
1. 路由/入口/控制器 → 发现业务操作（endpoint = 业务功能）
2. 服务/用例层实现 → 每个功能的内部逻辑与步骤
3. 数据模型/迁移 → 实体与状态
4. 队列/定时任务/事件 → 异步业务流
目标：读完后你能列出真实模块清单、每个模块的具体功能、以及至少一条端到端业务流（谁→做什么→数据→结果）。
只输出一个 JSON 对象，不要 Markdown 或解释文字：
{{"paths":["src/foo.ts"],"dirs":["src/services"],"done":false,"reason":"为什么读这些文件（一句话）"}}
规则：
- paths 只能是『目录结构』中真实存在的文件（项目相对路径）；最多 {max_files} 个
- dirs 可以请求确认目录存在（不读内容）；最多 {max_files} 个
- done=true 表示探索结束（已读够，可以生成导图）
- 优先读取：入口文件、路由/服务定义、核心模块实现、配置文件
- 不要重复请求已读过的文件"##,
        subject = if mode == "project" {
            "一个项目"
        } else {
            "一段需求"
        },
        max_files = max_files,
    )
}

/// 多轮探索循环：AI 每轮请求一批文件，工具读取并压缩内容作为下一轮上下文。
/// 返回 (最终上下文, 实际探索轮数, 累计读取文件数, 每轮日志)。
pub(super) async fn ai_explore_project(
    app: &Option<tauri::AppHandle>,
    acc: &UsageAcc,
    cancel: &std::sync::atomic::AtomicBool,
    provider: &ai::models::AiProvider,
    model: &str,
    initial_context: &str,
    project: &crate::commands::mindmap::scan::ProjectFiles,
    cfg: &ExplorerSettings,
) -> Result<(String, usize, usize, Vec<AiExploreRound>), String> {
    let mut context = initial_context.to_string();
    let mut requested: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut rounds = 0usize;
    let mut files_read = 0usize;
    let mut round_log: Vec<AiExploreRound> = Vec::new();
    for _round in 0..cfg.explorer_rounds as usize {
        cancel_err(app, cancel)?;
        let req = call_ai_json_for_explorer(
            app,
            acc,
            cancel,
            provider,
            model,
            &explorer_prompt("project", cfg.explorer_files_per_round as usize),
            &context,
        )
        .await?;
        rounds += 1;
        let done = req.get("done").and_then(|v| v.as_bool()).unwrap_or(false);
        let reason = req
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        // 解析顺序很重要：先按设置硬截断（防模型超量点单），再做去重标记——
        // 若先标记后截断，被截掉的文件会被误标为「已读」，下轮点它会遭到错误去重。
        let (files, dirs, rejected) =
            crate::commands::mindmap::scan::parse_ai_file_request(&req, project);
        let max_n = cfg.explorer_files_per_round as usize;
        let (files, dirs) = if files.len() > max_n || dirs.len() > max_n {
            let mut f = files;
            f.truncate(max_n);
            let mut d = dirs;
            d.truncate(max_n);
            (f, d)
        } else {
            (files, dirs)
        };
        // 去重：只把「本轮真正会读」的文件记入已读集合
        let fresh: Vec<String> = files
            .into_iter()
            .filter(|f| requested.insert(f.clone()))
            .collect();
        files_read += fresh.len();
        // Progreso: qué pidió el AI esta ronda y por qué
        emit_progress(
            app,
            "explore",
            serde_json::json!({
                "round": rounds,
                "total": cfg.explorer_rounds as usize,
                "reason": reason,
                "done": done,
            }),
        );
        if !fresh.is_empty() {
            emit_progress(app, "read", serde_json::json!({ "files": fresh }));
        }
        if done || (fresh.is_empty() && dirs.is_empty()) {
            // 探索结束：done=true 时记录 AI 的收尾理由与最后一批请求（不再读取）
            if done && !fresh.is_empty() {
                round_log.push(AiExploreRound {
                    round: rounds,
                    reason: reason.clone(),
                    files: fresh.iter().cloned().collect(),
                    dirs: dirs.iter().cloned().collect(),
                    truncated: false,
                });
            }
            break;
        }
        // 读取并压缩这一批文件，作为下一轮上下文（结构 + 已读内容 + 无效路径回执）
        let mut batch: Vec<(String, Option<String>, bool)> = Vec::new();
        let mut batch_chars = 0usize;
        let mut batch_truncated = false;
        for rel in &fresh {
            let per_file = (cfg.explorer_chars_per_file as usize)
                .min((cfg.explorer_batch_chars as usize).saturating_sub(batch_chars));
            if per_file == 0 {
                break;
            }
            let (content, truncated) =
                crate::commands::mindmap::scan::read_file_compressed(&project.root, rel, per_file);
            batch_truncated |= truncated;
            batch_chars += content.as_ref().map(|t| t.chars().count()).unwrap_or(0);
            batch.push((rel.clone(), content, truncated));
        }
        // 回执：模型点了但不存在/无效的路径明确反馈（含 done 收尾轮，否则模型可能
        // 反复点同一批不存在的文件白白烧轮次）。
        if !rejected.is_empty() {
            emit_progress(
                app,
                "reject",
                serde_json::json!({ "round": rounds, "paths": rejected }),
            );
        }
        context = format!(
            "{}\n\n{}",
            context,
            crate::commands::mindmap::scan::format_file_batch(&batch, &dirs, &rejected)
        );
        // 本轮记录：理由 + 实际读取清单（供导入报告展示探索过程）
        round_log.push(AiExploreRound {
            round: rounds,
            reason: reason.clone(),
            files: fresh.iter().cloned().collect(),
            dirs: dirs.iter().cloned().collect(),
            truncated: batch_truncated,
        });
    }
    Ok((context, rounds, files_read, round_log))
}

/// 类型路由主流程：分类 → 逐视图生成（各建独立文档），单个视图失败不影响其它视图。
/// project_root 非空（项目模式）时收集文件集用于证据校验，并把项目根写入文档 source_desc。
pub(super) async fn run_ai_router(
    provider: &ai::models::AiProvider,
    model: &str,
    mode: &str,
    context: &str,
    root_name: &str,
    project_root: Option<&str>,
    app: &Option<tauri::AppHandle>,
    cancel: &std::sync::atomic::AtomicBool,
    depth: u8,
    requested_views: &[String],
    run_id: &str,
    target_document_id: Option<&str>,
    replace_existing: bool,
) -> Result<AiImportResult, String> {
    // 本次运行的总 token 累计器（逐请求记录 + 完成时随报告返回、随文档留痕）
    let usage = UsageAcc::default();
    // 项目模式：文件集上下文（证据锚定校验用：存在性 + 内容相关度）
    let project_files = match project_root {
        Some(p) => match crate::commands::mindmap::scan::collect_project_files(p) {
            Ok(f) => Some(f),
            Err(_) => None, // 文件集失败不阻断生成，只是跳过证据校验
        },
        None => None,
    };
    // 多轮探索（仅项目模式）：AI 每轮请求一批文件，工具读取并压缩内容追加到上下文。
    // 探索失败不阻断生成，回退为纯结构扫描上下文；但会推送 fail 事件——否则前端
    // 只看到 AI 停在「扫描完成」很久没有动静，不知道探索已失败、正在走降级路径。
    let mut context = context.to_string();
    let mut exploration: Vec<AiExploreRound> = Vec::new();
    // 探索预算来自全局设置（每轮点单上限随 prompt 一起告知 AI）
    let explorer_cfg = crate::commands::mindmap::settings::load_explorer_settings();
    if let Some(pf) = &project_files {
        match ai_explore_project(
            app,
            &usage,
            cancel,
            provider,
            model,
            &context,
            pf,
            &explorer_cfg,
        )
        .await
        {
            Ok((enriched, _rounds, files_read, round_log)) => {
                context = enriched;
                exploration = round_log;
                let _ = files_read; // 统计信息目前仅用于日志/调试
            }
            Err(e) => {
                log_ai_transport_failure(
                    app,
                    model,
                    &format!("探索阶段失败（回退为纯结构扫描继续）: {}", e),
                );
            }
        }
    }
    let context = context.as_str();
    // 1. 确定要生成的视图：用户显式勾选时直接采用（跳过 AI 路由，省一次请求）；
    // 未勾选时走 AI 类型路由判断，失败回退单架构视图（推送 fail 事件告知前端降级原因）。
    let mut views: Vec<String> = Vec::new();
    if !requested_views.is_empty() {
        // 过滤为合法视图名并保持用户勾选顺序
        let mut seen = std::collections::HashSet::new();
        views = requested_views
            .iter()
            .filter(|v| VIEW_LABELS.iter().any(|(k, _)| k == *v))
            .filter(|v| seen.insert((*v).clone()))
            .cloned()
            .collect();
        emit_progress(app, "route", serde_json::json!({ "views": views }));
    }
    if views.is_empty() {
        emit_progress(app, "route", serde_json::json!({}));
        views = vec!["architecture".to_string()];
        match call_ai_json(
            app,
            &usage,
            cancel,
            provider,
            model,
            &router_prompt(mode),
            context,
        )
        .await
        {
            Ok(router_json) => {
                if let Some(arr) = router_json.get("views").and_then(|v| v.as_array()) {
                    let picked: Vec<String> = arr
                        .iter()
                        .filter_map(|v| {
                            v.get("type")
                                .and_then(|t| t.as_str())
                                .map(|s| s.to_string())
                        })
                        .filter(|t| VIEW_LABELS.iter().any(|(k, _)| k == t))
                        .collect();
                    let mut seen = std::collections::HashSet::new();
                    views = picked
                        .into_iter()
                        .filter(|t| seen.insert(t.clone()))
                        .take(3)
                        .collect();
                    if views.is_empty() {
                        views = vec!["architecture".to_string()];
                    }
                }
            }
            Err(e) => {
                // 取消不算降级（run 已被用户终止，后续 cancel 检查会统一处理）；
                // 其余失败推送 fail 并走单「架构」视图回退，前端进度面板有交代。
                if !is_cancelled(cancel) {
                    log_ai_transport_failure(
                        app,
                        model,
                        &format!("视图路由失败（回退为单架构视图）: {}", e),
                    );
                }
            }
        }
    }
    emit_progress(app, "route", serde_json::json!({ "views": views }));

    // 2. 逐视图生成：各建独立文档；失败清理空文档并记录原因
    let mut docs: Vec<DocumentFull> = Vec::new();
    let mut failures: Vec<AiImportFailure> = Vec::new();
    let mut reports: Vec<AiImportReport> = Vec::new();
    let mut primary: Option<String> = None;
    for view in &views {
        cancel_err(app, cancel)?;
        let doc_name = format!("{} · {}", root_name, view_label(view));
        emit_progress(app, "view", serde_json::json!({ "view": view }));
        let (doc, owns_doc) = if primary.is_none() {
            if let Some(target_id) = target_document_id {
                let target = crate::commands::mindmap::db::load_full(target_id)?
                    .ok_or("目标思维导图不存在")?;
                (target.document, false)
            } else {
                (
                    crate::commands::mindmap::db::create_document(
                        &doc_name,
                        "",
                        if mode == "project" {
                            "ai_project"
                        } else {
                            "ai_text"
                        },
                        None,
                    )?,
                    true,
                )
            }
        } else {
            (
                crate::commands::mindmap::db::create_document(
                    &doc_name,
                    "",
                    if mode == "project" {
                        "ai_project"
                    } else {
                        "ai_text"
                    },
                    None,
                )?,
                true,
            )
        };
        let user = if mode == "project" {
            context.to_string()
        } else {
            format!("分析以下文字提取结构化需求：\n\n{}", context)
        };
        let user = if replace_existing && primary.is_none() {
            let existing = crate::commands::mindmap::db::load_full(&doc.id)?
                .map(|f| {
                    f.nodes
                        .iter()
                        .map(|n| {
                            format!(
                                "- {} [{}]：{}",
                                n.name,
                                n.kind,
                                n.detail.chars().take(300).collect::<String>()
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            format!("{}\n\n当前目标思维导图已有节点（请根据用户指令修改，输出修改后的完整节点树；不需要保留被删除的节点）：\n{}", user, existing)
        } else {
            user
        };
        // 项目模式：记录项目根路径到文档 source_desc，供证据文件定位
        if let Some(p) = project_root {
            let _ = crate::commands::mindmap::db::update_source_desc(&doc.id, p);
        }
        // 该视图开始前的用量快照：diff 出「本视图独占」的请求/token（探索/路由等公共部分不计入）
        let view_base = usage.snapshot();
        let outcome = async {
            let (parsed, errs, rounds) = ai_generate_with_repair(
                app,
                &usage,
                cancel,
                provider,
                model,
                &view_prompt(mode, view, depth),
                &user,
                3,
                project_files.as_ref(),
                run_id,
            )
            .await?;
            Ok::<_, String>((parsed, errs, rounds))
        }
        .await;
        // 生成中途被取消：清理半成品文档并整体中止（不是单个视图失败，不继续下一个视图）
        if is_cancelled(cancel) {
            let _ = if owns_doc {
                crate::commands::mindmap::db::delete_document(&doc.id)
            } else {
                Ok(())
            };
            emit_progress(app, "cancel", serde_json::json!({}));
            return Err(ERR_CANCELLED.into());
        }
        let (parsed, errs, rounds) = match outcome {
            Ok(x) => x,
            Err(e) => {
                let _ = if owns_doc {
                    crate::commands::mindmap::db::delete_document(&doc.id)
                } else {
                    Ok(())
                }; // 清理空文档
                emit_progress(
                    app,
                    "fail",
                    serde_json::json!({ "detail": format!("{}: {}", view_label(view), e) }),
                );
                failures.push(AiImportFailure {
                    view: view.clone(),
                    reason: e,
                });
                continue;
            }
        };
        if rounds > 1 {
            emit_progress(
                app,
                "repair",
                serde_json::json!({ "count": errs.len(), "rounds": rounds }),
            );
        }
        let replacing_target = replace_existing && primary.is_none();
        match import_ai_nodes(&doc.id, parsed, errs.clone(), &doc_name, replacing_target).await {
            Ok(mut full) => {
                if primary.is_none() {
                    primary = Some(full.document.id.clone());
                }
                // 本视图 token 留痕：写入文档累计统计，并同步到返回对象让报告弹窗立即可见
                let view_usage = usage.diff(&view_base);
                if view_usage.requests > 0 {
                    let _ = crate::commands::mindmap::db::add_ai_usage(
                        &full.document.id,
                        view_usage.input_tokens,
                        view_usage.output_tokens,
                    );
                    full.document.ai_imports += 1;
                    full.document.ai_input_tokens += view_usage.input_tokens as i64;
                    full.document.ai_output_tokens += view_usage.output_tokens as i64;
                }
                // 证据统计：引用数 / 有证据节点数 / 命中率（项目模式用文件集核验）
                let evidence_count = full.nodes.iter().map(|n| n.sources.len()).sum();
                let evidence_nodes = full.nodes.iter().filter(|n| !n.sources.is_empty()).count();
                let (evidence_hit_count, evidence_verified) = match &project_files {
                    Some(pf) => {
                        let files = &pf.files;
                        let hit = full
                            .nodes
                            .iter()
                            .flat_map(|n| &n.sources)
                            .filter(|s| {
                                files.contains(s.as_str())
                                    || files.iter().any(|f| f.starts_with(&format!("{}/", s)))
                            })
                            .count();
                        (hit, true)
                    }
                    None => (evidence_count, false),
                };
                reports.push(AiImportReport {
                    document_id: full.document.id.clone(),
                    view: view.clone(),
                    node_count: full.nodes.len(),
                    repair_rounds: rounds,
                    diagnostics: errs,
                    evidence_count,
                    evidence_hit_count,
                    evidence_verified,
                    evidence_nodes,
                    usage: view_usage,
                });
                docs.push(full);
                // 视图落库即广播：前端可「边生成边绘制」，不必等全部视图结束。
                // 携带文档 id + 视图名，前端按需拉取 DocumentFull 增量渲染。
                emit_progress(
                    app,
                    "view_done",
                    serde_json::json!({ "doc_id": doc.id, "view": view, "index": docs.len() - 1 }),
                );
            }
            Err(e) => {
                let _ = if owns_doc {
                    crate::commands::mindmap::db::delete_document(&doc.id)
                } else {
                    Ok(())
                };
                failures.push(AiImportFailure {
                    view: view.clone(),
                    reason: e,
                });
            }
        }
    }
    if docs.is_empty() {
        return Err(failures
            .first()
            .map(|f| format!("「{}」视图生成失败: {}", view_label(&f.view), f.reason))
            .unwrap_or_else(|| "AI 未生成任何视图".into()));
    }
    Ok(AiImportResult {
        primary_id: primary.unwrap_or_else(|| docs[0].document.id.clone()),
        documents: docs,
        failures,
        reports,
        usage: usage.snapshot(),
        exploration,
    })
}

/// 取消指定 run_id 的 AI 导入运行（前端点「停止」时调用；各循环/流式块边界会检查标志并中断）。
/// 若运行正阻塞在「询问用户」等待上，同时向询问通道发送取消标记解除阻塞，避免卡死。
#[tauri::command]
pub fn mm_ai_cancel(run_id: String) -> Result<(), String> {
    if run_id.trim().is_empty() {
        return Err("run_id 为空".into());
    }
    ai_cancel_flag(&run_id).store(true, std::sync::atomic::Ordering::Relaxed);
    ask_send_cancel(&run_id);
    Ok(())
}

/// 回填 AI 询问的用户答案（前端 AgentWorkbench 的询问表单提交时调用）。
/// answer 为 JSON 对象（字段键→值）或字符串；后端把回答追加进提示词后继续生成。
#[tauri::command]
pub fn mm_ai_answer(run_id: String, answer: serde_json::Value) -> Result<(), String> {
    if run_id.trim().is_empty() {
        return Err("run_id 为空".into());
    }
    ask_send_answer(&run_id, answer)
}

#[tauri::command]
pub async fn mm_ai_from_project(
    app: tauri::AppHandle,
    input: AiGenerateProjectInput,
) -> Result<AiImportResult, String> {
    let pp = input.project_path.trim().to_string();
    if pp.is_empty() {
        return Err("路径为空".into());
    }
    let pname = std::path::Path::new(&pp)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("项目")
        .to_string();
    let app_opt = Some(app);
    // 本次运行的取消标志：按 run_id 独立，前后端共用同一标识
    let run_id = if input.run_id.trim().is_empty() {
        "import".to_string()
    } else {
        input.run_id.clone()
    };
    let cancel = ai_cancel_flag(&run_id);
    let result = async {
        emit_progress(&app_opt, "scan", serde_json::json!({}));
        // 扫描（含技术栈/目录规模/标记等仓库证据）
        let context = crate::commands::mindmap::scan::scan_project_with_hint(
            &pp,
            input.user_hint.as_deref(),
        )?;
        emit_progress(&app_opt, "scan", serde_json::json!({ "done": true }));
        let (provider, model) = resolve_provider_model(&input.provider_id, &input.model_id)?;
        run_ai_router(
            &provider,
            &model,
            "project",
            &context,
            &pname,
            Some(&pp),
            &app_opt,
            &cancel,
            input.depth.clamp(1, 5),
            &input.views,
            &run_id,
            Some(&input.document_id),
            input.replace_existing,
        )
        .await
    }
    .await;
    ai_drop_flag(&run_id);
    result
}

#[tauri::command]
pub async fn mm_ai_from_text(
    app: tauri::AppHandle,
    input: AiGenerateTextInput,
) -> Result<AiImportResult, String> {
    let text = input.text.trim().to_string();
    if text.is_empty() {
        return Err("文本为空".into());
    }
    let title = if input.title.trim().is_empty() {
        "需求分析"
    } else {
        input.title.trim()
    };
    let (provider, model) = resolve_provider_model(&input.provider_id, &input.model_id)?;
    let app_opt = Some(app);
    let run_id = if input.run_id.trim().is_empty() {
        "import".to_string()
    } else {
        input.run_id.clone()
    };
    let cancel = ai_cancel_flag(&run_id);
    let result = run_ai_router(
        &provider,
        &model,
        "text",
        &text,
        title,
        None,
        &app_opt,
        &cancel,
        3,
        &[],
        &run_id,
        Some(&input.document_id),
        input.replace_existing,
    )
    .await;
    ai_drop_flag(&run_id);
    result
}
