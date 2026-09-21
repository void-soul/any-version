//! GitHub star 导入：分页解析与条目映射。
//!
//! 只做**纯计算 + 只读请求**，不调用任何写入接口（本模块不 unstar、不改仓库）。

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
pub async fn fetch_starred_page(token: &str, url: &str) -> Result<(Value, Option<String>), String> {
    let (body, link) = request_with_link(token, url).await?;
    Ok((body, next_page_url(link.as_deref())))
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
    request_with_link(token, url).await.map(|(body, _)| body)
}

async fn request_with_link(token: &str, url: &str) -> Result<(Value, Option<String>), String> {
    let resp = crate::commands::utils::get_http_client()
        .get(url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github+json")
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
    use super::{map_status_error, next_page_url, repo_to_favorite, starred_url, SOURCE};
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
