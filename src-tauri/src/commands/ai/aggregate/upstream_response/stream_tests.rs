use super::*;

const TEXT: &str = "data: {\"choices\":[{\"delta\":{\"content\":\"中文😀\"}}]}\n\n";
const FINISH: &str = "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n";

async fn render(
    parts: Vec<Result<Vec<u8>, std::io::Error>>,
    inbound: &'static str,
    outbound: Outbound,
    hang: bool,
) -> String {
    use futures_util::StreamExt;
    let stream = futures_util::stream::iter(parts)
        .chain(futures_util::stream::pending().take(if hang { 1 } else { 0 }));
    let response = reqwest::Response::from(axum::http::Response::new(reqwest::Body::wrap_stream(
        stream,
    )));
    let candidate = AggCandidate {
        provider_id: "fixture".into(),
        provider_name: "fixture".into(),
        outbound,
        base_url: String::new(),
        api_key: String::new(),
        model_id: "fixture-model".into(),
        include_v1: None,
        headers: vec![],
        supports_image: None,
    };
    let response = cross_protocol_response(
        None,
        candidate,
        inbound,
        response,
        std::time::Instant::now(),
        std::time::Duration::from_millis(20),
    );
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    String::from_utf8(
        axum::body::to_bytes(response.into_body(), 65536)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap()
}
fn part(text: &str) -> Result<Vec<u8>, std::io::Error> {
    Ok(text.as_bytes().to_vec())
}

#[tokio::test]
async fn partial_eof_transport_error_and_timeout_never_manufacture_message_stop() {
    for (tail, hang) in [
        (None, false),
        (
            Some(Err(std::io::Error::from(
                std::io::ErrorKind::ConnectionReset,
            ))),
            false,
        ),
        (None, true),
    ] {
        let mut parts = vec![part(TEXT)];
        if let Some(tail) = tail {
            parts.push(tail);
        }
        let output = render(parts, "anthropic", Outbound::OpenAi, hang).await;
        assert!(output.contains("event: error"), "{output}");
        assert!(
            output.contains("content_block_stop"),
            "open content must close: {output}"
        );
        assert!(!output.contains("message_stop"), "{output}");
        assert!(!output.contains("end_turn"), "{output}");
    }
}

#[tokio::test]
async fn finish_or_done_closes_anthropic_once_and_ignores_post_done_garbage() {
    for tail in [FINISH, "data: [DONE]\n\ndata: not-json\n\n"] {
        let output = render(
            vec![part(TEXT), part(tail)],
            "anthropic",
            Outbound::OpenAi,
            false,
        )
        .await;
        assert_eq!(output.matches("event: message_stop").count(), 1, "{output}");
        assert!(!output.contains("event: error"), "{output}");
        assert!(
            !output.contains("data: [DONE]"),
            "Anthropic client must receive protocol completion: {output}"
        );
    }
}

#[tokio::test]
async fn malformed_frames_error_payloads_and_utf8_tails_fail_without_normal_completion() {
    for tail in [
        b"data: {bad}\n\n".to_vec(),
        b"data: {\"error\":{\"message\":\"bad\"}}\n\n".to_vec(),
        b"data: \xe4\xb8".to_vec(),
    ] {
        let output = render(
            vec![part(TEXT), Ok(tail)],
            "anthropic",
            Outbound::OpenAi,
            false,
        )
        .await;
        assert!(output.contains("event: error"), "{output}");
        assert!(!output.contains("message_stop"), "{output}");
    }
}

#[tokio::test]
async fn bytewise_unicode_tool_arguments_are_preserved() {
    let wire="\u{feff}: ping\r\ndata: {\"choices\":\r\ndata: [{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-a\",\"type\":\"function\",\"function\":{\"name\":\"read\",\"arguments\":\"{\\\"path\\\":\\\"中文😀\\\"}\"}}]}}]}\r\n\r\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\r\r".as_bytes();
    let output = render(
        wire.chunks(1).map(|byte| Ok(byte.to_vec())).collect(),
        "anthropic",
        Outbound::OpenAi,
        false,
    )
    .await;
    assert!(output.contains("中文😀"), "{output}");
    assert!(output.contains("input_json_delta"), "{output}");
    assert_eq!(output.matches("event: message_stop").count(), 1, "{output}");
}

#[tokio::test]
async fn anthropic_source_truncation_fails_in_openai_protocol() {
    let start="event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"m\",\"model\":\"m\",\"usage\":{\"input_tokens\":0}}}\n\n";
    let text="data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"中文😀\"}}\n\n";
    let output = render(
        vec![part(start), part(text)],
        "openai",
        Outbound::Anthropic,
        false,
    )
    .await;
    assert!(output.contains("upstream_parse"), "{output}");
    assert!(!output.contains("\"finish_reason\":\"stop\""), "{output}");
    assert_eq!(output.matches("data: [DONE]").count(), 1, "{output}");
}
