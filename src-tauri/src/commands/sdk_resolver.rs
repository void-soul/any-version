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
//! 每种 SDK 在 sdk_registry.rs 中定义一组 FindRule，按优先级排列。
//! 本模块的 find_sdk_root() 按优先级依次尝试，返回第一个匹配的结果。

use std::path::{Path, PathBuf};
use serde::{Serialize, Deserialize};

/// SDK 被发现时的结果
#[derive(Debug, Clone)]
pub struct SdkLocation {
    /// SDK 根目录
    pub root: PathBuf,
    /// 来源描述（如 "Scoop", "Chocolatey", "环境变量 JAVA_HOME" 等）
    pub source: String,
}

/// 安装来源类型（用于 UI 显示和去重）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallSource {
    /// AnyVersion 管理的版本
    AnyVersion,
    /// Scoop 包管理器
    Scoop,
    /// Chocolatey 包管理器
    Chocolatey,
    /// MSYS2 环境
    Msys2,
    /// Cygwin 环境
    Cygwin,
    /// conda / Anaconda / Miniconda
    Conda,
    /// nvm-windows
    Nvm,
    /// pyenv-win
    Pyenv,
    /// Volta
    Volta,
    /// rustup
    Rustup,
    /// Go workspace
    GoPath,
    /// winget
    Winget,
    /// Program Files
    ProgramFiles,
    /// 环境变量直接指向
    EnvVar,
    /// 其他 / 手动安装
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResolvePattern {
    PathContains { keyword: String, exe: String },
    EnvBin { env: String, bin_sub: String, exe: String },
    FixedPath { path: String, exe: String },
    ParentDirPattern { parent_env: String, rel_pattern: String, exe: String },
}

/// 单条解析规则
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindRule {
    /// 规则模式
    pub pattern: ResolvePattern,
    /// 来源标签
    pub source_label: String,
    /// 优先级（越小越优先，0 = 最高）
    pub priority: u8,
    /// 发现后，实际 SDK 根目录相对于匹配路径的向上回溯层数
    pub root_offset: u8,
}

/// 对某个 SDK 执行路径解析，按优先级返回第一个匹配结果。
pub fn find_sdk_root(_sdk_id: &str, find_rules: &[FindRule]) -> Option<SdkLocation> {
    let links_dir = crate::commands::config::load_config().links_dir;
    let links_lower = links_dir.to_lowercase();

    let mut candidates: Vec<(u8, SdkLocation)> = Vec::new();

    for rule in find_rules {
        let matched_path = match &rule.pattern {
            ResolvePattern::PathContains { keyword, exe } => {
                resolve_from_path(keyword, exe)
            }
            ResolvePattern::EnvBin { env, bin_sub, exe } => {
                resolve_from_env_bin(env, bin_sub, exe)
            }
            ResolvePattern::FixedPath { path, exe } => {
                resolve_from_fixed(path, exe)
            }
            ResolvePattern::ParentDirPattern { parent_env, rel_pattern, exe } => {
                resolve_from_parent_dir(parent_env, rel_pattern, exe)
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

/// 枚举某个 SDK 在系统上的所有安装位置（用于"未管理的 SDK"检测）
pub fn find_all_installations(_sdk_id: &str, find_rules: &[FindRule]) -> Vec<SdkLocation> {
    let links_dir = crate::commands::config::load_config().links_dir;
    let links_lower = links_dir.to_lowercase();
    let mut results = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for rule in find_rules {
        let matched_path = match &rule.pattern {
            ResolvePattern::PathContains { keyword, exe } => {
                resolve_from_path(keyword, exe)
            }
            ResolvePattern::EnvBin { env, bin_sub, exe } => {
                resolve_from_env_bin(env, bin_sub, exe)
            }
            ResolvePattern::FixedPath { path, exe } => {
                resolve_from_fixed(path, exe)
            }
            ResolvePattern::ParentDirPattern { parent_env, rel_pattern, exe } => {
                resolve_from_parent_dir(parent_env, rel_pattern, exe)
            }
        };

        if let Some(mut path) = matched_path {
            if path.to_string_lossy().to_lowercase().contains(&links_lower) {
                continue;
            }

            for _ in 0..rule.root_offset {
                if let Some(parent) = path.parent() {
                    path = parent.to_path_buf();
                }
            }

            let key = path.to_string_lossy().to_lowercase();
            if seen.insert(key) {
                results.push(SdkLocation {
                    root: path,
                    source: rule.source_label.clone(),
                });
            }
        }
    }

    results
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

/// 检查固定路径
fn resolve_from_fixed(fixed: &str, exe_name: &str) -> Option<PathBuf> {
    let path = Path::new(fixed);
    if path.join(exe_name).exists() {
        return Some(path.to_path_buf());
    }
    if path.join("bin").join(exe_name).exists() {
        return Some(path.join("bin"));
    }

    // 特殊处理 Qt 目录：扫描 C:\Qt\<version>\<compiler>\bin\<exe_name>
    let fixed_lower = fixed.to_lowercase();
    if path.exists() && (fixed_lower.ends_with("qt") || fixed_lower.ends_with("qt\\") || fixed_lower.ends_with("qt/")) {
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.filter_map(|e| e.ok()) {
                let p1 = entry.path();
                if p1.is_dir() {
                    // 例如 C:\Qt\6.8.3
                    if let Ok(sub_entries) = std::fs::read_dir(p1) {
                        for sub_entry in sub_entries.filter_map(|e| e.ok()) {
                            let p2 = sub_entry.path();
                            if p2.is_dir() {
                                // 例如 C:\Qt\6.8.3\msvc2022_64
                                let bin = p2.join("bin");
                                if bin.join(exe_name).exists() {
                                    return Some(bin);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

/// 从父目录环境变量展开，按模式查找
fn resolve_from_parent_dir(parent_env: &str, rel_pattern: &str, exe_name: &str) -> Option<PathBuf> {
    let parent = crate::commands::env::get_registry_env_any(parent_env)
        .map(|(v, _)| v)
        .unwrap_or_default();
    let parent_path = if parent.is_empty() {
        // 尝试已知的默认位置
        return None;
    } else {
        PathBuf::from(&parent)
    };

    let target = parent_path.join(rel_pattern);
    if target.join(exe_name).exists() {
        Some(target)
    } else {
        None
    }
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
