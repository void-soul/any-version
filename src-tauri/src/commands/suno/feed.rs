use super::*;

// ─── 分页：把主页的全部歌曲拉完 ───

/// 翻页接口。主页 SSR 只内嵌前 ~20 首，其余要靠它按 cursor 拉。
///
/// 实测要点（都是踩出来的）：
/// - 域名是 `studio-api-prod`（连字符），不是 `studio-api.prod`；
/// - 方法 POST，路径 `/api/unified/feed`（**不是** `/api/feed/v3`，那个匿名一律 401）；
/// - 匿名可用，**不要带 cookie**（前端就是 `credentials: "omit"`）。
pub(super) const UNIFIED_FEED_URL: &str = "https://studio-api-prod.suno.com/api/unified/feed";
/// 每页条数（接口上限就是 20）。
pub(super) const PAGE_SIZE: usize = 20;
/// 总量兜底上限：正常主页几百首够用，防止接口返回异常 next_cursor 导致死循环。
pub(super) const PAGE_MAX_ITEMS: usize = 2000;

/// 从主页 HTML 里取目标用户 id（翻页接口的 `target_user_id`）。
///
/// 必须用 `"v2Data":{"user_id":"` 锚定：页面里有二十多处 `user_id`（歌曲作者、点赞者…），
/// 取第一个会拿到错的用户。实测这个锚点反转义后唯一命中。
pub(super) fn extract_target_user_id(html: &str) -> Option<String> {
    let unescaped = html.replace("\\\"", "\"");
    let anchor = "\"v2Data\":{\"user_id\":\"";
    let start = unescaped.find(anchor)? + anchor.len();
    let rest = unescaped.get(start..)?;
    let end = rest.find('"')?;
    let id = rest.get(..end)?;
    // 保守校验：只接受 UUID 形态，避免锚点附近结构变化时抓出半截字符串
    (id.len() == 36 && id.chars().filter(|c| *c == '-').count() == 4).then(|| id.to_string())
}

/// `browser-token`：base64url(`{"timestamp":<毫秒>}`)，无 padding。
///
/// 这就是浏览器里那串 token 的**全部**内容——不含任何身份信息，纯粹是个时间戳凭证，
/// 所以匿名也能过（实测 200）。别以为它缺了什么登录态而去加 Cookie。
pub(super) fn browser_token() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let payload = format!("{{\"timestamp\":{millis}}}");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload)
}

/// `device-id`：随机 UUID v4（本仓库没有 uuid crate，用 getrandom 自己拼）。
pub(super) fn new_uuid_v4() -> Result<String, String> {
    let mut b = [0u8; 16];
    getrandom::getrandom(&mut b).map_err(|e| format!("生成 device-id 失败: {e}"))?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    Ok(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    ))
}

/// 拉一页，返回 `(本页歌曲, 下一页游标)`。游标为空串表示到底了。
pub(super) async fn fetch_feed_page(
    client: &reqwest::Client,
    target_user_id: &str,
    device_id: &str,
    cursor: &str,
) -> Result<(Vec<SunoSong>, String), String> {
    let body = serde_json::json!({
        "feed_id": "user_songs",
        "cursor": cursor,
        "page_size": PAGE_SIZE,
        "request_metadata": { "sort_by": "created_at" },
        "target_user_id": target_user_id,
    });
    let resp = client
        .post(UNIFIED_FEED_URL)
        .header("accept", "*/*")
        // token 要手工拼成 {"token":"..."} 这个 JSON 字符串，不是裸 token
        .header(
            "browser-token",
            format!("{{\"token\":\"{token}\"}}", token = browser_token()),
        )
        .header("device-id", device_id)
        .header("referer", "https://suno.com/")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("拉取第 {cursor} 页失败: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        // 游标越过末页时接口回 404，属于正常终止而不是错误
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok((Vec::new(), String::new()));
        }
        return Err(format!("拉取第 {cursor} 页返回 HTTP {status}"));
    }
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("解析第 {cursor} 页失败: {e}"))?;

    let next_cursor = json
        .get("feed")
        .and_then(|f| f.get("next_cursor"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // items[].content_item 与主页 SSR 里的完全同构，直接复用同一个解析函数
    let songs = json
        .get("feed")
        .and_then(|f| f.get("items"))
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("content_item"))
                .filter_map(parse_content_item)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok((songs, next_cursor))
}

/// 逐页拉完整个主页（去重）。`Err` 只在第一页就失败时返回——翻页中途断掉就把已有的交出去。
pub(super) async fn fetch_all_songs(
    client: &reqwest::Client,
    target_user_id: &str,
) -> Result<Vec<SunoSong>, String> {
    let device_id = new_uuid_v4()?;
    let mut all: Vec<SunoSong> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut cursor = String::new();

    loop {
        let (songs, next) = fetch_feed_page(client, target_user_id, &device_id, &cursor).await?;
        for song in songs {
            if seen.insert(song.id.clone()) {
                all.push(song);
            }
        }
        if next.is_empty() || all.len() >= PAGE_MAX_ITEMS {
            break;
        }
        cursor = next;
    }
    Ok(all)
}
