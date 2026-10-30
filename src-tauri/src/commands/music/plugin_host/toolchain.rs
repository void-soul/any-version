use super::*;

// ─── node / npm 解析（策略：只认裸名 + 用户真实 PATH）───

/// 用户真实 PATH 的分段。
///
/// **刻意不用 `std::env::var("PATH")`**：`lib.rs::sync_process_path` 会把 Kira 托管目录
/// （`data_dir/sdk/*`、`links_dir`）**前置**到本进程 PATH，于是裸名 `node` 会解析到
/// Kira 自己管的那个，而不是用户终端里那个 —— 与「使用用户本机的 node、不与 SDK 模块
/// 交互」直接相悖。读注册表里的用户级 / 系统级 PATH，才是用户终端里的真实顺序。
#[cfg(windows)]
pub(super) fn user_path_entries() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for raw in [
        crate::commands::env::get_registry_env("PATH"),
        crate::commands::env::get_system_registry_env("PATH"),
    ] {
        if let Some(value) = raw {
            out.extend(std::env::split_paths(&value));
        }
    }
    // 注册表读不到（受限环境）时退回进程 PATH：总比完全没有强。
    if out.is_empty() {
        if let Ok(value) = std::env::var("PATH") {
            out.extend(std::env::split_paths(&value));
        }
    }
    out
}

#[cfg(not(windows))]
pub(super) fn user_path_entries() -> Vec<PathBuf> {
    std::env::var("PATH")
        .map(|value| std::env::split_paths(&value).collect())
        .unwrap_or_default()
}

/// 候选文件名。
///
/// Windows 上 `npm` 在 PATH 里通常只有 `npm.cmd` 与 `npm.ps1` —— 而 `.ps1` **不可执行**
/// （CreateProcess 会报 os error 193），所以只认 PE/批处理扩展名。
pub(super) fn candidate_names(base: &str) -> Vec<String> {
    if cfg!(windows) {
        vec![
            format!("{base}.exe"),
            format!("{base}.cmd"),
            format!("{base}.bat"),
            base.to_string(),
        ]
    } else {
        vec![base.to_string()]
    }
}

pub(super) fn find_exe(base: &str) -> Option<PathBuf> {
    for dir in user_path_entries() {
        if dir.as_os_str().is_empty() {
            continue;
        }
        for name in candidate_names(base) {
            let candidate = dir.join(&name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// 一次可执行的调用方式。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    /// 磁盘上真实命中的文件（展示 / 日志用）。
    pub resolved: PathBuf,
    /// 实际交给 `Command::new` 的程序。
    pub program: PathBuf,
    /// 需要插入的前置参数（脚本类要走 `cmd /c`）。
    pub prefix_args: Vec<String>,
}

/// 把命中路径包装成可执行的调用方式。
///
/// Windows 上只有 `.exe` / `.com` 能被 CreateProcess 直接执行；`.cmd` / `.bat` 必须经
/// `cmd.exe /c`，否则报 os error 193（本项目 `node_manager::resolve_exe_invocation`
/// 有同款注释与处理）。
pub(super) fn invocation(resolved: PathBuf) -> Invocation {
    let lower = resolved.to_string_lossy().to_lowercase();
    let needs_shell = cfg!(windows) && !(lower.ends_with(".exe") || lower.ends_with(".com"));
    if needs_shell {
        let cmd = std::env::var("COMSPEC")
            .unwrap_or_else(|_| "C:\\Windows\\System32\\cmd.exe".to_string());
        Invocation {
            program: PathBuf::from(cmd),
            prefix_args: vec!["/c".to_string(), resolved.to_string_lossy().to_string()],
            resolved,
        }
    } else {
        Invocation {
            program: resolved.clone(),
            prefix_args: Vec::new(),
            resolved,
        }
    }
}

/// 本机可用的 node / npm。
#[derive(Debug, Clone)]
pub struct Toolchain {
    pub node: Invocation,
    pub npm: Invocation,
}

pub(super) fn missing_node_hint() -> String {
    "未检测到 Node.js（PATH 中找不到 node/npm）。音乐插件需要本机已安装的 Node.js。\
     请先安装（https://nodejs.org）后重试"
        .to_string()
}

/// 解析 node / npm。两者缺一都算不可用 —— 装依赖要用 npm。
pub fn resolve_toolchain() -> Result<Toolchain, String> {
    let node = find_exe("node").ok_or_else(missing_node_hint)?;
    let npm = find_exe("npm").ok_or_else(missing_node_hint)?;
    Ok(Toolchain {
        node: invocation(node),
        npm: invocation(npm),
    })
}

#[cfg(windows)]
pub(super) fn hide_console(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
pub(super) fn hide_console(_cmd: &mut Command) {}
