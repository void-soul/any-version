//! 上游请求的公共构造：URL 拼接 + 鉴权头注入。
//!
//! 协议转换代理（`server.rs`）与本地聚合（`commands/ai/aggregate.rs`）都要「把出站请求
//! 拼到供应商端点上、带上对应协议的鉴权头」，此前两处各写了一份，已经出现行为漂移：
//! - URL 拼接：代理用 `contains("/messages")` 守卫（会把 `.../xxx/messages/v2` 误判为已含端点），
//!   聚合用 `ends_with`；代理还不认供应商的 `include_v1` 开关（聚合认）。
//! - 鉴权：代理给 anthropic 出站同时发 `x-api-key` + `Authorization: Bearer`，聚合只发 `x-api-key`。
//! - 自定义头：代理注入 `provider.custom_headers`，聚合完全不注入。
//!
//! 本模块把这两件事各收敛成一份，两边都调它，并统一语义为「聚合的超集」：
//! 更严的 URL 守卫（`ends_with`）+ 支持 `include_v1` + anthropic 双头 + 自定义头。

use reqwest::RequestBuilder;

use super::headers as upstream_headers;
use super::types::UpstreamHeader;

/// 出站协议统一用字符串（"openai" / "anthropic" / "google"），
/// 聚合的 `Outbound` 通过 `as_str()` 转成它，代理本来就是 `&str`。
///
/// 按出站协议拼上游 URL，返回 `(url, 鉴权头名)`。
///
/// `include_v1`：`None` = 自动（URL 结尾已是 `/v1` 就不补）；`Some(true)` = 一定补；
/// `Some(false)` = 一定不补（对接把版本号写进网关路径、或干脆不带版本号的兼容层）。
/// 无 `/v1` 概念的 google 协议忽略该参数。
pub fn resolve_url(
    protocol: &str,
    base: &str,
    model: &str,
    is_stream: bool,
    include_v1: Option<bool>,
) -> (String, &'static str) {
    let base = base.trim().trim_end_matches('/');
    match protocol {
        "openai" => {
            let url = if base.ends_with("/chat/completions") {
                base.to_string()
            } else if needs_v1(base, include_v1) {
                format!("{base}/v1/chat/completions")
            } else {
                format!("{base}/chat/completions")
            };
            (url, "Authorization")
        }
        "anthropic" => {
            let url = if base.ends_with("/messages") {
                base.to_string()
            } else if needs_v1(base, include_v1) {
                format!("{base}/v1/messages")
            } else {
                format!("{base}/messages")
            };
            (url, "x-api-key")
        }
        "google" => {
            let gbase = if let Some(stripped) = base.strip_suffix("/v1beta") {
                stripped
            } else {
                base
            };
            let gbase = gbase.trim_end_matches('/');
            let url = if is_stream {
                format!("{gbase}/v1beta/models/{model}:streamGenerateContent?alt=sse")
            } else {
                format!("{gbase}/v1beta/models/{model}:generateContent")
            };
            (url, "x-goog-api-key")
        }
        _ => (String::new(), "Authorization"),
    }
}

/// 是否需要在 base 后面补一段 `/v1`。
fn needs_v1(base: &str, want: Option<bool>) -> bool {
    match want {
        Some(v) => v && !base.ends_with("/v1"),
        None => !base.ends_with("/v1"),
    }
}

/// 注入鉴权头 + 供应商自定义头（与协议无关，纯函数）。
///
/// 语义与原 `server.rs::build_upstream_request` 完全一致：
/// - anthropic（`x-api-key`）：同时发 `x-api-key` 与 `Authorization: Bearer`（有的网关只认其一），
///   外加 `anthropic-version: 2023-06-01`；
/// - google（`x-goog-api-key`）：只发 `x-goog-api-key`；
/// - openai 等（`Authorization`）：发 `Bearer <key>`；
/// - 用户显式配置的同名头优先（不重复注入）；自定义头最后追加。
pub fn inject_auth(
    req: RequestBuilder,
    auth_name: &str,
    api_key: &str,
    custom_headers: &[UpstreamHeader],
) -> RequestBuilder {
    let mut req = req;
    match auth_name {
        "x-api-key" => {
            if !upstream_headers::overrides(custom_headers, "x-api-key") {
                req = req.header("x-api-key", api_key);
            }
            if !upstream_headers::has_authorization(custom_headers) {
                req = req.header("Authorization", format!("Bearer {api_key}"));
            }
            if !upstream_headers::overrides(custom_headers, "anthropic-version") {
                req = req.header("anthropic-version", "2023-06-01");
            }
        }
        "x-goog-api-key" => {
            if !upstream_headers::overrides(custom_headers, "x-goog-api-key") {
                req = req.header("x-goog-api-key", api_key);
            }
        }
        _ => {
            if !upstream_headers::has_authorization(custom_headers) {
                req = req.header("Authorization", format!("Bearer {api_key}"));
            }
        }
    }
    upstream_headers::apply(req, custom_headers)
}

#[cfg(test)]
mod tests {
    use super::{inject_auth, resolve_url};
    use crate::proxy::types::UpstreamHeader;

    fn h(key: &str, value: &str) -> UpstreamHeader {
        UpstreamHeader {
            key: key.to_string(),
            value: value.to_string(),
        }
    }

    #[test]
    fn url_guards_use_ends_with_not_contains() {
        // 历史 bug：代理用 contains("/messages")，会把带 "messages" 子串的路径误判为已含端点
        let (url, _) = resolve_url("anthropic", "https://gw.xxx/messages-gateway", "m", false, None);
        assert_eq!(url, "https://gw.xxx/messages-gateway/v1/messages");
        // 真正已含端点才不补
        let (url, _) = resolve_url("anthropic", "https://gw.xxx/v1/messages", "m", false, None);
        assert_eq!(url, "https://gw.xxx/v1/messages");
    }

    #[test]
    fn url_honors_include_v1_switch() {
        // 自动（None）：结尾没有 /v1 就补
        assert_eq!(
            resolve_url("openai", "https://x.com/api", "m", false, None).0,
            "https://x.com/api/v1/chat/completions"
        );
        // 显式不补（网关自带版本段）
        assert_eq!(
            resolve_url("openai", "https://x.com/api/v9", "m", false, Some(false)).0,
            "https://x.com/api/v9/chat/completions"
        );
        // 显式补（但绝不拼出 /v1/v1）
        assert_eq!(
            resolve_url("openai", "https://x.com/api", "m", false, Some(true)).0,
            "https://x.com/api/v1/chat/completions"
        );
        assert_eq!(
            resolve_url("openai", "https://x.com/api/v1", "m", false, Some(true)).0,
            "https://x.com/api/v1/chat/completions"
        );
    }

    #[test]
    fn google_strips_v1beta_and_appends_model_path() {
        let (url, auth) = resolve_url("google", "https://g.com/v1beta", "gemini-2.5-pro", true, None);
        assert_eq!(url, "https://g.com/v1beta/models/gemini-2.5-pro:streamGenerateContent?alt=sse");
        assert_eq!(auth, "x-goog-api-key");
        let (url, _) = resolve_url("google", "https://g.com", "m", false, None);
        assert_eq!(url, "https://g.com/v1beta/models/m:generateContent");
    }

    #[test]
    fn auth_injects_dual_headers_for_anthropic_and_bearer_for_openai() {
        let client = reqwest::Client::new();

        let req = inject_auth(
            client.post("http://127.0.0.1:1/x"),
            "x-api-key",
            "sk-a",
            &[],
        )
        .build()
        .unwrap();
        let headers = req.headers();
        assert_eq!(headers.get("x-api-key").unwrap(), "sk-a");
        assert_eq!(headers.get("authorization").unwrap(), "Bearer sk-a");
        assert_eq!(headers.get("anthropic-version").unwrap(), "2023-06-01");

        let req = inject_auth(
            client.post("http://127.0.0.1:1/x"),
            "Authorization",
            "sk-o",
            &[],
        )
        .build()
        .unwrap();
        assert_eq!(req.headers().get("authorization").unwrap(), "Bearer sk-o");
        assert!(req.headers().get("x-api-key").is_none());
    }

    #[test]
    fn explicit_custom_headers_win_over_defaults() {
        let client = reqwest::Client::new();
        // 用户显式配置了 Authorization → 不注入默认 bearer；自定义头仍追加
        let req = inject_auth(
            client.post("http://127.0.0.1:1/x"),
            "Authorization",
            "sk-o",
            &[h("Authorization", "Bearer user-own"), h("X-Trace", "t1")],
        )
        .build()
        .unwrap();
        let values: Vec<&str> = req
            .headers()
            .get_all("authorization")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .collect();
        assert_eq!(values, vec!["Bearer user-own"], "有显式 Authorization 就不该再注入默认 bearer");
        assert_eq!(req.headers().get("x-trace").unwrap(), "t1");
    }
}
