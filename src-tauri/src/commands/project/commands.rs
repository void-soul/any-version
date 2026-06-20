//! 项目管理模块 — Tauri 命令层。
//!
//! 暴露给前端的所有 project_* 命令，内部调用 scanner 模块执行实际逻辑。

use super::types::{ProjectStatus, ProjectDetail, ManagePreview};
use super::scanner;

/// 获取所有项目列表及运行时状态
#[tauri::command]
pub fn project_list() -> Result<Vec<ProjectStatus>, String> {
    scanner::list_projects()
}

/// 获取单个项目运行时状态
#[tauri::command]
pub fn project_status(id: String) -> Result<ProjectStatus, String> {
    scanner::get_project_status(&id)
}

/// 获取项目详情（定义 + 状态）
#[tauri::command]
pub fn project_detail(id: String) -> Result<ProjectDetail, String> {
    scanner::get_project_detail(&id)
}

/// 预览托管操作步骤
#[tauri::command]
pub fn project_preview_manage(id: String) -> Result<ManagePreview, String> {
    scanner::preview_manage(&id)
}

/// 托管项目（将项目纳入 AnyVersion 管理）
///
/// 执行步骤：
/// 1. 备份当前环境变量
/// 2. 清理 PATH 中的外部条目
/// 3. 设置项目环境变量（指向 AnyVersion 管理目录）
/// 4. 添加 bin 路径到 PATH
/// 5. 将项目 ID 添加到 managed_items
/// 6. 保存配置
#[tauri::command]
pub fn project_manage(id: String) -> Result<(), String> {
    use crate::commands::config::load_config;
    use crate::commands::env::{get_registry_env_any, set_registry_env, add_to_user_path};
    use super::registry;

    let def = registry::find_by_id(&id)
        .ok_or_else(|| format!("未找到项目: {}", id))?;

    let mut config = load_config();

    // 如果已托管，直接返回
    if config.managed_items.contains(id.as_str()) {
        return Ok(());
    }

    // 1. 备份当前环境变量
    for var_def in &def.env_vars {
        if let Some((val, _)) = get_registry_env_any(&var_def.name) {
            if !val.to_lowercase().contains(&config.links_dir.to_lowercase()) {
                config.original_envs.entry(var_def.name.clone()).or_insert(val);
            }
        }
    }

    // 2. 清理 PATH 中的外部条目
    use std::path::Path;
    if let Some(user_path) = crate::commands::env::get_registry_env("PATH") {
        let parts = std::env::split_paths(&user_path)
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        let mut matched_entries = Vec::new();
        let mut remaining_entries = Vec::new();

        for p_str in parts {
            if p_str.is_empty() {
                continue;
            }
            let p_lower = p_str.to_lowercase();

            if !p_lower.contains(&config.links_dir.to_lowercase()) {
                let mut matches = false;
                for rule in &def.find_rules {
                    match &rule.pattern {
                        super::types::ResolvePattern::PathContains { path_key, .. } => {
                            if p_lower.contains(&path_key.to_lowercase()) {
                                matches = true;
                                break;
                            }
                        }
                        super::types::ResolvePattern::FixedPath { path: fixed_path, .. } => {
                            if p_lower.contains(&fixed_path.to_lowercase()) {
                                matches = true;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                if matches {
                    matched_entries.push(p_str.clone());
                    continue;
                }
            }
            remaining_entries.push(p_str);
        }

        if !matched_entries.is_empty() {
            config.original_paths.entry(id.clone()).or_insert_with(Vec::new).extend(matched_entries);

            let new_path = std::env::join_paths(remaining_entries.iter().map(Path::new))
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .to_string();
            set_registry_env("PATH", &new_path)?;
        }
    }

    // 3. 添加到 managed_items
    config.managed_items.insert(id.clone());
    crate::commands::config::save_config(&config)?;

    // 4. 设置项目环境变量
    let link_dir = Path::new(&config.links_dir).join(&id);
    let link_str = link_dir.to_string_lossy().to_string();
    for var_def in &def.env_vars {
        let value = match var_def.name.as_str() {
            "CARGO_HOME"       => format!("{}\\.cargo", link_str),
            "RUSTUP_HOME"      => format!("{}\\.rustup", link_str),
            "ANDROID_SDK_HOME" => format!("{}\\.android", link_str),
            "NPM_CONFIG_PREFIX" => format!("{}\\node_modules", link_str),
            "PGDATA"           => format!("{}\\data", link_str),
            _                  => link_str.clone(),
        };
        let _ = set_registry_env(&var_def.name, &value);
    }

    // 5. 添加 bin 路径到 PATH
    let bin_paths = scanner::get_bin_paths(&def.id, &link_str);
    if !bin_paths.is_empty() {
        let _ = add_to_user_path(&bin_paths);
    }

    Ok(())
}

/// 取消托管项目（从 AnyVersion 管理中移除）
///
/// 执行步骤：
/// 1. 移除项目环境变量
/// 2. 从 PATH 中移除 AnyVersion 添加的条目
/// 3. 还原原始环境变量
/// 4. 还原原始 PATH 条目
/// 5. 从 managed_items 中移除
/// 6. 保存配置
#[tauri::command]
pub fn project_unmanage(id: String) -> Result<(), String> {
    use crate::commands::config::load_config;
    use crate::commands::env::{set_registry_env, remove_from_user_path, add_to_user_path};
    use super::registry;

    let def = registry::find_by_id(&id)
        .ok_or_else(|| format!("未找到项目: {}", id))?;

    let mut config = load_config();

    // 如果未托管，直接返回
    if !config.managed_items.contains(id.as_str()) {
        return Ok(());
    }

    // 1. 移除项目环境变量
    for var_def in &def.env_vars {
        let _ = set_registry_env(&var_def.name, "");
    }

    // 2. 从 PATH 中移除 AnyVersion 添加的条目
    let link_dir = std::path::Path::new(&config.links_dir).join(&id);
    let link_str = link_dir.to_string_lossy().to_string();
    let bin_paths = scanner::get_bin_paths(&def.id, &link_str);
    if !bin_paths.is_empty() {
        let _ = remove_from_user_path(&bin_paths);
    }

    // 3. 还原原始环境变量
    for var_def in &def.env_vars {
        if let Some(orig_val) = config.original_envs.remove(&var_def.name) {
            let _ = set_registry_env(&var_def.name, &orig_val);
        }
    }

    // 4. 还原原始 PATH 条目
    if let Some(orig_paths) = config.original_paths.remove(&id) {
        let _ = add_to_user_path(&orig_paths);
    }

    // 5. 从 managed_items 中移除
    config.managed_items.remove(&id);
    crate::commands::config::save_config(&config)?;

    Ok(())
}

/// 执行 shell 命令并捕获输出（用于包管理器版本检测、镜像切换等）
#[tauri::command]
pub fn run_cmd_capture(cmd: String) -> Result<String, String> {
    let output = super::super::hidden_cmd::hidden_cmd("cmd")
        .args(&["/c", &cmd])
        .output()
        .map_err(|e| format!("执行命令失败: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if output.status.success() {
        Ok(stdout)
    } else if !stderr.is_empty() {
        Err(stderr)
    } else {
        Err(format!("命令执行失败 (exit code: {})", output.status.code().unwrap_or(-1)))
    }
}

/// 获取包管理器缓存信息
#[derive(serde::Serialize, Clone, Debug)]
pub struct PkgCacheInfo {
    pub path: String,
    pub size: String,
    pub is_link: bool,
    pub real_target: String,
}

#[tauri::command]
pub fn get_pkg_cache_info(cmd: String) -> Result<PkgCacheInfo, String> {
    let cache_path = run_cmd_capture(cmd)?;
    if cache_path.is_empty() {
        return Err("缓存路径为空".to_string());
    }

    let path = std::path::Path::new(&cache_path);
    let (is_link, real_target) = if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            let target = std::fs::read_link(path)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            (true, target)
        } else {
            (false, String::new())
        }
    } else {
        (false, String::new())
    };

    let size = crate::commands::cache::get_dir_size(path);
    let size_str = crate::commands::cache::format_bytes(size);

    Ok(PkgCacheInfo {
        path: cache_path,
        size: size_str,
        is_link,
        real_target,
    })
}

/// 迁移包管理器缓存目录
#[tauri::command]
pub fn migrate_pkg_cache(cache_detect_cmd: String, new_path: String) -> Result<(), String> {
    let cache_path = run_cmd_capture(cache_detect_cmd)?;
    if cache_path.is_empty() {
        return Err("缓存路径为空".to_string());
    }
    crate::commands::cache::migrate_cache_path_raw(&cache_path, &new_path)
}
