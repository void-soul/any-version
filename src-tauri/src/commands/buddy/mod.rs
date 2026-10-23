//! Buddy 模块：管理本地 WorkBuddy / CodeBuddy CN 账号。
//!
//! 复刻自 cockpit-tools 的 workbuddy / codebuddy-cn 账号管理：
//! - 账号库：Kira data_dir/buddy/ 下 JSON 索引 + 每账号文件
//! - 本地导入：读取本机 IDE 的登录态（auth 文件 / state.vscdb secret），再经官方 API 补全资料与用量
//! - 切换：把目标账号写回本机 IDE 的登录态（auth 文件 / state.vscdb 加密注入）
//! - 新增：OAuth 授权登录（展示验证链接轮询）或直接粘贴 token
//! - 用量：查询官方 user-resource / dosage / payment，展示在账号卡片
//! - 签到：手动签到 + 后台自动签到调度器（每天随机时间窗口）
//! - 签到/派旅行的每日归档（`daily_history`）：计划、实际、归来、派出次数，供前端日历使用
//! - 会话：列出本机会话（WorkBuddy workbuddy.db / CodeBuddy CN codebuddy-sessions.vscdb）

mod action_log;
mod api;
mod auto_checkin;
mod auto_travel;
mod client_process;
mod codebuddy_cn;
mod crypto;
mod daily_history;
mod expiry;
pub(crate) mod growth;
mod models;
mod scrypt_kdf;
mod session_sync;
mod session_transfer;
mod sessions;
mod store;
pub(crate) mod twoapi;
mod third_party_import;
mod workbuddy;

use models::{BuddyAccount, BuddyPlatform};
use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

// ─── 切号互斥闸 ───
//
// 抄 WorkDaddy `codebuddy-native.js::replaceNativeSession`（2026-09-29 `4ed7410`）的三个状态锁：
// `refreshInFlight` / `loginInProgress` / `logoutInProgress` 都在**第一个 await 之前同步预留**，
// 入口处若任一事务在进行就**直接拒绝**（不排队、不等待），并在 finally 里统一释放。
//
// 我们的对应：同一平台的切号事务互斥。连点两次、或切号与切号重叠时，第二次立刻拿到可操作错误，
// 而不是两个事务同时写客户端登录态（那会写坏 auth 文件 / secret）。
static SWITCH_IN_FLIGHT: LazyLock<Mutex<HashSet<&'static str>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// 进入即占用；离开作用域（含任何错误返回）自动释放。
pub(crate) struct SwitchGuard {
    platform: &'static str,
    display: String,
}

impl SwitchGuard {
    pub(crate) fn acquire(platform: BuddyPlatform) -> Result<Self, String> {
        let key = platform.as_str();
        let display = client_process::app_display_name(platform).to_string();
        let mut set = SWITCH_IN_FLIGHT
            .lock()
            .map_err(|_| format!("切换 {} 账号失败：内部锁已损坏", display))?;
        if !set.insert(key) {
            return Err(format!("{} 正在切换账号中，请等当前切换结束后再试", display));
        }
        Ok(Self {
            platform: key,
            display,
        })
    }
}

impl Drop for SwitchGuard {
    fn drop(&mut self) {
        if let Ok(mut set) = SWITCH_IN_FLIGHT.lock() {
            set.remove(self.platform);
        }
    }
}

/// 切号落地的**纯逻辑**判定：把延时复查的结果与目标账号比对。
///
/// 为什么要延时复查（2026-10-17 真机故障）：客户端正处在**退出/启动窗口**时切号，
/// 它会用自己缓存的会话把我们写进去的登录态覆盖回去，而我们仍然返回"切换成功" ——
/// 用户只看到"切了号，客户端还是原来那个"。WorkDaddy 靠 `refreshInFlight` 锁住自己的刷新，
/// 但它**锁不住客户端进程**，所以它没法做这件事；我们是直接改客户端文件，只能靠事后确认。
///
/// `probe` 三态：
/// - `Ok(Some(token))` = 登录态里读到的就是某个 token（与期望相同 → 切号生效）
/// - `Ok(None)`       = 登录态已不存在（客户端登出 / 被删）
/// - `Err(e)`         = **探测本身失败**（取不到官方密钥、库被占用…）→ 不判定切号失败，
///   否则会把"环境问题"报成"切号没生效"，误导用户去排查错方向。
///
/// `logged_out` = 客户端的「已登出」标记仍然存在。**这一条必须先判**：
/// 标记在时 WorkBuddy 根本不读登录文件（WorkDaddy `retireLogoutMarker` 的注释原话），
/// 此时 token 读起来完全正确，但用户打开客户端看到的是登录页 —— 只看 token 会把
/// 「切了号却要我重新登录」这种最典型的失败判成成功。
pub(crate) fn diagnose_switch_landing(
    probe: Result<Option<String>, String>,
    expected: &str,
    display: &str,
    logged_out: bool,
) -> Option<String> {
    if logged_out {
        return Some(format!(
            "{display} 仍带着「已登出」标记，客户端会忽略刚写回的登录态并提示登录，\
             请完全退出客户端（含托盘）后重试"
        ));
    }
    match probe {
        Ok(Some(token)) if token == expected => None,
        Ok(Some(_)) => Some(format!(
            "{display} 的登录态已被改回另一个账号（切号期间客户端仍在运行并回写），\
             请完全退出客户端（含托盘）后重试"
        )),
        Ok(None) => Some(format!(
            "{display} 的登录态已不存在（客户端在切号期间登出或覆盖了登录信息），\
             请完全退出客户端（含托盘）后重试"
        )),
        Err(e) => {
            eprintln!("[buddy] 切号落地复查无法执行（不影响切号结果）: {e}");
            None
        }
    }
}

/// 切号写入后等客户端落定，再确认登录态确实是我们写的。
///
/// 只在 `spawn_blocking` 的切号线程里调用，所以这里阻塞式 sleep 是安全的。
fn confirm_switch_landed(platform: BuddyPlatform, expected_token: &str) -> Option<String> {
    std::thread::sleep(std::time::Duration::from_millis(400));
    let display = client_process::app_display_name(platform);
    let probe = match platform {
        BuddyPlatform::Workbuddy => workbuddy::read_session_access_token(),
        BuddyPlatform::CodebuddyCn => codebuddy_cn::read_session_access_token(),
    };
    // CN 用 state.vscdb，没有 `.logged-out` 标记这个概念
    let logged_out = match platform {
        BuddyPlatform::Workbuddy => workbuddy::logout_marker_present(),
        BuddyPlatform::CodebuddyCn => false,
    };
    diagnose_switch_landing(probe, expected_token, display, logged_out)
}

// ─── 切换进度事件（前端展示：关闭 → 合并 → 写入 → 启动 → 完成） ───

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddySwitchProgress {
    pub platform: String,
    pub account_id: String,
    /// checking | merging | writing | done（切换后不再自动启动客户端，故无 launching 阶段）
    pub stage: String,
    /// 合并阶段：已扫描工作区数
    pub scanned_workspaces: usize,
    pub message: Option<String>,
    /// 合并结束时一次性带上「只同步有变化的会话」台账（其余阶段为 None，不占载荷）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync: Option<session_sync::SessionSyncSummary>,
}

pub(crate) fn emit_switch_progress(
    app: Option<&tauri::AppHandle>,
    platform: BuddyPlatform,
    account_id: &str,
    stage: &str,
    scanned_workspaces: usize,
    message: Option<String>,
) {
    emit_switch_progress_inner(
        app,
        platform,
        account_id,
        stage,
        scanned_workspaces,
        message,
        None,
    );
}

/// 合并结束时上报逐会话明细（成功 / 跳过 / 冲突 / 失败）。
pub(crate) fn emit_switch_sync_progress(
    app: Option<&tauri::AppHandle>,
    platform: BuddyPlatform,
    account_id: &str,
    sync: &session_sync::SessionSyncSummary,
) {
    emit_switch_progress_inner(
        app,
        platform,
        account_id,
        "merging",
        0,
        None,
        Some(sync.clone()),
    );
}

fn emit_switch_progress_inner(
    app: Option<&tauri::AppHandle>,
    platform: BuddyPlatform,
    account_id: &str,
    stage: &str,
    scanned_workspaces: usize,
    message: Option<String>,
    sync: Option<session_sync::SessionSyncSummary>,
) {
    let Some(handle) = app else { return };
    use tauri::Emitter;
    let _ = handle.emit(
        "buddy-switch-progress",
        BuddySwitchProgress {
            platform: platform.as_str().to_string(),
            account_id: account_id.to_string(),
            stage: stage.to_string(),
            scanned_workspaces,
            message,
            sync,
        },
    );
}

// ─── 冲突裁决进度事件 ───

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddyConflictProgress {
    pub platform: String,
    /// resolving | done
    pub stage: String,
    /// 本次要处理的冲突条数
    pub total: usize,
    /// 已处理条数
    pub done: usize,
    /// 当前处理的会话 id
    pub current_id: Option<String>,
    /// 单条结果文案（已合并 / 已覆盖 / 已保留 / 已忽略）
    pub message: Option<String>,
}

/// 冲突裁决的进度上报上下文。
#[derive(Clone, Copy)]
pub(crate) struct ConflictProgressCtx<'a> {
    pub app: Option<&'a tauri::AppHandle>,
    pub platform: BuddyPlatform,
    pub total: usize,
}

pub(crate) fn emit_conflict_progress(
    app: Option<&tauri::AppHandle>,
    platform: BuddyPlatform,
    stage: &str,
    total: usize,
    done: usize,
    current_id: Option<String>,
    message: Option<String>,
) {
    let Some(handle) = app else { return };
    use tauri::Emitter;
    let _ = handle.emit(
        "buddy-conflict-progress",
        BuddyConflictProgress {
            platform: platform.as_str().to_string(),
            stage: stage.to_string(),
            total,
            done,
            current_id,
            message,
        },
    );
}

fn platform_from_str(s: &str) -> Result<BuddyPlatform, String> {
    BuddyPlatform::from_str(s).ok_or_else(|| format!("未知平台: {}", s))
}

// ─── 账号库命令 ───

/// 账号列表项 = 账号 + **认证域信息**。
///
/// 官方把国内版认证域从 `www.codebuddy.cn` 迁到了 `www.workbuddy.cn`，两个域签发的 token
/// 在**客户端侧不通用**（WorkBuddy 5.8.x 拿老域 token 鉴权直接 401，"服务器无法确认你的账号身份"），
/// 但在**面板侧是通用的**（聚合网关通吃，签到/用量/成长照常跑）。
/// 所以列表要把域摆出来：匹配 → 绿色可选；不匹配 → 红色不可选，省得白切一趟再撞 401。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountListItem {
    #[serde(flatten)]
    account: BuddyAccount,
    /// 令牌签发域（JWT `iss` 的 origin）；老 token / 非 JWT 解不出时为 None
    auth_origin: Option<String>,
    /// 该平台客户端鉴权时使用的域
    platform_auth_origin: String,
    /// 是否与客户端域一致；`None` = 解不出签发域（**不拦**，交给用户判断）
    auth_origin_matches: Option<bool>,
}

#[tauri::command]
pub fn buddy_list_accounts(platform: String) -> Result<Vec<AccountListItem>, String> {
    let platform = platform_from_str(&platform)?;
    let client_origin = api::client_auth_origin(platform);
    Ok(store::list_accounts(platform)
        .into_iter()
        .map(|account| {
            let auth_origin = api::token_issuer_origin(&account.access_token);
            let auth_origin_matches = auth_origin.as_deref().map(|o| o == client_origin);
            AccountListItem {
                account,
                auth_origin,
                platform_auth_origin: client_origin.to_string(),
                auth_origin_matches,
            }
        })
        .collect())
}

#[tauri::command]
pub fn buddy_delete_account(platform: String, account_id: String) -> Result<(), String> {
    let platform = platform_from_str(&platform)?;
    store::delete_account(platform, &account_id)
}

// ─── 过期时间列（两平台共享：全局列 schema，同邮箱账号的时间值互通） ───

// ─── 账号视图偏好（主账号 + 排序模式） ───

#[tauri::command]
pub fn buddy_get_account_view(platform: String) -> Result<store::BuddyAccountView, String> {
    let platform = platform_from_str(&platform)?;
    Ok(store::load_view(platform))
}

/// 设置主账号（`None` = 取消主账号）；排序模式沿用当前值。
#[tauri::command]
pub fn buddy_set_primary_account(
    platform: String,
    account_id: Option<String>,
) -> Result<store::BuddyAccountView, String> {
    let platform = platform_from_str(&platform)?;
    let view = store::load_view(platform);
    store::save_view_with_mode(platform, account_id, &view.order_mode)
}

/// 设置排序模式（lastUsed / quota / expiry）；主账号沿用当前值。
#[tauri::command]
pub fn buddy_set_account_order_mode(
    platform: String,
    order_mode: String,
) -> Result<store::BuddyAccountView, String> {
    let platform = platform_from_str(&platform)?;
    let view = store::load_view(platform);
    store::save_view_with_mode(platform, view.primary_account_id, &order_mode)
}

#[tauri::command]
pub fn buddy_get_expiry_columns() -> Result<Vec<expiry::ExpiryColumn>, String> {
    Ok(expiry::load_columns())
}

#[tauri::command]
pub fn buddy_set_expiry_columns(
    columns: Vec<expiry::ExpiryColumn>,
) -> Result<Vec<expiry::ExpiryColumn>, String> {
    let old_ids: Vec<String> = expiry::load_columns().into_iter().map(|c| c.id).collect();
    let saved = expiry::save_columns(columns)?;
    let new_ids: std::collections::HashSet<String> = saved.iter().map(|c| c.id.clone()).collect();
    // 被删除的列：清理两平台账号里遗留的时间值
    for id in &old_ids {
        if !new_ids.contains(id) {
            store::prune_expiry_column(BuddyPlatform::CodebuddyCn, id);
            store::prune_expiry_column(BuddyPlatform::Workbuddy, id);
        }
    }
    Ok(saved)
}

/// 保存账号的过期时间值；同邮箱账号在另一平台的值自动同步（两平台共享同一账号的倒计时）。
#[tauri::command]
pub fn buddy_set_expiry_times(
    platform: String,
    account_id: String,
    times: std::collections::HashMap<String, i64>,
) -> Result<BuddyAccount, String> {
    let platform = platform_from_str(&platform)?;
    store::set_expiry_times_shared(platform, &account_id, times)
}

// ─── 自动派 Buddy 旅行（WorkBuddy 专属活动） ───

#[tauri::command]
pub fn buddy_auto_travel_get_config() -> Result<auto_travel::BuddyAutoTravelConfig, String> {
    auto_travel::get_config_checked()
}

#[tauri::command]
pub fn buddy_auto_travel_save_config(
    config: auto_travel::BuddyAutoTravelConfig,
) -> Result<(), String> {
    auto_travel::save_config(&config)
}

/// 手动立即执行一轮旅行状态机（派出 / 领取 / 跳过）。
/// 今日派旅行任务列表（账号 + 计划/实际/归来时间 + 派出次数）。
#[tauri::command]
pub fn buddy_auto_travel_tasks() -> Result<auto_travel::BuddyTravelTasksView, String> {
    auto_travel::build_travel_tasks_view()
}

/// 每日归档查询（日历用）：`[from, to]`（含端点，`YYYY-MM-DD`）内每天每账号的记录。
#[tauri::command]
pub fn buddy_get_daily_records(from: String, to: String) -> Result<daily_history::DailyMap, String> {
    daily_history::get_range(&from, &to)
}

#[tauri::command]
pub async fn buddy_auto_travel_run(app: tauri::AppHandle, force: Option<bool>) -> Result<String, String> {
    auto_travel::run_auto_travel_cycle_if_needed(&app, force.unwrap_or(false)).await
}

/// 启动后台自动旅行调度（仅 WorkBuddy）。
pub fn start_auto_travel_scheduler(app: tauri::AppHandle) {
    auto_travel::start_auto_travel_scheduler(app);
}

/// 启动时调用：把两平台同邮箱账号缺失的倒计时互相补齐（幂等，无缺失时不写文件）。
pub fn backfill_expiry_times() {
    store::backfill_expiry_times();
}

#[tauri::command]
pub fn buddy_delete_accounts(platform: String, account_ids: Vec<String>) -> Result<(), String> {
    let platform = platform_from_str(&platform)?;
    store::delete_accounts(platform, &account_ids)
}

#[tauri::command]
pub fn buddy_export_accounts(platform: String, account_ids: Vec<String>) -> Result<String, String> {
    let platform = platform_from_str(&platform)?;
    store::export_accounts(platform, &account_ids)
}

#[tauri::command]
pub fn buddy_import_accounts(
    platform: String,
    json_content: String,
) -> Result<Vec<BuddyAccount>, String> {
    let platform = platform_from_str(&platform)?;
    store::import_accounts(platform, &json_content)
}

// ─── 第三方工具导出文件导入（WorkDaddy 加密包 / cockpit-tools 明文 JSON） ───

/// 从第三方工具的**导出文件**导入账号（不读取对方的数据目录）。
///
/// - `workdaddy`：WorkDaddy「账号导出」生成的加密 JSON（信封 v3 = gzip + AES-256-GCM，
///   密钥由 scrypt 从导出密码派生），载荷 `account/{uid,info}` 逐个还原成 WorkBuddy 账号。
/// - `cockpit-tools`：cockpit-tools 中选中账号「导出」生成的明文 JSON 数组。
///
/// 与 `buddy_import_accounts` 同语义：只落库，不调用官方 API 补全。
#[tauri::command]
pub fn buddy_import_third_party(
    platform: String,
    tool: String,
    json_content: String,
    password: Option<String>,
) -> Result<Vec<BuddyAccount>, String> {
    let platform = platform_from_str(&platform)?;
    let tool = tool.trim().to_ascii_lowercase();
    let parsed = match tool.as_str() {
        third_party_import::TOOL_WORKDADDY => {
            // SKILL.md §0.1 硬规则：WorkDaddy 只有 WorkBuddy（cn/ai）两个 profile
            if platform != BuddyPlatform::Workbuddy {
                return Err(
                    "WorkDaddy 导出文件只包含 WorkBuddy 账号，请在 WorkBuddy 平台下导入".to_string(),
                );
            }
            third_party_import::parse_workdaddy_export(&json_content, password.as_deref())?
        }
        third_party_import::TOOL_COCKPIT => {
            third_party_import::parse_cockpit_tools_export(platform, &json_content)?
        }
        other => return Err(format!("不支持的第三方导入来源: {}", other)),
    };

    let mut saved = Vec::with_capacity(parsed.len());
    for account in parsed {
        saved.push(store::upsert_account(platform, account)?);
    }
    Ok(saved)
}

// ─── 账号跨平台互导（复刻 sync_workbuddy_to_codebuddy_cn / sync_codebuddy_cn_to_workbuddy） ───

#[tauri::command]
pub fn buddy_sync_accounts(
    from_platform: String,
    to_platform: String,
    account_ids: Vec<String>,
) -> Result<i32, String> {
    let from = platform_from_str(&from_platform)?;
    let to = platform_from_str(&to_platform)?;
    let synced = store::sync_accounts(from, to, &account_ids)?;
    Ok(synced as i32)
}

// ─── 本地导入（导入后经官方 API 补全资料/用量，并清理同 token 占位账号） ───

#[tauri::command]
pub async fn buddy_import_from_local(platform: String) -> Result<Option<BuddyAccount>, String> {
    let platform = platform_from_str(&platform)?;
    let local = match platform {
        BuddyPlatform::Workbuddy => {
            workbuddy::import_payload_from_local(platform)?
        }
        BuddyPlatform::CodebuddyCn => codebuddy_cn::import_payload_from_local()?,
    };
    let Some(mut account) = local else {
        return Ok(None);
    };

    // 用官方 API 补全账号资料与用量
    match api::build_payload_from_token(platform, &account.access_token).await {
        Ok(enriched) => {
            if enriched.uid.is_some() {
                account.uid = enriched.uid;
            }
            if enriched.nickname.is_some() {
                account.nickname = enriched.nickname;
            }
            if enriched.refresh_token.is_some() {
                account.refresh_token = enriched.refresh_token;
            }
            if enriched.domain.is_some() {
                account.domain = enriched.domain;
            }
            if enriched.expires_at.is_some() {
                account.expires_at = enriched.expires_at;
            }
            if enriched.profile_raw.is_some() {
                account.profile_raw = enriched.profile_raw;
            }
            if enriched.quota_raw.is_some() {
                account.quota_raw = enriched.quota_raw;
            }
            if enriched.usage_raw.is_some() {
                account.usage_raw = enriched.usage_raw;
            }
            if enriched.dosage_notify_code.is_some() {
                account.dosage_notify_code = enriched.dosage_notify_code;
            }
            if enriched.dosage_notify_zh.is_some() {
                account.dosage_notify_zh = enriched.dosage_notify_zh;
            }
            if enriched.dosage_notify_en.is_some() {
                account.dosage_notify_en = enriched.dosage_notify_en;
            }
            if enriched.payment_type.is_some() {
                account.payment_type = enriched.payment_type;
            }
            if account.email.trim().is_empty() || account.email == "unknown" {
                account.email = enriched.email;
            }
        }
        Err(err) => {
            eprintln!("[Buddy Import Local] 拉取账号资料失败，将保留本地导入结果: {}", err);
        }
    }

    let mut saved = store::upsert_account(platform, account)?;

    // 清理同 token 的历史占位账号（email unknown / 无 uid）
    for existing in store::list_accounts(platform) {
        if existing.id == saved.id {
            continue;
        }
        if existing.access_token != saved.access_token {
            continue;
        }
        let is_placeholder = existing.email.trim().eq_ignore_ascii_case("unknown")
            || existing.email.trim().is_empty()
            || existing
                .uid
                .as_deref()
                .map(|s| s.trim().is_empty())
                .unwrap_or(true);
        if is_placeholder {
            if let Err(err) = store::delete_account(platform, &existing.id) {
                eprintln!("[Buddy Import Local] 清理占位账号失败: id={}, error={}", existing.id, err);
            }
        }
    }

    // 导入后刷新一次 token（失败则保留原账号）
    match api::refresh_payload_for_account(platform, &saved).await {
        Ok((refreshed, _quota_err)) => {
            saved = store::upsert_account(platform, refreshed)?;
        }
        Err(e) => {
            eprintln!("[Buddy Import Local] 登录后刷新失败，保留原账号信息: {}", e);
        }
    }
    Ok(Some(saved))
}

// ─── 当前账号 ───

#[tauri::command]
pub fn buddy_get_current_account_id(platform: String) -> Result<Option<String>, String> {
    let platform = platform_from_str(&platform)?;
    let accounts = store::list_accounts(platform);

    // 优先用持久化的当前账号映射（切换时写入，稳定可靠）
    if let Some(stored) = store::get_current_account_id(platform) {
        if accounts.iter().any(|a| a.id == stored) {
            return Ok(Some(stored));
        }
        // 持久化 id 已不存在（账号被删除）→ 清理
        let _ = store::set_current_account_id(platform, None);
    }

    // 回退：读取本机客户端当前登录账号
    let current = match platform {
        BuddyPlatform::Workbuddy => {
            workbuddy::resolve_current_account_id(platform, &accounts)
        }
        BuddyPlatform::CodebuddyCn => codebuddy_cn::resolve_current_account_id(&accounts),
    };
    Ok(current)
}

// ─── 切换账号 ───

#[tauri::command]
pub async fn buddy_switch_account(
    app: tauri::AppHandle,
    platform: String,
    account_id: String,
) -> Result<(String, Option<session_transfer::SessionTransferReport>), String> {
    let platform = platform_from_str(&platform)?;
    let account = store::load_account(platform, &account_id)
        .ok_or_else(|| format!("账号不存在: {}", account_id))?;

    // 切换时序：检查客户端已退出 → 合并来源会话 → 写入登录态 → 重新启动。
    // 各阶段通过 buddy-switch-progress 事件上报前端；合并统计随报告一并返回（前端渲染为文本）。
    tauri::async_runtime::spawn_blocking(move || {
        // 0) 占用切号互斥闸（连点两次 → 第二次立刻报错，不并行写登录态）
        let _guard = SwitchGuard::acquire(platform)?;
        // 1) **只检查**客户端是否已退出：约定由用户自己关闭，程序不代关（更不会强杀），
        //    进程占用 db/会话目录，强关可能丢未落盘的会话。仍在运行 → 中止并提示手动退出。
        emit_switch_progress(Some(&app), platform, &account_id, "checking", 0, None);
        client_process::ensure_not_running(platform)?;

        // 2) 切换之前合并来源账号本地会话（合并进度在 session_transfer 内部上报）
        let transfer_report = session_transfer::transfer_on_switch(platform, &account, Some(&app))?;
        if let Some(report) = &transfer_report {
            eprintln!(
                "[Buddy Switch] 会话合并报告: 新增会话={}, 替换会话={}, 重映射记录={}, 扫描工作区={}, 会话总数={}, 已同步={}, 跳过={}, 冲突={}, 部分失败={}, 失败={}, 全部无变化={}",
                report.added_conversations,
                report.replaced_conversations,
                report.updated_session_rows,
                report.scanned_workspaces,
                report.sync.total,
                report.sync.copied,
                report.sync.skipped,
                report.sync.conflict,
                report.sync.partial,
                report.sync.failed,
                report.sync.unchanged
            );
        }

        // 3) 写入目标账号登录态
        emit_switch_progress(Some(&app), platform, &account_id, "writing", 0, None);
        let message = match platform {
            BuddyPlatform::Workbuddy => {
                workbuddy::switch_account(platform, &account_id)?
            }
            BuddyPlatform::CodebuddyCn => codebuddy_cn::switch_account(&account_id)?,
        };
        // 3.5) **延时复查**：等客户端落定后再确认登录态确实是我们写的。
        //     客户端处在退出/启动窗口时会用缓存会话回写覆盖（真机 2026-10-17 踩过），
        //     那种情况必须报错而不是返回成功 —— 静默成功比失败更糟。
        if let Some(problem) = confirm_switch_landed(platform, &account.access_token) {
            eprintln!("[Buddy Switch] 切号未生效：{problem}");
            return Err(problem);
        }

        // 复查通过才算切号成功，此时才持久化当前账号（复刻 provider_current_state）。
        // 先写的话，复查失败返回 Err 时「当前账号」已经是新账号了 ——
        // 刷新后面板会显示一个并未生效的当前账号。
        let _ = store::set_current_account_id(platform, Some(&account_id));

        // 4) 切换后**不自动启动客户端**（两个平台一致）。
        //    切换时客户端本来就必须是「已退出」状态（见上面 ensure_not_running），
        //    自动拉起会打断用户手头的事：他可能正准备自己开窗口、或者正在处理别的窗口。
        //    两个平台都需要手动启动，统一提示即可。
        // 认证域体检：官方迁过域（codebuddy.cn → workbuddy.cn），两个域的 token 不通用。
        // 不阻断切换（老域账号的签到/用量仍能跑），但必须说清楚，否则用户看到的是
        // "切成功了、客户端却提示登录失效"这种哑故障。
        let mut message = format!(
            "{}，请手动启动 {}",
            message,
            client_process::app_display_name(platform)
        );
        if let Some(warning) = api::auth_origin_warning(platform, &account.access_token) {
            eprintln!("[Buddy Switch] 认证域不匹配：{warning}");
            message = format!("{}\n注意：{}", message, warning);
        }
        emit_switch_progress(Some(&app), platform, &account_id, "done", 0, None);
        Ok((message, transfer_report))
    })
    .await
    .map_err(|e| format!("切换账号后台任务失败: {}", e))?
}

// ─── Token 刷新 / 用量 ───

#[tauri::command]
pub async fn buddy_refresh_token(
    platform: String,
    account_id: String,
) -> Result<BuddyAccount, String> {
    let platform = platform_from_str(&platform)?;
    let account = store::load_account(platform, &account_id)
        .ok_or_else(|| format!("账号不存在: {}", account_id))?;
    let (refreshed, quota_err) = api::refresh_payload_for_account(platform, &account).await?;
    let mut saved = store::upsert_account(platform, refreshed)?;
    if let Some(err) = quota_err {
        saved.quota_query_last_error = Some(err.clone());
        saved.quota_query_last_error_at = Some(chrono::Utc::now().timestamp_millis());
        saved = store::upsert_account(platform, saved)?;
    }
    Ok(saved)
}

#[tauri::command]
pub async fn buddy_refresh_tokens(platform: String, account_ids: Vec<String>) -> Result<i32, String> {
    let platform = platform_from_str(&platform)?;
    // 空列表 = 全部账号（兼容旧「刷新全部」语义）；非空则只刷新勾选的
    let accounts: Vec<BuddyAccount> = if account_ids.is_empty() {
        store::list_accounts(platform)
    } else {
        account_ids.iter().filter_map(|id| store::load_account(platform, id)).collect()
    };
    let mut success = 0;
    for account in accounts {
        match api::refresh_payload_for_account(platform, &account).await {
            Ok((refreshed, quota_err)) => {
                let mut saved = store::upsert_account(platform, refreshed)?;
                // 与 buddy_query_usage / buddy_query_all_usage 一致：失败原因必须落盘。
                // 这条路径就是面板的「刷新选中/刷新全部」，以前把 quota_err 丢掉，
                // 于是「为什么这个号没数据 / 为什么它要重新登录」在界面上永远查不到。
                if let Some(err) = quota_err {
                    saved.quota_query_last_error = Some(err);
                    saved.quota_query_last_error_at = Some(chrono::Utc::now().timestamp_millis());
                    saved = store::upsert_account(platform, saved)?;
                }
                success += 1;
            }
            Err(e) => {
                eprintln!("[Buddy] 刷新账号 {} 失败: {}", account.id, e);
            }
        }
    }
    Ok(success)
}

#[tauri::command]
pub async fn buddy_query_usage(
    platform: String,
    account_id: String,
) -> Result<BuddyAccount, String> {
    let platform = platform_from_str(&platform)?;
    let account = store::load_account(platform, &account_id)
        .ok_or_else(|| format!("账号不存在: {}", account_id))?;
    let (refreshed, quota_err) = api::refresh_payload_for_account(platform, &account).await?;
    let mut saved = store::upsert_account(platform, refreshed)?;
    if let Some(err) = quota_err {
        saved.quota_query_last_error = Some(err);
        saved.quota_query_last_error_at = Some(chrono::Utc::now().timestamp_millis());
        saved = store::upsert_account(platform, saved)?;
    }
    Ok(saved)
}

#[tauri::command]
pub async fn buddy_query_all_usage(platform: String) -> Result<i32, String> {
    let platform = platform_from_str(&platform)?;
    let accounts = store::list_accounts(platform);
    let now_ms = chrono::Utc::now().timestamp_millis();
    // 已判吊销的账号在冷却期内跳过：面板每次打开都会自动同步一次，
    // 不设冷却等于每次打开都把死号打成一轮 401 风暴（也正是 2026-10-23 的教训之一）。
    const REVOKED_COOLDOWN_MS: i64 = 60 * 60 * 1000;
    let mut success = 0;
    for account in accounts {
        if account.status.as_deref() == Some(api::STATUS_AUTH_REVOKED) {
            if let Some(at) = account.quota_query_last_error_at {
                if now_ms.saturating_sub(at) < REVOKED_COOLDOWN_MS {
                    continue;
                }
            }
        }
        match api::refresh_payload_for_account(platform, &account).await {
            Ok((refreshed, quota_err)) => {
                let mut saved = store::upsert_account(platform, refreshed)?;
                // 批量路径也要把失败原因落盘：否则「为什么这个号没数据」在界面上永远查不到
                if let Some(err) = quota_err {
                    saved.quota_query_last_error = Some(err);
                    saved.quota_query_last_error_at = Some(now_ms);
                    saved = store::upsert_account(platform, saved)?;
                }
                success += 1;
            }
            Err(e) => {
                eprintln!("[Buddy] 查询账号 {} 用量失败: {}", account.id, e);
            }
        }
    }
    Ok(success)
}

// ─── 新增：OAuth 登录 / 粘贴 token ───

#[tauri::command]
pub async fn buddy_oauth_start(platform: String) -> Result<api::OAuthStartResponse, String> {
    let platform = platform_from_str(&platform)?;
    api::oauth_start(platform).await
}

#[tauri::command]
pub async fn buddy_oauth_complete(
    platform: String,
    login_id: String,
) -> Result<BuddyAccount, String> {
    let platform = platform_from_str(&platform)?;
    let account = api::oauth_complete(platform, &login_id).await?;
    let saved = store::upsert_account(platform, account)?;
    // 登录后补全用量
    match api::refresh_payload_for_account(platform, &saved).await {
        Ok((refreshed, _)) => Ok(store::upsert_account(platform, refreshed)?),
        Err(e) => {
            eprintln!("[Buddy OAuth] 登录后刷新失败，保留原账号信息: {}", e);
            Ok(saved)
        }
    }
}

#[tauri::command]
pub fn buddy_oauth_cancel(platform: String, login_id: Option<String>) -> Result<(), String> {
    let platform = platform_from_str(&platform)?;
    api::oauth_cancel(platform, login_id.as_deref())
}

#[tauri::command]
pub async fn buddy_add_account_with_token(
    platform: String,
    access_token: String,
) -> Result<BuddyAccount, String> {
    let platform = platform_from_str(&platform)?;
    let account = api::build_payload_from_token(platform, access_token.trim()).await?;
    store::upsert_account(platform, account)
}

// ─── 签到 ───

#[tauri::command]
pub async fn buddy_checkin(
    app: tauri::AppHandle,
    platform: String,
    account_id: String,
) -> Result<api::CheckinResponse, String> {
    let platform = platform_from_str(&platform)?;
    let account = store::load_account(platform, &account_id)
        .ok_or_else(|| format!("账号不存在: {}", account_id))?;

    let mut response = api::perform_checkin(
        &account.access_token,
        account.uid.as_deref(),
        account.enterprise_id.as_deref(),
        account.domain.as_deref(),
    )
    .await
    .map_err(|err| {
        // AUTH_EXPIRED 前缀是给调度器用的内部约定，手动签到要展示给人看的文案
        match err.strip_prefix(api::AUTH_EXPIRED_PREFIX) {
            Some(text) => text.to_string(),
            None => err,
        }
    })?;

    // 失败但非「活动未开启」时回查一次状态：官方在部分通道会用业务码返回「今日已签到」，
    // 不回查就会把「已签到」显示成失败。自动签到早已这么做，手动路径此前漏了这一步。
    if !response.success && response.inactive != Some(true) {
        if let Ok(status) = api::get_checkin_status(
            &account.access_token,
            account.uid.as_deref(),
            account.enterprise_id.as_deref(),
            account.domain.as_deref(),
        )
        .await
        {
            if status.today_checked_in {
                response.success = true;
                response.already = Some(true);
                response.message = Some("今日已完成签到".to_string());
            }
        }
    }

    let already = response.already == Some(true);
    let inactive = response.inactive == Some(true);

    if response.success || inactive {
        let now = chrono::Utc::now().timestamp();
        let streak = response
            .streak_days
            .unwrap_or_else(|| account.checkin_streak.saturating_add(1));
        let reward = response.reward.clone().or_else(|| {
            response.credit.map(|credit| serde_json::json!({ "credit": credit }))
        });
        if response.success {
            api::update_checkin_info(platform, &account_id, Some(now), streak, reward)
                .map_err(|e| format!("签到成功但更新状态失败: {}", e))?;
        }

        // 手动签到同样要"落账"：写每日归档 + 行为日志。
        // 之前这里只更新了账号自身的 last_checkin_time，于是手动签到成功后
        // 账号状态卡与日历仍显示「待生成」，属于本模块的历史缺陷。
        let email = if account.email.trim().is_empty() {
            account.id.clone()
        } else {
            account.email.clone()
        };
        let today = daily_history::today_string();
        let credit = response.credit;
        // 三种「无需重试」的终态：已签到 / 活动未开启 / 本次成功
        let (archive_status, log_status, fallback_message) = if inactive {
            (
                daily_history::checkin_status::INACTIVE,
                "inactive",
                "签到活动未开启",
            )
        } else if already {
            (
                daily_history::checkin_status::ALREADY_CHECKED,
                "already_checked",
                "今日已完成签到",
            )
        } else {
            (
                daily_history::checkin_status::SUCCESS,
                "success",
                "签到成功（手动）",
            )
        };
        let message = response
            .message
            .clone()
            .filter(|text| !text.trim().is_empty())
            .unwrap_or_else(|| fallback_message.to_string());
        let patch = daily_history::CheckinPatch::new()
            .actual_time(daily_history::now_time_string())
            .status(archive_status)
            .source("kira")
            .credit(credit)
            .streak(Some(streak))
            .message(Some(message.clone()));
        if let Err(err) = daily_history::upsert_checkin(&today, &account_id, &email, patch) {
            eprintln!("[Buddy] 手动签到写入归档失败({}): {}", account_id, err);
        }
        if let Err(err) = action_log::append_action_logs(&[action_log::make_entry(
            "checkin",
            &account_id,
            &email,
            log_status,
            Some(message),
            credit,
        )]) {
            eprintln!("[Buddy] 手动签到写入行为日志失败({}): {}", account_id, err);
        }
        use tauri::Emitter;
        let _ = app.emit("buddy-action-logs-changed", ());
    }

    Ok(response)
}

#[tauri::command]
pub async fn buddy_checkin_status(
    platform: String,
    account_id: String,
) -> Result<api::CheckinStatusResponse, String> {
    let platform = platform_from_str(&platform)?;
    let account = store::load_account(platform, &account_id)
        .ok_or_else(|| format!("账号不存在: {}", account_id))?;
    api::get_checkin_status(
        &account.access_token,
        account.uid.as_deref(),
        account.enterprise_id.as_deref(),
        account.domain.as_deref(),
    )
    .await
}

// ─── 自动签到 ───

#[tauri::command]
pub fn buddy_auto_checkin_get_config() -> Result<auto_checkin::BuddyAutoCheckinConfig, String> {
    auto_checkin::get_config_checked()
}

#[tauri::command]
pub fn buddy_auto_checkin_save_config(
    config: auto_checkin::BuddyAutoCheckinConfig,
) -> Result<(), String> {
    auto_checkin::save_config(&config)
}

#[tauri::command]
pub fn buddy_get_action_logs() -> Result<Vec<action_log::BuddyActionLogEntry>, String> {
    action_log::get_action_logs()
}

#[tauri::command]
pub fn buddy_clear_action_logs() -> Result<(), String> {
    action_log::clear_action_logs()
}

/// 今日签到任务列表（账号 + 计划时间 + 状态）。签到仅限 WorkBuddy，固定按 WB 账号返回。
#[tauri::command]
pub fn buddy_auto_checkin_tasks() -> Result<auto_checkin::BuddyCheckinTasksView, String> {
    auto_checkin::build_tasks_view(BuddyPlatform::Workbuddy)
}

/// 手动立即执行一轮自动签到（仅 WorkBuddy）。
#[tauri::command]
pub async fn buddy_auto_checkin_run(
    app: tauri::AppHandle,
    force: Option<bool>,
) -> Result<String, String> {
    auto_checkin::run_auto_checkin_cycle_if_needed(BuddyPlatform::Workbuddy, &app, force.unwrap_or(false))
        .await
}

// ─── 会话管理 ───

/// 删除会话（数据库记录 + 本地会话文件，含正文与辅助目录）。
///
/// 走 `spawn_blocking`：一次可能删几百 MB 正文，同步命令会把界面卡住。
/// 进入时占**切号闸**（与 `buddy_switch_account` 同一把）—— 会话同步 worker 与切号并行时，
/// 它可能已读到源行、在删除返回后又把会话写回来（WorkDaddy 1.2.64 的同类问题）。
#[tauri::command]
pub async fn buddy_delete_sessions(
    platform: String,
    conversation_ids: Vec<String>,
) -> Result<sessions::BuddySessionDeleteReport, String> {
    let platform_enum = platform_from_str(&platform)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = SwitchGuard::acquire(platform_enum)?;
        sessions::delete_sessions(&platform, &conversation_ids)
    })
    .await
    .map_err(|e| format!("删除会话后台任务失败: {}", e))?
}

#[tauri::command]
pub fn buddy_list_sessions(
    platform: String,
    keyword: Option<String>,
    status: Option<String>,
) -> Result<Vec<sessions::BuddySessionRecord>, String> {
    let filter = sessions::BuddySessionFilter { keyword, status };
    sessions::list_sessions(&platform, &filter)
}

/// 分叉会话：复制正文并用新 id 注册一条会话记录，原会话保持不动。
#[tauri::command]
pub fn buddy_fork_session(
    platform: String,
    conversation_id: String,
    title: Option<String>,
) -> Result<sessions::BuddySessionForkReport, String> {
    sessions::fork_session(&platform, &conversation_id, title)
}

/// 改会话的所属目录（**连正文一起搬**到目标目录对应的工作区）。
///
/// 只改索引里的 `cwd` 不改正文会让会话变成「内容缺失」，所以两处一起改，
/// 并在搬之前整批备份一次（备份目录在返回值的 `backup_dir`，要还原就整棵放回去）。
#[tauri::command]
pub fn buddy_move_sessions(
    platform: String,
    conversation_ids: Vec<String>,
    target_dir: String,
) -> Result<sessions::BuddySessionMoveReport, String> {
    // WorkBuddy 的会话正文不在 md5(cwd) 目录里（5.x 是共享库），搬法完全不同，先不支持
    if platform_from_str(&platform)? != models::BuddyPlatform::CodebuddyCn {
        return Err("目前只支持 CodeBuddy CN 会话改目录".to_string());
    }
    if conversation_ids.is_empty() {
        return Err("请先选中要改目录的会话".to_string());
    }
    sessions::move_sessions_to_dir(&conversation_ids, &target_dir)
}

/// 列出**当前登录账号**（作为合并目标）的待处理会话冲突。
///
/// 冲突文件按目标账号分文件落盘，来回切换会留下多份旧快照；只按当前账号
/// 过滤，避免「横幅 7 个、弹窗 13 条」的错位（多出的是过时方向的旧记录）。
/// 冲突目前只会产生于 CodeBuddy CN（WorkBuddy 的合并是共享库重映射，无基线差异）。
#[tauri::command]
pub fn buddy_list_session_conflicts(
    platform: String,
) -> Result<Vec<session_sync::PendingConflict>, String> {
    match platform_from_str(&platform)? {
        BuddyPlatform::CodebuddyCn => {
            Ok(session_transfer::codebuddy::list_pending_conflicts_for_current_account())
        }
        BuddyPlatform::Workbuddy => Ok(Vec::new()),
    }
}

/// 裁决一条会话冲突，返回当前账号剩余的待处理冲突。
///
/// `action`：`merge`（取较新）/ `overwrite`（用来源覆盖目标）/ `keep`（保留目标）/ `ignore`。
///
/// 走 `spawn_blocking`：裁决要**递归复制整个会话目录**（几十~几百 MB）并重写 index，
/// 同步命令会把这些全塞进主线程 —— 表现为点完按钮界面直接卡死、无任何反馈。
/// 切换账号（`buddy_switch_account`）早就是这么做的，这里补齐同一处理。
#[tauri::command]
pub async fn buddy_resolve_session_conflict(
    app: tauri::AppHandle,
    platform: String,
    conversation_id: String,
    action: String,
) -> Result<Vec<session_sync::PendingConflict>, String> {
    let platform = platform_from_str(&platform)?;
    let action = session_transfer::codebuddy::ConflictAction::parse(&action)?;
    match platform {
        BuddyPlatform::CodebuddyCn => {
            // 单条也走批量入口：批量版本一次加锁、共用备份目录与基线，
            // 而单条版本每次都会清空备份目录（串行调用会丢掉前面的回滚点）。
            let ids = vec![conversation_id];
            let total = ids.len();
            tauri::async_runtime::spawn_blocking(move || {
                let ctx = ConflictProgressCtx { app: Some(&app), platform, total };
                let result =
                    session_transfer::codebuddy::resolve_conflicts_command(&ids, action, Some(&ctx));
                emit_conflict_progress(Some(&app), platform, "done", total, total, None, None);
                result
            })
            .await
            .map_err(|e| format!("冲突处理任务异常退出: {}", e))?
        }
        BuddyPlatform::Workbuddy => Err("WorkBuddy 的会话合并不产生冲突，无需处理".to_string()),
    }
}

/// **批量**裁决会话冲突：对选中的多条执行同一 action，返回当前账号剩余的待处理冲突。
///
/// 走批量命令而不是前端串行调单条：备份目录在每次 `prepare_backup_root` 时会被清空，
/// 串行调用只会留下最后一条的备份（前面的无法回滚）。
///
/// 同样是 `spawn_blocking` + 逐条进度事件，避免大批量处理时界面卡死。
#[tauri::command]
pub async fn buddy_resolve_session_conflicts(
    app: tauri::AppHandle,
    platform: String,
    conversation_ids: Vec<String>,
    action: String,
) -> Result<Vec<session_sync::PendingConflict>, String> {
    let platform = platform_from_str(&platform)?;
    let action = session_transfer::codebuddy::ConflictAction::parse(&action)?;
    match platform {
        BuddyPlatform::CodebuddyCn => {
            let total = conversation_ids.len();
            tauri::async_runtime::spawn_blocking(move || {
                let ctx = ConflictProgressCtx { app: Some(&app), platform, total };
                let result = session_transfer::codebuddy::resolve_conflicts_command(
                    &conversation_ids,
                    action,
                    Some(&ctx),
                );
                emit_conflict_progress(Some(&app), platform, "done", total, total, None, None);
                result
            })
            .await
            .map_err(|e| format!("冲突处理任务异常退出: {}", e))?
        }
        BuddyPlatform::Workbuddy => Err("WorkBuddy 的会话合并不产生冲突，无需处理".to_string()),
    }
}

/// 列出当前账号的**冲突处理结果**（会话明细据此显示「已合并 / 已覆盖 / 已保留」，
/// 而不是永远停在切换那一刻的初始状态）。
#[tauri::command]
pub fn buddy_list_conflict_resolutions(
    platform: String,
) -> Result<Vec<session_sync::ConflictResolution>, String> {
    match platform_from_str(&platform)? {
        BuddyPlatform::CodebuddyCn => {
            Ok(session_transfer::codebuddy::list_conflict_resolutions_for_current_account())
        }
        BuddyPlatform::Workbuddy => Ok(Vec::new()),
    }
}

/// 读取冲突会话某一侧（source / target）的对话内容预览。
#[tauri::command]
pub fn buddy_read_conflict_messages(
    platform: String,
    conversation_id: String,
    side: String,
) -> Result<Vec<session_transfer::codebuddy::ConflictMessage>, String> {
    match platform_from_str(&platform)? {
        BuddyPlatform::CodebuddyCn => {
            session_transfer::codebuddy::read_conflict_messages(&conversation_id, &side)
        }
        BuddyPlatform::Workbuddy => Err("WorkBuddy 的会话合并不产生冲突，无需处理".to_string()),
    }
}

// ─── 路径信息（供前端展示/诊断） ───

#[tauri::command]
pub fn buddy_get_paths(platform: String) -> Result<serde_json::Value, String> {
    let platform = platform_from_str(&platform)?;
    let (data_dir, state_db, auth_file) = match platform {
        // 无数据目录/state.vscdb 时对应函数返回 None，只有登录文件
        BuddyPlatform::Workbuddy => (
            workbuddy::default_data_dir(platform).map(|p| p.to_string_lossy().to_string()),
            workbuddy::default_state_db_path(platform).map(|p| p.to_string_lossy().to_string()),
            workbuddy::default_auth_file_path(platform).map(|p| p.to_string_lossy().to_string()),
        ),
        BuddyPlatform::CodebuddyCn => (
            codebuddy_cn::default_data_dir().map(|p| p.to_string_lossy().to_string()),
            codebuddy_cn::default_state_db_path().map(|p| p.to_string_lossy().to_string()),
            None,
        ),
    };
    Ok(serde_json::json!({
        "platform": platform.as_str(),
        "dataDir": data_dir,
        "stateDb": state_db,
        "authFile": auth_file,
    }))
}

// ─── 客户端路径配置（切换时关闭/重启的 WorkBuddy / CodeBuddy CN） ───

#[tauri::command]
pub fn buddy_get_client_paths() -> Result<Vec<client_process::BuddyClientPath>, String> {
    Ok(client_process::client_paths_view())
}

#[tauri::command]
pub fn buddy_set_client_path(platform: String, path: String) -> Result<(), String> {
    let platform = platform_from_str(&platform)?;
    client_process::set_client_path(platform, &path)
}

/// 启动后台自动签到调度器（应用启动时调用）
pub fn start_auto_checkin_scheduler(app: tauri::AppHandle) {
    auto_checkin::start_auto_checkin_scheduler(app);
}

#[cfg(test)]
mod switch_guard_tests {
    use super::*;

    #[test]
    fn landing_is_ok_only_when_token_matches() {
        assert!(diagnose_switch_landing(Ok(Some("tok-a".into())), "tok-a", "WorkBuddy", false).is_none());
    }

    #[test]
    fn landing_reports_missing_and_overwritten_states() {
        let missing = diagnose_switch_landing(Ok(None), "tok-a", "WorkBuddy", false)
            .expect("登录态消失必须报错，不能静默成功");
        assert!(missing.contains("已不存在"), "{}", missing);
        assert!(missing.contains("完全退出"), "要给可操作提示: {}", missing);

        let overwritten = diagnose_switch_landing(Ok(Some("tok-b".into())), "tok-a", "WorkBuddy", false)
            .expect("被回写成别人必须报错");
        assert!(overwritten.contains("改回另一个账号"), "{}", overwritten);
    }

    #[test]
    fn landing_probe_failure_does_not_fail_the_switch() {
        // 探测本身失败（取不到官方密钥 / 库被占用）不能报成"切号没生效"
        assert!(diagnose_switch_landing(Err("取密钥失败".into()), "tok-a", "WorkBuddy", false).is_none());
    }

    #[test]
    fn landing_fails_when_logout_marker_survives_even_if_token_matches() {
        // 标记在 → 客户端不读登录文件：token 读起来完全正确也必须判失败，
        // 否则用户看到的是"切好了"却在客户端被要求重新登录
        let problem = diagnose_switch_landing(Ok(Some("tok-a".into())), "tok-a", "WorkBuddy", true)
            .expect("标记仍在必须报错");
        assert!(problem.contains("已登出"), "{}", problem);
        assert!(problem.contains("完全退出"), "要给可操作提示: {}", problem);
    }

    #[test]
    fn switch_guard_is_exclusive_per_platform_and_released_on_drop() {
        let first = SwitchGuard::acquire(BuddyPlatform::Workbuddy).expect("首次应拿到闸");
        let err = SwitchGuard::acquire(BuddyPlatform::Workbuddy)
            .err()
            .expect("同平台第二次必须被拒（不排队）");
        assert!(err.contains("正在切换账号"), "{}", err);
        assert!(
            SwitchGuard::acquire(BuddyPlatform::CodebuddyCn).is_ok(),
            "不同平台不应互相阻塞"
        );
        drop(first);
        assert!(
            SwitchGuard::acquire(BuddyPlatform::Workbuddy).is_ok(),
            "释放后必须能再次获取"
        );
    }
}