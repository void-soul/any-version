//! 失效检测：仓库被删 / 改名（重定向）。
//!
//! 只有 GitHub 走官方接口，风险低；B站 / 知乎等各自接入时再定探测方式。

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

#[cfg(test)]
mod tests {
    use super::{classify_repo_response, GoneStatus};
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

    /// 限流 / 服务端错误不能当成失效，否则会把活着的收藏误标成已失效。
    #[test]
    fn rate_limit_and_server_errors_are_unknown() {
        assert_eq!(classify_repo_response(403, None, "o/r"), GoneStatus::Unknown);
        assert_eq!(classify_repo_response(429, None, "o/r"), GoneStatus::Unknown);
        assert_eq!(classify_repo_response(500, None, "o/r"), GoneStatus::Unknown);
        assert_eq!(classify_repo_response(200, None, "o/r"), GoneStatus::Unknown);
    }
}
