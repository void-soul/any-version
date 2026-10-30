use super::{
    browser_token, extract_target_user_id, is_profile_url, normalize_fetch_url,
    normalize_profile_url, parse_profile_html, sanitize_filename, unique_path,
};
use serde_json::json;

/// 宽松校验通过后，必须补成绝对 URL —— 否则 reqwest 报 relative URL without a base。
#[test]
fn fetch_url_is_always_absolute() {
    assert_eq!(
        normalize_fetch_url("suno.com/@user"),
        "https://suno.com/@user"
    );
    assert_eq!(
        normalize_fetch_url("https://suno.com/@user"),
        "https://suno.com/@user"
    );
    // 已有 http 不改写（保持用户原意）
    assert_eq!(
        normalize_fetch_url("http://suno.com/@user"),
        "http://suno.com/@user"
    );
    // 校验通过的每种写法，归一化后都必须是绝对 URL
    for raw in [
        "https://suno.com/@u",
        "http://suno.com/@u",
        "suno.com/@u",
        "https://www.suno.com/@u",
    ] {
        assert!(is_profile_url(raw), "前提：{raw} 应通过校验");
        let out = normalize_fetch_url(raw);
        assert!(
            out.starts_with("http://") || out.starts_with("https://"),
            "归一化后必须是绝对 URL：{out}"
        );
    }
}

/// 分页凭证：`browser-token` 解开就是一个时间戳，没有身份信息。
#[test]
fn browser_token_is_base64url_timestamp() {
    use base64::Engine;
    let token = browser_token();
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(token.as_bytes())
        .expect("token 必须是合法 base64url");
    let text = String::from_utf8(decoded).expect("解码后应是 UTF-8");
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("应是 JSON");
    assert!(
        parsed.get("timestamp").and_then(|v| v.as_i64()).is_some(),
        "token 里应有 timestamp：{text}"
    );
    // 无 padding
    assert!(!token.contains('='), "不该带 padding：{token}");
}

/// `target_user_id` 只能取 v2Data 块里的那个，不能被其它 user_id 干扰。
#[test]
fn target_user_id_anchors_on_v2data() {
    let html = r#"..."v2Data\":{\"user_id\":\"1498d99f-3aae-4c41-ada4-6642d539a30d\",\"metadata\":{\"handle\":\"echoingpromoter3561"}}"#;
    assert_eq!(
        extract_target_user_id(html).as_deref(),
        Some("1498d99f-3aae-4c41-ada4-6642d539a30d")
    );
    // 前面出现的 song author user_id 不能干扰
    let html2 = r#"\"content_item\":{\"id\":\"x\",\"user_id\":\"00000000-0000-4000-8000-000000000000\"}"v2Data\":{\"user_id\":\"1498d99f-3aae-4c41-ada4-6642d539a30d\"}"#;
    assert_eq!(
        extract_target_user_id(html2).as_deref(),
        Some("1498d99f-3aae-4c41-ada4-6642d539a30d")
    );
    // 抓不到 / 格式异常时返回 None，不硬猜
    assert_eq!(extract_target_user_id("no user here"), None);
    assert_eq!(
        extract_target_user_id(r#"\"v2Data\":{\"user_id\":\"short\"}"#),
        None
    );
}

/// 翻页响应里的 content_item 与主页 SSR 同构，能被同一个解析函数吃下。
#[test]
fn parses_unified_feed_page_shape() {
    let page = json!({
        "feed": {
            "feed_id": "user_songs",
            "items": [
                {"content_type": "clip", "content_item": {
                    "status": "complete", "title": "第一首",
                    "id": "clip-1", "video_url": "https://cdn1.suno.ai/clip-1.mp4",
                    "image_large_url": "https://cdn2.suno.ai/i1.jpeg", "play_count": 7
                }},
                {"content_type": "clip", "content_item": {
                    "status": "streaming", "title": "生成中", "id": "clip-2",
                    "video_url": "https://cdn1.suno.ai/clip-2.mp4"
                }}
            ],
            "next_cursor": "20"
        }
    });
    let items = page["feed"]["items"].as_array().unwrap();
    let songs: Vec<_> = items
        .iter()
        .filter_map(|i| i.get("content_item"))
        .filter_map(super::parse_content_item)
        .collect();
    assert_eq!(songs.len(), 1, "草稿应被过滤");
    assert_eq!(songs[0].title, "第一首");
    assert_eq!(songs[0].audio_url, "https://cdn1.suno.ai/clip-1.mp4");
    assert_eq!(page["feed"]["next_cursor"].as_str(), Some("20"));
}

#[test]
fn profile_url_only_accepts_user_homepage() {
    assert!(is_profile_url("https://suno.com/@echoingpromoter3561"));
    assert!(is_profile_url("http://suno.com/@user"));
    assert!(is_profile_url("suno.com/@user"));
    assert!(is_profile_url("https://www.suno.com/@user"));
    assert!(is_profile_url("https://suno.com/@user/"));
    assert!(!is_profile_url("https://suno.com/song/abc-123"));
    assert!(!is_profile_url("https://suno.com/@"));
    assert!(!is_profile_url("https://example.com/@user"));
    assert!(!is_profile_url("https://suno.com/"));
    assert_eq!(
        normalize_profile_url("https://suno.com/@User/"),
        "https://suno.com/@user"
    );
}

#[test]
fn parses_songs_from_rsc_payload() {
    // 模拟 RSC payload 里的转义片段（两首歌，一首 complete、一首草稿）
    let html = r#"<script>...\"items\":[{\"content_type\":\"clip\",\"content_item\":{\"status\":\"complete\",\"title\":\"我的歌\",\"play_count\":12,\"id\":\"abc-123\",\"entity_type\":\"song_schema\",\"video_url\":\"https://cdn1.suno.ai/abc-123.mp4\",\"media_urls\":[{\"url\":\"https://d2lwuy8qc234o3.cloudfront.net/1/clip/abc-123.m4a\",\"content_type\":\"m4a-opus\"}],\"image_url\":\"https://cdn2.suno.ai/image_abc-123.jpeg\"}},{\"content_type\":\"clip\",\"content_item\":{\"status\":\"streaming\",\"title\":\"生成中\",\"id\":\"draft-1\",\"media_urls\":[{\"url\":\"https://x.m4a\"}]}}]..."#;
    let songs = parse_profile_html(html).unwrap();
    assert_eq!(songs.len(), 1, "草稿应被过滤");
    assert_eq!(songs[0].title, "我的歌");
    assert_eq!(songs[0].id, "abc-123");
    assert_eq!(songs[0].audio_url, "https://cdn1.suno.ai/abc-123.mp4");
    assert_eq!(songs[0].play_count, 12);
    assert!(songs[0]
        .image_url
        .as_deref()
        .unwrap()
        .contains("image_abc-123"));
}

#[test]
fn dedupes_by_song_id() {
    let html = r#"{\"content_item\":{\"status\":\"complete\",\"title\":\"A\",\"id\":\"same-1\",\"video_url\":\"https://x/same-1.mp4\"}}{\"content_item\":{\"status\":\"complete\",\"title\":\"A\",\"id\":\"same-1\",\"video_url\":\"https://x/same-1.mp4\"}}"#;
    let songs = parse_profile_html(html).unwrap();
    assert_eq!(songs.len(), 1);
}

/// 真实结构：外层 `content_item` 是 feed 容器，歌曲嵌套在 `items[].content_item` 里。
/// 解析必须穿透 feed 容器，把内层歌曲都取出来（否则只抓到 feed 外壳、漏掉全部歌曲）。
#[test]
fn parses_songs_nested_in_feed() {
    let html = r#"\"content_item\":{\"feed_id\":\"user_pinned_songs\",\"feed_container_type\":\"synthetic_playlist\",\"feed_title\":\"Pinned\",\"items\":[{\"content_type\":\"clip\",\"content_item\":{\"status\":\"complete\",\"title\":\"我的歌\",\"play_count\":12,\"id\":\"abc-123\",\"video_url\":\"https://cdn1.suno.ai/abc-123.mp4\",\"image_url\":\"https://cdn2.suno.ai/image_abc-123.jpeg\"}},{\"content_type\":\"clip\",\"content_item\":{\"status\":\"complete\",\"title\":\"第二首\",\"id\":\"def-456\",\"video_url\":\"https://cdn1.suno.ai/def-456.mp4\"}}]}"#;
    let songs = parse_profile_html(html).unwrap();
    assert_eq!(songs.len(), 2, "应穿透 feed 容器解析出两首歌");
    assert_eq!(songs[0].title, "我的歌");
    assert_eq!(songs[0].id, "abc-123");
    assert_eq!(songs[1].title, "第二首");
    assert_eq!(songs[1].id, "def-456");
}

#[test]
fn sanitizes_illegal_chars() {
    assert_eq!(
        sanitize_filename("a/b\\c:d*e?f\"g<h>i|j"),
        "a_b_c_d_e_f_g_h_i_j"
    );
    assert_eq!(sanitize_filename("  "), "Untitled");
    assert_eq!(sanitize_filename("标题 123"), "标题 123");
}

/// 重名时自动加 `V2`/`V3`… 后缀（不是资源管理器默认的 ` (1)`），且返回的路径是**原子占位**出来的。
#[test]
fn unique_path_adds_version_suffix() {
    // 目录名带 pid + 纳秒时间戳：测试二进制可能并行/重复运行同一用例，
    // 固定目录名会让彼此踩文件（表现为随机的 remove_file 失败）。
    let dir = temp_test_dir("suno_unique_test");
    let base = dir.join("我的歌.mp3");
    std::fs::write(&base, b"x").unwrap();

    // 原件 → V2
    let v2 = unique_path(base.clone()).unwrap();
    assert_eq!(v2.file_name().unwrap(), "我的歌V2.mp3");
    assert!(v2.exists(), "占位文件必须当场存在");

    // V2 也占用了 → V3
    std::fs::write(&v2, b"x").unwrap();
    let v3 = unique_path(base.clone()).unwrap();
    assert_eq!(v3.file_name().unwrap(), "我的歌V3.mp3");

    // 删掉 V2 后应补回 V2，不跳号留空洞
    std::fs::remove_file(&v2).unwrap();
    assert_eq!(
        unique_path(base.clone()).unwrap().file_name().unwrap(),
        "我的歌V2.mp3"
    );

    // 不存在的路径直接占位，不加工名字
    let fresh = dir.join("新歌.mp3");
    let got = unique_path(fresh.clone()).unwrap();
    assert_eq!(got, fresh);

    // 本身以 V2 结尾的名字不被误判：`DemoV2` → `DemoV2V2`
    let demo = dir.join("DemoV2.mp3");
    std::fs::write(&demo, b"x").unwrap();
    assert_eq!(
        unique_path(demo).unwrap().file_name().unwrap(),
        "DemoV2V2.mp3"
    );

    // 无扩展名时也带序号
    let noext = dir.join("裸文件");
    std::fs::write(&noext, b"x").unwrap();
    assert_eq!(unique_path(noext).unwrap().file_name().unwrap(), "裸文件V2");

    std::fs::remove_dir_all(&dir).unwrap();
}

/// 并发占位必须给出**不同**的名字——这正是改成 `create_new` 要解决的 TOCTOU。
#[test]
fn unique_path_is_race_free() {
    let dir = temp_test_dir("suno_race_test");
    let base = dir.join("同首歌.mp3");
    std::fs::write(&base, b"x").unwrap();

    let handles: Vec<_> = (0..8)
        .map(|_| {
            let base = base.clone();
            std::thread::spawn(move || unique_path(base).unwrap())
        })
        .collect();
    let mut got: Vec<std::path::PathBuf> = handles
        .into_iter()
        .map(|h| h.join().expect("占位不应失败"))
        .collect();

    let total = got.len();
    got.sort();
    got.dedup();
    assert_eq!(got.len(), total, "并发占位出现了重复名字：{got:?}");

    std::fs::remove_dir_all(&dir).unwrap();
}

/// 建一个本用例独占的临时目录（pid + 纳秒，避免并行测试互踩）。
fn temp_test_dir(prefix: &str) -> std::path::PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("{prefix}_{}_{}", std::process::id(), stamp));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
