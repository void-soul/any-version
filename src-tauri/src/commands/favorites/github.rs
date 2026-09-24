//! GitHub star 导入：分页解析与条目映射。
//!
//! 只做**纯计算 + 只读请求**，不调用任何写入接口（本模块不 unstar、不改仓库）。

use base64::Engine;
use serde_json::{json, Value};

use super::db::NewFavorite;

/// 数据源标识（写入 `favorite.source`，**不要随意改**：已落库数据靠它分区）
pub const SOURCE: &str = "github";

/// 从 `Link` 响应头里取下一页 URL。
///
/// GitHub 的分页只靠这个头（响应体里没有总数 / 游标）。格式：
/// `<https://api.github.com/user/1/starred?page=2>; rel="next", <...>; rel="last"`
/// 只要 `rel="next"`；`prev` / `last` / `first` 一律忽略，否则会来回翻页死循环。
pub fn next_page_url(link: Option<&str>) -> Option<String> {
    let link = link?;
    for part in link.split(',') {
        let part = part.trim();
        let Some((url, rest)) = part.split_once(';') else {
            continue;
        };
        let url = url.trim().trim_start_matches('<').trim_end_matches('>');
        if url.is_empty() {
            continue;
        }
        let is_next = rest
            .split(';')
            .filter_map(|kv| kv.split_once('='))
            .any(|(k, v)| {
                k.trim() == "rel" && v.trim().trim_matches('"').split_whitespace().any(|r| r == "next")
            });
        if is_next {
            return Some(url.to_string());
        }
    }
    None
}

/// 一个仓库条目 → 待落库形态。
///
/// `external_id` 用**仓库 id 而不是 full_name**：仓库改名 / 转让后 full_name 会变，
/// 用它会导进重复条目（而仓库 id 不变，改名能被正确识别成"同一条，需要更新"）。
pub fn repo_to_favorite(repo: &Value) -> Option<NewFavorite> {
    let id = repo.get("id")?.as_i64()?;
    let full_name = repo.get("full_name")?.as_str()?.trim();
    if full_name.is_empty() {
        return None;
    }
    let url = repo
        .get("html_url")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("https://github.com/{}", full_name));

    let description = repo
        .get("description")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    Some(NewFavorite {
        source: SOURCE.to_string(),
        external_id: id.to_string(),
        url,
        title: full_name.to_string(),
        subtitle: Some(full_name.to_string()),
        description,
        // 仓库对象本身不带收藏时间，只有 star 列表元素才有（见 starred_item_to_favorite）
        favorited_at: None,
        // 归类时要用：语言 + topics 是最有价值的两个信号
        extra_json: Some(
            json!({
                "language": repo.get("language").and_then(|v| v.as_str()),
                "topics": repo.get("topics").and_then(|v| v.as_array()).cloned().unwrap_or_default(),
                "stars": repo.get("stargazers_count").and_then(|v| v.as_i64()),
            })
            .to_string(),
        ),
        initial_status: None,
    })
}

/// star 列表首页 URL（后续翻页全靠 `Link` 头，这里只拼第一页）。
pub fn starred_url(login: &str) -> String {
    format!(
        "https://api.github.com/users/{}/starred?per_page=100&sort=created&direction=desc",
        login
    )
}

/// 把 HTTP 状态翻成可操作提示：401 与 403 的处理方式完全不同（换 token vs 等限流），
/// 直接抛 "HTTP 403" 用户没法自助。
pub fn map_status_error(status: u16, what: &str) -> String {
    format!(
        "{}失败 (HTTP {}){}",
        what,
        status,
        crate::commands::utils::github_status_hint(status)
    )
}

/// 取 token 归属用户的 login：不让用户手填用户名（填错就拉到别人的 star）。
pub async fn fetch_user_login(token: &str) -> Result<String, String> {
    let resp = request(token, "https://api.github.com/user").await?;
    resp.get("login")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "GitHub 未返回 login，请检查 Token 是否有效".to_string())
}

/// 拉一页 star 列表，返回（响应体，下一页 URL）。
///
/// 带 `application/vnd.github.star+json` 媒体类型：这样每个元素是 `{ starred_at, repo }`，
/// 才能拿到**平台记录的收藏时间**；不带的话元素只有仓库对象，时间就永久丢失了。
pub async fn fetch_starred_page(token: &str, url: &str) -> Result<(Value, Option<String>), String> {
    let (body, link) = request_with_link(token, url, "application/vnd.github.star+json").await?;
    Ok((body, next_page_url(link.as_deref())))
}

/// 一条 star 列表元素 → 待落库条目。
///
/// 见 [`fetch_starred_page`]：带 star+json 时元素是 `{ starred_at, repo }` 两层结构，
/// 不带时元素本身就是仓库对象（`starred_at` 取不到，收藏时间为空）。
pub fn starred_item_to_favorite(item: &Value) -> Option<NewFavorite> {
    let repo = item.get("repo").unwrap_or(item);
    let mut favorite = repo_to_favorite(repo)?;
    favorite.favorited_at = item
        .get("starred_at")
        .and_then(|v| v.as_str())
        .and_then(super::db::rfc3339_to_local_str);
    Some(favorite)
}

/// 查单个仓库是否存在（失效检测用）。返回 `(状态码, 响应体)`——
/// 404 本身就是结论之一，所以这里**不把非 2xx 当错误抛**。
pub async fn fetch_repo(token: &str, full_name: &str) -> Result<(u16, Option<Value>), String> {
    let url = format!("https://api.github.com/repos/{}", full_name);
    let resp = crate::commands::utils::get_http_client()
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "Any-Version-Manager")
        .send()
        .await
        .map_err(|e| format!("请求 GitHub 失败: {}", e))?;
    let status = resp.status().as_u16();
    let body = resp.json::<Value>().await.ok();
    Ok((status, body))
}

/// 带鉴权的 GET，失败时带上可操作提示。
async fn request(token: &str, url: &str) -> Result<Value, String> {
    request_with_link(token, url, "application/vnd.github+json")
        .await
        .map(|(body, _)| body)
}

/// 中文 README 的候选文件名，**按优先级**排在默认 README 之前。
///
/// 只要 `README_ZH.md`：中文说明在这套命名里最常见，多试五六种变体换来的是
/// 每次展开都能多打几个 404 请求，不值。大小写各留一个是因为
/// contents 接口的路径**区分大小写**，而实际仓库里两种写法都有。
pub const README_ZH_CANDIDATES: [&str; 2] = ["README_ZH.md", "readme_zh.md"];

/// 带可选 token 的 GET builder。
///
/// token 为空时**不加 Authorization 头**：加一个空的 `Authorization:` 会被 GitHub
/// 当成无效凭证直接 401，而公开仓库不带头本来就能读。
fn github_get(
    url: &str,
    accept: &'static str,
    token: &str,
) -> reqwest::RequestBuilder {
    let mut request = crate::commands::utils::get_http_client()
        .get(url)
        .header("Accept", accept)
        .header("User-Agent", "Any-Version-Manager")
        .header("X-GitHub-Api-Version", "2022-11-28");
    if !token.trim().is_empty() {
        request = request.header("Authorization", format!("Bearer {}", token.trim()));
    }
    request
}

/// 取仓库里某个文件的**原始文本**；文件不存在返回 `None`（404 不是错误）。
pub async fn fetch_file_text(
    token: &str,
    full_name: &str,
    path: &str,
) -> Result<Option<String>, String> {
    let url = format!("https://api.github.com/repos/{}/contents/{}", full_name, path);
    // raw 直接给文件内容，省掉一次 base64 解码
    let resp = github_get(&url, "application/vnd.github.raw", token)
        .send()
        .await
        .map_err(|e| format!("请求 GitHub 失败: {}", e))?;

    let status = resp.status().as_u16();
    if status == 404 {
        return Ok(None);
    }
    if !(200..300).contains(&status) {
        return Err(map_status_error(status, "读取 README"));
    }
    let text = resp
        .text()
        .await
        .map_err(|e| format!("读取 README 内容失败: {}", e))?;
    Ok(Some(text))
}

/// 取仓库的默认 README（GitHub 自己解析文件名），返回 `(文件名, 内容)`。
pub async fn fetch_default_readme(
    token: &str,
    full_name: &str,
) -> Result<Option<(String, String)>, String> {
    let url = format!("https://api.github.com/repos/{}/readme", full_name);
    let resp = github_get(&url, "application/vnd.github+json", token)
        .send()
        .await
        .map_err(|e| format!("请求 GitHub 失败: {}", e))?;

    let status = resp.status().as_u16();
    if status == 404 {
        return Ok(None);
    }
    if !(200..300).contains(&status) {
        return Err(map_status_error(status, "读取 README"));
    }
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 GitHub 响应失败: {}", e))?;
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("README")
        .to_string();
    // content 是 base64（含换行），用标准表解码
    let encoded = body
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "GitHub 未返回 README 内容".to_string())?;
    Ok(Some((name, decode_readme(encoded)?)))
}

/// 解码 GitHub 返回的 base64 正文（**含换行**，必须先去掉空白）。
pub fn decode_readme(encoded: &str) -> Result<String, String> {
    let cleaned: String = encoded.chars().filter(|c| !c.is_whitespace()).collect();
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(cleaned.as_bytes())
        .map_err(|e| format!("解码 README 失败: {}", e))?;
    Ok(String::from_utf8_lossy(&decoded).to_string())
}

async fn request_with_link(
    token: &str,
    url: &str,
    accept: &str,
) -> Result<(Value, Option<String>), String> {
    let resp = crate::commands::utils::get_http_client()
        .get(url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", accept)
        .header("User-Agent", "Any-Version-Manager")
        .send()
        .await
        .map_err(|e| format!("请求 GitHub 失败: {}", e))?;

    let status = resp.status().as_u16();
    let link = resp
        .headers()
        .get(reqwest::header::LINK)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    if !(200..300).contains(&status) {
        return Err(map_status_error(status, "拉取 GitHub 数据"));
    }
    let body = resp
        .json::<Value>()
        .await
        .map_err(|e| format!("解析 GitHub 响应失败: {}", e))?;
    Ok((body, link))
}

#[cfg(test)]
mod tests {
    use super::{
        decode_readme, map_status_error, next_page_url, repo_to_favorite, starred_url,
        README_ZH_CANDIDATES, SOURCE,
    };
    use serde_json::json;

    #[test]
    fn parses_next_from_link_header() {
        let link = r#"<https://api.github.com/user/1/starred?page=2>; rel="next", <https://api.github.com/user/1/starred?page=9>; rel="last""#;
        assert_eq!(
            next_page_url(Some(link)).as_deref(),
            Some("https://api.github.com/user/1/starred?page=2")
        );
    }

    /// 只有 prev / last 时不能翻页，否则导入会死循环。
    #[test]
    fn ignores_link_headers_without_next() {
        assert_eq!(next_page_url(Some(r#"<https://x/1>; rel="prev""#)), None);
        assert_eq!(
            next_page_url(Some(
                r#"<https://x/1>; rel="prev", <https://x/9>; rel="last""#
            )),
            None
        );
        assert_eq!(next_page_url(None), None);
        assert_eq!(next_page_url(Some("")), None);
    }

    #[test]
    fn maps_repo_to_favorite_using_native_id() {
        let repo = json!({
            "id": 42,
            "full_name": "o/r",
            "html_url": "https://github.com/o/r",
            "description": "a cli",
            "language": "Rust",
            "topics": ["cli", "rust"],
            "stargazers_count": 123
        });
        let fav = repo_to_favorite(&repo).unwrap();
        assert_eq!(fav.source, SOURCE);
        assert_eq!(fav.external_id, "42", "必须用仓库 id，不能用 full_name");
        assert_eq!(fav.title, "o/r");
        assert_eq!(fav.url, "https://github.com/o/r");
        assert!(fav.extra_json.as_ref().unwrap().contains("Rust"));
        assert!(fav.extra_json.as_ref().unwrap().contains("cli"));
    }

    /// 没有 description / topics 的仓库照样要导入（只靠名字归类也得能用）。
    #[test]
    fn missing_optional_fields_still_imports() {
        let repo = json!({ "id": 7, "full_name": "o/none", "html_url": "https://github.com/o/none" });
        let fav = repo_to_favorite(&repo).unwrap();
        assert_eq!(fav.description, None);
        assert_eq!(fav.external_id, "7");
    }

    /// 缺 id 或 full_name 的异常条目跳过，不要让整批导入失败。
    #[test]
    fn malformed_entries_are_skipped() {
        assert!(repo_to_favorite(&json!({ "full_name": "o/no-id" })).is_none());
        assert!(repo_to_favorite(&json!({ "id": 1 })).is_none());
        assert!(repo_to_favorite(&json!({ "id": 1, "full_name": "  " })).is_none());
    }

    /// 没 html_url 时用 full_name 拼一个（GitHub 极少见，但不能因此丢条目）。
    #[test]
    fn falls_back_to_full_name_url() {
        let fav = repo_to_favorite(&json!({ "id": 9, "full_name": "o/r" })).unwrap();
        assert_eq!(fav.url, "https://github.com/o/r");
    }

    /// 中文 README 优先：`README_ZH.md` 必须排在第一个。
    /// （顺序即优先级，改动顺序要连带改这个测试，别默默调。）
    #[test]
    fn readme_zh_candidate_comes_first() {
        assert_eq!(README_ZH_CANDIDATES[0], "README_ZH.md");
        // 默认 README 由 GitHub 自己解析（能认 README.md/.rst/无扩展名），不放进候选表
        assert!(
            README_ZH_CANDIDATES
                .iter()
                .all(|n| n.eq_ignore_ascii_case("readme_zh.md")),
            "候选表里只该有中文 README 的两种大小写写法"
        );
    }

    /// README 正文是带换行的 base64，必须先剥空白再解码。
    #[test]
    fn decode_readme_strips_newlines() {
        let encoded = "aGVsbG8K\n\nd29ybGQ=";
        assert_eq!(decode_readme(encoded).unwrap(), "hello\nworld");
        assert!(decode_readme("!!!not-base64!!!").is_err());
    }

    #[test]
    fn starred_url_requests_largest_page() {
        assert_eq!(
            starred_url("octocat"),
            "https://api.github.com/users/octocat/starred?per_page=100&sort=created&direction=desc"
        );
    }

    /// 401 必须明说「token 无效」，403 必须明说「限流」——否则用户没法自助。
    #[test]
    fn error_messages_are_actionable() {
        let unauthorized = map_status_error(401, "拉取 GitHub 数据");
        assert!(unauthorized.contains("401"));
        assert!(unauthorized.contains("GITHUB_TOKEN"));

        let limited = map_status_error(403, "拉取 GitHub 数据");
        assert!(limited.contains("限流"));
        assert!(limited.contains("Token"));
    }
}
