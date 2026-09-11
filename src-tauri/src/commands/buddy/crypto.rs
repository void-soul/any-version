//! VS Code 系 IDE 的 safe storage 加解密（移植自 cockpit-tools `vscode_inject.rs`）。
//!
//! 平台加密模型：
//! - Windows: `Local State` 的 `os_crypt.encrypted_key` + DPAPI，载荷为 `v10` + AES-256-GCM
//! - macOS: Keychain "Code Safe Storage" 口令，载荷为 `v10` + AES-128-CBC（PBKDF2-SHA1 1003 轮）
//! - Linux: Secret Service 口令（v11 载荷），回退 `v10` 固定密钥 + AES-128-CBC
//!
//! 供 Buddy 模块读取/写回 WorkBuddy / CodeBuddy CN 的 `state.vscdb` 登录态。

use std::path::Path;

use aes_gcm::aead::generic_array::GenericArray;
use aes_gcm::aead::{Aead, AeadCore, OsRng};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::{engine::general_purpose, Engine as _};
use rusqlite::Connection;

#[cfg(not(target_os = "windows"))]
use aes::Aes128;
#[cfg(not(target_os = "windows"))]
use cbc::cipher::block_padding::Pkcs7;
#[cfg(not(target_os = "windows"))]
use cbc::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
#[cfg(not(target_os = "windows"))]
use pbkdf2::pbkdf2_hmac;
#[cfg(not(target_os = "windows"))]
use sha1::Sha1;

#[cfg(target_os = "windows")]
use windows_sys::Win32::Foundation::HLOCAL;
#[cfg(target_os = "windows")]
use windows_sys::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};

#[cfg(not(target_os = "windows"))]
type Aes128CbcDec = cbc::Decryptor<Aes128>;
#[cfg(not(target_os = "windows"))]
type Aes128CbcEnc = cbc::Encryptor<Aes128>;

const V10_PREFIX: &[u8] = b"v10";
#[cfg(not(target_os = "windows"))]
const V11_PREFIX: &[u8] = b"v11";
#[cfg(not(target_os = "windows"))]
const CBC_IV: [u8; 16] = [b' '; 16];
#[cfg(not(target_os = "windows"))]
const SALT: &[u8] = b"saltysalt";

// PBKDF2-HMAC-SHA1(1 iteration, key = "peanuts", salt = "saltysalt")
#[cfg(target_os = "linux")]
const LINUX_V10_KEY: [u8; 16] = [
    0xfd, 0x62, 0x1f, 0xe5, 0xa2, 0xb4, 0x02, 0x53, 0x9d, 0xfa, 0x14, 0x7c, 0xa9, 0x27, 0x27, 0x78,
];

// PBKDF2-HMAC-SHA1(1 iteration, key = "", salt = "saltysalt")
#[cfg(target_os = "linux")]
const LINUX_EMPTY_KEY: [u8; 16] = [
    0xd0, 0xd0, 0xec, 0x9c, 0x7d, 0x77, 0xd4, 0x3a, 0xc5, 0x41, 0x87, 0xfa, 0x48, 0x18, 0xd1, 0x7f,
];

/// 平台标识，决定 macOS Keychain / Linux Secret Service 的服务名候选
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Platform {
    WorkBuddy,
    CodeBuddyCn,
}

#[cfg(not(target_os = "windows"))]
fn platform_app_names(platform: Platform) -> &'static [&'static str] {
    match platform {
        Platform::WorkBuddy => &["WorkBuddy", "workbuddy", "workbuddy-cn", "workbuddycn"],
        Platform::CodeBuddyCn => &["CodeBuddy CN", "codebuddy cn", "codebuddy-cn", "codebuddycn"],
    }
}

/// VS Code 数据根目录（`data_root`）：用户传入的 WorkBuddy/CodeBuddy CN 安装目录。
/// safe storage 的 key 文件（Windows `Local State` / macOS Keychain / Linux Secret Service）
/// 都基于该根目录解析。当前主目标平台为 Windows。
fn get_encryption_key(data_root: &Path) -> Result<Vec<u8>, String> {
    #[cfg(target_os = "windows")]
    {
        let path = data_root.join("Local State");
        if !path.exists() {
            return Err(format!("未找到 Local State 密钥文件: {}", path.display()));
        }
        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("读取 Local State 失败: {}", e))?;
        let json: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| format!("解析 Local State JSON 失败: {}", e))?;
        let encrypted_key_b64 = json["os_crypt"]["encrypted_key"]
            .as_str()
            .ok_or("Local State 中缺少 os_crypt.encrypted_key")?;
        let encrypted_key_bytes = general_purpose::STANDARD
            .decode(encrypted_key_b64)
            .map_err(|e| format!("encrypted_key base64 解码失败: {}", e))?;
        if encrypted_key_bytes.len() < 6 {
            return Err("encrypted_key 数据过短".to_string());
        }
        let prefix = String::from_utf8_lossy(&encrypted_key_bytes[..5]);
        if prefix != "DPAPI" {
            return Err(format!("encrypted_key 前缀不是 DPAPI，实际: {}", prefix));
        }
        let dpapi_blob = &encrypted_key_bytes[5..];
        let key = dpapi_decrypt(dpapi_blob)?;
        if key.len() != 32 {
            return Err(format!("解密后的 AES 密钥长度异常: {}", key.len()));
        }
        Ok(key)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = data_root;
        Err("当前平台暂不支持读取 safe storage 密钥".to_string())
    }
}

#[cfg(target_os = "windows")]
fn dpapi_decrypt(encrypted: &[u8]) -> Result<Vec<u8>, String> {
    unsafe {
        let mut input = CRYPT_INTEGER_BLOB {
            cbData: encrypted.len() as u32,
            pbData: encrypted.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        let ok = CryptUnprotectData(
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
                "DPAPI CryptUnprotectData 调用失败 (error={})",
                std::io::Error::last_os_error()
            ));
        }
        let result =
            std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        // 释放 DPAPI 输出的内存（LocalAlloc/LocalFree 约定）
        let handle: HLOCAL = output.pbData as *mut core::ffi::c_void;
        windows_sys::Win32::Foundation::LocalFree(handle);
        Ok(result)
    }
}

fn decrypt_windows_gcm_v10(key: &[u8], encrypted: &[u8]) -> Result<Vec<u8>, String> {
    if encrypted.len() < 31 {
        return Err("密文数据过短".to_string());
    }
    if &encrypted[..3] != V10_PREFIX {
        return Err(format!(
            "不是 Windows v10 格式，前缀: {:?}",
            &encrypted[..encrypted.len().min(3)]
        ));
    }
    let nonce_bytes = &encrypted[3..15];
    let ciphertext = &encrypted[15..];
    let cipher = Aes256Gcm::new(GenericArray::from_slice(key));
    let nonce = Nonce::from_slice(nonce_bytes);
    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| format!("AES-GCM 解密失败: {:?}", e))
}

fn encrypt_windows_gcm_v10(key: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = Aes256Gcm::new(GenericArray::from_slice(key));
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|e| format!("AES-GCM 加密失败: {:?}", e))?;
    let mut result = Vec::with_capacity(3 + 12 + ciphertext.len());
    result.extend_from_slice(V10_PREFIX);
    result.extend_from_slice(nonce.as_slice());
    result.extend_from_slice(&ciphertext);
    Ok(result)
}

#[cfg(not(target_os = "windows"))]
fn decrypt_cbc_prefixed(
    encrypted: &[u8],
    expected_prefix: &[u8],
    key: &[u8; 16],
) -> Result<Vec<u8>, String> {
    if !encrypted.starts_with(expected_prefix) {
        return Err(format!(
            "非预期密文前缀: {:?}",
            &encrypted[..encrypted.len().min(3)]
        ));
    }
    let raw = &encrypted[expected_prefix.len()..];
    let mut buf = raw.to_vec();
    let cipher = Aes128CbcDec::new_from_slices(key, &CBC_IV)
        .map_err(|e| format!("初始化 AES-CBC 解密器失败: {:?}", e))?;
    let plaintext = cipher
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .map_err(|e| format!("AES-CBC 解密失败: {:?}", e))?
        .to_vec();
    Ok(plaintext)
}

#[cfg(not(target_os = "windows"))]
fn encrypt_cbc_prefixed(
    prefix: &[u8],
    key: &[u8; 16],
    plaintext: &[u8],
) -> Result<Vec<u8>, String> {
    let cipher = Aes128CbcEnc::new_from_slices(key, &CBC_IV)
        .map_err(|e| format!("初始化 AES-CBC 加密器失败: {:?}", e))?;
    let mut buf = plaintext.to_vec();
    let msg_len = buf.len();
    let pad_len = 16 - (msg_len % 16);
    buf.resize(msg_len + pad_len, 0);
    let ciphertext = cipher
        .encrypt_padded_mut::<Pkcs7>(&mut buf, msg_len)
        .map_err(|e| format!("AES-CBC 加密失败: {:?}", e))?
        .to_vec();
    let mut result = Vec::with_capacity(prefix.len() + ciphertext.len());
    result.extend_from_slice(prefix);
    result.extend_from_slice(&ciphertext);
    Ok(result)
}

#[cfg(not(target_os = "windows"))]
fn pbkdf2_sha1_key(password: &str, iterations: u32) -> [u8; 16] {
    let mut key = [0u8; 16];
    pbkdf2_hmac::<Sha1>(password.as_bytes(), SALT, iterations, &mut key);
    key
}

#[cfg(not(target_os = "windows"))]
fn run_command_get_trimmed(program: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

#[cfg(target_os = "linux")]
fn get_linux_v11_key(platform: Platform) -> Option<[u8; 16]> {
    for app in platform_app_names(platform) {
        if let Some(password) =
            run_command_get_trimmed("secret-tool", &["lookup", "application", app])
        {
            return Some(pbkdf2_sha1_key(&password, 1));
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn get_macos_safe_storage_password(platform: Platform) -> Result<String, String> {
    for app in platform_app_names(platform) {
        let service = format!("{} Safe Storage", app);
        for args in [
            vec!["find-generic-password", "-w", "-s", &service],
            vec!["find-generic-password", "-w", "-s", &service, "-a", app],
        ] {
            if let Some(password) = run_command_get_trimmed("security", &args) {
                return Ok(password);
            }
        }
    }
    Err("无法从 Keychain 读取 Safe Storage 口令".to_string())
}

/// 解密 safe storage 密文。`data_root` 为 IDE 数据根目录（含 `Local State` 等）。
pub fn decrypt_secret_payload(
    encrypted: &[u8],
    data_root: &Path,
    platform: Platform,
) -> Result<Vec<u8>, String> {
    #[cfg(target_os = "windows")]
    {
        let _ = platform;
        let key = get_encryption_key(data_root)?;
        decrypt_windows_gcm_v10(&key, encrypted)
    }

    #[cfg(target_os = "macos")]
    {
        let password = get_macos_safe_storage_password(platform)?;
        let key = pbkdf2_sha1_key(&password, 1003);
        decrypt_cbc_prefixed(encrypted, V10_PREFIX, &key)
    }

    #[cfg(target_os = "linux")]
    {
        if encrypted.starts_with(V11_PREFIX) {
            let key = get_linux_v11_key(platform).ok_or(
                "无法加载 Linux secret storage 密钥（v11 载荷）".to_string(),
            )?;
            decrypt_cbc_prefixed(encrypted, V11_PREFIX, &key)
                .or_else(|_| decrypt_cbc_prefixed(encrypted, V11_PREFIX, &LINUX_EMPTY_KEY))
        } else if encrypted.starts_with(V10_PREFIX) {
            decrypt_cbc_prefixed(encrypted, V10_PREFIX, &LINUX_V10_KEY)
                .or_else(|_| decrypt_cbc_prefixed(encrypted, V10_PREFIX, &LINUX_EMPTY_KEY))
        } else {
            Err(format!(
                "不支持的密文前缀: {:?}",
                &encrypted[..encrypted.len().min(3)]
            ))
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = (encrypted, data_root, platform);
        Err("不支持当前平台".to_string())
    }
}

/// 加密明文为 safe storage 载荷（保持与客户端相同的平台格式）。
pub fn encrypt_secret_payload(
    plaintext: &[u8],
    data_root: &Path,
    platform: Platform,
) -> Result<Vec<u8>, String> {
    #[cfg(target_os = "windows")]
    {
        let _ = platform;
        let key = get_encryption_key(data_root)?;
        encrypt_windows_gcm_v10(&key, plaintext)
    }

    #[cfg(target_os = "macos")]
    {
        let password = get_macos_safe_storage_password(platform)?;
        let key = pbkdf2_sha1_key(&password, 1003);
        encrypt_cbc_prefixed(V10_PREFIX, &key, plaintext)
    }

    #[cfg(target_os = "linux")]
    {
        let _ = data_root;
        if let Some(key) = get_linux_v11_key(platform) {
            encrypt_cbc_prefixed(V11_PREFIX, &key, plaintext)
        } else {
            encrypt_cbc_prefixed(V10_PREFIX, &LINUX_V10_KEY, plaintext)
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = (plaintext, data_root, platform);
        Err("不支持当前平台".to_string())
    }
}

/// 从 `state.vscdb` 读取指定 extension/key 的 secret 原始值（Buffer JSON 字符串）。
pub fn read_secret_storage_value(
    db_path: &Path,
    extension_id: &str,
    key: &str,
) -> Result<Option<String>, String> {
    if !db_path.exists() {
        return Ok(None);
    }
    let conn = Connection::open(db_path).map_err(|e| {
        format!(
            "打开 state.vscdb 失败 {}: {}",
            db_path.display(),
            e
        )
    })?;
    let secret_key = format!(
        r#"secret://{{"extensionId":"{}","key":"{}"}}"#,
        extension_id, key
    );
    match conn.query_row(
        "SELECT value FROM ItemTable WHERE key = ?1",
        [secret_key.as_str()],
        |row| row.get::<_, String>(0),
    ) {
        Ok(value) => Ok(Some(value)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(format!("查询 secret 失败: {}", e)),
    }
}

/// 将 Buffer JSON（`{"type":"Buffer","data":[...]}`）解码为字节。
pub fn decode_buffer_data(buffer: &serde_json::Value) -> Result<Vec<u8>, String> {
    let data_arr = buffer["data"]
        .as_array()
        .ok_or("secret 数据不是 Buffer 格式")?;
    let mut bytes: Vec<u8> = Vec::with_capacity(data_arr.len());
    for (idx, v) in data_arr.iter().enumerate() {
        let n = v
            .as_u64()
            .ok_or_else(|| format!("secret 元素 {} 不是整数", idx))?;
        if n > 255 {
            return Err(format!("secret 元素 {} 超出字节范围 ({} > 255)", idx, n));
        }
        bytes.push(n as u8);
    }
    Ok(bytes)
}

/// 解码 safe storage 值：Buffer 格式则解密；否则按原样返回字符串。
pub fn decode_secret_storage_value(
    raw_value: &str,
    data_root: &Path,
    platform: Platform,
) -> Result<String, String> {
    let parsed: serde_json::Value = match serde_json::from_str(raw_value) {
        Ok(value) => value,
        Err(_) => return Ok(raw_value.to_string()),
    };
    if parsed.get("data").is_some() {
        let encrypted_bytes = decode_buffer_data(&parsed)?;
        let decrypted = decrypt_secret_payload(&encrypted_bytes, data_root, platform)?;
        return String::from_utf8(decrypted)
            .map_err(|e| format!("解密结果不是合法 UTF-8: {}", e));
    }
    if let Some(value) = parsed.as_str() {
        return Ok(value.to_string());
    }
    Ok(raw_value.to_string())
}

/// 把明文加密为 Buffer JSON，写回 `state.vscdb` 的指定 key。
pub fn inject_secret_to_state_db(
    db_path: &Path,
    db_key: &str,
    plaintext: &str,
    data_root: &Path,
    platform: Platform,
) -> Result<(), String> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建 state.vscdb 父目录失败: {}", e))?;
    }
    let conn = Connection::open(db_path)
        .map_err(|e| format!("打开 state.vscdb 失败: {}", e))?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS ItemTable (key TEXT PRIMARY KEY, value TEXT)",
        [],
    )
    .map_err(|e| format!("初始化 ItemTable 失败: {}", e))?;

    let encrypted = encrypt_secret_payload(plaintext.as_bytes(), data_root, platform)?;
    let buffer_json = serde_json::json!({
        "type": "Buffer",
        "data": encrypted
    });
    let buffer_str = serde_json::to_string(&buffer_json)
        .map_err(|e| format!("序列化 Buffer 失败: {}", e))?;
    conn.execute(
        "INSERT OR REPLACE INTO ItemTable (key, value) VALUES (?1, ?2)",
        rusqlite::params![db_key, buffer_str],
    )
    .map_err(|e| format!("写入 state.vscdb 失败: {}", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_data_roundtrip() {
        let bytes: Vec<u8> = vec![0u8, 1, 2, 255, 128];
        let json = serde_json::json!({"type": "Buffer", "data": bytes.clone()});
        assert_eq!(decode_buffer_data(&json).unwrap(), bytes);
    }

    #[test]
    fn buffer_data_rejects_out_of_range() {
        let json = serde_json::json!({"type": "Buffer", "data": [0, 300]});
        assert!(decode_buffer_data(&json).is_err());
    }

    #[test]
    fn decode_plain_string_passthrough() {
        // 非 Buffer 格式：直接返回原字符串（客户端兼容路径）
        let raw = r#"{"token":"abc"}"#;
        let out = decode_secret_storage_value(
            raw,
            Path::new("."),
            Platform::WorkBuddy,
        )
        .unwrap();
        assert_eq!(out, raw);
    }

    #[test]
    fn secret_key_format() {
        // 验证 state.vscdb key 与官方客户端格式一致
        let key = format!(
            r#"secret://{{"extensionId":"{}","key":"{}"}}"#,
            "tencent-cloud.coding-copilot", "planning-genie.new.accessTokencn"
        );
        assert_eq!(
            key,
            r#"secret://{"extensionId":"tencent-cloud.coding-copilot","key":"planning-genie.new.accessTokencn"}"#
        );
    }
}