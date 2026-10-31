//! Application composition for module activation; shared runtime ownership has
//! no business dependencies. Shared AI/settings capabilities remain available.
use std::collections::HashSet;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager};

pub fn initialize() {
    let disabled = super::config::load_config().disabled_modules;
    let disabled: Vec<_> = disabled
        .into_iter()
        .filter(|id| id != "settings" && crate::module_policy::MODULE_IDS.contains(&id.as_str()))
        .collect();
    crate::runtime_tasks::configure(&disabled);
}

async fn stop(app: &AppHandle, module: &str) -> Result<(), String> {
    let mut errors = Vec::new();
    match module {
        "sdk" => super::project::versions::cancel_all_installs(),
        "suno" => super::suno::cancel_all_downloads(),
        _ => {}
    }
    if let Err(error) = crate::runtime_tasks::disable(module).await {
        errors.push(error);
    }
    // Drain admitted startup and filesystem transactions before taking service snapshots.
    // Their workers retain the guard even if the invoking future is cancelled.
    if let Err(error) =
        crate::runtime_tasks::wait_for_idle(module, std::time::Duration::from_secs(90)).await
    {
        errors.push(error);
    }
    let cleanup: Result<(), String> = async {
        match module {
            "sdk" | "suno" => super::foundation::owned_process::ensure_released(module)?,
            "clipboard" => {
                super::clipboard::stop_monitor().await?;
                super::clipboard::release_runtime_resources(app)?;
            }
            "music" => {
                if let Some(player) = app.try_state::<super::music::MusicPlayerState>() {
                    player.release_resources();
                }
                tauri::async_runtime::spawn_blocking(super::music::stop_host)
                    .await
                    .map_err(|e| e.to_string())??;
            }
            "buddy" => {
                super::buddy::twoapi::buddy2api_stop()?;
                super::buddy::release_runtime_resources()?;
            }
            "cert" => {
                super::cert_receiver::cert_receiver_stop().await?;
                super::cert_receiver::release_runtime_resources()?;
                if let Some(state) = app.try_state::<super::cert::CertScheduler>() {
                    state.state.lock().unwrap_or_else(|e| e.into_inner()).log = Vec::new();
                }
            }
            "mihomo" => {
                if let Some(state) = app.try_state::<super::mihomo::MihomoState>() {
                    state.scheduler_running.store(false, Ordering::SeqCst);
                    super::mihomo::traffic::reset_recorder_after_stop();
                    let inner = state.inner().clone();
                    super::mihomo::mihomo_stop(app.clone(), state).await?;
                    *inner
                        .runtime_config_str
                        .lock()
                        .unwrap_or_else(|e| e.into_inner()) = String::new();
                    *inner
                        .runtime_warnings
                        .lock()
                        .unwrap_or_else(|e| e.into_inner()) = Vec::new();
                    *inner.auto_last.lock().unwrap_or_else(|e| e.into_inner()) =
                        std::collections::HashMap::new();
                }
            }
            "api" => super::api::loadtest::stop_all_runs().await?,
            "httpServer" => {
                if let Some(state) = app.try_state::<super::http_server::HttpServerState>() {
                    super::http_server::stop_all_http_servers_inner(&state).await?;
                }
            }
            "rtspServer" => {
                if let Some(state) = app.try_state::<super::rtsp_server::RtspServerState>() {
                    super::rtsp_server::shutdown_module(state.inner().clone()).await?;
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
                let result = super::ai::aggregate::stop_aggregate_service(app.clone());
                super::ai::collab::stop_all_room_proxies();
                result?;
            }
            "translate" => super::ai::translate::release_runtime_resources(app)?,
            "picky" => {
                super::picky::release_runtime_resources(app)?;
                super::foundation::owned_process::ensure_released("picky")?;
            }
            _ => {}
        }
        Ok(())
    }
    .await;
    if let Err(error) = cleanup {
        errors.push(error);
    }
    if matches!(module, "ai" | "buddy") {
        if let Err(error) = crate::proxy::server::wait_for_stopped_proxies().await {
            errors.push(error);
        }
    }
    // Confirm no cleanup path left an admitted operation behind.
    if let Err(error) =
        crate::runtime_tasks::wait_for_idle(module, std::time::Duration::from_secs(5)).await
    {
        errors.push(error);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("；"))
    }
}

fn start_background(app: &AppHandle, module: &str) -> Result<(), String> {
    let status = crate::runtime_tasks::status(module);
    if status.tasks > 0 || status.active_operations > 0 {
        return Err("模块仍在停止，请等待安全收尾后重新启用".into());
    }
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
    validate_ids(&toolbar_modules)?;
    validate_ids(&disabled_modules)?;
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
        // A prior failed shutdown may retain native handles even when task counts are zero.
        // Reconfirm cleanup before allowing a new generation to start.
        let restarted = async {
            stop(&app, module).await?;
            start_background(&app, module)
        }
        .await;
        if let Err(error) = restarted {
            errors.push(format!("{module}: {error}"));
            // A failed enable must not leave the settings UI claiming a usable module.
            let _ = crate::runtime_tasks::disable(module).await;
            if let Err(error) = super::config::mutate_config(|config| {
                if !config.disabled_modules.contains(module) {
                    config.disabled_modules.push(module.clone());
                }
                Ok(())
            }) {
                errors.push(error);
            }
        }
    }
    super::launcher::commands::refresh_global_hotkeys(app);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

fn validate_ids(ids: &[String]) -> Result<(), String> {
    if let Some(id) = ids
        .iter()
        .find(|id| !crate::module_policy::MODULE_IDS.contains(&id.as_str()))
    {
        Err(format!("未知模块：{id}"))
    } else {
        Ok(())
    }
}

#[tauri::command]
pub fn get_module_runtime_status() -> Vec<crate::runtime_tasks::ModuleStatus> {
    crate::module_policy::MODULE_IDS
        .iter()
        .map(|module| crate::runtime_tasks::status(module))
        .collect()
}
