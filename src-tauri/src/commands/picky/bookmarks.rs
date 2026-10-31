use super::*;

// ─── 命令：收藏 ───

/// 一次性加载全量状态（收藏/评论/标签/关联）。
#[tauri::command]
pub fn picky_get_state() -> Result<PickyState, String> {
    let conn = open_db()?;
    state_to_json(&conn)
}

/// 新增收藏。title 为空时用 url 兜底；可传入从网页抓取的元数据（imageUrl/faviconUrl）。
/// 添加后若有 URL，立即在后台走浏览器渲染链路自动补全元数据（JS 页面也能抓到），
/// 用户无需再手动点「刷新元数据」。
#[tauri::command]
pub fn picky_add_bookmark(
    app: tauri::AppHandle,
    title: Option<String>,
    url: Option<String>,
    description: Option<String>,
    image_url: Option<String>,
    favicon_url: Option<String>,
) -> Result<Bookmark, String> {
    let conn = open_db()?;
    let mut bm = Bookmark {
        title: title.unwrap_or_default().trim().to_string(),
        url: url.map(|u| u.trim().to_string()).filter(|u| !u.is_empty()),
        description: description
            .map(|d| d.trim().to_string())
            .filter(|d| !d.is_empty()),
        image_url: image_url
            .map(|u| u.trim().to_string())
            .filter(|u| !u.is_empty()),
        favicon_url: favicon_url
            .map(|u| u.trim().to_string())
            .filter(|u| !u.is_empty()),
        ..Default::default()
    };
    if bm.title.is_empty() {
        bm.title = bm.url.clone().unwrap_or_else(|| "未命名收藏".to_string());
    }
    let bookmark_id = bm.id.clone();
    insert_bookmark(&conn, &bm)?;
    drop(conn);
    schedule_auto_sync();
    // 后台自动补全元数据：只对「当前标题仍是裸 URL/空描述」的收藏生效（refetch 内部会覆盖）。
    // 用独立任务跑，添加操作本身立即返回，不阻塞 UI。
    if bm.url.is_some() {
        crate::runtime_tasks::spawn_task("picky", &owned_task_name("metadata"), async move {
            match picky_refetch_metadata_inner(&app, &bookmark_id).await {
                Ok(_) => {
                    let _ = app.emit("picky-bookmark-updated", &bookmark_id);
                }
                Err(_) => { /* 自动补全失败静默：用户仍可手动刷新 */ }
            }
        });
    }
    Ok(bm)
}

#[tauri::command]
pub fn picky_update_bookmark(bookmark: Bookmark) -> Result<(), String> {
    let conn = open_db()?;
    if !bookmark_exists(&conn, &bookmark.id) {
        return Err("收藏不存在".to_string());
    }
    let mut bm = bookmark;
    bm.updated_at = now_iso();
    insert_bookmark(&conn, &bm)?;
    schedule_auto_sync();
    Ok(())
}

/// 切换收藏状态（refined：false=收藏中，true=已归档）。
#[tauri::command]
pub fn picky_set_refined(id: String, refined: bool) -> Result<(), String> {
    let conn = open_db()?;
    conn.execute(
        "UPDATE picky_bookmarks SET refined=?1, updated_at=?2 WHERE id=?3",
        rusqlite::params![refined as i64, now_iso(), id],
    )
    .map_err(|e| format!("更新收藏状态失败: {}", e))?;
    schedule_auto_sync();
    Ok(())
}

/// 重新抓取单个收藏页面的完整元数据（标题、描述、图片、Favicon）。
///
/// 真实元数据抓取**必须走浏览器模拟**：很多页面（尤其是 JS 渲染的 SPA / 懒加载页）
/// 用普通 HTTP GET 拿到的 HTML 只有空壳，`<title>`/OG 标签是由脚本动态写入的。
/// 因此这里先调 Edge 无头渲染（`--dump-dom` + 虚拟时间快进），从渲染后的完整 DOM
/// 提取标题/描述/图片/favicon；仅当本机没有 Edge 时，才退回普通 HTTP 抓取兜底。
#[tauri::command]
pub async fn picky_refetch_metadata(app: tauri::AppHandle, id: String) -> Result<Bookmark, String> {
    picky_refetch_metadata_inner(&app, &id).await
}

/// 实际的元数据重抓实现（供命令与新增后的自动补全共用）。
pub(super) async fn picky_refetch_metadata_inner(
    app: &tauri::AppHandle,
    id: &str,
) -> Result<Bookmark, String> {
    crate::runtime_tasks::run_if_enabled("picky", async move {
        let conn = open_db()?;
        let bm = get_bookmark(&conn, id)?;
        let url = bm.url.clone().ok_or_else(|| "该收藏无 URL".to_string())?;
        crate::exit_log::exit_log(&format!("[picky-meta] 开始 refetch id={id} url={url}"));

        // 1) 应用内隐藏 WebView 抓取（首选：Tauri「iframe」方案，比外部无头进程稳）
        let mut rich = fetch_meta_via_webview(app, &url).await;
        let mut source = if rich.is_some() { "webview" } else { "" };

        // 2) WebView 失败 → 无头浏览器渲染（渲染后 DOM 含 JS 动态内容）
        if rich.is_none() {
            if let Some(dom) = render_dom_via_browser(&url).await {
                rich = Some(parse_rendered_meta(&dom, &url));
                source = "browser";
            }
        }

        // 3) 两者都没有 → 退回普通 HTTP 抓取的标题/favicon（仅做窄兜底）
        let http_fallback = if rich.is_none() {
            crate::commands::launcher::windows::fetch_url_metadata_with_timeout(
                &url,
                std::time::Duration::from_secs(10),
            )
            .await
            .ok()
        } else {
            None
        };

        let mut updated = bm.clone();
        match &rich {
            Some(r) => {
                if let Some(t) = &r.title {
                    updated.title = t.clone();
                }
                if let Some(d) = &r.description {
                    updated.description = Some(d.clone());
                }
                if let Some(img) = &r.image_url {
                    updated.image_url = Some(img.clone());
                }
                if let Some(f) = &r.favicon_url {
                    updated.favicon_url = Some(f.clone());
                }
                // 正文：仅在成功提取到非空正文时回填（不覆盖已有正文为空的情况——
                // 页面可能本次没渲染出正文，保留上次抓到的）
                if let Some(c) = &r.content {
                    if !c.trim().is_empty() {
                        updated.content = Some(c.clone());
                    }
                }
            }
            None => {
                if let Some(m) = &http_fallback {
                    // 防降级污染：HTTP 兜底在某些站点（如知乎）只能拿到裸 host（如 "www.zhihu.com"），
                    // 用它覆盖标题只会产生垃圾数据 —— host 形态（无空白、含点、纯 ASCII 可打印）或与
                    // URL 主机相同的「标题」一律拒绝。
                    let hostlike = |t: &str| {
                        !t.is_empty()
                            && !t.chars().any(char::is_whitespace)
                            && t.contains('.')
                            && t.chars().all(|c| c.is_ascii_graphic())
                    };
                    let url_host = url
                        .split_once("://")
                        .map(|(_, rest)| rest.split(['/', '?', ':']).next().unwrap_or(""))
                        .unwrap_or("");
                    let matches_host = !url_host.is_empty()
                        && m.title.trim_start_matches("www.")
                            == url_host.trim_start_matches("www.");
                    if m.title != url && !m.title.is_empty() && !hostlike(&m.title) && !matches_host
                    {
                        updated.title = m.title.clone();
                    }
                    if let Some(icon) = m.icon.clone() {
                        updated.favicon_url = Some(icon);
                    }
                }
            }
        }
        updated.meta_fetched = true;
        updated.updated_at = now_iso();
        insert_bookmark(&conn, &updated)?;
        drop(conn);
        schedule_auto_sync();
        crate::exit_log::exit_log(&format!(
            "[picky-meta] 完成 id={id} 来源={} title={:?} desc={}",
            if http_fallback.is_some() {
                "http"
            } else {
                source
            },
            updated.title,
            updated
                .description
                .as_deref()
                .map(|d| d.chars().take(60).collect::<String>())
                .unwrap_or_default(),
        ));
        Ok(updated)
    })
    .await
}

#[tauri::command]
pub fn picky_delete_bookmark(id: String) -> Result<(), String> {
    let conn = open_db()?;
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    // 先记墓碑再删：否则下一次云同步会把它们从云端（还没收到删除）原样补回来。
    let comment_ids: Vec<String> = tx
        .prepare("SELECT id FROM picky_comments WHERE bookmark_id=?1")
        .and_then(|mut s| {
            s.query_map([&id], |r| r.get(0))
                .and_then(|rows| rows.collect::<Result<Vec<String>, _>>())
        })
        .map_err(|e| format!("读取待删评论失败: {}", e))?;
    record_tombstones(&tx, ENT_COMMENT, &comment_ids)?;
    record_tombstones(&tx, ENT_BOOKMARK, &[id.clone()])?;
    tx.execute("DELETE FROM picky_bookmarks WHERE id=?1", [&id])
        .map_err(|e| format!("删除收藏失败: {}", e))?;
    tx.execute("DELETE FROM picky_comments WHERE bookmark_id=?1", [&id])
        .map_err(|e| e.to_string())?;
    tx.execute(
        "DELETE FROM picky_bookmark_tags WHERE bookmark_id=?1",
        [&id],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    schedule_auto_sync();
    Ok(())
}
