//! 鉴权凭据的唯一真源：加密安全随机字节 → hex。
//!
//! 设计红线：**RNG 失败必须报错并阻止服务启动，绝不回退到时间戳/进程号等可预测值**。
//! 本地监听服务（代理 / 证书接收端）的 token 一旦可预测，等于没有鉴权。

/// 生成 `n_bytes` 个加密随机字节的 hex 字符串（长度 = n_bytes * 2）。
///
/// 返回 `Err` 表示 OS RNG 不可用；调用方必须停止启动流程，不得用弱值替代。
pub fn secure_hex(n_bytes: usize) -> Result<String, String> {
    if n_bytes == 0 {
        return Err("随机字节数必须大于 0".into());
    }
    let mut buf = vec![0u8; n_bytes];
    getrandom::getrandom(&mut buf)
        .map_err(|e| format!("系统随机数不可用，无法生成鉴权凭据: {e}"))?;
    Ok(buf.iter().map(|b| format!("{:02x}", b)).collect())
}

/// 本地代理每次启动的鉴权 token（32 字节 → 64 位 hex）。
pub fn fresh_proxy_token() -> Result<String, String> {
    secure_hex(32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_has_expected_length_and_is_hex() {
        let token = fresh_proxy_token().unwrap();
        assert_eq!(token.len(), 64);
        assert!(token.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn two_tokens_differ() {
        assert_ne!(fresh_proxy_token().unwrap(), fresh_proxy_token().unwrap());
    }

    #[test]
    fn zero_length_is_rejected() {
        assert!(secure_hex(0).is_err());
    }
}
