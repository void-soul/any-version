//! 供应商自定义上游请求头。
//!
//! 抄自 CodexPlusPlus ea0ac5d（issue #1685）。三处发往上游的请求——供应商连通性测试、
//! 模型列表拉取、协议代理转发——共用本模块，保证用户配置在三处行为一致。
//!
//! 两条硬约束：
//! 1. 传输层 / 逐跳头（`Host`、`Content-Length` 等）由 HTTP 客户端按实际报文决定，
//!    用户配置一律拒绝，避免造出不合法的请求；
//! 2. 显式配置的 `Authorization` 优先于供应商 API Key —— 存在自定义 `Authorization`
//!    时不再注入 bearer，优先级只此一种，不做隐式合并。

use reqwest::header::{HeaderName, HeaderValue};
use reqwest::RequestBuilder;

use super::types::UpstreamHeader;

/// 传输层 / 逐跳头：由 reqwest 与实际连接决定，用户配置没有意义且会破坏请求。
const FORBIDDEN_HEADERS: &[&str] = &[
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "keep-alive",
    "proxy-connection",
    "te",
    "trailer",
    "upgrade",
    "expect",
];

/// 值属于凭据的头：日志与导出必须打码。
const SENSITIVE_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "api-key",
    "x-api-key",
    "x-goog-api-key",
    "x-auth-token",
    "cookie",
    "set-cookie",
];

/// 自定义头条数上限（表单可能被粘贴进大量行）。
pub const MAX_HEADERS: usize = 64;

/// 是否为协议层掌控、不允许用户覆盖的传输头。
pub fn is_forbidden(name: &str) -> bool {
    let lowered = name.trim().to_ascii_lowercase();
    FORBIDDEN_HEADERS.contains(&lowered.as_str())
}

/// 值是否敏感（需要脱敏后再进日志 / 导出）。
pub fn is_sensitive(name: &str) -> bool {
    let lowered = name.trim().to_ascii_lowercase();
    SENSITIVE_HEADERS.contains(&lowered.as_str())
        || lowered.contains("token")
        || lowered.contains("secret")
}

/// 用户是否显式配置了某个头（带非空值）——用于决定是否跳过默认注入，
/// 避免同一个头被默认值 + 用户值重复写入（reqwest 的 `header()` 是 append 语义）。
pub fn overrides(headers: &[UpstreamHeader], name: &str) -> bool {
    headers.iter().any(|header| {
        header.key.trim().eq_ignore_ascii_case(name) && !header.value.trim().is_empty()
    })
}

/// 用户是否显式配置了 `Authorization`（带非空值）。
///
/// 存在自定义 `Authorization` 时不再注入供应商 key 的 bearer：有些网关要求
/// 用自家 token 而非 API Key 鉴权，隐式合并只会让请求被拒。
pub fn has_authorization(headers: &[UpstreamHeader]) -> bool {
    overrides(headers, "authorization")
}

/// 校验自定义头列表：空行（未填写的占位行）跳过，其余非法项给出明确错误。
///
/// 报错只回显头名称，不回显值——值可能是凭据。
pub fn validate(headers: &[UpstreamHeader]) -> Result<(), String> {
    let mut seen: Vec<String> = Vec::new();
    for header in headers {
        let key = header.key.trim();
        if key.is_empty() {
            // 表单里新增但还没填的行，视为未配置。
            continue;
        }
        let name = HeaderName::from_bytes(key.as_bytes())
            .map_err(|_| format!("自定义请求头「{key}」不是合法的 HTTP 头名称"))?;
        if is_forbidden(name.as_str()) {
            return Err(format!(
                "自定义请求头「{key}」由协议层掌控，不允许覆盖（Host、Content-Length 等传输头）"
            ));
        }
        let lowered = name.as_str().to_string();
        if seen.contains(&lowered) {
            return Err(format!("自定义请求头「{key}」重复配置"));
        }
        seen.push(lowered);
        HeaderValue::from_str(&header.value)
            .map_err(|_| format!("自定义请求头「{key}」的值包含非法字符"))?;
    }
    Ok(())
}

/// 保存时规范化：去空行、压缩首尾空白、按头名称去重（大小写不敏感）、限制条数。
pub fn normalize(headers: &[UpstreamHeader]) -> Vec<UpstreamHeader> {
    let mut out: Vec<UpstreamHeader> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for header in headers {
        let key = header.key.trim();
        if key.is_empty() {
            continue;
        }
        let lowered = key.to_ascii_lowercase();
        if seen.contains(&lowered) {
            continue;
        }
        seen.push(lowered);
        out.push(UpstreamHeader {
            key: key.to_string(),
            value: header.value.trim().to_string(),
        });
        if out.len() >= MAX_HEADERS {
            break;
        }
    }
    out
}

/// 把自定义头追加到请求上；非法项直接跳过（调用方应先 `validate`）。
pub fn apply(mut builder: RequestBuilder, headers: &[UpstreamHeader]) -> RequestBuilder {
    for header in headers {
        let key = header.key.trim();
        if key.is_empty() || is_forbidden(key) {
            continue;
        }
        let Ok(name) = HeaderName::from_bytes(key.as_bytes()) else {
            continue;
        };
        let Ok(value) = HeaderValue::from_str(&header.value) else {
            continue;
        };
        builder = builder.header(name, value);
    }
    builder
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::types::UpstreamHeader;

    fn h(key: &str, value: &str) -> UpstreamHeader {
        UpstreamHeader {
            key: key.to_string(),
            value: value.to_string(),
        }
    }

    #[test]
    fn forbidden_transport_headers_are_detected() {
        for name in [
            "Host",
            "content-length",
            "Transfer-Encoding",
            "connection",
            "keep-alive",
            "proxy-connection",
            "TE",
            "Trailer",
            "Upgrade",
            "Expect",
        ] {
            assert!(is_forbidden(name), "应判为传输层头: {name}");
        }
        assert!(!is_forbidden("x-request-id"));
        assert!(!is_forbidden("User-Agent"));
    }

    #[test]
    fn validate_rejects_forbidden_and_duplicate_headers() {
        assert!(validate(&[h("Host", "evil.example")]).is_err());
        // 同名头（大小写不敏感）重复配置要报错，否则注入顺序不确定
        assert!(validate(&[h("X-Foo", "a"), h("x-foo", "b")]).is_err());
        // 空行（表单里新增但没填完的占位行）视为未配置
        assert!(validate(&[h("", ""), h("   ", "x")]).is_ok());
        assert!(validate(&[h("X-Trace-Id", "abc-123")]).is_ok());
    }

    #[test]
    fn validate_error_message_never_echoes_the_value() {
        // 值可能是凭据，报错只回显头名称
        let err = validate(&[h("Host", "super-secret-value")]).unwrap_err();
        assert!(err.contains("Host"), "{err}");
        assert!(!err.contains("super-secret-value"), "{err}");
    }

    #[test]
    fn normalize_trims_drops_empty_and_dedupes_case_insensitively() {
        let out = normalize(&[h("  X-A  ", " 1 "), h("", ""), h("x-a", "2"), h("X-B", "3")]);
        assert_eq!(out, vec![h("X-A", "1"), h("X-B", "3")]);
    }

    #[test]
    fn normalize_caps_the_list_length() {
        let many: Vec<UpstreamHeader> = (0..MAX_HEADERS + 16)
            .map(|i| h(&format!("X-H-{i}"), "v"))
            .collect();
        assert_eq!(normalize(&many).len(), MAX_HEADERS);
    }

    #[test]
    fn sensitive_headers_are_detected_by_name_and_substring() {
        for name in [
            "Authorization",
            "proxy-authorization",
            "x-api-key",
            "x-goog-api-key",
            "Cookie",
            "X-Auth-Token",
            "X-Custom-Secret",
            "X-My-Token",
        ] {
            assert!(is_sensitive(name), "应判为敏感头: {name}");
        }
        assert!(!is_sensitive("x-request-id"));
    }

    #[test]
    fn has_authorization_requires_a_non_empty_value() {
        assert!(has_authorization(&[h("authorization", "Bearer x")]));
        assert!(has_authorization(&[h("AUTHORIZATION", "x")]));
        // 只填了名字没填值 → 不算显式配置，仍应注入供应商 key
        assert!(!has_authorization(&[h("authorization", "  ")]));
        assert!(!has_authorization(&[h("x-api-key", "k")]));
    }

    #[test]
    fn apply_keeps_valid_headers_and_skips_forbidden_or_malformed() {
        let req = apply(
            reqwest::Client::new().post("http://127.0.0.1:1/echo"),
            &[
                h("X-Trace", "abc"),
                h("Host", "evil.example"),
                h("bad header", "v"),
            ],
        )
        .build()
        .expect("请求应可构建");
        assert_eq!(
            req.headers().get("x-trace").and_then(|v| v.to_str().ok()),
            Some("abc")
        );
        assert!(req.headers().get("host").is_none(), "传输层头不得注入");
    }
}
