pub mod commands;

/// 启动时同步注册表中的完整 PATH 到当前进程。
/// 解决 windows_subsystem="windows" 模式下进程 PATH 不含用户 PATH 的问题。
fn sync_process_path() {
    use commands::env::{get_registry_env, get_system_registry_env};
    let mut paths: Vec<String> = Vec::new();
    if let Some(sys) = get_system_registry_env("PATH") {
        paths.push(sys);
    }
    if let Some(user) = get_registry_env("PATH") {
        paths.push(user);
    }
    // 当前进程的 PATH 可能包含 IDE/构建工具注入的路径，排在后面兜底
    if let Ok(current) = std::env::var("PATH") {
        // 只保留不在注册表路径中的部分（去重）
        let reg_paths: std::collections::HashSet<&str> = paths.iter()
            .flat_map(|s| s.split(';'))
            .collect();
        let extra: Vec<&str> = current.split(';')
            .filter(|p| !reg_paths.contains(p))
            .collect();
        if !extra.is_empty() {
            paths.push(extra.join(";"));
        }
    }
    let full_path = paths.join(";");
    std::env::set_var("PATH", &full_path);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    sync_process_path();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::config::get_config,
            commands::config::update_config,
            commands::config::delete_old_storage_dirs,
            commands::config::get_app_version,
            commands::env::scan_environment,
            commands::env::resolve_problems,
            commands::env::create_env_backup,
            commands::env::list_env_backups,
            commands::env::delete_env_backup,
            commands::env::restore_env_backup,
            commands::env::toggle_item_management,
            commands::env::repair_registry_env_types,
            commands::env::get_user_configurable_vars,
            commands::env::set_user_configurable_var,
            commands::env::delete_user_configurable_var,
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
            commands::project::commands::project_unmanage,
            commands::project::commands::project_preview_unmanage,
            commands::project::commands::run_cmd_capture,
            commands::project::commands::get_pkg_cache_info,
            commands::project::commands::migrate_pkg_storage,
            commands::project::commands::handle_point_storage_files,
            commands::project::commands::clean_pkg_cache,
            commands::project::commands::get_legacy_backup,
            commands::project::versions::project_list_remote_versions,
            commands::project::versions::project_install_version,
            commands::project::versions::project_uninstall_version,
            commands::project::versions::project_use_version,
            commands::project::scoop_manifest::update_projects_from_scoop,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
