pub mod commands;
pub mod proxy;
mod tray;
pub mod exit_log;

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Manager, Emitter, Listener};

/// 用户通过托盘「退出」主动请求退出。置位后，即便 Tauri 在窗口销毁流程中
/// 发出 code=None 的 ExitRequested，也不应拦截真正的退出意图。
pub static USER_QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Windows 防 FOUC：标记主页面首次 PageLoadEvent::Finished 是否已处理（仅触发一次 show）。
#[cfg(target_os = "windows")]
pub static STARTUP_PAGE_HANDLED: AtomicBool = AtomicBool::new(false);

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
    exit_log::exit_log("=== app 启动 run() ===");
    cleanup_legacy_env_vars();
    sync_process_path();

    let builder = tauri::Builder::default();

    // Windows 防 FOUC：原生窗口层先暗（tauri.conf backgroundColor），
    // 主窗口在 tauri.conf 中 visible:false，待主页面 PageLoadEvent::Finished
    // 后再 show，避免 WebView2 白底先绘制导致的白黑闪屏（移植自 cc-switch #6252）。
    #[cfg(target_os = "windows")]
    let builder = builder.on_page_load(move |webview, payload| {
        if webview.label() == "main"
            && payload.event() == tauri::webview::PageLoadEvent::Finished
            && payload.url().scheme() != "about"
            && !STARTUP_PAGE_HANDLED.swap(true, Ordering::Relaxed)
            && !std::env::args().any(|a| a == "--minimized")
        {
            let app = webview.app_handle();
            if let Some(window) = app.get_webview_window("main") {
                crate::tray::focus_main_window(&window);
                exit_log::exit_log("主页面加载完成，主窗口已显示（防 FOUC）");
            }
        }
    });

    // 单一实例约束：仅在非开发环境生效，便于开发时并行启动多个实例调试。
    // debug_assertions 在 `tauri dev`（debug 构建）下为真，release 构建为假。
    // 注意：使用影子绑定（shadowing）而非 `let mut` + 重复赋值，
    // 否则 debug 构建下单一实例插件分支被排除后 builder 未初始化，
    // 会导致 `builder.plugin(...)` 接收未初始化值而被推断为 `()`，编译失败。
    #[cfg(not(debug_assertions))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        // 主窗口可能被销毁（见 on_window_event 的 CloseRequested 处理），
        // 因此优先复用，缺失时经 tray::show_main_window 重建。
        if app.get_webview_window("main").is_some() {
            if let Some(window) = app.get_webview_window("main") {
                crate::tray::focus_main_window(&window);
            }
        } else {
            crate::tray::show_main_window(app);
        }
    }));
    #[cfg(debug_assertions)]
    let builder = builder;

    builder
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
        .manage(commands::cert::CertScheduler::default())
        .manage(commands::rtsp_server::RtspServerState::default())
        .setup(|app| {
            if let Ok(res_dir) = app.path().resource_dir() {
                crate::commands::utils::set_resource_dir(res_dir);
            }
            tray::build_tray(app.handle())?;
            // 接收前端在切换顶级模块时上报的当前页面，供模块专属热键做「显示/隐藏」切换判定
            {
                let h = app.handle().clone();
                h.listen("launcher-active-page", move |event| {
                    if let Ok(page) = serde_json::from_str::<String>(event.payload()) {
                        crate::commands::launcher::windows::set_current_page(&page);
                    }
                });
            }
            // 启动时兜底重建 SDK 锚点 junction（修复迁移后遗留的普通空目录）
            {
                let config = crate::commands::config::load_config();
                let rebuilt = crate::commands::config::rebuild_sdk_junctions(&config);
                if !rebuilt.is_empty() {
                    exit_log::exit_log(&format!(
                        "[startup] 重建 SDK 锚点 junction: {:?}", rebuilt
                    ));
                }
            }
            commands::cert::start_scheduler(app.handle().clone());
            let mihomo_state = commands::mihomo::init_state();
            app.manage(mihomo_state.clone());
            commands::mihomo::start_scheduler(app.handle().clone(), mihomo_state.clone());

            // 统一服务自启编排 (Mihomo 代理 / RTSP 流媒体 / SDK 数据库与中间件服务)
            let h = app.handle().clone();
            let s_mihomo = mihomo_state.clone();
            let config_clone = crate::commands::config::load_config();
            tauri::async_runtime::spawn(async move {
                let auto_start_services = config_clone.auto_start_services.clone();

                // 1. Mihomo 代理自启 (兼顾 auto_start_core 与 auto_start_services.contains("mihomo"))
                let mihomo_enabled = auto_start_services.contains("mihomo")
                    || s_mihomo.app_config.lock().map(|c| c.auto_start_core).unwrap_or(false);
                if mihomo_enabled {
                    exit_log::exit_log("[autostart] 正在自启 Mihomo 代理服务...");
                    let _ = commands::mihomo::launch_core(&h, s_mihomo).await;
                }

                // 2. RTSP 推流服务自启
                if auto_start_services.contains("rtsp") {
                    if let Some(rtsp_state) = h.try_state::<commands::rtsp_server::RtspServerState>() {
                        exit_log::exit_log("[autostart] 正在自启 RTSP 推流服务...");
                        let rtsp_config: commands::rtsp_server::RtspConfig = config_clone
                            .last_servers
                            .rtsp
                            .and_then(|v| serde_json::from_value(v).ok())
                            .unwrap_or_else(|| commands::rtsp_server::RtspConfig {
                                id: Some("rtsp-auto".to_string()),
                                source_type: "testsrc".to_string(),
                                camera_name: None,
                                file_path: None,
                                port: 8554,
                                path_name: "live".to_string(),
                                allow_lan: false,
                                loop_file: true,
                                include_audio: false,
                                audio_device: None,
                                resolution: Some("1280x720".to_string()),
                                fps: Some(30),
                                transport: Some("tcp".to_string()),
                                video_codec: Some("h264".to_string()),
                                gpu_accel: Some("cpu".to_string()),
                            });
                        let _ = commands::rtsp_server::start_rtsp_server(h.clone(), rtsp_state, rtsp_config);
                    }
                }

                // 3. SDK 后台服务自启 (MySQL / Redis / MongoDB / PostgreSQL / Nginx / FRPC / FRPS 等)
                for svc_id in &auto_start_services {
                    if svc_id == "mihomo" || svc_id == "rtsp" {
                        continue;
                    }
                    // 自动启动前先判断服务是否已开启：已在运行，或端口已被占用（无论是否本实例
                    // 进程），都视为"服务已开启/在跑"，跳过自启，避免"启动时才报错"。
                    let already_running = crate::commands::project::registry::find_by_id(svc_id)
                        .map(|def| {
                            let st = crate::commands::service::service_status_for_def(&def);
                            st.running
                                || st.status.as_deref() == Some("port_conflict")
                        })
                        .unwrap_or(false);
                    if already_running {
                        exit_log::exit_log(&format!("[autostart] 服务 {} 已在运行/端口被占用，跳过自启", svc_id));
                        continue;
                    }
                    exit_log::exit_log(&format!("[autostart] 正在自启 SDK 服务: {}", svc_id));
                    if let Err(e) = commands::service::start_service_inner(svc_id.clone(), None) {
                        exit_log::exit_log(&format!("[autostart] 自启服务 {} 失败: {}", svc_id, e));
                    } else {
                        exit_log::exit_log(&format!("[autostart] 自启服务 {} 成功", svc_id));
                        let _ = h.emit("service-status-changed", svc_id);
                    }
                }

                // 刷新托盘菜单
                let _ = crate::tray::rebuild_tray_menu(&h);
            });

            // 初始化启动器数据库，并注册「唤起/隐藏主窗口」全局快捷键。
            // 注意：该快捷键只负责切换窗口显示状态，不拦截普通输入框按键。
            let _ = commands::launcher::db::init_db();
            // 剪贴板监控（启动器热键注册之后，便于复用其热键线程记录前台窗口）
            let _ = commands::clipboard::init_clipboard_state(app.handle());
            if let Ok(setting) = commands::launcher::db::get_settings() {
                let _ = commands::launcher::windows::register_global_hotkeys(
                    app.handle().clone(),
                    &setting.module_hotkeys,
                );
            }

            // 窗口在 tauri.conf.json 中设为 visible:false。
            // 带 `--minimized`（开机自启）时保持隐藏在托盘。
            // 普通启动：Windows 下交给 on_page_load（页面 Finished 后）再显示主窗口，
            // 以消除 WebView2 白底先绘制导致的闪屏（防 FOUC）；其余平台此处直接显示。
            let start_minimized = std::env::args().any(|a| a == "--minimized");
            #[cfg(not(target_os = "windows"))]
            if !start_minimized {
                if let Some(win) = app.get_webview_window("main") {
                    crate::tray::focus_main_window(&win);
                }
            }
            #[cfg(target_os = "windows")]
            if start_minimized {
                exit_log::exit_log("防 FOUC：--minimized 自启，主窗口保持隐藏");
            } else {
                exit_log::exit_log("防 FOUC：Windows 普通启动，等待主页面加载完成后显示主窗口");
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    // 关闭主窗口时「隐藏」而非「销毁」。
                    // 历史（1.0.9 dc94e6d）曾改为 destroy() 以规避 WebView2 高负载
                    // 长时运行后白屏/无法退出的问题，但实测 destroy() 会破坏 Windows
                    // 托盘原生菜单的事件投递：主窗口销毁后 on_menu_event 不再触发，
                    // 表现为托盘菜单能弹出、点击却无响应（且重建窗口后仍不恢复）。
                    // 因此回退为 hide()，保留窗口对象，确保托盘菜单事件正常。
                    // 白屏问题的兜底：show_main_window 在检测到 WebView2 挂起时会
                    // 尝试 reload；如需彻底规避可后续引入「定期重建窗口」机制。
                    api.prevent_close();
                    exit_log::exit_log("CloseRequested: hide 主窗口（保留窗口对象，保证托盘菜单事件正常）");
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::config::get_config,
            commands::config::update_config,
            commands::config::get_data_dir_cmd,
            commands::config::get_sdk_dir_cmd,
            commands::config::get_auto_start_services,
            commands::config::set_auto_start_service,
            commands::config::get_appearance_config,
            commands::config::set_module_theme_color,
            commands::config::set_global_font,
            commands::config::set_module_order,
            commands::config::set_module_layout,
            commands::config::import_custom_font,
            commands::config::clear_custom_font,
            commands::config::list_system_fonts,
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
            commands::ai::skills::get_skill_overview,
            commands::ai::skills::get_skill_meta,
            commands::ai::skills::update_skill_meta,
            commands::ai::skills::get_skill_tools_status,
            commands::ai::skills::toggle_tool_symlink_setting,
            commands::ai::skills::manage_tool_skills,
            commands::ai::skills::unmanage_tool_skills,
            commands::ai::skills::manage_all_tool_skills,
            commands::ai::skills::install_skill,
            commands::ai::skills::uninstall_skill,
            commands::ai::skills::get_skill_files,
            commands::ai::skills::install_skill_from_source,
            commands::ai::skills::migrate_legacy_skills,
            commands::ai::config::get_provider_presets,
            commands::ai::detect::detect_ai_tools,
            commands::ai::detect::check_ai_tool_versions,

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
            commands::ai::collab::collab_compact_session,
            commands::ai::collab::collab_get_snapshot,
            commands::ai::collab::collab_get_turns,
            commands::ai::collab::collab_get_agents,
            commands::ai::collab::collab_list_tasks,
            commands::ai::collab::collab_task_action,
            commands::ai::tools::install_ai_tool,
            commands::ai::tools::upgrade_ai_tool,
            commands::ai::tools::uninstall_ai_tool,
            commands::ai::cache::get_ai_tool_cache_info,
            commands::ai::cache::migrate_ai_tool_cache,
            commands::ai::cache::clean_ai_tool_cache,
            commands::ai::cache::open_ai_tool_cache_dir,
            commands::ai::cache::open_ai_tool_cache_dir_path,
            commands::ai::skills::install_skill_from_online,
            commands::ai::sessions::scan_tool_sessions_parallel,
            commands::ai_registry::reload_ai_registry,
            commands::ai_registry::update_tool_profile,
            commands::ai::tool_paths::get_tool_path_override_file,
            commands::tool_version::check_all_tool_versions,
            commands::tool_version::check_tool_version,
            commands::tool_version::upgrade_tool,

            // 证书管理（Q-005）
            commands::cert::cert_list,
            commands::cert::cert_create,
            commands::cert::cert_update_nodes,
            commands::cert::cert_delete,
            commands::cert::cert_issue_now,
            commands::cert::cert_get_pem,
            commands::cert::deploy_node_list,
            commands::cert::deploy_node_upsert,
            commands::cert::deploy_node_delete,
            commands::cert::deploy_node_test,
            commands::cert::credential_list,
            commands::cert::credential_upsert,
            commands::cert::credential_delete,
            commands::cert::cert_scheduler_status,
            commands::cert::cert_scheduler_set,
            commands::cert::cert_scheduler_run_now,

            // RTSP 服务器
            commands::rtsp_server::get_rtsp_camera_devices,
            commands::rtsp_server::start_rtsp_server,
            commands::rtsp_server::stop_rtsp_server,
            commands::rtsp_server::stop_all_rtsp_servers,
            commands::rtsp_server::get_rtsp_server_status,
            commands::rtsp_server::get_all_rtsp_server_statuses,
            commands::rtsp_server::get_all_local_ips,
            commands::file_io::read_text_file,
            commands::file_io::write_text_file,
            commands::file_io::list_sibling_markdown,
            commands::file_io::resolve_markdown_link,
            commands::utils::check_bin_assets,
            commands::utils::download_bin_assets,

            // Mihomo 代理
                commands::mihomo::mihomo_get_state,
                commands::mihomo::mihomo_clear_warnings,
                commands::mihomo::mihomo_controller_info,
                commands::mihomo::mihomo_start,
                commands::mihomo::mihomo_stop,
                commands::mihomo::mihomo_restart,
                commands::mihomo::mihomo_set_core_path,
                commands::mihomo::mihomo_close_all_connections,
                commands::mihomo::mihomo_get_app_config,
                commands::mihomo::mihomo_patch_app_config,
                commands::mihomo::mihomo_get_controled_config,
                commands::mihomo::mihomo_patch_controled_config,
                commands::mihomo::mihomo_get_runtime_config,
                commands::mihomo::mihomo_update_runtime_config,
                commands::mihomo::mihomo_get_profile_config,
                commands::mihomo::mihomo_set_profile_config,
                commands::mihomo::mihomo_get_profile_item,
                commands::mihomo::mihomo_get_profile_str,
                commands::mihomo::mihomo_set_profile_str,
                commands::mihomo::mihomo_add_profile,
                commands::mihomo::mihomo_remove_profile,
                commands::mihomo::mihomo_update_profile,
                commands::mihomo::mihomo_change_current_profile,
                commands::mihomo::mihomo_validate_subscription,
                commands::mihomo::mihomo_import_subscription,
                commands::mihomo::mihomo_import_file,
                commands::mihomo::mihomo_update_subscription,
                commands::mihomo::mihomo_get_profile_status,
                commands::mihomo::mihomo_get_profile_file_path,
                commands::mihomo::mihomo_get_override_config,
                commands::mihomo::mihomo_set_override_config,
                commands::mihomo::mihomo_get_override_item,
                commands::mihomo::mihomo_add_override,
                commands::mihomo::mihomo_remove_override,
                commands::mihomo::mihomo_update_override,
                commands::mihomo::mihomo_get_override,
                commands::mihomo::mihomo_set_override,
                commands::mihomo::mihomo_api,
                commands::mihomo::mihomo_version,
                commands::mihomo::mihomo_proxies,
                commands::mihomo::mihomo_groups,
                commands::mihomo::mihomo_rules,
                commands::mihomo::mihomo_proxy_providers,
                commands::mihomo::mihomo_rule_providers,
                commands::mihomo::mihomo_change_proxy,
                commands::mihomo::mihomo_unfixed_proxy,
                commands::mihomo::mihomo_proxy_delay,
                commands::mihomo::mihomo_test_delay,
                commands::mihomo::mihomo_group_delay,
                commands::mihomo::mihomo_provider_healthcheck,
                commands::mihomo::mihomo_update_proxy_provider,
                commands::mihomo::mihomo_update_rule_provider,
                commands::mihomo::mihomo_rules_disable,
                commands::mihomo::mihomo_smart_group_weights,
                commands::mihomo::mihomo_smart_flush_cache,
                commands::mihomo::mihomo_patch_config,
                commands::mihomo::mihomo_hot_reload_config,
                commands::mihomo::mihomo_set_mode,
                commands::mihomo::mihomo_select_proxy,
                commands::mihomo::mihomo_save_secondary_proxies,
                commands::mihomo::mihomo_set_tun,
                commands::mihomo::mihomo_close_connection,
                commands::mihomo::mihomo_get_connections,
                commands::mihomo::mihomo_get_memory,
                commands::mihomo::mihomo_get_logs,
                commands::mihomo::mihomo_set_sys_proxy,
                commands::mihomo::mihomo_get_sys_proxy,
                commands::mihomo::mihomo_upgrade,
                commands::mihomo::mihomo_upgrade_geo,
                commands::mihomo::mihomo_upgrade_ui,
                commands::mihomo::mihomo_open_substore,
                commands::mihomo::mihomo_setup_firewall,
                commands::mihomo::mihomo_open_uwp_tool,
                commands::mihomo::mihomo_get_override_exec_log,
                commands::mihomo::mihomo_get_rule_str,
                commands::mihomo::mihomo_set_rule_str,
                commands::mihomo::mihomo_get_rule_override,
                commands::mihomo::mihomo_set_rule_override,
                commands::mihomo::mihomo_get_file_str,
                commands::mihomo::mihomo_set_file_str,
                commands::mihomo::mihomo_convert_mrs_ruleset,
                commands::mihomo::mihomo_detach_core,
                // ---- 内核版本管理 ----
                commands::mihomo::github::mihomo_core_variants,
                // ---- 托盘菜单配置 ----
                commands::config::get_tray_menu_config,
                commands::config::set_tray_menu_config,
                commands::mihomo::github::mihomo_download_ui,
                // ---- 网络信息 ----
                commands::mihomo::netinfo::mihomo_fetch_ip_info,
                commands::mihomo::netinfo::mihomo_measure_latency,
                commands::mihomo::netinfo::mihomo_get_interfaces,
                // ---- 备份恢复 ----
                commands::mihomo::backup::mihomo_export_local_backup,
                commands::mihomo::backup::mihomo_import_local_backup,
                commands::mihomo::backup::mihomo_webdav_backup,
                commands::mihomo::backup::mihomo_webdav_list,
                commands::mihomo::backup::mihomo_webdav_restore,
                commands::mihomo::backup::mihomo_webdav_delete,
                // ---- Sub-Store ----
                commands::mihomo::substore::mihomo_substore_download,
                commands::mihomo::substore::mihomo_substore_start,
                commands::mihomo::substore::mihomo_substore_stop,
                commands::mihomo::substore::mihomo_substore_status,
                commands::mihomo::substore::mihomo_substore_subs,
                commands::mihomo::substore::mihomo_substore_collections,
                // ---- 杂项 ----
                commands::mihomo::misc::mihomo_check_admin,
                commands::mihomo::misc::mihomo_restart_as_admin,
                commands::mihomo::misc::mihomo_check_tun_permissions,
                commands::mihomo::misc::mihomo_hot_reload,
                commands::mihomo::misc::mihomo_copy_env,
                commands::mihomo::misc::mihomo_clear_logs,
                commands::mihomo::misc::mihomo_open_path,
                commands::mihomo::misc::mihomo_cleanup_logs,
                // ---- 任务 ----
                commands::tasks::tasks_init,
                commands::tasks::tasks_list_by_date,
                commands::tasks::tasks_list_range,
                commands::tasks::tasks_search,
                commands::tasks::tasks_list_overdue,
                commands::tasks::tasks_create,
                commands::tasks::tasks_update,
                commands::tasks::tasks_set_progress,
                commands::tasks::tasks_move,
                commands::tasks::tasks_carry_over,
                commands::tasks::tasks_reorder,
                commands::tasks::tasks_set_archived,
                commands::tasks::tasks_delete,
                commands::tasks::tasks_add_log,
                commands::tasks::tasks_list_logs,
                commands::tasks::tasks_list_moves,
                commands::tasks::tasks_summary,
                commands::tasks::tasks_day_stats,

                // ---- Node 项目管理器 ----
                commands::node_manager::npm_list_projects,
                commands::node_manager::npm_deps,
                commands::node_manager::npm_status,
                commands::node_manager::npm_install,
                commands::node_manager::npm_upgrade,
                commands::node_manager::npm_install_deps,
                commands::node_manager::npm_start,
                commands::node_manager::npm_stop,
                commands::node_manager::npm_open,
                commands::node_manager::get_node_projects_dir,
                commands::node_manager::update_node_projects_dir,
                commands::node_manager::npm_check_update,

                // ---- 启动器模块（复刻 DawnLauncher） ----
                commands::launcher::commands::launcher_get_classifications,
                commands::launcher::commands::launcher_save_classification,
                commands::launcher::commands::launcher_delete_classification,
                commands::launcher::commands::launcher_reorder_classifications,
                commands::launcher::commands::launcher_get_items,
                commands::launcher::commands::launcher_get_all_items,
                commands::launcher::commands::launcher_save_item,
                commands::launcher::commands::launcher_batch_add_items,
                commands::launcher::commands::launcher_delete_item,
                commands::launcher::commands::launcher_reorder_items,
                commands::launcher::commands::launcher_execute_item,
                commands::launcher::commands::launcher_execute_raw,
                commands::launcher::commands::launcher_execute_system_command,
                commands::launcher::commands::launcher_open_file_location,
                commands::launcher::commands::launcher_extract_icon,
                commands::launcher::commands::launcher_resolve_shortcut,
                commands::launcher::commands::launcher_fetch_url_info,
                commands::launcher::commands::launcher_check_items,
                commands::launcher::commands::launcher_stop_check,
                commands::launcher::commands::launcher_scan_start_menu,
                commands::launcher::commands::launcher_scan_appx,
                commands::launcher::commands::launcher_scan_folder,
                commands::launcher::commands::launcher_get_settings,
                commands::launcher::commands::launcher_save_settings,
                commands::launcher::commands::launcher_register_hotkey,
                commands::launcher::commands::launcher_import_browser_bookmarks,
                commands::launcher::commands::launcher_process_dropped_paths,
                commands::launcher::commands::launcher_export_backup,
                commands::launcher::commands::launcher_import_backup,
                commands::launcher::commands::launcher_import_backup_file,

                // ---- 剪贴板管理器（复刻 CopyQ） ----
                commands::clipboard::clipboard_get_items,
                commands::clipboard::clipboard_delete_item,
                commands::clipboard::clipboard_pin_item,
                commands::clipboard::clipboard_clear_history,
                commands::clipboard::clipboard_copy_item,
                commands::clipboard::clipboard_paste_item,
                commands::clipboard::clipboard_get_settings,
                commands::clipboard::clipboard_save_settings,
                commands::clipboard::clipboard_get_ignored_apps,
                commands::clipboard::clipboard_add_ignored_app,
                commands::clipboard::clipboard_remove_ignored_app,
                commands::clipboard::clipboard_remember_window,
                commands::clipboard::clipboard_get_image,
                commands::otp::otp_list,
                commands::otp::otp_add,
                commands::otp::otp_update,
                commands::otp::otp_delete,
                commands::otp::otp_toggle_pin,
                commands::otp::otp_import_uri,
                commands::otp::otp_generate_code,
                commands::otp::otp_remaining_seconds,
                commands::otp::otp_mark_copied,
                commands::otp::otp_list_categories,
                commands::otp::otp_add_category,
                commands::otp::otp_rename_category,
                commands::otp::otp_delete_category,
                commands::otp::otp_set_token_categories,
                commands::otp::otp_list_brands,
                commands::otp::otp_match_brand,
                commands::otp::otp_scan_qr,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { code, api, .. } = event {
                let quit = USER_QUIT_REQUESTED.load(Ordering::SeqCst);
                exit_log::exit_log(&format!(
                    "ExitRequested fired: code={:?} user_quit_requested={} prevent_exit={}",
                    code,
                    quit,
                    code.is_none() && !quit
                ));
                // 用户通过托盘「退出」主动请求时（已带退出码），放行并执行清理。
                // 仅当「无退出码且用户未主动请求退出」时拦截（即：仅关闭主窗口，
                // 意图是保留托盘常驻，而非退出整个应用）。
                if code.is_none() && !quit {
                    api.prevent_exit();
                    exit_log::exit_log("ExitRequested: 拦截退出（仅关主窗口，保留托盘）");
                } else {
                    // 关键修复：清理逻辑（停常驻代理、kill Mihomo 内核、清系统代理）
                    // 放到后台线程执行，绝不阻塞主退出路径。否则 set_sys_proxy 等
                    // 同步 shell 调用偶发卡死会导致进程永远走不到真正退出，表现为
                    // 「点了托盘退出但程序仍在」。后台线程跑清理，主线程直接强制退出。
                    // 退出清理：只关闭「需随应用退出」的服务（RTSP、mihomo，以及应用内
                    // Http server 随进程自然终止）。Launcher「启动」模块与 SDK 模块启动的
                    // 独立进程因使用 CREATE_BREAKAWAY_FROM_JOB 已与 AnyVersion 解耦，不会
                    // 被清理，退出后保持运行。
                    exit_log::exit_log("ExitRequested: 进入退出分支，启动清理线程 + 兜底强杀");
                    let app_handle = app.clone();
                    std::thread::spawn(move || {
                        exit_log::exit_log("cleanup thread: start (kill_on_exit / stop_all_rtsp)");
                        // 先关 RTSP（内存 kill ffmpeg/mediamtx 子进程），再关 mihomo。
                        commands::rtsp_server::stop_all_rtsp_servers_inner(
                            &app_handle.state::<commands::rtsp_server::RtspServerState>(),
                        );
                        exit_log::exit_log("cleanup thread: stop_all_rtsp_servers done");
                        commands::mihomo::kill_on_exit(
                            &**app_handle.state::<commands::mihomo::MihomoState>(),
                        );
                        exit_log::exit_log("cleanup thread: kill_on_exit done");
                        // 清理完成后再真正退出进程（比主线程 4s 兜底更早，保证不卡住用户）。
                        exit_log::exit_log("cleanup thread: all done, process::exit(0)");
                        std::process::exit(0);
                    });
                    // 兜底：清理线程若卡死（如 set_sys_proxy 同步 shell 卡顿），主线程在
                    // 足够宽限后强制退出，避免「点了退出但程序仍在」。
                    std::thread::spawn(|| {
                        std::thread::sleep(std::time::Duration::from_millis(4000));
                        exit_log::exit_log("force exit fallback: std::process::exit(0) now");
                        std::process::exit(0);
                    });
                }
            }
        });
}
