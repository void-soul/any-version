//! account commands responsibility.
use super::*;

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
    pub(super) account: BuddyAccount,
    /// 令牌签发域（JWT `iss` 的 origin）；老 token / 非 JWT 解不出时为 None
    pub(super) auth_origin: Option<String>,
    /// 该平台客户端鉴权时使用的域
    pub(super) platform_auth_origin: String,
    /// 是否与客户端域一致；`None` = 解不出签发域（**不拦**，交给用户判断）
    pub(super) auth_origin_matches: Option<bool>,
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
pub fn buddy_get_daily_records(
    from: String,
    to: String,
) -> Result<daily_history::DailyMap, String> {
    daily_history::get_range(&from, &to)
}

#[tauri::command]
pub async fn buddy_auto_travel_run(
    app: tauri::AppHandle,
    force: Option<bool>,
) -> Result<String, String> {
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
                    "WorkDaddy 导出文件只包含 WorkBuddy 账号，请在 WorkBuddy 平台下导入"
                        .to_string(),
                );
            }
            third_party_import::parse_workdaddy_export(&json_content, password.as_deref())?
        }
        third_party_import::TOOL_COCKPIT => {
            third_party_import::parse_cockpit_tools_export(platform, &json_content)?
        }
        other => return Err(format!("不支持的第三方导入来源: {}", other)),
    };

    store::validate_accounts(&parsed)?;
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
