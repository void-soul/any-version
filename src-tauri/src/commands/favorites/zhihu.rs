//! 知乎收藏导入（**只读**）——在知乎页面上下文里发 API 请求。
//!
//! 为什么不用 HTTP 客户端 + Cookie：知乎要求 `x-zse-96` 签名，
//! 构成是 `"2.0_" + u(md5(x-zse-93 + path + d_c0))`，其中 `u()` 是知乎自己的
//! jsvmp 混淆函数（没有公开算法、随前端版本变化）。纯 Rust 复现既不稳定也不现实。
//!
//! 因此这里复用 [`crate::commands::picky`] 已确立的「隐藏 WebviewWindow + eval」方案：
//! 把请求放进**知乎页面自己的上下文**里发（`fetch` + `credentials: 'include'`），
//! 签名由知乎自己的 JS 完成，我们只把 JSON 取回来。代价是首次要在应用内登录一次知乎。

use serde_json::{json, Value};

use super::db::NewFavorite;

/// 数据源标识（写入 `favorite.source`）
pub const SOURCE: &str = "zhihu";

/// 隐藏窗口 label：**同一个 label 复用**（与 picky 每次销毁不同）——
/// 登录态就活在这个 webview 的 cookie 里，销毁窗口等于把用户登录态丢了。
pub const VIEW_LABEL: &str = "favorites-zhihu-view";

/// 主页（隐藏窗口的宿主页；同源才能带 Cookie 调 API）
pub const HOME_URL: &str = "https://www.zhihu.com/";

/// 登录页（用户点「登录知乎」时打开的可见窗口）
pub const LOGIN_URL: &str = "https://www.zhihu.com/signin";

/// 每页条数：知乎 items 接口 limit 上限是 20。
pub const PAGE_SIZE: usize = 20;

/// 一次导入最多翻多少页。
pub const MAX_PAGES: usize = 500;

/// 页面上下文里取值的全局变量名（`window.__xx`），每次请求前清零。
pub const RESULT_VAR: &str = "__favZhihuFetch";

/// 收藏夹列表（我创建的）。
pub fn collections_path(url_token: &str) -> String {
    format!(
        "/api/v4/members/{}/collections?offset=0&limit=50",
        url_token
    )
}

/// 收藏夹内容（第 offset 条开始的 PAGE_SIZE 条）。
pub fn items_path(collection_id: &str, offset: usize) -> String {
    format!(
        "/api/v4/collections/{}/items?offset={}&limit={}",
        collection_id, offset, PAGE_SIZE
    )
}

/// 构造「发起请求」的 JS。
///
/// URL 用 `serde_json::to_string` 转义后注入，避免路径里出现引号/反斜杠时把脚本拼坏
/// （收藏夹 id 虽然是我们自己拼的，但拼串注入是这类代码最容易出的洞）。
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

/// 解析轮询到的 `window.__favZhihuFetch` 值。
///
/// `eval_with_callback` 会把 JS 字符串再序列化一层（回调收到的是带引号的 JSON 串），
/// 所以要先剥一层再剥一层——picky 里也踩过同样的坑。
pub fn parse_poll(raw: &str) -> Result<Option<Value>, String> {
    let outer: Value = serde_json::from_str(raw.trim())
        .map_err(|e| format!("读取知乎响应失败: {}", e))?;
    let inner_text = match outer {
        Value::Null => return Ok(None),
        Value::String(s) => s,
        other => other.to_string(),
    };
    let payload: Value = serde_json::from_str(&inner_text)
        .map_err(|e| format!("解析知乎响应失败: {}", e))?;
    if let Some(err) = payload.get("error").and_then(|v| v.as_str()) {
        return Err(format!("知乎页面内请求失败: {}", err));
    }
    let status = payload.get("status").and_then(|v| v.as_i64()).unwrap_or(0);
    let body = payload
        .get("body")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if status == 404 {
        return Err("知乎接口返回 404（接口可能已改版）".to_string());
    }
    if status == 403 || status == 401 {
        return Err("知乎拒绝了请求（未登录或账号受限），请先在应用内登录知乎".to_string());
    }
    let parsed: Value =
        serde_json::from_str(body).map_err(|e| format!("解析知乎 JSON 失败: {}", e))?;
    if let Some(error) = parsed.get("error") {
        let message = error
            .pointer("/message")
            .and_then(|v| v.as_str())
            .or_else(|| error.as_str())
            .unwrap_or("未知错误");
        return Err(format!("知乎返回错误: {}", message));
    }
    Ok(Some(parsed))
}

/// 一条收藏项 → 待落库条目。
///
/// 知乎的 items 结构是 `{ content: {...}, created: 时间戳 }`，而 `content` 的类型
/// 有 answer / article / zvideo / pin 等多种，字段各不相同，所以这里逐层兜底取值，
/// 任一项缺失都只影响该条而不是整批。
pub fn item_to_favorite(item: &Value, collection_title: &str) -> Option<NewFavorite> {
    let content = item.get("content").unwrap_or(item);
    let kind = content
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let id = content
        .get("id")
        .and_then(|v| v.as_i64())
        .or_else(|| item.get("id").and_then(|v| v.as_i64()))?;

    let question_title = content
        .pointer("/question/title")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let own_title = content
        .get("title")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let excerpt = content
        .get("excerpt")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let title = question_title
        .clone()
        .or(own_title)
        .or_else(|| excerpt.as_ref().map(|s| s.chars().take(60).collect()))?;

    let question_id = content.pointer("/question/id").and_then(|v| v.as_i64());
    let url = match kind {
        "answer" => match question_id {
            Some(qid) => format!("https://www.zhihu.com/question/{}/answer/{}", qid, id),
            None => format!("https://www.zhihu.com/answer/{}", id),
        },
        "article" => format!("https://zhuanlan.zhihu.com/p/{}", id),
        "zvideo" => format!("https://www.zhihu.com/zvideo/{}", id),
        "pin" => format!("https://www.zhihu.com/pin/{}", id),
        _ => content
            .get("url")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())?,
    };

    let author = content
        .pointer("/author/name")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    Some(NewFavorite {
        source: SOURCE.to_string(),
        // 不同类型可能有相同数字 id，所以把类型拼进去当去重键
        external_id: format!("{}:{}", kind, id),
        url,
        title,
        subtitle: author.or_else(|| Some(collection_title.to_string())),
        description: excerpt,
        extra_json: Some(
            json!({
                "kind": kind,
                "collection": collection_title,
                "created": item.get("created").and_then(|v| v.as_i64())
                    .or_else(|| item.get("created_time").and_then(|v| v.as_i64())),
                "voteup": content.get("voteup_count").and_then(|v| v.as_i64()),
            })
            .to_string(),
        ),
        initial_status: None,
    })
}

/// 隐藏窗口上的操作必须串行：同一时刻只有一个 fetch 在跑，否则全局变量会互相覆盖。
static WEBVIEW_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 取回结果的最长等待（知乎接口偶发慢，比 picky 的 6s 放宽）。
const FETCH_TIMEOUT_SECS: u64 = 20;
/// 首次打开页面等待 `document.readyState === 'complete'` 的预算。
const READY_TIMEOUT_SECS: u64 = 30;

/// 确保隐藏窗口存在（不存在就以知乎主页为宿主建一个）。
///
/// `visible = true` 时顺便显示并聚焦（用于登录）；否则保持隐藏。
fn ensure_view(app: &tauri::AppHandle, visible: bool) -> Result<tauri::WebviewWindow, String> {
    use tauri::Manager;

    if let Some(window) = app.get_webview_window(VIEW_LABEL) {
        if visible {
            let _ = window.show();
            let _ = window.set_focus();
        }
        return Ok(window);
    }

    let url = tauri::WebviewUrl::External(
        HOME_URL
            .parse()
            .map_err(|e| format!("知乎地址解析失败: {}", e))?,
    );
    let window = tauri::WebviewWindowBuilder::new(app, VIEW_LABEL, url)
        .title("知乎")
        .inner_size(1100.0, 800.0)
        .visible(visible)
        .build()
        .map_err(|e| format!("创建知乎窗口失败: {}", e))?;
    Ok(window)
}

/// 轮询 `document.readyState` 直到 complete。
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

/// 在知乎页面上下文里请求一个 API 路径（同源 + 带 Cookie，签名由知乎 JS 完成）。
pub async fn fetch_api(app: &tauri::AppHandle, path: &str) -> Result<Value, String> {
    use tokio::sync::oneshot;

    let _guard = WEBVIEW_LOCK.lock().await;
    let window = ensure_view(app, false)?;
    if !wait_ready(&window).await {
        return Err("知乎页面加载超时，请点「登录知乎」打开窗口重试".to_string());
    }

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
            match parse_poll(&raw)? {
                Some(value) => return Ok(value),
                None => {}
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("知乎请求超时（可能未登录或网络受限）".to_string());
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

/// 打开（或显示）可见的知乎窗口，让用户登录。登录态留在该 webview 的 Cookie 里。
pub fn open_login_window(app: &tauri::AppHandle) -> Result<(), String> {
    use tauri::Manager;

    if let Some(window) = app.get_webview_window(VIEW_LABEL) {
        let _ = window.show();
        let _ = window.set_focus();
        let _ = window.navigate(
            LOGIN_URL
                .parse()
                .map_err(|e| format!("登录地址解析失败: {}", e))?,
        );
        return Ok(());
    }
    let url = tauri::WebviewUrl::External(
        LOGIN_URL
            .parse()
            .map_err(|e| format!("登录地址解析失败: {}", e))?,
    );
    tauri::WebviewWindowBuilder::new(app, VIEW_LABEL, url)
        .title("登录知乎")
        .inner_size(1100.0, 800.0)
        .build()
        .map_err(|e| format!("创建知乎登录窗口失败: {}", e))?;
    Ok(())
}

/// 当前登录用户的 url_token（未登录返回 None）。
pub async fn fetch_url_token(app: &tauri::AppHandle) -> Result<Option<String>, String> {
    let me = match fetch_api(app, "/api/v4/me").await {
        Ok(value) => value,
        // 未登录时接口返回 401/403，这与「接口挂了」是两回事
        Err(e) if e.contains("登录") => return Ok(None),
        Err(e) => return Err(e),
    };
    Ok(me
        .get("url_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::{build_fetch_js, collections_path, item_to_favorite, items_path, parse_poll, RESULT_VAR};
    use serde_json::json;

    #[test]
    fn paths_use_documented_endpoints() {
        assert_eq!(
            items_path("12345", 40),
            "/api/v4/collections/12345/items?offset=40&limit=20"
        );
        assert!(collections_path("someone").contains("/api/v4/members/someone/collections"));
    }

    /// URL 必须被安全转义，不能被拼进脚本体里。
    #[test]
    fn fetch_js_escapes_the_url() {
        let js = build_fetch_js("/api/v4/collections/1/items?offset=0&limit=20");
        assert!(js.contains("credentials: 'include'"), "必须带 Cookie：{}", js);
        assert!(js.contains(&format!("window.{} = null", RESULT_VAR)));
        // 含引号的路径不会把脚本拼破
        let risky = build_fetch_js("/x\";alert(1);//");
        assert!(risky.contains(r#"\";alert(1);//"#), "URL 未转义: {}", risky);
    }

    #[test]
    fn parse_poll_unwraps_double_encoded_payload() {
        let payload = json!({ "status": 200, "body": r#"{"data":[]}"# }).to_string();
        let raw = serde_json::to_string(&payload).unwrap(); // 外层再序列化一次
        let value = parse_poll(&raw).unwrap().unwrap();
        assert_eq!(value["data"], json!([]));
    }

    /// 还没返回时是 null：调用方据此继续轮询。
    #[test]
    fn parse_poll_returns_none_while_pending() {
        assert!(parse_poll("null").unwrap().is_none());
    }

    #[test]
    fn parse_poll_reports_http_and_page_errors() {
        let forbidden = serde_json::to_string(&json!({"status":403,"body":""}).to_string()).unwrap();
        assert!(parse_poll(&forbidden).unwrap_err().contains("登录"));
        let not_found = serde_json::to_string(&json!({"status":404,"body":""}).to_string()).unwrap();
        assert!(parse_poll(&not_found).unwrap_err().contains("404"));
        let page_error = serde_json::to_string(&json!({"error":"TypeError"}).to_string()).unwrap();
        assert!(parse_poll(&page_error).is_err());
        // 知乎自己的业务错误
        let body = json!({ "error": { "message": "未登录" } }).to_string();
        let raw = serde_json::to_string(&json!({"status":200,"body":body}).to_string()).unwrap();
        assert!(parse_poll(&raw).unwrap_err().contains("未登录"));
    }

    #[test]
    fn maps_answer_using_question_title_and_answer_url() {
        let item = json!({
            "content": {
                "type": "answer",
                "id": 52361406,
                "question": { "id": 19550517, "title": "如何评价 X？" },
                "author": { "name": "某人" },
                "excerpt": "摘要内容",
                "voteup_count": 12
            },
            "created": 1500000
        });
        let fav = item_to_favorite(&item, "我的收藏").unwrap();
        assert_eq!(fav.external_id, "answer:52361406", "不同类型可能撞 id，必须带类型");
        assert_eq!(fav.url, "https://www.zhihu.com/question/19550517/answer/52361406");
        assert_eq!(fav.title, "如何评价 X？");
        assert_eq!(fav.subtitle.as_deref(), Some("某人"));
        assert_eq!(fav.description.as_deref(), Some("摘要内容"));
    }

    #[test]
    fn maps_article_and_zvideo_urls() {
        let article = json!({ "content": { "type": "article", "id": 7, "title": "专栏文" } });
        assert_eq!(
            item_to_favorite(&article, "夹").unwrap().url,
            "https://zhuanlan.zhihu.com/p/7"
        );
        let zvideo = json!({ "content": { "type": "zvideo", "id": 8, "title": "视频" } });
        assert_eq!(
            item_to_favorite(&zvideo, "夹").unwrap().url,
            "https://www.zhihu.com/zvideo/8"
        );
    }

    /// 标题缺失时退回摘要前 60 字；连摘要都没有才跳过该条。
    #[test]
    fn falls_back_to_excerpt_then_skips() {
        let long = "字".repeat(120);
        let item = json!({ "content": { "type": "answer", "id": 1, "excerpt": long } });
        let fav = item_to_favorite(&item, "夹").unwrap();
        assert_eq!(fav.title.chars().count(), 60);

        let empty = json!({ "content": { "type": "answer", "id": 1 } });
        assert!(item_to_favorite(&empty, "夹").is_none());
    }
}
