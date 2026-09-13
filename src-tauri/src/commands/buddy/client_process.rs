//! Buddy 模块客户端进程管理：切换账号时「关闭运行中的客户端 → 切换」。
//!
//! 复刻自 cockpit-tools `modules/process_*`（workbuddy / codebuddy-cn 部分）：
//! - 进程识别：按安装 exe 名称匹配（sysinfo），与参考实现的 PowerShell/sysinfo 探测等价
//! - 关闭（按平台区分）：
//!   · WorkBuddy：优雅退出，超时升级强杀（taskkill /T /F / kill -9），保证切换可以继续
//!   · CodeBuddy CN：只发送优雅退出请求（WM_CLOSE / kill -15），超时后提示用户手动退出——
//!     不做强杀（强杀会让客户端来不及保存状态、丢失未落盘的会话）
//! - 启动（按平台区分）：WorkBuddy 切换后自动重启；CodeBuddy CN 不自动启动，
//!   由调用方提示用户手动启动。安装路径解析（client_paths.json + 常见安装位置候选）
//!   供启动与设置页展示共用

use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::models::BuddyPlatform;
use crate::commands::config::get_data_dir;

pub const APP_PATH_MISSING_PREFIX: &str = "APP_PATH_NOT_FOUND:";

/// 客户端展示名（用于消息文案）
pub fn app_display_name(platform: BuddyPlatform) -> &'static str {
    match platform {
        BuddyPlatform::Workbuddy => "WorkBuddy",
        BuddyPlatform::CodebuddyCn => "CodeBuddy CN",
    }
}

/// 进程名候选（Windows 含 .exe；macOS/Linux 为可执行文件名，不区分大小写匹配）
fn process_name_candidates(platform: BuddyPlatform) -> &'static [&'static str] {
    match platform {
        BuddyPlatform::Workbuddy => &["WorkBuddy.exe", "WorkBuddy", "workbuddy"],
        BuddyPlatform::CodebuddyCn => &["CodeBuddy CN.exe", "CodeBuddy.exe", "CodeBuddy CN", "codebuddy-cn"],
    }
}

/// 客户端启动路径配置（buddy/client_paths.json：{ "workbuddy": "...", "codebuddy-cn": "..." }）
fn client_paths_file() -> PathBuf {
    get_data_dir().join("buddy").join("client_paths.json")
}

fn read_client_paths() -> serde_json::Map<String, serde_json::Value> {
    std::fs::read_to_string(client_paths_file())
        .ok()
        .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

/// 用户配置的启动路径覆盖
fn configured_launch_path(platform: BuddyPlatform) -> Option<PathBuf> {
    let map = read_client_paths();
    let raw = map.get(platform.as_str())?.as_str()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let candidate = PathBuf::from(trimmed);
    candidate.exists().then_some(candidate)
}

/// 设置/清除（空字符串）某平台的启动路径覆盖。非空路径必须存在。
pub fn set_client_path(platform: BuddyPlatform, path: &str) -> Result<(), String> {
    let mut map = read_client_paths();
    let trimmed = path.trim();
    if trimmed.is_empty() {
        map.remove(platform.as_str());
    } else {
        let candidate = PathBuf::from(trimmed);
        if !candidate.exists() {
            return Err(format!("路径不存在：{}", trimmed));
        }
        map.insert(
            platform.as_str().to_string(),
            serde_json::Value::String(trimmed.to_string()),
        );
    }
    let content = serde_json::to_string_pretty(&serde_json::Value::Object(map))
        .map_err(|e| format!("序列化客户端路径配置失败: {}", e))?;
    super::store::write_atomic(&client_paths_file(), &content)
}

/// 单平台客户端路径视图（供设置界面展示）
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuddyClientPath {
    pub platform: String,
    pub label: String,
    /// 用户配置的覆盖路径（未配置为 null）
    pub configured: Option<String>,
    /// 实际生效路径（覆盖优先，其次自动检测；均无则 null）
    pub resolved: Option<String>,
}

/// 两个平台的客户端路径视图
pub fn client_paths_view() -> Vec<BuddyClientPath> {
    let map = read_client_paths();
    [BuddyPlatform::Workbuddy, BuddyPlatform::CodebuddyCn]
        .iter()
        .map(|platform| {
            let configured = map
                .get(platform.as_str())
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            BuddyClientPath {
                platform: platform.as_str().to_string(),
                label: app_display_name(*platform).to_string(),
                configured,
                resolved: resolve_launch_path(*platform).map(|p| p.to_string_lossy().to_string()),
            }
        })
        .collect()
}

/// 常见安装位置候选（复刻 cockpit-tools process_path_resolution 的 Windows/macOS/Linux 候选）
fn default_launch_candidates(platform: BuddyPlatform) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    match platform {
        BuddyPlatform::Workbuddy => {
            #[cfg(target_os = "windows")]
            {
                if let Ok(local) = std::env::var("LOCALAPPDATA") {
                    candidates.push(
                        PathBuf::from(local)
                            .join("Programs")
                            .join("WorkBuddy")
                            .join("WorkBuddy.exe"),
                    );
                }
                if let Ok(pf) = std::env::var("PROGRAMFILES") {
                    candidates.push(PathBuf::from(pf).join("WorkBuddy").join("WorkBuddy.exe"));
                }
            }
            #[cfg(target_os = "macos")]
            {
                candidates.push(PathBuf::from("/Applications/WorkBuddy.app"));
            }
            #[cfg(target_os = "linux")]
            {
                for bin in ["/usr/bin/workbuddy", "/usr/local/bin/workbuddy", "/opt/workbuddy/workbuddy"] {
                    candidates.push(PathBuf::from(bin));
                }
            }
        }
        BuddyPlatform::CodebuddyCn => {
            #[cfg(target_os = "windows")]
            {
                if let Ok(local) = std::env::var("LOCALAPPDATA") {
                    for exe in ["CodeBuddy CN.exe", "CodeBuddy.exe"] {
                        candidates.push(
                            PathBuf::from(&local)
                                .join("Programs")
                                .join("CodeBuddy CN")
                                .join(exe),
                        );
                    }
                }
                if let Ok(pf) = std::env::var("PROGRAMFILES") {
                    for exe in ["CodeBuddy CN.exe", "CodeBuddy.exe"] {
                        candidates.push(PathBuf::from(&pf).join("CodeBuddy CN").join(exe));
                    }
                }
            }
            #[cfg(target_os = "macos")]
            {
                candidates.push(PathBuf::from("/Applications/CodeBuddy CN.app"));
            }
            #[cfg(target_os = "linux")]
            {
                for bin in [
                    "/usr/bin/codebuddy-cn",
                    "/usr/local/bin/codebuddy-cn",
                    "/opt/codebuddy-cn/codebuddy-cn",
                ] {
                    candidates.push(PathBuf::from(bin));
                }
            }
        }
    }
    candidates
}

/// 解析客户端启动路径（配置覆盖优先，其次常见安装位置）
pub fn resolve_launch_path(platform: BuddyPlatform) -> Option<PathBuf> {
    if let Some(path) = configured_launch_path(platform) {
        return Some(path);
    }
    default_launch_candidates(platform).into_iter().find(|p| p.exists())
}

/// 正在运行的客户端进程 pid 列表（按进程名匹配，不区分大小写）
pub fn running_pids(platform: BuddyPlatform) -> Vec<u32> {
    let mut pids: Vec<u32> = process_pairs(platform).into_iter().map(|(pid, _)| pid).collect();
    pids.sort_unstable();
    pids
}

/// 客户端进程快照：pid + 父 pid（Electron 的渲染/GPU/工具子进程与主进程同名）。
fn process_pairs(platform: BuddyPlatform) -> Vec<(u32, Option<u32>)> {
    let names: Vec<String> = process_name_candidates(platform)
        .iter()
        .map(|n| n.to_lowercase())
        .collect();
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All);
    sys.processes()
        .iter()
        .filter(|(_, process)| {
            names.iter().any(|name| {
                process
                    .name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(name)
            })
        })
        .map(|(pid, process)| (pid.as_u32(), process.parent().map(|p| p.as_u32())))
        .filter(|(pid, _)| *pid != std::process::id())
        .collect()
}

/// 从候选 pid 中挑顶层进程（父进程不在候选集内）：
/// 优雅退出只需关 Electron 主进程的窗口，子进程会随主进程一起退出。
fn root_targets(pids: &[u32], parent_of: &dyn Fn(u32) -> Option<u32>) -> Vec<u32> {
    pids.iter()
        .copied()
        .filter(|pid| {
            parent_of(*pid)
                .map(|parent| pids.contains(&parent))
                .unwrap_or(false)
                == false
        })
        .collect()
}

/// taskkill 执行并收集 stderr 错误。
#[cfg(target_os = "windows")]
fn run_taskkill(args: &[&str]) -> Result<(), String> {
    let output = crate::commands::hidden_cmd::hidden_cmd("taskkill")
        .args(args)
        .output()
        .map_err(|e| format!("taskkill 启动失败: {}", e))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "taskkill 失败: {}",
            crate::commands::file_io::decode_text_bytes(&output.stderr).trim()
        ))
    }
}

#[cfg(not(target_os = "windows"))]
fn run_kill(sig: &str, pid: u32) -> Result<(), String> {
    let output = std::process::Command::new("kill")
        .args([sig, &pid.to_string()])
        .output()
        .map_err(|e| format!("kill 启动失败: {}", e))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "kill 失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// 优雅退出：不带 /F 的 taskkill 会给进程窗口发 WM_CLOSE，应用走自身退出流程。
/// 不带 /T：/T 会尝试逐个关闭进程树里的子进程，而 Electron 的 GPU/渲染/工具子进程
/// 没有窗口收不到 WM_CLOSE，taskkill 会刷一屏"只有强制终止才能终止此进程"并返回非 0；
/// 子进程本就会随主进程退出。unix 等价 kill -15。
fn request_graceful_close(pid: u32) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        run_taskkill(&["/PID", &pid.to_string()])
    }
    #[cfg(not(target_os = "windows"))]
    {
        run_kill("-15", pid)
    }
}

/// 强杀（仅 WorkBuddy 在优雅退出超时后升级使用）：taskkill /T /F；unix 用 kill -9。
fn request_force_kill(pid: u32) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        run_taskkill(&["/PID", &pid.to_string(), "/T", "/F"])
    }
    #[cfg(not(target_os = "windows"))]
    {
        run_kill("-9", pid)
    }
}

/// 等待给定进程全部退出；超时返回 false。
fn wait_pids_exit(pids: &[u32], timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        let mut sys = sysinfo::System::new();
        let wanted: Vec<sysinfo::Pid> = pids.iter().map(|p| sysinfo::Pid::from_u32(*p)).collect();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&wanted));
        let alive = wanted.iter().any(|p| sys.process(*p).is_some());
        if !alive {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// 关闭运行中的客户端（按平台区分强杀策略）：
/// - WorkBuddy：优雅退出 → 超时升级强杀剩余进程，保证切换可以继续；
/// - CodeBuddy CN：只做优雅退出，超时后返回可操作错误，提示用户手动退出后再切换
///   （不强杀：强杀会让客户端来不及保存状态、丢失未落盘的会话）。
/// 没有进程在运行时直接返回 Ok。
pub fn close_running(platform: BuddyPlatform, timeout_secs: u64) -> Result<(), String> {
    let pairs = process_pairs(platform);
    if pairs.is_empty() {
        return Ok(());
    }
    let all: Vec<u32> = pairs.iter().map(|(pid, _)| *pid).collect();
    let parent_map: std::collections::HashMap<u32, Option<u32>> = pairs.into_iter().collect();
    let targets = root_targets(&all, &|pid| *parent_map.get(&pid).unwrap_or(&None));

    let mut last_error = None;
    eprintln!(
        "[Buddy Close] {} 请求优雅退出: 顶层 pid={:?}（全部 {:?}）",
        app_display_name(platform),
        targets,
        all
    );
    for pid in &targets {
        if let Err(err) = request_graceful_close(*pid) {
            eprintln!("[Buddy Close] pid={} 优雅退出请求失败: {}", pid, err);
            last_error = Some(err);
        }
    }
    let total = Duration::from_secs(timeout_secs);
    let graceful_wait = total.mul_f32(0.75);
    if wait_pids_exit(&all, graceful_wait) {
        return Ok(());
    }

    let stuck = running_pids(platform);
    if stuck.is_empty() {
        return Ok(());
    }

    // CodeBuddy CN：不强杀——等待超时即报错，由前端提示用户手动退出后再切换
    if let BuddyPlatform::CodebuddyCn = platform {
        let detail = last_error.map(|e| format!("（{}）", e)).unwrap_or_default();
        return Err(format!(
            "检测到 CodeBuddy CN 仍在运行（pid: {:?}），已请求其退出但未响应，请手动退出后重试{}",
            stuck, detail
        ));
    }

    // WorkBuddy：优雅退出超时，升级强杀剩余进程
    eprintln!("[Buddy Close] WorkBuddy 优雅退出超时，升级强杀: pid={:?}", stuck);
    for pid in &stuck {
        if let Err(err) = request_force_kill(*pid) {
            eprintln!("[Buddy Close] pid={} 强杀失败: {}", pid, err);
            last_error = Some(err);
        }
    }
    if wait_pids_exit(&stuck, total.saturating_sub(graceful_wait)) {
        return Ok(());
    }
    let alive = running_pids(platform);
    if alive.is_empty() {
        return Ok(());
    }
    Err(format!(
        "无法关闭运行中的 WorkBuddy（pid: {:?}），请手动关闭后重试{}",
        alive,
        last_error
            .map(|e| format!("：{}", e))
            .unwrap_or_default()
    ))
}

/// 重新启动客户端（`--new-window`，分离于本进程）。仅 WorkBuddy 在切换后调用。
/// 未找到安装路径返回 APP_PATH_NOT_FOUND 前缀错误，由调用方按警告处理。
pub fn launch(platform: BuddyPlatform) -> Result<(), String> {
    let app_name = app_display_name(platform);
    let path = resolve_launch_path(platform).ok_or_else(|| {
        format!(
            "{}未找到 {} 安装路径，可在 {} 中配置",
            APP_PATH_MISSING_PREFIX,
            app_name,
            "buddy/client_paths.json"
        )
    })?;

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        let mut cmd = std::process::Command::new(&path);
        cmd.creation_flags(0x0800_0000 | CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
        cmd.arg("--new-window");
        let child = cmd
            .spawn()
            .map_err(|e| format!("启动 {} 失败：{}", app_name, e))?;
        eprintln!("[Buddy Launch] {} 已启动: pid={}, path={:?}", app_name, child.id(), path);
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("open")
            .args(["-n", &path.to_string_lossy(), "--args", "--new-window"])
            .output()
            .map_err(|e| format!("启动 {} 失败：{}", app_name, e))?;
        if !output.status.success() {
            return Err(format!(
                "启动 {} 失败：{}",
                app_name,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        eprintln!("[Buddy Launch] {} 已启动: path={:?}", app_name, path);
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        let child = std::process::Command::new(&path)
            .arg("--new-window")
            .spawn()
            .map_err(|e| format!("启动 {} 失败：{}", app_name, e))?;
        eprintln!("[Buddy Launch] {} 已启动: pid={}, path={:?}", app_name, child.id(), path);
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err(format!("暂不支持在当前平台启动 {}", app_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_candidates_match_expected_exes() {
        let cn = process_name_candidates(BuddyPlatform::CodebuddyCn);
        assert!(cn.iter().any(|n| n.eq_ignore_ascii_case("CodeBuddy CN.exe")));
        let wb = process_name_candidates(BuddyPlatform::Workbuddy);
        assert!(wb.iter().any(|n| n.eq_ignore_ascii_case("WorkBuddy.exe")));
    }

    #[test]
    fn root_targets_picks_electron_main_processes() {
        // 100/200 为两个独立启动的主进程（父是资源管理器 4）；101/102 是其渲染/GPU 子进程
        let all = vec![100u32, 101, 102, 200];
        let parent = |p: u32| match p {
            100 => Some(4),
            101 | 102 => Some(100),
            200 => Some(4),
            _ => None,
        };
        assert_eq!(root_targets(&all, &parent), vec![100, 200]);
    }

    #[test]
    fn running_pids_excludes_self() {
        // 本测试进程的 name 不应匹配任何 Buddy 客户端；仅验证调用不 panic 且不含自身
        let pids = running_pids(BuddyPlatform::Workbuddy);
        assert!(!pids.contains(&std::process::id()));
    }

    #[test]
    fn resolve_launch_path_is_none_or_existing() {
        if let Some(path) = resolve_launch_path(BuddyPlatform::Workbuddy) {
            assert!(path.exists());
        }
    }

    #[test]
    fn set_client_path_rejects_missing_file() {
        let err = set_client_path(BuddyPlatform::Workbuddy, "Z:/kira-buddy-definitely-missing.exe").unwrap_err();
        assert!(err.contains("路径不存在"), "unexpected error: {err}");
    }
}
