//! 从 Cookie 里**尽力**解析过期时间，用于提前提醒用户换 Cookie。
//!
//! 定位是「锦上添花」：解析得出来就提前提醒，解析不出来（平台换了格式）也**不算错**——
//! 真正的兜底是运行期检测：请求拿到 401/403 就把状态标成 `expired`，
//! 所以这里的解析失败只会让提醒晚一点发生，不会让功能坏掉。
//!
//! 这也是不引 `jsonwebtoken` 之类依赖的原因：只为读一个 `exp` 字段不值得。

use base64::Engine;

use super::zhihu;

/// 按凭证键解析过期时间（unix 秒）。解析不出来返回 None。
pub fn parse_cookie_expiry(source: &str, cookie: &str) -> Option<i64> {
    match source {
        zhihu::COOKIE_KEY => cookie_value(cookie, "z_c0").and_then(|v| jwt_exp(&v)),
        "bilibili" => cookie_value(cookie, "SESSDATA").and_then(|v| sessdata_exp(&v)),
        _ => None,
    }
}

/// 从 `a=b; c=d` 形式的 Cookie 串里取某个键的值（大小写不敏感）。
fn cookie_value(cookie: &str, name: &str) -> Option<String> {
    cookie.split([';', '\n']).find_map(|segment| {
        let (key, value) = segment.trim().split_once('=')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then(|| value.trim().to_string())
    })
}

/// 从 JWT 形态的 token 里读 `exp`（知乎的 `z_c0` 就是 JWT）。
fn jwt_exp(token: &str) -> Option<i64> {
    let payload = token.split('.').nth(1)?;
    let raw = decode_base64(payload)?;
    let value: serde_json::Value = serde_json::from_slice(&raw).ok()?;
    value.get("exp").and_then(|v| v.as_i64())
}

/// 从 B站 SESSDATA 里读过期时间。
///
/// SESSDATA 是 URL 编码的，解出来常见两种形态：直接是 JSON（带 `expires`），
/// 或者 `信息,时间戳,签名` 的逗号串。两种都试，都不中就返回 None。
fn sessdata_exp(value: &str) -> Option<i64> {
    let decoded = percent_decode(value);
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&decoded) {
        if let Some(exp) = json
            .get("expires")
            .or_else(|| json.get("exp"))
            .and_then(|v| v.as_i64())
        {
            return Some(normalize_epoch(exp));
        }
    }
    // `信息,时间戳,签名`：中间那段的量级能区分秒/毫秒
    decoded
        .split(',')
        .nth(1)
        .and_then(|field| field.trim().parse::<i64>().ok())
        .map(normalize_epoch)
}

/// 统一成秒：B站给的是毫秒（13 位），知乎给的是秒（10 位）。
fn normalize_epoch(value: i64) -> i64 {
    if value > 100_000_000_000 {
        value / 1000
    } else {
        value
    }
}

/// 宽松的 base64 解码：JWT 用 URL-safe 无填充，别处可能是标准表，两种都试。
fn decode_base64(input: &str) -> Option<Vec<u8>> {
    let cleaned = input.trim();
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cleaned)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(cleaned))
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(cleaned))
        .or_else(|_| base64::engine::general_purpose::STANDARD.decode(cleaned))
        .ok()
}

/// 处理 `%XX` 转义（够用即可，不做 `+` 号语义）。
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or_default();
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
mod tests {
    use base64::Engine;

    use super::{cookie_value, normalize_epoch, parse_cookie_expiry, percent_decode, sessdata_exp};
    use crate::commands::favorites::zhihu;

    /// 造一个 JWT 形态的 token（只要第二段能被解出来即可）。
    fn fake_jwt(payload: &str) -> String {
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload);
        format!("header.{}.signature", encoded)
    }

    /// 知乎 z_c0 是 JWT，读 exp 当过期时间。
    #[test]
    fn zhihu_z_c0_reads_jwt_exp() {
        let cookie = format!(
            "d_c0=abc; z_c0={}; other=1",
            fake_jwt(r#"{"exp":1800000000,"id":"x"}"#)
        );
        assert_eq!(
            parse_cookie_expiry(zhihu::COOKIE_KEY, &cookie),
            Some(1800000000)
        );
    }

    /// B站 SESSDATA 里带毫秒时间戳，要归一化成秒。
    #[test]
    fn bilibili_sessdata_normalizes_millis() {
        let cookie = "SESSDATA=info%2C1800000000000%2Csig; bili_jct=x";
        assert_eq!(parse_cookie_expiry("bilibili", cookie), Some(1800000000));
    }

    /// SESSDATA 也可能直接是 URL 编码的 JSON。
    #[test]
    fn bilibili_sessdata_reads_json_expires() {
        let decoded = percent_decode("%7B%22expires%22%3A1800000000%7D");
        assert_eq!(decoded, r#"{"expires":1800000000}"#);
        assert_eq!(sessdata_exp(&decoded), Some(1800000000));
    }

    /// 解析不出来不能报错、不能瞎猜：返回 None，让运行期检测兜底。
    #[test]
    fn unparseable_cookie_returns_none() {
        assert_eq!(parse_cookie_expiry(zhihu::COOKIE_KEY, "z_c0=not-a-jwt"), None);
        assert_eq!(parse_cookie_expiry(zhihu::COOKIE_KEY, "d_c0=abc"), None);
        assert_eq!(parse_cookie_expiry("bilibili", "SESSDATA=;;"), None);
        assert_eq!(parse_cookie_expiry("github", "token"), None);
    }

    /// Cookie 键名大小写不敏感，值里含 `=` 也要能取全（base64 padding）。
    #[test]
    fn cookie_value_is_case_insensitive_and_keeps_equals() {
        assert_eq!(
            cookie_value("sessdata=abc==; other", "SESSDATA"),
            Some("abc==".to_string())
        );
        assert_eq!(cookie_value("z_c0 = v ", "z_c0"), Some("v".to_string()));
    }

    #[test]
    fn normalize_epoch_distinguishes_seconds_and_millis() {
        assert_eq!(normalize_epoch(1800000000), 1800000000);
        assert_eq!(normalize_epoch(1800000000000), 1800000000);
    }
}
