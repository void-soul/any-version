use super::{algo, OtpToken};

/// Generate a current code without allowing malformed persisted parameters to panic native code.
pub(super) fn generate_code(token: OtpToken) -> Result<String, String> {
    let kind = algo::TokenType::from_str(&token.token_type).ok_or("未知令牌类型")?;
    let hash = algo::HashAlgo::from_str(&token.algorithm).ok_or("未知哈希算法")?;
    if token.secret.trim().is_empty() {
        return Err("密钥不能为空".into());
    }
    if !(1..=9).contains(&token.digits) {
        return Err("验证码位数必须在1到9之间".into());
    }
    if matches!(kind, algo::TokenType::Totp | algo::TokenType::Motp) && token.period <= 0 {
        return Err("验证码周期必须为正整数".into());
    }
    if kind == algo::TokenType::Hotp && token.counter < 0 {
        return Err("验证码计数器不能为负数".into());
    }
    Ok(algo::generate(
        kind,
        &token.secret,
        &token.pin,
        token.digits as usize,
        token.period as u64,
        token.counter as u64,
        hash,
        chrono::Utc::now().timestamp_millis(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> OtpToken {
        OtpToken {
            secret: "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ".into(),
            ..Default::default()
        }
    }
    #[test]
    fn malformed_parameters_return_errors_instead_of_division_or_power_panics() {
        for period in [0, -1] {
            let mut token = fixture();
            token.period = period;
            assert!(generate_code(token).is_err());
        }
        for digits in [0, -1, 10, i64::MAX] {
            let mut token = fixture();
            token.digits = digits;
            assert!(generate_code(token).is_err());
        }
        let mut token = fixture();
        token.token_type = "HOTP".into();
        token.counter = -1;
        assert!(generate_code(token).is_err());
    }
    #[test]
    fn native_special_types_generate_their_existing_contract_shapes() {
        let mut token = fixture();
        token.token_type = "MOTP".into();
        token.pin = "1234".into();
        let motp = generate_code(token.clone()).unwrap();
        assert_eq!(motp.len(), 6);
        assert!(motp.chars().all(|c| c.is_ascii_hexdigit()));
        token.token_type = "Yandex".into();
        token.digits = 8;
        let yandex = generate_code(token).unwrap();
        assert_eq!(yandex.len(), 8);
        assert!(yandex.chars().all(|c| c.is_ascii_lowercase()));
    }
}
