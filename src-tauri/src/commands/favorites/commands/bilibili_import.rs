use super::*;

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

pub(super) async fn import_bilibili_inner(app: tauri::AppHandle) -> Result<ImportResult, String> {
    let cookie =
        db::with_conn(|conn| db::get_credential(conn, bilibili::SOURCE))?.ok_or_else(|| {
            "未配置 B站 Cookie：请在收藏模块里粘贴登录后的 Cookie（含 SESSDATA）".to_string()
        })?;

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
