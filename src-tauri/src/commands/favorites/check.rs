//! 失效检测：条目打不开（404）/ 被改名搬家（重定向）。
//!
//! - GitHub 走官方接口：能顺带认出「改名/转让」，比只看 HTTP 状态准；
//! - 其余来源（浏览器书签 / B站 / 知乎…）走 **HTTP 探测**：HEAD 为主，
//!   被拒时回退带 Range 的 GET（不少站点不接受 HEAD）。

use serde_json::Value;

/// 单个条目的探测结论。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoneStatus {
    Ok,
    /// 仓库没了（404）
    Gone,
    /// 改名或转让了：带上新的 URL
    Redirect(String),
    /// 探测本身失败（限流 / 网络），**不能当成失效**
    Unknown,
}

impl GoneStatus {
    pub fn as_db_status(&self) -> &'static str {
        match self {
            GoneStatus::Ok => "ok",
            GoneStatus::Gone => "gone",
            GoneStatus::Redirect(_) => "redirect",
            GoneStatus::Unknown => "unknown",
        }
    }
}

/// 把「查仓库」的响应翻成结论。
///
/// 最容易踩的坑：限流（403/429）时接口是通的、仓库也在，只是这次没查成 ——
/// 标成 `gone` 会让用户以为收藏失效了。所以只有 404 才是失效，其它一律 Unknown。
pub fn classify_repo_response(
    status: u16,
    body: Option<&Value>,
    requested_full_name: &str,
) -> GoneStatus {
    if status == 404 {
        return GoneStatus::Gone;
    }
    if status != 200 {
        return GoneStatus::Unknown;
    }
    let Some(body) = body else {
        return GoneStatus::Unknown;
    };
    let current = body.get("full_name").and_then(|v| v.as_str());
    match current {
        // 大小写不同也算改名（GitHub 对大小写不敏感但会规范化显示）
        Some(name) if !name.eq_ignore_ascii_case(requested_full_name) => GoneStatus::Redirect(
            body.get("html_url")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("https://github.com/{}", name)),
        ),
        _ => GoneStatus::Ok,
    }
}

// ─── 通用 HTTP 探测（给非 GitHub 来源用） ───

/// 浏览器 UA：很多站点对默认 UA（reqwest/x.y）直接 403，
/// 那样会把活着的页面一律判成 Unknown —— 检测就白做了。
pub const PROBE_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";

/// URL 探测结论（HTTP 状态 → 语义）。
///
/// 判定原则与 GitHub 那套一致：**只有明确的「不存在」才算失效**，
/// 403/429/5xx/超时全算 Unknown —— 反爬与临时故障太常见，误标「已失效」比漏标更糟。
pub fn classify_http_status(status: u16) -> GoneStatus {
    match status {
        200..=299 => GoneStatus::Ok,
        // 301/302 由 reqwest 自动跟随；手动传 3xx（未跟随）也算活着
        300..=399 => GoneStatus::Ok,
        404 | 410 => GoneStatus::Gone,
        _ => GoneStatus::Unknown,
    }
}

/// 是否值得用 GET 再试一次（HEAD 被站点拒绝时）。
pub fn should_retry_with_get(status: u16) -> bool {
    matches!(status, 400 | 401 | 403 | 405 | 406 | 501)
}

/// 规范化 URL 用于「是否搬家」的比较：去掉末尾斜杠、忽略 http/https 与 www 差异。
///
/// 好多站点会把 `http://x.com/a` 跳到 `https://www.x.com/a/`，那是同一页，
/// 报「已搬家」只会让用户以为收藏变了。
pub fn same_page(a: &str, b: &str) -> bool {
    fn norm(url: &str) -> String {
        let s = url.trim();
        let s = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://")).unwrap_or(s);
        let s = s.strip_prefix("www.").unwrap_or(s);
        let s = s.split(['#', '?']).next().unwrap_or(s);
        s.trim_end_matches('/').to_ascii_lowercase()
    }
    norm(a) == norm(b)
}

/// 探测一个 URL：返回结论（含「搬家」的新地址）。
pub async fn probe_url(
    client: &reqwest::Client,
    url: &str,
) -> GoneStatus {
    let url = url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        // javascript: / chrome:// / file:// 这类书签没法探测，别猜
        return GoneStatus::Unknown;
    }
    let head = client.head(url).header("User-Agent", PROBE_UA).send().await;
    let resp = match head {
        Ok(r) if !should_retry_with_get(r.status().as_u16()) => r,
        // HEAD 被拒（或直接失败）→ 用 GET 取 1 字节再试
        _ => match client
            .get(url)
            .header("User-Agent", PROBE_UA)
            .header("Range", "bytes=0-0")
            .send()
            .await
        {
            Ok(r) => r,
            Err(_) => return GoneStatus::Unknown,
        },
    };
    let status = resp.status().as_u16();
    let verdict = classify_http_status(status);
    if verdict == GoneStatus::Ok {
        let final_url = resp.url().as_str().to_string();
        if !same_page(&final_url, url) {
            return GoneStatus::Redirect(final_url);
        }
    }
    verdict
}

#[cfg(test)]
mod tests {
    use super::{classify_http_status, classify_repo_response, same_page, should_retry_with_get, GoneStatus};
    use serde_json::json;

    #[test]
    fn missing_repository_is_gone() {
        assert_eq!(classify_repo_response(404, None, "o/r"), GoneStatus::Gone);
    }

    /// 改名要认出来并给出新地址（用户点进去才不会 404）。
    #[test]
    fn renamed_repository_is_redirect() {
        let body = json!({ "full_name": "O/R2", "html_url": "https://github.com/O/R2" });
        assert_eq!(
            classify_repo_response(200, Some(&body), "o/r"),
            GoneStatus::Redirect("https://github.com/O/R2".into())
        );
    }

    #[test]
    fn same_name_is_ok() {
        let body = json!({ "full_name": "o/r" });
        assert_eq!(classify_repo_response(200, Some(&body), "o/r"), GoneStatus::Ok);
        // 只差大小写不算改名
        let upper = json!({ "full_name": "O/R" });
        assert_eq!(classify_repo_response(200, Some(&upper), "o/r"), GoneStatus::Ok);
    }

    /// 通用 HTTP 判定：只有 404/410 是失效，反爬与临时故障一律 Unknown。
    #[test]
    fn http_status_only_404_and_410_mean_gone() {
        assert_eq!(classify_http_status(200), GoneStatus::Ok);
        assert_eq!(classify_http_status(204), GoneStatus::Ok);
        assert_eq!(classify_http_status(404), GoneStatus::Gone);
        assert_eq!(classify_http_status(410), GoneStatus::Gone);
        // 3xx 视为活着（reqwest 会自动跟随；个别站点手动返回时也说明页面还在）
        assert_eq!(classify_http_status(301), GoneStatus::Ok);
        for s in [400, 401, 403, 429, 500, 502, 503] {
            assert_eq!(classify_http_status(s), GoneStatus::Unknown, "状态 {s} 不该被判失效");
        }
        // HEAD 被拒 → 用 GET 再试
        assert!(should_retry_with_get(405));
        assert!(should_retry_with_get(403));
        assert!(!should_retry_with_get(200));
        assert!(!should_retry_with_get(404));
    }

    /// 「搬家」判定要忽略 http/https、www 与末尾斜杠的差异，否则满屏假搬家。
    #[test]
    fn same_page_ignores_scheme_www_and_trailing_slash() {
        assert!(same_page("https://www.x.com/a/", "http://x.com/a"));
        assert!(same_page("https://x.com/a?utm=1", "https://x.com/a"));
        assert!(!same_page("https://x.com/a", "https://x.com/b"));
        assert!(!same_page("https://x.com/a", "https://y.com/a"));
    }

    /// 限流 / 服务端错误不能当成失效，否则会把活着的收藏误标成已失效。
    #[test]
    fn rate_limit_and_server_errors_are_unknown() {
        assert_eq!(classify_repo_response(403, None, "o/r"), GoneStatus::Unknown);
        assert_eq!(classify_repo_response(429, None, "o/r"), GoneStatus::Unknown);
        assert_eq!(classify_repo_response(500, None, "o/r"), GoneStatus::Unknown);
        assert_eq!(classify_repo_response(200, None, "o/r"), GoneStatus::Unknown);
    }
}
