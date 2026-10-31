//! Application composition for module activation; shared runtime ownership has
//! no business dependencies. Shared AI/settings capabilities remain available.
use std::collections::HashSet;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager};

pub fn initialize() {
    crate::runtime_tasks::configure(&super::config::load_config().disabled_modules);
}

async fn stop(app: &AppHandle, module: &str) -> Result<(), String> {
    crate::runtime_tasks::disable(module).await;
    match module {
        "clipboard" => super::clipboard::stop_monitor().await?,
        "music" => {
            if let Some(player) = app.try_state::<super::music::MusicPlayerState>() {
                player.stop();
            }
            tauri::async_runtime::spawn_blocking(super::music::stop_host)
                .await
                .map_err(|e| e.to_string())??;
        }
        "buddy" => {
            super::buddy::twoapi::buddy2api_stop()?;
        }
        "cert" => {
            super::cert_receiver::cert_receiver_stop().await?;
        }
        "mihomo" => {
            if let Some(state) = app.try_state::<super::mihomo::MihomoState>() {
                state.scheduler_running.store(false, Ordering::SeqCst);
                super::mihomo::traffic::reset_recorder_after_stop();
                super::mihomo::mihomo_stop(app.clone(), state).await?;
            }
        }
        "api" => super::api::loadtest::stop_all_runs().await?,
        "httpServer" => {
            if let Some(state) = app.try_state::<super::http_server::HttpServerState>() {
                let servers = std::mem::take(&mut *state.servers.lock().await);
                for (_, server) in servers {
                    let _ = server.stop_tx.send(());
                }
            }
        }
        "rtspServer" => {
            if let Some(state) = app.try_state::<super::rtsp_server::RtspServerState>() {
                super::rtsp_server::stop_all_rtsp_servers_inner(&state);
            }
        }
        "serial" => {
            if let (Some(serial), Some(sim)) = (
                app.try_state::<super::serialtool::SerialState>(),
                app.try_state::<super::serialtool::SimState>(),
            ) {
                super::serialtool::close_all(&serial, &sim);
            }
        }
        "wsDebugger" => {
            if let Some(state) = app.try_state::<super::wstool::WsState>() {
                super::wstool::close_all(&state);
            }
        }
        "ai" => {
            super::ai::aggregate::stop_aggregate_service(app.clone())?;
            super::ai::collab::stop_all_room_proxies();
        }
        _ => {}
    }
    Ok(())
}

fn start_background(app: &AppHandle, module: &str) -> Result<(), String> {
    crate::runtime_tasks::enable(module);
    match module {
        "clipboard" => super::clipboard::init_clipboard_state(app)?,
        "music" => super::music::start_queue_watcher(app.clone()),
        "buddy" => {
            super::buddy::start_auto_checkin_scheduler(app.clone());
            super::buddy::start_auto_travel_scheduler(app.clone());
        }
        "cert" => {
            super::cert::start_scheduler(app.clone());
            super::cert_receiver::start_on_boot();
        }
        "mihomo" => {
            if let Some(state) = app.try_state::<super::mihomo::MihomoState>() {
                super::mihomo::start_scheduler(app.clone(), (*state).clone());
            }
        }
        _ => {}
    }
    Ok(())
}

#[tauri::command]
pub async fn set_module_layout(
    app: AppHandle,
    toolbar_modules: Vec<String>,
    disabled_modules: Vec<String>,
) -> Result<(), String> {
    static UPDATE: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    let _update = UPDATE
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    if disabled_modules.iter().any(|id| id == "settings") {
        return Err("设置模块不能禁用".into());
    }
    let previous: HashSet<String> = super::config::load_config()
        .disabled_modules
        .into_iter()
        .collect();
    let disabled: HashSet<String> = disabled_modules.iter().cloned().collect();
    super::config::set_module_layout(toolbar_modules, disabled_modules)?;
    let mut errors = vec![];
    for module in &disabled {
        if let Err(error) = stop(&app, module).await {
            errors.push(format!("{module}: {error}"));
        }
    }
    for module in previous.difference(&disabled) {
        if let Err(error) = start_background(&app, module) {
            errors.push(format!("{module}: {error}"));
        }
    }
    super::launcher::commands::refresh_global_hotkeys(app);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}
