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
    /// import | classify
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

/// 取消正在进行的导入（下一页开始前生效）。
#[tauri::command]
pub fn fav_cancel_import() -> Result<(), String> {
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

    let budget = limit.unwrap_or(usize::MAX);
    let mut result = ClassifyResult {
        model: model.clone(),
        ..ClassifyResult::default()
    };
    // 总盘子 = 开始时未归类的条数；进度条用它算百分比
    let total_pending =
        db::with_conn(|conn| db::select_unclassified(conn, 1_000_000))?.len();
    let mut batch_no = 0usize;

    loop {
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
}

/// 手动检测失效（目前只覆盖 GitHub）。
///
/// `all = false` 时只探测没查过的条目；`all = true` 全量重测。
/// 撞到限流（403/429）就**停下并如实上报**，而不是把活着的收藏误标成失效。
#[tauri::command]
pub async fn fav_check_gone(all: Option<bool>) -> Result<CheckResult, String> {
    // 与导入用同一个（收藏模块自己的）Token：同一份权限，不该出现「导入能用、检测不能用」
    let token = favorites_github_token()?;
    let items = db::with_conn(|conn| db::select_for_check(conn, github::SOURCE, all.unwrap_or(false), 5_000))?;

    let mut result = CheckResult::default();
    for (id, full_name, _url) in items {
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
    }

    crate::exit_log!(
        "[收藏] 失效检测完成: checked={}, gone={}, redirect={}, unknown={}, aborted={}",
        result.checked,
        result.gone,
        result.redirect,
        result.unknown,
        result.aborted
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
        }
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

/// 【实验】用粘贴的 Cookie 直连知乎，依次探测 `/me`、`/collections`、`/collections/{id}/items`。
///
/// 返回 JSON 文本：带 `verdict`（ok / invalid_cookie / needs_signature / unknown）、
/// `conclusion` 与三个接口各自的 `status`/`body`。结果同时写入 exit.log。
#[tauri::command]
pub async fn fav_zhihu_probe() -> Result<String, String> {
    // 读的是**实验专用槽位** `zhihu-cookie`：官方接口的 Access Secret 存在 `zhihu`，
    // 两者互不覆盖——把 Cookie 存进 Secret 的槽位会把用户配好的凭证顶掉。
    let cookie =
        db::with_conn(|conn| db::get_credential(conn, zhihu::COOKIE_KEY))?.ok_or_else(|| {
            "请先点烧瓶图标粘贴知乎 Cookie（需含 z_c0 登录态与 d_c0；这与「知乎 Access Secret」是两个独立输入框）"
                .to_string()
        })?;
    let report = zhihu::probe_cookie(&cookie).await?;
    crate::exit_log!("[收藏-知乎] 实验结果: {}", report);
    Ok(report)
}

/// 知乎官方 API 的 GET（带鉴权头）。
async fn zhihu_get(access_secret: &str, path: &str) -> Result<Value, String> {
    let client = crate::commands::utils::get_http_client();
    let mut request = client.get(format!("{}{}", zhihu::BASE_URL, path));
    for (name, value) in zhihu::auth_headers(access_secret, zhihu_now_secs()) {
        request = request.header(name, value);
    }
    let resp = request
        .send()
        .await
        .map_err(|e| format!("请求知乎开放平台失败: {}", e))?;
    let status = resp.status().as_u16();
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析知乎响应失败 (HTTP {}): {}", status, e))?;
    zhihu::parse_envelope(&body)
}

fn zhihu_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 导入知乎收藏（只读，走开放平台官方接口）。
///
/// 逐个收藏夹按 `Paging.NextOffset` 翻页直到 `IsEnd`。
/// ⚠️ 用户数据接口按自然日配额（默认 100 次/天、未实名 10 次/天），每次翻页消耗一次。
/// 每个收藏夹的断点存在本地（`favorite_import_state.cursor`），跨天续传时
/// 直接从上次位置继续——配额全部花在新内容上，而不是重抓已导过的页。
#[tauri::command]
pub async fn fav_import_zhihu(app: tauri::AppHandle) -> Result<ImportResult, String> {
    let secret = db::with_conn(|conn| db::get_credential(conn, zhihu::SOURCE))?.ok_or_else(|| {
        "未配置知乎 Access Secret：点收藏模块的钥匙按钮，粘贴在 developer.zhihu.com/profile 生成的 Access Secret"
            .to_string()
    })?;

    CANCEL.store(false, Ordering::SeqCst);

    // 额度预检（官方文档：查询额度不消耗业务额度）。额度为 0 时直接明说，
    // 别让用户对着「第 N 页突然失败」的现场猜原因。
    let quota = match zhihu_get(&secret, &zhihu::quota_path()).await {
        Ok(payload) => zhihu::parse_quota(&payload),
        Err(_) => None, // 预检失败不阻塞导入，让真正的导入请求给出错误
    };
    if let Some((remaining, total)) = quota {
        crate::exit_log!("[收藏] 知乎用户数据额度: 剩余 {}/{}", remaining, total);
        if remaining == 0 {
            return Err(format!(
                "知乎今日额度已用完（剩余 0/{}）：按自然日配额，次日恢复；总额度见开放平台「各接口剩余配额」面板",
                total
            ));
        }
    }

    let favlists = zhihu_get(&secret, &zhihu::favlists_path()).await?;
    let folders = favlists
        .get("Items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut result = ImportResult::default();
    'outer: for folder in folders {
        let token = folder.get("UrlToken").and_then(|v| v.as_i64());
        let title = folder
            .get("Title")
            .and_then(|v| v.as_str())
            .unwrap_or("知乎收藏夹")
            .to_string();
        let Some(token) = token else {
            continue;
        };
        let state_key = format!("{}:{}", zhihu::SOURCE, token);

        // 断点续传：从上次翻到的页继续，而不是每次都从第 0 页烧额度
        let mut offset = db::with_conn(|conn| db::get_import_cursor(conn, &state_key))?
            .and_then(|cursor| cursor.parse::<usize>().ok())
            .unwrap_or(0);
        if offset > 0 {
            crate::exit_log!("[收藏] 知乎收藏夹「{}」从断点 {} 继续导入", title, offset);
        }
        // 服务端认为的收藏夹总数（公开范围口径）；进进度条，也能回答
        // 「是接口截断还是本来就这么多」——与 UI 显示的总数对不上时看它
        let mut folder_total: Option<i64> = None;
        let mut folder_fetched = 0usize;

        loop {
            if CANCEL.load(Ordering::SeqCst) {
                result.cancelled = true;
                // 取消也要存断点：下次从这页继续
                let _ = db::with_conn(|conn| {
                    db::set_import_cursor(conn, &state_key, Some(&offset.to_string()))
                });
                break 'outer;
            }
            // 单个收藏夹失败（比如私有收藏夹平台不允许读）不能拖垮整个导入：
            // 跳过它、记下原因，其余收藏夹照常导完。断点保留在失败前的位置，
            // 明天额度恢复后从同一位置重试。
            let payload = match zhihu_get(&secret, &zhihu::contents_path(token, offset)).await {
                Ok(payload) => payload,
                Err(e) => {
                    result.failed.push(format!("{}（{}）", title, e));
                    crate::exit_log!(
                        "[收藏-知乎] 收藏夹「{}」第 {} 页读取失败，已跳过: {}",
                        title,
                        offset,
                        e
                    );
                    continue 'outer;
                }
            };
            let (page_items, is_end, next_offset, totals) = zhihu::parse_contents_page(&payload);
            if totals.is_some() {
                folder_total = totals;
            }
            folder_fetched += page_items.len();
            crate::exit_log!(
                "[收藏-知乎] 「{}」 offset={} -> {} 条, is_end={}, next={:?}, totals={:?}",
                title,
                offset,
                page_items.len(),
                is_end,
                next_offset,
                folder_total
            );

            let favorites: Vec<NewFavorite> = page_items
                .iter()
                .filter_map(|item| zhihu::item_to_favorite(item, &title))
                .collect();
            result.fetched += favorites.len();
            let delta = db::with_conn(|conn| {
                let mut added = 0usize;
                let mut updated = 0usize;
                let mut skipped = 0usize;
                for item in &favorites {
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
                &FavoritesProgress {
                    folder_fetched: Some(folder_fetched),
                    folder_total,
                    ..import_progress(
                        "import",
                        zhihu::SOURCE,
                        Some(title.clone()),
                        Some(format!("offset {}", offset)),
                        &result,
                        false,
                    )
                },
            );

            if is_end || next_offset == usize::MAX {
                // 该收藏夹已导完：清断点，下次导入从头检查是否有新收藏
                let _ = db::with_conn(|conn| db::set_import_cursor(conn, &state_key, None));
                break;
            }
            offset = next_offset;
            let _ = db::with_conn(|conn| {
                db::set_import_cursor(conn, &state_key, Some(&offset.to_string()))
            });
            // 页与页之间留间隔：上次 1 秒内连发 9 个请求，触发风控（30003/412）是 177
            // 条停下来的头号嫌疑
            tokio::time::sleep(std::time::Duration::from_millis(zhihu::PAGE_DELAY_MS)).await;
        }
    }

    db::with_conn(|conn| db::mark_imported(conn, zhihu::SOURCE, result.fetched))?;
    crate::exit_log!(
        "[收藏] 知乎导入完成: fetched={}, added={}, updated={}, skipped={}, failed_folders={}, cancelled={}",
        result.fetched,
        result.added,
        result.updated,
        result.skipped,
        result.failed.len(),
        result.cancelled
    );
    emit_progress(
        &app,
        &import_progress("import", zhihu::SOURCE, None, None, &result, true),
    );
    Ok(result)
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
