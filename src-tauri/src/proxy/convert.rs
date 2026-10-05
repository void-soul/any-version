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

/// 把 OpenAI 形态请求里的 `developer` 角色改写成 `system`。
///
/// `developer` 是 OpenAI 在 GPT-5 之后给 `system` 起的**新名字**，语义完全相同；
/// 但大量第三方兼容网关只认老的 `system`。实测 WorkBuddy（`copilot.tencent.com`）
/// 遇到 `developer` 会直接回 400 `Illegal API invocation from an unapproved channel`
/// （code 11128）—— 报文写着"渠道/鉴权"，实际是消息角色不认识，极难定位：
/// 同一秒里 `favorites` 走别的路径就正常，只有 Codex 的请求被拦。
///
/// Codex 尤其爱用这个角色：Responses API 的 `input` 里塞的是
/// `{"type":"message","role":"developer"}`，转换时会被原样搬进 chat 的 `messages`
/// （见 `responses::push_input_messages`），所以这不是边缘情况。
///
/// 统一改写成 `system` 是安全的：认 `developer` 的上游（OpenAI 官方）也接受 `system`。
/// 只动 `developer`，其余角色（`system` / `user` / `assistant` / `tool`）原样保留。
///
/// **必须在 `convert_request` 之后、优化器之前调用**：整流器要按角色判断消息块
/// （如图片降级只看 `messages`），角色没归一化它就认不出哪条是系统提示。
pub fn normalize_openai_message_roles(body: &mut Value) {
    use serde_json::json;

    let Some(messages) = body.get_mut("messages").and_then(|m| m.as_array_mut()) else {
        return;
    };
    for msg in messages.iter_mut() {
        if msg.get("role").and_then(|r| r.as_str()) == Some("developer") {
            if let Some(obj) = msg.as_object_mut() {
                obj.insert("role".into(), json!("system"));
            }
        }
    }
}

/// 标识非聊天模型的 tag（图像/视频生成，客户端拿去聊天会被上游拒）
pub const NON_CHAT_MODEL_TAGS: &[&str] = &["text-to-image", "image-to-image", "text-to-video"];

/// 名称里出现这些片段的是补全/改写/跳转等内部功能，不作为聊天模型暴露
pub const INTERNAL_MODEL_KEYWORDS: &[&str] = &["completion", "rewrite", "jump", "codewise"];

/// 判断一条模型目录项是不是「可聊天的模型」。
///
/// 规则来自 WorkBuddy 后端 `/v2/enterprises/personal/models` 的实际结构，三条：
/// 1. `tags` 命中图像/视频 → 不是聊天模型
/// 2. `vendor == "tencent"` → 内部模型（补全/跳转等）
/// 3. id 含内部功能关键词 → 不是聊天模型
///
/// **这是唯一一份过滤规则**：`proxy` 的 `/v1/models` 与 Buddy 2API 的 preflight
/// 都调它，避免两处漂移（镜像 Python 版 `_filter_chat_models` 的规则）。
pub fn is_chat_model(model: &Value) -> bool {
    let Some(id) = model.get("id").and_then(|v| v.as_str()) else {
        return false;
    };
    if id.is_empty() {
        return false;
    }
    let is_non_chat = model
        .get("tags")
        .and_then(|t| t.as_array())
        .map(|tags| {
            tags.iter().any(|t| {
                t.as_str()
                    .map(|s| NON_CHAT_MODEL_TAGS.contains(&s))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false);
    if is_non_chat {
        return false;
    }
    if model.get("vendor").and_then(|v| v.as_str()) == Some("tencent") {
        return false;
    }
    let lower = id.to_ascii_lowercase();
    !INTERNAL_MODEL_KEYWORDS.iter().any(|k| lower.contains(k))
}

/// 把非 OpenAI 形状的模型目录响应规范化成 OpenAI 的 `{object:"list", data:[...]}`。
///
/// 有些上游（WorkBuddy 的 `/v2/enterprises/personal/models`）返回的是
/// `{"code":0,"data":{"models":[{"id":…,"tags":[…]}]}}`，客户端按 OpenAI 形状解析会
/// 拿到空列表（`data` 不是数组）。这里做一次归一，让 `/v1/models` 对客户端始终可用。
///
/// 已经是 OpenAI 形状的**原样返回**（不猜、不动供应商自己的字段）；认不出来的形状也
/// 原样返回 —— 宁可返回供应商原样，也不要返回空列表把客户端坑了。
///
/// `filter_non_chat`：是否滤掉非聊天模型（图像/视频/内部功能）。默认场景（OpenAI
/// 形状的供应商）不需要；WorkBuddy 这类把图像模型也混在目录里的后端要开。
pub fn normalize_models_response(body: Value, filter_non_chat: bool) -> Value {
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
        .filter(|m| !filter_non_chat || is_chat_model(m))
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
        let out = normalize_models_response(body, false);
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
        let out = normalize_models_response(body.clone(), false);
        assert_eq!(out, body);
    }

    #[test]
    fn unrecognizable_shape_is_returned_as_is() {
        // 认不出来时原样返回：宁可返回供应商原样，也不要返回空列表把客户端坑了
        let body = json!({"error_msg":"404 Route Not Found"});
        let out = normalize_models_response(body.clone(), false);
        assert_eq!(out, body);
    }

    #[test]
    fn entries_without_id_are_skipped() {
        let body = json!({"data":{"models":[{"vendor":"f"},{"id":"ok"},{"id":""}]}});
        let out = normalize_models_response(body, false);
        assert_eq!(out["data"].as_array().unwrap().len(), 1);
        assert_eq!(out["data"][0]["id"], "ok");
    }
}

#[cfg(test)]
mod chat_model_filter_tests {
    use super::{is_chat_model, normalize_models_response};
    use serde_json::json;

    #[test]
    fn image_and_video_models_are_not_chat_models() {
        // hunyuan-image-alpha 这类：客户端枚举到了去聊天会被上游拒
        assert!(!is_chat_model(
            &json!({"id":"hunyuan-image-alpha","tags":["text-to-image"]})
        ));
        assert!(!is_chat_model(
            &json!({"id":"kling-v3-i2v","tags":["text-to-video"]})
        ));
        assert!(!is_chat_model(
            &json!({"id":"x","tags":["image-to-image","other"]})
        ));
    }

    #[test]
    fn tencent_vendor_and_internal_keywords_are_excluded() {
        assert!(!is_chat_model(&json!({"id":"whatever","vendor":"tencent"})));
        assert!(!is_chat_model(&json!({"id":"codewise-Completion"})));
        assert!(!is_chat_model(&json!({"id":"foo-REWRITE-bar"})));
        assert!(!is_chat_model(&json!({"id":"jump-node"})));
    }

    #[test]
    fn real_chat_models_pass() {
        for id in ["hy3", "space-bunny", "glm-5.3", "kimi-k3-1", "deepseek-v4-pro"] {
            assert!(is_chat_model(&json!({"id": id})), "{} 应该保留", id);
        }
    }

    #[test]
    fn filter_flag_controls_whether_image_models_are_dropped() {
        let body = json!({"data":{"models":[
            {"id":"hy3"},
            {"id":"hunyuan-image-alpha","tags":["text-to-image"]},
            {"id":"space-bunny"}
        ]}});
        // 关：全给（不猜供应商语义）
        let all = normalize_models_response(body.clone(), false);
        assert_eq!(all["data"].as_array().unwrap().len(), 3);
        // 开：滤掉图像模型，但 space-bunny 这种新模型必须留着
        let filtered = normalize_models_response(body, true);
        let ids: Vec<&str> = filtered["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["hy3", "space-bunny"]);
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
mod developer_role_tests {
    use super::normalize_openai_message_roles;
    use serde_json::json;

    /// 只改 `developer`，其余角色一个都不许动。
    #[test]
    fn only_developer_becomes_system() {
        let mut body = json!({"messages": [
            {"role": "developer", "content": "sys1"},
            {"role": "system", "content": "sys2"},
            {"role": "user", "content": "q"},
            {"role": "assistant", "content": "a"},
            {"role": "tool", "content": "t", "tool_call_id": "c1"},
            {"role": "developer", "content": ""}
        ]});
        normalize_openai_message_roles(&mut body);
        let roles: Vec<&str> = body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["role"].as_str().unwrap())
            .collect();
        assert_eq!(
            roles,
            vec!["system", "system", "user", "assistant", "tool", "system"]
        );
        // 内容必须原样保留（含那条空 developer）
        assert_eq!(body["messages"][0]["content"], json!("sys1"));
        assert_eq!(body["messages"][5]["content"], json!(""));
    }

    /// 结构不完整时不能 panic：没有 messages / messages 不是数组 / role 不是字符串。
    #[test]
    fn tolerates_bodies_without_messages() {
        for body in [
            json!({}),
            json!({"messages": "nope"}),
            json!({"messages": [{"content": "无角色"}]}),
            json!({"messages": [{"role": 7}]}),
        ] {
            let mut b = body.clone();
            normalize_openai_message_roles(&mut b);
            assert_eq!(b, body, "不该改动任何东西：{b}");
        }
    }

    /// **闭环**：Codex 风格的 Responses 请求（系统提示拆成多条 `developer` 塞在 input）
    /// 走完 转换 → 归一化 之后，`messages` 里不能再有 `developer`。
    ///
    /// 这条锁的是实测故障（Q-0324）：WorkBuddy 收到 `developer` 会回
    /// 400 `11128 Illegal API invocation from an unapproved channel`。
    ///
    /// 转换链要与 `server.rs::responses_handler` 一致：它自己先调 `responses_to_chat`
    /// （`convert_request` **不**处理 `("responses","openai")`），再把 chat 体交给
    /// `process_request` —— 归一化就在 `process_request` 里做。
    #[test]
    fn codex_responses_request_ends_up_with_no_developer_role() {
        let codex = json!({
            "model": "gpt-6-astra",
            "input": [
                {"type": "message", "role": "developer",
                 "content": [{"type": "input_text", "text": "You are Codex."}]},
                {"type": "message", "role": "user",
                 "content": [{"type": "input_text", "text": "hi"}]}
            ]
        });
        let mut chat = crate::proxy::responses::responses_to_chat(&codex);
        // 转换本身就该把 input 里的角色原样搬过来（不做语义判断）——
        // 先确认这条前提成立，免得测试因为别的原因"通过"
        assert_eq!(chat["messages"][0]["role"], json!("developer"), "前提：转换不认 developer");

        normalize_openai_message_roles(&mut chat);
        let roles: Vec<&str> = chat["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["role"].as_str().unwrap())
            .collect();
        assert!(
            !roles.contains(&"developer"),
            "出站体里不该再有 developer：{roles:?}"
        );
        assert_eq!(roles, vec!["system", "user"]);
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
