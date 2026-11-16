use serde_json::json;

use super::*;

fn sample_page() -> Value {
    json!({
        "data": [{
            "id": "111",
            "text": "Hello https://t.co/abc",
            "created_at": "2026-11-10T10:00:00.000Z",
            "author_id": "42",
            "lang": "en",
            "entities": {
                "urls": [
                    { "url": "https://t.co/abc", "expanded_url": "https://example.com/a" }
                ]
            },
            "public_metrics": { "like_count": 5, "retweet_count": 2 }
        }],
        "includes": { "users": [{ "id": "42", "username": "alice", "name": "Alice" }] },
        "meta": { "result_count": 1, "next_token": "NEXT" }
    })
}

#[test]
fn bookmarks_url_uses_user_id() {
    assert_eq!(
        bookmarks_url("42"),
        "https://api.x.com/2/users/42/bookmarks"
    );
}

#[test]
fn page_params_sets_max_results_and_optional_token() {
    let first = page_params(None);
    assert!(first
        .iter()
        .any(|(key, value)| *key == "max_results" && value == "100"));
    assert!(!first.iter().any(|(key, _)| *key == "pagination_token"));
    assert!(first.iter().any(|(key, _)| *key == "post.fields"));

    let next = page_params(Some("T"));
    assert!(next
        .iter()
        .any(|(key, value)| *key == "pagination_token" && value == "T"));
}

#[test]
fn parse_page_maps_post_with_author() {
    let page = parse_page(&sample_page()).unwrap();
    assert_eq!(page.items.len(), 1);
    let favorite = &page.items[0];
    assert_eq!(favorite.source, "x");
    assert_eq!(favorite.external_id, "111");
    assert_eq!(favorite.url, "https://x.com/alice/status/111");
    assert_eq!(favorite.title, "Hello https://example.com/a");
    assert_eq!(favorite.subtitle.as_deref(), Some("@alice · Alice"));
    // 官方接口不返回收藏时间
    assert_eq!(favorite.favorited_at, None);
    assert_eq!(page.next_token.as_deref(), Some("NEXT"));
}

#[test]
fn parse_page_prefers_note_post_text() {
    let mut body = sample_page();
    body["data"][0]["text"] = json!("truncated…");
    body["data"][0]["note_post"] = json!({ "text": "FULL LONG TEXT" });
    let page = parse_page(&body).unwrap();
    assert_eq!(page.items[0].description.as_deref(), Some("FULL LONG TEXT"));
}

#[test]
fn parse_page_falls_back_url_without_author() {
    let mut body = sample_page();
    body.as_object_mut().unwrap().remove("includes");
    let page = parse_page(&body).unwrap();
    assert_eq!(page.items[0].url, "https://x.com/i/web/status/111");
}

#[test]
fn parse_page_reports_error_without_data() {
    let err = parse_page(&json!({
        "errors": [{ "title": "Not Authorized", "detail": "Unauthorized" }]
    }))
    .unwrap_err();
    assert!(
        err.contains("Not Authorized") || err.contains("Unauthorized"),
        "实际: {err}"
    );
}

#[test]
fn parse_page_keeps_data_when_partial_errors_exist() {
    let mut body = sample_page();
    body["errors"] = json!([{ "title": "Partial", "detail": "some posts unavailable" }]);
    let page = parse_page(&body).unwrap();
    assert_eq!(page.items.len(), 1, "部分失败不该丢掉已返回的数据");
}

#[test]
fn should_stop_only_when_page_has_no_new_items() {
    assert!(should_stop(0, 100), "整页都是旧数据，增量到此为止");
    assert!(!should_stop(1, 100), "有新增就继续翻");
    assert!(!should_stop(0, 0), "空页交给 next_token 判断");
}

#[test]
fn page_params_with_uses_legacy_field_name() {
    let params = page_params_with(Some("T"), "tweet.fields", LEGACY_POST_FIELDS);
    assert!(params.iter().any(|(key, _)| *key == "tweet.fields"));
    assert!(!params.iter().any(|(key, _)| *key == "post.fields"));
    assert!(params
        .iter()
        .any(|(key, value)| *key == "pagination_token" && value == "T"));
}

#[test]
fn parse_page_reads_legacy_note_tweet() {
    let mut body = sample_page();
    body["data"][0]["text"] = json!("truncated…");
    body["data"][0]["note_tweet"] = json!({
        "note_tweet_results": { "result": { "text": "LEGACY LONG TEXT" } }
    });
    let page = parse_page(&body).unwrap();
    assert_eq!(
        page.items[0].description.as_deref(),
        Some("LEGACY LONG TEXT")
    );
}
