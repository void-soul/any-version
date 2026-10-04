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

/// 把非 OpenAI 形状的模型目录响应规范化成 OpenAI 的 `{object:"list", data:[...]}`。
///
/// 有些上游（WorkBuddy 的 `/v2/enterprises/personal/models`）返回的是
/// `{"code":0,"data":{"models":[{"id":…,"tags":[…]}]}}`，客户端按 OpenAI 形状解析会
/// 拿到空列表（`data` 不是数组）。这里做一次归一，让 `/v1/models` 对客户端始终可用。
///
/// 已经是 OpenAI 形状的**原样返回**（不猜、不动供应商自己的字段）；认不出来的形状也
/// 原样返回 —— 宁可返回供应商原样，也不要返回空列表把客户端坑了。
pub fn normalize_models_response(body: Value) -> Value {
    use serde_json::json;
    if body.get("data").map(|d| d.is_array()).unwrap_or(false) {
        return body;
    }
    let Some(models) = body
        .get("data")
        .and_then(|d| d.get("models"))
        .and_then(|m| m.as_array())
    else {
        return body;
    };
    let data: Vec<Value> = models
        .iter()
        .filter_map(|m| m.get("id").and_then(|v| v.as_str()))
        .filter(|id| !id.is_empty())
        .map(|id| {
            json!({
                "id": id,
                "object": "model",
                "created": 1700000000,
                "owned_by": "upstream",
            })
        })
        .collect();
    json!({ "object": "list", "data": data })
}

#[cfg(test)]
mod model_normalize_tests {
    use super::normalize_models_response;
    use serde_json::json;

    #[test]
    fn backend_shape_becomes_openai_list() {
        // WorkBuddy 后端：data.models → OpenAI 的 data 数组，客户端才枚举得到
        let body = json!({"code":0,"data":{"models":[
            {"id":"hy3","vendor":"f"},
            {"id":"space-bunny","vendor":"f"}
        ]}});
        let out = normalize_models_response(body);
        assert_eq!(out["object"], "list");
        let ids: Vec<&str> = out["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["hy3", "space-bunny"], "顺序保持上游顺序");
    }

    #[test]
    fn already_openai_shape_is_returned_untouched() {
        // 已经是 OpenAI 形状就别动它 —— 不猜供应商字段
        let body = json!({"object":"list","data":[{"id":"a","object":"model","owned_by":"x"}]});
        let out = normalize_models_response(body.clone());
        assert_eq!(out, body);
    }

    #[test]
    fn unrecognizable_shape_is_returned_as_is() {
        // 认不出来时原样返回：宁可返回供应商原样，也不要返回空列表把客户端坑了
        let body = json!({"error_msg":"404 Route Not Found"});
        let out = normalize_models_response(body.clone());
        assert_eq!(out, body);
    }

    #[test]
    fn entries_without_id_are_skipped() {
        let body = json!({"data":{"models":[{"vendor":"f"},{"id":"ok"},{"id":""}]}});
        let out = normalize_models_response(body);
        assert_eq!(out["data"].as_array().unwrap().len(), 1);
        assert_eq!(out["data"][0]["id"], "ok");
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
