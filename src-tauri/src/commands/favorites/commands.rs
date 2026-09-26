//! 收藏模块的 Tauri 命令。
//!
//! 全部**只读**：不调用任何平台的写入接口。

use std::sync::Mutex;

use serde::Serialize;
use serde_json::Value;

use super::bilibili;
use super::check;
use super::zhihu;
use super::classify::{self, BATCH_SIZE};
use super::db::{self, NewFavorite};
use super::github;

// ─── 任务调度：谁可以和谁同时跑 ───
//
// 三个平台的导入各拉各的接口、互不相干，**允许同时跑**（用户点三个「导入」就该三个一起跑）；
// 但「导入」与「加工」（AI 归类 / 失效检测）必须互斥：加工要扫全库挑条目，
// 和正在写入的导入抢同一份数据，也会把刚导入、还没稳定的条目算进去。
//
// 因此这里维护两张表（都按**任务名**分开记，不再是一个全局 bool）：
// - RUNNING：当前正在跑的任务，用于互斥判定；
// - CANCEL：用户点过「停止」的任务，用于循环里的中断检查。
//
// 用一张全局 bool 的时代，任一任务入口都会把它清掉 —— 并发下这等于顺手抹掉别人的
// 停止请求；点「停止 GitHub」也会把同时跑的 B站/知乎一起停掉。

const TASK_GITHUB: &str = "github";
const TASK_BILIBILI: &str = "bilibili";
const TASK_ZHIHU: &str = "zhihu";
const TASK_CLASSIFY: &str = "classify";
const TASK_CHECK: &str = "check";
const ALL_TASKS: [&str; 5] = [TASK_GITHUB, TASK_BILIBILI, TASK_ZHIHU, TASK_CLASSIFY, TASK_CHECK];

/// 当前正在跑的任务名（同一时刻最多：三个导入，或单独一个加工任务）。
static RUNNING: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// 被用户点「停止」的任务名。
static CANCEL: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// 锁被毒化时照样取内容：这里存的是任务名列表，坏掉一个也不会让收藏模块不可用。
fn tasks_lock<'a>(
    g: std::sync::PoisonError<std::sync::MutexGuard<'a, Vec<String>>>,
) -> std::sync::MutexGuard<'a, Vec<String>> {
    g.into_inner()
}

fn is_import_task(task: &str) -> bool {
    matches!(task, TASK_GITHUB | TASK_BILIBILI | TASK_ZHIHU)
}

fn task_label(task: &str) -> &'static str {
    match task {
        TASK_GITHUB => "GitHub 导入",
        TASK_BILIBILI => "B站导入",
        TASK_ZHIHU => "知乎导入",
        TASK_CLASSIFY => "AI 归类",
        TASK_CHECK => "失效检测",
        _ => "其他任务",
    }
}

/// 两个任务能否同时跑：只有「导入 × 导入」可以，其余一律互斥（含同名任务自身）。
fn tasks_conflict(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    !(is_import_task(a) && is_import_task(b))
}

/// 任务开工：与在跑的任务互斥时直接报错，界面上按钮也是灰的，双保险。
fn begin_task(task: &str) -> Result<(), String> {
    let mut g = RUNNING.lock().unwrap_or_else(tasks_lock);
    if let Some(other) = g.iter().find(|t| tasks_conflict(task, t)) {
        return Err(format!(
            "「{}」正在运行，请等它结束（或点「停止」）后再开始",
            task_label(other)
        ));
    }
    g.push(task.to_string());
    Ok(())
}

/// 任务收工（成功失败都要调，否则后面的任务会被永久挡住）。
fn end_task(task: &str) {
    let mut g = RUNNING.lock().unwrap_or_else(tasks_lock);
    if let Some(pos) = g.iter().position(|t| t == task) {
        g.remove(pos);
    }
}

/// 清掉本任务的停止标记：上一次点过停止，不该影响这一次。
fn reset_cancel(task: &str) {
    let mut g = CANCEL.lock().unwrap_or_else(tasks_lock);
    g.retain(|t| t != task);
}

/// 用户是否点了本任务的「停止」。
fn is_cancelled(task: &str) -> bool {
    CANCEL.lock().unwrap_or_else(tasks_lock).iter().any(|t| t == task)
}

/// 请求停止：`task` 为 None 时停止所有任务（前端的兜底用法）。
fn request_cancel(task: Option<&str>) {
    let mut g = CANCEL.lock().unwrap_or_else(tasks_lock);
    let targets: Vec<&str> = match task {
        Some(t) if !t.trim().is_empty() => vec![t],
        _ => ALL_TASKS.to_vec(),
    };
    for t in targets {
        if !g.iter().any(|x| x == t) {
            g.push(t.to_string());
        }
    }
}

/// 单次导入最多翻多少页：star 上千时防止一次跑太久（每页 100 条）。
const MAX_PAGES: usize = 100;

/// 导入 / 归类的实时进度事件（前端 `favorites-progress`）。
///
/// 字段全部可选、按阶段取用：导入阶段给抓取/新增计数与当前收藏夹，
/// 归类阶段给已归类/剩余/标签数。`done = true` 表示整趟结束（汇总值）。
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct FavoritesProgress {
    /// import | classify | check
    stage: &'static str,
    /// 任务名（github / bilibili / zhihu / classify / check）：
    /// 多个导入会同时发进度，前端按它分行展示，否则后一个会盖掉前一个。
    task: String,
    source: Option<String>,
    /// 当前正在处理的收藏夹 / 阶段说明
    folder: Option<String>,
    message: Option<String>,
    fetched: Option<usize>,
    added: Option<usize>,
    updated: Option<usize>,
    skipped: Option<usize>,
    classified: Option<usize>,
    tags_written: Option<usize>,
    remaining: Option<usize>,
    /// 知乎专用：当前收藏夹已抓取条数 / 服务端报告的总数（Paging.Totals）
    folder_fetched: Option<usize>,
    folder_total: Option<i64>,
    /// 失效检测专用：已探测条数 / 本轮待探测总数
    checked: Option<usize>,
    check_total: Option<usize>,
    done: bool,
}

fn emit_progress(app: &tauri::AppHandle, progress: &FavoritesProgress) {
    use tauri::Emitter;
    // `task` 是前端分行的键：漏填（默认空串）时事件会被前端按任务名过滤掉，
    // 表现为「后端在跑、界面一行进度都没有」——不会报错，只能靠日志看出来。
    // 这里留一条兜底告警；真正的把关是 `every_progress_literal_sets_task` 那条测试。
    if progress.task.is_empty() {
        eprintln!(
            "[收藏] 进度事件缺 task（前端会丢弃它）: stage={}, source={:?}",
            progress.stage, progress.source
        );
    }
    let _ = app.emit("favorites-progress", progress);
}

/// 汇总一条已累计的导入计数。
fn import_progress(
    stage: &'static str,
    source: &str,
    folder: Option<String>,
    message: Option<String>,
    result: &ImportResult,
    done: bool,
) -> FavoritesProgress {
    FavoritesProgress {
        stage,
        // 导入阶段任务名就是来源名（github / bilibili / zhihu）
        task: source.to_string(),
        source: Some(source.to_string()),
        folder,
        message,
        fetched: Some(result.fetched),
        added: Some(result.added),
        updated: Some(result.updated),
        skipped: Some(result.skipped),
        done,
        ..FavoritesProgress::default()
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub login: String,
    /// 本次抓到的条数
    pub fetched: usize,
    pub added: usize,
    pub updated: usize,
    pub skipped: usize,
    /// 命中删除墓碑而跳过的条数：**用户之前删过、平台上还在**，这次没有捞回来
    #[serde(default)]
    pub skipped_deleted: usize,
    pub cancelled: bool,
    /// 读取失败的收藏夹（`标题（原因）`）；其余收藏夹照常导入
    #[serde(default)]
    pub failed: Vec<String>,
}

/// 收藏模块**自己的** GitHub Token。
///
/// 刻意与 SDK 模块的 `github_token`（及 `GITHUB_TOKEN` 环境变量）**互不共享**：
/// 收藏导入需要的是「读你 star 的权限」，和 SDK 刷版本列表是两件事，
/// 用户可能只想给其中一个配 token。因此这里只认收藏模块自己存的凭证。
fn favorites_github_token() -> Result<String, String> {
    db::with_conn(|conn| db::get_credential(conn, github::SOURCE))?.ok_or_else(|| {
        "未配置 GitHub Token：请在收藏模块点「GitHub Token」按钮配置（与 SDK 模块的 Token 相互独立）"
            .to_string()
    })
}

/// 导入 GitHub star（只读）。
///
/// 幂等：同一批数据第二次导入 `added = 0`，全部落到 `skipped`（或内容有变时 `updated`）。
#[tauri::command]
pub async fn fav_import_github(
    app: tauri::AppHandle,
    max_pages: Option<usize>,
) -> Result<ImportResult, String> {
    begin_task(TASK_GITHUB)?;
    let out = import_github_inner(app, max_pages).await;
    end_task(TASK_GITHUB);
    out
}

async fn import_github_inner(
    app: tauri::AppHandle,
    max_pages: Option<usize>,
) -> Result<ImportResult, String> {
    let token = favorites_github_token()?;

    reset_cancel(TASK_GITHUB);
    let login = github::fetch_user_login(&token).await?;

    let mut url = github::starred_url(&login);
    let limit = max_pages.unwrap_or(MAX_PAGES).max(1);
    let mut result = ImportResult {
        login: login.clone(),
        ..ImportResult::default()
    };

    let mut page_no = 0usize;
    for _ in 0..limit {
        if is_cancelled(TASK_GITHUB) {
            result.cancelled = true;
            break;
        }
        page_no += 1;
        let (body, next) = github::fetch_starred_page(&token, &url).await?;
        let repos = body
            .as_array()
            .ok_or_else(|| "GitHub 返回的 star 列表格式异常".to_string())?;
        // star 列表元素是 `{ starred_at, repo }`：由 starred_item_to_favorite 取出收藏时间
        let items: Vec<NewFavorite> = repos
            .iter()
            .filter_map(github::starred_item_to_favorite)
            .collect();
        result.fetched += items.len();

        // 整页在一个连接里写完：既能少取锁，也让这一页的写入是原子的
        let delta = db::with_conn(|conn| {
            let mut added = 0usize;
            let mut updated = 0usize;
            let mut skipped = 0usize;
            let mut skipped_deleted = 0usize;
            for item in &items {
                match db::upsert(conn, item)? {
                    db::UpsertOutcome::Added => added += 1,
                    db::UpsertOutcome::Updated => updated += 1,
                    db::UpsertOutcome::Skipped => skipped += 1,
                    // 用户删过：平台还在，但这次不要捞回来
                    db::UpsertOutcome::Deleted => skipped_deleted += 1,
                }
            }
            Ok((added, updated, skipped, skipped_deleted))
        })?;
        result.added += delta.0;
        result.updated += delta.1;
        result.skipped += delta.2;
        result.skipped_deleted += delta.3;

        emit_progress(
            &app,
            &import_progress(
                "import",
                github::SOURCE,
                Some(format!("第 {} 页", page_no)),
                None,
                &result,
                false,
            ),
        );

        match next {
            Some(next_url) => url = next_url,
            None => break,
        }
    }

    db::with_conn(|conn| db::mark_imported(conn, github::SOURCE, result.fetched))?;
    crate::exit_log!(
        "[收藏] GitHub 导入完成: login={}, fetched={}, added={}, updated={}, skipped={}, cancelled={}",
        result.login,
        result.fetched,
        result.added,
        result.updated,
        result.skipped,
        result.cancelled
    );
    emit_progress(
        &app,
        &import_progress("import", github::SOURCE, None, None, &result, true),
    );
    Ok(result)
}

/// 停止长任务（导入 / AI 归类 / 失效检测）。
///
/// 各任务在**下一个循环开始前**检查自己的停止标记，所以停止是「不再往下走」
/// 而不是中断进行中的请求。`task` 为空时停止全部（导入可以同时跑，通常按名字停其中一个）。
#[tauri::command]
pub fn fav_cancel(task: Option<String>) -> Result<(), String> {
    request_cancel(task.as_deref());
    Ok(())
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassifyResult {
    /// 本轮归类的条目数
    pub classified: usize,
    /// 新写出的「条目-标签」对数（多标签会大于条目数）
    pub tags_written: usize,
    pub model: String,
    /// 还剩多少条没归类（下一轮继续）
    pub remaining: usize,
    /// 用户中途点了停止（已归类的部分保留）
    pub cancelled: bool,
}

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
    begin_task(TASK_CLASSIFY)?;
    let out = classify_inner(app, provider_id, model_id, limit, reclassify.unwrap_or(false)).await;
    end_task(TASK_CLASSIFY);
    out
}

async fn classify_inner(
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
        let prompt = classify::build_prompt(&batch);
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
        let written = db::with_conn(|conn| db::apply_tags(conn, &batch, &groups, &model, reclassify))?;
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

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckResult {
    pub checked: usize,
    pub gone: usize,
    pub redirect: usize,
    pub unknown: usize,
    /// 因限流提前中断（没跑完，下次再点会继续）
    pub aborted: bool,
    /// 用户点了停止（已检测的条目状态已落库）
    pub cancelled: bool,
    /// 库里有 GitHub 条目但没配 Token → 这批没查（其它来源照查）
    pub skipped_no_token: usize,
}

/// 手动检测失效（覆盖所有来源）。
///
/// GitHub 走官方接口（能认出改名/转让）；浏览器书签、B站、知乎等走 HTTP 探测
/// （404/410 = 失效，403/429/5xx/超时 = 未知，**不会**把活着的页面误标成失效）。
/// `all = false` 时只探测没查过的条目；`all = true` 全量重测。
/// GitHub 撞到限流（403/429）只停 GitHub 那一段并如实上报。
#[tauri::command]
pub async fn fav_check_gone(
    app: tauri::AppHandle,
    all: Option<bool>,
) -> Result<CheckResult, String> {
    begin_task(TASK_CHECK)?;
    let out = check_gone_inner(app, all).await;
    end_task(TASK_CHECK);
    out
}

/// 一次并发探测多少个 URL（太小慢、太大容易被目标站按 IP 限流）。
const PROBE_CONCURRENCY: usize = 8;

async fn check_gone_inner(app: tauri::AppHandle, all: Option<bool>) -> Result<CheckResult, String> {
    reset_cancel(TASK_CHECK);
    let all = all.unwrap_or(false);

    // GitHub 与其它来源分开：前者走 API（能认出改名），后者走 HTTP 探测。
    // 没配 Token 只影响 GitHub 那一段，**不能**让整轮检测直接失败
    // （以前是硬要求，等于浏览器书签这些永远查不了失效）。
    let token = db::with_conn(|conn| db::get_credential(conn, github::SOURCE)).ok().flatten();
    let sources = db::with_conn(|conn| db::all_sources(conn))?;
    let gh_source = github::SOURCE.to_string();
    let non_gh: Vec<String> = sources.iter().filter(|s| **s != gh_source).cloned().collect();

    let gh_items = if token.is_some() {
        db::with_conn(|conn| {
            db::select_for_check_multi(conn, &[gh_source.clone()], all, 5_000)
        })?
    } else {
        Vec::new()
    };
    // 没 Token 但库里有 GitHub 条目：如实计入「跳过」，不要假装检测过
    let skipped_no_token = if token.is_none() {
        db::with_conn(|conn| db::select_for_check_multi(conn, &[gh_source.clone()], all, 5_000))?.len()
    } else {
        0
    };
    let url_items = db::with_conn(|conn| {
        db::select_for_check_multi(conn, &non_gh, all, 5_000)
    })?;

    let total = gh_items.len() + url_items.len();
    let mut result = CheckResult {
        skipped_no_token,
        ..CheckResult::default()
    };
    // 探测很慢（串行几秒一条 / 并发也要数百毫秒一条）：先把进度条点亮，
    // 之后每查完一条更新一次。
    let mut emit = |result: &CheckResult, source: &str, message: Option<String>, done: bool| {
        emit_progress(
            &app,
            &FavoritesProgress {
                stage: "check",
                task: TASK_CHECK.to_string(),
                source: Some(source.to_string()),
                message,
                checked: Some(result.checked),
                check_total: Some(total),
                done,
                ..FavoritesProgress::default()
            },
        );
    };
    emit(&result, github::SOURCE, None, false);

    // ── 第一阶段：GitHub（官方接口，能识别改名） ──
    if let Some(token) = token.as_deref() {
        for (id, _source, full_name, _url) in gh_items {
            if is_cancelled(TASK_CHECK) {
                result.cancelled = true;
                break;
            }
            let (status, body) = github::fetch_repo(token, &full_name).await?;
            if status == 403 || status == 429 {
                // 限流：GitHub 这段停下（后面别的来源不受影响）
                result.aborted = true;
                break;
            }
            let verdict = check::classify_repo_response(status, body.as_ref(), &full_name);
            apply_verdict(id, &verdict, &mut result)?;
            emit(&result, github::SOURCE, Some(full_name), false);
        }
    }

    // ── 第二阶段：其余来源走 HTTP 探测（并发，逐块落库） ──
    if !url_items.is_empty() && !result.aborted {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| format!("构建 HTTP 客户端失败: {e}"))?;
        for chunk in url_items.chunks(PROBE_CONCURRENCY) {
            if is_cancelled(TASK_CHECK) {
                result.cancelled = true;
                break;
            }
            let probes = futures_util::future::join_all(
                chunk.iter().map(|(_, _, _, url)| check::probe_url(&client, url)),
            )
            .await;
            for ((id, source, title, _url), verdict) in chunk.iter().zip(probes) {
                apply_verdict(*id, &verdict, &mut result)?;
                emit(&result, source, Some(title.clone()), false);
            }
        }
    }

    crate::exit_log!(
        "[收藏] 失效检测完成: checked={}, gone={}, redirect={}, unknown={}, aborted={}, skipped_no_token={}",
        result.checked,
        result.gone,
        result.redirect,
        result.unknown,
        result.aborted,
        result.skipped_no_token
    );
    // done 事件带上最终计数，前端据此收尾（aborted 时总数还是原值，进度条会停在中途，
    // 这是刻意的：让用户看到「没跑完」而不是假装 100%）
    emit(&result, github::SOURCE, None, true);
    Ok(result)
}

/// 把一条探测结论落库并累计计数（GitHub / HTTP 两条路共用）。
fn apply_verdict(
    id: i64,
    verdict: &check::GoneStatus,
    result: &mut CheckResult,
) -> Result<(), String> {
    match verdict {
        check::GoneStatus::Ok => {
            db::with_conn(|conn| db::apply_status(conn, id, "ok", None))?;
        }
        check::GoneStatus::Gone => {
            db::with_conn(|conn| db::apply_status(conn, id, "gone", None))?;
            result.gone += 1;
        }
        check::GoneStatus::Redirect(new_url) => {
            db::with_conn(|conn| db::apply_status(conn, id, "redirect", Some(new_url)))?;
            result.redirect += 1;
        }
        check::GoneStatus::Unknown => {
            db::with_conn(|conn| db::apply_status(conn, id, "unknown", None))?;
            result.unknown += 1;
        }
    }
    result.checked += 1;
    Ok(())
}

/// 导入 B站收藏（只读，需要 Cookie）。
///
/// 逐个收藏夹分页拉取：`created/list-all` → 每个 `resource/list`。
/// 同样幂等：第二次导入 added = 0。
#[tauri::command]
pub async fn fav_import_bilibili(app: tauri::AppHandle) -> Result<ImportResult, String> {
    begin_task(TASK_BILIBILI)?;
    let out = import_bilibili_inner(app).await;
    end_task(TASK_BILIBILI);
    out
}

async fn import_bilibili_inner(app: tauri::AppHandle) -> Result<ImportResult, String> {
    let cookie = db::with_conn(|conn| db::get_credential(conn, bilibili::SOURCE))?
        .ok_or_else(|| "未配置 B站 Cookie：请在收藏模块里粘贴登录后的 Cookie（含 SESSDATA）".to_string())?;

    reset_cancel(TASK_BILIBILI);
    let session = bilibili::fetch_session(&cookie).await?;
    if session.mid.is_none() {
        return Err("Cookie 未生效（B站没返回登录用户），请重新粘贴 Cookie".to_string());
    }
    let folders = bilibili::fetch_folders(&cookie, &session).await?;
    if folders.is_empty() {
        return Ok(ImportResult::default());
    }

    let mut result = ImportResult::default();
    'outer: for (folder_id, folder_title) in &folders {
        for page in 1..=bilibili::MAX_PAGES {
            if is_cancelled(TASK_BILIBILI) {
                result.cancelled = true;
                break 'outer;
            }
            let (medias, has_more) =
                bilibili::fetch_folder_page(&cookie, &session, *folder_id, page).await?;
            if medias.is_empty() {
                break;
            }
            let items: Vec<NewFavorite> = medias
                .iter()
                .filter_map(|media| bilibili::media_to_favorite(media, folder_title))
                .collect();
            result.fetched += items.len();
            let delta = db::with_conn(|conn| {
                let mut added = 0usize;
                let mut updated = 0usize;
                let mut skipped = 0usize;
                let mut skipped_deleted = 0usize;
                for item in &items {
                    match db::upsert(conn, item)? {
                        db::UpsertOutcome::Added => added += 1,
                        db::UpsertOutcome::Updated => updated += 1,
                        db::UpsertOutcome::Skipped => skipped += 1,
                        db::UpsertOutcome::Deleted => skipped_deleted += 1,
                    }
                }
                Ok((added, updated, skipped, skipped_deleted))
            })?;
            result.added += delta.0;
            result.updated += delta.1;
            result.skipped += delta.2;
            result.skipped_deleted += delta.3;
            emit_progress(
                &app,
                &import_progress(
                    "import",
                    bilibili::SOURCE,
                    Some(folder_title.clone()),
                    Some(format!("第 {} 页", page)),
                    &result,
                    false,
                ),
            );
            if !has_more {
                break;
            }
            // 每页之间留间隔：B站对**收藏夹这类账号态接口**的风控比公开接口紧得多，
            // 连发几百个请求的下场是 Cookie 直接失效（甚至封号），
            // 所以这个延迟是账号安全措施，不是性能参数，不要为了「快一点」调小。
            tokio::time::sleep(std::time::Duration::from_millis(bilibili::PAGE_DELAY_MS)).await;
        }
        // 收藏夹之间也歇一下：连续切换收藏夹时请求同样密集
        tokio::time::sleep(std::time::Duration::from_millis(bilibili::PAGE_DELAY_MS)).await;
    }

    db::with_conn(|conn| db::mark_imported(conn, bilibili::SOURCE, result.fetched))?;
    crate::exit_log!(
        "[收藏] B站导入完成: folders={}, fetched={}, added={}, updated={}, skipped={}, cancelled={}",
        folders.len(),
        result.fetched,
        result.added,
        result.updated,
        result.skipped,
        result.cancelled
    );
    emit_progress(
        &app,
        &import_progress("import", bilibili::SOURCE, None, None, &result, true),
    );
    Ok(result)
}

/// 读知乎 Cookie（需含 z_c0 登录态与 d_c0）。
fn zhihu_cookie() -> Result<String, String> {
    db::with_conn(|conn| db::get_credential(conn, zhihu::COOKIE_KEY))?.ok_or_else(|| {
        "未配置知乎 Cookie：点收藏模块的烧瓶按钮，粘贴浏览器里复制的整条 Cookie（需含 z_c0 与 d_c0）"
            .to_string()
    })
}

/// 单趟导入最多翻多少页（内容接口每页 [`zhihu::PAGE_SIZE`] 条）。
///
/// 2032 条的收藏夹约 102 页，给到 2000 页足够用；上限的意义是防「服务端一直
/// 返回 is_end=false」时把我们拖进死循环。
const ZHIHU_MAX_PAGES: usize = 2000;

/// 导入知乎收藏（Cookie 直连，**私有收藏夹也能拿到**）。
///
/// 单条收藏夹内容接口一次给 20 条，2032 条的收藏夹要 102 次请求，
/// 所以间隔 [`zhihu::PAGE_DELAY_MS`] 是账号安全措施，不是性能参数。
#[tauri::command]
pub async fn fav_import_zhihu(app: tauri::AppHandle) -> Result<ImportResult, String> {
    begin_task(TASK_ZHIHU)?;
    let out = import_zhihu_inner(app).await;
    end_task(TASK_ZHIHU);
    out
}

async fn import_zhihu_inner(app: tauri::AppHandle) -> Result<ImportResult, String> {
    let cookie = zhihu_cookie()?;
    reset_cancel(TASK_ZHIHU);

    // ① 拿用户 id：收藏夹列表要按 id 查，url_token 是主页地址那套
    let (status, body) = zhihu::cookie_get_path(&cookie, "/api/v4/me").await?;
    if !(200..300).contains(&status) {
        // 401/403 基本都是 Cookie 过期——顺手把状态落库，UI 才能主动提醒
        if status == 401 || status == 403 {
            let _ = db::with_conn(|conn| db::mark_credential_status(conn, zhihu::COOKIE_KEY, "expired"));
        }
        return Err(format!(
            "知乎登录态校验失败 (HTTP {})：Cookie 可能已失效，请重新复制整条 Cookie。响应: {}",
            status,
            crate::commands::utils::truncate_utf8(&body, 200)
        ));
    }
    let _ = db::with_conn(|conn| db::mark_credential_status(conn, zhihu::COOKIE_KEY, "ok"));
    let person_id = zhihu::parse_person_id(&body)
        .ok_or_else(|| "知乎 /me 未返回用户 id，无法列出收藏夹".to_string())?;
    let login = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(|s| s.to_string()))
        .unwrap_or_else(|| person_id.clone());

    let mut result = ImportResult {
        login,
        ..ImportResult::default()
    };

    // 先点亮进度条：第一页要等网络 + 400ms 间隔，不先发一条用户会以为没点动
    emit_progress(
        &app,
        &import_progress(
            "import",
            zhihu::SOURCE,
            Some("正在获取收藏夹列表".to_string()),
            None,
            &result,
            false,
        ),
    );

    // ② 收藏夹列表（分页）
    let mut collections: Vec<zhihu::CookieCollection> = Vec::new();
    let mut offset = 0usize;
    let mut pages = 0usize;
    loop {
        if is_cancelled(TASK_ZHIHU) {
            result.cancelled = true;
            break;
        }
        pages += 1;
        if pages > ZHIHU_MAX_PAGES {
            result
                .failed
                .push("收藏夹列表翻页超过上限，已停止".to_string());
            break;
        }
        let (status, body) =
            zhihu::cookie_get_path(&cookie, &zhihu::collections_path(&person_id, offset)).await?;
        if status == 403 || status == 429 {
            return Err(format!(
                "知乎限流 (HTTP {})：停在第 {} 个收藏夹前，稍后再试（已导入的部分保留）",
                status,
                collections.len()
            ));
        }
        if !(200..300).contains(&status) {
            return Err(format!(
                "拉取收藏夹列表失败 (HTTP {})：响应 {}",
                status,
                crate::commands::utils::truncate_utf8(&body, 200)
            ));
        }
        let (page, is_end) = zhihu::parse_collections_page(&body)?;
        let got = page.len();
        collections.extend(page);
        offset += got;
        if is_end || got == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(zhihu::PAGE_DELAY_MS)).await;
    }

    crate::exit_log!(
        "[收藏] 知乎(Cookie) 共 {} 个收藏夹，开始导入内容",
        collections.len()
    );
    // 收藏夹数量确定后立刻发一条：进度条的分母（folderTotal）由此而来
    emit_progress(
        &app,
        &import_progress(
            "import",
            zhihu::SOURCE,
            Some(format!("共 {} 个收藏夹", collections.len())),
            None,
            &result,
            false,
        ),
    );

    // ③ 逐个收藏夹抓内容
    'folders: for collection in &collections {
        let mut offset = 0usize;
        let mut pages = 0usize;
        loop {
            if is_cancelled(TASK_ZHIHU) {
                result.cancelled = true;
                break 'folders;
            }
            pages += 1;
            if pages > ZHIHU_MAX_PAGES {
                result.failed.push(format!("「{}」翻页超过上限，已停止", collection.title));
                break;
            }
            let path = zhihu::cookie_items_path(collection.id, offset);
            let (status, body) = zhihu::cookie_get_path(&cookie, &path).await?;
            if status == 403 || status == 429 {
                result.failed.push(format!(
                    "「{}」被限流 (HTTP {})，已导入的部分保留",
                    collection.title, status
                ));
                break;
            }
            if !(200..300).contains(&status) {
                result.failed.push(format!(
                    "「{}」拉取失败 (HTTP {})：{}",
                    collection.title,
                    status,
                    crate::commands::utils::truncate_utf8(&body, 120)
                ));
                break;
            }
            let (items, is_end) = zhihu::parse_cookie_items_page(&body, collection)?;
            let got = items.len();
            for item in &items {
                // upsert + 顺手缓存正文：内容接口已经把全文给我们了，不存白不存
                let (outcome, id) = db::with_conn(|conn| db::upsert_with_id(conn, &item.favorite))?;
                match outcome {
                    db::UpsertOutcome::Added => result.added += 1,
                    db::UpsertOutcome::Updated => result.updated += 1,
                    db::UpsertOutcome::Skipped => result.skipped += 1,
                    db::UpsertOutcome::Deleted => result.skipped_deleted += 1,
                }
                // 命中墓碑：条目没入库，id 为 0，别去缓存正文
                if outcome == db::UpsertOutcome::Deleted {
                    continue;
                }
                if !item.text.is_empty() || item.html.is_some() {
                    db::with_conn(|conn| {
                        db::put_content(
                            conn,
                            id,
                            &item.text,
                            item.html.as_deref(),
                            Some(&collection.title),
                        )
                    })?;
                }
            }
            result.fetched += got;
            offset += got;

            emit_progress(
                &app,
                &FavoritesProgress {
                    stage: "import",
                    // `task` 必须填：前端按它分行展示进度，漏填会让这条事件被丢掉
                    // （知乎导入之前就是这样：后端在发、界面什么都不显示）
                    task: TASK_ZHIHU.to_string(),
                    source: Some(zhihu::SOURCE.to_string()),
                    folder: Some(collection.title.clone()),
                    message: Some(format!("已抓取 {} 条", result.fetched)),
                    fetched: Some(result.fetched),
                    added: Some(result.added),
                    updated: Some(result.updated),
                    skipped: Some(result.skipped),
                    folder_fetched: Some(offset),
                    folder_total: Some(collection.item_count as i64),
                    done: false,
                    ..FavoritesProgress::default()
                },
            );

            if is_end || got == 0 {
                break;
            }
            // 每页之间歇一下：这条路线依赖用户 Cookie，跑太猛会被风控甚至封号
            tokio::time::sleep(std::time::Duration::from_millis(zhihu::PAGE_DELAY_MS)).await;
        }
    }

    db::with_conn(|conn| db::mark_imported(conn, zhihu::SOURCE, result.fetched))?;
    crate::exit_log!(
        "[收藏] 知乎(Cookie) 导入完成: 收藏夹={}, 抓取={}, 新增={}, 更新={}, 无变化={}, 取消={}",
        collections.len(),
        result.fetched,
        result.added,
        result.updated,
        result.skipped,
        result.cancelled
    );
    emit_progress(
        &app,
        &import_progress("import", zhihu::SOURCE, None, None, &result, true),
    );
    Ok(result)
}


/// 读各平台凭证的健康状态（用于「Cookie 快过期 / 已失效」提示）。
///
/// 前端在模块打开时调一次：纯本地查询、不联网，所以不会给平台添任何请求。
#[tauri::command]
pub fn fav_credential_status() -> Result<Vec<db::CredentialStatus>, String> {
    db::with_conn(|conn| {
        ["github", "bilibili", zhihu::COOKIE_KEY]
            .iter()
            .map(|source| db::credential_status(conn, source))
            .collect()
    })
}

/// 读条目正文缓存（知乎收藏的内容，或已抓过的 GitHub README）。
///
/// 返回 `None` = 没缓存过，由前端决定要不要去抓（GitHub README 是懒抓的）。
#[tauri::command]
pub fn fav_get_content(id: i64) -> Result<Option<db::CachedContent>, String> {
    db::with_conn(|conn| db::get_content(conn, id))
}

/// 抓某个 GitHub 条目的 README（**优先 `README_ZH.md`**）并缓存。
///
/// 顺序：`README_ZH.md` → GitHub 自己认的默认 README。
/// 先找中文版是因为不少中文项目把中文说明单独放一个文件，默认 README 反而是英文简介。
/// 命中后按文件名写进 `favorite_content.label`，界面上能看出读的是哪一份。
#[tauri::command]
pub async fn fav_github_readme(
    id: i64,
    refresh: Option<bool>,
) -> Result<db::CachedContent, String> {
    if refresh != Some(true) {
        if let Some(cached) = db::with_conn(|conn| db::get_content(conn, id))? {
            return Ok(cached);
        }
    }

    let (source, full_name, _) = db::with_conn(|conn| db::find_by_id(conn, id))?
        .ok_or_else(|| format!("条目 {} 不存在", id))?;
    if source != github::SOURCE {
        return Err("只有 GitHub 条目支持查看 README".to_string());
    }
    let token =
        db::with_conn(|conn| db::get_credential(conn, github::SOURCE))?.unwrap_or_default();

    for name in github::README_ZH_CANDIDATES {
        if let Some(text) = github::fetch_file_text(&token, &full_name, name).await? {
            return cache_readme(id, &text, Some(name));
        }
    }
    match github::fetch_default_readme(&token, &full_name).await? {
        Some((name, text)) => cache_readme(id, &text, Some(&name)),
        None => Err(format!("{} 没有 README", full_name)),
    }
}

/// 写入 README 缓存并回读（回读是为了带上统一的 `fetchedAt`）。
fn cache_readme(id: i64, text: &str, label: Option<&str>) -> Result<db::CachedContent, String> {
    db::with_conn(|conn| db::put_content(conn, id, text, None, label))?;
    db::with_conn(|conn| db::get_content(conn, id))?
        .ok_or_else(|| "缓存 README 失败".to_string())
}

/// 读取收藏模块自己的 GitHub Token（未配置返回空串；**不读 SDK 模块的配置**）。
#[tauri::command]
pub fn fav_get_github_token() -> Result<String, String> {
    Ok(db::with_conn(|conn| db::get_credential(conn, github::SOURCE))?.unwrap_or_default())
}

/// 保存 / 清除收藏模块自己的 GitHub Token（传空串即清除）。
#[tauri::command]
pub fn fav_set_github_token(token: String) -> Result<(), String> {
    db::with_conn(|conn| db::set_credential(conn, github::SOURCE, &token))
}

/// 设置某平台的 Cookie（B站：含 SESSDATA 的完整 Cookie 串；传空串清除）。
#[tauri::command]
pub fn fav_set_credential(source: String, cookie: String) -> Result<(), String> {
    db::with_conn(|conn| db::set_credential(conn, &source, &cookie))
}

/// 某平台是否已配置 Cookie（**不返回内容**，只回 true/false）。
#[tauri::command]
pub fn fav_has_credential(source: String) -> Result<bool, String> {
    db::with_conn(|conn| db::get_credential(conn, &source).map(|c| c.is_some()))
}

/// 读取某平台已保存的凭证原文（未配置返回空串）。
///
/// 仅供配置弹窗回显（GitHub Token 弹窗一直有这个能力）；只在本机 UI 里展示。
#[tauri::command]
pub fn fav_get_credential(source: String) -> Result<String, String> {
    Ok(db::with_conn(|conn| db::get_credential(conn, &source))?.unwrap_or_default())
}

/// 列出收藏条目。
///
/// `sort`：`favorited` 按收藏时间 / `created` 按入库时间 / 其它按最近更新。
/// `favorited_since`：只保留收藏时间不早于该时刻的条目（本地时间字符串，由前端按
/// 「今天 / 近 7 天 / 近 30 天 / 今年」这类预设算好再传，后端不猜时区）。
#[tauri::command]
pub fn fav_list(
    source: Option<String>,
    category_id: Option<i64>,
    status: Option<String>,
    keyword: Option<String>,
    sort: Option<String>,
    favorited_since: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<db::FavoriteRow>, String> {
    db::with_conn(|conn| {
        db::list(
            conn,
            &db::ListFilter {
                source,
                category_id,
                status,
                keyword,
                sort,
                favorited_since,
                limit: limit.unwrap_or(0),
            },
        )
    })
}

/// 导入浏览器收藏夹（Edge / Chrome）。
///
/// 从启动模块搬过来的能力，但**书签目录会建成多级分类树**（以前压平成一层的做法
/// 把用户整理好的目录结构丢了）。可以多设备/多 Profile 反复导入：靠
/// `(source, external_id)` 去重，第二次只是把条目挂到新出现的目录上。
#[tauri::command]
pub fn fav_import_bookmarks(
    browser: String,
    custom_path: Option<String>,
) -> Result<super::bookmarks::BookmarkImportResult, String> {
    super::bookmarks::import(&browser, custom_path.as_deref())
}

// ─── 分类树（多级，操作逻辑对齐启动模块） ───

/// 列出整棵分类树（含条目数）。
#[tauri::command]
pub fn fav_list_categories() -> Result<Vec<db::CategoryNode>, String> {
    db::with_conn(|conn| db::list_category_tree(conn))
}

/// 新建分类（顶层传 null）。
#[tauri::command]
pub fn fav_create_category(name: String, parent_id: Option<i64>) -> Result<i64, String> {
    db::with_conn(|conn| db::create_category(conn, &name, parent_id))
}

/// 重命名分类。
#[tauri::command]
pub fn fav_rename_category(id: i64, name: String) -> Result<(), String> {
    db::with_conn(|conn| db::rename_category(conn, id, &name))
}

/// 移动分类（换父级；顶层传 null）。自己 / 子孙作目标会被拒绝。
#[tauri::command]
pub fn fav_move_category(id: i64, parent_id: Option<i64>) -> Result<(), String> {
    db::with_conn(|conn| db::move_category(conn, id, parent_id))
}

/// 删除分类及其所有子分类（**不动条目本身**，只解开关联）。
#[tauri::command]
pub fn fav_delete_category(id: i64) -> Result<usize, String> {
    db::with_conn(|conn| db::delete_category(conn, id))
}

/// 同级排序：`[(id, sort_order)]` 批量写回。
#[tauri::command]
pub fn fav_reorder_categories(orders: Vec<(i64, i32)>) -> Result<(), String> {
    db::with_conn(|conn| db::reorder_categories(conn, &orders))
}

/// 全量替换某条目的分类（人工操作 → 同时锁定，AI 归类不再改动它）。
#[tauri::command]
pub fn fav_set_item_categories(id: i64, category_ids: Vec<i64>) -> Result<(), String> {
    db::with_conn(|conn| db::set_item_categories_manual(conn, id, &category_ids))
}

/// 人工设置标签（全量替换 + 锁定，后续 AI 归类不再改动）。
#[tauri::command]
pub fn fav_set_tags(id: i64, tags: Vec<String>) -> Result<(), String> {
    db::with_conn(|conn| db::set_tags(conn, id, &tags))
}

/// 删除本地条目（**只删本地**，不动平台）。
///
/// 同时立一条**删除墓碑**（`favorite_deleted`）：平台上的收藏还在，下次导入一定会再拉到它，
/// 没有碑就会被当成新条目重新插入（表现为「删了又回来」）。
#[tauri::command]
pub fn fav_delete(id: i64) -> Result<bool, String> {
    db::with_conn(|conn| db::delete(conn, id))
}

/// 列出删除墓碑（设置页展示「已删除 n 条」并允许重新纳入）。
#[tauri::command]
pub fn fav_list_deleted() -> Result<Vec<db::FavoriteDeletedRow>, String> {
    db::with_conn(|conn| db::list_deleted(conn))
}

/// 清除删除墓碑 → 这些条目下次导入会重新进来。
///
/// `source` + `externalId` 都给 = 只恢复一条；只给 source = 恢复该来源全部；都不给 = 全部恢复。
#[tauri::command]
pub fn fav_restore_deleted(source: Option<String>, external_id: Option<String>) -> Result<usize, String> {
    db::with_conn(|conn| db::clear_deleted(conn, source.as_deref(), external_id.as_deref()))
}

/// 概览计数。
#[tauri::command]
pub fn fav_stats() -> Result<db::FavoriteStats, String> {
    db::with_conn(|conn| db::stats(conn))
}

/// 供应商：必须是用户**显式选的**那个，不做任何兜底。
///
/// 以前缺省时会回落到「AI 模块的默认供应商」，结果是「以为跑在 A 上，其实花的是 B 的额度」。
fn pick_explicit_provider<'a>(
    providers: &'a [crate::commands::ai::models::AiProvider],
    provider_id: &Option<String>,
) -> Result<&'a crate::commands::ai::models::AiProvider, String> {
    let pid = provider_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "请先选择供应商（收藏模块的归类不使用 AI 模块的默认供应商）".to_string())?;
    providers
        .iter()
        .find(|p| p.id == pid)
        .ok_or_else(|| format!("未找到供应商: {pid}"))
}

/// 模型：同样必须显式选，**不**回落供应商的激活模型 / 首个模型。
fn pick_explicit_model(
    provider: &crate::commands::ai::models::AiProvider,
    model_id: &Option<String>,
) -> Result<String, String> {
    model_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            format!(
                "请先选择模型（供应商「{}」的激活模型/首个模型不会自动沿用）",
                provider.name
            )
        })
}

/// 解析「用哪个供应商的哪个模型」——两项都必须显式指定。
fn resolve_ai_target(
    cfg: &crate::commands::ai::models::AiConfig,
    provider_id: &Option<String>,
    model_id: &Option<String>,
) -> Result<(crate::commands::ai::models::AiProvider, String), String> {
    let provider = pick_explicit_provider(&cfg.providers, provider_id)?.clone();
    let model = pick_explicit_model(&provider, model_id)?;
    if provider.openai_url.is_empty() {
        return Err(format!("供应商「{}」未配置 OpenAI 兼容端点", provider.name));
    }
    if provider.api_key.is_empty() {
        return Err(format!("供应商「{}」未配置 API Key", provider.name));
    }
    Ok((provider, model))
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::{pick_explicit_model, pick_explicit_provider};

    /// 归类用的供应商与模型都必须由用户显式选定：不挑「第一个可用供应商」，
    /// 也不沿用供应商的激活模型/首个模型——否则用户看到的与实际调用到的会不一致。
    #[test]
    fn classify_target_must_be_explicit() {
        use crate::commands::ai::models::AiProvider;
        let provider: AiProvider = serde_json::from_value(serde_json::json!({
            "id": "p1",
            "name": "P1",
            "api_key": "sk-x",
            "openai_url": "https://api.example.com/v1",
            "models": [{ "id": "m1", "name": "M1" }],
            "active_model_id": "m1"
        }))
        .expect("provider 能反序列化");
        let providers = vec![provider];

        // 没选供应商 → 报错（旧实现会回落到「第一个可用供应商」）
        assert!(pick_explicit_provider(&providers, &None).is_err());
        assert!(pick_explicit_provider(&providers, &Some("  ".into())).is_err());
        // 选了不存在的供应商 → 报错
        assert!(pick_explicit_provider(&providers, &Some("nope".into())).is_err());

        let picked = pick_explicit_provider(&providers, &Some("p1".into())).unwrap();
        assert_eq!(picked.id, "p1");
        // 选了供应商但没选模型 → 报错（旧实现会沿用 active_model_id）
        assert!(pick_explicit_model(picked, &None).is_err());
        assert_eq!(pick_explicit_model(picked, &Some("m1".into())).unwrap(), "m1");
        assert_eq!(
            pick_explicit_model(picked, &Some(" m9 ".into())).unwrap(),
            "m9",
            "选了什么就发什么（模型列表可能更新过）"
        );
    }

    /// 所有 `FavoritesProgress { ... }` 构造点都必须**显式**写 `task`。
    ///
    /// `task` 有默认值（空串）：漏填既不会编译报错、也不会运行时报错，只会在前端
    /// 按任务名过滤进度时被静默丢弃——知乎导入就是这样「点了没有任何进度条」。
    /// 用源码扫描把它钉死：以后新增进度事件忘写 task，这条测试立刻红。
    #[test]
    fn every_progress_literal_sets_task() {
        const MARK: &str = "FavoritesProgress {";
        let src = include_str!("commands.rs");
        // 只扫生产代码：测试模块自己也会出现这个字符串，混进来就成了「自己证明自己」
        let scan = src.split("#[cfg(test)]").next().unwrap_or(src);

        let mut cursor = 0usize;
        let mut found = 0usize;
        while let Some(pos) = scan[cursor..].find(MARK) {
            let idx = cursor + pos;
            let line_start = scan[..idx].rfind('\n').map(|p| p + 1).unwrap_or(0);
            let head = &scan[line_start..idx];
            let after = idx + MARK.len();
            // 两类不是「构造点」的命中要跳过：
            // - 结构体定义（`struct FavoritesProgress {`）：字段列表里当然有 task
            // - 函数返回类型（`fn f() -> FavoritesProgress {`）：真正的构造点紧跟其后
            let is_decl = head.contains("struct") || head.trim_end().ends_with("->");
            if !is_decl {
                let end = scan[after..]
                    .find(MARK)
                    .map(|p| after + p)
                    .unwrap_or(scan.len());
                let block = &scan[after..end];
                assert!(
                    block.contains("task:"),
                    "进度事件漏了 task 字段（前端会静默丢弃它）: {}",
                    block.chars().take(240).collect::<String>()
                );
                found += 1;
            }
            cursor = after;
        }
        assert!(found >= 5, "没扫到进度事件构造点，测试本身失效了（只有 {found} 处）");
    }

    /// 并发规则：三个平台的导入可以同时跑，加工任务（归类 / 检测）与一切互斥。
    #[test]
    fn import_tasks_can_run_concurrently_but_block_processing() {
        assert!(!tasks_conflict(TASK_GITHUB, TASK_BILIBILI));
        assert!(!tasks_conflict(TASK_BILIBILI, TASK_ZHIHU));
        // 同一任务不能重入（界面已置灰，这里是兜底）
        assert!(tasks_conflict(TASK_GITHUB, TASK_GITHUB));
        // 导入 × 加工互斥（双向）
        assert!(tasks_conflict(TASK_GITHUB, TASK_CLASSIFY));
        assert!(tasks_conflict(TASK_CLASSIFY, TASK_GITHUB));
        assert!(tasks_conflict(TASK_ZHIHU, TASK_CHECK));
        // 加工任务之间也互斥
        assert!(tasks_conflict(TASK_CLASSIFY, TASK_CHECK));
        assert!(is_import_task(TASK_ZHIHU) && !is_import_task(TASK_CHECK));
    }

    /// 停止标记按任务分开：停掉 GitHub 不该顺手停掉正在跑的 B站导入，
    /// 别的任务开工也不能把本任务的停止请求清掉（旧实现是全局 bool，两个问题都有）。
    #[test]
    fn cancel_flags_are_per_task() {
        reset_cancel(TASK_GITHUB);
        reset_cancel(TASK_BILIBILI);
        request_cancel(Some(TASK_GITHUB));
        assert!(is_cancelled(TASK_GITHUB));
        assert!(!is_cancelled(TASK_BILIBILI), "停一个导入不能牵连另一个");

        reset_cancel(TASK_BILIBILI);
        assert!(
            is_cancelled(TASK_GITHUB),
            "别的任务开工不该抹掉本任务的停止请求"
        );

        // 不带任务名 = 全停
        request_cancel(None);
        assert!(is_cancelled(TASK_GITHUB) && is_cancelled(TASK_BILIBILI) && is_cancelled(TASK_ZHIHU));
        for t in ALL_TASKS {
            reset_cancel(t);
        }
        assert!(!is_cancelled(TASK_GITHUB));
    }

    /// 互斥登记：开工被拒不留残留，收工后同名任务可以再开工。
    #[test]
    fn begin_end_task_guards_conflicts() {
        for t in ALL_TASKS {
            end_task(t);
        }
        assert!(begin_task(TASK_GITHUB).is_ok());
        assert!(begin_task(TASK_BILIBILI).is_ok(), "导入之间不互斥");
        assert!(begin_task(TASK_CLASSIFY).is_err(), "有导入在跑时不能归类");
        end_task(TASK_GITHUB);
        end_task(TASK_BILIBILI);
        assert!(begin_task(TASK_CLASSIFY).is_ok(), "导入结束后归类可开工");
        end_task(TASK_CLASSIFY);
    }
}
