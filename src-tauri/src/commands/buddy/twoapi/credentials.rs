//! 2API 凭据状态机：内存里的当前账号 + 两类刷新触发 + **写回保护**。
//!
//! 写回保护是这一层存在的核心理由：2API 与 Buddy 面板共用同一个登录态文件
//! （`workbuddy-desktop.info`）。若 token 刷新在网络往返期间遇到 Buddy 切号，
//! 而我们仍用请求发起时的内存副本把文件写回，用户的切换会被**静默回滚**
//! （2026-10-03 在 Python 版上实测复现）。因此：读时记录 mtime+len，写回前比对，
//! 被改过就只更新内存、放弃写文件。

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

/// 覆盖登录态文件所在目录的环境变量（与 Python 版同名，便于迁移）
pub const AUTH_DIR_ENV: &str = "CODEBUDDY_AUTH_DIR";

/// 定位登录态文件：`%LOCALAPPDATA%/CodeBuddyExtension/Data/Public/auth/workbuddy-desktop.info`
/// （macOS: `~/Library/Application Support/…`；Linux: `~/.local/share/…`）。
/// 优先"无时间戳后缀的当前文件"，否则按 mtime 取最新 —— 与 Python 版 `find_auth_file` 一致。
pub fn find_auth_file() -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(dir) = std::env::var_os(AUTH_DIR_ENV) {
        dirs.push(PathBuf::from(dir));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(PathBuf::from(local).join("CodeBuddyExtension/Data/Public/auth"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        dirs.push(
            home.join("Library/Application Support/CodeBuddyExtension/Data/Public/auth"),
        );
        dirs.push(home.join(".local/share/CodeBuddyExtension/Data/Public/auth"));
    }
    for dir in dirs {
        if !dir.is_dir() {
            continue;
        }
        let current = dir.join("workbuddy-desktop.info");
        if current.is_file() {
            return Some(current);
        }
        if let Ok(entries) = std::fs::read_dir(&dir) {
            if let Some(newest) = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().map(|x| x == "info").unwrap_or(false))
                .filter(|p| p.is_file())
                .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
            {
                return Some(newest);
            }
        }
    }
    None
}

/// 解密后的当前账号（可直接用于注入上游鉴权头）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub uid: String,
    /// 昵称（WorkBuddy 里是 `$wbEncrypted` 加密字段，解密后可能为空）
    pub nickname: String,
    /// 邮箱（登录态里未必有）
    pub email: String,
    pub enterprise_id: String,
    pub domain: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at_ms: i64,
}

impl Account {
    /// 面板展示用：昵称 → 邮箱 → uid。
    ///
    /// 不给 uid 是因为那是一串无意义的 UUID：用户看到"当前账号"是要认人，
    /// 不是要核对 ID（真要核对，预检结果里有）。
    pub fn display_name(&self) -> String {
        for candidate in [&self.nickname, &self.email] {
            let trimmed = candidate.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
        self.uid.clone()
    }
}

/// 读文件时留下的快照：写回前用它判断"期间有没有人改过"
#[derive(Debug, Clone)]
pub struct Snapshot {
    mtime: Option<SystemTime>,
    len: u64,
    raw: serde_json::Value,
}

/// 凭据状态机。
///
/// 两条刷新路径都收敛到这里：
/// - [`reload_if_changed`]：文件 mtime/len 变了就重读（用户在 WorkBuddy 客户端里切号）
/// - [`reload_from_buddy`]：Buddy 面板切号成功后主动调用，同进程内无需等 mtime
pub struct Credentials {
    path: PathBuf,
    secret: String,
    state: Mutex<Option<Snapshot>>,
}

impl Credentials {
    pub fn new(path: PathBuf, secret: String) -> Self {
        Self {
            path,
            secret,
            state: Mutex::new(None),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn stamp(&self) -> (Option<SystemTime>, u64) {
        match std::fs::metadata(&self.path) {
            Ok(m) => (m.modified().ok(), m.len()),
            Err(_) => (None, 0),
        }
    }

    fn read_raw(&self) -> Result<serde_json::Value, String> {
        let text = std::fs::read_to_string(&self.path)
            .map_err(|e| format!("读取 {} 失败: {e}", self.path.display()))?;
        serde_json::from_str(&text)
            .map_err(|e| format!("解析 {} 失败: {e}", self.path.display()))
    }

    fn store(&self, raw: serde_json::Value) -> Result<(), String> {
        let (mtime, len) = self.stamp();
        let mut guard = self
            .state
            .lock()
            .map_err(|_| "凭据锁已中毒（此前有 panic 传播）".to_string())?;
        *guard = Some(Snapshot { mtime, len, raw });
        Ok(())
    }

    /// 文件被外部改过就重读（每个请求前调用）
    pub fn reload_if_changed(&self) -> Result<(), String> {
        let (mtime, len) = self.stamp();
        let stale = match &*self.state.lock().map_err(|_| "凭据锁已中毒".to_string())? {
            None => true,
            Some(s) => s.mtime != mtime || s.len != len,
        };
        if stale {
            let raw = self.read_raw()?;
            self.store(raw)?;
        }
        Ok(())
    }

    /// Buddy 面板切号成功后调用：不看 mtime，直接重读
    pub fn reload_from_buddy(&self) -> Result<(), String> {
        let raw = self.read_raw()?;
        self.store(raw)
    }

    /// 拿当前快照（必要时先刷新）
    pub fn snapshot(&self) -> Result<Snapshot, String> {
        self.reload_if_changed()?;
        self.state
            .lock()
            .map_err(|_| "凭据锁已中毒".to_string())?
            .clone()
            .ok_or_else(|| "凭据尚未加载".to_string())
    }

    /// 当前账号（`$wbEncrypted` 字段已解密）
    pub fn current(&self) -> Result<Account, String> {
        let snap = self.snapshot()?;
        parse_account(&snap.raw, &self.secret)
    }

    /// ★ 写回保护：读时记录了 mtime+len，写回前比对。
    ///
    /// 被改过（Buddy 切号）就**只更新内存、放弃写文件**，返回 `false`。
    /// 缺这一步会让 token 刷新把用户的切换静默回滚 —— 实测确认过。
    pub fn write_back_if_unchanged(
        &self,
        snap: &Snapshot,
        new_access_token: &str,
    ) -> Result<bool, String> {
        let (mtime, len) = self.stamp();
        if mtime != snap.mtime || len != snap.len {
            return Ok(false);
        }
        let mut raw = snap.raw.clone();
        super::atrest::set_access_token(&mut raw, new_access_token, &self.secret)?;
        let tmp = self.path.with_extension("info.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&raw).unwrap())
            .map_err(|e| format!("写临时文件 {} 失败: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path)
            .map_err(|e| format!("替换登录态文件 {} 失败: {e}", self.path.display()))?;
        let (m2, l2) = self.stamp();
        let mut guard = self
            .state
            .lock()
            .map_err(|_| "凭据锁已中毒".to_string())?;
        *guard = Some(Snapshot {
            mtime: m2,
            len: l2,
            raw,
        });
        Ok(true)
    }
}

/// 从登录态 JSON 解析出账号（解密敏感字段）
fn parse_account(raw: &serde_json::Value, secret: &str) -> Result<Account, String> {
    let auth = raw.get("auth").cloned().unwrap_or(serde_json::Value::Null);
    let account = raw.get("account").cloned().unwrap_or(serde_json::Value::Null);
    let field = |v: &serde_json::Value, name: &str| -> Result<String, String> {
        match v.get(name) {
            None | Some(serde_json::Value::Null) => Ok(String::new()),
            Some(serde_json::Value::String(s)) => {
                super::atrest::decrypt_field(&serde_json::Value::String(s.clone()), secret)
            }
            Some(other) => super::atrest::decrypt_field(other, secret),
        }
    };
    Ok(Account {
        access_token: field(&auth, "accessToken")?,
        refresh_token: field(&auth, "refreshToken")?,
        nickname: field(&account, "nickname")?,
        email: account
            .get("email")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        domain: {
            let d = auth.get("domain").and_then(|v| v.as_str()).unwrap_or("");
            if d.is_empty() {
                "www.codebuddy.cn".to_string()
            } else {
                d.to_string()
            }
        },
        expires_at_ms: auth.get("expiresAt").and_then(|v| v.as_i64()).unwrap_or(0),
        uid: account.get("uid").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        enterprise_id: account
            .get("enterpriseId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SECRET: &str = "Sik9U5aXhCdwTVEwsEySDOmDoB9r9ntFxHF1fst9LQI=";

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kira-2api-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 写一份登录态；`access_token` 可传明文或加密包装
    fn write_auth(path: &Path, uid: &str, access_token: serde_json::Value) {
        let value = json!({
            "auth": {
                "accessToken": access_token,
                "refreshToken": "refresh-token",
                "expiresAt": 4102444800000i64,
                "domain": "www.codebuddy.cn"
            },
            "account": { "uid": uid, "enterpriseId": "ent-1" }
        });
        std::fs::write(path, serde_json::to_string_pretty(&value).unwrap()).unwrap();
    }

    /// 两次写入之间留一点间隔：某些文件系统时间戳精度不细，mtime 可能相同。
    /// （真机上"同一秒内连切两次"实测能检测到，但测试不必依赖这一点）
    fn gap() {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    #[test]
    fn reload_picks_up_file_change() {
        // 用户在 WorkBuddy 客户端里切号 → 文件变了 → 必须重读
        let dir = temp_dir("reload");
        let path = dir.join("workbuddy-desktop.info");
        write_auth(&path, "uid-A", json!("token-a"));
        let cred = super::Credentials::new(path.clone(), SECRET.to_string());
        assert_eq!(cred.current().unwrap().uid, "uid-A");
        gap();
        write_auth(&path, "uid-B", json!("token-b"));
        cred.reload_if_changed().unwrap();
        assert_eq!(cred.current().unwrap().uid, "uid-B", "文件变了必须重读");
        assert_eq!(cred.current().unwrap().access_token, "token-b");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn buddy_switch_triggers_reload_without_waiting_for_mtime() {
        // Buddy 面板切号后主动 reload：同进程直接改内存，不靠 mtime 轮询
        let dir = temp_dir("buddy");
        let path = dir.join("workbuddy-desktop.info");
        write_auth(&path, "uid-A", json!("token-a"));
        let cred = super::Credentials::new(path.clone(), SECRET.to_string());
        assert_eq!(cred.current().unwrap().uid, "uid-A");
        write_auth(&path, "uid-SWITCHED", json!("token-switched"));
        cred.reload_from_buddy().unwrap();
        assert_eq!(cred.current().unwrap().uid, "uid-SWITCHED");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn writeback_is_skipped_when_file_changed_underneath() {
        // ★ 2026-10-03 实测复现的 lost update：
        // 刷新 token 期间 Buddy 切号，写回必须跳过，否则用户的切换被静默回滚。
        let dir = temp_dir("race");
        let path = dir.join("workbuddy-desktop.info");
        write_auth(&path, "uid-ORIGINAL", json!("token-original"));
        let cred = super::Credentials::new(path.clone(), SECRET.to_string());
        let snapshot = cred.snapshot().unwrap();
        gap();
        write_auth(&path, "uid-SWITCHED-BY-BUDDY", json!("token-switched"));
        let written = cred
            .write_back_if_unchanged(&snapshot, "refreshed-token")
            .unwrap();
        assert!(!written, "文件被改过 → 必须放弃写回");
        let on_disk: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            on_disk["account"]["uid"], "uid-SWITCHED-BY-BUDDY",
            "Buddy 的切换不能被覆盖"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn writeback_proceeds_when_file_untouched() {
        let dir = temp_dir("writeback");
        let path = dir.join("workbuddy-desktop.info");
        write_auth(&path, "uid-A", json!("token-a"));
        let cred = super::Credentials::new(path.clone(), SECRET.to_string());
        let snapshot = cred.snapshot().unwrap();
        assert!(cred
            .write_back_if_unchanged(&snapshot, "new-token")
            .unwrap());
        let on_disk: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(on_disk["auth"]["accessToken"], "new-token");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn writeback_keeps_encrypted_wrapper() {
        // 写回加密字段必须仍是 $wbEncrypted 包装：明文写回会让 WorkBuddy 客户端
        // 报 integrity 错误甚至重置登录态（上游 issue #23）
        let dir = temp_dir("enc");
        let path = dir.join("workbuddy-desktop.info");
        let wrapped = super::super::atrest::encrypt_field("token-plain", SECRET).unwrap();
        write_auth(&path, "uid-A", wrapped);
        let cred = super::Credentials::new(path.clone(), SECRET.to_string());
        assert_eq!(cred.current().unwrap().access_token, "token-plain");
        let snapshot = cred.snapshot().unwrap();
        assert!(cred
            .write_back_if_unchanged(&snapshot, "token-rotated")
            .unwrap());
        let on_disk: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            on_disk["auth"]["accessToken"]["$wbEncrypted"], 1,
            "必须仍是加密包装"
        );
        let back =
            super::super::atrest::decrypt_field(&on_disk["auth"]["accessToken"], SECRET).unwrap();
        assert_eq!(back, "token-rotated", "写回后应能解出新的 token");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn writeback_detects_change_even_when_mtime_is_identical() {
        // 为什么指纹里要带**长度**：部分文件系统的时间戳精度很粗（1~2 秒），
        // 同一时刻内的两次改动 mtime 可能完全相同。只比 mtime 会把
        // "已经被切号"误判成"没变过"，于是把旧账号的 token 写回去 —— 就是那个 lost update。
        // 变异验证实测：去掉 len 比较后，下面这个测试是抓不住的。
        let dir = temp_dir("samemtime");
        let path = dir.join("workbuddy-desktop.info");
        write_auth(&path, "uid-A", json!("token-a"));
        let cred = super::Credentials::new(path.clone(), SECRET.to_string());
        let snapshot = cred.snapshot().unwrap();

        // 造一个 mtime 完全相同、但内容长度不同的文件（模拟 Buddy 在同一刻切号）
        let mut raw: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        raw["account"]["uid"] = json!("uid-SWITCHED-BY-BUDDY");
        raw["account"]["padding"] = json!("x".repeat(64));
        std::fs::write(&path, serde_json::to_string_pretty(&raw).unwrap()).unwrap();
        let stamp = snapshot.mtime.expect("文件应存在");
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(stamp)
            .unwrap();

        let written = cred
            .write_back_if_unchanged(&snapshot, "refreshed-token")
            .unwrap();
        assert!(!written, "mtime 相同但长度不同 → 仍要判定为被改过");
        let on_disk: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(on_disk["account"]["uid"], "uid-SWITCHED-BY-BUDDY");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_yields_readable_error() {
        let cred = super::Credentials::new(
            PathBuf::from("D:/no/such/workbuddy-desktop.info"),
            SECRET.to_string(),
        );
        let err = cred.current().unwrap_err();
        assert!(err.contains("workbuddy-desktop.info"), "错误要带文件名: {err}");
    }

    #[test]
    fn malformed_json_yields_readable_error() {
        let dir = temp_dir("bad");
        let path = dir.join("workbuddy-desktop.info");
        std::fs::write(&path, "{ not json").unwrap();
        let cred = super::Credentials::new(path.clone(), SECRET.to_string());
        let err = cred.current().unwrap_err();
        assert!(err.contains("workbuddy-desktop.info"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn account_fields_fall_back_when_absent() {
        // 缺字段要给可用的默认值，而不是崩在 unwrap 上
        let dir = temp_dir("sparse");
        let path = dir.join("workbuddy-desktop.info");
        std::fs::write(&path, r#"{"auth":{},"account":{}}"#).unwrap();
        let cred = super::Credentials::new(path.clone(), SECRET.to_string());
        let acct = cred.current().unwrap();
        assert_eq!(acct.uid, "");
        assert_eq!(acct.domain, "www.codebuddy.cn", "域名要有默认值");
        assert_eq!(acct.expires_at_ms, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_auth_file_prefers_env_dir() {
        // 显式指定目录必须优先（本机测试与便携场景都靠它）
        let dir = temp_dir("authdir");
        let path = dir.join("workbuddy-desktop.info");
        write_auth(&path, "uid-env", json!("t"));
        std::env::set_var(AUTH_DIR_ENV, &dir);
        let found = super::find_auth_file();
        std::env::remove_var(AUTH_DIR_ENV);
        assert_eq!(found, Some(path));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_auth_file_falls_back_to_timestamped_newest() {
        // 没有"当前文件"时按 mtime 取最新
        let dir = temp_dir("stamped");
        let old = dir.join("workbuddy-desktop-1.info");
        write_auth(&old, "uid-old", json!("t"));
        gap();
        let new = dir.join("workbuddy-desktop-2.info");
        write_auth(&new, "uid-new", json!("t"));
        std::env::set_var(AUTH_DIR_ENV, &dir);
        let found = super::find_auth_file();
        std::env::remove_var(AUTH_DIR_ENV);
        assert_eq!(found, Some(new), "应取 mtime 最新的那个");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
