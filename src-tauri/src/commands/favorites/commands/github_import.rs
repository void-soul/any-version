use super::*;

/// 收藏模块**自己的** GitHub Token。
///
/// 刻意与 SDK 模块的 `github_token`（及 `GITHUB_TOKEN` 环境变量）**互不共享**：
/// 收藏导入需要的是「读你 star 的权限」，和 SDK 刷版本列表是两件事，
/// 用户可能只想给其中一个配 token。因此这里只认收藏模块自己存的凭证。
pub(super) fn favorites_github_token() -> Result<String, String> {
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
    crate::runtime_tasks::run_if_enabled("favorites", async move {
        let _task = TaskGuard::begin(TASK_GITHUB)?;
        let out = import_github_inner(app, max_pages).await;
        out
    })
    .await
}

pub(super) async fn import_github_inner(
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
