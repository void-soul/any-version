use super::*;
use base64::Engine as _;

// ---------------------------------------------------------------------------
// 敏感字段加密（AES-256-GCM，带 base64 混淆迁移兼容）
// ---------------------------------------------------------------------------

pub(super) const CRED_ENCRYPTION_MARKER: &str = "ENC_V2:";

/// 获取或生成机器级加密密钥（32 字节）。
/// 密钥存储在 `~/.any-version/certs/.master_key`，权限由 OS 文件权限保护。
pub(super) fn get_or_create_master_key() -> Result<[u8; 32], String> {
    crate::commands::secrets::get_or_create_master_key()
}

/// 使用 AES-256-GCM 加密字符串，返回 base64(nonce || ciphertext)。
pub(super) fn encrypt_secret(plaintext: &str) -> Result<String, String> {
    if plaintext.is_empty() {
        return Ok(String::new());
    }
    let key = get_or_create_master_key()?;
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|e| format!("创建 AES 密码器失败: {:?}", e))?;
    // 12 字节随机 nonce
    let mut nonce_bytes = [0u8; 12];
    getrandom::getrandom(&mut nonce_bytes[..]).map_err(|e| format!("生成 nonce 失败: {}", e))?;
    let nonce = Nonce::from_slice(&nonce_bytes[..]);
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| format!("AES 加密失败: {:?}", e))?;
    // 拼接 nonce + ciphertext，然后 base64
    let mut combined = Vec::with_capacity(12 + ciphertext.len());
    combined.extend_from_slice(&nonce_bytes);
    combined.extend_from_slice(&ciphertext);
    Ok(format!(
        "{}{}",
        CRED_ENCRYPTION_MARKER,
        B64.encode(&combined)
    ))
}

/// 解密 AES-256-GCM 加密的字符串。
pub(super) fn decrypt_secret(ciphertext_b64: &str) -> Result<String, String> {
    if ciphertext_b64.is_empty() {
        return Ok(String::new());
    }
    // 兼容旧版 base64 混淆（无 ENC_V2: 前缀）
    if !ciphertext_b64.starts_with(CRED_ENCRYPTION_MARKER) {
        // 旧版：直接 base64 解码
        return B64
            .decode(ciphertext_b64)
            .map(|b| String::from_utf8_lossy(&b).to_string())
            .map_err(|e| format!("旧版 base64 解码失败: {}", e));
    }
    let combined_b64 = &ciphertext_b64[CRED_ENCRYPTION_MARKER.len()..];
    let combined = B64
        .decode(combined_b64)
        .map_err(|e| format!("AES 密文 base64 解码失败: {}", e))?;
    if combined.len() < 13 {
        return Err("AES 密文太短".to_string());
    }
    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let key = get_or_create_master_key()?;
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|e| format!("创建 AES 密码器失败: {:?}", e))?;
    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| format!("AES 解密失败（密钥不匹配或数据损坏）: {:?}", e))?;
    String::from_utf8(plaintext).map_err(|e| format!("AES 解密结果 UTF-8 解码失败: {}", e))
}

// 持久化凭据时对 value 做混淆；读取时还原。
pub(super) fn encrypt_credential(mut c: Credential) -> Credential {
    for v in c.data.values_mut() {
        if !v.is_empty() {
            if let Ok(encrypted) = encrypt_secret(v) {
                *v = encrypted;
            } else {
                // 加密失败时回退到旧版 base64（保证不丢数据）
                *v = B64.encode(v.as_bytes());
            }
        }
    }
    c
}
pub(super) fn decrypt_credential(mut c: Credential) -> Credential {
    for v in c.data.values_mut() {
        if !v.is_empty() {
            if let Ok(decrypted) = decrypt_secret(v) {
                *v = decrypted;
            }
            // 解密失败时保留原值（可能是旧版 base64，已兼容）
        }
    }
    c
}
