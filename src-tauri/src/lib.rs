pub mod commands;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::config::get_config,
            commands::config::update_config,
            commands::env::scan_environment,
            commands::env::resolve_problems,
            commands::env::create_env_backup,
            commands::env::list_env_backups,
            commands::env::delete_env_backup,
            commands::env::restore_env_backup,
            commands::env::toggle_item_management,
            commands::cache::get_caches_list,
            commands::cache::migrate_cache_path,
            commands::hosts::read_hosts,
            commands::hosts::write_hosts,
            commands::port::check_port_status,
            commands::port::kill_port_owner,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
