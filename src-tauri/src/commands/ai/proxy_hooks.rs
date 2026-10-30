//! Application adapters for the reusable protocol proxy.
use crate::proxy::{
    hooks::{ApplicationHooks, UsageRecord},
    server::ProxyState,
};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use serde_json::{json, Value};

pub struct AppProxyHooks;
impl ApplicationHooks for AppProxyHooks {
    fn extend_routes(&self, router: Router<ProxyState>) -> Router<ProxyState> {
        router
            .route("/collab/agent/send", post(collab_agent_send_handler))
            .route("/collab/agent/message", post(collab_agent_message_handler))
            .route("/collab/agent/task", post(collab_agent_task_handler))
    }
    fn independently_authenticated(&self, path: &str) -> bool {
        matches!(
            path,
            "/collab/agent/send" | "/collab/agent/message" | "/collab/agent/task"
        )
    }
    fn record_usage(&self, record: UsageRecord<'_>) {
        let entry = super::usage::UsageEntry::success(
            record.tool_id,
            record.model,
            Some(record.provider_id),
        )
        .tokens(record.input_tokens, record.output_tokens)
        .cache(record.cache_read_tokens, record.cache_write_tokens)
        .timing(record.duration_ms, record.first_token_ms);
        if let Err(error) = super::usage::log_usage_entry(&entry) {
            tracing::warn!(%error, "proxy usage persistence failed");
        }
    }
    fn record_failure(&self, tool: &str, model: &str, provider: &str, class: &str) {
        if let Err(error) = super::usage::log_usage_failure(tool, model, Some(provider), class) {
            tracing::warn!(%error, "proxy failure persistence failed");
        }
    }
}

// ─── 协同 agent 总线端点（C/D/E：agent 主动互聊 / 消息 / 任务流） ───
// 鉴权：X-Collab-Token 必须等于该房间的 bus token（由 dispatch_to_tool 注入到被派发 agent 的环境）。

async fn collab_agent_send_handler(
    State(state): State<ProxyState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let room_id = body
        .get("room_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let token = headers
        .get("X-Collab-Token")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if token != super::collab::room_bus_token(&room_id) {
        return (StatusCode::FORBIDDEN, "invalid token").into_response();
    }
    let app = match &state.app_handle {
        Some(a) => a.clone(),
        None => return (StatusCode::INTERNAL_SERVER_ERROR, "no app handle").into_response(),
    };
    let from = body
        .get("from")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let to = body
        .get("to")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let message = body
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let parent = body
        .get("parent")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let request_id =
        super::collab::agent_delegate(app, &room_id, &from, &to, &message, parent.as_deref()).await;
    match super::collab::await_delegation(&request_id, 1800).await {
        Some((status, result)) => {
            Json(json!({ "status": status, "result": result })).into_response()
        }
        None => (StatusCode::REQUEST_TIMEOUT, "delegation not found").into_response(),
    }
}

async fn collab_agent_message_handler(
    State(state): State<ProxyState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let room_id = body
        .get("room_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let token = headers
        .get("X-Collab-Token")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if token != super::collab::room_bus_token(&room_id) {
        return (StatusCode::FORBIDDEN, "invalid token").into_response();
    }
    let app = match &state.app_handle {
        Some(a) => a.clone(),
        None => return (StatusCode::INTERNAL_SERVER_ERROR, "no app handle").into_response(),
    };
    let from = body
        .get("from")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let message = body
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let parent = body
        .get("parent")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    super::collab::collab_post_message(app, &room_id, &from, &message, parent.as_deref());
    Json(json!({ "ok": true })).into_response()
}

async fn collab_agent_task_handler(
    State(state): State<ProxyState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let room_id = body
        .get("room_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let token = headers
        .get("X-Collab-Token")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if token != super::collab::room_bus_token(&room_id) {
        return (StatusCode::FORBIDDEN, "invalid token").into_response();
    }
    let app = match &state.app_handle {
        Some(a) => a.clone(),
        None => return (StatusCode::INTERNAL_SERVER_ERROR, "no app handle").into_response(),
    };
    let res = super::collab::collab_task_op(app, &room_id, &body);
    Json(res).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_registered_bus_routes_bypass_proxy_auth() {
        let hooks = AppProxyHooks;
        for path in [
            "/collab/agent/send",
            "/collab/agent/message",
            "/collab/agent/task",
        ] {
            assert!(hooks.independently_authenticated(path));
        }
        for path in [
            "/collab",
            "/collab/unknown",
            "/collaboration",
            "/v1/messages",
        ] {
            assert!(!hooks.independently_authenticated(path));
        }
        let _: Router<ProxyState> = hooks.extend_routes(Router::new());
    }
}
