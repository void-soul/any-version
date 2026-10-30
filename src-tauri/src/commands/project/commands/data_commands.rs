use super::*;

/// - 若无设置命令或旧环境变量在 HKLM（系统级，无法删除），则通过在 HKCU 中写入新路径来实现优先级覆盖；
/// - 若执行命令失败且之前删除了 HKCU 环境变量，则进行恢复/兜底设置。
#[tauri::command]
pub fn project_set_cache_path(
    project_id: String,
    pm_id: String,
    new_path: String,
    extra_cache_id: Option<String>,
) -> Result<(), String> {
    // 防注入：new_path 会替换进 cache_set_cmd_template / 注册表环境变量
    crate::commands::utils::validate_subst_value(&new_path)?;
    if pm_id == "maven" && extra_cache_id.is_none() {
        return set_maven_local_repository(&new_path);
    }

    use crate::commands::env::{get_registry_env_any, set_registry_env};
    use crate::commands::project::registry;

    let def =
        registry::find_by_id(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;

    let pm = def
        .package_managers
        .iter()
        .find(|p| p.id == pm_id)
        .ok_or_else(|| format!("未找到包管理器: {}", pm_id))?;

    // 附加缓存（pnpm.cache 等）：使用附加缓存的 env_var / set_cmd_template
    let extra = match &extra_cache_id {
        Some(extra_id) => Some(
            pm.extra_caches
                .iter()
                .find(|e| &e.id == extra_id)
                .ok_or_else(|| format!("包管理器 {} 中未找到附加缓存: {}", pm_id, extra_id))?,
        ),
        None => None,
    };
    // 指向模式的缓存环境变量与设置命令（附加缓存优先，否则用主缓存）
    let cache_env_var = extra
        .and_then(|e| e.env_var.as_ref())
        .or(extra.is_none().then(|| pm.cache_env_var.as_ref()).flatten());
    let cache_set_cmd_template = extra.and_then(|e| e.set_cmd_template.as_ref()).or(extra
        .is_none()
        .then(|| pm.cache_set_cmd_template.as_ref())
        .flatten());

    let mut env_var_updated = false;
    let mut hkcu_deleted = false;
    let mut deleted_var_name = String::new();
    // 应用启动时可能继承了旧的 PNPM_CONFIG_CACHE_DIR 等环境变量。
    // 当存在明确的设置命令时，必须移除当前进程中的旧值，否则它会覆盖配置文件。
    let mut process_env_removed: Option<(String, String)> = None;

    if let Some(env_var) = cache_env_var {
        if cache_set_cmd_template.is_some() {
            if let Ok(old_value) = std::env::var(env_var) {
                std::env::remove_var(env_var);
                process_env_removed = Some((env_var.clone(), old_value));
            }
        }

        if let Some((_, source)) = get_registry_env_any(env_var) {
            if cache_set_cmd_template.is_none() {
                // 没有命令行设置方式，必须修改注册表环境变量
                set_registry_env(env_var, &new_path)?;
                std::env::set_var(env_var, &new_path);
                env_var_updated = true;
            } else if source == "HKLM" {
                // 如果是系统级环境变量，无法删除，只能在 HKCU 中写入新值以覆盖它
                set_registry_env(env_var, &new_path)?;
                std::env::set_var(env_var, &new_path);
                env_var_updated = true;
            } else {
                // 如果是用户级环境变量 (HKCU)，且有命令可以设置（如 go env -w）
                // 我们优先将其从 HKCU 中删除，以便改为由 commands (go env -w) 托管
                set_registry_env(env_var, "")?;
                std::env::remove_var(env_var);
                hkcu_deleted = true;
                deleted_var_name = env_var.clone();
            }
        } else {
            // 如果注册表中完全没有配置该环境变量
            if cache_set_cmd_template.is_none() {
                // 如果没有命令行模板，则直接通过注册表环境变量来设置
                set_registry_env(env_var, &new_path)?;
                std::env::set_var(env_var, &new_path);
                env_var_updated = true;
            }
        }
    }

    if let Some(ref tpl) = cache_set_cmd_template {
        let cmd = tpl.replace("{path}", &new_path);
        let set_result = if cmd.starts_with("pnpm config set") {
            crate::commands::utils::run_simple_command_checked(&cmd)
        } else {
            run_cmd_capture(cmd, Some(project_id.clone()))
        };
        if let Err(e) = set_result {
            // 如果命令执行失败，恢复本进程原先继承的环境变量。
            if let Some((env_name, old_value)) = process_env_removed.take() {
                std::env::set_var(&env_name, &old_value);
            }
            if hkcu_deleted {
                // 如果之前删除了 HKCU 环境变量，且命令执行失败，则需要恢复/设置 HKCU 环境变量作为兜底
                let _ = set_registry_env(&deleted_var_name, &new_path);
                std::env::set_var(&deleted_var_name, &new_path);
            } else if !env_var_updated {
                // 如果既没删除也没修改过环境变量，则如果有环境变量名，降级/兜底通过修改注册表环境变量来设置它
                //（例如：在 Go 1.13 以下不支持 go env -w 时，直接改环境变量是唯一的设置方式）
                if let Some(env_var) = cache_env_var {
                    set_registry_env(env_var, &new_path)?;
                    std::env::set_var(env_var, &new_path);
                } else {
                    return Err(e);
                }
            }
        }

        // 设置命令成功后立即回读，避免 pnpm 配置写入了另一层或被旧环境变量覆盖。
        if let Some(extra_def) = extra {
            if let Some(ref detect_cmd) = extra_def.detect_cmd {
                let output = if detect_cmd.starts_with("pnpm config get") {
                    crate::commands::utils::run_simple_command_checked(detect_cmd)
                } else {
                    run_cmd_capture(detect_cmd.clone(), Some(project_id.clone()))
                }
                .map_err(|e| format!("设置成功但回读缓存路径失败: {}", e))?;
                let actual = resolve_detected_path(&output, extra_def.detect_json_path.as_deref())
                    .ok_or_else(|| "设置成功但回读不到有效缓存路径".to_string())?;
                if !std::path::Path::new(&actual).eq(std::path::Path::new(&new_path)) {
                    return Err(format!("缓存路径设置未生效，实际路径仍为: {}", actual));
                }
            }
        }
    }

    // 设置完成后，同步更新配置中的缓存路径（主缓存如 NuGet.Config；附加缓存无需写文件）
    if extra.is_none() {
        crate::commands::utils::apply_cache_config_writes(pm, &new_path);
    }

    Ok(())
}

/// - action: "move" 拷贝到新路径后删除旧文件，"delete" 直接删除旧文件，"keep" 不做操作。
#[tauri::command]
pub fn handle_point_storage_files(
    app_handle: tauri::AppHandle,
    old_path: String,
    new_path: String,
    action: String,
) -> Result<(), String> {
    if action == "keep" {
        return Ok(());
    }
    let old = std::path::Path::new(&old_path);
    if !old.exists() {
        return Ok(()); // 旧路径不存在，无需处理
    }
    if action == "delete" {
        std::fs::remove_dir_all(old).map_err(|e| format!("删除旧文件失败: {}", e))?;
        let _ = app_handle.emit(
            "migrate-storage-progress",
            crate::commands::cache::MigrateStorageProgress {
                stage: "已删除旧文件".to_string(),
                current: 1,
                total: 1,
                file_name: String::new(),
            },
        );
    } else if action == "move" {
        crate::commands::cache::move_dir_with_progress(
            &app_handle,
            old,
            std::path::Path::new(&new_path),
        )
        .map_err(|e| e.to_string())?;
        crate::commands::cache::remove_dir_all_forced(old)
            .map_err(|e| format!("删除旧文件失败: {}", e))?;
        let _ = app_handle.emit(
            "migrate-storage-progress",
            crate::commands::cache::MigrateStorageProgress {
                stage: "已完成移动".to_string(),
                current: 1,
                total: 1,
                file_name: String::new(),
            },
        );
    }
    Ok(())
}

/// 清理包管理器缓存 — 只清缓存文件，不跟随 junction，保留目录结构。
#[tauri::command]
pub fn clean_pkg_cache(
    app_handle: tauri::AppHandle,
    project_id: String,
    pm_id: String,
    cache_path: Option<String>,
) -> Result<(), String> {
    let resolved = if let Some(ref cp) = cache_path {
        cp.clone()
    } else {
        resolve_pkg_storage_path_dynamic(&project_id, &pm_id, "cache", None)?
    };
    crate::commands::cache::clean_pkg_cache_impl(&app_handle, &resolved)
}

#[tauri::command]
pub fn migrate_data_dir(
    app_handle: tauri::AppHandle,
    project_id: String,
    orig_path: String,
    new_path: String,
) -> Result<(), String> {
    use std::fs;
    use std::path::Path;

    // 1. 确保服务不在运行中
    let status = scanner::get_project_status(&project_id)?;
    if let Some(svc_status) = status.service_status {
        if svc_status.running {
            return Err("服务正在运行中，请先停止服务后再进行数据迁移。".to_string());
        }
    }

    let orig = Path::new(&orig_path);
    let target = Path::new(&new_path);

    if orig == target {
        return Err("原路径与目标路径相同，无需迁移".to_string());
    }

    // 源目录可能还没生成（如 scoop 的全局安装目录 %ProgramData%\scoop，
    // 只要没装过 -g 软件就一直不存在）：这时没有内容可搬，
    // 把它和目标目录都建出来再挂 Junction —— 否则这类「还没用到的目录」在界面上没有入口可设置。
    let source_existed = crate::commands::cache::ensure_migration_source_dir(orig)?;

    // Ensure target parent folder exists
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("无法创建目标父目录: {}", e))?;
    }

    if !source_existed {
        fs::create_dir_all(target).map_err(|e| format!("无法创建目标目录: {}", e))?;
    } else {
        let is_symlink = fs::symlink_metadata(orig)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);

        if is_symlink {
            // Just remove old junction link
            fs::remove_dir(orig).map_err(|e| format!("无法移除已有的旧链接: {}", e))?;
        } else {
            // Copy directory safely (database data files: delete_old_first = false)
            // We use migrate_pkg_storage_impl with delete_old_first=false, storage_kind="data"
            crate::commands::cache::migrate_pkg_storage_impl(
                &app_handle,
                &orig_path,
                &new_path,
                "data",
                false,
            )?;
        }
    }

    // Create Junction
    crate::commands::cache::create_junction(orig, target)?;

    Ok(())
}

#[tauri::command]
pub fn delete_data_dir(project_id: String, path: String) -> Result<(), String> {
    use std::fs;
    use std::path::Path;

    // 1. 确保服务不在运行中
    let status = scanner::get_project_status(&project_id)?;
    if let Some(svc_status) = status.service_status {
        if svc_status.running {
            return Err("服务正在运行中，请先停止服务后再删除其数据目录。".to_string());
        }
    }

    let p = Path::new(&path);
    if !p.exists() {
        return Err("路径不存在".to_string());
    }

    let is_symlink = fs::symlink_metadata(p)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);

    if is_symlink {
        if let Ok(target) = fs::read_link(p) {
            let _ = fs::remove_dir(p);
            if target.exists() {
                fs::remove_dir_all(&target).map_err(|e| format!("删除链接目标目录失败: {}", e))?;
            }
        } else {
            fs::remove_dir(p).map_err(|e| format!("删除链接失败: {}", e))?;
        }
    } else {
        fs::remove_dir_all(p).map_err(|e| format!("删除数据目录失败: {}", e))?;
    }

    Ok(())
}

#[tauri::command]
pub fn project_set_data_dir(
    project_id: String,
    dir_id: String,
    new_path: String,
) -> Result<(), String> {
    use crate::commands::config::{load_config, save_config};
    use crate::commands::project::registry;

    let def =
        registry::find_by_id(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    if !def.data_dirs.iter().any(|d| d.id == dir_id) {
        return Err(format!("项目 {} 未定义数据目录: {}", project_id, dir_id));
    }

    let status = scanner::get_project_status(&project_id)?;
    if let Some(svc_status) = status.service_status {
        if svc_status.running {
            return Err("服务正在运行中，请先停止服务后再修改数据/日志路径。".to_string());
        }
    }

    let mut config = load_config();
    if new_path.trim().is_empty() {
        if let Some(paths) = config.custom_data_paths.get_mut(&project_id) {
            paths.remove(&dir_id);
            if paths.is_empty() {
                config.custom_data_paths.remove(&project_id);
            }
        }
    } else {
        let p = std::path::Path::new(&new_path);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("无法创建父目录: {}", e))?;
        }
        config
            .custom_data_paths
            .entry(project_id)
            .or_default()
            .insert(dir_id, new_path);
    }

    save_config(&config)
}
#[tauri::command]
pub fn project_set_custom_path(id: String, path: String) -> Result<(), String> {
    use crate::commands::config::{load_config, save_config};
    let mut config = load_config();
    if path.is_empty() {
        config.custom_install_paths.remove(&id);
    } else {
        let p = std::path::Path::new(&path);
        if !p.exists() {
            return Err("指定的路径不存在".to_string());
        }
        config.custom_install_paths.insert(id, path);
    }
    save_config(&config)?;
    Ok(())
}
