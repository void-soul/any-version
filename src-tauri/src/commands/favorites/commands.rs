//! 收藏模块的 Tauri 命令。
//!
//! 全部**只读**：不调用任何平台的写入接口。

use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use serde_json::Value;

use super::bilibili;
use super::check;
use super::zhihu;
use super::classify::{self, BATCH_SIZE};
use super::db::{self, NewFavorite};
use super::github;

/// 用户点「取消导入」后，当前分页循环会在下一页开始前退出。
static CANCEL: AtomicBool = AtomicBool::new(false);

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
    let token = favorites_github_token()?;

    CANCEL.store(false, Ordering::SeqCst);
    let login = github::fetch_user_login(&token).await?;

    let mut url = github::starred_url(&login);
    let limit = max_pages.unwrap_or(MAX_PAGES).max(1);
    let mut result = ImportResult {
        login: login.clone(),
        ..ImportResult::default()
    };

    let mut page_no = 0usize;
    for _ in 0..limit {
        if CANCEL.load(Ordering::SeqCst) {
            result.cancelled = true;
            break;
        }
        page_no += 1;
        let (body, next) = github::fetch_starred_page(&token, &url).await?;
        let repos = body
            .as_array()
            .ok_or_else(|| "GitHub 返回的 star 列表格式异常".to_string())?;
        let items: Vec<NewFavorite> = repos.iter().filter_map(github::repo_to_favorite).collect();
        result.fetched += items.len();

        // 整页在一个连接里写完：既能少取锁，也让这一页的写入是原子的
        let delta = db::with_conn(|conn| {
            let mut added = 0usize;
            let mut updated = 0usize;
            let mut skipped = 0usize;
            for item in &items {
                match db::upsert(conn, item)? {
                    db::UpsertOutcome::Added => added += 1,
                    db::UpsertOutcome::Updated => updated += 1,
                    db::UpsertOutcome::Skipped => skipped += 1,
                }
            }
            Ok((added, updated, skipped))
        })?;
        result.added += delta.0;
        result.updated += delta.1;
        result.skipped += delta.2;

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

/// 停止当前长任务（导入 / AI 归类 / 失效检测）。
///
/// 三类任务共用这一个标志：同一时刻只可能有一个在跑（界面上的按钮互斥），
/// 各自在**下一个循环开始前**检查它，所以停止是「不再往下走」而不是中断进行中的请求。
/// 每个任务的入口都会先把它清掉，不会出现「上次取消了、这次一开就停」。
#[tauri::command]
pub fn fav_cancel() -> Result<(), String> {
    CANCEL.store(true, Ordering::SeqCst);
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

/// 用 AI 给**未归类**的条目打分类标签（多标签）。
///
/// - 只对「没有标签且未被人工锁定」的条目跑，反复点击不会浪费已归类的部分；
/// - 每批 [`BATCH_SIZE`] 条，一批失败就整批放弃并报错（不写半截错误分类）；
/// - `limit` 给一个上限，方便先试 40 条看效果再决定要不要全量跑。
#[tauri::command]
pub async fn fav_classify(
    app: tauri::AppHandle,
    provider_id: Option<String>,
    model_id: Option<String>,
    limit: Option<usize>,
) -> Result<ClassifyResult, String> {
    let cfg = crate::commands::ai::config::load_ai_config();
    let (provider, model) = resolve_ai_target(&cfg, &provider_id, &model_id)?;

    CANCEL.store(false, Ordering::SeqCst);
    let budget = limit.unwrap_or(usize::MAX);
    let mut result = ClassifyResult {
        model: model.clone(),
        ..ClassifyResult::default()
    };
    // 总盘子 = 开始时未归类的条数；进度条用它算百分比
    let total_pending =
        db::with_conn(|conn| db::select_unclassified(conn, 1_000_000))?.len();
    let mut batch_no = 0usize;

    // 先发一条 0/N：每批要等模型返回（几十秒都可能），不先点亮进度条用户会以为没反应
    emit_progress(
        &app,
        &FavoritesProgress {
            stage: "classify",
            source: None,
            message: Some(format!("每批 {} 条", BATCH_SIZE)),
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
        if CANCEL.load(Ordering::SeqCst) {
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
        )
        .await
        .map_err(|e| format!("{}（供应商: {}，模型: {}）", e, provider.name, model))?;

        // 解析失败 → 直接返回错误：整批不落库，避免写进半截错误分类
        let groups = classify::parse_groups(&outcome.text, batch.len())?;
        let written = db::with_conn(|conn| db::apply_tags(conn, &batch, &groups, &model))?;
        result.tags_written += written;
        result.classified += batch.len();

        emit_progress(
            &app,
            &FavoritesProgress {
                stage: "classify",
                message: Some(format!("第 {} 批（每批 {} 条）", batch_no, BATCH_SIZE)),
                classified: Some(result.classified),
                tags_written: Some(result.tags_written),
                remaining: Some(total_pending.saturating_sub(result.classified)),
                done: false,
                ..FavoritesProgress::default()
            },
        );

        if let Some(usage) = &outcome.usage {
            crate::commands::ai::usage::log_usage_from_json(
                "favorites",
                &model,
                Some(&provider.id),
                usage,
            );
        }
    }

    result.remaining = db::with_conn(|conn| db::select_unclassified(conn, 1_000_000))?.len();
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
}

/// 手动检测失效（目前只覆盖 GitHub）。
///
/// `all = false` 时只探测没查过的条目；`all = true` 全量重测。
/// 撞到限流（403/429）就**停下并如实上报**，而不是把活着的收藏误标成失效。
#[tauri::command]
pub async fn fav_check_gone(
    app: tauri::AppHandle,
    all: Option<bool>,
) -> Result<CheckResult, String> {
    // 与导入用同一个（收藏模块自己的）Token：同一份权限，不该出现「导入能用、检测不能用」
    let token = favorites_github_token()?;
    CANCEL.store(false, Ordering::SeqCst);
    let items = db::with_conn(|conn| db::select_for_check(conn, github::SOURCE, all.unwrap_or(false), 5_000))?;

    let total = items.len();
    let mut result = CheckResult::default();
    // 探测是**逐条串行**的，几千条会跑很久：没有进度用户会以为卡死。
    // 先发一条 0/N 把进度条点亮，之后每查完一条更新一次。
    emit_progress(
        &app,
        &FavoritesProgress {
            stage: "check",
            source: Some(github::SOURCE.to_string()),
            checked: Some(0),
            check_total: Some(total),
            done: false,
            ..FavoritesProgress::default()
        },
    );

    for (id, full_name, _url) in items {
        if CANCEL.load(Ordering::SeqCst) {
            result.cancelled = true;
            break;
        }
        let (status, body) = github::fetch_repo(&token, &full_name).await?;
        if status == 403 || status == 429 {
            result.aborted = true;
            break;
        }
        match check::classify_repo_response(status, body.as_ref(), &full_name) {
            check::GoneStatus::Ok => {
                db::with_conn(|conn| db::apply_status(conn, id, "ok", None))?;
            }
            check::GoneStatus::Gone => {
                db::with_conn(|conn| db::apply_status(conn, id, "gone", None))?;
                result.gone += 1;
            }
            check::GoneStatus::Redirect(new_url) => {
                db::with_conn(|conn| db::apply_status(conn, id, "redirect", Some(&new_url)))?;
                result.redirect += 1;
            }
            check::GoneStatus::Unknown => {
                db::with_conn(|conn| db::apply_status(conn, id, "unknown", None))?;
                result.unknown += 1;
            }
        }
        result.checked += 1;
        emit_progress(
            &app,
            &FavoritesProgress {
                stage: "check",
                source: Some(github::SOURCE.to_string()),
                message: Some(full_name),
                checked: Some(result.checked),
                check_total: Some(total),
                done: false,
                ..FavoritesProgress::default()
            },
        );
    }

    crate::exit_log!(
        "[收藏] 失效检测完成: checked={}, gone={}, redirect={}, unknown={}, aborted={}",
        result.checked,
        result.gone,
        result.redirect,
        result.unknown,
        result.aborted
    );
    // done 事件带上最终计数，前端据此收尾（aborted 时总数还是原值，进度条会停在中途，
    // 这是刻意的：让用户看到「没跑完」而不是假装 100%）
    emit_progress(
        &app,
        &FavoritesProgress {
            stage: "check",
            source: Some(github::SOURCE.to_string()),
            checked: Some(result.checked),
            check_total: Some(total),
            done: true,
            ..FavoritesProgress::default()
        },
    );
    Ok(result)
}

/// 导入 B站收藏（只读，需要 Cookie）。
///
/// 逐个收藏夹分页拉取：`created/list-all` → 每个 `resource/list`。
/// 同样幂等：第二次导入 added = 0。
#[tauri::command]
pub async fn fav_import_bilibili(app: tauri::AppHandle) -> Result<ImportResult, String> {
    let cookie = db::with_conn(|conn| db::get_credential(conn, bilibili::SOURCE))?
        .ok_or_else(|| "未配置 B站 Cookie：请在收藏模块里粘贴登录后的 Cookie（含 SESSDATA）".to_string())?;

    CANCEL.store(false, Ordering::SeqCst);
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
            if CANCEL.load(Ordering::SeqCst) {
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
                for item in &items {
                    match db::upsert(conn, item)? {
                        db::UpsertOutcome::Added => added += 1,
                        db::UpsertOutcome::Updated => updated += 1,
                        db::UpsertOutcome::Skipped => skipped += 1,
                    }
                }
                Ok((added, updated, skipped))
            })?;
            result.added += delta.0;
            result.updated += delta.1;
            result.skipped += delta.2;
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
    let cookie = zhihu_cookie()?;
    CANCEL.store(false, Ordering::SeqCst);

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

    // ② 收藏夹列表（分页）
    let mut collections: Vec<zhihu::CookieCollection> = Vec::new();
    let mut offset = 0usize;
    let mut pages = 0usize;
    loop {
        if CANCEL.load(Ordering::SeqCst) {
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

    // ③ 逐个收藏夹抓内容
    'folders: for collection in &collections {
        let mut offset = 0usize;
        let mut pages = 0usize;
        loop {
            if CANCEL.load(Ordering::SeqCst) {
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
#[tauri::command]
pub fn fav_list(
    source: Option<String>,
    tag: Option<String>,
    status: Option<String>,
    keyword: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<db::FavoriteRow>, String> {
    db::with_conn(|conn| {
        db::list(
            conn,
            &db::ListFilter {
                source,
                tag,
                status,
                keyword,
                limit: limit.unwrap_or(0),
            },
        )
    })
}

/// 人工设置标签（全量替换 + 锁定，后续 AI 归类不再改动）。
#[tauri::command]
pub fn fav_set_tags(id: i64, tags: Vec<String>) -> Result<(), String> {
    db::with_conn(|conn| db::set_tags(conn, id, &tags))
}

/// 删除本地条目（**只删本地**，不动平台）。
#[tauri::command]
pub fn fav_delete(id: i64) -> Result<bool, String> {
    db::with_conn(|conn| db::delete(conn, id))
}

/// 概览计数。
#[tauri::command]
pub fn fav_stats() -> Result<db::FavoriteStats, String> {
    db::with_conn(|conn| db::stats(conn))
}

/// 解析「用哪个供应商的哪个模型」：显式指定优先，否则沿用 AI 模块的默认供应商。
///
/// 与翻译共用同一套回退链（默认供应商 → 第一个可用供应商），避免两个模块各写一份。
fn resolve_ai_target(
    cfg: &crate::commands::ai::models::AiConfig,
    provider_id: &Option<String>,
    model_id: &Option<String>,
) -> Result<(crate::commands::ai::models::AiProvider, String), String> {
    let provider = match provider_id {
        Some(pid) => cfg
            .providers
            .iter()
            .find(|p| &p.id == pid)
            .cloned()
            .ok_or_else(|| format!("未找到供应商: {}", pid))?,
        None => cfg
            .providers
            .iter()
            .find(|p| !p.api_key.is_empty() && !p.openai_url.is_empty())
            .cloned()
            .ok_or_else(|| "没有配置了 OpenAI 端点和 API Key 的供应商".to_string())?,
    };
    if provider.openai_url.is_empty() {
        return Err(format!("供应商「{}」未配置 OpenAI 兼容端点", provider.name));
    }
    if provider.api_key.is_empty() {
        return Err(format!("供应商「{}」未配置 API Key", provider.name));
    }
    let model = model_id
        .clone()
        .or_else(|| provider.active_model_id.clone())
        .or_else(|| provider.models.first().map(|m| m.id.clone()))
        .ok_or_else(|| format!("供应商「{}」未配置任何模型", provider.name))?;
    Ok((provider, model))
}
