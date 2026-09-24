//! 知乎收藏导入（**只读**）。
//!
//! 走 **Cookie 直连**：用户粘贴浏览器 Cookie（z_c0 登录态 + d_c0），Rust 侧把它放进请求头
//! 打 `www.zhihu.com/api/v4/*`。为什么不移植 zse96 签名：zhihu-plus-plus 是 AGPL-3.0，
//! 代码进 Kira 会传染整个应用。
//!
//! **已确认（2026-09-23）**：`/me`、`/api/v4/people/{id}/collections`、
//! `/api/v4/collections/{id}/items` 三个接口用 Cookie 裸请求全部 200 —— 知乎的
//! `x-zse-96` 签名是**按接口**校验的（浏览器端统一签名不代表服务端逐个强制），
//! 这三个恰好都不校验。
//!
//! 上线前有两条路：开放平台官方接口（Access Secret + 配额）与 Cookie。**官方路线已删除**：
//! 它标注「获取公开内容」，实测 2032 条的收藏夹只返回 159 条（私有/失效内容被过滤），
//! 拿不到全量就等于没用，而 Cookie 路线一条不少。
//!
//! ⚠️ 踩过的三个坑（都有测试钉住）：
//! - 用隐藏 WebView 注入 Cookie 那条路**建立不了登录态**（首页仍跳 `/signin`），
//!   于是拿到 401 后被误判成「签名拦截」——那是游客在打登录态接口，说明不了签名的事。
//! - 结论必须**先判登录态再判签名**，否则同样的 401 会被解释成完全相反的两件事。
//! - `GET /api/v4/collections` 返回 **405**：那是「创建收藏夹」的 POST 端点，
//!   列表得按 `/people/{id}/collections` 查，而且要用 **id** 不是 `url_token`。

use serde_json::{json, Value};

use super::db::NewFavorite;

/// 数据源标识（写入 `favorite.source`）
pub const SOURCE: &str = "zhihu";

/// 凭证键。
///
/// 保留 `zhihu-cookie` 这个名字而不是改成 `zhihu`：用户已经按这个键存过 Cookie，
/// 改名等于让他们重粘一次，没有任何好处。
pub const COOKIE_KEY: &str = "zhihu-cookie";

/// 每页条数（与浏览器一致；服务端似乎不认更大的 limit，取实测值最稳）
pub const PAGE_SIZE: usize = 20;

/// 页与页之间的间隔（毫秒）。
///
/// 这条路线用的是**用户自己的 Cookie**，请求太密轻则限流重则风控封号，
/// 这个延迟不是性能优化而是账号安全措施，不要为了「快一点」调小。
pub const PAGE_DELAY_MS: u64 = 400;

// ─── Cookie 直连 ───

/// 探测用的桌面 UA：知乎对明显非浏览器的 UA 会直接 403，
/// 这里只是让请求看起来像个普通浏览器，不做任何指纹伪装。
const DESKTOP_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";

/// 带浏览器风味的 API GET（Cookie 原样放进请求头）。
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

/// 知乎站点根地址
const HOST: &str = "https://www.zhihu.com";

/// 按**路径**发请求（导入循环与探测共用）。
///
/// Referer 按目标分类给：知乎对 Referer 有校验，给一个真实存在的同站页面最稳。
pub async fn cookie_get_path(cookie: &str, path: &str) -> Result<(u16, String), String> {
    let referer = if path.starts_with("/api/v4/collections/") {
        "https://www.zhihu.com/collections"
    } else {
        "https://www.zhihu.com/"
    };
    cookie_get(cookie, &format!("{}{}", HOST, path), referer).await
}

/// 从 `/api/v4/me` 的返回体里取用户 id —— 收藏夹列表要按**这个 id** 查，不是 `url_token`。
///
/// 2026-09-23 从浏览器实抓确认真实路径是 `/api/v4/people/{id}/collections`，
/// 其中 `{id}` 是 `56369d958f9395250fec460bcff34da9` 这种 id，
/// 而 `url_token`（`logic-magican-88`）是主页地址用的那串，两者不能混。
pub fn parse_person_id(body: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    let id = value.get("id")?;
    match id {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// 收藏夹列表路径（分页按 `offset`；实测 `paging.next` 也是这个形态）。
pub fn collections_path(person_id: &str, offset: usize) -> String {
    format!(
        "/api/v4/people/{}/collections?offset={}&limit={}",
        person_id, offset, PAGE_SIZE
    )
}

/// 收藏夹内容路径（**全量导入真正要用的那个接口**）。
pub fn cookie_items_path(collection_id: i64, offset: usize) -> String {
    format!(
        "/api/v4/collections/{}/items?offset={}&limit={}",
        collection_id, offset, PAGE_SIZE
    )
}

/// 收藏夹（列表页一项）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CookieCollection {
    pub id: i64,
    pub title: String,
    /// 服务端报告的条数（含非公开内容，官方接口给的是过滤后的数）
    pub item_count: usize,
    pub is_public: bool,
}

/// 解析收藏夹列表页 → (收藏夹, 是否最后一页)。
pub fn parse_collections_page(body: &str) -> Result<(Vec<CookieCollection>, bool), String> {
    let value: Value =
        serde_json::from_str(body).map_err(|e| format!("解析知乎收藏夹列表失败: {}", e))?;
    if let Some(err) = value.get("error") {
        return Err(format!("知乎收藏夹列表返回错误: {}", err));
    }
    let is_end = value
        .get("paging")
        .and_then(|p| p.get("is_end"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let collections = value
        .get("data")
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|item| {
                    let id = item.get("id").and_then(|v| v.as_i64())?;
                    let title = item
                        .get("title")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .unwrap_or("未命名收藏夹")
                        .to_string();
                    let item_count = item
                        .get("item_count")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as usize;
                    let is_public = item
                        .get("is_public")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true);
                    Some(CookieCollection {
                        id,
                        title,
                        item_count,
                        is_public,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok((collections, is_end))
}

/// 去掉 HTML 标签取纯文本（`pin` 的正文就是 HTML，没有 title 字段）。
///
/// 只做标签剥离 + 常见实体反转义 + 空白折叠：目的是给出人可读的一行摘要，
/// 不做完整 HTML 解析（这个模块不需要渲染，多引一个解析库不值）。
pub fn strip_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut tag = String::new();
    let mut in_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' => {
                in_tag = false;
                // `<br/>`、`</p>` 这些在视觉上是分隔，直接删会把前后文字粘成一个词
                // （「世界</p><b>粗」→「世界粗」），所以替换成一个空格。
                if tag_breaks_line(&tag) {
                    out.push(' ');
                }
            }
            _ if in_tag => tag.push(ch),
            _ => out.push(ch),
        }
    }
    let unescaped = out
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&ldquo;", "「")
        .replace("&rdquo;", "」");
    unescaped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 这个标签是否需要替换成空格（块级 / 换行类标签）。
fn tag_breaks_line(tag: &str) -> bool {
    let name = tag
        .trim_start_matches('/')
        .split(|c: char| c.is_whitespace() || c == '/')
        .next()
        .unwrap_or("");
    matches!(
        name.to_ascii_lowercase().as_str(),
        "br" | "p"
            | "div"
            | "li"
            | "ul"
            | "ol"
            | "tr"
            | "td"
            | "th"
            | "table"
            | "hr"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "blockquote"
            | "section"
            | "figure"
            | "figcaption"
    )
}

/// 归一化知乎链接：去掉查询串、锚点与末尾斜杠。
///
/// **存在的唯一理由是跨路线去重**：同一条内容经官方接口与 Cookie 两条路拿到的 URL
/// 会差一些跟踪参数（pin 的 `?native=0`、回答页的各种 `?utm_*`），如果去重键直接用
/// 原始 URL，同一篇内容从两个入口各导一次就会变成两条记录。
/// 归一化后两条路线产出同一个值，第二次导入只会更新、不会新增。
pub fn normalize_zhihu_url(url: &str) -> String {
    let trimmed = url.trim();
    let without_fragment = trimmed.split('#').next().unwrap_or(trimmed);
    without_fragment
        .split('?')
        .next()
        .unwrap_or(without_fragment)
        .trim_end_matches('/')
        .to_string()
}

/// 一条收藏内容 → 统一条目。
///
/// 去重键是 **`{type}:{id}`**：`article` 的 id 与 `answer` 的 id 不在同一命名空间，
/// 不加类型前缀会存在理论上撞车的可能，而幂等是这模块的核心承诺。
///
/// 标题三级回退：`title` → `excerpt_title` → 正文纯文本首段。`pin` 没有 `title`
/// （实测），只靠 `title` 会让所有想法都变成「无标题」。
pub fn item_to_cookie_favorite(
    content: &Value,
    favored_at: Option<i64>,
    collection: &CookieCollection,
) -> Option<NewFavorite> {
    let kind = content.get("type").and_then(|v| v.as_str())?;
    let content_id = match content.get("id")? {
        Value::String(s) if !s.is_empty() => format!("{}:{}", kind, s),
        Value::Number(n) => format!("{}:{}", kind, n),
        _ => return None,
    };
    let url = content
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if url.is_empty() {
        return None;
    }
    // 去重键用归一化 URL（与官方路线一致），内容 id 存进 extra 备查
    let external_id = normalize_zhihu_url(&url);

    let body_text = content
        .get("content")
        .and_then(|v| v.as_str())
        .map(strip_html)
        .unwrap_or_default();
    let excerpt = content
        .get("excerpt_title")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let title = content
        .get("title")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| excerpt.clone())
        .or_else(|| {
            let head = crate::commands::utils::truncate_utf8(&body_text, 120).to_string();
            (!head.is_empty()).then_some(head)
        })
        .unwrap_or_else(|| url.clone());

    let author = content
        .get("author")
        .and_then(|a| a.get("name"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let author_token = content
        .get("author")
        .and_then(|a| a.get("url_token"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let description = {
        let text = crate::commands::utils::truncate_utf8(&body_text, 300).to_string();
        if text.is_empty() {
            excerpt.clone()
        } else {
            Some(text)
        }
    };

    let extra = json!({
        "type": kind,
        "contentId": content_id,
        "author": author,
        "authorToken": author_token,
        "collection": collection.title,
        "collectionId": collection.id,
        "favoredAt": favored_at,
        "isPublic": collection.is_public,
    });

    Some(NewFavorite {
        source: SOURCE.to_string(),
        external_id,
        url,
        title,
        subtitle: author,
        description,
        extra_json: Some(extra.to_string()),
        // 外层 `created` 就是收藏时间（unix 秒）：单独存一列，供按收藏时间排序/过滤
        favorited_at: favored_at.and_then(super::db::unix_to_local_str),
        // 收藏接口返回的条目本身就存在，无需预置失效状态
        initial_status: None,
    })
}

/// 收藏内容页里的一条：待落库条目 + 正文（正文另外存进 `favorite_content`）。
#[derive(Debug, Clone)]
pub struct ZhihuItem {
    pub favorite: NewFavorite,
    /// 正文纯文本（已剥标签，列表展开时直接显示）
    pub text: String,
    /// 正文原始 HTML（留档；前端不直接渲染远端 HTML，避免注入）
    pub html: Option<String>,
}

/// 解析收藏夹内容页 → (条目, 是否最后一页)。
///
/// 正文顺手一起带出来：内容接口本来就把 `content` 全文给了（实测一页 20 条约 366KB），
/// 不缓存等于每次都白拿一遍再丢掉。
pub fn parse_cookie_items_page(
    body: &str,
    collection: &CookieCollection,
) -> Result<(Vec<ZhihuItem>, bool), String> {
    let value: Value =
        serde_json::from_str(body).map_err(|e| format!("解析知乎收藏内容失败: {}", e))?;
    if let Some(err) = value.get("error") {
        return Err(format!("知乎收藏内容返回错误: {}", err));
    }
    let is_end = value
        .get("paging")
        .and_then(|p| p.get("is_end"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let items = value
        .get("data")
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|entry| {
                    // 外层 `created` 是**收藏时间**，内容自己的 `created` 是发布时间
                    let favored_at = entry.get("created").and_then(|v| v.as_i64());
                    let content = entry.get("content")?;
                    let favorite = item_to_cookie_favorite(content, favored_at, collection)?;
                    let html = content
                        .get("content")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .filter(|s| !s.is_empty());
                    let text = html.as_deref().map(strip_html).unwrap_or_default();
                    Some(ZhihuItem {
                        favorite,
                        text,
                        html,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok((items, is_end))
}

#[cfg(test)]
mod tests {
    use super::{
        collections_path, cookie_items_path, item_to_cookie_favorite, normalize_zhihu_url,
        parse_collections_page, parse_cookie_items_page, parse_person_id, strip_html,
        CookieCollection, PAGE_SIZE,
    };
    use serde_json::json;

    /// 测试用收藏夹（映射只用到 title / id / is_public）。
    fn sample_collection() -> CookieCollection {
        CookieCollection {
            id: 580815780,
            title: "20260922".to_string(),
            item_count: 2032,
            is_public: false,
        }
    }

    /// 路径按实抓结果拼：收藏夹列表用 **people id**，内容用收藏夹 id。
    #[test]
    fn paths_match_verified_endpoints() {
        assert_eq!(
            collections_path("56369d958f9395250fec460bcff34da9", 0),
            format!(
                "/api/v4/people/56369d958f9395250fec460bcff34da9/collections?offset=0&limit={}",
                PAGE_SIZE
            )
        );
        assert_eq!(
            cookie_items_path(580815780, 20),
            format!(
                "/api/v4/collections/580815780/items?offset=20&limit={}",
                PAGE_SIZE
            )
        );
    }

    /// /me 返回体里取用户 id（字符串形态）；结构不对不 panic。
    #[test]
    fn parse_person_id_reads_me_response() {
        assert_eq!(
            parse_person_id(r#"{"id":"56369d958f9395250fec460bcff34da9","name":"走刀口"}"#),
            Some("56369d958f9395250fec460bcff34da9".to_string())
        );
        // 数字形态也接受（接口变了不至于直接崩）
        assert_eq!(parse_person_id(r#"{"id":123}"#), Some("123".to_string()));
        assert_eq!(parse_person_id(r#"{"id":""}"#), None);
        assert_eq!(parse_person_id("not json"), None);
    }

    /// 收藏夹列表解析：**私有收藏夹也算**（`is_public=false` 实测就有）。
    #[test]
    fn parse_collections_reads_private_folders() {
        let body = r#"{"paging":{"is_end":true,"totals":2},"data":[
            {"id":580815780,"title":"20260922","is_public":false,"item_count":2032},
            {"id":196117121,"title":"20260921","is_public":false,"item_count":125}]}"#;
        let (collections, is_end) = parse_collections_page(body).unwrap();
        assert!(is_end);
        assert_eq!(collections.len(), 2);
        assert_eq!(collections[0].id, 580815780);
        assert_eq!(collections[0].item_count, 2032);
        assert!(!collections[0].is_public, "私有收藏夹必须保留");
    }

    /// 收藏夹列表返回错误体时要把错误抛出去，而不是当成「0 个收藏夹」静默成功。
    #[test]
    fn parse_collections_reports_api_error() {
        let err = parse_collections_page(r#"{"error":{"code":100,"name":"X"}}"#).unwrap_err();
        assert!(err.contains("错误"), "实际: {}", err);
    }

    /// **实测**：`pin` 没有 `title`，只有 `excerpt_title`；标题必须回退到它，
    /// 否则所有「想法」都会变成无标题条目。
    #[test]
    fn pin_without_title_falls_back_to_excerpt() {
        let collection = sample_collection();
        let content = json!({
            "id": "2077691230142649695", "type": "pin",
            "url": "https://www.zhihu.com/pin/2077691230142649695?native=0",
            "excerpt_title": "它教你从头训一个超小语言模型",
            "content": "<p>正文<strong>加粗</strong>内容</p>",
            "author": { "name": "someone", "url_token": "someone-1" }
        });
        let fav = item_to_cookie_favorite(&content, Some(1790124359), &collection).unwrap();
        assert_eq!(fav.title, "它教你从头训一个超小语言模型");
        assert_eq!(fav.external_id, "https://www.zhihu.com/pin/2077691230142649695");
        assert!(fav.extra_json.as_deref().unwrap().contains("pin:2077691230142649695"));
        assert_eq!(fav.subtitle.as_deref(), Some("someone"));
        assert!(fav.description.as_deref().unwrap().contains("正文加粗内容"));
    }

    /// **跨路线去重**：同一条内容经官方接口与 Cookie 两条路导入，必须得到同一个
    /// `external_id`，否则换个入口再导一次就多一条重复记录。
    /// 同一条内容带不同的跟踪参数，去重键必须一致（否则同一篇会被重复导入）。
    #[test]
    fn dedup_key_ignores_tracking_params() {
        let collection = sample_collection();
        let bare = item_to_cookie_favorite(
            &json!({"id":"1","type":"article","url":"https://zhuanlan.zhihu.com/p/1","title":"a"}),
            None,
            &collection,
        )
        .unwrap();
        let tracked = item_to_cookie_favorite(
            &json!({"id":"1","type":"article",
                    "url":"https://zhuanlan.zhihu.com/p/1?utm_id=0#tip","title":"a"}),
            None,
            &collection,
        )
        .unwrap();
        assert_eq!(bare.external_id, tracked.external_id);
    }

    /// URL 归一化：查询串、锚点、末尾斜杠都去掉，但不同内容仍不同。
    #[test]
    fn normalize_url_strips_params_and_fragment() {
        assert_eq!(
            normalize_zhihu_url("https://www.zhihu.com/pin/123?native=0"),
            "https://www.zhihu.com/pin/123"
        );
        assert_eq!(
            normalize_zhihu_url(" https://zhuanlan.zhihu.com/p/1#tip "),
            "https://zhuanlan.zhihu.com/p/1"
        );
        assert_eq!(
            normalize_zhihu_url("https://www.zhihu.com/question/1/answer/2/"),
            "https://www.zhihu.com/question/1/answer/2"
        );
        assert_ne!(
            normalize_zhihu_url("https://www.zhihu.com/pin/1"),
            normalize_zhihu_url("https://www.zhihu.com/pin/2")
        );
    }

    /// 缺 id / 缺 url 的畸形条目直接跳过，不让它污染库。
    #[test]
    fn malformed_content_is_skipped() {
        let collection = sample_collection();
        assert!(item_to_cookie_favorite(&json!({"type":"pin"}), None, &collection).is_none());
        assert!(item_to_cookie_favorite(
            &json!({"id":"1","type":"pin","url":""}),
            None,
            &collection
        )
        .is_none());
    }

    /// 收藏内容解析：外层 `created` 是**收藏时间**，要写进 extra。
    #[test]
    fn cookie_items_page_reads_shape_and_favored_at() {
        let collection = sample_collection();
        let body = r#"{"paging":{"is_end":false,"totals":2032},"data":[
            {"created":1740533662,"content":{"id":"26425730763","type":"article",
             "title":"标题","url":"https://zhuanlan.zhihu.com/p/26425730763",
             "author":{"name":"游戏茶馆"}}},
            {"created":1790084038,"content":{"id":"999","type":"pin","url":"",
             "excerpt_title":"想法"}}]}"#;
        let (items, is_end) = parse_cookie_items_page(body, &collection).unwrap();
        assert!(!is_end);
        assert_eq!(items.len(), 1, "url 为空的条目要跳过");
        assert!(items[0]
            .favorite
            .extra_json
            .as_deref()
            .unwrap()
            .contains("1740533662"));
    }

    /// 正文要一起带出来（列表展开直接看，不用二次请求）。
    #[test]
    fn items_page_carries_content_text() {
        let collection = sample_collection();
        let body = r#"{"paging":{"is_end":true},"data":[
            {"created":1,"content":{"id":"9","type":"pin",
             "url":"https://www.zhihu.com/pin/9","excerpt_title":"想法",
             "content":"<p>正文<b>加粗</b></p>"}}]}"#;
        let (items, _) = parse_cookie_items_page(body, &collection).unwrap();
        assert_eq!(items[0].text, "正文加粗");
        assert_eq!(items[0].html.as_deref(), Some("<p>正文<b>加粗</b></p>"));
    }

    /// HTML 剥离：标签、实体、连续空白都要处理干净（pin 正文是 HTML）。
    #[test]
    fn strip_html_removes_tags_and_entities() {
        assert_eq!(
            strip_html("<p>你好&nbsp;&amp; 世界</p><br/><b>粗</b>"),
            "你好 & 世界 粗"
        );
        assert_eq!(strip_html("<p class=\"a\"><br/></p>"), "");
        assert_eq!(strip_html("纯文本"), "纯文本");
    }

    /// 空列表 / 结构不同都不 panic（此时没有 id 可取，拉取循环会直接结束）。
    #[test]
    fn parse_collections_tolerates_empty_data() {
        let (empty, is_end) = parse_collections_page(r#"{"data":[],"paging":{"is_end":true}}"#).unwrap();
        assert!(empty.is_empty());
        assert!(is_end);
        let (missing, _) = parse_collections_page(r#"{"paging":{}}"#).unwrap();
        assert!(missing.is_empty());
    }
}
