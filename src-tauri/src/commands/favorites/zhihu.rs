//! 知乎收藏导入（**只读**）——走知乎**开放平台官方接口**。
//!
//! 为什么不用 webview / 逆向：`www.zhihu.com/api/v4/*` 要求 `x-zse-96` 签名
//! （jsvmp 混淆，随版本变化）；而开放平台（`developer.zhihu.com`）对「用户收藏」
//! 提供了官方接口，鉴权只是 `Authorization: Bearer <AccessSecret>` + 秒级时间戳头，
//! 在 developer.zhihu.com/profile 生成 Access Secret 即可使用。
//!
//! 接口（来自官方文档 `developer.zhihu.com/docs?key=user_collections`）：
//! - `GET /api/v1/user/favlists`                        收藏夹列表（Limit，无分页）
//! - `GET /api/v1/user/favlist_contents`                收藏夹内容（Offset/Limit + Paging.IsEnd）
//!
//! ⚠️ 额度：这类用户数据接口按「自然日」配额（user_data 项，默认 100 次/天、
//! 未实名 10 次/天），每次翻页都消耗一次。超限返回 30001/30002，
//! 报错里会明说，不会静默给空列表。

use serde_json::{json, Value};

use super::db::NewFavorite;

/// 数据源标识（写入 `favorite.source`）
pub const SOURCE: &str = "zhihu";

pub const BASE_URL: &str = "https://developer.zhihu.com";

/// 每页条数（官方默认 20；文档未给出上限，用保守值）
pub const PAGE_SIZE: usize = 20;

/// 收藏夹列表请求的 Limit（接口无分页字段，一次尽量多取）
pub const FAVLISTS_LIMIT: usize = 100;

/// 收藏夹列表。
pub fn favlists_path() -> String {
    format!("/api/v1/user/favlists?Limit={}", FAVLISTS_LIMIT)
}

/// 收藏夹内容第 offset 条开始的分页。
pub fn contents_path(favlist_url_token: i64, offset: usize) -> String {
    format!(
        "/api/v1/user/favlist_contents?FavlistUrlToken={}&Offset={}&Limit={}",
        favlist_url_token, offset, PAGE_SIZE
    )
}

/// 构造鉴权请求头。
///
/// `X-Request-Timestamp` 与服务器时间差不能超过 10 分钟，所以每次请求都取当前时间；
/// 时间戳作为参数传入便于测试。
pub fn auth_headers(access_secret: &str, timestamp: u64) -> Vec<(&'static str, String)> {
    vec![
        ("Authorization", format!("Bearer {}", access_secret)),
        ("X-Request-Timestamp", timestamp.to_string()),
        ("Content-Type", "application/json".to_string()),
    ]
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 解析响应外层：官方文档用 PascalCase（`Data`/`Code`），做一层兼容防改版。
///
/// 返回 `Data` 部分；`Code != 0` 时给出可操作提示。
pub fn parse_envelope(body: &Value) -> Result<Value, String> {
    let code = body
        .get("Code")
        .or_else(|| body.get("code"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    if code != 0 {
        let message = body
            .get("Message")
            .or_else(|| body.get("message"))
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        return Err(map_api_error(code, message));
    }
    body.get("Data")
        .or_else(|| body.get("data"))
        .cloned()
        .ok_or_else(|| "知乎响应缺少 Data 字段（接口可能已改版）".to_string())
}

/// 官方错误码 → 可操作提示。
pub fn map_api_error(code: i64, message: &str) -> String {
    let hint = match code {
        20001 => "（Access Secret 无效或已过期，请到 developer.zhihu.com/profile 重新生成）",
        30001 | 30002 => {
            "（今日用户数据额度已用完：这类接口按自然日配额，默认 100 次/天、未实名 10 次/天，明天再导或在开放平台查看额度）"
        }
        30003 => "（被知乎风控拒绝，请稍后再试）",
        10001 => "（参数错误，可能是接口改版）",
        90001 => "（知乎服务端内部错误，稍后再试）",
        _ => "",
    };
    format!("知乎返回错误 {} {}{}", code, message, hint)
}

/// 一条收藏内容 → 待落库条目。
///
/// 官方 items **没有内容 id**，字段是 `Url / ContentType / Title / Summary / Author`；
/// 去重键用 **Url**（同一内容的规范链接，稳定且唯一）。
/// 字段全是 PascalCase，缺哪个都只影响该条。
pub fn item_to_favorite(item: &Value, collection_title: &str) -> Option<NewFavorite> {
    let url = item
        .get("Url")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())?;
    let title = item
        .get("Title")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())?;
    let kind = item
        .get("ContentType")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let summary = item
        .get("Summary")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let author = item
        .pointer("/Author/Name")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    Some(NewFavorite {
        source: SOURCE.to_string(),
        external_id: url.clone(),
        url,
        title,
        subtitle: author.or_else(|| Some(collection_title.to_string())),
        description: summary,
        extra_json: Some(
            json!({
                "kind": kind,
                "collection": collection_title,
                "fav_time": item.get("FavTime").and_then(|v| v.as_i64()),
                "created": item.get("CreatedAt").and_then(|v| v.as_i64()),
                "like_count": item.get("LikeCount").and_then(|v| v.as_i64()),
            })
            .to_string(),
        ),
        initial_status: None,
    })
}

/// 从内容页响应里取（条目数组，是否结束，下一个 Offset）。
pub fn parse_contents_page(payload: &Value) -> (Vec<Value>, bool, usize) {
    let items = payload
        .get("Items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let is_end = payload
        .pointer("/Paging/IsEnd")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let next_offset = payload
        .pointer("/Paging/NextOffset")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<usize>().ok());
    (items, is_end, next_offset.unwrap_or(usize::MAX))
}

#[cfg(test)]
mod tests {
    use super::{
        auth_headers, contents_path, favlists_path, item_to_favorite, map_api_error,
        parse_contents_page, parse_envelope, FAVLISTS_LIMIT, PAGE_SIZE,
    };
    use serde_json::json;

    #[test]
    fn paths_match_official_endpoints() {
        assert_eq!(
            favlists_path(),
            format!("/api/v1/user/favlists?Limit={}", FAVLISTS_LIMIT)
        );
        assert_eq!(
            contents_path(123456789, 40),
            format!(
                "/api/v1/user/favlist_contents?FavlistUrlToken=123456789&Offset=40&Limit={}",
                PAGE_SIZE
            )
        );
    }

    #[test]
    fn auth_headers_carry_bearer_and_timestamp() {
        let headers = auth_headers("secret-1", 1742822400);
        assert!(headers.contains(&("Authorization", "Bearer secret-1".to_string())));
        assert!(headers.contains(&("X-Request-Timestamp", "1742822400".to_string())));
        assert!(headers.contains(&("Content-Type", "application/json".to_string())));
    }

    /// 官方是 PascalCase，但做一层小写兼容（改版不至于直接坏）。
    #[test]
    fn envelope_accepts_both_casings() {
        let pascal = json!({"Code": 0, "Data": {"Items": []}});
        assert!(parse_envelope(&pascal).is_ok());
        let lower = json!({"code": 0, "data": {"Items": []}});
        assert!(parse_envelope(&lower).is_ok());
    }

    #[test]
    fn api_errors_are_actionable() {
        assert!(map_api_error(20001, "").contains("Access Secret"));
        assert!(map_api_error(30001, "").contains("额度"));
        assert!(map_api_error(30002, "").contains("额度"));
    }

    #[test]
    fn envelope_reports_error_codes() {
        let body = json!({"Code": 20001, "Message": "auth failed"});
        let err = parse_envelope(&body).unwrap_err();
        assert!(err.contains("20001") && err.contains("Access Secret"), "实际: {}", err);
        // 缺 Data 视为异常而不是空结果
        let empty = json!({"Code": 0});
        assert!(parse_envelope(&empty).is_err());
    }

    #[test]
    fn maps_item_using_url_as_dedup_key() {
        let item = json!({
            "ContentType": "answer",
            "Url": "https://www.zhihu.com/question/1/answer/2",
            "Title": "如何评价 X？",
            "Summary": "摘要",
            "FavTime": 1700000000,
            "LikeCount": 12,
            "Author": { "Name": "某人" }
        });
        let fav = item_to_favorite(&item, "我的收藏").unwrap();
        // 官方 items 没有内容 id，Url 就是唯一稳定标识
        assert_eq!(fav.external_id, "https://www.zhihu.com/question/1/answer/2");
        assert_eq!(fav.url, "https://www.zhihu.com/question/1/answer/2");
        assert_eq!(fav.title, "如何评价 X？");
        assert_eq!(fav.subtitle.as_deref(), Some("某人"));
        assert_eq!(fav.description.as_deref(), Some("摘要"));
        assert!(fav.extra_json.as_ref().unwrap().contains("我的收藏"));
    }

    /// 缺 Url 或 Title 的条目跳过，别让整批失败。
    #[test]
    fn malformed_items_are_skipped() {
        assert!(item_to_favorite(&json!({"Title": "x"}), "夹").is_none());
        assert!(item_to_favorite(&json!({"Url": "https://x"}), "夹").is_none());
    }

    #[test]
    fn contents_page_reads_paging() {
        let payload = json!({
            "Items": [{"Url": "https://x", "Title": "t"}],
            "Paging": { "IsEnd": false, "NextOffset": "40", "Totals": 100 }
        });
        let (items, is_end, next) = parse_contents_page(&payload);
        assert_eq!(items.len(), 1);
        assert!(!is_end);
        assert_eq!(next, 40);

        let ended = json!({ "Items": [], "Paging": { "IsEnd": true } });
        let (items, is_end, next) = parse_contents_page(&ended);
        assert!(items.is_empty() && is_end);
        // 没给 NextOffset 时返回哨兵值，调用方按「结束」处理
        assert_eq!(next, usize::MAX);
    }
}
