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
pub async fn fav_import_github(max_pages: Option<usize>) -> Result<ImportResult, String> {
    let token = favorites_github_token()?;

    CANCEL.store(false, Ordering::SeqCst);
    let login = github::fetch_user_login(&token).await?;

    let mut url = github::starred_url(&login);
    let limit = max_pages.unwrap_or(MAX_PAGES).max(1);
    let mut result = ImportResult {
        login: login.clone(),
        ..ImportResult::default()
    };

    for _ in 0..limit {
        if CANCEL.load(Ordering::SeqCst) {
            result.cancelled = true;
            break;
        }
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

    loop {
        let done = result.classified;
        if done >= budget {
            break;
        }
        let batch = db::with_conn(|conn| db::select_unclassified(conn, BATCH_SIZE))?;
        if batch.is_empty() {
            break;
        }
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
pub async fn fav_import_bilibili() -> Result<ImportResult, String> {
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
    Ok(result)
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
/// ⚠️ 用户数据接口按自然日配额（默认 100 次/天、未实名 10 次/天），每次翻页消耗一次；
/// 超限时会收到明确的 30001/30002 报错，而不是空列表。
#[tauri::command]
pub async fn fav_import_zhihu() -> Result<ImportResult, String> {
    let secret = db::with_conn(|conn| db::get_credential(conn, zhihu::SOURCE))?.ok_or_else(|| {
        "未配置知乎 Access Secret：点收藏模块的钥匙按钮，粘贴在 developer.zhihu.com/profile 生成的 Access Secret"
            .to_string()
    })?;

    CANCEL.store(false, Ordering::SeqCst);
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

        let mut offset = 0usize;
        loop {
            if CANCEL.load(Ordering::SeqCst) {
                result.cancelled = true;
                break 'outer;
            }
            // 单个收藏夹失败（比如私有收藏夹平台不允许读）不能拖垮整个导入：
            // 跳过它、记下原因，其余收藏夹照常导完
            let payload = match zhihu_get(&secret, &zhihu::contents_path(token, offset)).await {
                Ok(payload) => payload,
                Err(e) => {
                    result.failed.push(format!("{}（{}）", title, e));
                    crate::exit_log!("[收藏] 知乎收藏夹读取失败，已跳过: {} ({})", title, e);
                    continue 'outer;
                }
            };
            let (items, is_end, next_offset) = zhihu::parse_contents_page(&payload);

            let favorites: Vec<NewFavorite> = items
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

            if is_end || next_offset == usize::MAX {
                break;
            }
            offset = next_offset;
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
