//! 把 RDP 密码交给系统 `mstsc` 的唯一安全通道：DPAPI 密文写进 `.rdp` 的
//! `password 51:b:<hex>` 字段。
//!
//! 为什么不用明文：`.rdp` 是纯文本文件，明文密码会留在磁盘上。
//! DPAPI blob 绑定「当前 Windows 用户 + 本机」，拷到别处解不开。
//!
//! 纯逻辑（hex 编解码、行格式）放在这里以便跨平台单测；
//! 真正的 `CryptProtectData` 调用见下方 `#[cfg(windows)]` 段。

use super::models::RdpConnection;

/// 取连接的明文密码。两种来源互不混用：
/// FinalShell 导入的走它自己的 DES 兼容层，本地新建的走 `ENC_V2:`（AES-GCM）。
/// 返回 `Ok(None)` 表示该连接没有保存密码（应由系统弹凭据框）。
pub fn resolve_plain_password(conn: &RdpConnection) -> Result<Option<String>, String> {
    if let Some(cipher) = conn.finalshell_cipher.as_deref() {
        return super::finalshell_crypto::decrypt(cipher).map(Some);
    }
    match conn.password.as_deref() {
        Some(enc) if !enc.is_empty() => crate::commands::secrets::decrypt_secret(enc).map(Some),
        _ => Ok(None),
    }
}

/// Windows `.rdp` 的 `password 51:b:` 要求 DPAPI 明文是 **UTF-16LE**。
///
/// 直接加密 UTF-8 会让 mstsc 解出乱码，表现为「明明存了密码却永远弹凭据框」。
pub fn password_utf16le(plain: &str) -> Vec<u8> {
    plain.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// 小写十六进制，长度为偶数。
pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 严格解析十六进制；奇数长度或非十六进制字符一律返回 `None`。
pub fn from_hex(text: &str) -> Option<Vec<u8>> {
    if text.is_empty() || text.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(text.len() / 2);
    let bytes = text.as_bytes();
    for pair in bytes.chunks(2) {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
    }
    Some(out)
}

/// 生成 `.rdp` 的密码行；空密文返回空串（调用方应改为弹凭据框）。
pub fn rdp_password_line(blob: &[u8]) -> String {
    if blob.is_empty() {
        return String::new();
    }
    format!("password 51:b:{}\r\n", to_hex(blob))
}

#[cfg(windows)]
mod dpapi {
    use windows_sys::Win32::Foundation::{LocalFree, BOOL};
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB,
    };

    fn blob(bytes: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: bytes.len() as u32,
            // DPAPI 只读入参，这里不修改内容
            pbData: bytes.as_ptr() as *mut u8,
        }
    }

    /// 用当前用户的 DPAPI 主密钥加密。`flags=0` 表示不弹 UI、不额外绑定 entropy。
    pub fn protect(plain: &str) -> Result<Vec<u8>, String> {
        let plain = super::password_utf16le(plain);
        unsafe {
            let mut input = blob(&plain);
            let mut output = CRYPT_INTEGER_BLOB {
                cbData: 0,
                pbData: std::ptr::null_mut(),
            };
            let ok: BOOL = CryptProtectData(
                &mut input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                &mut output,
            );
            if ok == 0 {
                return Err(format!(
                    "DPAPI 加密失败: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let out = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
            LocalFree(output.pbData as *mut core::ffi::c_void);
            Ok(out)
        }
    }

    /// 仅用于自检：确认写进文件的 blob 能被同一用户解回原文。
    pub fn unprotect(blob_bytes: &[u8]) -> Result<String, String> {
        unsafe {
            let mut input = blob(blob_bytes);
            let mut output = CRYPT_INTEGER_BLOB {
                cbData: 0,
                pbData: std::ptr::null_mut(),
            };
            let ok: BOOL = CryptUnprotectData(
                &mut input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                &mut output,
            );
            if ok == 0 {
                return Err(format!(
                    "DPAPI 解密失败: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
            LocalFree(output.pbData as *mut core::ffi::c_void);
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            String::from_utf16(&units).map_err(|_| "DPAPI 明文不是 UTF-16LE".to_string())
        }
    }
}

#[cfg(windows)]
pub use dpapi::{protect, unprotect};
