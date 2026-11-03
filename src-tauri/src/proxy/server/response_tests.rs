use super::*;
use std::sync::Arc;
use tokio::sync::RwLock;

async fn aggregate(parts: Vec<Result<&'static str, std::io::Error>>) -> (Response, super::super::ProxyState) {
    let config = ProxyConfig::default();
    let state = super::super::ProxyState {
        config: Arc::new(RwLock::new(config.clone())),
        client: reqwest::Client::new(),
        stats: Arc::new(RwLock::new(super::super::ProxyStats::default())),
        app_handle: None,
        collab_room_id: None,
        tool_id: String::new(),
    };
    let body = reqwest::Body::wrap_stream(futures_util::stream::iter(parts));
    let upstream = reqwest::Response::from(axum::http::Response::new(body));
    let response = aggregate_upstream_stream(&state, upstream, &config, "openai", "openai", "m", "m").await;
    (response, state)
}

const TEXT: &str = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"half\"}}]}\n\n";
const FINISH: &str = "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n";

#[tokio::test]
async fn truncated_aggregate_is_not_reported_as_success() {
    let (response, state) = aggregate(vec![Ok(TEXT)]).await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let stats = state.stats.read().await;
    assert_eq!((stats.success_requests, stats.failed_requests), (0, 1));
}

#[tokio::test]
async fn upstream_error_frames_and_transport_errors_fail_aggregation() {
    for tail in [
        Ok("data: {\"error\":{\"message\":\"quota denied\"}}\n\ndata: [DONE]\n\n"),
        Err(std::io::Error::from(std::io::ErrorKind::ConnectionReset)),
    ] {
        let (response, state) = aggregate(vec![Ok(TEXT), tail]).await;
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(state.stats.read().await.success_requests, 0);
    }
}

#[tokio::test]
async fn role_or_usage_only_stream_is_not_a_completion() {
    let (response, _) = aggregate(vec![Ok("data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\ndata: {\"usage\":{\"total_tokens\":1}}\n\ndata: [DONE]\n\n")]).await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn finish_or_done_completes_an_aggregate() {
    for tail in [FINISH, "data: [DONE]\n\n"] {
        let (response, state) = aggregate(vec![Ok(TEXT), Ok(tail)]).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(state.stats.read().await.success_requests, 1);
    }
}
