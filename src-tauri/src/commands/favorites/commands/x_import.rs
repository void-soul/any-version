use super::*;

/// 读 X Cookie（需含 `auth_token` 与 `ct0`）。
pub(super) fn x_cookie() -> Result<String, String> {
    db::with_conn(|conn| db::get_credential(conn, x::COOKIE_KEY))?.ok_or_else(|| {
        "未配置 X Cookie：在收藏模块的导入菜单里点「X Cookie」，\
         粘贴浏览器里复制的整条 Cookie（需含 auth_token 与 ct0）"
            .to_string()
    })
}

/// 解析这次要用的 queryId：用户覆盖优先，其次内置值。
pub(super) fn x_query_id() -> Result<String, String> {
    let stored = db::with_conn(|conn| db::get_credential(conn, x::QUERY_ID_KEY))?;
    stored
        .as_deref()
        .and_then(x::query_id_from_credential)
        .or_else(|| x::query_id_from_credential(x::DEFAULT_QUERY_ID))
        .ok_or_else(|| {
            "尚未配置 X 接口 ID：X 的 Web 接口会随前端发版更换地址，\
             请在「X 接口 ID」里粘贴一次请求地址 —— 登录 x.com 打开书签页，\
             开发者工具「网络」里筛 /i/api/graphql/，复制任意 Bookmarks 请求的完整 URL。"
                .to_string()
        })
}

/// 导入 X 书签（只读，需要 Cookie + 接口 ID）。
///
/// 幂等：同一批数据第二次导入 `added = 0`；分页按响应里的 bottom cursor 翻页。
#[tauri::command]
pub async fn fav_import_x(app: tauri::AppHandle) -> Result<ImportResult, String> {
    crate::runtime_tasks::run_if_enabled("favorites", async move {
        let _task = TaskGuard::begin(TASK_X)?;
        import_x_inner(app).await
    })
    .await
}

pub(super) async fn import_x_inner(app: tauri::AppHandle) -> Result<ImportResult, String> {
    let cookie = x_cookie()?;
    let (auth_token, ct0) = x::extract_auth(&cookie)?;
    let query_id = x_query_id()?;

    reset_cancel(TASK_X);
    let mut cursor = String::new();
    let mut result = ImportResult::default();
    let mut pages = 0usize;

    for page_no in 1..=x::MAX_PAGES {
        pages = page_no;
        if is_cancelled(TASK_X) {
            result.cancelled = true;
            break;
        }

        let (status, body) = x::fetch_bookmarks_page(&query_id, &auth_token, &ct0, &cursor).await?;

        // 401/403 基本都是 Cookie 过期：顺手落库状态，UI 才能主动提醒
        if status == 401 || status == 403 {
            let _ =
                db::with_conn(|conn| db::mark_credential_status(conn, x::COOKIE_KEY, "expired"));
        }
        if !(200..300).contains(&status) {
            return Err(format!(
                "{}；响应: {}",
                x::map_status_error(status),
                crate::commands::utils::truncate_utf8(&body, 200)
            ));
        }
        let _ = db::with_conn(|conn| db::mark_credential_status(conn, x::COOKIE_KEY, "ok"));

        let value: serde_json::Value =
            serde_json::from_str(&body).map_err(|e| format!("X 返回的不是合法 JSON: {e}"))?;
        let page = x::parse_timeline_page(&value)?;
        result.fetched += page.items.len();

        // 整页在一个连接里写完：这一页的条目与正文缓存要么都成，要么都不落
        let delta = db::with_conn(|conn| {
            let mut added = 0usize;
            let mut updated = 0usize;
            let mut skipped = 0usize;
            let mut skipped_deleted = 0usize;
            for item in &page.items {
                let (outcome, id) = db::upsert_with_id(conn, item)?;
                match outcome {
                    db::UpsertOutcome::Added => added += 1,
                    db::UpsertOutcome::Updated => updated += 1,
                    db::UpsertOutcome::Skipped => skipped += 1,
                    db::UpsertOutcome::Deleted => skipped_deleted += 1,
                }
                // 命中墓碑：条目没入库，id 为 0，别去缓存正文
                if outcome == db::UpsertOutcome::Deleted {
                    continue;
                }
                // 书签接口把全文一并给了：顺手缓存，AI 归类/检索就不用再撞登录墙
                if let Some(text) = item.description.as_deref() {
                    if !text.trim().is_empty() {
                        db::put_content(conn, id, text, None, Some("X"))?;
                    }
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
                x::SOURCE,
                Some(format!("第 {} 页", page_no)),
                Some(format!("已抓取 {} 条", result.fetched)),
                &result,
                false,
            ),
        );

        // 空页 / 没有游标 / 游标没前进 → 已经到底（缺这条会沿空页翻到页数上限）
        if x::should_stop_paging(&page, &cursor) {
            break;
        }
        cursor = page.next_cursor.clone().unwrap_or_default();

        // 每页之间歇一下：这条路线依赖用户 Cookie，跑太猛会被风控甚至封号，
        // 所以这个延迟是账号安全措施，不是性能参数，不要为了「快一点」调小。
        tokio::time::sleep(std::time::Duration::from_millis(x::PAGE_DELAY_MS)).await;
    }

    db::with_conn(|conn| db::mark_imported(conn, x::SOURCE, result.fetched))?;
    crate::exit_log!(
        "[收藏] X 书签导入完成: pages={}, fetched={}, added={}, updated={}, skipped={}, cancelled={}",
        pages,
        result.fetched,
        result.added,
        result.updated,
        result.skipped,
        result.cancelled
    );
    emit_progress(
        &app,
        &import_progress("import", x::SOURCE, None, None, &result, true),
    );
    Ok(result)
}
