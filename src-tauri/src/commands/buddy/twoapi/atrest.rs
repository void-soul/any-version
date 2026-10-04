//! WorkBuddy at-rest 密钥获取与 `$wbEncrypted` 字段加解密。
//!
//! WorkBuddy 5.6 把登录态里的敏感字段包成 `{"$wbEncrypted":1,"envelope":"<base64>"}`，
//! 密钥只能从 WorkBuddy 自带 Electron 的原生模块拿（`electron_browser_workbuddy_storage`
//! 的 `loggerGet`）。这是与 Python 版 workbuddy2api 逐字节兼容的实现。

use std::path::PathBuf;
use std::process::Command;

/// 手工指定 WorkBuddy 可执行文件（装在非默认位置时用；面板里填写的就是它）
pub const ELECTRON_PATH_ENV: &str = "WORKBUDDY_ELECTRON_PATH";

/// 调原生模块拿 at-rest 密钥的 JS。以 `ELECTRON_RUN_AS_NODE=1` 运行。
const LOGGERGET_JS: &str = "const b = process._linkedBinding('electron_browser_workbuddy_storage');\
process.stdout.write(b.loggerGet().toString('utf8'));";

/// WorkBuddy 可执行文件的候选位置。
///
/// **不能只查默认位置**：本机装在 `D:\sim-tool\WorkBuddy\WorkBuddy.exe`，
/// 只查 `%LOCALAPPDATA%\Programs\WorkBuddy` 会取不到密钥 →
/// 所有对话 500 但 `/health` 等旁路全 200（2026-10-03 实测）。
///
/// 顺序：显式 override → 环境变量 → 已知自定义安装位置 → 各平台默认位置。
pub fn electron_candidates() -> Vec<PathBuf> {
    electron_candidates_with(None)
}

/// 同 [`electron_candidates`]，但可显式指定首选路径（面板填的路径）。
pub fn electron_candidates_with(override_path: Option<PathBuf>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut push = |p: PathBuf| {
        if !out.contains(&p) {
            out.push(p);
        }
    };
    if let Some(p) = override_path {
        push(p);
    }
    if let Some(p) = std::env::var_os(ELECTRON_PATH_ENV).map(PathBuf::from) {
        push(p);
    }
    for extra in ["D:/sim-tool/WorkBuddy/WorkBuddy.exe", "D:/WorkBuddy/WorkBuddy.exe"] {
        push(PathBuf::from(extra));
    }
    match std::env::consts::OS {
        "windows" => {
            if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
                push(
                    local
                        .join("Programs")
                        .join("WorkBuddy")
                        .join("WorkBuddy.exe"),
                );
            }
            push(PathBuf::from("C:/Program Files/WorkBuddy/WorkBuddy.exe"));
        }
        "macos" => push(PathBuf::from(
            "/Applications/WorkBuddy.app/Contents/MacOS/Electron",
        )),
        _ => {
            push(PathBuf::from("/opt/WorkBuddy/workbuddy"));
            push(PathBuf::from("/usr/bin/workbuddy"));
        }
    }
    out.retain(|p| p.is_file());
    out
}

/// 用给定候选列表取 key payload（测试可注入空列表来验证错误信息）。
///
/// 失败信息必须**列出试过的路径** —— 否则"服务正常但对话全 500"无从查起。
fn fetch_key_payload_from(candidates: &[PathBuf]) -> Result<serde_json::Value, String> {
    for exe in candidates {
        let out = Command::new(exe)
            .env("ELECTRON_RUN_AS_NODE", "1")
            .arg("-e")
            .arg(LOGGERGET_JS)
            .output();
        match out {
            Ok(o) if o.status.success() && !o.stdout.is_empty() => {
                match serde_json::from_slice::<serde_json::Value>(&o.stdout) {
                    Ok(v) => return Ok(v),
                    Err(e) => eprintln!(
                        "[2api] loggerGet 输出不是 JSON（{}）: {e}",
                        exe.display()
                    ),
                }
            }
            Ok(o) => eprintln!(
                "[2api] loggerGet 无有效输出: {} rc={:?} err={}",
                exe.display(),
                o.status.code(),
                String::from_utf8_lossy(&o.stderr).trim()
            ),
            Err(e) => eprintln!("[2api] 调用 WorkBuddy 失败 {}: {e}", exe.display()),
        }
    }
    let tried = if candidates.is_empty() {
        "（没有候选路径 —— WorkBuddy 可能未安装）".to_string()
    } else {
        candidates
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("、")
    };
    Err(format!(
        "无法获取 WorkBuddy at-rest 密钥（loggerGet）。已试路径：{tried}。\
         若 WorkBuddy 装在其它位置，请在 2API 面板填写其可执行文件路径（也可设 {ELECTRON_PATH_ENV}）。"
    ))
}

/// 取 key payload（用默认候选列表）
pub fn fetch_key_payload() -> Result<serde_json::Value, String> {
    fetch_key_payload_from(&electron_candidates())
}

/// 从 payload 里取出 `atRestSecretKey`
pub fn extract_secret(payload: &serde_json::Value) -> Result<String, String> {
    payload
        .get("atRestSecretKey")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            "WorkBuddy 返回的 loggerGet 结果里没有 atRestSecretKey（客户端版本可能已变更该接口）"
                .to_string()
        })
}

/// 派生 `(key, keyId)`，与 Python 版逐字节一致：
/// `key = sha256(secret_b64 的 utf8 字节)`，`keyId = sha256(key) 的前 8 字节 hex`。
pub fn derive_protector_key(secret_b64: &str) -> ([u8; 32], String) {
    use sha2::{Digest, Sha256};
    let key = Sha256::digest(secret_b64.as_bytes());
    // keyId 是 **key 的 sha256** 的前 8 字节 hex（不是 key 自身的前 8 字节）。
    // 少这一步 hash 的话，keyId 与客户端写入 envelope 的值永远对不上，
    // 而 keyId 既参与 AAD 又被显式校验 → 每一个 $wbEncrypted 字段都解不开。
    let key_id = Sha256::digest(key)
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let mut out = [0u8; 32];
    out.copy_from_slice(&key);
    (out, key_id)
}

// ── `$wbEncrypted` 字段加解密 ──
//
// envelope（外层 `{"$wbEncrypted":1,"envelope":"<base64>"}`，内层解码后）：
// `{"suite":1,"keyId":"<16hex>","nonce":"<b64 12B>","authTag":"<b64 16B>","ciphertext":"<b64>"}`
//
// 注意 AEAD 收到的密文是 `ciphertext || authTag` 拼接（Rust 侧 tag 在密文末尾），
// 写回时要拆开存 —— 与 Python 版 `ct[:-16]` / `ct[-16:]` 一致。

/// 是否是 WorkBuddy 5.6+ 的加密字段包装
pub fn is_encrypted(value: &serde_json::Value) -> bool {
    value.get("$wbEncrypted").and_then(|v| v.as_i64()) == Some(1)
        && value.get("envelope").and_then(|v| v.as_str()).is_some()
}

/// 长度前缀（field framing）：`u32be(len(x)) ‖ x`
fn lp(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}

/// AAD 帧 `sym-v1`。keyId 既在这里被绑定，也被 envelope 显式校验 —— 算错就解不开。
pub fn aad_field(key_id: &str) -> Vec<u8> {
    let mut aad = b"WB-AAD\0\x01".to_vec();
    lp(&mut aad, b"WBEV1");
    lp(&mut aad, b"sym-v1");
    aad.extend_from_slice(&1u32.to_be_bytes());
    lp(&mut aad, key_id.as_bytes());
    aad.extend_from_slice(&[0x02, 0x00, 0x00]);
    aad
}

/// 把明文加密成 `$wbEncrypted` 包装（写回登录态时用，保持客户端 5.6+ 可读）
pub fn encrypt_field(plaintext: &str, secret_b64: &str) -> Result<serde_json::Value, String> {
    use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
    use aes_gcm::Aes256Gcm;
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};

    let (key, key_id) = derive_protector_key(secret_b64);
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| format!("AES 初始化失败: {e}"))?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let aad = aad_field(&key_id);
    let ct = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext.as_bytes(),
                aad: &aad,
            },
        )
        .map_err(|e| format!("AES-GCM 加密失败: {e}"))?;
    let (body, tag) = ct.split_at(ct.len() - 16);
    let envelope = serde_json::json!({
        "suite": 1,
        "keyId": key_id,
        "nonce": B64.encode(nonce.as_slice()),
        "authTag": B64.encode(tag),
        "ciphertext": B64.encode(body),
    });
    Ok(serde_json::json!({
        "$wbEncrypted": 1,
        "envelope": B64.encode(serde_json::to_vec(&envelope).unwrap()),
    }))
}

/// 解密 `$wbEncrypted` 字段；明文（5.6 之前）原样返回。
///
/// `secret_b64` 与写 envelope 的客户端必须是同一台机器的 WorkBuddy，否则 keyId 对不上。
pub fn decrypt_field(value: &serde_json::Value, secret_b64: &str) -> Result<String, String> {
    use aes_gcm::aead::{Aead, KeyInit, Payload};
    use aes_gcm::{Aes256Gcm, Nonce};
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};

    if !is_encrypted(value) {
        return Ok(value.as_str().unwrap_or_default().to_string());
    }
    let (key, my_key_id) = derive_protector_key(secret_b64);
    let raw = B64
        .decode(value["envelope"].as_str().unwrap_or_default())
        .map_err(|e| format!("envelope 不是合法 base64: {e}"))?;
    let env: serde_json::Value =
        serde_json::from_slice(&raw).map_err(|e| format!("envelope 不是合法 JSON: {e}"))?;
    if env["suite"].as_u64() != Some(1) {
        return Err(format!("不支持的加密 suite：{}", env["suite"]));
    }
    let env_key_id = env["keyId"].as_str().unwrap_or_default();
    if env_key_id != my_key_id {
        return Err(format!(
            "envelope keyId({env_key_id}) 与本机派生 keyId({my_key_id}) 不一致，\
             WorkBuddy 可能已更换密钥，请重新获取"
        ));
    }
    let decode = |name: &str| -> Result<Vec<u8>, String> {
        B64.decode(env[name].as_str().unwrap_or_default())
            .map_err(|e| format!("envelope.{name} 不是合法 base64: {e}"))
    };
    let nonce = decode("nonce")?;
    let mut ct = decode("ciphertext")?;
    ct.extend_from_slice(&decode("authTag")?);
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| format!("AES 初始化失败: {e}"))?;
    let pt = cipher
        .decrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &ct,
                aad: &aad_field(env_key_id),
            },
        )
        .map_err(|e| format!("AES-GCM 解密失败（密钥不匹配或数据已损坏）: {e}"))?;
    String::from_utf8(pt).map_err(|e| format!("解密结果不是 UTF-8: {e}"))
}

/// 写回登录态时替换 `auth.accessToken`，**保持原有的加密包装形式**。
///
/// 原来明文就仍写明文，原来是 `$wbEncrypted` 就重新加密写回 —— 明文写回会让
/// WorkBuddy 客户端读到后报 integrity 错误，甚至重置登录态（上游 issue #23）。
pub fn set_access_token(
    raw: &mut serde_json::Value,
    new_token: &str,
    secret_b64: &str,
) -> Result<(), String> {
    let field = raw
        .get_mut("auth")
        .and_then(|a| a.get_mut("accessToken"))
        .ok_or_else(|| "登录态 JSON 里没有 auth.accessToken".to_string())?;
    let replacement = if is_encrypted(&field.clone()) {
        encrypt_field(new_token, secret_b64)?
    } else {
        serde_json::Value::String(new_token.to_string())
    };
    *field = replacement;
    Ok(())
}

/// 写回登录态时替换 `auth.refreshToken`（与 accessToken 同样保持加密包装）。
pub fn set_refresh_token(
    raw: &mut serde_json::Value,
    new_token: &str,
    secret_b64: &str,
) -> Result<(), String> {
    let field = raw
        .get_mut("auth")
        .and_then(|a| a.get_mut("refreshToken"))
        .ok_or_else(|| "登录态 JSON 里没有 auth.refreshToken".to_string())?;
    let replacement = if is_encrypted(&field.clone()) {
        encrypt_field(new_token, secret_b64)?
    } else {
        serde_json::Value::String(new_token.to_string())
    };
    *field = replacement;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_are_absolute_paths() {
        // 候选必须是绝对路径：相对路径在不同工作目录下含义不同
        for p in electron_candidates() {
            assert!(p.is_absolute(), "候选必须是绝对路径: {}", p.display());
        }
    }

    #[test]
    fn override_path_comes_first_and_survives_filtering() {
        // 面板填的路径必须排第一，且不能被 is_file 过滤掉
        let exe = std::env::current_exe().expect("当前可执行文件必然存在");
        let cands = electron_candidates_with(Some(exe.clone()));
        assert_eq!(cands.first(), Some(&exe));
    }

    #[test]
    fn derive_key_shape_and_determinism() {
        let (key, key_id) = derive_protector_key("Sik9U5aXhCdwTVEwsEySDOmDoB9r9ntFxHF1fst9LQI=");
        assert_eq!(key.len(), 32, "AES-256-GCM 密钥必须是 32 字节");
        assert_eq!(key_id.len(), 16, "keyId 是 16 个 hex 字符");
        assert!(key_id.chars().all(|c| c.is_ascii_hexdigit()));
        let (key2, key_id2) = derive_protector_key("Sik9U5aXhCdwTVEwsEySDOmDoB9r9ntFxHF1fst9LQI=");
        assert_eq!(key, key2, "同 secret 必须派生出同 key");
        assert_eq!(key_id, key_id2);
    }

    #[test]
    fn derive_algorithm_matches_python() {
        // 钉死派生算法（Python: key=sha256(secret)；keyId=sha256(key) 前 16 hex）
        use sha2::{Digest, Sha256};
        let secret = "test-secret";
        let (key, key_id) = derive_protector_key(secret);
        let expect_key = Sha256::digest(secret.as_bytes());
        assert_eq!(key.to_vec(), expect_key.to_vec());
        let expect_id: String = Sha256::digest(expect_key)
            .iter()
            .take(8)
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(key_id, expect_id);
    }

    #[test]
    fn missing_key_payload_error_lists_tried_paths() {
        // 拿不到密钥时必须可归因：列出试过的路径（今天的教训：静默失败无法定位）
        let err = fetch_key_payload_from(&[]).unwrap_err();
        assert!(err.contains("已试路径"), "错误要说明已试路径: {err}");
        assert!(
            err.contains(ELECTRON_PATH_ENV),
            "错误要告诉用户怎么手工指定: {err}"
        );
    }

    #[test]
    fn nonexistent_candidate_reports_its_path() {
        let ghost = PathBuf::from("D:/definitely-not-installed/WorkBuddy.exe");
        let err = fetch_key_payload_from(&[ghost.clone()]).unwrap_err();
        assert!(err.contains(&ghost.display().to_string()), "{err}");
    }

    #[test]
    fn extract_secret_rejects_payload_without_key() {
        let payload = serde_json::json!({"version": 1});
        let err = extract_secret(&payload).unwrap_err();
        assert!(err.contains("atRestSecretKey"), "{err}");
        assert!(extract_secret(&serde_json::json!({"atRestSecretKey": ""})).is_err());
        assert_eq!(
            extract_secret(&serde_json::json!({"atRestSecretKey": "abc"})).unwrap(),
            "abc"
        );
    }

    // ── 以下为 Task 2（加解密）的用例 ──

    #[test]
    fn aad_frame_is_byte_exact() {
        // 帧错一位就解不开：sym-v1 field framing 必须字节级钉死
        //   "WB-AAD\0" ‖ 0x01 ‖ u32be(5)+"WBEV1" ‖ u32be(7)+"sym-v1" ‖ u32be(1)
        //   ‖ u32be(len)+keyId ‖ 0x02 ‖ 0x00 ‖ 0x00
        // （"WBEV1" 是 5 个 ASCII 字符，"sym-v1" 是 6 个 —— 长度前缀按字节数，不是"看起来像几位"）
        let key_id = "0123456789abcdef";
        let mut expect: Vec<u8> = Vec::new();
        expect.extend_from_slice(b"WB-AAD\0\x01");
        expect.extend_from_slice(&5u32.to_be_bytes());
        expect.extend_from_slice(b"WBEV1");
        expect.extend_from_slice(&6u32.to_be_bytes());
        expect.extend_from_slice(b"sym-v1");
        expect.extend_from_slice(&1u32.to_be_bytes());
        expect.extend_from_slice(&16u32.to_be_bytes());
        expect.extend_from_slice(key_id.as_bytes());
        expect.extend_from_slice(&[0x02, 0x00, 0x00]);
        assert_eq!(super::aad_field(key_id), expect);
    }

    #[test]
    fn aad_frame_uses_actual_key_id_length() {
        // keyId 长度不是固定 16 时，长度前缀必须跟着变（否则解不开）
        let aad = super::aad_field("abcd");
        // 找到 keyId 之前的 4 字节长度前缀
        let pos = aad
            .windows(4)
            .position(|w| w == 4u32.to_be_bytes() && &aad[aad.len() - 7..aad.len() - 3] == b"abcd")
            .expect("应存在长度前缀 4");
        assert_eq!(u32::from_be_bytes([aad[pos], aad[pos + 1], aad[pos + 2], aad[pos + 3]]), 4);
    }

    #[test]
    fn encrypt_then_decrypt_roundtrips() {
        let secret = "Sik9U5aXhCdwTVEwsEySDOmDoB9r9ntFxHF1fst9LQI=";
        let wrapped = super::encrypt_field("hello-token", secret).unwrap();
        assert_eq!(wrapped["$wbEncrypted"], 1);
        assert!(wrapped["envelope"].as_str().is_some(), "envelope 应是 base64 字符串");
        assert_eq!(super::decrypt_field(&wrapped, secret).unwrap(), "hello-token");
    }

    #[test]
    fn roundtrip_preserves_unicode_and_long_text() {
        // token / 昵称可能含非 ASCII 与较长内容
        let secret = "Sik9U5aXhCdwTVEwsEySDOmDoB9r9ntFxHF1fst9LQI=";
        let plain = "张三の昵称-🚀-".repeat(200);
        let wrapped = super::encrypt_field(&plain, secret).unwrap();
        assert_eq!(super::decrypt_field(&wrapped, secret).unwrap(), plain);
    }

    #[test]
    fn plaintext_field_passes_through() {
        // 5.6 之前的明文字段不能报错（Python 版 is_encrypted_field 为假时原样返回）
        assert_eq!(super::decrypt_field(&serde_json::json!("plain"), "x").unwrap(), "plain");
        assert!(!super::is_encrypted(&serde_json::json!("plain")));
        assert!(super::is_encrypted(
            &serde_json::json!({"$wbEncrypted": 1, "envelope": "e"})
        ));
    }

    #[test]
    fn non_string_non_encrypted_field_yields_empty() {
        assert_eq!(super::decrypt_field(&serde_json::json!(42), "x").unwrap(), "");
        assert_eq!(super::decrypt_field(&serde_json::Value::Null, "x").unwrap(), "");
    }

    #[test]
    fn keyid_mismatch_reports_both_ids() {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let secret = "Sik9U5aXhCdwTVEwsEySDOmDoB9r9ntFxHF1fst9LQI=";
        let mut wrapped = super::encrypt_field("x", secret).unwrap();
        let mut env: serde_json::Value = serde_json::from_str(
            std::str::from_utf8(&STANDARD.decode(wrapped["envelope"].as_str().unwrap()).unwrap())
                .unwrap(),
        )
        .unwrap();
        env["keyId"] = serde_json::json!("ffffffffffffffff");
        wrapped["envelope"] =
            serde_json::Value::String(STANDARD.encode(serde_json::to_vec(&env).unwrap()));
        let err = super::decrypt_field(&wrapped, secret).unwrap_err();
        assert!(err.contains("ffffffffffffffff"), "错误要带上 envelope 的 keyId: {err}");
        assert!(
            err.contains(&derive_protector_key(secret).1),
            "错误要带上本机 keyId: {err}"
        );
    }

    #[test]
    fn unsupported_suite_is_rejected() {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let secret = "Sik9U5aXhCdwTVEwsEySDOmDoB9r9ntFxHF1fst9LQI=";
        let mut wrapped = super::encrypt_field("x", secret).unwrap();
        let mut env: serde_json::Value = serde_json::from_str(
            std::str::from_utf8(&STANDARD.decode(wrapped["envelope"].as_str().unwrap()).unwrap())
                .unwrap(),
        )
        .unwrap();
        env["suite"] = serde_json::json!(2);
        wrapped["envelope"] =
            serde_json::Value::String(STANDARD.encode(serde_json::to_vec(&env).unwrap()));
        let err = super::decrypt_field(&wrapped, secret).unwrap_err();
        assert!(err.contains("suite"), "{err}");
    }

    #[test]
    fn wrong_secret_fails_to_decrypt() {
        let wrapped = super::encrypt_field("secret-data", "right-secret").unwrap();
        // 换个 secret → keyId 不同 → 明确报 keyId 不一致
        assert!(super::decrypt_field(&wrapped, "wrong-secret").is_err());
    }

    #[test]
    fn decrypt_rejects_garbage_envelope() {
        let bad = serde_json::json!({"$wbEncrypted": 1, "envelope": "not-base64!!"});
        assert!(super::decrypt_field(&bad, "secret").is_err());
        let bad2 = serde_json::json!({"$wbEncrypted": 1, "envelope": "aGVsbG8="}); // 合法 b64 但不是 JSON
        assert!(super::decrypt_field(&bad2, "secret").is_err());
    }

    #[test]
    fn encrypt_uses_random_nonce_each_time() {
        // 同一明文两次加密必须产出不同 envelope（nonce 随机）
        let a = super::encrypt_field("same", "secret").unwrap();
        let b = super::encrypt_field("same", "secret").unwrap();
        assert_ne!(a["envelope"], b["envelope"], "nonce 必须每次随机");
    }

    #[test]
    fn envelope_shape_matches_python() {
        // envelope 内层字段名与拼装顺序要与 Python 版一致（客户端按这些字段读）
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let secret = "Sik9U5aXhCdwTVEwsEySDOmDoB9r9ntFxHF1fst9LQI=";
        let wrapped = super::encrypt_field("payload", secret).unwrap();
        let raw = STANDARD.decode(wrapped["envelope"].as_str().unwrap()).unwrap();
        let env: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        let (_, key_id) = derive_protector_key(secret);
        assert_eq!(env["suite"], 1);
        assert_eq!(env["keyId"], key_id.as_str());
        for field in ["nonce", "authTag", "ciphertext"] {
            assert!(env[field].as_str().is_some(), "缺字段 {field}");
        }
        // nonce 12 字节、authTag 16 字节
        assert_eq!(STANDARD.decode(env["nonce"].as_str().unwrap()).unwrap().len(), 12);
        assert_eq!(STANDARD.decode(env["authTag"].as_str().unwrap()).unwrap().len(), 16);
    }

    /// 真机验证：用本机 WorkBuddy 登录态走一遍解密 → 重新加密 → 再解密。
    ///
    /// 单元测试的 AAD 帧与派生算法是**我们按 Python 版推出来的**；只有拿客户端
    /// 真正写出的 envelope 解开，才算与 WorkBuddy 5.6 实际兼容 —— 帧差一位就全盘失败。
    ///
    /// 需要本机安装 WorkBuddy，默认不跑：
    /// `cargo test --no-default-features --lib -- --ignored twoapi::atrest::real`
    #[test]
    #[ignore = "需要本机安装并登录 WorkBuddy"]
    fn real_auth_file_decrypt_roundtrip() {
        let Some(path) = real_auth_file() else {
            eprintln!("跳过：本机未找到 workbuddy-desktop.info");
            return;
        };
        eprintln!("auth 文件: {}", path.display());
        let raw: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let token = raw["auth"]["accessToken"].clone();
        if !super::is_encrypted(&token) {
            eprintln!("跳过：该 auth 文件是明文格式（旧版客户端）");
            return;
        }

        let candidates = electron_candidates();
        eprintln!(
            "WorkBuddy 候选路径: {}",
            candidates
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join("、")
        );
        let secret = extract_secret(&fetch_key_payload_from(&candidates).unwrap()).unwrap();

        let plain = super::decrypt_field(&token, &secret).expect("解密 accessToken 失败");
        assert!(!plain.is_empty(), "解密结果不应为空");
        let head: String = plain.chars().take(20).collect();
        eprintln!("accessToken 解密成功: {head}… 长度 {}", plain.len());

        // 往返：明文 → 加密 → 解密 必须一致（写回登录态时依赖这个）
        let rewrapped = super::encrypt_field(&plain, &secret).unwrap();
        assert_eq!(super::decrypt_field(&rewrapped, &secret).unwrap(), plain);
        eprintln!("往返加密验证通过");

        // 换密钥必须解不开（证明 AAD/keyId 真的在起作用，不是碰巧解开）
        let other = super::encrypt_field(&plain, "d3Jvbmctc2VjcmV0").unwrap();
        assert!(super::decrypt_field(&other, &secret).is_err());
        eprintln!("错误密钥被正确拒绝");
    }

    fn real_auth_file() -> Option<std::path::PathBuf> {
        let mut dirs: Vec<std::path::PathBuf> = Vec::new();
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(std::path::PathBuf::from(local).join("CodeBuddyExtension/Data/Public/auth"));
        }
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(std::path::PathBuf::from(home).join("Library/Application Support/CodeBuddyExtension/Data/Public/auth"));
        }
        dirs.into_iter()
            .map(|d| d.join("workbuddy-desktop.info"))
            .find(|p| p.is_file())
    }
}
