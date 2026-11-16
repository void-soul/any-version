//! X（x.com）书签导入：Cookie 认证 + 内部 GraphQL 只读接口。
//!
//! 与 B站 / 知乎同构：只用用户自己浏览器里的登录态读自己的书签，
//! **不调用任何写入接口**（不取消书签、不点赞、不发帖）。
//!
//! 路线说明：X 官方 API 需要开发者应用 + OAuth2 用户授权，且只返回最近 800 条；
//! 这里走 x.com Web 端自己用的 GraphQL 接口，能翻到全量。代价是 queryId 会随
//! 前端发版变化：内置值只作兜底，用户可在「X 接口 ID」里粘贴整段请求 URL 覆盖，
//! 失效时报错会直接说明怎么取新值。

use serde_json::{json, Value};

use super::db::NewFavorite;

/// 数据源标识（写入 `favorite.source`，**不要随意改**：已落库数据靠它分区）。
pub const SOURCE: &str = "x";

/// 凭证键：整条浏览器 Cookie（需含 `auth_token` 与 `ct0`）。
pub const COOKIE_KEY: &str = "x-cookie";

/// 覆盖 queryId 的凭证键（高级项，可选）。
pub const QUERY_ID_KEY: &str = "x-query-id";

/// Web 端公开 Bearer：所有浏览器共用同一份，不是用户凭证。
///
/// 与 queryId 一样会随前端发版更换：失效时从浏览器任意 x.com 请求的
/// `authorization` 头里复制一份替换即可。
pub const BEARER: &str = "AAAAAAAAAAAAAAAAAAAAANRILgAAAAAAnNwIzUejRCOuH5E6I8xnZz4puTs%3D1Zv7ttfk8LF81IUq16cHjhLTvJu4FA33AGWWjCpTnA";

/// 内置 queryId：留空表示「必须由用户提供」。
///
/// X 的 queryId 会随前端发版变化（不同来源的观测从数周到每周不等），
/// 写死一个过期值只会让用户看到 404 却不知道为什么。这里选择显式缺失：
/// 用户第一次导入时按提示粘贴一次请求 URL，之后一直复用。
pub const DEFAULT_QUERY_ID: &str = "";

/// 每页条数（X 的 Bookmarks 接口默认 20）。
pub const PAGE_SIZE: usize = 20;

/// 页间间隔：这条路线依赖用户 Cookie，跑太猛会被风控甚至封号，属账号安全参数。
pub const PAGE_DELAY_MS: u64 = 800;

/// 单趟最多翻多少页：防止服务端一直给 cursor 时把导入拖进死循环。
pub const MAX_PAGES: usize = 200;

/// 标题最多保留多少字符（X 推文没有标题，取正文首行）。
pub const TITLE_MAX_CHARS: usize = 80;

/// 从整条 Cookie 里取出 `auth_token` 与 `ct0`。
pub fn extract_auth(cookie: &str) -> Result<(String, String), String> {
    let auth = cookie_value(cookie, "auth_token");
    let ct0 = cookie_value(cookie, "ct0");
    match (auth, ct0) {
        (Some(auth), Some(ct0)) => Ok((auth, ct0)),
        (None, Some(_)) => Err(missing_cookie("auth_token")),
        (Some(_), None) => Err(missing_cookie("ct0")),
        (None, None) => Err(missing_cookie("auth_token / ct0")),
    }
}

fn missing_cookie(key: &str) -> String {
    format!(
        "X Cookie 缺少 {key}：在浏览器登录 x.com 后，从开发者工具「网络」面板\
         复制任意请求的完整 Cookie（需含 auth_token 与 ct0）。只保存在本机，用于读取你自己的书签。"
    )
}

/// 从 `a=b; c=d` 形式的 Cookie 串里取值（大小写不敏感，允许首尾引号与空格）。
fn cookie_value(cookie: &str, name: &str) -> Option<String> {
    cookie.split([';', '\n']).find_map(|segment| {
        let (key, value) = segment.trim().split_once('=')?;
        if !key.trim().eq_ignore_ascii_case(name) {
            return None;
        }
        let value = value.trim().trim_matches('"').trim();
        (!value.is_empty()).then(|| value.to_string())
    })
}

/// 解析用户填的「X 接口 ID」：允许直接粘贴整段请求 URL，也接受裸 ID。
pub fn query_id_from_credential(stored: &str) -> Option<String> {
    let first = stored
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    // 整段 URL：https://x.com/i/api/graphql/<queryId>/Bookmarks?variables=...
    if let Some(rest) = first.split("/graphql/").nth(1) {
        let id = rest.split(['/', '?']).next().unwrap_or("").trim();
        return (!id.is_empty()).then(|| id.to_string());
    }
    // 裸 ID（可能带引号或 `queryId:` 前缀）
    let id = first
        .trim_matches(|c: char| c == '"' || c == '\'' || c.is_whitespace())
        .trim_start_matches("queryId:")
        .trim();
    (!id.is_empty()).then(|| id.to_string())
}

/// Bookmarks 接口地址（分页靠 variables 里的 cursor）。
pub fn bookmarks_url(query_id: &str) -> String {
    format!("https://x.com/i/api/graphql/{query_id}/Bookmarks")
}

/// 桌面浏览器 UA：X 对明显非浏览器的客户端会直接拒绝。
const DESKTOP_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                          (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";

/// 拉一页书签：返回（HTTP 状态，响应体）。
///
/// Cookie 原样放进请求头，只做 GET —— 这条路线依赖用户自己的登录态，
/// **不调用任何写入接口**。
pub async fn fetch_bookmarks_page(
    query_id: &str,
    auth_token: &str,
    ct0: &str,
    cursor: &str,
) -> Result<(u16, String), String> {
    let variables = bookmarks_variables(cursor).to_string();
    let features = bookmarks_features().to_string();
    let resp = crate::commands::utils::get_http_client()
        .get(bookmarks_url(query_id))
        .query(&[
            ("variables", variables.as_str()),
            ("features", features.as_str()),
        ])
        .header("authorization", format!("Bearer {BEARER}"))
        .header("x-csrf-token", ct0)
        .header("cookie", format!("auth_token={auth_token}; ct0={ct0}"))
        .header("x-twitter-active-user", "yes")
        .header("x-twitter-auth-type", "OAuth2Session")
        .header("x-twitter-client-language", "en")
        .header("User-Agent", DESKTOP_UA)
        .header("Referer", "https://x.com/i/bookmarks")
        .header("Accept", "*/*")
        .send()
        .await
        .map_err(|e| format!("请求 X 书签接口失败: {e}"))?;
    let status = resp.status().as_u16();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("读取 X 响应体失败: {e}"))?;
    Ok((status, body))
}

/// 请求的 `variables`（cursor 为空串表示第一页）。
pub fn bookmarks_variables(cursor: &str) -> Value {
    json!({
        "count": PAGE_SIZE,
        "cursor": cursor,
        "includePromotedContent": false,
        "withClientEventToken": false,
        "withBirdwatchNotes": false,
        "withVoice": true,
    })
}

/// 请求的 `features`：必须与 queryId 同版本，缺失会被 GraphQL 校验拒绝（422）。
pub fn bookmarks_features() -> Value {
    json!({
        "rweb_video_screen_enabled": false,
        "payments_enabled": false,
        "profile_label_improvements_pcf_label_in_post_enabled": true,
        "rweb_tipjar_consumption_enabled": true,
        "verified_phone_label_enabled": false,
        "creator_subscriptions_tweet_preview_api_enabled": true,
        "responsive_web_graphql_timeline_navigation_enabled": true,
        "responsive_web_graphql_skip_user_profile_image_extensions_enabled": false,
        "premium_content_api_read_enabled": false,
        "communities_web_enable_tweet_community_results_fetch": true,
        "c9s_tweet_anatomy_moderator_badge_enabled": true,
        "responsive_web_grok_analyze_button_fetch_trends_enabled": false,
        "responsive_web_grok_analyze_post_followups_enabled": true,
        "responsive_web_jetfuel_frame": false,
        "responsive_web_grok_share_attachment_enabled": true,
        "articles_preview_enabled": true,
        "responsive_web_edit_tweet_api_enabled": true,
        "graphql_is_translatable_rweb_tweet_is_translatable_enabled": true,
        "view_counts_everywhere_api_enabled": true,
        "longform_notetweets_consumption_enabled": true,
        "responsive_web_twitter_article_tweet_consumption_enabled": true,
        "tweet_awards_web_tipping_enabled": false,
        "responsive_web_grok_show_grok_translated_post": false,
        "responsive_web_grok_analysis_button_from_backend": true,
        "creator_subscriptions_quote_tweet_preview_enabled": false,
        "freedom_of_speech_not_reach_fetch_enabled": true,
        "standardized_nudges_misinfo": true,
        "tweet_with_visibility_results_prefer_gql_limited_actions_policy_enabled": true,
        "longform_notetweets_rich_text_read_enabled": true,
        "longform_notetweets_inline_media_enabled": true,
        "responsive_web_grok_image_annotation_enabled": true,
        "responsive_web_grok_community_note_auto_translation_is_enabled": false,
        "responsive_web_enhance_cards_enabled": false,
    })
}

/// 一页的解析结果。`entry_count` 统计**所有**条目（含墓碑，不含 cursor），
/// 用来区分「真的到底了」和「这页恰好全是被删的推文」。
#[derive(Debug, Clone)]
pub struct TimelinePage {
    pub items: Vec<NewFavorite>,
    pub next_cursor: Option<String>,
    pub entry_count: usize,
}

/// 分页必须停的三条理由：本页没有任何条目 / 没有下一页游标 / 游标没前进。
/// 缺了前两条会沿空页一直翻到页数上限（真实故障：33 个书签翻了 77 页）。
pub fn should_stop_paging(page: &TimelinePage, requested_cursor: &str) -> bool {
    let next = page
        .next_cursor
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    page.entry_count == 0 || next.is_none() || next == Some(requested_cursor.trim())
}

/// 解析一整页响应（含条目计数与下一页 cursor）。
pub fn parse_timeline_page(body: &Value) -> Result<TimelinePage, String> {
    if let Some(errors) = body.get("errors").and_then(|v| v.as_array()) {
        if !errors.is_empty() {
            let message = errors
                .iter()
                .filter_map(|e| e.get("message").and_then(|m| m.as_str()))
                .collect::<Vec<_>>()
                .join("；");
            return Err(format!("X 接口返回错误: {message}"));
        }
    }

    let instructions = find_instructions(body).ok_or_else(|| {
        "X 返回的书签列表格式异常（没找到 timeline.instructions）：\
         接口可能已改版，请更新「X 接口 ID」后重试"
            .to_string()
    })?;

    let mut items = Vec::new();
    let mut cursor = None;
    let mut entry_count = 0usize;
    for instruction in instructions {
        if let Some(entries) = instruction.get("entries").and_then(|v| v.as_array()) {
            for entry in entries {
                collect_entry(entry, &mut items, &mut cursor, &mut entry_count);
            }
        }
        // TimelineReplaceEntry / TimelinePinEntry 把单条 entry 放在 `entry`
        if let Some(entry) = instruction.get("entry") {
            collect_entry(entry, &mut items, &mut cursor, &mut entry_count);
        }
    }
    Ok(TimelinePage { items, next_cursor: cursor, entry_count })
}

/// 兼容旧签名：返回（条目，下一页 cursor）。
pub fn parse_timeline(body: &Value) -> Result<(Vec<NewFavorite>, Option<String>), String> {
    let page = parse_timeline_page(body)?;
    Ok((page.items, page.next_cursor))
}

/// 在 `data` 下找第一个带 `timeline.instructions` 的对象：新旧版响应键名不同
/// （`bookmark_timeline_v2` / `bookmark_timeline`），不写死具体键。
fn find_instructions(body: &Value) -> Option<&Vec<Value>> {
    let data = body.get("data")?.as_object()?;
    data.values()
        .find_map(|value| value.get("timeline")?.get("instructions")?.as_array())
}

fn collect_entry(
    entry: &Value,
    items: &mut Vec<NewFavorite>,
    cursor: &mut Option<String>,
    entry_count: &mut usize,
) {
    let Some(content) = entry.get("content") else {
        return;
    };

    // 底部分页游标
    let is_bottom_cursor = content.get("cursorType").and_then(|v| v.as_str()) == Some("Bottom")
        || entry
            .get("entryId")
            .and_then(|v| v.as_str())
            .is_some_and(|id| id.starts_with("cursor-bottom"));
    if is_bottom_cursor {
        if let Some(value) = content
            .get("value")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            *cursor = Some(value.to_string());
        }
        return;
    }

    // 非 cursor 的条目（含墓碑）都算「本页有内容」，这是判断是否到底的依据
    *entry_count += 1;

    if let Some(result) = content
        .get("itemContent")
        .and_then(|c| c.get("tweet_results"))
        .and_then(|t| t.get("result"))
    {
        if let Some(favorite) = tweet_to_favorite(result) {
            items.push(favorite);
        }
    }
}

/// 单条 `tweet_results.result` → 待落库条目。
pub fn tweet_to_favorite(result: &Value) -> Option<NewFavorite> {
    let typename = result
        .get("__typename")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    // 已删除 / 不可见：没有可展示的内容，也没有稳定的外部 id
    if typename == "TweetTombstone" {
        return None;
    }
    let tweet = if typename == "TweetWithVisibilityResults" {
        result.get("tweet")?
    } else {
        result
    };

    let rest_id = tweet.get("rest_id")?.as_str()?.trim();
    if rest_id.is_empty() {
        return None;
    }

    let legacy = tweet.get("legacy");
    let user = tweet
        .get("core")
        .and_then(|c| c.get("user_results"))
        .and_then(|u| u.get("result"));
    let screen_name = user
        .and_then(|u| nested_str(u, &["core", "screen_name"]))
        .or_else(|| user.and_then(|u| nested_str(u, &["legacy", "screen_name"])))
        .unwrap_or("");
    let author = user
        .and_then(|u| nested_str(u, &["core", "name"]))
        .or_else(|| user.and_then(|u| nested_str(u, &["legacy", "name"])))
        .unwrap_or("");

    let raw_text = tweet
        .get("note_tweet")
        .and_then(|n| n.get("note_tweet_results"))
        .and_then(|n| n.get("result"))
        .and_then(|r| r.get("text"))
        .and_then(|v| v.as_str())
        .or_else(|| {
            legacy
                .and_then(|l| l.get("full_text"))
                .and_then(|v| v.as_str())
        })
        .unwrap_or("");
    let text = expand_t_co(raw_text, legacy.and_then(|l| l.get("entities")));

    let url = if screen_name.is_empty() {
        format!("https://x.com/i/web/status/{rest_id}")
    } else {
        format!("https://x.com/{screen_name}/status/{rest_id}")
    };

    let subtitle = match (screen_name.is_empty(), author.is_empty()) {
        (false, false) => Some(format!("@{screen_name} · {author}")),
        (false, true) => Some(format!("@{screen_name}")),
        (true, false) => Some(author.to_string()),
        (true, true) => None,
    };

    Some(NewFavorite {
        source: SOURCE.to_string(),
        external_id: rest_id.to_string(),
        url,
        title: first_line_title(&text),
        subtitle,
        description: (!text.trim().is_empty()).then_some(text),
        // 接口不返回书签时间：**不用推文发布时间冒充**，展示与排序回退 created_at
        favorited_at: None,
        extra_json: Some(
            json!({
                "screenName": screen_name,
                "author": author,
                "createdAt": legacy.and_then(|l| l.get("created_at")).and_then(|v| v.as_str()),
                "favoriteCount": legacy.and_then(|l| l.get("favorite_count")).and_then(|v| v.as_i64()),
                "retweetCount": legacy.and_then(|l| l.get("retweet_count")).and_then(|v| v.as_i64()),
                "lang": legacy.and_then(|l| l.get("lang")).and_then(|v| v.as_str()),
                "quotedTweetId": tweet
                    .get("quoted_status_result")
                    .and_then(|q| q.get("result"))
                    .and_then(|r| r.get("rest_id"))
                    .and_then(|v| v.as_str()),
                "isRetweet": legacy.and_then(|l| l.get("retweeted_status_result")).is_some(),
            })
            .to_string(),
        ),
        initial_status: None,
    })
}

fn nested_str<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter()
        .try_fold(value, |cur, key| cur.get(*key))?
        .as_str()
}

/// 把正文里的 `t.co` 短链换成真实地址（X 的短链对 AI 归类没有信息量）。
pub(super) fn expand_t_co(text: &str, entities: Option<&Value>) -> String {
    let Some(urls) = entities
        .and_then(|e| e.get("urls"))
        .and_then(|v| v.as_array())
    else {
        return text.to_string();
    };
    let mut out = text.to_string();
    for url in urls {
        let (Some(short), Some(expanded)) = (
            url.get("url").and_then(|v| v.as_str()),
            url.get("expanded_url").and_then(|v| v.as_str()),
        ) else {
            continue;
        };
        if !short.is_empty() {
            out = out.replace(short, expanded);
        }
    }
    out
}

/// 推文没有标题：取正文首行，按字符截断（中文不能按字节切）。
pub(super) fn first_line_title(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    let mut chars = line.chars();
    let mut title: String = chars.by_ref().take(TITLE_MAX_CHARS).collect();
    if chars.next().is_some() {
        title.push('…');
    }
    title
}

/// 把 HTTP 状态翻成可操作提示。
pub fn map_status_error(status: u16) -> String {
    let hint = match status {
        401 | 403 => {
            "：Cookie 可能已失效，请在「X Cookie」里重新粘贴整条 Cookie（需含 auth_token 与 ct0）"
        }
        404 => {
            "：X 的接口 ID 可能已随前端发版变化，请在「X 接口 ID」里粘贴新的请求地址\
             （登录 x.com 打开书签页，开发者工具「网络」里筛 /i/api/graphql/，\
             复制任意 Bookmarks 请求的完整 URL）"
        }
        422 => {
            "：GraphQL 校验失败，通常是接口 ID 与 features 版本不匹配；\
             请先在「X 接口 ID」里更新请求地址后重试"
        }
        429 => "：请求过于频繁，请稍后再试（X 对私有接口限流较严）",
        _ => "",
    };
    format!("X 书签导入失败 (HTTP {status}){hint}")
}

#[cfg(test)]
mod tests;
