//! SDK 路径解析器 — 在用户电脑上定位 SDK 的实际安装位置。
//!
//! 覆盖所有常见的 Windows 安装方式：
//!   - Scoop         (用户级, %USERPROFILE%\scoop\apps\...)
//!   - Chocolatey    (系统级, %ProgramData%\chocolatey\lib\...\tools)
//!   - MSYS2         (C:\msys64\...)
//!   - Cygwin        (C:\cygwin64\...)
//!   - conda         (环境变量 CONDA_PREFIX / 用户目录)
//!   - nvm-windows   (环境变量 NVM_HOME / %APPDATA%\nvm)
//!   - pyenv-win     (环境变量 PYENV_ROOT / %USERPROFILE%\.pyenv)
//!   - Volta         (%LOCALAPPDATA%\Volta\...)
//!   - rustup        (环境变量 RUSTUP_HOME / %USERPROFILE%\.rustup)
//!   - Go            (环境变量 GOPATH / %USERPROFILE%\go\bin)
//!   - winget / 手动安装 (Program Files, 自定义路径等)
//!
//! 规则来源：`projects/<id>/find_rules.json`，与项目定义共用同一套类型
//! （`project::types::{FindRule, ResolvePattern}`）——**不再单独定义一份**，
//! 避免两侧字段名漂移导致配置文件只能被其中一方解析。
//! 本模块的 find_sdk_root() 按优先级依次尝试，返回第一个匹配的结果。

use std::path::{Path, PathBuf};

use crate::commands::project::types::{FindRule, ResolvePattern};

/// SDK 被发现时的结果
#[derive(Debug, Clone)]
pub struct SdkLocation {
    /// SDK 根目录
    pub root: PathBuf,
    /// 来源描述（如 "Scoop", "Chocolatey", "环境变量 JAVA_HOME" 等）
    pub source: String,
}

/// 对某个 SDK 执行路径解析，按优先级返回第一个匹配结果。
pub fn find_sdk_root(_sdk_id: &str, find_rules: &[FindRule]) -> Option<SdkLocation> {
    let links_dir = crate::commands::config::load_config().links_dir;
    let links_lower = links_dir.to_lowercase();

    let mut candidates: Vec<(u8, SdkLocation)> = Vec::new();

    for rule in find_rules {
        let matched_path = match &rule.pattern {
            ResolvePattern::PathContains { path_key, exe_name } => {
                resolve_from_path(path_key, exe_name)
            }
            ResolvePattern::EnvBin { env_var, bin_sub, exe_name } => {
                resolve_from_env_bin(env_var, bin_sub, exe_name)
            }
            ResolvePattern::FixedPath { path, exe_name } => {
                resolve_from_fixed(path, exe_name)
            }
        };

        if let Some(mut path) = matched_path {
            // 跳过 AnyVersion 管理的目录
            if path.to_string_lossy().to_lowercase().contains(&links_lower) {
                continue;
            }

            // 应用 root_offset（向上回溯到根目录）
            for _ in 0..rule.root_offset {
                if let Some(parent) = path.parent() {
                    path = parent.to_path_buf();
                }
            }

            // 检查是否已发现相同根目录（去重）
            let path_str = path.to_string_lossy().to_lowercase();
            if candidates.iter().any(|(_, c)| c.root.to_string_lossy().to_lowercase() == path_str) {
                continue;
            }

            candidates.push((rule.priority, SdkLocation {
                root: path,
                source: rule.source_label.clone(),
            }));
        }
    }

    // 按优先级排序，返回最佳匹配
    candidates.sort_by_key(|(p, _)| *p);
    candidates.into_iter().map(|(_, loc)| loc).next()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  内部解析函数
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 扫描 PATH，查找包含 path_key 的条目，检查 exe 是否存在
fn resolve_from_path(path_key: &str, exe_name: &str) -> Option<PathBuf> {
    let path_key_lower = path_key.to_lowercase();

    // 同时检查用户级和系统级 PATH
    for path_val in get_all_path_values() {
        for entry in std::env::split_paths(&path_val) {
            if entry.as_os_str().is_empty() {
                continue;
            }
            let entry_str = entry.to_string_lossy().to_lowercase();
            if !entry_str.contains(&path_key_lower) {
                continue;
            }
            // 检查 exe 是否在该目录
            if entry.join(exe_name).exists() {
                return Some(entry);
            }
            // 也检查 bin 子目录
            if entry.join("bin").join(exe_name).exists() {
                return Some(entry.join("bin"));
            }
            // 也检查父目录（有时 PATH 指向 bin 子目录）
            if let Some(parent) = entry.parent() {
                if parent.join(exe_name).exists() {
                    return Some(parent.to_path_buf());
                }
            }
        }
    }
    None
}

/// 从环境变量获取根目录，拼接 bin 子路径，检查 exe
fn resolve_from_env_bin(env_var: &str, bin_sub: &str, exe_name: &str) -> Option<PathBuf> {
    let root = crate::commands::env::get_registry_env_any(env_var)?;
    let root_path = Path::new(&root.0);
    let bin_path = if bin_sub.is_empty() {
        root_path.to_path_buf()
    } else {
        root_path.join(bin_sub)
    };

    if bin_path.join(exe_name).exists() {
        Some(bin_path)
    } else if root_path.join(exe_name).exists() {
        Some(root_path.to_path_buf())
    } else {
        None
    }
}

/// 检查固定路径。
///
/// 固定路径允许引用共享目录根（`{program_files}` / `{msys2}` …），
/// 统一走 `utils::expand_home` 展开，避免各项目把 `C:\Program Files`
/// 之类的机器目录重复写死在不同文件里。
fn resolve_from_fixed(fixed: &str, exe_name: &str) -> Option<PathBuf> {
    let expanded = crate::commands::utils::expand_home(fixed);
    let path = Path::new(&expanded);
    if path.join(exe_name).exists() {
        return Some(path.to_path_buf());
    }
    if path.join("bin").join(exe_name).exists() {
        return Some(path.join("bin"));
    }

    None
}

/// 合并用户级和系统级 PATH 的值
fn get_all_path_values() -> Vec<String> {
    let mut result = Vec::new();
    if let Some(val) = crate::commands::env::get_registry_env("PATH") {
        result.push(val);
    }
    if let Some(val) = crate::commands::env::get_system_registry_env("PATH") {
        result.push(val);
    }
    // 也检查当前进程的 PATH（覆盖运行时临时添加的情况）
    if let Ok(val) = std::env::var("PATH") {
        result.push(val);
    }
    result
}
