use serde_json::json;

use super::*;

fn sample_tweet() -> Value {
    json!({
        "__typename": "Tweet",
        "rest_id": "1234567890",
        "core": {
            "user_results": {
                "result": {
                    "__typename": "User",
                    "core": { "screen_name": "alice", "name": "Alice" },
                    "legacy": { "screen_name": "alice", "name": "Alice" }
                }
            }
        },
        "legacy": {
            "full_text": "Hello https://t.co/abc world",
            "created_at": "Wed Nov 12 10:00:00 +0000 2026",
            "favorite_count": 12,
            "retweet_count": 3,
            "lang": "en",
            "entities": {
                "urls": [
                    { "url": "https://t.co/abc", "expanded_url": "https://example.com/a" }
                ]
            }
        }
    })
}

// ─── Cookie ───

#[test]
fn extract_auth_reads_both_tokens() {
    let (auth, ct0) = extract_auth("auth_token=aaa; ct0=bbb").unwrap();
    assert_eq!(auth, "aaa");
    assert_eq!(ct0, "bbb");
}

#[test]
fn extract_auth_names_the_missing_key() {
    let err = extract_auth("auth_token=aaa").unwrap_err();
    assert!(err.contains("ct0"), "错误应指明缺 ct0，实际: {err}");
}

#[test]
fn extract_auth_tolerates_case_quotes_and_spaces() {
    let (auth, ct0) = extract_auth(r#" AUTH_TOKEN = "aaa" ; Ct0 = bbb "#).unwrap();
    assert_eq!(auth, "aaa");
    assert_eq!(ct0, "bbb");
}

// ─── 条目映射 ───

#[test]
fn tweet_to_favorite_maps_core_fields() {
    let fav = tweet_to_favorite(&sample_tweet()).unwrap();
    assert_eq!(fav.source, "x");
    assert_eq!(fav.external_id, "1234567890");
    assert_eq!(fav.url, "https://x.com/alice/status/1234567890");
    assert_eq!(fav.title, "Hello https://example.com/a world");
    assert_eq!(fav.subtitle.as_deref(), Some("@alice · Alice"));
    assert_eq!(
        fav.description.as_deref(),
        Some("Hello https://example.com/a world")
    );
    // 接口不返回书签时间：绝不能用推文发布时间冒充
    assert_eq!(fav.favorited_at, None);
    let extra: Value = serde_json::from_str(fav.extra_json.as_deref().unwrap()).unwrap();
    assert_eq!(extra["screenName"], "alice");
    assert_eq!(extra["favoriteCount"], 12);
}

#[test]
fn tweet_to_favorite_unwraps_visibility_wrapper() {
    let wrapped = json!({
        "__typename": "TweetWithVisibilityResults",
        "tweet": sample_tweet()
    });
    let fav = tweet_to_favorite(&wrapped).unwrap();
    assert_eq!(fav.external_id, "1234567890");
    assert_eq!(fav.url, "https://x.com/alice/status/1234567890");
}

#[test]
fn tweet_to_favorite_prefers_note_tweet_for_long_text() {
    let mut tweet = sample_tweet();
    tweet["note_tweet"] = json!({
        "note_tweet_results": { "result": { "text": "LONG BODY" } }
    });
    let fav = tweet_to_favorite(&tweet).unwrap();
    assert_eq!(fav.description.as_deref(), Some("LONG BODY"));
}

#[test]
fn tweet_to_favorite_expands_t_co_links() {
    let mut tweet = sample_tweet();
    tweet["legacy"]["full_text"] = json!("see https://t.co/abc now");
    let fav = tweet_to_favorite(&tweet).unwrap();
    assert_eq!(
        fav.description.as_deref(),
        Some("see https://example.com/a now")
    );
}

#[test]
fn tweet_to_favorite_falls_back_url_without_screen_name() {
    let mut tweet = sample_tweet();
    tweet.as_object_mut().unwrap().remove("core");
    let fav = tweet_to_favorite(&tweet).unwrap();
    assert_eq!(fav.url, "https://x.com/i/web/status/1234567890");
}

#[test]
fn tweet_to_favorite_truncates_title_by_chars() {
    let mut tweet = sample_tweet();
    tweet["legacy"]["full_text"] = json!("中".repeat(200));
    let fav = tweet_to_favorite(&tweet).unwrap();
    assert_eq!(fav.title.chars().count(), TITLE_MAX_CHARS + 1);
    assert!(fav.title.ends_with('…'));
}

#[test]
fn tweet_to_favorite_skips_tombstone() {
    let tombstone = json!({
        "__typename": "TweetTombstone",
        "tombstone": { "text": { "text": "This Tweet is unavailable" } }
    });
    assert!(tweet_to_favorite(&tombstone).is_none());
}

// ─── 分页 ───

#[test]
fn parse_timeline_returns_items_and_bottom_cursor() {
    let body = json!({
        "data": {
            "bookmark_timeline_v2": {
                "timeline": {
                    "instructions": [
                        { "type": "TimelineClearCache" },
                        {
                            "type": "TimelineAddEntries",
                            "entries": [
                                {
                                    "entryId": "tweet-1234567890",
                                    "content": {
                                        "entryType": "TimelineTimelineItem",
                                        "itemContent": {
                                            "tweet_results": { "result": sample_tweet() }
                                        }
                                    }
                                },
                                {
                                    "entryId": "cursor-bottom-1",
                                    "content": {
                                        "entryType": "TimelineTimelineCursor",
                                        "cursorType": "Bottom",
                                        "value": "CURSOR_2"
                                    }
                                }
                            ]
                        }
                    ]
                }
            }
        }
    });
    let (items, cursor) = parse_timeline(&body).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].external_id, "1234567890");
    assert_eq!(cursor.as_deref(), Some("CURSOR_2"));
}

#[test]
fn parse_timeline_reads_replace_entry() {
    let body = json!({
        "data": {
            "bookmark_timeline": {
                "timeline": {
                    "instructions": [
                        {
                            "type": "TimelineReplaceEntry",
                            "entry": {
                                "entryId": "tweet-1234567890",
                                "content": {
                                    "itemContent": {
                                        "tweet_results": { "result": sample_tweet() }
                                    }
                                }
                            }
                        }
                    ]
                }
            }
        }
    });
    let (items, cursor) = parse_timeline(&body).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(cursor, None);
}

#[test]
fn parse_timeline_reports_missing_timeline() {
    let err = parse_timeline(&json!({ "data": {} })).unwrap_err();
    assert!(err.contains("格式"), "实际: {err}");
}

#[test]
fn parse_timeline_surfaces_graphql_errors() {
    let err = parse_timeline(&json!({
        "errors": [{ "message": "Query not found" }]
    }))
    .unwrap_err();
    assert!(err.contains("Query not found"), "实际: {err}");
}

// ─── 请求与错误文案 ───

#[test]
fn bookmarks_variables_carries_cursor() {
    assert_eq!(
        bookmarks_variables("").as_object().unwrap()["count"],
        PAGE_SIZE
    );
    assert_eq!(bookmarks_variables("").as_object().unwrap()["cursor"], "");
    assert_eq!(
        bookmarks_variables("CUR").as_object().unwrap()["cursor"],
        "CUR"
    );
}

#[test]
fn bookmarks_url_uses_query_id() {
    let url = bookmarks_url("QID");
    assert!(url.starts_with("https://x.com/i/api/graphql/QID/Bookmarks"));
}

#[test]
fn query_id_accepts_bare_id_or_full_url() {
    assert_eq!(
        query_id_from_credential("  abc123 \n").as_deref(),
        Some("abc123")
    );
    assert_eq!(
        query_id_from_credential("https://x.com/i/api/graphql/XYZ789/Bookmarks?variables=%7B%7D")
            .as_deref(),
        Some("XYZ789")
    );
    assert_eq!(query_id_from_credential("   "), None);
}

#[test]
fn map_status_error_points_to_the_right_fix() {
    assert!(map_status_error(401).contains("Cookie"));
    assert!(map_status_error(403).contains("Cookie"));
    assert!(map_status_error(404).contains("接口 ID"));
    assert!(map_status_error(422).contains("接口 ID"));
    assert!(map_status_error(429).contains("稍后"));
}

// ─── 分页终止条件（用户反馈：33 个书签却翻了 77 页） ───

fn cursor_only_body(value: &str) -> serde_json::Value {
    json!({
        "data": { "bookmark_timeline_v2": { "timeline": { "instructions": [
            { "type": "TimelineAddEntries", "entries": [
                { "entryId": "cursor-bottom-1", "content": {
                    "entryType": "TimelineTimelineCursor", "cursorType": "Bottom", "value": value
                }}
            ]}
        ]}}}
    })
}

/// 本页一个条目都没有，说明已经到底：即使服务端还回了一个非空 cursor 也必须停，
/// 否则会沿着空页一路翻到页数上限。
#[test]
fn empty_page_stops_paging_even_with_cursor() {
    let page = parse_timeline_page(&cursor_only_body("CURSOR_2")).unwrap();
    assert_eq!(page.entry_count, 0, "只有 cursor 的页应算 0 条");
    assert_eq!(page.next_cursor.as_deref(), Some("CURSOR_2"));
    assert!(should_stop_paging(&page, "CURSOR_1"), "空页必须停止");
}

/// 游标没有前进会原地死循环，必须停。
#[test]
fn unchanged_cursor_stops_paging() {
    let page = TimelinePage {
        items: Vec::new(),
        next_cursor: Some("SAME".into()),
        entry_count: 20,
    };
    assert!(should_stop_paging(&page, "SAME"));
}

#[test]
fn normal_page_keeps_paging() {
    let page = TimelinePage {
        items: Vec::new(),
        next_cursor: Some("NEXT".into()),
        entry_count: 20,
    };
    assert!(!should_stop_paging(&page, "PREV"));
}

#[test]
fn missing_cursor_stops_paging() {
    let page = TimelinePage { items: Vec::new(), next_cursor: None, entry_count: 20 };
    assert!(should_stop_paging(&page, "PREV"));
    let blank = TimelinePage { items: Vec::new(), next_cursor: Some("  ".into()), entry_count: 20 };
    assert!(should_stop_paging(&blank, "PREV"), "空白 cursor 等同没有");
}

/// 全是墓碑的页仍算有内容，不能仅因为解析出 0 条就停（否则会漏后面的书签）。
#[test]
fn page_with_only_tombstones_is_not_treated_as_empty() {
    let body = json!({
        "data": { "bookmark_timeline_v2": { "timeline": { "instructions": [
            { "type": "TimelineAddEntries", "entries": [
                { "entryId": "tweet-1", "content": {
                    "entryType": "TimelineTimelineItem",
                    "itemContent": { "tweet_results": { "result": { "__typename": "TweetTombstone" } } }
                }},
                { "entryId": "cursor-bottom-1", "content": {
                    "entryType": "TimelineTimelineCursor", "cursorType": "Bottom", "value": "CURSOR_2"
                }}
            ]}
        ]}}}
    });
    let page = parse_timeline_page(&body).unwrap();
    assert_eq!(page.items.len(), 0);
    assert_eq!(page.entry_count, 1, "墓碑也算条目");
    assert!(!should_stop_paging(&page, "CURSOR_1"));
}
