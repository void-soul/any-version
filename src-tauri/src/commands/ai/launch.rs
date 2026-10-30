use super::models::*;
use crate::commands::ai_registry::{registry, ConfigFileDef, ToolConfig};
use crate::commands::hidden_cmd;
use crate::proxy::types::ModelRoute;
use std::collections::HashMap;
use std::fs;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use super::config::{
    load_ai_config, load_last_launch_configs, load_sessions, save_last_launch_configs,
    save_sessions_to_file,
};
use super::terminal::{get_terminal_exe_cfg, is_ext_terminal};

/// Windows 应用执行别名（Store / MSIX）的 URI 前缀。
///
/// 这类应用在系统里**只有包注册，没有常规 exe 路径**：`%LOCALAPPDATA%\Programs\…`
/// 根本不存在，唯一能拉起它的是这个由 shell 解析的虚拟路径。
pub(crate) const APPS_FOLDER_URI_PREFIX: &str = "shell:AppsFolder\\";

/// 解析真正要执行的启动命令。
///
/// 1. 工具声明了 `startCommand` → 用它（如 `mimo .`，命令名交给方言解析）；
/// 2. 没声明（桌面应用的 paths.json 就是空串）→ 用**检测到的 exe 绝对路径**
///    （默认路径或用户在界面上手填的那条）；
/// 3. 还没有 → 退回 `detect_cmd` 的命令名（CLI 工具装在 PATH 里时的老行为）；
/// 4. 桌面端再退一步 → `shell:AppsFolder\…` 包 URI（Store 安装版唯一可启动的东西）；
/// 5. 全都没有 → None，由调用方报错，而不是拿空命令去启动。
///
/// 第 4 步是补上的：此前 Store 版 Claude / ChatGPT 桌面端走到这里直接返回 None，
/// 表现为「明明检测到了已安装，点启动却报『没有可用的启动命令』」，只能让用户手填路径。
pub(crate) fn resolve_start_command(
    tool_id: &str,
    paths: &crate::commands::ai_registry::PathConfig,
    start_command: &str,
) -> Option<String> {
    let declared = start_command.trim();
    if !declared.is_empty() {
        return Some(declared.to_string());
    }
    // 声明路径 → 兜底（npm 实际前缀 / 注册表卸载项）。两者都不接的话会出现
    //「检测到了（策略 3 用兜底命中）但启动找不到命令」，等于白检测。
    //
    // **必须再确认一次文件还在**：兜底来源是带缓存的（注册表卸载项索引 / npm 前缀），
    // 应用被卸载/重装后缓存里留着的是**已删除的旧路径**。拿它去 spawn 只会得到
    // `os error 3`（系统找不到指定的路径），点启动毫无反应 —— 实测：Claude 桌面端
    // 从 Squirrel 版换成 MSIX(Store) 版后就是这个症状。所以只认真存在的文件，
    // 不存在就继续往下走，最终落到 Store 包 URI 那条路。
    let resolved_exe = live_exe(
        super::tool_paths::find_declared_exe(tool_id, &paths.paths, "").or_else(|| {
            super::tool_paths::find_fallback_exe(tool_id, &paths.command, &paths.paths)
        }),
    );
    if let Some(exe) = resolved_exe {
        return Some(exe.to_string_lossy().to_string());
    }
    let fallback = paths
        .detect_cmd
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim();
    if !fallback.is_empty() {
        return Some(fallback.to_string());
    }
    #[cfg(windows)]
    {
        if crate::commands::ai_registry::tool_kind_of(Some(&paths.category)) == "desktop" {
            if let Some(uri) = resolve_store_launch_uri(paths) {
                eprintln!("[cli] 磁盘上没有 exe，回退到包 URI 启动: {}", uri);
                return Some(uri);
            }
        }
    }
    None
}

/// 只接受**当前确实存在**的exe 路径（`None` 透传）。
///
/// 存在的理由：启动命令有两级来源，第二级（`find_fallback_exe`：npm 实际全局前缀 +
/// 注册表卸载项索引）读的是**带缓存**的外部状态。Kira 常驻托盘，应用在那期间被
/// 卸载/重装是常事，缓存里那条路径就变成了死路。实测症状：Claude 桌面端从
/// Squirrel 版换成 MSIX(Store) 版后，检测显示「已安装（死路径）」，点启动报
/// `os error 3`（系统找不到指定的路径），窗口永不出现。
fn live_exe(exe: Option<PathBuf>) -> Option<PathBuf> {
    let exe = exe?;
    if exe.is_file() {
        return Some(exe);
    }
    eprintln!(
        "[cli] 检测到的可执行文件已不存在（多半是应用被重装过），改走后续策略: {}",
        exe.display()
    );
    None
}

/// 解析 Store（MSIX）应用的启动 URI：**先问系统包注册，再回退声明里的硬编码值**。
///
/// 为什么不能直接用声明的 `launchUri`：它把 publisher hash 写死了
/// （`shell:AppsFolder\OpenAI.Codex_2p2nqsd0c76g0!App`）。装的是 beta 渠道
/// （`OpenAI.CodexBeta`）或别的 publisher 时，这个 AUMID 不存在 —— 点启动要么没反应，
/// 要么 explorer 弹一个文件夹窗口。抄 EchoBird `codex_binary.rs::
/// resolve_desktop_launch_uri_scanned`：用 `Get-AppxPackage` 拿真实的
/// PackageFamilyName，与 publisher hash 无关。
///
/// 只在「磁盘上没有 exe」这条回退分支上调用，装了 exe 的工具不会去起 PowerShell。
#[cfg(windows)]
fn resolve_store_launch_uri(paths: &crate::commands::ai_registry::PathConfig) -> Option<String> {
    let declared = paths.launch_uri.as_deref().map(|s| s.trim()).unwrap_or("");
    if !declared.starts_with(APPS_FOLDER_URI_PREFIX) {
        return None;
    }
    // `shell:AppsFolder\<族名>!<应用 Id>`：族名换掉，应用 Id 沿用声明里的
    let package = super::detect::package_name_from_launch_uri(declared)?;
    // stable 优先、beta 兜底（EchoBird 实测 beta 渠道此前完全拉不起来）
    let candidates = [package.as_str(), &format!("{}Beta", package)];
    match crate::commands::project::scanner::msix_package_family_name(&candidates) {
        Ok(Some(pfn)) if pfn != package => {
            eprintln!("[cli] 已解析真实包族名: {}（声明里是 {}）", pfn, package);
            return Some(compose_apps_folder_uri(&pfn, declared));
        }
        Ok(_) => {}
        Err(e) => eprintln!("[cli] 包族名查询失败，回退声明的 URI: {}", e),
    }
    Some(declared.to_string())
}

/// 用真实包族名重组 AUMID：`shell:AppsFolder\<族名>!<应用 Id>`。
/// 应用 Id 沿用声明里的（`Claude` / `App` …），只有族名会被换掉。
#[cfg(windows)]
fn compose_apps_folder_uri(package_family_name: &str, declared_uri: &str) -> String {
    let app_id = declared_uri.split('!').nth(1).unwrap_or("App");
    format!(
        "{}{}!{}",
        APPS_FOLDER_URI_PREFIX, package_family_name, app_id
    )
}

/// 启动时的工作目录：用户没填项目目录时，用启动命令所在目录（桌面应用按 exe 启动，
/// exe 的目录最合理），再退到用户主目录。
///
/// 为什么要兜底：`cmd /c start /d ""`、`Set-Location -LiteralPath ''` 都会失败，
/// 而桌面应用本来就与项目目录无关。
pub(crate) fn default_work_dir(start_cmd: &str) -> String {
    let first = start_cmd.split_whitespace().next().unwrap_or("");
    let path = std::path::Path::new(first);
    if path.is_file() {
        if let Some(dir) = path.parent() {
            return dir.to_string_lossy().to_string();
        }
    }
    crate::commands::utils::get_home_dir()
        .to_string_lossy()
        .to_string()
}

/// 轮询代理服务器的 /health 端点，等待代理就绪。
/// 最多重试 50 次（每次 100ms），总计最多 5 秒。返回是否就绪。
/// 把 `dir` 前置到当前进程 PATH 前面，返回给 `Command::env("PATH", ..)` 的值。
///
/// 用途：工具装在 PATH 之外的目录（scoop/choco/curl 安装，或刚装完 PATH 未刷新）时，
/// 让子进程里的裸命令（`cmd /k claude`）能被解析。取不到当前 PATH 时退化为只有该目录。
fn prepend_to_path(dir: &std::path::Path) -> Option<std::ffi::OsString> {
    let mut paths = vec![dir.to_path_buf()];
    if let Some(current) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&current));
    }
    std::env::join_paths(paths).ok()
}

async fn wait_for_proxy_ready(listen_address: &str, port: u16) -> bool {
    let health_url = format!("http://{}:{}/health", listen_address, port);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(500))
        .build()
        .unwrap_or_default();
    for i in 0..50u32 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if client.get(&health_url).send().await.is_ok() {
            eprintln!("[proxy] ✓ 代理就绪 (尝试 {} 次)", i + 1);
            return true;
        }
    }
    eprintln!("[proxy] ⚠ 代理未在 5 秒内就绪");
    false
}

mod proxy_runtime;
pub use proxy_runtime::*;

mod applied_models;
pub use applied_models::*;

mod process_launch;
pub use process_launch::*;

mod model_configuration;
pub use model_configuration::*;

mod config_plan;
pub use config_plan::*;

mod config_formats;
pub use config_formats::*;

#[cfg(test)]
mod tests;
