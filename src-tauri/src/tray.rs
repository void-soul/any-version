use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::menu::{Menu, MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};

const MAIN_WINDOW_LABEL: &str = "main";
const MAIN_WINDOW_TITLE: &str = "AnyVersion 开发助理";
const MAIN_WINDOW_WIDTH: f64 = 1150.0;
const MAIN_WINDOW_HEIGHT: f64 = 780.0;

const TRAY_ID: &str = "main-tray";
const ID_SHOW: &str = "show";
const ID_QUIT: &str = "quit";
const ID_EMPTY: &str = "__empty";
const ID_SWITCH_PREFIX: &str = "switch::";
const ID_SERVICE_START_PREFIX: &str = "service-start::";
const ID_SERVICE_STOP_PREFIX: &str = "service-stop::";
// ---- 内置服务 / Mihomo ----
const ID_HTTP_STOP_PREFIX: &str = "http-stop::";
const ID_HTTP_STOP_ALL: &str = "http-stop-all";
const ID_HTTP_START_LAST: &str = "http-start-last";
const ID_RTSP_STOP_PREFIX: &str = "rtsp-stop::";
const ID_RTSP_STOP_ALL: &str = "rtsp-stop-all";
const ID_RTSP_START_LAST: &str = "rtsp-start-last";
const ID_MIHOMO_TOGGLE: &str = "mihomo-toggle";
const ID_MIHOMO_MODE_PREFIX: &str = "mihomo-mode::";
const ID_MIHOMO_PROFILE_PREFIX: &str = "mihomo-profile::";
const ID_MIHOMO_PROXY_PREFIX: &str = "mihomo-proxy::";
/// 代理菜单项 id 内部分隔符（节点名可能含 "::"，故用不可见字符）
const PROXY_SEP: char = '\u{1}';

/// Mihomo 代理/模式快照：托盘菜单是同步构建的，不能在其中发起 HTTP 请求，
/// 这里缓存一份由后台任务刷新的数据。
struct MihomoSnapshot {
    at: Instant,
    mode: String,
    /// (组名, 当前节点, 候选节点)
    groups: Vec<(String, String, Vec<String>)>,
}

static MIHOMO_SNAPSHOT: Mutex<Option<MihomoSnapshot>> = Mutex::new(None);
static SNAPSHOT_INFLIGHT: AtomicBool = AtomicBool::new(false);
const SNAPSHOT_TTL: Duration = Duration::from_secs(20);

/// 托盘右键菜单是否正处于打开状态。打开期间禁止重建菜单——
/// `tray.set_menu` 会直接关闭正在显示的菜单（表现为“打开几秒后自动消失”）。
static TRAY_MENU_OPEN: AtomicBool = AtomicBool::new(false);

pub fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_menu(app)?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("AnyVersion")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button,
                button_state,
                ..
            } = event
            {
                match (button, button_state) {
                    (MouseButton::Left, MouseButtonState::Up) => show_main_window(tray.app_handle()),
                    // 右键即将弹出菜单：标记打开，避免重建导致菜单闪退
                    (MouseButton::Right, MouseButtonState::Up) => mark_tray_menu_open(),
                    _ => {}
                }
            }
        })
        .on_menu_event(|app, event| {
            // 选中菜单项即意味着菜单即将关闭，解除“打开中”标记，允许后续重建
            TRAY_MENU_OPEN.store(false, Ordering::SeqCst);
            let id = event.id.as_ref();
            match id {
                ID_SHOW => show_main_window(app),
                ID_QUIT => {
                    crate::USER_QUIT_REQUESTED.store(true, Ordering::SeqCst);
                    // 兜底强杀：即使 app.exit(0) 因后台 async 任务（scheduler/watchdog）
                    // 卡在 Tauri 内部优雅关闭流程中，也能在短暂宽限后强制退出进程，
                    // 彻底解决「开启 mihomo 长时间后托盘退出无响应」。
                    std::thread::spawn(|| {
                        std::thread::sleep(std::time::Duration::from_millis(500));
                        std::process::exit(0);
                    });
                    app.exit(0);
                }
                other if other.starts_with(ID_SWITCH_PREFIX) => {
                    if let Some((project_id, version)) = parse_switch_id(other) {
                        if crate::commands::project::versions::project_use_version_inner(project_id, version).is_ok() {
                            let _ = rebuild_tray_menu(app);
                        }
                    }
                }
                other if other.starts_with(ID_SERVICE_START_PREFIX) => {
                    if let Some(service_id) = other.strip_prefix(ID_SERVICE_START_PREFIX) {
                        if let Err(error) = crate::commands::service::start_service_inner(service_id.to_string(), None) {
                            eprintln!("failed to start service {service_id}: {error}");
                        }
                        let _ = rebuild_tray_menu(app);
                    }
                }
                other if other.starts_with(ID_SERVICE_STOP_PREFIX) => {
                    if let Some(service_id) = other.strip_prefix(ID_SERVICE_STOP_PREFIX) {
                        if let Err(error) = crate::commands::service::stop_service_inner(service_id.to_string()) {
                            eprintln!("failed to stop service {service_id}: {error}");
                        }
                        let _ = rebuild_tray_menu(app);
                    }
                }
                other => handle_extra_menu(app, other),
            }
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    builder.build(app)?;
    Ok(())
}

pub fn rebuild_tray_menu(app: &AppHandle) -> tauri::Result<()> {
    // 菜单打开期间不要重建，否则会关闭正在显示的右键菜单
    if TRAY_MENU_OPEN.load(Ordering::SeqCst) {
        return Ok(());
    }
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_menu(Some(build_menu(app)?))?;
    }
    Ok(())
}

/// HTTP / RTSP / Mihomo 相关菜单项的处理
fn handle_extra_menu(app: &AppHandle, id: &str) {
    // ---- HTTP 静态服务 ----
    if let Some(port) = id.strip_prefix(ID_HTTP_STOP_PREFIX).and_then(|p| p.parse::<u16>().ok()) {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Some(state) = app.try_state::<crate::commands::http_server::HttpServerState>() {
                let servers = state.servers.clone();
                let mut map = servers.lock().await;
                if let Some(info) = map.remove(&port) {
                    let _ = info.stop_tx.send(());
                }
            }
            let _ = rebuild_tray_menu(&app);
        });
        return;
    }
    if id == ID_HTTP_STOP_ALL {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Some(state) = app.try_state::<crate::commands::http_server::HttpServerState>() {
                let servers = state.servers.clone();
                let mut map = servers.lock().await;
                for (_, info) in map.drain() {
                    let _ = info.stop_tx.send(());
                }
            }
            let _ = rebuild_tray_menu(&app);
        });
        return;
    }
    if id == ID_HTTP_START_LAST {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let last = crate::commands::config::load_config().last_servers.http;
            let (path, port, host) = match last {
                Some(v) => (
                    v.get("path").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                    v.get("port").and_then(|x| x.as_u64()).unwrap_or(0) as u16,
                    v.get("host").and_then(|x| x.as_str()).map(|s| s.to_string()),
                ),
                None => return,
            };
            if path.is_empty() || port == 0 {
                return;
            }
            if let Some(state) = app.try_state::<crate::commands::http_server::HttpServerState>() {
                let servers = state.servers.clone();
                if let Err(e) =
                    crate::commands::http_server::start_http_server_inner(servers, path, port, host).await
                {
                    eprintln!("[tray] 启动 HTTP 服务失败: {e}");
                }
            }
            let _ = rebuild_tray_menu(&app);
        });
        return;
    }

    // ---- RTSP 推流服务 ----
    if let Some(inst) = id.strip_prefix(ID_RTSP_STOP_PREFIX) {
        if let Some(state) = app.try_state::<crate::commands::rtsp_server::RtspServerState>() {
            let _ =
                crate::commands::rtsp_server::stop_rtsp_server(app.clone(), state, inst.to_string());
        }
        let _ = rebuild_tray_menu(app);
        return;
    }
    if id == ID_RTSP_STOP_ALL {
        if let Some(state) = app.try_state::<crate::commands::rtsp_server::RtspServerState>() {
            let ids: Vec<String> = state.servers.lock().keys().cloned().collect();
            for inst in ids {
                let _ = crate::commands::rtsp_server::stop_rtsp_server(
                    app.clone(),
                    state.clone(),
                    inst,
                );
            }
        }
        let _ = rebuild_tray_menu(app);
        return;
    }
    if id == ID_RTSP_START_LAST {
        let last = crate::commands::config::load_config().last_servers.rtsp;
        if let (Some(v), Some(state)) = (
            last,
            app.try_state::<crate::commands::rtsp_server::RtspServerState>(),
        ) {
            match serde_json::from_value::<crate::commands::rtsp_server::RtspConfig>(v) {
                Ok(cfg) => {
                    if let Err(e) =
                        crate::commands::rtsp_server::start_rtsp_server(app.clone(), state, cfg)
                    {
                        eprintln!("[tray] 启动 RTSP 服务失败: {e}");
                    }
                }
                Err(e) => eprintln!("[tray] 上次 RTSP 配置无法解析: {e}"),
            }
        }
        let _ = rebuild_tray_menu(app);
        return;
    }

    // ---- Mihomo ----
    if id == ID_MIHOMO_TOGGLE {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let state = match app.try_state::<crate::commands::mihomo::MihomoState>() {
                Some(s) => s.inner().clone(),
                None => return,
            };
            let running = state.child.lock().unwrap().is_some();
            let r = if running {
                crate::commands::mihomo::manager::stop_core(&state);
                Ok(())
            } else {
                crate::commands::mihomo::manager::launch_core(&app, state.clone()).await
            };
            if let Err(e) = r {
                eprintln!("[tray] mihomo 启停失败: {e}");
            }
            invalidate_mihomo_snapshot();
            let _ = rebuild_tray_menu(&app);
        });
        return;
    }
    if let Some(mode) = id.strip_prefix(ID_MIHOMO_MODE_PREFIX) {
        let app = app.clone();
        let mode = mode.to_string();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = crate::commands::mihomo::tray_set_mode(&app, &mode).await {
                eprintln!("[tray] 切换模式失败: {e}");
            }
            invalidate_mihomo_snapshot();
            let _ = rebuild_tray_menu(&app);
        });
        return;
    }
    if let Some(profile) = id.strip_prefix(ID_MIHOMO_PROFILE_PREFIX) {
        let app = app.clone();
        let profile = profile.to_string();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = crate::commands::mihomo::tray_change_profile(&app, &profile).await {
                eprintln!("[tray] 切换订阅失败: {e}");
            }
            invalidate_mihomo_snapshot();
            let _ = rebuild_tray_menu(&app);
        });
        return;
    }
    if let Some(rest) = id.strip_prefix(ID_MIHOMO_PROXY_PREFIX) {
        if let Some((group, node)) = rest.split_once(PROXY_SEP) {
            let app = app.clone();
            let (group, node) = (group.to_string(), node.to_string());
            tauri::async_runtime::spawn(async move {
                if let Err(e) = crate::commands::mihomo::tray_change_proxy(&app, &group, &node).await {
                    eprintln!("[tray] 切换节点失败: {e}");
                }
                invalidate_mihomo_snapshot();
                let _ = rebuild_tray_menu(&app);
            });
        }
    }
}

fn invalidate_mihomo_snapshot() {
    if let Ok(mut g) = MIHOMO_SNAPSHOT.lock() {
        *g = None;
    }
}

/// 标记托盘菜单已打开，并启动兜底定时器：若用户未点选任何项直接关闭菜单，
/// 超时后自动复位标记，避免菜单重建被永久禁止（此时托盘数据可能短暂滞后）。
fn mark_tray_menu_open() {
    TRAY_MENU_OPEN.store(true, Ordering::SeqCst);
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(30)).await;
        TRAY_MENU_OPEN.store(false, Ordering::SeqCst);
    });
}

/// 后台刷新 Mihomo 快照（模式 + 代理组），完成后重建菜单
fn spawn_snapshot_refresh(app: &AppHandle) {
    if SNAPSHOT_INFLIGHT.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let snapshot = crate::commands::mihomo::tray_snapshot(&app).await;
        if let Some((mode, groups)) = snapshot {
            if let Ok(mut g) = MIHOMO_SNAPSHOT.lock() {
                *g = Some(MihomoSnapshot {
                    at: Instant::now(),
                    mode,
                    groups,
                });
            }
        }
        SNAPSHOT_INFLIGHT.store(false, Ordering::SeqCst);
        let _ = rebuild_tray_menu(&app);
    });
}

#[tauri::command]
pub fn refresh_tray_menu(app: AppHandle) -> Result<(), String> {
    rebuild_tray_menu(&app).map_err(|e| e.to_string())
}

fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    let window = match app.get_webview_window(MAIN_WINDOW_LABEL) {
        Some(window) => window,
        None => match create_main_window(app) {
            Ok(window) => window,
            Err(error) => {
                eprintln!("failed to create main window: {error}");
                return;
            }
        },
    };

    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_decorations(false);
    let _ = window.set_focus();
}

fn create_main_window<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<tauri::WebviewWindow<R>> {
    let mut builder = WebviewWindowBuilder::new(
        app,
        MAIN_WINDOW_LABEL,
        WebviewUrl::App("index.html".into()),
    )
    .title(MAIN_WINDOW_TITLE)
    .inner_size(MAIN_WINDOW_WIDTH, MAIN_WINDOW_HEIGHT)
    .decorations(false)
    .center();

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone())?;
    }

    builder.build()
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let show_item = MenuItemBuilder::with_id(ID_SHOW, "显示主窗口").build(app)?;
    let mut builder = MenuBuilder::new(app).item(&show_item).separator();

    let config = crate::commands::config::load_config();
    let registry = crate::commands::project::registry::registry();
    let versions_dir = Path::new(&config.versions_dir);
    let links_dir = Path::new(&config.links_dir);
    let mut any_managed = false;

    for def in &registry {
        let id = &def.id;
        let delegation = crate::commands::project::scanner::get_project_delegation(&config, id, def);
        let show_vc = delegation.version_control;
        if !show_vc {
            continue;
        }

        let show_version = config.project_menu_configs.get(id).is_none_or(|c| c.show_version);
        if !show_version {
            continue;
        }

        let mut versions = scan_installed_versions(&versions_dir.join(id));
        if versions.is_empty() {
            continue;
        }
        any_managed = true;

        let active = resolve_active_version(&links_dir.join(id))
            .or_else(|| config.active_versions.get(id).cloned());
        let title = format!(
            "{} ({})",
            def.display_name,
            active.clone().unwrap_or_else(|| "未激活".to_string())
        );
        let mut submenu = SubmenuBuilder::new(app, title);

        for version in versions.drain(..) {
            let label = if Some(&version) == active.as_ref() {
                format!("✓ {}", version)
            } else {
                version.clone()
            };
            let switch_id = format!("{}{}::{}", ID_SWITCH_PREFIX, id, version);
            let item = MenuItemBuilder::with_id(&switch_id, &label).build(app)?;
            submenu = submenu.item(&item);
        }

        let submenu = submenu.build()?;
        builder = builder.item(&submenu);
    }

    if !any_managed {
        let empty = MenuItemBuilder::with_id(ID_EMPTY, "(没有完全托管的项目)")
            .enabled(false)
            .build(app)?;
        builder = builder.item(&empty);
    }

    // 服务：仅显示「已托管 + 已检出 install_root」的服务项，平铺到顶层（不再用「服务」外层包装）
    let mut any_service = false;
    for def in &registry {
        if def.category != crate::commands::project::types::ProjectCategory::Service && !def.is_service {
            continue;
        }
        if !config.managed_items.contains(&def.id) {
            continue;
        }

        let show_service = config.project_menu_configs.get(&def.id).is_none_or(|c| c.show_service);
        if !show_service {
            continue;
        }

        let status = crate::commands::service::service_status_for_def(def);
        let status_text = status.status.as_deref().unwrap_or(if status.running { "running" } else { "stopped" });
        if status_text == "not_installed" {
            continue;
        }
        if !any_service {
            builder = builder.separator();
            any_service = true;
        }

        let port_text = status.port.map(|p| format!(" :{}", p)).unwrap_or_default();
        let title = match status_text {
            "running" => format!("{} · 运行中{}", def.display_name, port_text),
            "port_conflict" => format!("{} · 端口冲突{}", def.display_name, port_text),
            _ => format!("{} · 已停止{}", def.display_name, port_text),
        };

        let mut service_submenu = SubmenuBuilder::new(app, title);
        let status_item = MenuItemBuilder::with_id(
            format!("service-status::{}", def.id),
            match status_text {
                "running" => "状态：运行中",
                "port_conflict" => "状态：端口被其他进程占用",
                _ => "状态：已停止",
            },
        )
        .enabled(false)
        .build(app)?;
        service_submenu = service_submenu.item(&status_item);

        if status.running {
            let item = MenuItemBuilder::with_id(
                format!("{}{}", ID_SERVICE_STOP_PREFIX, def.id),
                "停止服务",
            )
            .build(app)?;
            service_submenu = service_submenu.item(&item);
        } else if status_text == "stopped" {
            let item = MenuItemBuilder::with_id(
                format!("{}{}", ID_SERVICE_START_PREFIX, def.id),
                "启动服务",
            )
            .build(app)?;
            service_submenu = service_submenu.item(&item);
        }

        let service_submenu = service_submenu.build()?;
        builder = builder.item(&service_submenu);
    }

    // ---- 内置服务与 Mihomo（可在设置里控制是否显示）----
    let tray_cfg = config.tray_menu.clone();
    let mut extra_started = false;

    if tray_cfg.show_http_server {
        if let Some(item) = build_http_submenu(app)? {
            if !extra_started {
                extra_started = true;
                builder = builder.separator();
            }
            builder = builder.item(&item);
        }
    }
    if tray_cfg.show_rtsp_server {
        if let Some(item) = build_rtsp_submenu(app)? {
            if !extra_started {
                extra_started = true;
                builder = builder.separator();
            }
            builder = builder.item(&item);
        }
    }
    if tray_cfg.show_mihomo {
        if let Some(item) = build_mihomo_submenu(app, &tray_cfg)? {
            if !extra_started {
                builder = builder.separator();
            }
            builder = builder.item(&item);
        }
    }

    let quit_item = MenuItemBuilder::with_id(ID_QUIT, "退出 AnyVersion").build(app)?;
    builder = builder.separator().item(&quit_item);

    builder.build()
}

fn build_http_submenu(app: &AppHandle) -> tauri::Result<Option<tauri::menu::Submenu<tauri::Wry>>> {
    let state = match app.try_state::<crate::commands::http_server::HttpServerState>() {
        Some(s) => s,
        None => return Ok(None),
    };
    // 托盘菜单是同步构建的，拿不到锁就跳过本轮（不阻塞）
    let running: Vec<(u16, String)> = match state.servers.try_lock() {
        Ok(map) => map.iter().map(|(p, i)| (*p, i.path.clone())).collect(),
        Err(_) => return Ok(None),
    };
    let mut sub = SubmenuBuilder::new(
        app,
        if running.is_empty() {
            "HTTP 服务 · 未运行".to_string()
        } else {
            format!("HTTP 服务 · {} 个运行中", running.len())
        },
    );
    let mut ports: Vec<(u16, String)> = running;
    ports.sort_by_key(|(p, _)| *p);
    for (port, path) in &ports {
        let item = MenuItemBuilder::with_id(
            format!("{}{}", ID_HTTP_STOP_PREFIX, port),
            format!("停止 :{}  {}", port, shorten(path, 40)),
        )
        .build(app)?;
        sub = sub.item(&item);
    }
    if ports.len() > 1 {
        let item = MenuItemBuilder::with_id(ID_HTTP_STOP_ALL, "停止全部").build(app)?;
        sub = sub.item(&item);
    }
    let last = crate::commands::config::load_config().last_servers.http;
    if let Some(v) = last {
        let path = v.get("path").and_then(|x| x.as_str()).unwrap_or("");
        let port = v.get("port").and_then(|x| x.as_u64()).unwrap_or(0) as u16;
        let already = ports.iter().any(|(p, _)| *p == port);
        if !path.is_empty() && port > 0 && !already {
            let item = MenuItemBuilder::with_id(
                ID_HTTP_START_LAST,
                format!("启动上次目录 :{}  {}", port, shorten(path, 40)),
            )
            .build(app)?;
            sub = sub.separator().item(&item);
        }
    } else if ports.is_empty() {
        let item = MenuItemBuilder::with_id("http-none", "（在主界面启动一次后可在此快捷开关）")
            .enabled(false)
            .build(app)?;
        sub = sub.item(&item);
    }
    Ok(Some(sub.build()?))
}

fn build_rtsp_submenu(app: &AppHandle) -> tauri::Result<Option<tauri::menu::Submenu<tauri::Wry>>> {
    let state = match app.try_state::<crate::commands::rtsp_server::RtspServerState>() {
        Some(s) => s,
        None => return Ok(None),
    };
    let running: Vec<String> = match state.servers.try_lock() {
        Some(map) => map.keys().cloned().collect(),
        None => return Ok(None),
    };
    let mut sub = SubmenuBuilder::new(
        app,
        if running.is_empty() {
            "RTSP 服务 · 未运行".to_string()
        } else {
            format!("RTSP 服务 · {} 个运行中", running.len())
        },
    );
    let mut ids = running;
    ids.sort();
    for inst in &ids {
        let item = MenuItemBuilder::with_id(
            format!("{}{}", ID_RTSP_STOP_PREFIX, inst),
            format!("停止 {}", inst),
        )
        .build(app)?;
        sub = sub.item(&item);
    }
    if ids.len() > 1 {
        let item = MenuItemBuilder::with_id(ID_RTSP_STOP_ALL, "停止全部").build(app)?;
        sub = sub.item(&item);
    }
    let last = crate::commands::config::load_config().last_servers.rtsp;
    if let Some(v) = last {
        let inst = v
            .get("id")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let port = v.get("port").and_then(|x| x.as_u64()).unwrap_or(0);
        if !ids.contains(&inst) {
            let item = MenuItemBuilder::with_id(
                ID_RTSP_START_LAST,
                format!("启动上次配置 {}:{}", if inst.is_empty() { "推流" } else { &inst }, port),
            )
            .build(app)?;
            sub = sub.separator().item(&item);
        }
    } else if ids.is_empty() {
        let item = MenuItemBuilder::with_id("rtsp-none", "（在主界面启动一次后可在此快捷开关）")
            .enabled(false)
            .build(app)?;
        sub = sub.item(&item);
    }
    Ok(Some(sub.build()?))
}

fn build_mihomo_submenu(app: &AppHandle, cfg: &crate::commands::config::TrayMenuConfig) -> tauri::Result<Option<tauri::menu::Submenu<tauri::Wry>>> {
    let state = match app.try_state::<crate::commands::mihomo::MihomoState>() {
        Some(s) => s.inner().clone(),
        None => return Ok(None),
    };
    let running = state.child.lock().map(|c| c.is_some()).unwrap_or(false);
    let profiles: Vec<(String, String)> = state
        .profile_config
        .lock()
        .map(|p| p.items.iter().map(|i| (i.id.clone(), i.name.clone())).collect())
        .unwrap_or_default();
    let current_profile = state
        .app_config
        .lock()
        .map(|c| c.current_profile.clone())
        .unwrap_or_default();

    let mut sub = SubmenuBuilder::new(
        app,
        format!("Mihomo · {}", if running { "运行中" } else { "已停止" }),
    );
    let toggle = MenuItemBuilder::with_id(
        ID_MIHOMO_TOGGLE,
        if running { "停止内核" } else { "启动内核" },
    )
    .build(app)?;
    sub = sub.item(&toggle);

    // 快照（模式 + 代理组）：过期或缺失时后台刷新，本轮先用旧值渲染
    let snapshot_mode;
    let snapshot_groups;
    {
        let guard = MIHOMO_SNAPSHOT.lock().ok();
        let snap = guard.as_ref().and_then(|g| g.as_ref());
        let fresh = snap.map(|s| s.at.elapsed() < SNAPSHOT_TTL).unwrap_or(false);
        snapshot_mode = snap.map(|s| s.mode.clone()).unwrap_or_default();
        snapshot_groups = snap.map(|s| s.groups.clone()).unwrap_or_default();
        if running && !fresh {
            spawn_snapshot_refresh(app);
        }
    }

    if cfg.show_mihomo_mode {
        sub = sub.separator();
        for (id, label) in [("rule", "规则"), ("global", "全局"), ("direct", "直连")] {
            let checked = snapshot_mode == id;
            let item = MenuItemBuilder::with_id(
                format!("{}{}", ID_MIHOMO_MODE_PREFIX, id),
                if checked { format!("✓ {}", label) } else { label.to_string() },
            )
            .enabled(running)
            .build(app)?;
            sub = sub.item(&item);
        }
    }

    if cfg.show_mihomo_profiles && !profiles.is_empty() {
        let mut psub = SubmenuBuilder::new(app, "订阅");
        for (id, name) in profiles {
            let checked = id == current_profile;
            let item = MenuItemBuilder::with_id(
                format!("{}{}", ID_MIHOMO_PROFILE_PREFIX, id),
                if checked { format!("✓ {}", name) } else { name },
            )
            .build(app)?;
            psub = psub.item(&item);
        }
        let psub = psub.build()?;
        sub = sub.separator().item(&psub);
    }

    if cfg.show_mihomo_proxies && !snapshot_groups.is_empty() {
        sub = sub.separator();
        let limit = cfg.mihomo_proxy_limit.max(1);
        for (group, now, nodes) in snapshot_groups {
            let mut gsub = SubmenuBuilder::new(app, format!("{} · {}", group, shorten(&now, 18)));
            for node in nodes.iter().take(limit) {
                let checked = *node == now;
                let item = MenuItemBuilder::with_id(
                    format!("{}{}{}{}", ID_MIHOMO_PROXY_PREFIX, group, PROXY_SEP, node),
                    if checked { format!("✓ {}", node) } else { node.clone() },
                )
                .build(app)?;
                gsub = gsub.item(&item);
            }
            if nodes.len() > limit {
                let more = MenuItemBuilder::with_id(
                    format!("mihomo-more::{}", group),
                    format!("… 其余 {} 个节点请在主界面选择", nodes.len() - limit),
                )
                .enabled(false)
                .build(app)?;
                gsub = gsub.item(&more);
            }
            let gsub = gsub.build()?;
            sub = sub.item(&gsub);
        }
    }

    Ok(Some(sub.build()?))
}

fn shorten(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    let head: String = chars[..max.saturating_sub(1)].iter().collect();
    format!("{}…", head)
}

fn scan_installed_versions(dir: &Path) -> Vec<String> {
    let mut versions: Vec<String> = std::fs::read_dir(dir)
        .ok()
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok())
                .filter(|entry| {
                    entry
                        .file_type()
                        .map(|ty| ty.is_dir() || ty.is_symlink())
                        .unwrap_or(false)
                })
                .map(|entry| entry.file_name().to_string_lossy().to_string())
                .filter(|name| !name.starts_with('.'))
                .collect()
        })
        .unwrap_or_default();
    versions.sort();
    versions
}

fn resolve_active_version(junction_path: &Path) -> Option<String> {
    if !junction_path.exists() && !junction_path.is_symlink() {
        return None;
    }

    std::fs::canonicalize(junction_path)
        .ok()
        .and_then(|target| target.file_name().map(|name| name.to_string_lossy().to_string()))
}

fn parse_switch_id(id: &str) -> Option<(&str, &str)> {
    let rest = id.strip_prefix(ID_SWITCH_PREFIX)?;
    rest.split_once("::")
}
