//! 知乎收藏导入（**只读**）。
//!
//! 当前有两条通路：
//!
//! 1. **开放平台官方接口**（`developer.zhihu.com`，`fav_import_zhihu` 在用）：
//!    鉴权只是 `Bearer <AccessSecret>` + 时间戳，稳定、无逆向。
//!    ⚠️ 但接口说明原文是「获取指定收藏夹中的**公开内容**」——实测 2025 条的收藏夹
//!    只返回 159 条，失效/非公开内容被服务端过滤，`Paging.Totals` 也是过滤后的口径。
//!
//! 2. **Cookie + 隐藏 WebView**（`fav_zhihu_probe` 实验，待验证）：用户粘贴浏览器
//!    Cookie（z_c0 登录态 + d_c0），用 `WebviewWindow::set_cookie` 注入隐藏 WebView，
//!    在知乎页面上下文里 fetch `www.zhihu.com/api/v4/*`。目标是拿到全量收藏。
//!    为什么不移植 zse96 签名：zhihu-plus-plus 是 AGPL-3.0，代码进 Kira 会传染整个应用。
//!    实验要验证的假设：页面上下文里的裸 `fetch`（不带签名头）是否被 v4 接口放行。

use serde_json::{json, Value};

use super::db::NewFavorite;

/// 数据源标识（写入 `favorite.source`）
pub const SOURCE: &str = "zhihu";

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

// ─── Cookie + 隐藏 WebView（实验路线） ───

/// 隐藏窗口 label：复用同一个（Cookie 与页面状态都在它身上）。
pub const VIEW_LABEL: &str = "favorites-zhihu-view";

/// 宿主页（同源才能带 Cookie 调 API）
pub const HOME_URL: &str = "https://www.zhihu.com/";

/// 页面上下文里取回结果的全局变量名
pub const RESULT_VAR: &str = "__favZhihuFetch";

/// 取回结果的最长等待。
const FETCH_TIMEOUT_SECS: u64 = 20;
/// 首次打开页面等待 complete 的预算。
const READY_TIMEOUT_SECS: u64 = 30;

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

/// 构造「发起请求」的 JS（URL 经 JSON 转义注入，结果写进全局变量供轮询）。
pub fn build_fetch_js(url: &str) -> String {
    let literal = serde_json::to_string(url).unwrap_or_else(|_| "\"\"".to_string());
    format!(
        r#"(function () {{
  try {{
    window.{var} = null;
    fetch({url}, {{ credentials: 'include', headers: {{ 'accept': 'application/json, text/plain, */*' }} }})
      .then(function (r) {{ return r.text().then(function (t) {{ return {{ status: r.status, body: t }}; }}); }})
      .then(function (v) {{ window.{var} = JSON.stringify(v); }})
      .catch(function (e) {{ window.{var} = JSON.stringify({{ error: String(e) }}); }});
    return 'started';
  }} catch (e) {{
    window.{var} = JSON.stringify({{ error: String(e) }});
    return 'failed';
  }}
}})()"#,
        var = RESULT_VAR,
        url = literal
    )
}

/// 解析轮询到的结果：剥两层 JSON（eval 回传会多序列化一层），返回 (HTTP 状态, 响应体)。
pub fn parse_poll(raw: &str) -> Result<Option<(u16, String)>, String> {
    let outer: Value =
        serde_json::from_str(raw.trim()).map_err(|e| format!("读取知乎响应失败: {}", e))?;
    let inner_text = match outer {
        Value::Null => return Ok(None),
        Value::String(s) => s,
        other => other.to_string(),
    };
    let payload: Value =
        serde_json::from_str(&inner_text).map_err(|e| format!("解析知乎响应失败: {}", e))?;
    if let Some(err) = payload.get("error").and_then(|v| v.as_str()) {
        return Err(format!("知乎页面内请求失败: {}", err));
    }
    let status = payload.get("status").and_then(|v| v.as_u64()).unwrap_or(0) as u16;
    let body = payload
        .get("body")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    Ok(Some((status, body)))
}

static WEBVIEW_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 确保隐藏窗口存在（宿主页 = 知乎首页，同源 + Cookie）。
fn ensure_view(app: &tauri::AppHandle) -> Result<tauri::WebviewWindow, String> {
    use tauri::Manager;

    if let Some(window) = app.get_webview_window(VIEW_LABEL) {
        return Ok(window);
    }
    let url = tauri::WebviewUrl::External(
        HOME_URL
            .parse()
            .map_err(|e| format!("知乎地址解析失败: {}", e))?,
    );
    tauri::WebviewWindowBuilder::new(app, VIEW_LABEL, url)
        .title("知乎")
        .inner_size(1280.0, 820.0)
        .visible(false)
        .skip_taskbar(true)
        .on_page_load(|_, payload| {
            crate::exit_log!("[收藏-知乎] 页面事件: {:?} {}", payload.event(), payload.url());
        })
        .build()
        .map_err(|e| format!("创建知乎窗口失败: {}", e))
}

/// 轮询 document.readyState 直到 complete。
async fn wait_ready(window: &tauri::WebviewWindow) -> bool {
    use tokio::sync::oneshot;

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(READY_TIMEOUT_SECS);
    loop {
        let (tx, rx) = oneshot::channel::<String>();
        let slot = std::sync::Arc::new(std::sync::Mutex::new(Some(tx)));
        let slot2 = slot.clone();
        if window
            .eval_with_callback("document.readyState", move |s| {
                if let Some(t) = slot2.lock().unwrap().take() {
                    let _ = t.send(s);
                }
            })
            .is_err()
        {
            return false;
        }
        match tokio::time::timeout(std::time::Duration::from_secs(3), rx).await {
            Ok(Ok(state)) if state.contains("complete") => return true,
            _ => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
}

/// 在知乎页面上下文里 GET 一个 API 路径，返回 (HTTP 状态, 响应体)。
async fn fetch_api(
    app: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
    path: &str,
) -> Result<(u16, String), String> {
    use tokio::sync::oneshot;

    let url = format!("https://www.zhihu.com{}", path);
    let poll_var = format!("window.{}", RESULT_VAR);
    window
        .eval(build_fetch_js(&url))
        .map_err(|e| format!("在知乎页面里发起请求失败: {}", e))?;

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(FETCH_TIMEOUT_SECS);
    loop {
        let (tx, rx) = oneshot::channel::<String>();
        let slot = std::sync::Arc::new(std::sync::Mutex::new(Some(tx)));
        let slot2 = slot.clone();
        if window
            .eval_with_callback(poll_var.clone(), move |s| {
                if let Some(t) = slot2.lock().unwrap().take() {
                    let _ = t.send(s);
                }
            })
            .is_err()
        {
            return Err("读取知乎响应失败（窗口可能已被关闭）".to_string());
        }
        if let Ok(Ok(raw)) = tokio::time::timeout(std::time::Duration::from_secs(3), rx).await {
            if let Some((status, body)) = parse_poll(&raw)? {
                return Ok((status, body));
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("知乎请求超时".to_string());
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

/// 实验主流程：注入 Cookie → 重载页面 → 请求 `/api/v4/me`。
///
/// 返回一段 JSON 文本（状态码 + 响应体片段），直接给用户看实验结论：
/// - 200 且带用户信息 → Cookie 路线成立（裸 fetch 不被签名校验拦截）
/// - 401/403 → v4 接口强制签名，Cookie 方案证伪
pub async fn probe_with_cookie(app: &tauri::AppHandle, cookie_text: &str) -> Result<String, String> {
    use tauri::webview::cookie::Cookie as TauriCookie;

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

    let _guard = WEBVIEW_LOCK.lock().await;
    let window = ensure_view(app)?;
    if !wait_ready(&window).await {
        let _ = window.destroy();
        return Err("知乎页面加载超时（窗口已重置，请再试一次）".to_string());
    }

    // 注入 Cookie：domain 用 .zhihu.com（与浏览器里 z_c0 的域一致）
    for (name, value) in &pairs {
        let cookie = TauriCookie::build((name.as_str(), value.as_str()))
            .domain(".zhihu.com")
            .path("/")
            .secure(true)
            .http_only(false)
            .same_site(tauri::webview::cookie::SameSite::Lax)
            .build();
        if let Err(e) = window.set_cookie(cookie) {
            return Err(format!("注入 Cookie 失败（{}）: {}", name, e));
        }
    }
    crate::exit_log!("[收藏-知乎] 已注入 {} 个 Cookie，重载页面", pairs.len());

    // 重载让 Cookie 生效，再等一次 complete
    window
        .eval("location.reload()")
        .map_err(|e| format!("重载知乎页面失败: {}", e))?;
    if !wait_ready(&window).await {
        return Err("注入 Cookie 后页面加载超时".to_string());
    }
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

    // 实验核心：裸 fetch（不带任何签名头）能否通过 v4 接口
    let (status, body) = fetch_api(app, &window, "/api/v4/me").await?;
    let snippet: String = body.chars().take(500).collect();
    crate::exit_log!(
        "[收藏-知乎] 实验 /api/v4/me -> status={} body={}",
        status,
        snippet
    );

    Ok(json!({
        "status": status,
        "conclusion": if (200..300).contains(&status) {
            "Cookie 路线成立：裸 fetch 未被签名校验拦截，可以切到全量导入"
        } else if status == 401 || status == 403 {
            "Cookie 路线证伪：v4 接口强制 x-zse-96 签名，裸 fetch 被拒"
        } else {
            "未知结果，请把完整输出发给开发者"
        },
        "body": snippet,
    })
    .to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        auth_headers, contents_path, favlists_path, item_to_favorite, map_api_error,
        parse_contents_page, parse_cookie_pairs, parse_envelope, parse_quota, FAVLISTS_LIMIT,
        PAGE_SIZE,
    };
    use serde_json::json;

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
