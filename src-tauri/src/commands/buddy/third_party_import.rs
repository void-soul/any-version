//! 第三方工具导出文件导入（WorkDaddy / cockpit-tools）。
//!
//! 参考实现（SKILL.md §2.1 / §2.2）：
//! - **主参考 WorkDaddy**（纯逻辑，可直译）：
//!   - `scripts/secure-transfer.js`：导出信封 v3 = `gzip` + `AES-256-GCM`，密钥由
//!     `scrypt(password, salt)` 派生（Node `crypto.scryptSync` 默认 N=2^14 / r=8 / p=1 / keylen=32）；
//!     打包布局 `base64(iv(12) ‖ authTag(16) ‖ ciphertext)`，信封字段
//!     `{wbsExport:'WorkDaddy', version, exportType, kdf, compression, salt, data}`；
//!     `version ∈ {2,3}`（v2 不压缩明文）。
//!   - `daemon.js` `/api/accounts/export`：载荷 `{exportType:'WorkDaddy-accounts', version:2,
//!     accounts:[{uid, info}]}`，`info` = 账号备份文件（WorkBuddy 登录文件）原文。
//!   - **平台范围（§0.1 硬规则）**：WorkDaddy 只有 WorkBuddy（cn/ai）两个 profile，
//!     故 WorkDaddy 路径**只产出 WorkBuddy 账号**。
//! - **辅参考 cockpit-tools**（纯逻辑）：
//!   - `modules/{workbuddy_account,codebuddy_cn_account}.rs::export_accounts` =
//!     `serde_json::to_string_pretty(Vec<Account>)`，即**明文 JSON 数组**（snake_case 字段），
//!     键名与 `store::parse_accounts_json` 的归一化天然兼容。

use std::io::Read;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::{engine::general_purpose, Engine as _};
use flate2::read::GzDecoder;

use super::models::{BuddyAccount, BuddyPlatform};
use super::{store, workbuddy};

/// 前端的第三方来源 id（与 `buddy_import_third_party` 的 `tool` 参数一致）。
pub const TOOL_WORKDADDY: &str = "workdaddy";
pub const TOOL_COCKPIT: &str = "cockpit-tools";

/// WorkDaddy 导出信封（`secure-transfer.js`）。
const ENVELOPE_FIELD: &str = "wbsExport";
const ENVELOPE_MAGIC: &str = "WorkDaddy";
const ENVELOPE_KDF: &str = "aes-256-gcm+scrypt";
const EXPORT_KIND: &str = "accounts";
const EXPORT_TYPE_ACCOUNTS: &str = "WorkDaddy-accounts";
/// `secure-transfer.js` 的 `MAX_PASSWORD_LENGTH`。
const MAX_PASSWORD_LENGTH: usize = 1024;

/// scrypt 参数 = Node `crypto.scryptSync(password, salt, 32)` 的默认值。
const SCRYPT_LOG_N: u8 = 14;
const SCRYPT_R: u32 = 8;
const SCRYPT_P: u32 = 1;
const KEY_LEN: usize = 32;

/// 打包布局：`iv(12) ‖ authTag(16) ‖ ciphertext`。
const IV_LEN: usize = 12;
const TAG_LEN: usize = 16;

/// 内容是否为 WorkDaddy 加密导出信封（用于把误投的文件给出更准确的提示）。
pub fn is_workdaddy_envelope(content: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(content)
        .ok()
        .and_then(|value| {
            value
                .get(ENVELOPE_FIELD)
                .and_then(|v| v.as_str())
                .map(|magic| magic == ENVELOPE_MAGIC)
        })
        .unwrap_or(false)
}

/// scrypt 派生密钥（参数与 Node `crypto.scryptSync` 默认值一致，否则密文无法解开）。
fn derive_key(password: &str, salt: &[u8]) -> Result<[u8; KEY_LEN], String> {
    let mut key = [0u8; KEY_LEN];
    super::scrypt_kdf::derive(
        password.as_bytes(),
        salt,
        SCRYPT_LOG_N,
        SCRYPT_R,
        SCRYPT_P,
        &mut key,
    )?;
    Ok(key)
}

fn gunzip(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut decoder = GzDecoder::new(bytes);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|_| "导出数据无法解析或已损坏".to_string())?;
    Ok(out)
}

/// 解密 WorkDaddy 导出信封，返回载荷明文（字节）。
///
/// 与 `openEncryptedExport(content, 'accounts', password)` 逐条对齐：
/// 校验信封魔数/版本/kdf/导出类型、salt 长度、打包布局，失败语义也保持一致。
fn decrypt_envelope(content: &str, password: Option<&str>) -> Result<Vec<u8>, String> {
    let text = content.trim_start_matches('\u{feff}');
    let envelope: serde_json::Value = serde_json::from_str(text)
        .map_err(|_| "文件不是有效的导出 JSON".to_string())?;
    let obj = envelope
        .as_object()
        .ok_or_else(|| "不是有效的 WorkDaddy 加密导出文件".to_string())?;

    if obj.get(ENVELOPE_FIELD).and_then(|v| v.as_str()) != Some(ENVELOPE_MAGIC) {
        return Err("不是有效的 WorkDaddy 加密导出文件".to_string());
    }
    let version = obj
        .get("version")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| "导出文件缺少有效的 version 字段".to_string())?;
    if version != 2 && version != 3 {
        return Err(format!("不支持的 WorkDaddy 导出文件版本: {}", version));
    }
    // kdf / exportType 与参考一致：存在则必须匹配
    if let Some(kdf) = obj.get("kdf").and_then(|v| v.as_str()) {
        if kdf != ENVELOPE_KDF {
            return Err(format!("不支持的导出加密算法: {}", kdf));
        }
    }
    if let Some(kind) = obj.get("exportType").and_then(|v| v.as_str()) {
        if kind != EXPORT_KIND {
            return Err(format!(
                "导出文件类型不匹配（{}，需要账号导出）",
                kind
            ));
        }
    }

    let packed_b64 = obj
        .get("data")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "导出文件缺少 data 字段".to_string())?;
    let salt_b64 = obj
        .get("salt")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "导出文件缺少 salt 字段".to_string())?;

    let password = password.unwrap_or("");
    if password.trim().is_empty() {
        return Err("该导出文件已加密，请输入 WorkDaddy 导出密码".to_string());
    }
    if password.chars().count() > MAX_PASSWORD_LENGTH {
        return Err(format!("导出密码不能超过 {} 个字符", MAX_PASSWORD_LENGTH));
    }

    let packed = general_purpose::STANDARD
        .decode(packed_b64)
        .map_err(|_| "导出数据不是有效的 base64".to_string())?;
    if packed.len() <= IV_LEN + TAG_LEN {
        return Err("导出数据不完整或已损坏".to_string());
    }
    let salt = general_purpose::STANDARD
        .decode(salt_b64)
        .map_err(|_| "导出文件缺少有效的加密 salt".to_string())?;
    if salt.len() != 16 {
        return Err("导出文件缺少有效的加密 salt".to_string());
    }

    let key = derive_key(password, &salt)?;
    let iv = &packed[..IV_LEN];
    let tag = &packed[IV_LEN..IV_LEN + TAG_LEN];
    let ciphertext = &packed[IV_LEN + TAG_LEN..];
    // RustCrypto 的 Aes256Gcm 期望 `ciphertext ‖ tag`，而参考侧是 `iv ‖ tag ‖ ciphertext`
    let mut sealed = Vec::with_capacity(ciphertext.len() + TAG_LEN);
    sealed.extend_from_slice(ciphertext);
    sealed.extend_from_slice(tag);

    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| format!("初始化 AES-256-GCM 失败: {}", e))?;
    let mut plain = cipher
        .decrypt(Nonce::from_slice(iv), sealed.as_slice())
        .map_err(|_| "密码错误或导出文件已损坏".to_string())?;

    if version == 3 {
        plain = gunzip(&plain)?;
    }
    Ok(plain)
}

/// WorkDaddy 账号导出 → 账号列表（纯函数，不落库）。
pub fn parse_workdaddy_export(
    content: &str,
    password: Option<&str>,
) -> Result<Vec<BuddyAccount>, String> {
    let plain = decrypt_envelope(content, password)?;
    let payload: serde_json::Value = serde_json::from_slice(&plain)
        .map_err(|_| "导出数据无法解析或已损坏".to_string())?;
    if let Some(kind) = payload.get("exportType").and_then(|v| v.as_str()) {
        if kind != EXPORT_TYPE_ACCOUNTS {
            return Err(format!(
                "导出文件类型不匹配（{}，需要账号导出）",
                kind
            ));
        }
    }
    let accounts = payload
        .get("accounts")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "导出文件中没有账号数据".to_string())?;
    if accounts.is_empty() {
        return Err("导出文件中没有账号数据".to_string());
    }

    let mut parsed = Vec::with_capacity(accounts.len());
    for (idx, item) in accounts.iter().enumerate() {
        let index = idx + 1;
        let obj = item
            .as_object()
            .ok_or_else(|| format!("导出文件第 {} 个账号格式无效", index))?;
        let uid_hint = obj
            .get("uid")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        // 参考侧写入的是备份文件原文（字符串）；这里额外容忍手改后的对象写法
        let info = match obj.get("info") {
            Some(serde_json::Value::String(text)) => text.clone(),
            Some(value @ serde_json::Value::Object(_)) => value.to_string(),
            _ => return Err(format!("导出文件第 {} 个账号缺少 info 字段", index)),
        };
        let account = workbuddy::build_account_from_auth_text(&info, uid_hint)
            .map_err(|e| format!("导出文件第 {} 个账号解析失败: {}", index, e))?
            .ok_or_else(|| format!("导出文件第 {} 个账号没有登录信息", index))?;
        parsed.push(account);
    }
    Ok(parsed)
}

/// cockpit-tools 账号导出 → 账号列表（纯函数，不落库）。
///
/// cockpit-tools 的导出就是账号结构体数组的 `serde_json::to_string_pretty`，
/// 因此直接复用我们既有的 JSON 归一化解析（snake_case → camelCase + 缺省字段补全）。
pub fn parse_cockpit_tools_export(
    platform: BuddyPlatform,
    content: &str,
) -> Result<Vec<BuddyAccount>, String> {
    if is_workdaddy_envelope(content) {
        return Err("这是 WorkDaddy 加密导出文件，请改用上方的 WorkDaddy 来源导入".to_string());
    }
    let accounts = match store::parse_accounts_json(platform, content) {
        Ok(accounts) => accounts,
        Err(e) => {
            // 账号索引（workbuddy_accounts.json）只有摘要没有凭据，单独给出可操作的提示
            if e.contains("accessToken") {
                return Err(
                    "该文件不含账号凭据（可能只是 cockpit-tools 的账号索引）：请在 cockpit-tools 中选中账号并导出后再导入"
                        .to_string(),
                );
            }
            return Err(e);
        }
    };
    if accounts.is_empty() {
        return Err("文件中没有账号数据".to_string());
    }
    Ok(accounts)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WorkDaddy v3 信封（gzip + AES-256-GCM），由参考实现
    /// `createEncryptedExport('accounts', payload, 'pa55w0rd')` 生成，密码 `pa55w0rd`。
    const WORKDADDY_V3: &str = r#"{"wbsExport":"WorkDaddy","version":3,"exportType":"accounts","createdAt":"2026-09-24T00:00:00.000Z","kdf":"aes-256-gcm+scrypt","compression":"gzip","salt":"z4zl2IMf/4Z81quPoWnP1g==","data":"IrKd3/c0y3qOWK0oaG8pbkai+t082k3CkTVerxSdVz5wtqphP6Mhh15cOe/ncovq9taJKjxiScvbKtLzvWzBdmJDIBJxoPwsvlQhmzws6a2dxUfBTDEGAG2YtjW0tCkhq4tRWhsIKTOQ/8vkWYnwAjaH39DcbGc7tBJFu+zmzFvevr7v2Lx5hE+ia74fWAsVi0W4dU3OBwOE0Q8/UYqJhGiL1g8lR7ZktY0ZtRPPdyE7qVhlU1atD7VPOEPWzMPHkePwtT+Ln6DbKmXs/QuWkyOhoxqKvxpdUJs9yBIiWFD1FzwVRfWyTo0VPjXYpMW5GvqzsQxFK7Si+sGquNXEfne1TUFKrpVSLSip4eihrF+ykZbJjOArNTvLbZbuXnsNt8vm5rYbBy3H7b8="}"#;

    /// WorkDaddy v2 信封（不压缩），由参考实现 `encryptExport(plain, password)` 生成。
    const WORKDADDY_V2: &str = r#"{"wbsExport":"WorkDaddy","version":2,"exportType":"accounts","createdAt":"2026-09-24T00:00:00.000Z","kdf":"aes-256-gcm+scrypt","salt":"D/fR0NHVvVnHsDSg6DMnXQ==","data":"8hYVjLN7UHyjvUqMGHncCpmS03M7dlIjoxE+a5qERYblwQJ/thsf0VmP69+uiPHJuARUCiKPxhkXsB+OBM4OxhYg3oFaquofhzH1RdRYFEx79871dNRSPAfjdjdDwklpjn9o/lleG4rhB/gx0MUacjLFXb9MdtcVaEI/nGAuiowg8ysrTHREa/POfV9eIwL0o04/p3mqq/i9ghWByGJlOnH/jdFJt7cLcK7WDHq+RkComMLOGzzkvn6rgHwMRdcoLudD8hR4GAs9rIbxQ99ohB4iU98lQPVehkT3Gwn1Kt4vzyRdGi7DO/tvIdNzWc6GqpdAV3fhHDkqfhCqgrKLy4M6HaO3n8cwvENaASVw5uKziXIZTwzY52L1aWd7jykKqPrDPpzi6Eq2fXVZ5puCBWRJgWlZzQMpc6Igu7IpqQGfRjcEvHH0b5arBOe3TUj0V9DLiEUQ/sfWp/uv9aodGR2GOBpJK+CcjQytn7O7c0NjSZrP6qWdRQPEjsMLU1LsoKZmibj8kvbpq2qCUBHA06e63LAOCW06TSahmlceHNvDqhjhufWwaAci20HzlEySjymwFAuoawTNKWd9EoYbIFpNpB/JNk41MSGjTh/MT2Uc+cpll2fNXwv57+jl/c283o5bcKMQRuoCTDyeVXViTMU1C37PWnoDtxnBwMTiT5o0XZEmCoTzb9nH4FfgN8eD2OlJqdgbs5X9wlKh/15zaWWisnlexXs5QrIGnTGWcgQjzQ=="}"#;

    #[test]
    fn workdaddy_v3_envelope_decrypts_like_reference() {
        let accounts = parse_workdaddy_export(WORKDADDY_V3, Some("pa55w0rd")).unwrap();
        assert_eq!(accounts.len(), 2);
        let first = &accounts[0];
        assert_eq!(first.platform, "workbuddy");
        assert_eq!(first.uid.as_deref(), Some("u-1"));
        assert_eq!(first.nickname.as_deref(), Some("Tester"));
        assert_eq!(first.email, "tester@example.com");
        assert_eq!(first.access_token, "tok-aaaaaaaa.bbbbbbbb.cccccccc");
        assert_eq!(first.refresh_token.as_deref(), Some("refresh-1"));
        assert_eq!(first.domain.as_deref(), Some("www.codebuddy.cn"));
        assert_eq!(first.expires_at, Some(4_102_444_800_000));
        assert_eq!(first.id, format!("workbuddy_{:x}", md5::compute(b"u-1")));
        // 第二个账号缺 refresh/domain，仍然可导入
        assert_eq!(accounts[1].uid.as_deref(), Some("u-2"));
        assert_eq!(accounts[1].nickname.as_deref(), Some("Second"));
    }

    #[test]
    fn workdaddy_v2_envelope_is_uncompressed() {
        let accounts = parse_workdaddy_export(WORKDADDY_V2, Some("pa55w0rd")).unwrap();
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0].uid.as_deref(), Some("u-1"));
        assert_eq!(accounts[1].access_token, "tok-dddddddd.eeeeeeee.ffffffff");
    }

    #[test]
    fn workdaddy_wrong_password_is_reported() {
        let err = parse_workdaddy_export(WORKDADDY_V3, Some("wrong")).unwrap_err();
        assert!(err.contains("密码错误"), "unexpected error: {}", err);
    }

    #[test]
    fn workdaddy_missing_password_is_reported() {
        let err = parse_workdaddy_export(WORKDADDY_V3, None).unwrap_err();
        assert!(err.contains("导出密码"), "unexpected error: {}", err);
    }

    #[test]
    fn workdaddy_rejects_foreign_envelope() {
        let err = parse_workdaddy_export(r#"{"accounts":[]}"#, Some("pa55w0rd")).unwrap_err();
        assert!(err.contains("WorkDaddy"), "unexpected error: {}", err);
        assert!(!is_workdaddy_envelope(r#"{"accounts":[]}"#));
        assert!(is_workdaddy_envelope(WORKDADDY_V3));
    }

    #[test]
    fn workdaddy_rejects_other_export_kind() {
        // 信封声明的是快捷短语导出时，不应被当成账号导入
        let tampered = WORKDADDY_V3.replace("\"exportType\":\"accounts\"", "\"exportType\":\"quick-phrases\"");
        let err = parse_workdaddy_export(&tampered, Some("pa55w0rd")).unwrap_err();
        assert!(err.contains("类型不匹配"), "unexpected error: {}", err);
    }

    #[test]
    fn cockpit_tools_plain_array_maps_all_fields() {
        let content = r#"[
          {
            "id": "workbuddy_ab12",
            "email": "a@example.com",
            "uid": "uid-a",
            "nickname": "Alpha",
            "enterprise_id": "ent-1",
            "enterprise_name": "Ent",
            "tags": ["x"],
            "access_token": "tok-a",
            "refresh_token": "ref-a",
            "token_type": "Bearer",
            "expires_at": 4102444800000,
            "domain": "www.codebuddy.cn",
            "plan_type": "pro",
            "status": "normal"
          },
          {
            "id": "workbuddy_cd34",
            "email": "b@example.com",
            "access_token": "tok-b"
          }
        ]"#;
        let accounts =
            parse_cockpit_tools_export(BuddyPlatform::Workbuddy, content).unwrap();
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0].id, "workbuddy_ab12");
        assert_eq!(accounts[0].access_token, "tok-a");
        assert_eq!(accounts[0].refresh_token.as_deref(), Some("ref-a"));
        assert_eq!(accounts[0].enterprise_id.as_deref(), Some("ent-1"));
        assert_eq!(accounts[0].expires_at, Some(4_102_444_800_000));
        assert_eq!(accounts[0].tags.as_deref(), Some(&["x".to_string()][..]));
        assert!(accounts[0].created_at > 0);
        // 第二条缺 id / 时间字段：由解析层补齐
        assert!(!accounts[1].id.trim().is_empty());
        assert!(accounts[1].created_at > 0);
    }

    #[test]
    fn cockpit_tools_index_without_token_is_rejected() {
        // cockpit-tools 的 workbuddy_accounts.json 只是摘要索引（无 access_token）
        let content = r#"{"version":"1.0","accounts":[{"id":"workbuddy_ab12","email":"a@example.com","created_at":1,"last_used":1}]}"#;
        let err = parse_cockpit_tools_export(BuddyPlatform::Workbuddy, content).unwrap_err();
        assert!(err.contains("不含账号凭据"), "unexpected error: {}", err);
    }

    #[test]
    fn cockpit_tools_entry_rejects_workdaddy_envelope() {
        let err = parse_cockpit_tools_export(BuddyPlatform::Workbuddy, WORKDADDY_V3).unwrap_err();
        assert!(err.contains("WorkDaddy"), "unexpected error: {}", err);
    }
}
