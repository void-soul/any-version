//! 敏感字段加密（AES-256-GCM），主密钥存 `{data_dir}/certs/.master_key`。
//! 与 cert 模块使用同一把机器级主密钥，保证各模块凭据可互通加解密。

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::{engine::general_purpose, Engine as _};

#[cfg(not(test))]
use crate::commands::config::get_data_dir;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

const CRED_ENCRYPTION_MARKER: &str = "ENC_V2:";
static MASTER_KEY_LOCK: Mutex<()> = Mutex::new(());

/// 获取或生成机器级加密密钥（32 字节），与 cert 模块同路径。
pub fn get_or_create_master_key() -> Result<[u8; 32], String> {
    #[cfg(not(test))]
    let key_path = get_data_dir().join("certs").join(".master_key");
    #[cfg(test)]
    let key_path = std::env::temp_dir().join(format!("kira-secrets-tests-{}", std::process::id())).join(".master_key");
    master_key_at(&key_path)
}

/// One creator per process; create_new also prevents a second process from replacing the key.
/// A malformed existing key is never rotated implicitly: doing so loses all old credentials.
fn master_key_at(key_path: &Path) -> Result<[u8; 32], String> {
    let _guard = MASTER_KEY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if key_path.exists() {
        return read_master_key(key_path);
    }
    let mut key = [0u8; 32];
    getrandom::getrandom(&mut key).map_err(|e| format!("生成随机密钥失败: {}", e))?;
    if let Some(parent) = key_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建密钥目录失败: {}", e))?;
    }
    match std::fs::OpenOptions::new().write(true).create_new(true).open(key_path) {
        Ok(mut file) => {
            restrict_master_key_acl(key_path);
            file.write_all(general_purpose::STANDARD.encode(key).as_bytes())
                .and_then(|_| file.sync_all())
                .map_err(|e| format!("写入主密钥失败，请保留原文件并检查存储权限: {}", e))?;
            Ok(key)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // The winning process may still be writing. Never return our unpersisted key.
            for _ in 0..40 {
                if let Ok(persisted) = read_master_key(key_path) {
                    return Ok(persisted);
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            read_master_key(key_path)
        }
        Err(e) => Err(format!("创建主密钥失败: {}", e)),
    }
}

fn read_master_key(path: &Path) -> Result<[u8; 32], String> {
    let encoded = std::fs::read_to_string(path)
        .map_err(|e| format!("读取主密钥失败（未替换原文件）: {}", e))?;
    let bytes = general_purpose::STANDARD.decode(encoded.trim())
        .map_err(|_| "主密钥损坏，已停止凭据操作；请从备份恢复原主密钥".to_string())?;
    bytes.try_into().map_err(|_| "主密钥长度错误，已停止凭据操作；请从备份恢复原主密钥".to_string())
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

    fn isolated_dir() -> std::path::PathBuf {
        let mut nonce = [0u8; 8];
        getrandom::getrandom(&mut nonce).unwrap();
        let dir = std::env::temp_dir().join(format!("kira-master-key-{}", u64::from_ne_bytes(nonce)));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn concurrent_master_key_creation_returns_one_persisted_key() {
        let dir = isolated_dir();
        let path = dir.join(".master_key");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8).map(|_| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || { barrier.wait(); master_key_at(&path).unwrap() })
        }).collect();
        let keys: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        assert!(keys.iter().all(|k| k == &keys[0]));
        assert_eq!(read_master_key(&path).unwrap(), keys[0]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn malformed_master_key_is_preserved_instead_of_rotated() {
        let dir = isolated_dir();
        let path = dir.join(".master_key");
        std::fs::write(&path, "not a valid master key").unwrap();
        assert!(master_key_at(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "not a valid master key");
        std::fs::remove_dir_all(dir).unwrap();
    }

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
