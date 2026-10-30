use super::*;

/// 读知乎 Cookie（需含 z_c0 登录态与 d_c0）。
pub(super) fn zhihu_cookie() -> Result<String, String> {
    db::with_conn(|conn| db::get_credential(conn, zhihu::COOKIE_KEY))?.ok_or_else(|| {
        "未配置知乎 Cookie：点收藏模块的烧瓶按钮，粘贴浏览器里复制的整条 Cookie（需含 z_c0 与 d_c0）"
            .to_string()
    })
}

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

pub(super) async fn import_zhihu_inner(app: tauri::AppHandle) -> Result<ImportResult, String> {
    let cookie = zhihu_cookie()?;
    reset_cancel(TASK_ZHIHU);

    // ① 拿用户 id：收藏夹列表要按 id 查，url_token 是主页地址那套
    let (status, body) = zhihu::cookie_get_path(&cookie, "/api/v4/me").await?;
    if !(200..300).contains(&status) {
        // 401/403 基本都是 Cookie 过期——顺手把状态落库，UI 才能主动提醒
        if status == 401 || status == 403 {
            let _ = db::with_conn(|conn| {
                db::mark_credential_status(conn, zhihu::COOKIE_KEY, "expired")
            });
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
        .and_then(|v| {
            v.get("name")
                .and_then(|n| n.as_str())
                .map(|s| s.to_string())
        })
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
                result
                    .failed
                    .push(format!("「{}」翻页超过上限，已停止", collection.title));
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
