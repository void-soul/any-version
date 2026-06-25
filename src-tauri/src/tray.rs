use std::path::Path;

use tauri::menu::{Menu, MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};

const MAIN_WINDOW_LABEL: &str = "main";
const MAIN_WINDOW_TITLE: &str = "AnyVersion 开发环境管理器";
const MAIN_WINDOW_WIDTH: f64 = 1150.0;
const MAIN_WINDOW_HEIGHT: f64 = 780.0;

const TRAY_ID: &str = "main-tray";
const ID_SHOW: &str = "show";
const ID_QUIT: &str = "quit";
const ID_EMPTY: &str = "__empty";
const ID_SWITCH_PREFIX: &str = "switch::";

pub fn build_tray<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let menu = build_menu(app)?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("AnyVersion")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .on_menu_event(|app, event| {
            let id = event.id.as_ref();
            match id {
                ID_SHOW => show_main_window(app),
                ID_QUIT => app.exit(0),
                other if other.starts_with(ID_SWITCH_PREFIX) => {
                    if let Some((project_id, version)) = parse_switch_id(other) {
                        if crate::commands::project::versions::project_use_version_inner(project_id, version).is_ok() {
                            let _ = rebuild_tray_menu(app);
                        }
                    }
                }
                _ => {}
            }
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    builder.build(app)?;
    Ok(())
}

pub fn rebuild_tray_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_menu(Some(build_menu(app)?))?;
    }
    Ok(())
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
    .center();

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone())?;
    }

    builder.build()
}

fn build_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let show_item = MenuItemBuilder::with_id(ID_SHOW, "显示主窗口").build(app)?;
    let mut builder = MenuBuilder::new(app).item(&show_item).separator();

    let config = crate::commands::config::load_config();
    let registry = crate::commands::project::registry::registry();
    let versions_dir = Path::new(&config.versions_dir);
    let links_dir = Path::new(&config.links_dir);
    let mut any_managed = false;

    for def in &registry {
        let id = &def.id;
        let fully_managed = config.managed_items.contains(id)
            && !config.simple_managed_items.contains(id)
            && !def.simple_mode;
        if !fully_managed {
            continue;
        }

        let mut versions = scan_installed_versions(&versions_dir.join(id));
        if versions.is_empty() {
            continue;
        }
        any_managed = true;

        let active = resolve_active_version(&links_dir.join(id));
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

    let quit_item = MenuItemBuilder::with_id(ID_QUIT, "退出 AnyVersion").build(app)?;
    builder = builder.separator().item(&quit_item);

    builder.build()
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
