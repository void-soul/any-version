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

/// 派生账号 ID：`{platform}_{md5(seed)}`，seed = uid → email(小写) → `{prefix}_user`。
///
/// 规则与本地导入（`workbuddy.rs` / `codebuddy_cn.rs` 的 `build_account_from_local`）一致，
/// 这样同一个账号不管从哪条路进来，id（= 文件名）都一样。真正的去重仍由 uid/email 承担。
pub fn derive_account_id(platform: BuddyPlatform, uid: Option<&str>, email: &str) -> String {
    let seed = uid
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            let trimmed = email.trim().to_lowercase();
            if trimmed.is_empty() || trimmed == "unknown" {
                None
            } else {
                Some(trimmed)
            }
        })
        .unwrap_or_else(|| format!("{}_user", platform.id_prefix()));
    format!(
        "{}_{:x}",
        platform.id_prefix(),
        md5::compute(seed.as_bytes())
    )
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
    let content =
        serde_json::to_string_pretty(index).map_err(|e| format!("序列化账号索引失败: {}", e))?;
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

// ─── 账号视图偏好（主账号 + 排序模式） ───

/// 排序模式：最近使用（默认）
pub const ORDER_MODE_LAST_USED: &str = "lastUsed";
/// 排序模式：剩余额度多的在前
pub const ORDER_MODE_QUOTA: &str = "quota";
/// 排序模式：最早到期的在前
pub const ORDER_MODE_EXPIRY: &str = "expiry";

const ORDER_MODES: [&str; 3] = [ORDER_MODE_LAST_USED, ORDER_MODE_QUOTA, ORDER_MODE_EXPIRY];

/// 校验排序模式（未知值直接拒绝，避免把拼错的模式写进磁盘）。
pub fn validate_order_mode(mode: &str) -> Result<(), String> {
    if ORDER_MODES.contains(&mode) {
        Ok(())
    } else {
        Err(format!("未知的排序模式: {}", mode))
    }
}

/// 账号视图偏好：主账号 + 排序模式（对应参考实现的 `primary-account.json` + `meta.accountOrderMode`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddyAccountView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_account_id: Option<String>,
    #[serde(default = "default_order_mode")]
    pub order_mode: String,
}

fn default_order_mode() -> String {
    ORDER_MODE_LAST_USED.to_string()
}

impl Default for BuddyAccountView {
    fn default() -> Self {
        Self {
            primary_account_id: None,
            order_mode: default_order_mode(),
        }
    }
}

fn view_path(platform: BuddyPlatform) -> Result<PathBuf, String> {
    Ok(buddy_root()?.join(format!("{}_view.json", platform.accounts_dir_name())))
}

/// 读取视图偏好；主账号已不存在时自动清空（对齐参考实现「读取时校验」）。
pub fn load_view(platform: BuddyPlatform) -> BuddyAccountView {
    let Ok(path) = view_path(platform) else {
        return BuddyAccountView::default();
    };
    let mut view = fs::read_to_string(&path)
        .ok()
        .and_then(|c| serde_json::from_str::<BuddyAccountView>(&c).ok())
        .unwrap_or_default();
    if validate_order_mode(&view.order_mode).is_err() {
        view.order_mode = default_order_mode();
    }
    if let Some(id) = view.primary_account_id.clone() {
        if load_account(platform, &id).is_none() {
            view.primary_account_id = None;
        }
    }
    view
}

/// 保存视图偏好（主账号必须存在、排序模式必须合法）。
pub fn save_view_with_mode(
    platform: BuddyPlatform,
    primary_account_id: Option<String>,
    order_mode: &str,
) -> Result<BuddyAccountView, String> {
    validate_order_mode(order_mode)?;
    if let Some(id) = primary_account_id.as_deref() {
        if load_account(platform, id).is_none() {
            return Err(format!("账号不存在: {}", id));
        }
    }
    let view = BuddyAccountView {
        primary_account_id,
        order_mode: order_mode.to_string(),
    };
    save_view(platform, &view)?;
    Ok(view)
}

/// 直接落盘视图偏好（调用方自行保证字段合法）。
pub fn save_view(platform: BuddyPlatform, view: &BuddyAccountView) -> Result<(), String> {
    let _lock = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = view_path(platform)?;
    let content =
        serde_json::to_string_pretty(view).map_err(|e| format!("序列化账号视图失败: {}", e))?;
    write_atomic(&path, &content)
}

/// 新增或更新账号（按 uid/email 去重，更新 token 与 last_used）
pub fn upsert_account(
    platform: BuddyPlatform,
    account: BuddyAccount,
) -> Result<BuddyAccount, String> {
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

    // 新账号（库里没有同 uid/email）必须自己分配 id：
    // 「新增账号（OAuth / 粘贴 token）」两条路传进来的 `id` 都是空的，
    // 以前直接把它当文件名 → normalize_account_id("") → 报「账号 ID 不能为空」，
    // 而界面上根本没有让用户填 ID 的地方，文案把真因埋了。
    let account_id = existing_id.unwrap_or_else(|| {
        let given = account.id.trim().to_string();
        if given.is_empty() {
            derive_account_id(platform, account.uid.as_deref(), &account.email)
        } else {
            given
        }
    });
    let existing = load_account(platform, &account_id);

    let mut merged = account.clone();
    merged.id = account_id.clone();
    merged.created_at = existing.as_ref().map(|a| a.created_at).unwrap_or(now);
    merged.last_used = now;
    if merged.tags.is_none() {
        merged.tags = existing.as_ref().and_then(|a| a.tags.clone());
    }

    let path = account_file_path(platform, &account_id)?;
    let content =
        serde_json::to_string_pretty(&merged).map_err(|e| format!("序列化账号失败: {}", e))?;
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
/// `sync_accounts_to_workbuddy`）：复制指定账号（`account_ids` 为空 = 全部），
/// 目标平台 upsert 按 uid/email 去重；签到状态与标签属于单平台数据，不跨平台携带。返回成功数量。
pub fn sync_accounts(
    from: BuddyPlatform,
    to: BuddyPlatform,
    account_ids: &[String],
) -> Result<usize, String> {
    if from == to {
        return Err("来源与目标平台相同".to_string());
    }
    // 空列表 = 全部账号；非空则只同步勾选的
    let accounts: Vec<BuddyAccount> = if account_ids.is_empty() {
        list_accounts(from)
    } else {
        account_ids
            .iter()
            .filter_map(|id| load_account(from, id))
            .collect()
    };
    let mut synced = 0usize;
    for source in accounts {
        let mut target = source.clone();
        target.platform = to.as_str().to_string();
        target.id = format!(
            "{}_{:x}",
            to.id_prefix(),
            md5::compute(
                crate::commands::buddy::models::account_id_seed(&target, to.id_prefix()).as_bytes()
            )
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
    let mut account =
        load_account(platform, account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    account.expiry_times = times
        .into_iter()
        .filter(|(k, v)| !k.trim().is_empty() && *v > 0)
        .collect();
    let path = account_file_path(platform, account_id)?;
    let content =
        serde_json::to_string_pretty(&account).map_err(|e| format!("序列化账号失败: {}", e))?;
    write_atomic(&path, &content)?;
    Ok(account)
}

/// WorkBuddy 与 CodeBuddy CN 视为同一账号体系（同邮箱 = 同一账号）。
fn twin_platform(platform: BuddyPlatform) -> BuddyPlatform {
    match platform {
        BuddyPlatform::Workbuddy => BuddyPlatform::CodebuddyCn,
        BuddyPlatform::CodebuddyCn => BuddyPlatform::Workbuddy,
    }
}

/// 设置账号的过期时间值，并把相同值同步到另一平台中同邮箱的账号。
///
/// 两平台的账号记录相互独立（id 不同），但同一邮箱视为同一账号——在任一平台
/// 修改「过期时间列」的倒计时，另一平台同邮箱账号自动获得相同值。
/// email 为空或另一平台没有同邮箱账号时不产生同步（各自独立）。
pub fn set_expiry_times_shared(
    platform: BuddyPlatform,
    account_id: &str,
    times: std::collections::HashMap<String, i64>,
) -> Result<BuddyAccount, String> {
    let saved = set_expiry_times(platform, account_id, times.clone())?;
    sync_expiry_times_to_twin(platform, &saved, times);
    Ok(saved)
}

/// 把 saved 账号的过期时间值写到另一平台同邮箱账号（值不同才写，避免无谓的文件写入）。
fn sync_expiry_times_to_twin(
    platform: BuddyPlatform,
    saved: &BuddyAccount,
    times: std::collections::HashMap<String, i64>,
) {
    let email = saved.email.trim();
    if email.is_empty() {
        return;
    }
    let other = twin_platform(platform);
    for twin in list_accounts(other) {
        if twin.email.trim().eq_ignore_ascii_case(email) && twin.expiry_times != saved.expiry_times
        {
            let _ = set_expiry_times(other, &twin.id, times.clone());
        }
    }
}

/// 一次性回填：把两平台同邮箱账号**缺失**的过期时间值互相补齐（不覆盖已有值）。
///
/// 用于启用跨平台共享后，让 CodeBuddy CN 已有的倒计时立即对 WorkBuddy 生效
/// （反之亦然）。幂等：无缺失时不动任何文件。
pub fn backfill_expiry_times() {
    for platform in [BuddyPlatform::Workbuddy, BuddyPlatform::CodebuddyCn] {
        let other = twin_platform(platform);
        let by_email: std::collections::HashMap<String, BuddyAccount> = list_accounts(other)
            .into_iter()
            .filter(|a| !a.email.trim().is_empty())
            .map(|a| (a.email.trim().to_lowercase(), a))
            .collect();
        for account in list_accounts(platform) {
            let email = account.email.trim().to_lowercase();
            if email.is_empty() || account.expiry_times.is_empty() {
                continue;
            }
            let Some(twin) = by_email.get(&email) else {
                continue;
            };
            // 只补 twin 缺失的键；同键两边都有值时保留 twin 现值（不覆盖）
            let missing: std::collections::HashMap<String, i64> = account
                .expiry_times
                .iter()
                .filter(|(k, _)| !twin.expiry_times.contains_key(k.as_str()))
                .map(|(k, v)| (k.clone(), *v))
                .collect();
            if missing.is_empty() {
                continue;
            }
            let mut merged = twin.expiry_times.clone();
            merged.extend(missing);
            let _ = set_expiry_times(other, &twin.id, merged);
        }
    }
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
pub fn set_current_account_id(
    platform: BuddyPlatform,
    account_id: Option<&str>,
) -> Result<(), String> {
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

/// 解析账号 JSON（纯函数，不落库；落库见 `import_accounts`）。
///
/// 兼容性（JSON 本身有效但导入失败的常见原因）：
/// - UTF-8 BOM / UTF-16 编码（先按字节解码）
/// - 顶层为对象 `{"accounts": [...]}` 或单个账号对象，而非纯数组
/// - 缺少可选字段（platform / id / createdAt / lastUsed）时自动补全
pub fn parse_accounts_json(
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
    let value: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("解析导入 JSON 失败: {}", e))?;
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
            obj.insert(
                "platform".to_string(),
                serde_json::Value::String(platform.as_str().to_string()),
            );
        }
        // createdAt / lastUsed：缺省为当前时间
        obj.entry("createdAt")
            .or_insert_with(|| serde_json::json!(now));
        obj.entry("lastUsed")
            .or_insert_with(|| serde_json::json!(now));
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

        let mut account: BuddyAccount = serde_json::from_value(serde_json::Value::Object(obj))
            .map_err(|e| format!("解析导入 JSON 第 {} 个账号失败: {}", idx + 1, e))?;
        account.platform = platform.as_str().to_string();
        imported.push(account);
    }
    Ok(imported)
}

/// 导入账号 JSON 并落库（解析复用 `parse_accounts_json`，按 uid/email 去重）。
pub fn import_accounts(
    platform: BuddyPlatform,
    json_content: &str,
) -> Result<Vec<BuddyAccount>, String> {
    let parsed = parse_accounts_json(platform, json_content)?;
    let mut saved = Vec::with_capacity(parsed.len());
    for account in parsed {
        saved.push(upsert_account(platform, account)?);
    }
    Ok(saved)
}

#[cfg(test)]
mod tests;
