//! 知乎收藏导入（**只读**）。
//!
//! 当前有两条通路：
//!
//! 1. **开放平台官方接口**（`developer.zhihu.com`，`fav_import_zhihu` 在用）：
//!    鉴权只是 `Bearer <AccessSecret>` + 时间戳，稳定、无逆向。
//!    ⚠️ 但接口说明原文是「获取指定收藏夹中的**公开内容**」——实测 2025 条的收藏夹
//!    只返回 159 条，失效/非公开内容被服务端过滤，`Paging.Totals` 也是过滤后的口径。
//!
//! 2. **Cookie 直连**（`fav_zhihu_probe` 实验）：用户粘贴浏览器 Cookie（z_c0 登录态 +
//!    d_c0），Rust 侧直接把它放进请求头打 `www.zhihu.com/api/v4/*`，目标是拿到全量收藏。
//!    为什么不移植 zse96 签名：zhihu-plus-plus 是 AGPL-3.0，代码进 Kira 会传染整个应用。
//!
//!    **已确认（2026-09-23）**：`/api/v4/me` 用 Cookie 裸请求返回 200 并带回真实账号信息，
//!    即该接口**不校验** `x-zse-96`。知乎的签名校验是**按接口**的，所以探测逐个打
//!    `/me` → `/collections` → `/collections/{id}/items`，看断在哪一步，见 [`probe_verdict`]。
//!
//!    ⚠️ 踩过的两个坑（都有测试钉住）：
//!    - 用隐藏 WebView 注入 Cookie 那条路**建立不了登录态**（首页仍跳 `/signin`），
//!      于是拿到 401 后被误判成「签名拦截」——那是游客在打登录态接口，说明不了签名的事。
//!      该实现已删除。
//!    - 结论必须**先判登录态再判签名**，否则同样的 401 会被解释成完全相反的两件事。

use serde_json::{json, Value};

use super::db::NewFavorite;

/// 数据源标识（写入 `favorite.source`）
pub const SOURCE: &str = "zhihu";

/// 凭证键：官方接口的 Access Secret。
pub const SECRET_KEY: &str = "zhihu";

/// 凭证键：实验路线的 Cookie（**与上面那个是不同槽位**，互不覆盖）。
///
/// 之前实验代码直接复用 `zhihu` 槽位存 Cookie，会把用户配好的 Access Secret 顶掉。
pub const COOKIE_KEY: &str = "zhihu-cookie";

pub const BASE_URL: &str = "https://developer.zhihu.com";

/// 每页条数（官方默认 20；文档未给出上限，用保守值）
pub const PAGE_SIZE: usize = 20;

/// 页与页之间的间隔（毫秒）：避免连发请求触发风控。
pub const PAGE_DELAY_MS: u64 = 300;

/// 收藏夹列表请求的 Limit（接口无分页字段，一次尽量多取）
pub const FAVLISTS_LIMIT: usize = 100;

// ─── 开放平台官方接口 ───

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

/// 额度查询（官方文档：不消耗业务额度）。
pub fn quota_path() -> String {
    "/api/v1/quota?APIIDs=user_data".to_string()
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
            "（今日用户数据额度已用完：按自然日配额，剩余与总额度见开放平台「各接口剩余配额」面板，次日恢复）"
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

/// 从内容页响应里取（条目数组，是否结束，下一个 Offset，服务端报告的总数）。
///
/// `NextOffset` 文档说是 String，但按防御性处理：字符串和数字都接受——
/// 类型对不上时宁可少翻一页，也不能在这里 panic 或死循环。
/// `Totals` 是**服务端认为**的该收藏夹总条数（公开范围口径），用于进度百分比，
/// 也能回答「到底是接口截断还是本来就这么多」。
pub fn parse_contents_page(payload: &Value) -> (Vec<Value>, bool, usize, Option<i64>) {
    let items = payload
        .get("Items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let is_end = payload
        .pointer("/Paging/IsEnd")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let next_raw = payload.pointer("/Paging/NextOffset");
    let next_offset = match next_raw {
        Some(Value::String(s)) => s.parse::<usize>().ok(),
        Some(Value::Number(n)) => n.as_u64().map(|n| n as usize),
        _ => None,
    };
    let totals = payload.pointer("/Paging/Totals").and_then(|v| v.as_i64());
    (items, is_end, next_offset.unwrap_or(usize::MAX), totals)
}

/// 从额度响应里取（剩余，总额度）；字段缺失/格式不符返回 None（不让预检阻塞导入）。
pub fn parse_quota(payload: &Value) -> Option<(i64, i64)> {
    let value = payload
        .get("Data")
        .or_else(|| payload.get("data"))
        .unwrap_or(payload);
    let remaining = ["RemainingQuota", "remaining_quota"]
        .iter()
        .find_map(|key| value.get(key).and_then(|v| v.as_i64()))?;
    let total = ["TotalQuota", "total_quota"]
        .iter()
        .find_map(|key| value.get(key).and_then(|v| v.as_i64()))
        .unwrap_or(-1);
    Some((remaining, total))
}

// ─── Cookie 直连（实验路线） ───

/// 探测用的桌面 UA：知乎对明显非浏览器的 UA 会直接 403，
/// 这里只是让请求看起来像个普通浏览器，不做任何指纹伪装。
const DESKTOP_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";

/// 解析用户粘贴的 Cookie 串（支持 `a=b; c=d` 与每行一对，容忍换行）。
///
/// 同名取第一个（浏览器里同名 Cookie 本来就不该出现两次）；无效段跳过。
pub fn parse_cookie_pairs(text: &str) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = Vec::new();
    for segment in text.split([';', '\n']) {
        let segment = segment.trim();
        let Some((name, value)) = segment.split_once('=') else {
            continue;
        };
        let name = name.trim();
        let value = value.trim();
        if name.is_empty() || value.is_empty() {
            continue;
        }
        if !pairs.iter().any(|(n, _)| n == name) {
            pairs.push((name.to_string(), value.to_string()));
        }
    }
    pairs
}

/// 带浏览器风味的 API GET（Cookie 原样放进请求头）。
///
/// 名字不带 `zhihu_` 前缀是为了和 `commands.rs` 里那个「官方开放平台」的 `zhihu_get`
/// 区分开：这个走 Cookie，那个走 Access Secret。
///
/// 不走隐藏 WebView 那条老路：实测把 Cookie 注入 WebView 后知乎首页照样跳 `/signin`，
/// 登录态根本没建立（Cookie 存储、domain、重载时机全是变量）。直连只有一种解释——
/// 请求头里就是你粘贴的那串。
async fn cookie_get(cookie: &str, url: &str, referer: &str) -> Result<(u16, String), String> {
    let resp = crate::commands::utils::get_http_client()
        .get(url)
        .header("Cookie", cookie)
        .header("User-Agent", DESKTOP_UA)
        .header("Referer", referer)
        .header("Accept", "application/json, text/plain, */*")
        .header("Accept-Language", "zh-CN,zh;q=0.9")
        .header("x-requested-with", "fetch")
        .send()
        .await
        .map_err(|e| format!("直连知乎失败: {}", e))?;
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    Ok((status, body))
}

/// 探测「我的收藏夹列表」返回体里的第一个收藏夹 id。
///
/// 形状是 `{"data":[{"id":580815780,"title":"…"}],"paging":{…}}`；
/// 拿不到就返回 None（用户可能一个收藏夹都没有，此时 items 无法验证）。
pub fn first_collection_id(body: &str) -> Option<i64> {
    let value: Value = serde_json::from_str(body).ok()?;
    value
        .get("data")
        .and_then(|v| v.as_array())
        .and_then(|list| list.first())
        .and_then(|item| item.get("id"))
        .and_then(|id| id.as_i64())
}

/// 实验主流程：**依次打三个接口**，验证 Cookie 裸请求能走多远。
///
/// 顺序即依赖：`/me` 验登录态 → `/collections` 列收藏夹 → 取第一个 id 打
/// `/collections/{id}/items`（这才是全量导入真正要用的接口）。
///
/// 为什么不测「裸 fetch 整体可行性」而是逐接口测：知乎的 `x-zse-96` 签名是**按接口**
/// 校验的（实测 `/me` 不要签名），只测一个接口就下结论会误判整条路线。
pub async fn probe_cookie(cookie_text: &str) -> Result<String, String> {
    let pairs = parse_cookie_pairs(cookie_text);
    if pairs.is_empty() {
        return Err("Cookie 为空或格式无法解析（需要含 z_c0 与 d_c0）".to_string());
    }
    let names: Vec<&str> = pairs.iter().map(|(n, _)| n.as_str()).collect();
    for required in ["z_c0", "d_c0"] {
        if !names.contains(&required) {
            return Err(format!(
                "Cookie 里缺少 {}（必需）。已识别的键: {}",
                required,
                names.join(", ")
            ));
        }
    }

    // ① 登录态
    let (me_status, me_body) = cookie_get(
        cookie_text,
        "https://www.zhihu.com/api/v4/me",
        "https://www.zhihu.com/",
    )
    .await?;
    let me_snippet: String = me_body.chars().take(300).collect();
    crate::exit_log!(
        "[收藏-知乎] 实验① /api/v4/me -> status={} body={}",
        me_status,
        me_snippet
    );

    // ② 收藏夹列表
    let (list_status, list_body) = cookie_get(
        cookie_text,
        "https://www.zhihu.com/api/v4/collections?offset=0&limit=20",
        "https://www.zhihu.com/collections",
    )
    .await?;
    let list_snippet: String = list_body.chars().take(300).collect();
    crate::exit_log!(
        "[收藏-知乎] 实验② /api/v4/collections -> status={} body={}",
        list_status,
        list_snippet
    );

    // ③ 收藏夹内容（真正要用的那个接口）
    let first_id = first_collection_id(&list_body);
    let (items_status, items_path, items_snippet) = match first_id {
        Some(id) => {
            let path = format!("/api/v4/collections/{}/items?offset=0&limit=20", id);
            let (status, body) = cookie_get(
                cookie_text,
                &format!("https://www.zhihu.com{}", path),
                &format!("https://www.zhihu.com/collection/{}", id),
            )
            .await?;
            let snippet: String = body.chars().take(300).collect();
            crate::exit_log!(
                "[收藏-知乎] 实验③ {} -> status={} body={}",
                path,
                status,
                snippet
            );
            (Some(status), path, snippet)
        }
        None => {
            crate::exit_log!("[收藏-知乎] 实验③ 跳过：列表里没解析出收藏夹 id");
            (None, String::new(), String::new())
        }
    };

    let verdict = probe_verdict(me_status, list_status, items_status);
    Ok(json!({
        "verdict": verdict,
        "conclusion": probe_conclusion(verdict),
        "cookie": { "count": pairs.len() },
        "steps": {
            "me": { "status": me_status, "body": me_snippet },
            "collections": { "status": list_status, "body": list_snippet },
            "items": { "status": items_status, "path": items_path, "body": items_snippet },
        },
    })
    .to_string())
}

/// 探测结论的机器可读判定（前端靠它决定 toast 是成功还是失败）。
///
/// 抽成纯函数是因为「哪种状态算哪种结论」正是前两次误判的地方，必须有测试钉住。
///
/// `items` 为 None 表示列表里没解析出收藏夹 id（用户没有收藏夹），此时无法验证
/// 内容接口，按「已通过的部分」给结论而不是拦下来。
pub fn probe_verdict(me: u16, collections: u16, items: Option<u16>) -> &'static str {
    let ok = |status: u16| (200..300).contains(&status);
    let rejected = |status: u16| status == 401 || status == 403;

    // 登录态都没过 → Cookie 本身的问题，别往签名上扯
    if rejected(me) {
        return "invalid_cookie";
    }
    if !ok(me) {
        return "unknown";
    }
    // 已登录，但数据接口被拒 → 这才是签名拦截
    if rejected(collections) || items.map(rejected).unwrap_or(false) {
        return "needs_signature";
    }
    if ok(collections) && items.map(ok).unwrap_or(true) {
        return "ok";
    }
    "unknown"
}

/// [`probe_verdict`] 对应的人话解释。
pub fn probe_conclusion(verdict: &str) -> &'static str {
    match verdict {
        "ok" => "Cookie 直连可用（/me → /collections → /collections/{id}/items 全部通过）：可以切到 Cookie 路线做全量导入",
        "invalid_cookie" => "登录态没过：/api/v4/me 被拒。多半是 z_c0 失效或复制不完整，请在浏览器确认已登录后重新复制整条 Cookie 再测。注意：这不能证明签名是必需的。",
        "needs_signature" => "登录态正常，但收藏接口被拒：那几个接口确实要 x-zse-96 签名，Cookie 方案证伪",
        _ => "未知结果，请把完整输出发给开发者",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        auth_headers, contents_path, favlists_path, first_collection_id, item_to_favorite,
        map_api_error, parse_contents_page, parse_cookie_pairs, parse_envelope, parse_quota,
        probe_conclusion, probe_verdict, FAVLISTS_LIMIT, PAGE_SIZE,
    };
    use serde_json::json;

    /// **误判回归**：登录态没过时的 401 绝不能判成「需要签名」。
    /// 第一次实验就是这样错的（首页跳 /signin，那是游客在打登录态接口）。
    #[test]
    fn anonymous_401_is_not_a_signature_verdict() {
        assert_eq!(probe_verdict(401, 401, Some(401)), "invalid_cookie");
        assert!(probe_conclusion("invalid_cookie").contains("不能证明签名"));
    }

    /// 已登录但数据接口被拒 → 这才是签名问题（实测 /me 200 而收藏接口被拒的情形）。
    #[test]
    fn signed_in_but_rejected_means_signature() {
        assert_eq!(probe_verdict(200, 403, Some(403)), "needs_signature");
        assert_eq!(probe_verdict(200, 401, None), "needs_signature");
        assert!(probe_conclusion("needs_signature").contains("x-zse-96"));
    }

    /// 三个接口全通 → Cookie 直连可用（2026-09-23 实测 /me 就是这样的）。
    #[test]
    fn all_three_steps_passing_means_ok() {
        assert_eq!(probe_verdict(200, 200, Some(200)), "ok");
        assert!(probe_conclusion("ok").contains("全量导入"));
    }

    /// 用户一个收藏夹都没有时 items 无从验证，不能因此判失败。
    #[test]
    fn missing_collection_id_does_not_fail_the_probe() {
        assert_eq!(probe_verdict(200, 200, None), "ok");
    }

    /// 服务端 5xx 是「看不懂」，不能假装是签名或 Cookie 的问题。
    #[test]
    fn server_error_stays_unknown() {
        assert_eq!(probe_verdict(200, 200, Some(500)), "unknown");
        assert_eq!(probe_verdict(500, 200, Some(200)), "unknown");
    }

    /// 收藏夹列表解析：取第一个 id；结构不对/空列表都不 panic。
    #[test]
    fn first_collection_id_reads_data_array() {
        let body = r#"{"data":[{"id":580815780,"title":"默认收藏夹"}],"paging":{"is_end":false}}"#;
        assert_eq!(first_collection_id(body), Some(580815780));
        assert_eq!(first_collection_id(r#"{"data":[]}"#), None);
        assert_eq!(first_collection_id(r#"{"error":{"code":100}}"#), None);
        assert_eq!(first_collection_id("not json"), None);
    }


    /// Cookie 串解析：分号/换行都行、同名去重、无效段跳过。
    #[test]
    fn cookie_pairs_parse_tolerantly() {
        let pairs = parse_cookie_pairs("z_c0=abc; d_c0=def\n  bogus  \nother=1; z_c0=dup");
        assert_eq!(pairs.len(), 3, "实际: {:?}", pairs);
        assert!(
            pairs.contains(&("z_c0".to_string(), "abc".to_string())),
            "同名取第一个"
        );
        assert!(pairs.contains(&("d_c0".to_string(), "def".to_string())));
        assert!(pairs.contains(&("other".to_string(), "1".to_string())));

        // 没有等号的段跳过；首尾空白容忍
        assert!(parse_cookie_pairs("no-equals-sign").is_empty());
        assert_eq!(
            parse_cookie_pairs("  a = b  ")
                .first()
                .map(|(n, v)| (n.as_str(), v.as_str())),
            Some(("a", "b"))
        );
    }

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
        let (items, is_end, next, totals) = parse_contents_page(&payload);
        assert_eq!(items.len(), 1);
        assert!(!is_end);
        assert_eq!(next, 40);
        assert_eq!(totals, Some(100));

        let ended = json!({ "Items": [], "Paging": { "IsEnd": true } });
        let (items, is_end, next, totals) = parse_contents_page(&ended);
        assert!(items.is_empty() && is_end);
        // 没给 NextOffset 时返回哨兵值，调用方按「结束」处理
        assert_eq!(next, usize::MAX);
        assert_eq!(totals, None);
    }

    /// NextOffset 实际返回数字时也要能解析（文档写 String，但按防御性处理）。
    #[test]
    fn next_offset_as_number_is_accepted() {
        let payload = json!({
            "Items": [],
            "Paging": { "IsEnd": false, "NextOffset": 60 }
        });
        let (_, is_end, next, _) = parse_contents_page(&payload);
        assert!(!is_end);
        assert_eq!(next, 60);
    }

    /// 额度解析：PascalCase 与小写都认；缺字段返回 None（预检失败不阻塞导入）。
    #[test]
    fn quota_handles_both_casings_and_missing() {
        assert_eq!(parse_quota(&json!({"Data": {"RemainingQuota": 7}})), Some((7, -1)));
        assert_eq!(
            parse_quota(&json!({"data": {"remaining_quota": 0, "total_quota": 10000}})),
            Some((0, 10000))
        );
        assert_eq!(parse_quota(&json!({})), None);
    }
}
