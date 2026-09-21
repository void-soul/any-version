//! 收藏模块的 Tauri 命令。
//!
//! 全部**只读**：不调用任何平台的写入接口。

use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;

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
