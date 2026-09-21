//! 收藏模块的 Tauri 命令。
//!
//! 全部**只读**：不调用任何平台的写入接口。

use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;

use super::check;
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
}

/// 导入 GitHub star（只读）。
///
/// 幂等：同一批数据第二次导入 `added = 0`，全部落到 `skipped`（或内容有变时 `updated`）。
#[tauri::command]
pub async fn fav_import_github(max_pages: Option<usize>) -> Result<ImportResult, String> {
    let token = crate::commands::utils::github_api_token()
        .ok_or_else(|| "未配置 GitHub Token：请在 SDK 模块设置 GitHub Token，或设置 GITHUB_TOKEN 环境变量".to_string())?;

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
    let token = crate::commands::utils::github_api_token()
        .ok_or_else(|| "未配置 GitHub Token：请在 SDK 模块设置 GitHub Token，或设置 GITHUB_TOKEN 环境变量".to_string())?;
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
