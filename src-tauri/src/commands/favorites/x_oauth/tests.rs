use super::*;

fn sample_token(expires_at: i64) -> XToken {
    XToken {
        client_id: "cid".into(),
        access_token: "at".into(),
        refresh_token: Some("rt".into()),
        expires_at,
        user_id: "42".into(),
        screen_name: "alice".into(),
        name: "Alice".into(),
    }
}

#[test]
fn code_challenge_matches_rfc7636_vector() {
    // RFC 7636 附录 B 的官方测试向量
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    assert_eq!(
        code_challenge(verifier),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
}

#[test]
fn code_verifier_is_url_safe_and_long_enough() {
    let verifier = code_verifier().unwrap();
    assert!(
        verifier.len() >= 43,
        "PKCE verifier 至少 43 字符，实际 {}",
        verifier.len()
    );
    assert!(
        verifier
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-._~".contains(c)),
        "verifier 只能含 URL-safe 字符: {verifier}"
    );
}

#[test]
fn authorize_url_carries_pkce_params() {
    let url = authorize_url("cid", "st", "chal");
    assert!(url.starts_with(AUTHORIZE_URL));
    for needle in [
        "client_id=cid",
        "response_type=code",
        "code_challenge=chal",
        "code_challenge_method=S256",
        "state=st",
        "bookmark.read",
        "offline.access",
    ] {
        assert!(url.contains(needle), "缺少 {needle}: {url}");
    }
    assert!(
        url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A17890%2Fcallback"),
        "回调地址必须精确编码: {url}"
    );
}

#[test]
fn parse_token_response_uses_expires_in() {
    let body =
        r#"{"access_token":"at","refresh_token":"rt","expires_in":7200,"scope":"tweet.read"}"#;
    let token = parse_token_response(body, 1000).unwrap();
    assert_eq!(token.access_token, "at");
    assert_eq!(token.refresh_token.as_deref(), Some("rt"));
    assert_eq!(token.expires_at, 8200);
}

#[test]
fn parse_token_response_reports_error_body() {
    let err = parse_token_response(r#"{"error":"invalid_grant"}"#, 0).unwrap_err();
    assert!(
        err.contains("invalid_grant") || err.contains("access_token"),
        "实际: {err}"
    );
}

#[test]
fn token_needs_refresh_inside_safety_margin() {
    assert!(
        token_needs_refresh(&sample_token(100), 0),
        "剩余 100 秒小于 120 秒边界，应刷新"
    );
    assert!(
        token_needs_refresh(&sample_token(1000), 900),
        "剩余 100 秒小于 120 秒边界，应刷新"
    );
    assert!(
        !token_needs_refresh(&sample_token(1000), 0),
        "剩余 1000 秒不该刷新"
    );
}

/// 手动粘贴 token 时最常见的错误是漏字符/带空格，保存前必须挡住空值。
#[test]
fn manual_token_requires_client_id_and_access_token() {
    assert!(validate_manual_token("", "at").is_err(), "缺 client_id 应拒绝");
    assert!(validate_manual_token("cid", "").is_err(), "缺 access_token 应拒绝");
    assert!(validate_manual_token("  ", "  ").is_err(), "纯空白应拒绝");
    assert!(validate_manual_token("cid", "at").is_ok(), "两项齐全应通过");
}

/// token 等同于密码，落库必须是密文。
#[test]
fn encode_token_encrypts_at_rest() {
    let encoded = encode_token(&sample_token(1000)).expect("编码应成功");
    assert!(encoded.starts_with("ENC_V2:"), "必须是密文形态: {encoded}");
    assert!(!encoded.contains("\"access_token\""), "密文里不能出现明文字段");
    assert_eq!(decode_token(&encoded).expect("解码应成功").access_token, "at");
}

/// 旧版本把 token 明文写进 DB，升级后必须还能读出来（下次保存会自动转密文）。
#[test]
fn decode_token_accepts_legacy_plaintext() {
    let legacy = serde_json::to_string(&sample_token(1000)).unwrap();
    let token = decode_token(&legacy).expect("旧明文应仍可读");
    assert_eq!(token.client_id, "cid");
    assert_eq!(token.screen_name, "alice");
}

#[test]
fn decode_token_rejects_garbage() {
    assert!(decode_token("not json").is_err());
}
