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
        let cache_path = if let Some(ref raw_path) = def.cache_default_path {
            expand_home(raw_path)
        } else {
            String::new()
        };
        
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
            if let Some(ref raw_path) = def.cache_default_path {
                let cache_path = expand_home(raw_path);
                if Path::new(&cache_path).exists() {
                    clean_pkg_cache_impl(&app_handle, &cache_path)?;
                }
            }
        }
        "migrate" => {
            let new_path = target_path.ok_or_else(|| "迁移操作缺少 target_path 参数".to_string())?;
            if let Some(ref raw_path) = def.cache_default_path {
                let cache_path = expand_home(raw_path);
                migrate_pkg_storage_impl(
                    &app_handle,
                    &cache_path,
                    &new_path,
                    "cache",
                    false,
                )?;
            } else {
                return Err("该冲突管理器未配置默认缓存路径，无法迁移".to_string());
            }
        }
        "point" => {
            let new_path = target_path.ok_or_else(|| "指向操作缺少 target_path 参数".to_string())?;
            // 将环境变量全部在注册表中重定向到 new_path
            for var in &def.env_vars {
                set_registry_env(var, &new_path)?;
                std::env::set_var(var, &new_path);
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
