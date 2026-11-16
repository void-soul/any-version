//! X 官方 API v2 书签读取：分页参数、响应映射与增量判定。
//!
//! 官方接口**不返回收藏时间**（`post.fields` 里没有 `bookmarked_at`），
//! 所以 `favorited_at` 一律留空；增量靠「整页都没有新增就停」+ 本地记录的上次读取时间。
//!
//! 每次请求都是只读 GET，不做任何写操作。

use std::collections::HashMap;

use serde_json::Value;

use super::db::NewFavorite;
use super::x;

pub const API_BASE: &str = "https://api.x.com/2";

/// 官方文档：书签只给最近 800 条，每页最多 100 → 8 页封顶。
pub const MAX_PAGES: usize = 8;
pub const PAGE_SIZE: usize = 100;

/// 页间间隔：官方限流按 15 分钟窗口计，留出余量，别把额度一次打满。
pub const PAGE_DELAY_MS: u64 = 800;

/// 需要的字段（X 2026 文档已把 `tweet.fields` 改名为 `post.fields`）。
pub const POST_FIELDS: &str =
    "id,text,created_at,author_id,lang,entities,note_post,public_metrics,referenced_posts,attachments";

/// 旧版字段名（`tweet.fields` + `note_tweet`）：2026 文档改名后，老服务端仍只认这套。
pub const LEGACY_POST_FIELDS: &str =
    "id,text,created_at,author_id,lang,entities,note_tweet,public_metrics,referenced_posts,attachments";

/// 一页的解析结果。
#[derive(Debug, Clone, Default)]
pub struct Page {
    pub items: Vec<NewFavorite>,
    pub next_token: Option<String>,
}

pub fn bookmarks_url(user_id: &str) -> String {
    format!("{API_BASE}/users/{user_id}/bookmarks")
}

/// 请求参数（`pagination_token` 只有非首页才带）。
pub fn page_params(cursor: Option<&str>) -> Vec<(&'static str, String)> {
    page_params_with(cursor, "post.fields", POST_FIELDS)
}

/// 显式指定字段参数名与字段表：用于 `post.fields` → `tweet.fields` 的兼容回退。
pub fn page_params_with(
    cursor: Option<&str>,
    fields_param: &'static str,
    fields_value: &'static str,
) -> Vec<(&'static str, String)> {
    let mut params = vec![
        ("max_results", PAGE_SIZE.to_string()),
        (fields_param, fields_value.to_string()),
        ("expansions", "author_id".to_string()),
        ("user.fields", "username,name".to_string()),
    ];
    if let Some(token) = cursor.filter(|token| !token.is_empty()) {
        params.push(("pagination_token", token.to_string()));
    }
    params
}

pub fn parse_page(body: &Value) -> Result<Page, String> {
    let Some(data) = body.get("data").and_then(|value| value.as_array()) else {
        // 没有 data 才是真失败；有 data 时的 errors 属于「部分条目不可见」，不该丢整页
        if let Some(errors) = body.get("errors").and_then(|value| value.as_array()) {
            if !errors.is_empty() {
                let detail = errors
                    .iter()
                    .filter_map(|error| {
                        error
                            .get("detail")
                            .and_then(|value| value.as_str())
                            .or_else(|| error.get("title").and_then(|value| value.as_str()))
                            .or_else(|| error.get("message").and_then(|value| value.as_str()))
                    })
                    .collect::<Vec<_>>()
                    .join("；");
                return Err(format!("X 官方接口返回错误: {detail}"));
            }
        }
        return Err("X 官方接口没有返回书签数据".to_string());
    };

    let users = collect_users(body);
    let items = data
        .iter()
        .filter_map(|post| post_to_favorite(post, &users))
        .collect();
    let next_token = body
        .get("meta")
        .and_then(|meta| meta.get("next_token"))
        .and_then(|value| value.as_str())
        .map(str::to_string);
    Ok(Page { items, next_token })
}

/// 增量判定：这一页没有任何新增，说明已经追上上次导入的位置。
pub fn should_stop(added: usize, page_len: usize) -> bool {
    page_len > 0 && added == 0
}

/// `includes.users` → `author_id -> (username, name)`。
fn collect_users(body: &Value) -> HashMap<String, (String, String)> {
    let mut users = HashMap::new();
    let Some(list) = body
        .get("includes")
        .and_then(|includes| includes.get("users"))
        .and_then(|value| value.as_array())
    else {
        return users;
    };
    for user in list {
        let Some(id) = user.get("id").and_then(|value| value.as_str()) else {
            continue;
        };
        let username = user
            .get("username")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let name = user
            .get("name")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        users.insert(id.to_string(), (username.to_string(), name.to_string()));
    }
    users
}

fn referenced_id(post: &Value, kind: &str) -> Option<String> {
    post.get("referenced_posts")
        .and_then(|value| value.as_array())?
        .iter()
        .find(|reference| reference.get("type").and_then(|value| value.as_str()) == Some(kind))
        .and_then(|reference| reference.get("id"))
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

fn post_to_favorite(
    post: &Value,
    users: &HashMap<String, (String, String)>,
) -> Option<NewFavorite> {
    let id = post.get("id")?.as_str()?.trim();
    if id.is_empty() {
        return None;
    }

    let author_id = post
        .get("author_id")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let (screen_name, author) = users.get(author_id).cloned().unwrap_or_default();

    // 长贴：新版用 note_post.text，旧版用 note_tweet.note_tweet_results.result.text
    let raw_text = post
        .get("note_post")
        .and_then(|note| note.get("text"))
        .and_then(|value| value.as_str())
        .or_else(|| {
            post.get("note_tweet")
                .and_then(|note| note.get("note_tweet_results"))
                .and_then(|note| note.get("result"))
                .and_then(|result| result.get("text"))
                .and_then(|value| value.as_str())
        })
        .or_else(|| post.get("text").and_then(|value| value.as_str()))
        .unwrap_or("");
    let text = x::expand_t_co(raw_text, post.get("entities"));

    let url = if screen_name.is_empty() {
        format!("https://x.com/i/web/status/{id}")
    } else {
        format!("https://x.com/{screen_name}/status/{id}")
    };
    let subtitle = match (screen_name.is_empty(), author.is_empty()) {
        (false, false) => Some(format!("@{screen_name} · {author}")),
        (false, true) => Some(format!("@{screen_name}")),
        (true, false) => Some(author.clone()),
        (true, true) => None,
    };

    let metrics = post.get("public_metrics");
    Some(NewFavorite {
        source: x::SOURCE.to_string(),
        external_id: id.to_string(),
        url,
        title: x::first_line_title(&text),
        subtitle,
        description: (!text.trim().is_empty()).then_some(text),
        // 官方接口没有 bookmarked_at：不拿发帖时间冒充收藏时间
        favorited_at: None,
        extra_json: Some(
            serde_json::json!({
                "screenName": screen_name,
                "author": author,
                "createdAt": post.get("created_at").and_then(|value| value.as_str()),
                "favoriteCount": metrics
                    .and_then(|value| value.get("like_count"))
                    .and_then(|value| value.as_i64()),
                "retweetCount": metrics
                    .and_then(|value| value.get("retweet_count"))
                    .and_then(|value| value.as_i64()),
                "lang": post.get("lang").and_then(|value| value.as_str()),
                "quotedPostId": referenced_id(post, "quoted"),
                "isRetweet": referenced_id(post, "retweeted").is_some(),
            })
            .to_string(),
        ),
        initial_status: None,
    })
}

#[cfg(test)]
mod tests;
