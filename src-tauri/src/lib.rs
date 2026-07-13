pub mod commands;
pub mod proxy;
mod tray;

use tauri::Manager;

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
                for (managed_id, del) in &config.project_delegations {
                    if del.path_vars.is_empty() {
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
                                commands::project::types::ResolvePattern::FixedPath { path: fixed_path, exe_name }
                                    if p_lower.contains(&fixed_path.to_lowercase()) => {
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
        let delegation = config.project_delegations.get(&def.id);
        for var_def in &def.env_vars {
            let env_managed = delegation.is_some_and(|d| d.env_vars.contains(&var_def.name));
            let should_not_exist = !env_managed || var_def.tier.as_ref().is_some_and(|t| *t == EnvVarTier::Compat);
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
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        // 开机自启：注册时带 `--minimized`，开机自启时静默启动到托盘。
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .manage(commands::http_server::HttpServerState::default())
        .setup(|app| {
            if let Ok(res_dir) = app.path().resource_dir() {
                crate::commands::utils::set_resource_dir(res_dir);
            }
            tray::build_tray(app.handle())?;

            // 窗口在 tauri.conf.json 中设为 visible:false。
            // 普通启动时主动显示；带 `--minimized`（开机自启）时保持隐藏在托盘。
            let start_minimized = std::env::args().any(|a| a == "--minimized");
            if !start_minimized {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.show();
                    let _ = win.set_focus();
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.destroy();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::config::get_config,
            commands::config::update_config,
            commands::config::get_project_menu_config,
            commands::config::update_project_menu_config,
            commands::http_server::start_http_server,
            commands::http_server::stop_http_server,
            commands::http_server::get_running_http_servers,
            commands::img_base64::image_to_base64,
            commands::img_base64::save_base64_image,
            commands::config::delete_old_storage_dirs,
            commands::config::get_app_version,
            commands::config::get_rss_config,
            commands::config::set_rss_sources,
            commands::config::fetch_rss_feed,
            commands::env::toggle_item_management,
            commands::env::get_user_configurable_vars,
            commands::env::set_user_configurable_var,
            commands::env::delete_user_configurable_var,
            commands::env::is_admin,
            commands::env::create_env_backup,
            commands::env::list_env_backups,
            commands::env::delete_env_backup,
            commands::env::restore_env_backup,
            commands::env::get_path_directories,
            commands::env::save_path_directories,
            commands::cache::get_caches_list,
            commands::cache::migrate_cache_path,
            commands::hosts::read_hosts,
            commands::hosts::write_hosts,
            commands::port::check_port_status,
            commands::port::kill_port_owner,
            commands::port::get_reserved_ports,
            commands::pkg::get_global_packages,
            commands::pkg::upgrade_global_package,
            commands::pkg::upgrade_all_global_packages,
            commands::mirror::get_mirrors_list,
            commands::mirror::set_mirror,
            commands::service::get_running_services,
            commands::service::start_service,
            commands::service::stop_service,
            commands::service::force_stop_service,
            commands::service::read_service_config,
            commands::service::write_service_config,
            commands::project::commands::project_list,
            commands::project::commands::project_list_fast,
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
            commands::project::commands::project_set_data_dir,
            commands::project::commands::get_legacy_backup,
            commands::project::versions::project_list_remote_versions,
            commands::project::versions::project_install_version,
            commands::project::versions::project_cancel_install,
            commands::project::versions::project_uninstall_version,
            commands::project::versions::project_use_version,
            tray::refresh_tray_menu,
            commands::conflict::get_conflict_managers_status,
            commands::conflict::handle_conflict_manager_action,
            commands::ai::config::get_ai_config,
            commands::ai::config::save_ai_config,
            commands::ai::provider::fetch_provider_models,
            commands::ai::usage::record_usage,
            commands::ai::usage::get_usage_summary,
            commands::ai::usage::clear_usage,
            commands::ai::skills::get_skills,
            commands::ai::skills::get_skill_overview,
            commands::ai::skills::get_foreign_skills,
            commands::ai::skills::get_discoverable_skills,
            commands::ai::skills::install_skill,
            commands::ai::skills::uninstall_skill,
            commands::ai::skills::toggle_skill_tool,
            commands::ai::skills::register_store_skill,
            commands::ai::skills::get_skill_issues,
            commands::ai::skills::fix_skill_issue,
            commands::ai::skills::fix_all_issues,
            commands::ai::skills::get_skill_files,
            commands::ai::skills::install_skill_from_source,
            commands::ai::config::get_provider_presets,
            commands::ai::detect::detect_ai_tools,
            commands::ai::detect::check_ai_tool_versions,
            commands::ai::skills::scan_existing_skills,
            commands::ai::skills::import_existing_skill,
            commands::ai::skills::get_skill_tools,
            commands::ai::mcp::get_mcp_servers,
            commands::ai::mcp::save_mcp_server,
            commands::ai::mcp::delete_mcp_server,
            commands::ai::mcp::toggle_mcp_tool,
            commands::ai::mcp::get_mcp_tools,
            commands::ai::mcp::get_discovered_mcp,
            commands::ai::mcp::adopt_mcp_server,
            commands::ai::provider::test_model_connection,
            commands::ai::launch::launch_ai_tool,
            commands::ai::config::get_last_launch_config,
            commands::ai::config::get_all_last_launch_configs,
            commands::ai::config::save_last_launch_config,
            commands::ai::sessions::get_ai_sessions,
            commands::ai::sessions::remove_ai_session,
            commands::ai::provider::start_proxy,
            commands::ai::terminal::detect_terminals,
            commands::ai::sessions::scan_tool_sessions,
            commands::ai::collab::collab_create_room,
            commands::ai::collab::collab_list_rooms,
            commands::ai::collab::collab_get_messages,
            commands::ai::collab::collab_delete_room,
            commands::ai::collab::collab_send_message,
            commands::ai::collab::collab_cancel_dispatch,
            commands::ai::collab::collab_reset_session,
            commands::ai::collab::collab_respond_prompt,
            commands::ai::tools::upgrade_ai_tool,
            commands::ai::cache::get_ai_tool_cache_info,
            commands::ai::cache::migrate_ai_tool_cache,
            commands::ai::cache::clean_ai_tool_cache,
            commands::ai::cache::open_ai_tool_cache_dir,
            commands::ai::cache::open_ai_tool_cache_dir_path,
            commands::ai::skills::install_skill_from_online,
            commands::ai::sessions::scan_tool_sessions_parallel,
            commands::ai_registry::reload_ai_registry,
            commands::ai::tool_paths::get_tool_path_override_file,
            commands::tool_version::check_all_tool_versions,
            commands::tool_version::check_tool_version,
            commands::tool_version::upgrade_tool,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_, event| {
            if let tauri::RunEvent::ExitRequested { code, api, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                } else {
                    // 应用退出时清理所有常驻代理
                    commands::ai::collab::stop_all_room_proxies();
                }
            }
        });
}
