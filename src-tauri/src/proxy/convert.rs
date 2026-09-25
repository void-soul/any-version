//! 协议转换分发表（请求体 / 响应体）——协议转换代理与本地聚合共用。
//!
//! 此前 `server.rs` 与 `aggregate.rs` 各写一份 6 分支的 (inbound, outbound) match，
//! 是「URL 拼接 / 鉴权头」之外的第三处重复实现，且同样会漂移。这里收敛成一份。
//!
//! 同协议**不转换**（原样返回）：代理靠 `apply_masquerade` 改写模型、聚合靠
//! `rewrite_upstream_model` 改写模型，两者语义不同，不在本模块统一 ——
//! 本模块只负责「跨协议时怎么把 A 形态变成 B 形态」这一件事。

use serde_json::Value;

/// 请求体：P_in → P_out（同协议原样返回，不做模型名改写）。
pub fn convert_request(inbound: &str, outbound: &str, body: &Value, model: &str) -> Value {
    match (inbound, outbound) {
        (a, b) if a == b => body.clone(),
        ("anthropic", "openai") => crate::proxy::transform::anthropic_to_openai(body, model, None),
        ("openai", "anthropic") => crate::proxy::transform::openai_to_anthropic(body, model, None),
        ("anthropic", "google") => crate::proxy::google::anthropic_to_google(body, model),
        ("openai", "google") => crate::proxy::google::openai_to_google(body, model),
        ("google", "anthropic") => crate::proxy::google::google_to_anthropic(body, model),
        ("google", "openai") => crate::proxy::google::google_to_openai(body, model),
        _ => body.clone(),
    }
}

/// 响应体：P_out → P_in（同协议原样返回）。
pub fn convert_response(outbound: &str, inbound: &str, resp: &Value, claimed: &str) -> Value {
    match (outbound, inbound) {
        (a, b) if a == b => resp.clone(),
        ("openai", "anthropic") => crate::proxy::transform::openai_response_to_anthropic(resp, claimed),
        ("anthropic", "openai") => crate::proxy::transform::anthropic_response_to_openai(resp, claimed),
        ("google", "anthropic") => crate::proxy::google::google_response_to_anthropic(resp, claimed),
        ("google", "openai") => crate::proxy::google::google_response_to_openai(resp, claimed),
        ("anthropic", "google") => crate::proxy::google::anthropic_response_to_google(resp, claimed),
        ("openai", "google") => crate::proxy::google::openai_response_to_google(resp, claimed),
        _ => resp.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::{convert_request, convert_response};
    use serde_json::json;

    /// 同协议不转换：代理靠 masquerade 改写、聚合靠 rewrite，本模块必须原样返回。
    #[test]
    fn same_protocol_is_passthrough() {
        let body = json!({"model": "kiro-proxy", "messages": [{"role": "user", "content": "hi"}]});
        assert_eq!(convert_request("openai", "openai", &body, "deepseek-chat"), body);
        assert_eq!(convert_response("anthropic", "anthropic", &body, "deepseek-chat"), body);
    }

    /// 六组跨协议转换都有实现（不 panic / 返回非空即可，具体形态由 transform/google 各自测试覆盖）。
    #[test]
    fn all_cross_protocol_directions_are_covered() {
        let req = json!({"model": "m", "messages": [{"role": "user", "content": "hi"}]});
        let pairs = [
            ("anthropic", "openai"),
            ("openai", "anthropic"),
            ("anthropic", "google"),
            ("openai", "google"),
            ("google", "anthropic"),
            ("google", "openai"),
        ];
        for (i, o) in pairs {
            let v = convert_request(i, o, &req, "m");
            assert!(!v.is_null(), "{i}->{o} 请求转换不应为空");
        }
        let resp = json!({"id": "x"});
        let rpairs = [
            ("openai", "anthropic"),
            ("anthropic", "openai"),
            ("google", "anthropic"),
            ("google", "openai"),
            ("anthropic", "google"),
            ("openai", "google"),
        ];
        for (o, i) in rpairs {
            let v = convert_response(o, i, &resp, "m");
            assert!(!v.is_null(), "{o}->{i} 响应转换不应为空");
        }
    }
}
