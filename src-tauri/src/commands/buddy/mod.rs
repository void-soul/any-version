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
mod third_party_import;
mod workbuddy;

use models::{BuddyAccount, BuddyPlatform};

// ─── 切换进度事件（前端展示：关闭 → 合并 → 写入 → 启动 → 完成） ───

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddySwitchProgress {
    pub platform: String,
    pub account_id: String,
    /// closing | merging | writing | launching | done
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

fn platform_from_str(s: &str) -> Result<BuddyPlatform, String> {
    BuddyPlatform::from_str(s).ok_or_else(|| format!("未知平台: {}", s))
}

// ─── 账号库命令 ───

#[tauri::command]
pub fn buddy_list_accounts(platform: String) -> Result<Vec<BuddyAccount>, String> {
    let platform = platform_from_str(&platform)?;
    Ok(store::list_accounts(platform))
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
pub fn buddy_sync_accounts(from_platform: String, to_platform: String) -> Result<i32, String> {
    let from = platform_from_str(&from_platform)?;
    let to = platform_from_str(&to_platform)?;
    let synced = store::sync_accounts(from, to)?;
    Ok(synced as i32)
}

// ─── 本地导入（导入后经官方 API 补全资料/用量，并清理同 token 占位账号） ───

#[tauri::command]
pub async fn buddy_import_from_local(platform: String) -> Result<Option<BuddyAccount>, String> {
    let platform = platform_from_str(&platform)?;
    let local = match platform {
        BuddyPlatform::Workbuddy => workbuddy::import_payload_from_local()?,
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
        BuddyPlatform::Workbuddy => workbuddy::resolve_current_account_id(&accounts),
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

    // 复刻 cockpit-tools 切换时序：关闭运行中的客户端 → 合并来源会话 → 写入登录态 → 重新启动。
    // 各阶段通过 buddy-switch-progress 事件上报前端；合并统计随报告一并返回（前端渲染为文本）。
    tauri::async_runtime::spawn_blocking(move || {
        // 1) 请求关闭正在运行的客户端（进程占用 db/会话目录，必须先关再合并）。
        //    只做优雅退出、不强杀；客户端不响应时 close_running 报错，切换随之中止并提示手动退出。
        emit_switch_progress(Some(&app), platform, &account_id, "closing", 0, None);
        client_process::close_running(platform, 20)?;

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
            BuddyPlatform::Workbuddy => workbuddy::switch_account(&account_id)?,
            BuddyPlatform::CodebuddyCn => codebuddy_cn::switch_account(&account_id)?,
        };
        // 切换成功后持久化当前账号（复刻 provider_current_state，供前端稳定标识）
        let _ = store::set_current_account_id(platform, Some(&account_id));

        // 4) 启动行为按平台区分：WorkBuddy 自动重启客户端（启动失败不回滚切换）；
        //    CodeBuddy CN 不自动启动，提示用户手动启动。
        let mut message = message;
        match platform {
            BuddyPlatform::Workbuddy => {
                emit_switch_progress(Some(&app), platform, &account_id, "launching", 0, None);
                if let Err(err) = client_process::launch(platform) {
                    eprintln!(
                        "[Buddy Switch] {} 启动失败: {}",
                        client_process::app_display_name(platform),
                        err
                    );
                    message = format!(
                        "{}，但 {} 启动失败：{}",
                        message,
                        client_process::app_display_name(platform),
                        err.trim_start_matches(client_process::APP_PATH_MISSING_PREFIX)
                            .trim()
                    );
                }
            }
            BuddyPlatform::CodebuddyCn => {
                message = format!(
                    "{}，请手动启动 {}",
                    message,
                    client_process::app_display_name(platform)
                );
            }
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
pub async fn buddy_refresh_all_tokens(platform: String) -> Result<i32, String> {
    let platform = platform_from_str(&platform)?;
    let accounts = store::list_accounts(platform);
    let mut success = 0;
    for account in accounts {
        match api::refresh_payload_for_account(platform, &account).await {
            Ok((refreshed, _)) => {
                let _ = store::upsert_account(platform, refreshed);
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
    let mut success = 0;
    for account in accounts {
        match api::refresh_payload_for_account(platform, &account).await {
            Ok((refreshed, _)) => {
                let _ = store::upsert_account(platform, refreshed);
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

/// 删除会话（数据库记录 + 本地会话文件，含辅助目录）。
#[tauri::command]
pub fn buddy_delete_sessions(
    platform: String,
    conversation_ids: Vec<String>,
) -> Result<sessions::BuddySessionDeleteReport, String> {
    sessions::delete_sessions(&platform, &conversation_ids)
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

// ─── 路径信息（供前端展示/诊断） ───

#[tauri::command]
pub fn buddy_get_paths(platform: String) -> Result<serde_json::Value, String> {
    let platform = platform_from_str(&platform)?;
    let (data_dir, state_db, auth_file) = match platform {
        BuddyPlatform::Workbuddy => (
            workbuddy::default_data_dir().map(|p| p.to_string_lossy().to_string()),
            workbuddy::default_state_db_path().map(|p| p.to_string_lossy().to_string()),
            workbuddy::default_auth_file_path().map(|p| p.to_string_lossy().to_string()),
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