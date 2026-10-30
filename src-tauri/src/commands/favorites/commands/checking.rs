use super::*;

/// 手动检测失效（覆盖所有来源）。
///
/// GitHub 走官方接口（能认出改名/转让）；浏览器书签、B站、知乎等走 HTTP 探测
/// （404/410 = 失效，403/429/5xx/超时 = 未知，**不会**把活着的页面误标成失效）。
/// `all = false` 时只探测没查过的条目；`all = true` 全量重测。
/// GitHub 撞到限流（403/429）只停 GitHub 那一段并如实上报。
#[tauri::command]
pub async fn fav_check_gone(
    app: tauri::AppHandle,
    all: Option<bool>,
) -> Result<CheckResult, String> {
    begin_task(TASK_CHECK)?;
    let out = check_gone_inner(app, all).await;
    end_task(TASK_CHECK);
    out
}

pub(super) async fn check_gone_inner(
    app: tauri::AppHandle,
    all: Option<bool>,
) -> Result<CheckResult, String> {
    reset_cancel(TASK_CHECK);
    let all = all.unwrap_or(false);

    // GitHub 与其它来源分开：前者走 API（能认出改名），后者走 HTTP 探测。
    // 没配 Token 只影响 GitHub 那一段，**不能**让整轮检测直接失败
    // （以前是硬要求，等于浏览器书签这些永远查不了失效）。
    let token = db::with_conn(|conn| db::get_credential(conn, github::SOURCE))
        .ok()
        .flatten();
    let sources = db::with_conn(|conn| db::all_sources(conn))?;
    let gh_source = github::SOURCE.to_string();
    let non_gh: Vec<String> = sources
        .iter()
        .filter(|s| **s != gh_source)
        .cloned()
        .collect();

    let gh_items = if token.is_some() {
        db::with_conn(|conn| db::select_for_check_multi(conn, &[gh_source.clone()], all, 5_000))?
    } else {
        Vec::new()
    };
    // 没 Token 但库里有 GitHub 条目：如实计入「跳过」，不要假装检测过
    let skipped_no_token = if token.is_none() {
        db::with_conn(|conn| db::select_for_check_multi(conn, &[gh_source.clone()], all, 5_000))?
            .len()
    } else {
        0
    };
    let url_items = db::with_conn(|conn| db::select_for_check_multi(conn, &non_gh, all, 5_000))?;

    let total = gh_items.len() + url_items.len();
    let mut result = CheckResult {
        skipped_no_token,
        ..CheckResult::default()
    };
    // 探测很慢（串行几秒一条 / 并发也要数百毫秒一条）：先把进度条点亮，
    // 之后每查完一条更新一次。
    let mut emit = |result: &CheckResult, source: &str, message: Option<String>, done: bool| {
        emit_progress(
            &app,
            &FavoritesProgress {
                stage: "check",
                task: TASK_CHECK.to_string(),
                source: Some(source.to_string()),
                message,
                checked: Some(result.checked),
                check_total: Some(total),
                done,
                ..FavoritesProgress::default()
            },
        );
    };
    emit(&result, github::SOURCE, None, false);

    // ── 第一阶段：GitHub（官方接口，能识别改名） ──
    if let Some(token) = token.as_deref() {
        for (id, _source, full_name, _url) in gh_items {
            if is_cancelled(TASK_CHECK) {
                result.cancelled = true;
                break;
            }
            let (status, body) = github::fetch_repo(token, &full_name).await?;
            if status == 403 || status == 429 {
                // 限流：GitHub 这段停下（后面别的来源不受影响）
                result.aborted = true;
                break;
            }
            let verdict = check::classify_repo_response(status, body.as_ref(), &full_name);
            apply_verdict(id, &verdict, &mut result)?;
            emit(&result, github::SOURCE, Some(full_name), false);
        }
    }

    // ── 第二阶段：其余来源走 HTTP 探测（并发，逐块落库） ──
    if !url_items.is_empty() && !result.aborted {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| format!("构建 HTTP 客户端失败: {e}"))?;
        for chunk in url_items.chunks(PROBE_CONCURRENCY) {
            if is_cancelled(TASK_CHECK) {
                result.cancelled = true;
                break;
            }
            let probes = futures_util::future::join_all(
                chunk
                    .iter()
                    .map(|(_, _, _, url)| check::probe_url(&client, url)),
            )
            .await;
            for ((id, source, title, _url), verdict) in chunk.iter().zip(probes) {
                apply_verdict(*id, &verdict, &mut result)?;
                emit(&result, source, Some(title.clone()), false);
            }
        }
    }

    crate::exit_log!(
        "[收藏] 失效检测完成: checked={}, gone={}, redirect={}, unknown={}, aborted={}, skipped_no_token={}",
        result.checked,
        result.gone,
        result.redirect,
        result.unknown,
        result.aborted,
        result.skipped_no_token
    );
    // done 事件带上最终计数，前端据此收尾（aborted 时总数还是原值，进度条会停在中途，
    // 这是刻意的：让用户看到「没跑完」而不是假装 100%）
    emit(&result, github::SOURCE, None, true);
    Ok(result)
}

/// 把一条探测结论落库并累计计数（GitHub / HTTP 两条路共用）。
pub(super) fn apply_verdict(
    id: i64,
    verdict: &check::GoneStatus,
    result: &mut CheckResult,
) -> Result<(), String> {
    match verdict {
        check::GoneStatus::Ok => {
            db::with_conn(|conn| db::apply_status(conn, id, "ok", None))?;
        }
        check::GoneStatus::Gone => {
            db::with_conn(|conn| db::apply_status(conn, id, "gone", None))?;
            result.gone += 1;
        }
        check::GoneStatus::Redirect(new_url) => {
            db::with_conn(|conn| db::apply_status(conn, id, "redirect", Some(new_url)))?;
            result.redirect += 1;
        }
        check::GoneStatus::Unknown => {
            db::with_conn(|conn| db::apply_status(conn, id, "unknown", None))?;
            result.unknown += 1;
        }
    }
    result.checked += 1;
    Ok(())
}
