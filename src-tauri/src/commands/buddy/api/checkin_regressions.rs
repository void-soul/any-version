use super::*;

#[test]
fn transient_business_code_is_never_completed() {
    for status in [200, 400, 500] {
        let text = r#"{"code":10001,"msg":"签到失败，请稍后重试"}"#;
        match decode_checkin_response(status, text) {
            Ok(result) => {
                assert!(!result.success);
                assert_eq!(result.already, None);
                assert!(result.retryable);
            }
            Err(error) => {
                assert!(error.contains("(code=10001)"));
                assert!(crate::commands::buddy::auto_checkin::checkin_error_is_retryable(&error));
            }
        }
    }
}
#[test]
fn affirmative_completion_and_inactivity_are_distinct() {
    for text in [
        "今天已签到",
        "今日已完成签到",
        "已经领取",
        "already checked in",
    ] {
        let body = json!({"code":10001,"msg":text}).to_string();
        let result = decode_checkin_response(200, &body).unwrap();
        assert!(result.success);
        assert_eq!(result.already, Some(true));
        assert!(!result.retryable);
    }
    let inactive =
        decode_checkin_response(200, r#"{"code":10001,"msg":"签到活动已结束"}"#).unwrap();
    assert_eq!(inactive.inactive, Some(true));
    assert!(!inactive.success);
    assert!(!inactive.retryable);
    let hard = decode_checkin_response(200, r#"{"code":10000,"msg":"参数错误"}"#).unwrap();
    assert!(!hard.success);
    assert!(!hard.retryable);
}
#[test]
fn errors_keep_http_status_even_when_body_is_not_json() {
    for (status, expected) in [(500, true), (502, true), (400, false), (401, false)] {
        let error = decode_checkin_response(status, "<html>gateway</html>").unwrap_err();
        assert_eq!(
            crate::commands::buddy::auto_checkin::checkin_error_is_retryable(&error),
            expected
        );
    }
    assert!(
        !crate::commands::buddy::auto_checkin::checkin_error_is_retryable(&checkin_response_error(
            400,
            Some(10000),
            "参数错误"
        ))
    );
}

#[test]
fn authentication_wins_over_business_completion_messages() {
    for status in [401, 403] {
        for message in ["今天已签到", "签到活动已结束"] {
            let error =
                decode_checkin_response(status, &json!({"code":10001,"msg":message}).to_string())
                    .unwrap_err();
            assert!(error.starts_with(AUTH_EXPIRED_PREFIX));
            assert!(!crate::commands::buddy::auto_checkin::checkin_error_is_retryable(&error));
        }
    }
}

#[test]
fn status_decoder_keeps_transport_http_and_business_refusals() {
    for status in [500, 502] {
        assert!(is_retryable_checkin_error(
            &decode_checkin_status_response(status, "<html>gateway</html>").unwrap_err()
        ));
    }
    assert!(is_retryable_checkin_error(&checkin_transport_error(
        "checkin-status",
        "request timed out"
    )));
    assert!(is_retryable_checkin_error(&checkin_transport_error(
        "checkin-status (http=200)",
        "body reset"
    )));
    assert!(is_retryable_checkin_error(
        &decode_checkin_status_response(200, r#"{"code":10001,"msg":"稍后重试"}"#).unwrap_err()
    ));
    assert!(!is_retryable_checkin_error(
        &decode_checkin_status_response(401, "unauthorized").unwrap_err()
    ));
}
#[test]
fn fallback_failure_keeps_retryable_status_but_authentication_wins() {
    let missing = checkin_response_error(404, None, "unsupported route");
    let busy = checkin_response_error(500, Some(10001), "later");
    let error = combine_checkin_status_errors(&missing, &busy);
    assert!(is_retryable_checkin_error(&error));
    assert_eq!(checkin_error_http_status(&error), Some(500));
    let auth = checkin_response_error(401, None, "unauthorized");
    for error in [
        combine_checkin_status_errors(&auth, &busy),
        combine_checkin_status_errors(&busy, &auth),
    ] {
        assert!(error.starts_with(AUTH_EXPIRED_PREFIX));
        assert!(!is_retryable_checkin_error(&error));
    }
}
