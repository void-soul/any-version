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
            commands::cache::get_caches_list,
            commands::cache::migrate_cache_path,
            commands::sdk::get_sdks_list,
            commands::sdk::list_remote_versions,
            commands::sdk::get_sdk_download_info,
            commands::sdk::install_sdk_version,
            commands::sdk::query_custom_version,
            commands::sdk::uninstall_sdk_version,
            commands::sdk::use_sdk_version,
            commands::sdk::add_local_sdk_version,
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
            commands::service::stop_service
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
