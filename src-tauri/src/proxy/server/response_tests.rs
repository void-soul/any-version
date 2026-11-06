use super::*;
use std::sync::Arc;
use tokio::sync::RwLock;

async fn aggregate(
    parts: Vec<Result<&'static str, std::io::Error>>,
) -> (Response, super::super::ProxyState) {
    aggregate_bytes(
        parts
            .into_iter()
            .map(|part| part.map(|text| text.as_bytes().to_vec()))
            .collect(),
    )
    .await
}

async fn aggregate_bytes(
    parts: Vec<Result<Vec<u8>, std::io::Error>>,
) -> (Response, super::super::ProxyState) {
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
    let response =
        aggregate_upstream_stream(&state, upstream, &config, "openai", "openai", "m", "m").await;
    (response, state)
}

#[tokio::test]
async fn unicode_aggregate_survives_every_network_split_and_multiline_events() {
    let wire = "\u{feff}: ping\r\ndata: {\"choices\":\r\ndata: [{\"delta\":{\"content\":\"中文😀\"}}]}\r\n\r\ndata: {\"choices\":[{\"delta\":{\"content\":\"second\"}}]}\n\ndata: [DONE]\r\r".as_bytes();
    for cut in 0..=wire.len() {
        let (response, _) =
            aggregate_bytes(vec![Ok(wire[..cut].to_vec()), Ok(wire[cut..].to_vec())]).await;
        assert_eq!(response.status(), StatusCode::OK, "cut={cut}");
        let bytes = axum::body::to_bytes(response.into_body(), 65536)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            value["choices"][0]["message"]["content"], "中文😀second",
            "cut={cut}"
        );
    }
}

#[tokio::test]
async fn complete_json_tail_is_compatible_but_incomplete_utf8_fails() {
    let (response, _) = aggregate(vec![
        Ok(TEXT),
        Ok("data: {\"choices\":[{\"finish_reason\":\"stop\",\"delta\":{}}]}"),
    ])
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let (response, state) = aggregate_bytes(vec![
        Ok(TEXT.as_bytes().to_vec()),
        Ok(b"data: \xe4\xb8".to_vec()),
    ])
    .await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(state.stats.read().await.success_requests, 0);
}

#[tokio::test]
async fn observer_preserves_raw_bytes_and_cross_protocol_keeps_unicode() {
    let wire = "data: {\"choices\":[{\"delta\":{\"content\":\"中文😀\"}}]}\r\n\r\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n".as_bytes();
    for inbound in ["openai", "anthropic"] {
        let (_, state) = aggregate(vec![Ok(TEXT), Ok(FINISH)]).await;
        *state.stats.write().await = Default::default();
        let config = ProxyConfig::default();
        let body = reqwest::Body::wrap_stream(futures_util::stream::iter(
            wire.chunks(1)
                .map(|byte| Ok::<_, std::io::Error>(byte.to_vec()))
                .collect::<Vec<_>>(),
        ));
        let response = stream_response(
            &state,
            reqwest::Response::from(axum::http::Response::new(body)),
            &config,
            inbound,
            "openai",
            "m",
            "m",
        )
        .await;
        let bytes = axum::body::to_bytes(response.into_body(), 65536)
            .await
            .unwrap();
        if inbound == "openai" {
            assert_eq!(&bytes[..], wire);
        } else {
            assert!(std::str::from_utf8(&bytes).unwrap().contains("中文😀"));
        }
        assert_eq!(state.stats.read().await.success_requests, 1);
    }
}

#[tokio::test]
async fn partial_utf8_tail_fails_observation_and_conversion_without_fake_finish() {
    for inbound in ["openai", "anthropic"] {
        let (_, state) = aggregate(vec![Ok(TEXT), Ok(FINISH)]).await;
        *state.stats.write().await = Default::default();
        let config = ProxyConfig::default();
        let parts = vec![
            Ok::<_, std::io::Error>(TEXT.as_bytes().to_vec()),
            Ok(b"data: \xe4\xb8".to_vec()),
        ];
        let body = reqwest::Body::wrap_stream(futures_util::stream::iter(parts));
        let response = stream_response(
            &state,
            reqwest::Response::from(axum::http::Response::new(body)),
            &config,
            inbound,
            "openai",
            "m",
            "m",
        )
        .await;
        let result = axum::body::to_bytes(response.into_body(), 65536).await;
        if inbound == "openai" {
            assert!(result.is_err());
        } else {
            let bytes = result.unwrap();
            let text = std::str::from_utf8(&bytes).unwrap();
            assert!(text.contains("event: error"));
            assert!(!text.contains("message_stop"));
        }
        let stats = state.stats.read().await;
        assert_eq!((stats.success_requests, stats.failed_requests), (0, 1));
    }
}

const TEXT: &str = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"half\"}}]}\n\n";
const FINISH: &str =
    "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n";

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
