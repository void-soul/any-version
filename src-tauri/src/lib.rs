pub mod commands;
mod tray;

/// 同步注册表中的完整 PATH 到当前进程，并确保 AnyVersion 托管路径具有最高优先级。
/// 解决 windows_subsystem="windows" 模式下进程 PATH 不包含用户 PATH 的问题。
pub fn sync_process_path() {
    use commands::config::load_config;
    use commands::env::{get_registry_env, get_system_registry_env};

    let config = load_config();
    let links_dir_lower = config.links_dir.to_lowercase();

    let mut all_parts = Vec::new();
    if let Some(sys) = get_system_registry_env("PATH") {
        all_parts.extend(std::env::split_paths(&sys));
    }
    if let Some(user) = get_registry_env("PATH") {
        all_parts.extend(std::env::split_paths(&user));
    }
    if let Ok(current) = std::env::var("PATH") {
        all_parts.extend(std::env::split_paths(&current));
    }

    let mut av_paths = Vec::new();
    let mut other_paths = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for p in all_parts {
        if p.as_os_str().is_empty() {
            continue;
        }
        let p_str = p.to_string_lossy().to_string();
        let p_lower = p_str.to_lowercase();
        if seen.insert(p_lower.clone()) {
            if p_lower.contains(&links_dir_lower) {
                av_paths.push(p);
            } else {
                // 检查是否匹配任何已托管项目的查找规则，若匹配则将其过滤掉以防进程命令劫持（例如过滤掉旧版 D:\tool\go\bin\go.exe）
                let mut matches_managed_rule = false;
                for managed_id in &config.managed_items {
                    if config.simple_managed_items.contains(managed_id) {
                        continue;
                    }
                    if let Some(def) = commands::project::registry::find_by_id(managed_id) {
                        for rule in &def.find_rules {
                            match &rule.pattern {
                                commands::project::types::ResolvePattern::PathContains { path_key, exe_name } => {
                                    if p_lower.contains(&path_key.to_lowercase()) {
                                        let mut check_names = vec![exe_name.clone()];
                                        #[cfg(windows)]
                                        {
                                            let exe_lower = exe_name.to_lowercase();
                                            if !exe_lower.ends_with(".exe") && !exe_lower.ends_with(".cmd") && !exe_lower.ends_with(".bat") {
                                                check_names.push(format!("{}.exe", exe_name));
                                                check_names.push(format!("{}.cmd", exe_name));
                                                check_names.push(format!("{}.bat", exe_name));
                                            }
                                        }
                                        for name in &check_names {
                                            if p.join(name).exists() {
                                                matches_managed_rule = true;
                                                break;
                                            }
                                        }
                                    }
                                }
                                commands::project::types::ResolvePattern::FixedPath { path: fixed_path, exe_name } => {
                                    if p_lower.contains(&fixed_path.to_lowercase()) {
                                        let mut check_names = vec![exe_name.clone()];
                                        #[cfg(windows)]
                                        {
                                            let exe_lower = exe_name.to_lowercase();
                                            if !exe_lower.ends_with(".exe") && !exe_lower.ends_with(".cmd") && !exe_lower.ends_with(".bat") {
                                                check_names.push(format!("{}.exe", exe_name));
                                                check_names.push(format!("{}.cmd", exe_name));
                                                check_names.push(format!("{}.bat", exe_name));
                                            }
                                        }
                                        for name in &check_names {
                                            if p.join(name).exists() {
                                                matches_managed_rule = true;
                                                break;
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                            if matches_managed_rule { break; }
                        }
                    }
                    if matches_managed_rule { break; }
                }

                if !matches_managed_rule {
                    other_paths.push(p);
                }
            }
        }
    }

    let mut final_paths = av_paths;
    final_paths.extend(other_paths);

    if let Ok(full_path) = std::env::join_paths(final_paths) {
        std::env::set_var("PATH", &full_path);
    }
}

/// 启动时清理历史遗留的环境变量。
/// 如果某个项目的环境变量在注册表或当前进程中指向 AnyVersion 的链接目录，
/// 且该项目目前未被托管，或者该变量的级别是 Compat 级别（本不应该被托管设置），则主动将其清理掉。
/// 这能极大避免如全局 PYTHONHOME 导致的 Python 全局崩溃问题。
fn cleanup_legacy_env_vars() {
    use commands::config::load_config;
    use commands::env::{get_registry_env, set_registry_env};
    use commands::project::registry;
    use commands::project::types::EnvVarTier;

    let config = load_config();
    let links_dir_lower = config.links_dir.to_lowercase();
    let defs = registry::registry();

    for def in &defs {
        let is_managed = config.managed_items.contains(&def.id);
        for var_def in &def.env_vars {
            let should_not_exist = !is_managed || var_def.tier.as_ref().map_or(false, |t| *t == EnvVarTier::Compat);
            if should_not_exist {
                if let Some(val) = get_registry_env(&var_def.name) {
                    if val.to_lowercase().contains(&links_dir_lower) {
                        let _ = set_registry_env(&var_def.name, "");
                    }
                }
                if let Ok(val) = std::env::var(&var_def.name) {
                    if val.to_lowercase().contains(&links_dir_lower) {
                        std::env::remove_var(&var_def.name);
                    }
                }
            }
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    cleanup_legacy_env_vars();
    sync_process_path();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            tray::build_tray(app.handle())?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::config::get_config,
            commands::config::update_config,
            commands::config::delete_old_storage_dirs,
            commands::config::get_app_version,
            commands::env::toggle_item_management,
            commands::env::get_user_configurable_vars,
            commands::env::set_user_configurable_var,
            commands::env::delete_user_configurable_var,
            commands::env::create_env_backup,
            commands::env::list_env_backups,
            commands::env::delete_env_backup,
            commands::env::restore_env_backup,
            commands::cache::get_caches_list,
            commands::cache::migrate_cache_path,
            commands::hosts::read_hosts,
            commands::hosts::write_hosts,
            commands::port::check_port_status,
            commands::port::kill_port_owner,
            commands::port::get_reserved_ports,
            commands::pkg::get_global_packages,
            commands::pkg::upgrade_global_package,
            commands::mirror::get_mirrors_list,
            commands::mirror::set_mirror,
            commands::service::get_running_services,
            commands::service::start_service,
            commands::service::stop_service,
            commands::project::commands::project_list,
            commands::project::commands::project_status,
            commands::project::commands::project_detail,
            commands::project::commands::project_preview_manage,
            commands::project::commands::project_manage,
            commands::project::commands::project_repair_env_vars,
            commands::project::commands::project_unmanage,
            commands::project::commands::project_preview_unmanage,
            commands::project::commands::project_set_custom_path,
            commands::project::commands::check_git_repo_status,
            commands::project::commands::bootstrap_git_repo,
            commands::project::commands::update_git_repo,
            commands::project::commands::run_cmd_capture,
            commands::project::commands::get_pkg_cache_info,
            commands::project::commands::migrate_pkg_storage,
            commands::project::commands::project_set_cache_path,
            commands::project::commands::handle_point_storage_files,
            commands::project::commands::clean_pkg_cache,
            commands::project::commands::migrate_data_dir,
            commands::project::commands::delete_data_dir,
            commands::project::commands::get_legacy_backup,
            commands::project::versions::project_list_remote_versions,
            commands::project::versions::project_install_version,
            commands::project::versions::project_cancel_install,
            commands::project::versions::project_uninstall_version,
            commands::project::versions::project_use_version,
            tray::refresh_tray_menu,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
