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
pub(crate) mod twoapi;
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
            return Err(format!(
                "{} 正在切换账号中，请等当前切换结束后再试",
                display
            ));
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
        assert!(
            diagnose_switch_landing(Ok(Some("tok-a".into())), "tok-a", "WorkBuddy", false)
                .is_none()
        );
    }

    #[test]
    fn landing_reports_missing_and_overwritten_states() {
        let missing = diagnose_switch_landing(Ok(None), "tok-a", "WorkBuddy", false)
            .expect("登录态消失必须报错，不能静默成功");
        assert!(missing.contains("已不存在"), "{}", missing);
        assert!(missing.contains("完全退出"), "要给可操作提示: {}", missing);

        let overwritten =
            diagnose_switch_landing(Ok(Some("tok-b".into())), "tok-a", "WorkBuddy", false)
                .expect("被回写成别人必须报错");
        assert!(overwritten.contains("改回另一个账号"), "{}", overwritten);
    }

    #[test]
    fn landing_probe_failure_does_not_fail_the_switch() {
        // 探测本身失败（取不到官方密钥 / 库被占用）不能报成"切号没生效"
        assert!(
            diagnose_switch_landing(Err("取密钥失败".into()), "tok-a", "WorkBuddy", false)
                .is_none()
        );
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
mod account_commands;
pub use account_commands::*;

mod switch_commands;
pub use switch_commands::*;

mod usage_commands;
pub use usage_commands::*;

mod checkin_commands;
pub use checkin_commands::*;

mod session_commands;
pub use session_commands::*;
