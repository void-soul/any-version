use std::collections::HashMap;
use std::path::Path;
use crate::commands::project::types::ConflictManagerStatus;
use crate::commands::project::registry;
use crate::commands::utils::{expand_home, is_exe_in_path};
use crate::commands::env::{get_registry_env_any, set_registry_env, set_system_registry_env, broadcast_setting_change, get_registry_env, get_system_registry_env};
use crate::commands::cache::{get_dir_size, format_bytes, clean_pkg_cache_impl, migrate_pkg_storage_impl};

#[tauri::command]
pub fn get_conflict_managers_status(sdk_id: String) -> Result<Vec<ConflictManagerStatus>, String> {
    let project = registry::find_by_id(&sdk_id)
        .ok_or_else(|| format!("未找到项目: {}", sdk_id))?;
        
    let mut status_list = Vec::new();
    
    for def in &project.conflict_managers {
        // 1. 判断是否安装：检查可执行文件是否存在于 PATH
        let installed = if let Some(ref exe) = def.exe_name {
            is_exe_in_path(exe)
        } else {
            false
        };
        
        // 2. 获取环境变量状态：遍历 env_vars
        let mut env_vars_status = HashMap::new();
        for var in &def.env_vars {
            let val = get_registry_env_any(var).map(|(v, _)| v);
            env_vars_status.insert(var.clone(), val);
        }
        
        // 3. 获取 PATH 匹配状态
        let mut path_status = Vec::new();
        let mut check_path = |path_str: String| {
            for part in std::env::split_paths(&path_str) {
                let part_str = part.to_string_lossy().to_string();
                let part_lower = part_str.to_lowercase();
                for keyword in &def.path_keywords {
                    if part_lower.contains(&keyword.to_lowercase()) {
                        if !path_status.contains(&part_str) {
                            path_status.push(part_str.clone());
                        }
                    }
                }
            }
        };
        if let Some(user_path) = get_registry_env("PATH") {
            check_path(user_path);
        }
        if let Some(sys_path) = get_system_registry_env("PATH") {
            check_path(sys_path);
        }
        
        // 4. 获取缓存目录与空间大小
        // 以「缓存位置对应的环境变量」为唯一真源（如 rustup 的 RUSTUP_HOME、
        // nvm 的 NVM_HOME），当前值优先取自注册表；未配置时才回退到默认路径
        // cache_default_path（如 {home}\.rustup）。避免环境变量区与缓存路径区脱节。
        let mut cache_path = String::new();
        if let Some(ref env_var) = def.cache_env_var {
            if let Some(val) = env_vars_status.get(env_var).cloned().flatten() {
                if !val.trim().is_empty() {
                    cache_path = val;
                }
            }
        }
        if cache_path.is_empty() {
            if let Some(ref raw_path) = def.cache_default_path {
                cache_path = expand_home(raw_path);
            }
        }
        
        let cache_size = if !cache_path.is_empty() && Path::new(&cache_path).exists() {
            let size = get_dir_size(Path::new(&cache_path));
            format_bytes(size)
        } else {
            "0 B".to_string()
        };
        
        // 5. 判断是否已被禁用：
        // 所有的环境变量要么为 None，要么为空，且在 PATH 中没有匹配到的路径项
        let all_env_empty = env_vars_status.values().all(|v| v.as_ref().map_or(true, |val| val.is_empty()));
        let path_empty = path_status.is_empty();
        let is_disabled = all_env_empty && path_empty;
        
        status_list.push(ConflictManagerStatus {
            id: def.id.clone(),
            display_name: def.display_name.clone(),
            installed,
            env_vars_status,
            path_status,
            cache_path,
            cache_size,
            is_disabled,
        });
    }
    
    Ok(status_list)
}

/// 解析冲突管理器的当前缓存路径：
/// 优先使用缓存位置对应环境变量（cache_env_var，如 RUSTUP_HOME）的注册表当前值，
/// 未配置时回退到 cache_default_path（如 {home}\.rustup）。
/// 保证缓存路径区与「修复环境变量」校准的环境变量值保持一致（唯一真源）。
fn resolve_conflict_cache_path(def: &crate::commands::project::types::ConflictManagerDef) -> String {
    if let Some(ref env_var) = def.cache_env_var {
        if let Some((val, _)) = get_registry_env_any(env_var) {
            if !val.trim().is_empty() {
                return val;
            }
        }
    }
    if let Some(ref raw_path) = def.cache_default_path {
        return expand_home(raw_path);
    }
    String::new()
}

#[tauri::command]
pub fn handle_conflict_manager_action(
    app_handle: tauri::AppHandle,
    sdk_id: String,
    manager_id: String,
    action: String,
    target_path: Option<String>,
) -> Result<(), String> {
    let project = registry::find_by_id(&sdk_id)
        .ok_or_else(|| format!("未找到项目: {}", sdk_id))?;
        
    let def = project.conflict_managers.iter().find(|m| m.id == manager_id)
        .ok_or_else(|| format!("在项目 {} 下未找到冲突管理器: {}", sdk_id, manager_id))?;
        
    match action.as_str() {
        "clean" => {
            let cache_path = resolve_conflict_cache_path(def);
            if !cache_path.is_empty() && Path::new(&cache_path).exists() {
                clean_pkg_cache_impl(&app_handle, &cache_path)?;
            }
        }
        "migrate" => {
            let new_path = target_path.ok_or_else(|| "迁移操作缺少 target_path 参数".to_string())?;
            let cache_path = resolve_conflict_cache_path(def);
            if cache_path.is_empty() {
                return Err("该冲突管理器未配置缓存路径，无法迁移".to_string());
            }
            migrate_pkg_storage_impl(
                &app_handle,
                &cache_path,
                &new_path,
                "cache",
                false,
            )?;
            // 迁移（Junction 模式）后，把缓存位置环境变量同步到新路径，
            // 使「修复环境变量」之外的路由（环境变量区）也能感知实际缓存位置。
            // 注意：migrate 会先在原路径建 junction 指向 new_path，故把环境变量指向
            // 实际目录 new_path（而非 junction 的原路径），避免环境变量区与实际存储脱节。
            if let Some(ref env_var) = def.cache_env_var {
                set_registry_env(env_var, &new_path)?;
                std::env::set_var(env_var, &new_path);
            }
            broadcast_setting_change();
        }
        "point" => {
            let new_path = target_path.ok_or_else(|| "指向操作缺少 target_path 参数".to_string())?;
            // 仅将缓存位置相关的环境变量重定向到 new_path，而不是全部 env_vars。
            // 例如 rustup 的 env_vars 含 RUSTUP_HOME 与 RUSTUP_TOOLCHAIN，
            // 后者是工具链名而非目录路径，若被写入目录路径会导致 rustup 异常。
            let loc_var = def.cache_env_var.clone()
                .or_else(|| def.env_vars.first().cloned());
            if let Some(var) = loc_var {
                set_registry_env(&var, &new_path)?;
                std::env::set_var(&var, &new_path);
            }
            broadcast_setting_change();
        }
        "disable" => {
            // 1. 擦除环境变量
            for var in &def.env_vars {
                let _ = set_registry_env(var, "");
                std::env::remove_var(var);
                // 尝试系统级清理
                let _ = set_system_registry_env(var, "");
            }
            // 2. 清洗 PATH 变量
            disable_manager_in_path(&def.path_keywords)?;
            broadcast_setting_change();
        }
        _ => return Err(format!("未知的操作类型: {}", action)),
    }
    
    Ok(())
}

fn disable_manager_in_path(path_keywords: &[String]) -> Result<(), String> {
    // 1. 用户级 PATH
    if let Some(user_path) = get_registry_env("PATH") {
        let parts = std::env::split_paths(&user_path)
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>();
            
        let mut new_parts = Vec::new();
        let mut modified = false;
        
        for part in parts {
            let part_lower = part.to_lowercase();
            let mut matches_keyword = false;
            for keyword in path_keywords {
                if part_lower.contains(&keyword.to_lowercase()) {
                    matches_keyword = true;
                    break;
                }
            }
            if matches_keyword {
                modified = true;
            } else {
                new_parts.push(part);
            }
        }
        
        if modified {
            let new_path_str = std::env::join_paths(new_parts.iter().map(std::path::Path::new))
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .to_string();
            set_registry_env("PATH", &new_path_str)?;
        }
    }
    
    // 2. 系统级 PATH
    if let Some(sys_path) = get_system_registry_env("PATH") {
        let parts = std::env::split_paths(&sys_path)
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>();
            
        let mut new_parts = Vec::new();
        let mut modified = false;
        
        for part in parts {
            let part_lower = part.to_lowercase();
            let mut matches_keyword = false;
            for keyword in path_keywords {
                if part_lower.contains(&keyword.to_lowercase()) {
                    matches_keyword = true;
                    break;
                }
            }
            if matches_keyword {
                modified = true;
            } else {
                new_parts.push(part);
            }
        }
        
        if modified {
            let new_path_str = std::env::join_paths(new_parts.iter().map(std::path::Path::new))
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .to_string();
            let _ = set_system_registry_env("PATH", &new_path_str);
        }
    }
    
    Ok(())
}
