//! RDP 凭据写入 `.rdp` 的纯逻辑测试（不触碰真实密码）。
use super::credentials::{from_hex, password_utf16le, rdp_password_line, to_hex};

#[test]
fn hex_roundtrips_arbitrary_bytes() {
    let bytes = vec![0x00, 0x0f, 0x10, 0x80, 0xff, 0x7f];
    assert_eq!(from_hex(&to_hex(&bytes)), Some(bytes));
}

#[test]
fn hex_is_lowercase_and_even_length() {
    let hex = to_hex(&[0xAB, 0x01]);
    assert_eq!(hex, "ab01");
    assert_eq!(hex.len(), 4);
}

#[test]
fn invalid_hex_is_rejected() {
    assert_eq!(from_hex("zz"), None);
    assert_eq!(from_hex("abc"), None, "奇数长度必须拒绝");
    assert_eq!(from_hex(""), None);
}

/// mstsc 只认 `password 51:b:<hex>`，`51` 表示内容是 DPAPI 密文。
#[test]
fn rdp_line_uses_51_b_prefix() {
    let line = rdp_password_line(&[0xDE, 0xAD]);
    assert_eq!(line, "password 51:b:dead\r\n");
    assert!(line.ends_with("\r\n"), "rdp 文件必须用 CRLF 换行");
}

#[test]
fn empty_secret_produces_no_line() {
    assert!(rdp_password_line(&[]).is_empty(), "空密码不能写出空字段");
}

/// Windows `.rdp` 的 DPAPI 明文必须是 UTF-16LE；UTF-8 会让 mstsc 解出乱码。
#[test]
fn rdp_password_is_utf16le() {
    assert_eq!(password_utf16le("ab"), vec![0x61, 0x00, 0x62, 0x00]);
    // "密" U+5BC6 → C6 5B；"码" U+7801 → 01 78
    assert_eq!(password_utf16le("密码"), vec![0xC6, 0x5B, 0x01, 0x78]);
}
