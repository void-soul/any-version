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
}
