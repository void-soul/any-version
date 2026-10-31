//! Credential conventions shared by both HTTP adapters and pull examples.
pub fn api_key_for_sign(key: &str) -> &str {
    key.trim()
}
pub fn bearer_authorization(token: &str) -> String {
    format!("Bearer {}", token.trim())
}
pub fn validate_token(token: &str) -> Result<(), String> {
    if matches!(token.trim(), "" | "changeme" | "YOUR_SHARED_TOKEN") {
        Err("共享 Token 不能为空，也不能用示例值".into())
    } else {
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credentials_use_identical_whitespace_and_placeholder_rules() {
        assert_eq!(api_key_for_sign("  secret\r\n"), "secret");
        assert_eq!(bearer_authorization("  secret\r\n"), "Bearer secret");
        for value in ["", "  ", "changeme", " YOUR_SHARED_TOKEN "] {
            assert!(validate_token(value).is_err());
        }
        assert!(validate_token("isolation-only-value").is_ok());
    }
}
