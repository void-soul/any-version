//! OTP 算法实现（复刻 CloudOTP / RFC4226 / RFC6238）。
//!
//! 支持 5 种令牌类型：
//! - TOTP（时间型，Google Authenticator 兼容）
//! - HOTP（计数器型）
//! - MOTP（Mobile-OTP）
//! - Steam（5 位字母数字）
//! - Yandex（8 位小写字母）

use hmac::{Hmac, Mac};
use sha1::Sha1;
use sha2::{Sha256, Sha512};

/// 令牌类型
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TokenType {
    Totp,
    Hotp,
    Motp,
    Steam,
    Yandex,
}

impl TokenType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "TOTP" => Some(TokenType::Totp),
            "HOTP" => Some(TokenType::Hotp),
            "MOTP" => Some(TokenType::Motp),
            "STEAM" => Some(TokenType::Steam),
            "YAOTP" | "YANDEX" => Some(TokenType::Yandex),
            _ => None,
        }
    }
}

/// 哈希算法
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HashAlgo {
    Sha1,
    Sha256,
    Sha512,
}

impl HashAlgo {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "SHA1" => Some(HashAlgo::Sha1),
            "SHA256" => Some(HashAlgo::Sha256),
            "SHA512" => Some(HashAlgo::Sha512),
            _ => None,
        }
    }
}

/// Base32 编码（RFC4648，无 padding），用于 otpauth-migration 的 secret 字节转字符串。
pub fn base32_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::new();
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    for &b in bytes {
        acc = (acc << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((acc >> bits) & 0x1f) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((acc << (5 - bits)) & 0x1f) as usize] as char);
    }
    out
}

/// Base32 解码（RFC4648，宽容处理大小写/空白/padding），失败返回 None。
fn decode_base32(secret: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    let mut out = Vec::new();
    for c in secret.chars() {
        if c.is_whitespace() || c == '=' {
            continue;
        }
        let c = c.to_ascii_uppercase();
        let pos = ALPHABET.iter().position(|&a| a as char == c)?;
        acc = (acc << 5) | pos as u32;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// 动态截断（RFC4226 核心）
fn dynamic_truncate(digest: &[u8]) -> u32 {
    let offset = (digest[digest.len() - 1] & 0x0f) as usize;
    let binary = ((digest[offset] & 0x7f) as u32) << 24
        | ((digest[offset + 1] & 0xff) as u32) << 16
        | ((digest[offset + 2] & 0xff) as u32) << 8
        | (digest[offset + 3] & 0xff) as u32;
    binary
}

/// 计算 HMAC，返回动态截断后的整数（按算法选择）。
fn hotp_value(secret: &[u8], counter: u64, algo: HashAlgo) -> u32 {
    let counter_bytes = counter.to_be_bytes();
    match algo {
        HashAlgo::Sha1 => {
            let mut mac = Hmac::<Sha1>::new_from_slice(secret).unwrap();
            mac.update(&counter_bytes);
            dynamic_truncate(&mac.finalize().into_bytes())
        }
        HashAlgo::Sha256 => {
            let mut mac = Hmac::<Sha256>::new_from_slice(secret).unwrap();
            mac.update(&counter_bytes);
            dynamic_truncate(&mac.finalize().into_bytes())
        }
        HashAlgo::Sha512 => {
            let mut mac = Hmac::<Sha512>::new_from_slice(secret).unwrap();
            mac.update(&counter_bytes);
            dynamic_truncate(&mac.finalize().into_bytes())
        }
    }
}

fn pad_code(code: u32, digits: usize) -> String {
    let modulus = 10u32.pow(digits as u32);
    format!("{:0width$}", code % modulus, width = digits)
}

/// TOTP：time 为毫秒时间戳。
pub fn totp(secret: &str, time_ms: i64, digits: usize, interval: u64, algo: HashAlgo) -> String {
    let counter = ((time_ms / 1000) as u64) / interval;
    let key = decode_base32(secret).unwrap_or_default();
    if key.is_empty() {
        return "ERROR".to_string();
    }
    pad_code(hotp_value(&key, counter, algo), digits)
}

/// HOTP：counter 为计数器。
pub fn hotp(secret: &str, counter: u64, digits: usize, algo: HashAlgo) -> String {
    let key = decode_base32(secret).unwrap_or_default();
    if key.is_empty() {
        return "ERROR".to_string();
    }
    pad_code(hotp_value(&key, counter, algo), digits)
}

/// Steam TOTP：5 位字母数字，字符集 "23456789BCDFGHJKMNPQRTVWXY"。
pub fn steam(secret: &str, time_ms: i64) -> String {
    const STEAM_CHARS: &[u8] = b"23456789BCDFGHJKMNPQRTVWXY";
    let key = decode_base32(secret).unwrap_or_default();
    if key.is_empty() {
        return "ERROR".to_string();
    }
    let counter = ((time_ms / 1000) as u64) / 30;
    let counter_bytes = counter.to_be_bytes();
    let mut mac = Hmac::<Sha1>::new_from_slice(&key).unwrap();
    mac.update(&counter_bytes);
    let digest = mac.finalize().into_bytes();
    // Steam 特殊：offset 是最后一个字节 % 0xFF（不是 & 0x0f）
    let b = (digest[19] as usize) & 0xff;
    let mut code_point = ((digest[b] & 0x7f) as u32) << 24
        | ((digest[b + 1] & 0xff) as u32) << 16
        | ((digest[b + 2] & 0xff) as u32) << 8
        | (digest[b + 3] & 0xff) as u32;

    let mut code = String::new();
    for _ in 0..5 {
        code.push(STEAM_CHARS[(code_point % STEAM_CHARS.len() as u32) as usize] as char);
        code_point /= STEAM_CHARS.len() as u32;
    }
    code
}

/// MOTP：md5("{time/period}{secret}{pin}") 取前 digits 位。
pub fn motp(secret: &str, pin: &str, time_ms: i64, period: u64, digits: usize) -> String {
    let counter = ((time_ms / 1000) as u64) / period;
    let input = format!("{}{}{}", counter, secret, pin);
    let digest = md5::compute(input.as_bytes());
    let hex = format!("{:x}", digest);
    hex.chars().take(digits).collect()
}

/// Yandex OTP：key = sha256(pin + base32_decode(secret))，8 位小写字母。
pub fn yandex(secret: &str, pin: &str, time_ms: i64) -> String {
    use sha2::Digest;
    let decoded = decode_base32(secret).unwrap_or_default();
    if decoded.is_empty() {
        return "ERROR".to_string();
    }
    let mut hasher = Sha256::new();
    hasher.update(pin.as_bytes());
    hasher.update(&decoded);
    let key = hasher.finalize();

    let counter = ((time_ms / 1000) as u64) / 30;
    let counter_bytes = counter.to_be_bytes();
    let mut mac = Hmac::<Sha256>::new_from_slice(&key).unwrap();
    mac.update(&counter_bytes);
    let digest = mac.finalize().into_bytes();

    let offset = (digest[digest.len() - 1] & 0x0f) as usize;
    // 取 8 字节（Yandex 与标准 RFC 略不同，取 offset 起的 8 字节转 i64）
    let mut otp: u64 = 0;
    for i in 0..8 {
        otp = (otp << 8) | (digest[offset + i] as u64);
    }
    otp &= 0x7fff_ffff_ffff_ffff;

    // 转 8 位小写字母（base26）
    let mut code = String::new();
    let mut v = otp;
    for _ in 0..8 {
        code.insert(0, (b'a' + (v % 26) as u8) as char);
        v /= 26;
    }
    code
}

/// 统一入口：根据类型生成当前验证码。
pub fn generate(
    token_type: TokenType,
    secret: &str,
    pin: &str,
    digits: usize,
    period: u64,
    counter: u64,
    algo: HashAlgo,
    time_ms: i64,
) -> String {
    match token_type {
        TokenType::Totp => totp(secret, time_ms, digits, period, algo),
        TokenType::Hotp => hotp(secret, counter, digits, algo),
        TokenType::Motp => motp(secret, pin, time_ms, period, digits),
        TokenType::Steam => steam(secret, time_ms),
        TokenType::Yandex => yandex(secret, pin, time_ms),
    }
}

/// 计算距离下一个周期还剩余多少秒（用于倒计时）。
pub fn remaining_seconds(period: u64, time_ms: i64) -> u64 {
    let elapsed = ((time_ms / 1000) as u64) % period;
    period - elapsed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_totp_rfc6238() {
        // RFC 6238 测试向量：secret "12345678901234567890" (base32 of "12345678901234567890")
        // 使用标准测试 secret "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"（即 "12345678901234567890" 的 base32）
        let secret = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
        // T=59s, SHA1, 8 digits -> 94287082
        let code = totp(secret, 59_000, 8, 30, HashAlgo::Sha1);
        assert_eq!(code, "94287082");
        // T=1111111109, SHA256, 8 digits -> 67062674
        let code = totp(secret, 1_111_111_109_000, 8, 30, HashAlgo::Sha256);
        assert_eq!(code, "67062674");
    }

    #[test]
    fn test_steam() {
        // 已知 Steam secret 的测试向量（来自开源实现）
        let secret = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
        let code = steam(secret, 59_000);
        assert_eq!(code.len(), 5);
    }
}
