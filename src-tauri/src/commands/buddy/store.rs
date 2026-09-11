//! Buddy 账号库：JSON 索引 + 每账号详情文件（移植自 cockpit-tools 的 account 存储模式）。
//! 数据存放于 Kira data_dir 下的 `buddy/{platform}_accounts/`。

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::models::{BuddyAccount, BuddyAccountIndex, BuddyPlatform};
use crate::commands::config::get_data_dir;

static STORE_LOCK: Mutex<()> = Mutex::new(());

/// 测试可注入的数据根（默认使用 Kira data_dir）；并发测试用临时目录隔离。
#[cfg(test)]
static TEST_DATA_ROOT: Mutex<Option<PathBuf>> = Mutex::new(None);

#[cfg(test)]
pub(crate) fn set_test_data_root(path: PathBuf) {
    *TEST_DATA_ROOT.lock().unwrap() = Some(path);
}

fn data_root() -> PathBuf {
    #[cfg(test)]
    {
        if let Some(path) = TEST_DATA_ROOT.lock().unwrap().clone() {
            return path;
        }
    }
    get_data_dir()
}

fn buddy_root() -> Result<PathBuf, String> {
    let base = data_root().join("buddy");
    if !base.exists() {
        fs::create_dir_all(&base).map_err(|e| format!("创建 Buddy 数据目录失败: {}", e))?;
    }
    Ok(base)
}

fn accounts_dir(platform: BuddyPlatform) -> Result<PathBuf, String> {
    let dir = buddy_root()?.join(platform.accounts_dir_name());
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("创建账号目录失败: {}", e))?;
    }
    Ok(dir)
}

fn index_path(platform: BuddyPlatform) -> Result<PathBuf, String> {
    Ok(buddy_root()?.join(format!("{}.json", platform.accounts_dir_name())))
}

/// 账号 ID 规范化：仅允许字母/数字/._-，防路径穿越
pub fn normalize_account_id(account_id: &str) -> Result<String, String> {
    let trimmed = account_id.trim();
    if trimmed.is_empty() {
        return Err("账号 ID 不能为空".to_string());
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
        return Err("账号 ID 非法，包含路径字符".to_string());
    }
    let valid = trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.');
    if !valid {
        return Err("账号 ID 非法，仅允许字母/数字/._-".to_string());
    }
    Ok(trimmed.to_string())
}

fn account_file_path(platform: BuddyPlatform, account_id: &str) -> Result<PathBuf, String> {
    let normalized = normalize_account_id(account_id)?;
    Ok(accounts_dir(platform)?.join(format!("{}.json", normalized)))
}

/// 读取单个账号详情
pub fn load_account(platform: BuddyPlatform, account_id: &str) -> Option<BuddyAccount> {
    let path = account_file_path(platform, account_id).ok()?;
    if !path.exists() {
        return None;
    }
    let content = fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

fn load_index(platform: BuddyPlatform) -> BuddyAccountIndex {
    let path = match index_path(platform) {
        Ok(p) => p,
        Err(_) => return BuddyAccountIndex::default(),
    };
    if !path.exists() {
        return BuddyAccountIndex::default();
    }
    fs::read_to_string(&path)
        .ok()
        .and_then(|c| serde_json::from_str::<BuddyAccountIndex>(&c).ok())
        .unwrap_or_default()
}

fn save_index(platform: BuddyPlatform, index: &BuddyAccountIndex) -> Result<(), String> {
    let path = index_path(platform)?;
    let content = serde_json::to_string_pretty(index)
        .map_err(|e| format!("序列化账号索引失败: {}", e))?;
    write_atomic(&path, &content)
}

/// 原子写（临时文件 + 重命名）
pub fn write_atomic(path: &PathBuf, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {}", e))?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, content).map_err(|e| format!("写入临时文件失败: {}", e))?;
    fs::rename(&tmp, path).map_err(|e| format!("原子替换文件失败: {}", e))
}

/// 仅当文件哈希匹配时原子写（防并发覆盖官方客户端更新）。
/// 返回是否写入成功（哈希不匹配 → 不写并返回 false）。
pub fn write_atomic_if_hash_matches(
    path: &PathBuf,
    expected_hash: &[u8; 32],
    content: &str,
) -> Result<bool, String> {
    use sha2::{Digest, Sha256};
    let current = fs::read(path).map_err(|e| format!("读取文件失败: {}", e))?;
    let current_hash: [u8; 32] = Sha256::digest(&current).into();
    if current_hash != *expected_hash {
        return Ok(false);
    }
    write_atomic(path, content)?;
    Ok(true)
}

/// 仅当文件哈希匹配时删除（清理登出标记）。
pub fn remove_file_if_hash_matches(path: &PathBuf, expected_hash: &[u8; 32]) -> Result<bool, String> {
    use sha2::{Digest, Sha256};
    if !path.exists() {
        return Ok(false);
    }
    let current = fs::read(path).map_err(|e| format!("读取文件失败: {}", e))?;
    let current_hash: [u8; 32] = Sha256::digest(&current).into();
    if current_hash != *expected_hash {
        return Ok(false);
    }
    fs::remove_file(path).map_err(|e| format!("删除文件失败: {}", e))?;
    Ok(true)
}

/// 列表账号（按 last_used 倒序）
pub fn list_accounts(platform: BuddyPlatform) -> Vec<BuddyAccount> {
    let _lock = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    list_accounts_locked(platform)
}

/// 前提：调用方已持有 STORE_LOCK（std Mutex 不可重入，持锁期间再调 list_accounts 会死锁）
fn list_accounts_locked(platform: BuddyPlatform) -> Vec<BuddyAccount> {
    let index = load_index(platform);
    let mut accounts: Vec<BuddyAccount> = index
        .accounts
        .iter()
        .filter_map(|summary| load_account(platform, &summary.id))
        .collect();
    accounts.sort_by(|a, b| b.last_used.cmp(&a.last_used));
    accounts
}

/// 新增或更新账号（按 uid/email 去重，更新 token 与 last_used）
pub fn upsert_account(platform: BuddyPlatform, account: BuddyAccount) -> Result<BuddyAccount, String> {
    let _lock = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut index = load_index(platform);

    let now = chrono::Utc::now().timestamp();

    // 去重：按 uid 优先，其次 email
    let existing_id = index
        .accounts
        .iter()
        .filter_map(|summary| load_account(platform, &summary.id))
        .find(|existing| {
            let uid_match = existing
                .uid
                .as_ref()
                .zip(account.uid.as_ref())
                .map(|(a, b)| a.eq_ignore_ascii_case(b))
                .unwrap_or(false);
            if uid_match {
                return true;
            }
            let email_match = existing.email.eq_ignore_ascii_case(&account.email)
                && !account.email.trim().is_empty()
                && account.email != "unknown";
            email_match
        })
        .map(|a| a.id);

    let account_id = existing_id.unwrap_or(account.id.clone());
    let existing = load_account(platform, &account_id);

    let mut merged = account.clone();
    merged.id = account_id.clone();
    merged.created_at = existing.as_ref().map(|a| a.created_at).unwrap_or(now);
    merged.last_used = now;
    if merged.tags.is_none() {
        merged.tags = existing.as_ref().and_then(|a| a.tags.clone());
    }

    let path = account_file_path(platform, &account_id)?;
    let content = serde_json::to_string_pretty(&merged)
        .map_err(|e| format!("序列化账号失败: {}", e))?;
    write_atomic(&path, &content)?;

    // 更新索引
    let summary = merged.summary();
    if let Some(item) = index.accounts.iter_mut().find(|s| s.id == account_id) {
        *item = summary;
    } else {
        index.accounts.push(summary);
    }
    save_index(platform, &index)?;
    Ok(merged)
}

/// 删除账号（若删除的是当前账号，同时清空当前账号状态）
pub fn delete_account(platform: BuddyPlatform, account_id: &str) -> Result<(), String> {
    let was_current = {
        let _lock = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = account_file_path(platform, account_id)?;
        if path.exists() {
            fs::remove_file(&path).map_err(|e| format!("删除账号文件失败: {}", e))?;
        }
        let mut index = load_index(platform);
        index.accounts.retain(|s| s.id != account_id);
        save_index(platform, &index)?;
        get_current_account_id(platform).as_deref() == Some(account_id)
    };
    // 删除的是当前账号 → 清理持久化的当前账号映射（与参考实现一致）。
    // 注意：必须在 STORE_LOCK 释放后再写，避免 Mutex 不可重入导致死锁。
    if was_current {
        let _ = set_current_account_id(platform, None);
    }
    Ok(())
}

/// 批量删除
pub fn delete_accounts(platform: BuddyPlatform, account_ids: &[String]) -> Result<(), String> {
    for id in account_ids {
        delete_account(platform, id)?;
    }
    Ok(())
}

/// 账号跨平台互导（复刻 cockpit-tools `sync_accounts_to_codebuddy_cn` /
/// `sync_accounts_to_workbuddy`）：全量复制，目标平台 upsert 按 uid/email 去重；
/// 签到状态与标签属于单平台数据，不跨平台携带。返回成功数量。
pub fn sync_accounts(from: BuddyPlatform, to: BuddyPlatform) -> Result<usize, String> {
    if from == to {
        return Err("来源与目标平台相同".to_string());
    }
    let accounts = list_accounts(from);
    let mut synced = 0usize;
    for source in accounts {
        let mut target = source.clone();
        target.platform = to.as_str().to_string();
        target.id = format!(
            "{}_{:x}",
            to.id_prefix(),
            md5::compute(crate::commands::buddy::models::account_id_seed(&target, to.id_prefix()).as_bytes())
        );
        target.last_checkin_time = None;
        target.checkin_streak = 0;
        target.checkin_rewards = None;
        target.tags = None;
        match upsert_account(to, target) {
            Ok(_) => {
                synced += 1;
                eprintln!(
                    "[Buddy Sync] {} -> {} 同步成功: email={}",
                    from.as_str(),
                    to.as_str(),
                    source.email
                );
            }
            Err(err) => {
                eprintln!(
                    "[Buddy Sync] {} -> {} 同步失败: email={}, error={}",
                    from.as_str(),
                    to.as_str(),
                    source.email,
                    err
                );
            }
        }
    }
    Ok(synced)
}

/// 设置账号的"过期时间列 → 时间值"映射（列 id → epoch 毫秒）。直接回写账号文件，不改 last_used。
/// 传空 map 即清空；非正时间戳按删除处理。
pub fn set_expiry_times(
    platform: BuddyPlatform,
    account_id: &str,
    times: std::collections::HashMap<String, i64>,
) -> Result<BuddyAccount, String> {
    let _lock = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut account = load_account(platform, account_id)
        .ok_or_else(|| format!("账号不存在: {}", account_id))?;
    account.expiry_times = times.into_iter().filter(|(k, v)| !k.trim().is_empty() && *v > 0).collect();
    let path = account_file_path(platform, account_id)?;
    let content = serde_json::to_string_pretty(&account)
        .map_err(|e| format!("序列化账号失败: {}", e))?;
    write_atomic(&path, &content)?;
    Ok(account)
}

/// 从所有账号清除某列的时间值（删除过期时间列时调用）。
pub fn prune_expiry_column(platform: BuddyPlatform, column_id: &str) {
    let _lock = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    for account in list_accounts_locked(platform) {
        if !account.expiry_times.contains_key(column_id) {
            continue;
        }
        let mut updated = account.clone();
        updated.expiry_times.remove(column_id);
        if let Ok(path) = account_file_path(platform, &account.id) {
            if let Ok(content) = serde_json::to_string_pretty(&updated) {
                let _ = write_atomic(&path, &content);
            }
        }
    }
}

// ─── 当前账号持久化（复刻 provider_current_state） ───
// 记录「最后一次切换到的账号」，即使客户端登录态被改动/不可读，也能稳定标识当前账号。
// 存于 data_dir/buddy/current_accounts.json：{ version, currentAccounts: { platform: id } }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BuddyCurrentState {
    #[serde(default = "default_state_version")]
    pub version: String,
    #[serde(default)]
    pub current_accounts: std::collections::HashMap<String, String>,
}

fn default_state_version() -> String {
    "1.0".to_string()
}

fn current_state_path() -> Result<PathBuf, String> {
    Ok(buddy_root()?.join("current_accounts.json"))
}

fn load_current_state() -> BuddyCurrentState {
    let path = match current_state_path() {
        Ok(p) => p,
        Err(_) => return BuddyCurrentState::default(),
    };
    if !path.exists() {
        return BuddyCurrentState::default();
    }
    fs::read_to_string(&path)
        .ok()
        .and_then(|c| serde_json::from_str::<BuddyCurrentState>(&c).ok())
        .unwrap_or_default()
}

fn save_current_state(state: &BuddyCurrentState) -> Result<(), String> {
    let path = current_state_path()?;
    let content = serde_json::to_string_pretty(state)
        .map_err(|e| format!("序列化当前账号映射失败: {}", e))?;
    write_atomic(&path, &content)
}

/// 读取持久化的当前账号 id
pub fn get_current_account_id(platform: BuddyPlatform) -> Option<String> {
    load_current_state()
        .current_accounts
        .get(platform.as_str())
        .cloned()
        .filter(|id| !id.trim().is_empty())
}

/// 写入/清除当前账号 id
pub fn set_current_account_id(platform: BuddyPlatform, account_id: Option<&str>) -> Result<(), String> {
    let _lock = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut state = load_current_state();
    match account_id.map(str::trim).filter(|v| !v.is_empty()) {
        Some(id) => {
            state
                .current_accounts
                .insert(platform.as_str().to_string(), id.to_string());
        }
        None => {
            state.current_accounts.remove(platform.as_str());
        }
    }
    save_current_state(&state)
}

/// 导出账号为 JSON 数组字符串
pub fn export_accounts(platform: BuddyPlatform, account_ids: &[String]) -> Result<String, String> {
    let accounts: Vec<BuddyAccount> = account_ids
        .iter()
        .filter_map(|id| load_account(platform, id))
        .collect();
    serde_json::to_string_pretty(&accounts).map_err(|e| format!("导出失败: {}", e))
}

/// 导入账号 JSON。
///
/// 兼容性（JSON 本身有效但导入失败的常见原因）：
/// - UTF-8 BOM / UTF-16 编码（先按字节解码）
/// - 顶层为对象 `{"accounts": [...]}` 或单个账号对象，而非纯数组
/// - 缺少可选字段（platform / id / createdAt / lastUsed）时自动补全
pub fn import_accounts(
    platform: BuddyPlatform,
    json_content: &str,
) -> Result<Vec<BuddyAccount>, String> {
    // 1) 编码归一化：按字节识别 BOM/UTF-16，非 UTF-8 回退 GBK
    let content = if json_content.chars().any(|c| c == '\u{0}') {
        // 内容疑似以原始字节形式传入（含 NUL），按字节解码
        crate::commands::file_io::decode_text_bytes(json_content.as_bytes())
    } else {
        json_content.trim_start_matches('\u{feff}').to_string()
    };

    // 2) 结构归一化：数组 | 单个对象 | {"accounts":[...]} 等包装
    let value: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("解析导入 JSON 失败: {}", e))?;
    let items: Vec<serde_json::Value> = match value {
        serde_json::Value::Array(arr) => arr,
        serde_json::Value::Object(obj) => {
            let mut wrapped = None;
            for key in ["accounts", "data", "items", "list", "records"] {
                if let Some(serde_json::Value::Array(arr)) = obj.get(key) {
                    wrapped = Some(arr.clone());
                    break;
                }
            }
            wrapped.unwrap_or_else(|| vec![serde_json::Value::Object(obj)])
        }
        _ => return Err("导入 JSON 顶层既不是数组也不是对象".to_string()),
    };

    if items.is_empty() {
        return Ok(Vec::new());
    }

    // 3) 逐条补全可选字段后解析
    let now = chrono::Utc::now().timestamp();
    let mut imported = Vec::new();
    for (idx, item) in items.into_iter().enumerate() {
        let mut obj = match item {
            serde_json::Value::Object(o) => o,
            _ => {
                return Err(format!(
                    "导入 JSON 第 {} 个条目不是对象（可能混入了非账号数据）",
                    idx + 1
                ))
            }
        };

        // snake_case → camelCase（兼容手写/其它工具导出的键名，如 access_token → accessToken）
        let mut camel: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
        for (k, v) in obj.into_iter() {
            if k.contains('_') {
                let mut out = String::with_capacity(k.len());
                let mut upper_next = false;
                for c in k.chars() {
                    if c == '_' {
                        upper_next = true;
                    } else if upper_next {
                        out.push(c.to_ascii_uppercase());
                        upper_next = false;
                    } else {
                        out.push(c);
                    }
                }
                camel.insert(out, v);
            } else {
                camel.insert(k, v);
            }
        }
        obj = camel;

        // platform：缺省为当前目标平台
        if obj
            .get("platform")
            .map(|v| v.as_str().map(|s| s.trim().is_empty()).unwrap_or(true))
            .unwrap_or(true)
        {
            obj.insert("platform".to_string(), serde_json::Value::String(platform.as_str().to_string()));
        }
        // createdAt / lastUsed：缺省为当前时间
        obj.entry("createdAt").or_insert_with(|| serde_json::json!(now));
        obj.entry("lastUsed").or_insert_with(|| serde_json::json!(now));
        // id：缺失/为空时按 email/uid 生成稳定 id
        let id_empty = obj
            .get("id")
            .map(|v| v.as_str().map(|s| s.trim().is_empty()).unwrap_or(true))
            .unwrap_or(true);
        if id_empty {
            let seed = obj
                .get("email")
                .or_else(|| obj.get("uid"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.trim().to_lowercase())
                .unwrap_or_else(|| format!("import_{}_{}", platform.as_str(), idx + 1));
            let generated = format!("{}_{:x}", platform.as_str(), md5::compute(seed.as_bytes()));
            obj.insert("id".to_string(), serde_json::Value::String(generated));
        }

        let mut account: BuddyAccount = serde_json::from_value(serde_json::Value::Object(obj)).map_err(
            |e| format!("解析导入 JSON 第 {} 个账号失败: {}", idx + 1, e),
        )?;
        account.platform = platform.as_str().to_string();
        let saved = upsert_account(platform, account)?;
        imported.push(saved);
    }
    Ok(imported)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 进程唯一测试根 + 全局串行锁：写入型测试共享全局 TEST_DATA_ROOT，
    /// 并行会互相覆盖，必须串行执行。
    struct TestGuard(std::sync::MutexGuard<'static, ()>);

    fn test_root() -> (std::path::PathBuf, TestGuard) {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        // 恢复可能的毒锁，避免单个测试失败连锁拖垮后续测试
        let guard = TestGuard(LOCK.lock().unwrap_or_else(|e| e.into_inner()));
        static INIT: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
        let dir = INIT.get_or_init(|| {
            let d = std::env::temp_dir().join(format!("kira_buddy_store_test_{}", std::process::id()));
            std::fs::create_dir_all(&d).unwrap();
            set_test_data_root(d.clone());
            d
        });
        (dir.clone(), guard)
    }

    fn test_platform() -> BuddyPlatform {
        BuddyPlatform::Workbuddy
    }

    fn sample_account(id: &str, email: &str) -> BuddyAccount {
        BuddyAccount {
            id: id.to_string(),
            platform: "workbuddy".to_string(),
            email: email.to_string(),
            uid: Some(format!("uid-{}", id)),
            nickname: Some(format!("nick-{}", id)),
            enterprise_id: None,
            enterprise_name: None,
            tags: None,
            access_token: format!("token-{}", id),
            refresh_token: None,
            token_type: Some("Bearer".to_string()),
            expires_at: None,
            domain: None,
            plan_type: None,
            dosage_notify_code: None,
            dosage_notify_zh: None,
            dosage_notify_en: None,
            payment_type: None,
            quota_raw: None,
            usage_raw: None,
            profile_raw: None,
            status: None,
            status_reason: None,
            quota_query_last_error: None,
            quota_query_last_error_at: None,
            usage_updated_at: None,
            last_checkin_time: None,
            checkin_streak: 0,
            checkin_rewards: None,
            auth_raw: None,
            expiry_times: Default::default(),
            created_at: 0,
            last_used: 0,
        }
    }

    #[test]
    fn upsert_and_list_roundtrip() {
        let (_root, _guard) = test_root();
        let account = sample_account("acc1", "a@test.com");
        let saved = upsert_account(test_platform(), account).unwrap();
        assert_eq!(saved.id, "acc1");

        let list = list_accounts(test_platform());
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].email, "a@test.com");

        delete_account(test_platform(), "acc1").unwrap();
        assert!(list_accounts(test_platform()).is_empty());
    }

    #[test]
    fn upsert_dedupes_by_email() {
        let (_root, _guard) = test_root();
        upsert_account(test_platform(), sample_account("acc1", "same@test.com")).unwrap();
        let mut second = sample_account("acc2", "same@test.com");
        second.access_token = "token-acc2".to_string();
        let saved = upsert_account(test_platform(), second).unwrap();
        // 相同 email 去重 → 保留原 id
        assert_eq!(saved.id, "acc1");
        assert_eq!(saved.access_token, "token-acc2");
        assert_eq!(list_accounts(test_platform()).len(), 1);
        delete_account(test_platform(), "acc1").unwrap();
    }

    #[test]
    fn normalize_rejects_path_traversal() {
        let (_root, _guard) = test_root();
        assert!(normalize_account_id("../evil").is_err());
        assert!(normalize_account_id("a/b").is_err());
        assert!(normalize_account_id("").is_err());
        assert_eq!(normalize_account_id("acc-1_x.y").unwrap(), "acc-1_x.y");
    }

    #[test]
    fn export_import_roundtrip() {
        let (_root, _guard) = test_root();
        upsert_account(test_platform(), sample_account("exp1", "e@test.com")).unwrap();
        let json = export_accounts(test_platform(), &["exp1".to_string()]).unwrap();
        delete_account(test_platform(), "exp1").unwrap();
        let imported = import_accounts(test_platform(), &json).unwrap();
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].email, "e@test.com");
        delete_account(test_platform(), "exp1").unwrap();
    }

    #[test]
    fn import_strips_utf8_bom() {
        let (_root, _guard) = test_root();
        // 有效 JSON 前加 UTF-8 BOM（Windows 工具导出常见）
        let json = format!(
            "\u{feff}{}",
            serde_json::to_string(&vec![sample_account("bom1", "bom@test.com")]).unwrap()
        );
        let imported = import_accounts(test_platform(), &json).unwrap();
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].email, "bom@test.com");
        delete_account(test_platform(), "bom1").unwrap();
    }

    #[test]
    fn import_decodes_utf16_bytes() {
        let (_root, _guard) = test_root();
        // 以 UTF-16 LE 字节文件内容传入（模拟其它工具导出的 UTF-16 文件），
        // 走真实链路：read_text_file → decode_text_bytes → import_accounts
        let json_str = serde_json::to_string(&vec![sample_account("u16", "u16@test.com")]).unwrap();
        let mut bytes: Vec<u8> = vec![0xFF, 0xFE]; // UTF-16 LE BOM
        for u in json_str.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        let decoded = crate::commands::file_io::decode_text_bytes(&bytes);
        assert!(!decoded.contains('\u{0}'));
        let imported = import_accounts(test_platform(), &decoded).unwrap();
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].email, "u16@test.com");
        delete_account(test_platform(), "u16").unwrap();
    }

    #[test]
    fn import_accepts_wrapped_object_and_single_account() {
        let (_root, _guard) = test_root();
        // {"accounts": [...]} 包装
        let wrapped = serde_json::json!({
            "accounts": [serde_json::to_value(sample_account("w1", "w@test.com")).unwrap()]
        });
        let imported = import_accounts(test_platform(), &wrapped.to_string()).unwrap();
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].email, "w@test.com");
        delete_account(test_platform(), "w1").unwrap();

        // 单个账号对象（非数组）
        let single = serde_json::to_value(sample_account("s1", "s@test.com")).unwrap();
        let imported = import_accounts(test_platform(), &single.to_string()).unwrap();
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].email, "s@test.com");
        delete_account(test_platform(), "s1").unwrap();
    }

    #[test]
    fn import_defaults_missing_platform_and_snake_case_keys() {
        let (_root, _guard) = test_root();
        // 手写 snake_case 且无 platform 字段
        let json = r#"[
            {
                "id": "snake1",
                "email": "snake@test.com",
                "uid": "uid-snake1",
                "access_token": "tok-1",
                "refresh_token": "ref-1"
            }
        ]"#;
        let imported = import_accounts(test_platform(), json).unwrap();
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].email, "snake@test.com");
        assert_eq!(imported[0].access_token, "tok-1");
        assert_eq!(imported[0].refresh_token.as_deref(), Some("ref-1"));
        // platform 自动补为当前目标平台
        assert_eq!(imported[0].platform, "workbuddy");
        delete_account(test_platform(), "snake1").unwrap();
    }

    #[test]
    fn current_account_persist_and_clear() {
        let (_root, _guard) = test_root();
        let platform = test_platform();
        assert_eq!(get_current_account_id(platform), None);

        set_current_account_id(platform, Some("acc-cur")).unwrap();
        assert_eq!(get_current_account_id(platform).as_deref(), Some("acc-cur"));

        // 写入其它平台互不影响
        set_current_account_id(BuddyPlatform::CodebuddyCn, Some("cn-acc")).unwrap();
        assert_eq!(get_current_account_id(platform).as_deref(), Some("acc-cur"));
        assert_eq!(
            get_current_account_id(BuddyPlatform::CodebuddyCn).as_deref(),
            Some("cn-acc")
        );

        // 清空
        set_current_account_id(platform, None).unwrap();
        assert_eq!(get_current_account_id(platform), None);
        set_current_account_id(BuddyPlatform::CodebuddyCn, None).unwrap();
        assert_eq!(get_current_account_id(BuddyPlatform::CodebuddyCn), None);
    }

    #[test]
    fn delete_current_account_clears_state() {
        let (_root, _guard) = test_root();
        let platform = test_platform();
        upsert_account(platform, sample_account("cur1", "cur@test.com")).unwrap();
        set_current_account_id(platform, Some("cur1")).unwrap();
        assert_eq!(get_current_account_id(platform).as_deref(), Some("cur1"));

        delete_account(platform, "cur1").unwrap();
        assert_eq!(get_current_account_id(platform), None);
    }

    #[test]
    fn import_generates_id_when_missing() {
        let (_root, _guard) = test_root();
        let json = r#"[
            {
                "email": "noid@test.com",
                "accessToken": "tok-2"
            }
        ]"#;
        let imported = import_accounts(test_platform(), json).unwrap();
        assert_eq!(imported.len(), 1);
        assert!(!imported[0].id.is_empty());
        assert_eq!(imported[0].email, "noid@test.com");
        delete_account(test_platform(), &imported[0].id).unwrap();
    }

    #[test]
    fn sync_accounts_between_platforms_resets_checkin_and_dedupes() {
        let (_root, _guard) = test_root();
        let mut wb = sample_account("sync1", "sync@test.com");
        wb.checkin_streak = 7;
        wb.last_checkin_time = Some(123);
        wb.tags = Some(vec!["vip".to_string()]);
        upsert_account(test_platform(), wb).unwrap();

        // WB -> CN：复制成功，签数字段清零，id 按目标平台重新生成
        let n = sync_accounts(BuddyPlatform::Workbuddy, BuddyPlatform::CodebuddyCn).unwrap();
        assert_eq!(n, 1);
        let cn = list_accounts(BuddyPlatform::CodebuddyCn);
        assert_eq!(cn.len(), 1);
        assert_eq!(cn[0].email, "sync@test.com");
        assert_eq!(cn[0].platform, "codebuddy-cn");
        assert!(cn[0].id.starts_with("codebuddy_cn_"));
        assert_eq!(cn[0].checkin_streak, 0);
        assert_eq!(cn[0].last_checkin_time, None);
        assert_eq!(cn[0].access_token, "token-sync1");

        // 幂等：再次同步按 uid/email 去重，不新增
        let n2 = sync_accounts(BuddyPlatform::Workbuddy, BuddyPlatform::CodebuddyCn).unwrap();
        assert_eq!(n2, 1);
        assert_eq!(list_accounts(BuddyPlatform::CodebuddyCn).len(), 1);

        // 反向同步回 WB：uid 相同命中已有账号
        let n3 = sync_accounts(BuddyPlatform::CodebuddyCn, test_platform()).unwrap();
        assert_eq!(n3, 1);
        assert_eq!(list_accounts(test_platform()).len(), 1);

        // 自同步被拒绝
        assert!(sync_accounts(test_platform(), test_platform()).is_err());

        delete_account(test_platform(), "sync1").unwrap();
        for account in list_accounts(BuddyPlatform::CodebuddyCn) {
            delete_account(BuddyPlatform::CodebuddyCn, &account.id).unwrap();
        }
        assert!(list_accounts(BuddyPlatform::CodebuddyCn).is_empty());
    }

    fn times(entries: &[(&str, i64)]) -> std::collections::HashMap<String, i64> {
        entries.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn set_expiry_times_filters_and_roundtrips() {
        let (_root, _guard) = test_root();
        upsert_account(test_platform(), sample_account("tl1", "tl@test.com")).unwrap();

        let updated = set_expiry_times(
            test_platform(),
            "tl1",
            times(&[("c1", 1000), ("c2", 2000), ("c3", -5), ("", 9)]),
        )
        .unwrap();
        // 过滤非正与空 key
        assert_eq!(updated.expiry_times.get("c1"), Some(&1000));
        assert_eq!(updated.expiry_times.get("c2"), Some(&2000));
        assert!(!updated.expiry_times.contains_key("c3"));
        assert!(!updated.expiry_times.contains_key(""));
        // 回读持久化生效
        let reread = load_account(test_platform(), "tl1").unwrap();
        assert_eq!(reread.expiry_times.get("c2"), Some(&2000));

        // 清空
        let cleared = set_expiry_times(test_platform(), "tl1", times(&[])).unwrap();
        assert!(cleared.expiry_times.is_empty());

        // 删除列会清理所有账号该键
        set_expiry_times(test_platform(), "tl1", times(&[("c1", 5), ("c2", 6)])).unwrap();
        prune_expiry_column(test_platform(), "c1");
        let after = load_account(test_platform(), "tl1").unwrap();
        assert!(!after.expiry_times.contains_key("c1"));
        assert_eq!(after.expiry_times.get("c2"), Some(&6));

        // 不存在的账号报错
        assert!(set_expiry_times(test_platform(), "no-such", times(&[("c1", 1)])).is_err());
        delete_account(test_platform(), "tl1").unwrap();
    }
}