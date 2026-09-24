//! Codex 官方登录态（`~/.codex/auth.json`）账号快照。
//!
//! 抄自 ai-toolbox c126d68e（`feat(codex): import accounts from auth.json`）。其 AGENTS.md
//! 记着一条关键约束：**从页面粘贴导入的 auth.json 只保存为官方账号快照，不覆盖当前
//! live auth.json；只有用户显式应用该账号时才写入运行时文件。** 这里照此实现闭环：
//! 导入 → 快照库 → 应用（先备份原文件）→ 删除。
//!
//! 与 any-version 现有约定一致：账号类数据用 JSON 文件落盘（不做加密，与 API Key 同策略）。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Codex 运行时登录态文件（相对用户主目录）。
pub const CODEX_AUTH_RELATIVE_PATH: &str = ".codex/auth.json";
/// 快照库文件名（落在 `data_dir/codex_auth/` 下）。
pub const STORE_FILE_NAME: &str = "accounts.json";

/// 一份 auth.json 快照（含原始 token，故只落盘、不经命令原样回传前端）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexAuthAccount {
    pub id: String,
    pub label: String,
    pub saved_at: String,
    #[serde(default)]
    pub auth: Value,
}

/// 面向前端的摘要（不含任何 token）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexAuthSummary {
    pub id: String,
    pub label: String,
    pub saved_at: String,
    /// 是否为当前 live `~/.codex/auth.json` 对应的账号
    pub is_current: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct StoreFile {
    #[serde(default)]
    accounts: Vec<CodexAuthAccount>,
}

fn non_empty_str(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// auth.json 是否含可用的官方 Codex 登录态：access / refresh 双 token 都必须非空
/// （只有 access_token 时刷新不了，登出后的残留文件常长这样）。
pub fn auth_has_official_login(auth: &Value) -> bool {
    let tokens = auth.get("tokens");
    non_empty_str(tokens.and_then(|t| t.get("access_token"))).is_some()
        && non_empty_str(tokens.and_then(|t| t.get("refresh_token"))).is_some()
}

/// 解析用户粘贴的 auth.json 内容。
///
/// 报错文案只说明结构性原因，不回显内容（可能含凭据）。
pub fn parse_imported_auth_json(content: &str) -> Result<Value, String> {
    if content.trim().is_empty() {
        return Err("auth.json 内容为空".to_string());
    }
    let auth: Value =
        serde_json::from_str(content).map_err(|e| format!("auth.json 不是合法 JSON: {}", e))?;
    if !auth.is_object() {
        return Err("auth.json 必须是 JSON 对象".to_string());
    }
    if !auth_has_official_login(&auth) {
        return Err(
            "auth.json 不含官方 Codex 登录态（缺少 access_token / refresh_token）".to_string(),
        );
    }
    Ok(auth)
}

/// 账号身份指纹：优先 `tokens.account_id`（可读、跨设备稳定）；
/// 缺失时退回 refresh_token 的 SHA-256 前缀 —— 不把 token 明文写进 id / 文件名。
pub fn account_fingerprint(auth: &Value) -> String {
    let tokens = auth.get("tokens");
    if let Some(account_id) = non_empty_str(tokens.and_then(|t| t.get("account_id"))) {
        return format!("codex-{}", account_id);
    }
    let seed = non_empty_str(tokens.and_then(|t| t.get("refresh_token")))
        .or_else(|| non_empty_str(tokens.and_then(|t| t.get("access_token"))))
        .unwrap_or("unknown");
    let digest = Sha256::digest(seed.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{:02x}", b)).collect();
    format!("codex-{}", &hex[..16])
}

/// 快照显示名：显式名 > account_id > email > 兜底文案。
pub fn account_label_from_auth(auth: &Value, explicit: Option<&str>) -> String {
    if let Some(label) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        return label.to_string();
    }
    let tokens = auth.get("tokens");
    for key in ["account_id", "email"] {
        if let Some(value) = non_empty_str(tokens.and_then(|t| t.get(key))) {
            return value.to_string();
        }
    }
    "Codex 账号".to_string()
}

/// 插入或更新快照。返回是否真的写入了（`overwrite = false` 时同一指纹已存在则跳过）。
pub fn upsert_snapshot(
    list: &mut Vec<CodexAuthAccount>,
    account: CodexAuthAccount,
    overwrite: bool,
) -> bool {
    if let Some(existing) = list.iter_mut().find(|a| a.id == account.id) {
        if !overwrite {
            return false;
        }
        *existing = account;
        return true;
    }
    list.push(account);
    true
}

/// live auth.json 是否就是这份快照（同账号；token 轮换过也算）。
pub fn live_matches_snapshot(snapshot: &Value, live: &Value) -> bool {
    let snapshot_id = non_empty_str(snapshot.get("tokens").and_then(|t| t.get("account_id")));
    let live_id = non_empty_str(live.get("tokens").and_then(|t| t.get("account_id")));
    match (snapshot_id, live_id) {
        (Some(a), Some(b)) => a == b,
        _ => account_fingerprint(snapshot) == account_fingerprint(live),
    }
}

/// 快照库文件路径。
pub fn store_path(base: &Path) -> PathBuf {
    base.join("codex_auth").join(STORE_FILE_NAME)
}

/// 读取快照库；文件缺失或损坏一律返回空表（宁可空面板，也不要打不开）。
pub fn load_store(base: &Path) -> Vec<CodexAuthAccount> {
    let Ok(raw) = std::fs::read_to_string(store_path(base)) else {
        return Vec::new();
    };
    serde_json::from_str::<StoreFile>(&raw)
        .map(|file| file.accounts)
        .unwrap_or_default()
}

/// 原子写入快照库。
pub fn save_store(base: &Path, accounts: &[CodexAuthAccount]) -> Result<(), String> {
    let payload = serde_json::to_string_pretty(&StoreFile {
        accounts: accounts.to_vec(),
    })
    .map_err(|e| format!("序列化账号库失败: {}", e))?;
    crate::commands::config::atomic_write_file(&store_path(base), payload.as_bytes())
}

// ─── 运行时文件读写 ───

fn home_dir() -> Option<PathBuf> {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()?;
    if home.trim().is_empty() {
        return None;
    }
    Some(PathBuf::from(home))
}

/// live `~/.codex/auth.json` 路径。
pub fn live_auth_path() -> Option<PathBuf> {
    Some(home_dir()?.join(".codex").join("auth.json"))
}

/// 读取 live 登录态（不存在 / 损坏时 None）。
fn read_live_auth() -> Option<Value> {
    let raw = std::fs::read_to_string(live_auth_path()?).ok()?;
    serde_json::from_str(&raw).ok()
}

fn data_dir() -> PathBuf {
    crate::commands::config::get_data_dir()
}

fn now_iso() -> String {
    chrono::Local::now().to_rfc3339()
}

fn summarize(account: &CodexAuthAccount, live: Option<&Value>) -> CodexAuthSummary {
    CodexAuthSummary {
        id: account.id.clone(),
        label: account.label.clone(),
        saved_at: account.saved_at.clone(),
        is_current: live
            .map(|live| live_matches_snapshot(&account.auth, live))
            .unwrap_or(false),
    }
}

// ─── Tauri 命令 ───

/// 展示用路径信息（live 文件 + 快照库 + 是否已存在 live 登录态）。
#[tauri::command]
pub fn codex_auth_paths() -> Value {
    let live = live_auth_path();
    serde_json::json!({
        "liveAuth": live.as_ref().map(|p| p.to_string_lossy().to_string()),
        "liveExists": live.map(|p| p.exists()).unwrap_or(false),
        "store": store_path(&data_dir()).to_string_lossy().to_string(),
    })
}

#[tauri::command]
pub fn codex_auth_list() -> Result<Vec<CodexAuthSummary>, String> {
    let live = read_live_auth();
    Ok(load_store(&data_dir())
        .iter()
        .map(|account| summarize(account, live.as_ref()))
        .collect())
}

/// 从粘贴内容导入快照。`overwrite = false` 时同一账号已存在则原样返回旧条目。
#[tauri::command]
pub fn codex_auth_import(
    auth_json: String,
    label: Option<String>,
    overwrite: Option<bool>,
) -> Result<CodexAuthSummary, String> {
    let auth = parse_imported_auth_json(&auth_json)?;
    let base = data_dir();
    let mut accounts = load_store(&base);
    let id = account_fingerprint(&auth);
    let account = CodexAuthAccount {
        id: id.clone(),
        label: account_label_from_auth(&auth, label.as_deref()),
        saved_at: now_iso(),
        auth,
    };
    let changed = upsert_snapshot(&mut accounts, account, overwrite.unwrap_or(false));
    if changed {
        save_store(&base, &accounts)?;
    }
    let stored = accounts
        .iter()
        .find(|a| a.id == id)
        .cloned()
        .ok_or("导入后未能找到账号条目")?;
    Ok(summarize(&stored, read_live_auth().as_ref()))
}

/// 把当前 live 登录态存成快照（首次使用时的便捷入口）。
#[tauri::command]
pub fn codex_auth_capture_current(label: Option<String>) -> Result<CodexAuthSummary, String> {
    let live = read_live_auth().ok_or(
        "未找到 ~/.codex/auth.json（请先在 Codex 里完成官方登录，或粘贴 auth.json 内容导入）",
    )?;
    codex_auth_import(live.to_string(), label, Some(false))
}

/// 应用快照到 live auth.json；写入前先备份原文件。
#[tauri::command]
pub fn codex_auth_apply(id: String) -> Result<String, String> {
    let base = data_dir();
    let accounts = load_store(&base);
    let account = accounts
        .iter()
        .find(|a| a.id == id)
        .ok_or_else(|| format!("未找到账号快照: {}", id))?;
    // 双保险：库里存的内容也必须仍是合法登录态，避免手工改坏后写进运行时
    if !auth_has_official_login(&account.auth) {
        return Err("该快照不含完整登录态（缺少 access_token / refresh_token）".to_string());
    }

    let path = live_auth_path().ok_or("无法定位用户主目录")?;
    let backup = if path.exists() {
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
        let backup = path.with_file_name(format!("auth.json.bak-{}", stamp));
        std::fs::copy(&path, &backup)
            .map_err(|e| format!("备份原 auth.json 失败: {}", e))?;
        Some(backup)
    } else {
        None
    };

    let payload = serde_json::to_string_pretty(&account.auth)
        .map_err(|e| format!("序列化登录态失败: {}", e))?;
    crate::commands::config::atomic_write_file(&path, payload.as_bytes())?;

    Ok(match backup {
        Some(backup) => format!(
            "已应用「{}」，原登录态备份于 {}",
            account.label,
            backup.to_string_lossy()
        ),
        None => format!("已应用「{}」", account.label),
    })
}

#[tauri::command]
pub fn codex_auth_delete(id: String) -> Result<(), String> {
    let base = data_dir();
    let mut accounts = load_store(&base);
    let before = accounts.len();
    accounts.retain(|a| a.id != id);
    if accounts.len() == before {
        return Err(format!("未找到账号快照: {}", id));
    }
    save_store(&base, &accounts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn valid_auth() -> serde_json::Value {
        json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "access_token": "access-token-1",
                "refresh_token": "refresh-token-1",
                "account_id": "acct-1"
            }
        })
    }

    #[test]
    fn parse_rejects_blank_non_json_non_object_and_keyless() {
        assert!(parse_imported_auth_json("").is_err());
        assert!(parse_imported_auth_json("   \n ").is_err());
        assert!(parse_imported_auth_json("not-json").is_err());
        assert!(parse_imported_auth_json("[]").is_err());
        // 只剩 access_token：刷新不了，算不上官方登录态
        assert!(parse_imported_auth_json(
            r#"{"auth_mode":"chatgpt","tokens":{"access_token":"only"}}"#
        )
        .is_err());
        assert!(parse_imported_auth_json(r#"{"auth_mode":"chatgpt"}"#).is_err());
        assert!(parse_imported_auth_json("{}").is_err());
    }

    #[test]
    fn parse_accepts_valid_login_and_trims_surrounding_whitespace() {
        let parsed = parse_imported_auth_json(&format!("\n {}\n ", valid_auth())).unwrap();
        assert_eq!(parsed["auth_mode"], json!("chatgpt"));
        assert_eq!(parsed["tokens"]["refresh_token"], json!("refresh-token-1"));
    }

    #[test]
    fn blank_token_values_do_not_count_as_a_login() {
        // 空串 token 很常见（登出后残留），必须与缺失等价
        let blank = json!({"tokens": {"access_token": "  ", "refresh_token": "r"}});
        assert!(!auth_has_official_login(&blank));
        assert!(auth_has_official_login(&valid_auth()));
    }

    #[test]
    fn fingerprint_prefers_account_id_and_is_stable() {
        let auth = valid_auth();
        let fp = account_fingerprint(&auth);
        assert_eq!(fp, account_fingerprint(&auth));
        assert!(fp.contains("acct-1"), "应带可读的账号标识: {fp}");

        // 没有 account_id 时用 refresh_token 哈希区分账号，同一 token 仍然稳定
        let anonymous = json!({"tokens": {"access_token": "a", "refresh_token": "rt-9"}});
        let fp2 = account_fingerprint(&anonymous);
        assert_eq!(fp2, account_fingerprint(&anonymous));
        assert_ne!(fp2, fp);
        assert!(!fp2.contains("rt-9"), "不应把 refresh_token 明文写进 id: {fp2}");
    }

    #[test]
    fn label_prefers_explicit_then_account_id_then_email() {
        assert_eq!(account_label_from_auth(&valid_auth(), None), "acct-1");
        assert_eq!(account_label_from_auth(&valid_auth(), Some("  我的号  ")), "我的号");
        let by_email = json!({"tokens": {"access_token": "a", "refresh_token": "r", "email": "me@example.com"}});
        assert_eq!(account_label_from_auth(&by_email, None), "me@example.com");
        let bare = json!({"tokens": {"access_token": "a", "refresh_token": "r"}});
        assert_eq!(account_label_from_auth(&bare, None), "Codex 账号");
    }

    #[test]
    fn upsert_replaces_same_fingerprint_and_keeps_others() {
        let mut list: Vec<CodexAuthAccount> = Vec::new();
        let first = CodexAuthAccount {
            id: account_fingerprint(&valid_auth()),
            label: "第一版".to_string(),
            saved_at: "2026-09-24T00:00:00Z".to_string(),
            auth: valid_auth(),
        };
        upsert_snapshot(&mut list, first.clone(), false);
        assert_eq!(list.len(), 1);

        // 同一账号再次导入：默认去重（沿用旧条目，返回 false）
        let mut again = first.clone();
        again.label = "改名".to_string();
        assert!(!upsert_snapshot(&mut list, again.clone(), false));
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].label, "第一版");

        // 允许覆盖时替换内容
        assert!(upsert_snapshot(&mut list, again.clone(), true));
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].label, "改名");

        // 另一个账号追加
        let other = CodexAuthAccount {
            id: account_fingerprint(&json!({"tokens": {"access_token": "a", "refresh_token": "rt-2"}})),
            label: "第二版".to_string(),
            saved_at: "2026-09-24T01:00:00Z".to_string(),
            auth: json!({"tokens": {"access_token": "a", "refresh_token": "rt-2"}}),
        };
        assert!(upsert_snapshot(&mut list, other, false));
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn store_round_trips_and_survives_corrupt_file() {
        let dir = probe_dir("roundtrip");
        assert!(load_store(&dir).is_empty(), "缺失文件应返回空表而不是报错");

        let account = CodexAuthAccount {
            id: "fp-1".to_string(),
            label: "A".to_string(),
            saved_at: "2026-09-24T00:00:00Z".to_string(),
            auth: valid_auth(),
        };
        save_store(&dir, &[account.clone()]).unwrap();
        let loaded = load_store(&dir);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "fp-1");
        assert_eq!(loaded[0].auth["tokens"]["access_token"], json!("access-token-1"));

        // 文件被写坏时不 panic：当作空表，避免整个面板打不开
        std::fs::write(store_path(&dir), "{ not json").unwrap();
        assert!(load_store(&dir).is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn live_matches_snapshot_by_fingerprint() {
        let auth = valid_auth();
        let same = json!({"tokens": {"access_token": "rotated", "refresh_token": "refresh-token-1", "account_id": "acct-1"}});
        assert!(live_matches_snapshot(&auth, &same), "account_id 相同即视为同一账号");
        let other = json!({"tokens": {"access_token": "a", "refresh_token": "rt-other", "account_id": "acct-2"}});
        assert!(!live_matches_snapshot(&auth, &other));
    }

    fn probe_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!("anyver-codexauth-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
