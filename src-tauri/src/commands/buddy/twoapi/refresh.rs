use super::credentials::Account;
use std::time::{Duration, Instant};

#[derive(Clone)]
pub(super) struct Tokens {
    pub access: String,
    pub refresh: Option<String>,
    pub expires_at: Option<i64>,
}

pub(super) struct Failure {
    pub message: String,
    pub terminal: bool,
    pub wait: Option<Duration>,
}
impl Failure {
    pub fn temporary(message: impl Into<String>) -> Self {
        Self { message: message.into(), terminal: false, wait: None }
    }
}

#[derive(Default)]
pub(super) struct Policy {
    key: String,
    attempts: u32,
    until: Option<Instant>,
    terminal: bool,
    message: String,
}
impl Policy {
    pub fn check(&mut self, key: &str, now: Instant) -> Result<(), String> {
        if self.key != key { *self = Self { key: key.into(), ..Self::default() }; }
        if self.terminal { return Err(format!("{}；自动刷新已停止，请重新登录", self.message)); }
        if let Some(left) = self.until.and_then(|until| until.checked_duration_since(now)).filter(|left| !left.is_zero()) {
            return Err(format!("{}；{}秒后重试", self.message, left.as_secs().max(1)));
        }
        Ok(())
    }
    pub fn failed(&mut self, failure: &Failure, now: Instant) {
        self.attempts = self.attempts.saturating_add(1);
        let seconds = match self.attempts { 1 => 5, 2 => 15, 3 => 60, _ => 300 };
        self.until = Some(now + failure.wait.unwrap_or(Duration::from_secs(seconds)));
        self.terminal = failure.terminal;
        self.message = failure.message.clone();
    }
}

pub(super) fn identity(account: &Account) -> String {
    // Only a digest is retained by the retry policy; never print credentials.
    format!("{:x}", md5::compute(format!("{}\0{}\0{}\0{}\0{}", account.uid, account.access_token, account.refresh_token, account.domain, account.expires_at_ms)))
}

fn retry_after(value: Option<&reqwest::header::HeaderValue>, now: std::time::SystemTime) -> Duration {
    let seconds = value.and_then(|value| value.to_str().ok()).and_then(|text| {
        text.parse::<u64>().ok().or_else(|| {
            let date = chrono::DateTime::parse_from_rfc2822(text).ok()?;
            let now: chrono::DateTime<chrono::Utc> = now.into();
            Some(date.timestamp().saturating_sub(now.timestamp()).max(0) as u64)
        })
    }).unwrap_or(300).clamp(300, 6 * 60 * 60);
    Duration::from_secs(seconds)
}

pub(super) async fn fetch(account: Account) -> Result<Tokens, Failure> {
    if account.refresh_token.trim().is_empty() {
        return Err(Failure {message:"缺少刷新凭据".into(), terminal:true, wait:None});
    }
    let url = format!("{}/v2/plugin/auth/token/refresh", super::upstream::BACKEND_BASE);
    let response = reqwest::Client::new().post(url)
        .bearer_auth(&account.access_token)
        .header("X-User-Id", &account.uid).header("X-Enterprise-Id", &account.enterprise_id)
        .header("X-Tenant-Id", &account.enterprise_id).header("X-Domain", &account.domain)
        .header("X-Refresh-Token", &account.refresh_token).header("X-Auth-Refresh-Source", "plugin")
        .header("Content-Type", "application/json").timeout(Duration::from_secs(20))
        .send().await.map_err(|_| Failure::temporary("刷新token网络失败"))?;
    decode(response).await
}

async fn decode(response: reqwest::Response) -> Result<Tokens, Failure> {
    let status = response.status();
    let wait = (status.as_u16() == 429).then(|| retry_after(response.headers().get("Retry-After"), std::time::SystemTime::now()));
    let body: serde_json::Value = response.json().await.map_err(|_| Failure {
        message: format!("刷新响应不是JSON（HTTP {}）", status.as_u16()), terminal:false, wait,
    })?;
    if !status.is_success() || body.get("code").and_then(|code| code.as_i64()) != Some(0) {
        let message = body.get("msg").or_else(|| body.get("message")).and_then(|value| value.as_str()).unwrap_or("");
        let terminal = super::super::api::is_revoked_refresh_error(message)
            || body.get("code").and_then(|value| value.as_i64()) == Some(12153);
        return Err(Failure {
            message: if terminal { "刷新凭据已失效".into() } else { format!("刷新token被拒（HTTP {}）", status.as_u16()) },
            terminal, wait,
        });
    }
    let data = body.get("data").unwrap_or(&serde_json::Value::Null);
    let access = data.get("accessToken").and_then(|value| value.as_str()).filter(|value| !value.is_empty())
        .ok_or_else(|| Failure::temporary("刷新响应缺少accessToken"))?.to_string();
    let expires_at = super::super::api::token_expiry_at(data).filter(|expiry| *expiry > 0).or_else(|| {
        use base64::Engine;
        let payload = access.split('.').nth(1)?;
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        value.get("exp")?.as_i64().filter(|seconds| *seconds > 0).map(|seconds| seconds.saturating_mul(1000))
    });
    Ok(Tokens {
        access,
        refresh: data.get("refreshToken").and_then(|value| value.as_str()).filter(|value| !value.is_empty()).map(str::to_string),
        expires_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_backoff_and_terminal_errors_reset_only_for_new_credentials() {
        let now = Instant::now(); let mut policy = Policy::default();
        policy.check("old", now).unwrap();
        for seconds in [5,15,60,300,300] {
            policy.failed(&Failure::temporary("temporary"), now);
            assert!(policy.check("old", now + Duration::from_secs(seconds - 1)).is_err());
            policy.check("old", now + Duration::from_secs(seconds)).unwrap();
        }
        policy.failed(&Failure {message:"expired".into(), terminal:true, wait:None}, now);
        assert!(policy.check("old", now + Duration::from_secs(86400)).is_err());
        policy.check("new", now).unwrap();
    }
    #[test]
    fn retry_after_accepts_seconds_and_http_dates_with_bounds() {
        let value = |text| reqwest::header::HeaderValue::from_static(text);
        assert_eq!(retry_after(Some(&value("900")), std::time::UNIX_EPOCH).as_secs(), 900);
        assert_eq!(retry_after(Some(&value("1")), std::time::UNIX_EPOCH).as_secs(), 300);
        assert_eq!(retry_after(Some(&value("999999")), std::time::UNIX_EPOCH).as_secs(), 21600);
        assert_eq!(retry_after(Some(&value("Thu, 01 Jan 1970 00:15:00 GMT")), std::time::UNIX_EPOCH).as_secs(), 900);
    }
    #[tokio::test]
    async fn refresh_decoder_keeps_rate_limits_and_only_explicit_revocation_is_terminal() {
        let response = |status, body: &'static str| reqwest::Response::from(
            axum::http::Response::builder().status(status).header("Retry-After", "900")
                .body(reqwest::Body::from(body)).unwrap()
        );
        let error = decode(response(429, "gateway html")).await.err().unwrap();
        assert_eq!(error.wait.unwrap().as_secs(), 900);
        assert!(!error.terminal);
        let error = decode(response(401, r#"{"code":12153,"msg":"refresh token is invalid"}"#)).await.err().unwrap();
        assert!(error.terminal);
        let error = decode(response(401, "gateway html")).await.err().unwrap();
        assert!(!error.terminal);
        let tokens = decode(response(200, r#"{"code":0,"data":{"accessToken":"new","refreshToken":"rotated","expiresIn":3600}}"#)).await.ok().unwrap();
        assert_eq!(tokens.refresh.as_deref(), Some("rotated"));
        assert!(tokens.expires_at.unwrap() > super::super::credentials::now_ms() + 3500_000);
    }
}
