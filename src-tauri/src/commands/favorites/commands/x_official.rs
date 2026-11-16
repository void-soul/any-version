use super::*;

/// 给前端的授权信息（**不包含** access / refresh token）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XAccount {
    pub screen_name: String,
    pub name: String,
    pub client_id: String,
    pub expires_at: i64,
}

fn account_of(token: &x_oauth::XToken) -> XAccount {
    XAccount {
        screen_name: token.screen_name.clone(),
        name: token.name.clone(),
        client_id: token.client_id.clone(),
        expires_at: token.expires_at,
    }
}

/// 走一次 OAuth 2.0 PKCE 授权：打开系统浏览器 → 本地回调 → 保存 token。
#[tauri::command]
pub async fn fav_x_oauth_start(
    app: tauri::AppHandle,
    client_id: String,
) -> Result<XAccount, String> {
    let token = x_oauth::start_authorization(&app, &client_id).await?;
    Ok(account_of(&token))
}

/// 直接用手上已有的 Access Token / Refresh Token 落库，跳过浏览器授权。
///
/// 保存前会调 `/2/users/me` 验证 token 有效；`refreshToken` 可留空，
/// 但那样 2 小时后就得重新粘贴（刷新必须带 Client ID）。
#[tauri::command]
pub async fn fav_x_oauth_save_token(
    client_id: String,
    access_token: String,
    refresh_token: Option<String>,
) -> Result<XAccount, String> {
    let token =
        x_oauth::save_manual_token(&client_id, &access_token, refresh_token.as_deref()).await?;
    Ok(account_of(&token))
}

/// 读当前授权状态（**不返回** token，只给 UI 显示是否已授权与 @handle）。
#[tauri::command]
pub fn fav_x_oauth_status() -> Result<Option<XAccount>, String> {
    Ok(x_oauth::load_token()?.as_ref().map(account_of))
}

/// 导入 X 书签（官方 API，必须已授权）。
///
/// 增量：每页 upsert 后若**整页没有新增**，说明已经追上上次导入的位置，立即停止；
/// 这样第二次导入通常只花 1 次请求。页数上限来自官方的 800 条窗口。
#[tauri::command]
pub async fn fav_import_x_official(app: tauri::AppHandle) -> Result<ImportResult, String> {
    crate::runtime_tasks::run_if_enabled("favorites", async move {
        let _task = TaskGuard::begin(TASK_X)?;
        import_x_official_inner(app).await
    })
    .await
}

async fn fetch_page(
    user_id: &str,
    access_token: &str,
    cursor: Option<&str>,
    fields_param: &'static str,
    fields_value: &'static str,
) -> Result<(u16, String), String> {
    let params = x_api::page_params_with(cursor, fields_param, fields_value);
    let resp = crate::commands::utils::get_http_client()
        .get(x_api::bookmarks_url(user_id))
        .query(&params)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| format!("请求 X 官方书签接口失败: {e}"))?;
    let status = resp.status().as_u16();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("读取 X 响应体失败: {e}"))?;
    Ok((status, body))
}

pub(super) async fn import_x_official_inner(app: tauri::AppHandle) -> Result<ImportResult, String> {
    let token = x_oauth::ensure_access_token().await?;
    reset_cancel(TASK_X);

    let mut cursor: Option<String> = None;
    let mut result = ImportResult {
        login: if token.screen_name.is_empty() {
            token.name.clone()
        } else {
            format!("@{}", token.screen_name)
        },
        ..ImportResult::default()
    };

    for page_no in 1..=x_api::MAX_PAGES {
        if is_cancelled(TASK_X) {
            result.cancelled = true;
            break;
        }

        // 新版文档用 post.fields；老服务端只认 tweet.fields，400 时回退一次
        let (mut status, mut body) = fetch_page(
            &token.user_id,
            &token.access_token,
            cursor.as_deref(),
            "post.fields",
            x_api::POST_FIELDS,
        )
        .await?;
        if status == 400 {
            (status, body) = fetch_page(
                &token.user_id,
                &token.access_token,
                cursor.as_deref(),
                "tweet.fields",
                x_api::LEGACY_POST_FIELDS,
            )
            .await?;
        }

        if !(200..300).contains(&status) {
            if status == 401 || status == 403 {
                let _ = db::with_conn(|conn| {
                    db::mark_credential_status(conn, x_oauth::OAUTH_KEY, "expired")
                });
            }
            let hint = match status {
                401 | 403 => "：授权可能已失效，请在「X 官方授权」里重新授权",
                429 => "：达到官方限流，请稍后再试",
                _ => "",
            };
            return Err(format!(
                "X 官方书签接口失败 (HTTP {status}){hint}；响应: {}",
                crate::commands::utils::truncate_utf8(&body, 200)
            ));
        }
        let _ = db::with_conn(|conn| db::mark_credential_status(conn, x_oauth::OAUTH_KEY, "ok"));

        let value: serde_json::Value =
            serde_json::from_str(&body).map_err(|e| format!("X 返回的不是合法 JSON: {e}"))?;
        let page = x_api::parse_page(&value)?;
        result.fetched += page.items.len();

        // 整页在一个连接里写完：条目与正文缓存要么都成，要么都不落
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
                if outcome == db::UpsertOutcome::Deleted {
                    continue;
                }
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
                Some(if page_no == 1 {
                    "官方 API（增量）".to_string()
                } else {
                    format!("已抓取 {} 条", result.fetched)
                }),
                &result,
                false,
            ),
        );

        // 增量命中：这一页没有任何新增，后面都是上次已导入的旧数据
        if x_api::should_stop(delta.0, page.items.len()) {
            break;
        }
        match page.next_token {
            Some(next) if !next.is_empty() => cursor = Some(next),
            _ => break,
        }

        // 官方按 15 分钟窗口限流：页间留间隔，别把额度一次打满
        tokio::time::sleep(std::time::Duration::from_millis(x_api::PAGE_DELAY_MS)).await;
    }

    db::with_conn(|conn| db::mark_imported(conn, x::SOURCE, result.fetched))?;
    crate::exit_log!(
        "[收藏] X 官方导入完成: fetched={}, added={}, updated={}, skipped={}, cancelled={}",
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

/// 读某来源的上次导入时间与累计条数（给 UI 显示「上次读取」）。
#[tauri::command]
pub fn fav_import_state(source: String) -> Result<Option<db::ImportState>, String> {
    db::with_conn(|conn| db::get_import_state(conn, &source))
}
