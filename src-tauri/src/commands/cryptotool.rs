// ─── JWT / 加解密工具箱模块 ───
// 复用既有依赖：base64 / serde_json / hmac / sha2 / md5 / sha1 / aes-gcm。
// JWT 支持 HS256/HS384/HS512 签名校验；RS/ES 系列需公钥，暂不支持。

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::Serialize;
use sha1::Digest;
use sha2::{Sha256, Sha384, Sha512};

// ---------- Base64 ----------

#[tauri::command]
pub fn crypto_base64_encode(text: String) -> Result<String, String> {
    Ok(STANDARD.encode(text.as_bytes()))
}

#[tauri::command]
pub fn crypto_base64_decode(text: String) -> Result<String, String> {
    let bytes = STANDARD
        .decode(text.trim())
        .map_err(|e| format!("Base64 解码失败: {}", e))?;
    String::from_utf8(bytes).map_err(|_| "解码结果不是有效的 UTF-8（可能是二进制数据）".to_string())
}

// ---------- Hash ----------

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

#[derive(Serialize)]
pub struct HashResult {
    pub md5: String,
    pub sha1: String,
    pub sha256: String,
    pub sha512: String,
}

#[tauri::command]
pub fn crypto_hash(text: String) -> HashResult {
    let md5_hex = format!("{:x}", md5::compute(text.as_bytes()));
    let mut h_sha1 = sha1::Sha1::new();
    h_sha1.update(text.as_bytes());
    let mut h_sha256 = Sha256::new();
    h_sha256.update(text.as_bytes());
    let mut h_sha512 = Sha512::new();
    h_sha512.update(text.as_bytes());
    HashResult {
        md5: md5_hex,
        sha1: to_hex(&h_sha1.finalize()),
        sha256: to_hex(&h_sha256.finalize()),
        sha512: to_hex(&h_sha512.finalize()),
    }
}

// ---------- JWT ----------

#[derive(Serialize)]
pub struct JwtDecodeResult {
    pub header: serde_json::Value,
    pub payload: serde_json::Value,
    /// base64url 的原始签名段（hex 展示）
    pub signature_hex: String,
    pub alg: String,
}

fn b64url_decode(part: &str) -> Result<Vec<u8>, String> {
    URL_SAFE_NO_PAD
        .decode(part.trim())
        .map_err(|e| format!("JWT 段 Base64URL 解码失败: {}", e))
}

/// 解码 JWT（不校验签名）。
#[tauri::command]
pub fn jwt_decode(token: String) -> Result<JwtDecodeResult, String> {
    let token = token.trim();
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(format!("JWT 应有 3 段（header.payload.signature），实际 {} 段", parts.len()));
    }
    let header: serde_json::Value =
        serde_json::from_slice(&b64url_decode(parts[0])?).map_err(|e| format!("header 不是有效 JSON: {}", e))?;
    let payload: serde_json::Value =
        serde_json::from_slice(&b64url_decode(parts[1])?).map_err(|e| format!("payload 不是有效 JSON: {}", e))?;
    let sig = b64url_decode(parts[2])?;
    let alg = header
        .get("alg")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    Ok(JwtDecodeResult {
        header,
        payload,
        signature_hex: to_hex(&sig),
        alg,
    })
}

type HmacSha256 = Hmac<Sha256>;
type HmacSha384 = Hmac<Sha384>;
type HmacSha512 = Hmac<Sha512>;

/// 校验 JWT HMAC 系列签名（HS256/HS384/HS512），secret 为文本或 hex（自动识别）。
/// 返回 Ok(true/false) 表示校验完成；Err 为算法不支持或格式错误。
#[tauri::command]
pub fn jwt_verify_hs(token: String, secret: String) -> Result<bool, String> {
    let token = token.trim();
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err("JWT 格式不正确".to_string());
    }
    let header: serde_json::Value = serde_json::from_slice(&b64url_decode(parts[0])?)
        .map_err(|e| format!("header 解析失败: {}", e))?;
    let alg = header
        .get("alg")
        .and_then(|v| v.as_str())
        .ok_or("header 缺少 alg 字段")?
        .to_string();

    let signing_input = format!("{}.{}", parts[0], parts[1]);
    let expected = b64url_decode(parts[2])?;

    // secret 支持 "hex:" 前缀显式指定十六进制
    let secret_bytes: Vec<u8> = if let Some(hex) = secret.strip_prefix("hex:") {
        (0..hex.len() / 2)
            .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16))
            .collect::<Result<_, _>>()
            .map_err(|e| format!("hex secret 无效: {}", e))?
    } else {
        secret.into_bytes()
    };

    let ok = match alg.as_str() {
        "HS256" => {
            let mut mac = <HmacSha256 as Mac>::new_from_slice(&secret_bytes).map_err(|e| e.to_string())?;
            mac.update(signing_input.as_bytes());
            mac.verify_slice(&expected).is_ok()
        }
        "HS384" => {
            let mut mac = <HmacSha384 as Mac>::new_from_slice(&secret_bytes).map_err(|e| e.to_string())?;
            mac.update(signing_input.as_bytes());
            mac.verify_slice(&expected).is_ok()
        }
        "HS512" => {
            let mut mac = <HmacSha512 as Mac>::new_from_slice(&secret_bytes).map_err(|e| e.to_string())?;
            mac.update(signing_input.as_bytes());
            mac.verify_slice(&expected).is_ok()
        }
        other => return Err(format!("暂不支持算法 {}（仅支持 HS256/HS384/HS512）", other)),
    };
    Ok(ok)
}

// ---------- AES-256-GCM ----------

/// AES-256-GCM 加密。key 支持 "hex:" 前缀（32 字节）；不足 32 字节的文本密钥用 SHA-256 派生。
/// 返回 base64(nonce(12) || ciphertext)。
#[tauri::command]
pub fn aes_gcm_encrypt(plaintext: String, key: String) -> Result<String, String> {
    let key_bytes = derive_key(&key)?;
    let cipher = Aes256Gcm::new_from_slice(&key_bytes).map_err(|e| e.to_string())?;

    let mut nonce_bytes = [0u8; 12];
    getrandom::getrandom(&mut nonce_bytes).map_err(|e| format!("生成 nonce 失败: {}", e))?;
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ct = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| format!("加密失败: {}", e))?;

    let mut out = Vec::with_capacity(12 + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    Ok(STANDARD.encode(out))
}

/// AES-256-GCM 解密。输入为 base64(nonce(12) || ciphertext)，密钥派生规则同加密。
#[tauri::command]
pub fn aes_gcm_decrypt(data_b64: String, key: String) -> Result<String, String> {
    let raw = STANDARD
        .decode(data_b64.trim())
        .map_err(|e| format!("输入不是有效 Base64: {}", e))?;
    if raw.len() < 13 {
        return Err("数据太短（至少需要 12 字节 nonce + 1 字节密文）".to_string());
    }
    let key_bytes = derive_key(&key)?;
    let cipher = Aes256Gcm::new_from_slice(&key_bytes).map_err(|e| e.to_string())?;
    let (nonce, ct) = raw.split_at(12);
    let pt = cipher
        .decrypt(Nonce::from_slice(nonce), ct)
        .map_err(|_| "解密失败：密钥错误或数据被篡改".to_string())?;
    String::from_utf8(pt).map_err(|_| "解密结果不是有效的 UTF-8".to_string())
}

/// 密钥派生：hex: 前缀取原始字节（须 32 字节）；否则对文本做 SHA-256 得到 32 字节。
fn derive_key(key: &str) -> Result<Vec<u8>, String> {
    if let Some(hex) = key.strip_prefix("hex:") {
        let bytes = (0..hex.len() / 2)
            .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("hex 密钥无效: {}", e))?;
        if bytes.len() != 32 {
            return Err(format!("AES-256 需要 32 字节密钥，实际 {} 字节", bytes.len()));
        }
        Ok(bytes)
    } else {
        let mut hasher = Sha256::new();
        hasher.update(key.as_bytes());
        Ok(hasher.finalize().to_vec())
    }
}
