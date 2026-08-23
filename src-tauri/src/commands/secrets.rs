//! 敏感字段加密（AES-256-GCM），主密钥存 `{data_dir}/certs/.master_key`。
//! 与 cert 模块使用同一把机器级主密钥，保证各模块凭据可互通加解密。

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::{engine::general_purpose, Engine as _};

use crate::commands::config::get_data_dir;

const CRED_ENCRYPTION_MARKER: &str = "ENC_V2:";

/// 获取或生成机器级加密密钥（32 字节），与 cert 模块同路径。
pub fn get_or_create_master_key() -> Result<[u8; 32], String> {
    let key_path = get_data_dir().join("certs").join(".master_key");
    if key_path.exists() {
        if let Ok(encoded) = std::fs::read_to_string(&key_path) {
            if let Ok(bytes) = general_purpose::STANDARD.decode(encoded.trim()) {
                if bytes.len() == 32 {
                    let mut key = [0u8; 32];
                    key.copy_from_slice(&bytes);
                    return Ok(key);
                }
            }
        }
    }
    let mut key = [0u8; 32];
    getrandom::getrandom(&mut key).map_err(|e| format!("生成随机密钥失败: {}", e))?;
    if let Some(parent) = key_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建密钥目录失败: {}", e))?;
    }
    std::fs::write(&key_path, general_purpose::STANDARD.encode(&key))
        .map_err(|e| format!("写入主密钥失败: {}", e))?;
    restrict_master_key_acl(&key_path);
    Ok(key)
}

/// 启动时对既有主密钥文件收紧 ACL（仅当前用户可读写）。
/// 密钥是明文落盘，未限权时任何能读该目录的账户/进程都能解密所有凭据。
pub fn restrict_master_key_acl(path: &std::path::Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        if let Ok(user) = std::env::var("USERNAME") {
            let _ = std::process::Command::new("icacls")
                .arg(path)
                .arg("/inheritance:r")
                .arg("/grant:r")
                .arg(format!("{}:F", user))
                .creation_flags(0x08000000) // CREATE_NO_WINDOW
                .output();
        }
    }
    #[cfg(not(windows))]
    {
        let _ = path;
    }
}

/// AES-256-GCM 加密，返回 `ENC_V2:` + base64(nonce || ciphertext)。
pub fn encrypt_secret(plaintext: &str) -> Result<String, String> {
    if plaintext.is_empty() {
        return Ok(String::new());
    }
    let key = get_or_create_master_key()?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| format!("创建 AES 密码器失败: {:?}", e))?;
    let mut nonce_bytes = [0u8; 12];
    getrandom::getrandom(&mut nonce_bytes).map_err(|e| format!("生成 nonce 失败: {}", e))?;
    let nonce = Nonce::from_slice(&nonce_bytes[..]);
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| format!("AES 加密失败: {:?}", e))?;
    let mut combined = Vec::with_capacity(12 + ciphertext.len());
    combined.extend_from_slice(&nonce_bytes);
    combined.extend_from_slice(&ciphertext);
    Ok(format!("{}{}", CRED_ENCRYPTION_MARKER, general_purpose::STANDARD.encode(&combined)))
}

/// 解密 AES-256-GCM 密文；兼容旧版 base64 混淆（无 ENC_V2: 前缀）。
pub fn decrypt_secret(enc: &str) -> Result<String, String> {
    if enc.is_empty() {
        return Ok(String::new());
    }
    if !enc.starts_with(CRED_ENCRYPTION_MARKER) {
        return Ok(general_purpose::STANDARD
            .decode(enc)
            .map(|b| String::from_utf8_lossy(&b).to_string())
            .unwrap_or_default());
    }
    let combined_b64 = &enc[CRED_ENCRYPTION_MARKER.len()..];
    let combined = general_purpose::STANDARD
        .decode(combined_b64)
        .map_err(|e| format!("密文 base64 解码失败: {}", e))?;
    if combined.len() < 13 {
        return Err("密文太短".to_string());
    }
    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let key = get_or_create_master_key()?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| format!("创建 AES 密码器失败: {:?}", e))?;
    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| format!("AES 解密失败（密钥不匹配或数据损坏）: {:?}", e))?;
    String::from_utf8(plaintext).map_err(|e| format!("解密结果编码错误: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let secret = "test-secret-value-for-roundtrip";
        let enc = encrypt_secret(secret).expect("encrypt");
        assert!(enc.starts_with(CRED_ENCRYPTION_MARKER));
        assert_ne!(enc, format!("ENC_V2:{}", secret));
        assert_eq!(decrypt_secret(&enc).expect("decrypt"), secret);
    }

    #[test]
    fn decrypt_tampered_fails() {
        let enc = encrypt_secret("hello").expect("encrypt");
        // 篡改密文末尾一个字符（base64 载荷中间）
        let mut bytes = enc.into_bytes();
        let idx = bytes.len() - 2;
        bytes[idx] = if bytes[idx] == b'A' { b'B' } else { b'A' };
        let tampered = String::from_utf8(bytes).unwrap();
        assert!(decrypt_secret(&tampered).is_err());
    }

    #[test]
    fn empty_secret_roundtrips() {
        assert_eq!(encrypt_secret("").unwrap(), "");
        assert_eq!(decrypt_secret("").unwrap(), "");
    }

    #[test]
    fn legacy_base64_no_marker_still_decrypts_as_obfuscation() {
        // 旧版无 ENC_V2 前缀的 base64（非加密，仅混淆）——兼容读取
        let legacy = general_purpose::STANDARD.encode("legacy-value");
        assert_eq!(decrypt_secret(&legacy).unwrap(), "legacy-value");
    }
}
