use std::path::Path;
use std::thread;
use tauri::{AppHandle, Manager};

use super::db;
use super::models::{
    AppxItem, Classification, Item, LauncherSetting, ScannedProgram, ShortcutInfo, UrlMetadata,
};
use super::windows::{
    extract_file_icon, fetch_url_metadata, register_global_hotkey, resolve_shortcut,
    scan_appx_list, scan_associated_folder, scan_start_menu, shell_execute, system_item_execute,
};

#[tauri::command]
pub async fn launcher_get_classifications() -> Result<Vec<Classification>, String> {
    db::list_classifications()
}

#[tauri::command]
pub async fn launcher_save_classification(classification: Classification) -> Result<i64, String> {
    db::save_classification(&classification)
}

#[tauri::command]
pub async fn launcher_delete_classification(id: i64) -> Result<(), String> {
    db::delete_classification(id)
}

#[tauri::command]
pub async fn launcher_reorder_classifications(orders: Vec<(i64, i32)>) -> Result<(), String> {
    db::reorder_classifications(orders)
}

#[tauri::command]
pub async fn launcher_get_items(classification_id: i64) -> Result<Vec<Item>, String> {
    db::list_items_by_classification(classification_id)
}

#[tauri::command]
pub async fn launcher_get_all_items() -> Result<Vec<Item>, String> {
    db::list_all_items()
}

#[tauri::command]
pub async fn launcher_save_item(item: Item) -> Result<i64, String> {
    db::save_item(&item)
}

#[tauri::command]
pub async fn launcher_batch_add_items(items: Vec<Item>) -> Result<Vec<i64>, String> {
    db::batch_add_items(items)
}

#[tauri::command]
pub async fn launcher_delete_item(id: i64) -> Result<(), String> {
    db::delete_item(id)
}

#[tauri::command]
pub async fn launcher_reorder_items(orders: Vec<(i64, i32)>) -> Result<(), String> {
    db::reorder_items(orders)
}

#[tauri::command]
pub async fn launcher_execute_item(
    app: AppHandle,
    item_id: Option<i64>,
    item: Item,
) -> Result<(), String> {
    let settings = db::get_settings().unwrap_or_default();
    let run_admin = item.data.run_as_admin || settings.default_run_as_admin;
    let op = if run_admin { "runas" } else { "open" };

    match item.item_type {
        0 => {
            // 文件 / 可执行程序
            let target = item.data.target.unwrap_or_default();
            let params = item.data.params.unwrap_or_default();
            let start_loc = item.data.start_location.as_deref();
            shell_execute(op, &target, &params, start_loc)?;
        }
        1 => {
            // 文件夹
            let target = item.data.target.unwrap_or_default();
            shell_execute("explore", &target, "", None)?;
        }
        2 => {
            // 网址
            let target = item.data.target.unwrap_or_default();
            shell_execute("open", &target, "", None)?;
        }
        3 => {
            // 系统指令
            let target = item.data.target.unwrap_or_default();
            system_item_execute(&target, item.data.params.as_deref())?;
        }
        4 => {
            // Appx 应用
            let target = item.data.target.unwrap_or_default();
            let app_target = if target.starts_with("shell:AppsFolder") {
                target
            } else {
                format!("shell:AppsFolder\\{}", target)
            };
            shell_execute("open", "explorer.exe", &app_target, None)?;
        }
        5 => {
            // 多项目连环启动
            if let Some(multi) = item.data.multi_items {
                thread::spawn(move || {
                    for sub in multi {
                        let sub_op = if sub.run_as_admin { "runas" } else { "open" };
                        let _ = shell_execute(sub_op, &sub.target, sub.params.as_deref().unwrap_or(""), None);
                        if sub.delay_ms > 0 {
                            thread::sleep(std::time::Duration::from_millis(sub.delay_ms));
                        }
                    }
                });
            }
        }
        _ => {
            let target = item.data.target.unwrap_or_default();
            shell_execute(op, &target, "", None)?;
        }
    }

    if let Some(id) = item_id {
        let _ = db::increment_item_open_count(id);
    }

    if settings.open_after_hide {
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.minimize();
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn launcher_execute_raw(
    file: String,
    params: Option<String>,
    run_as_admin: bool,
    start_location: Option<String>,
) -> Result<(), String> {
    let op = if run_as_admin { "runas" } else { "open" };
    shell_execute(op, &file, params.as_deref().unwrap_or(""), start_location.as_deref())
}

#[tauri::command]
pub async fn launcher_execute_system_command(
    target: String,
    params: Option<String>,
) -> Result<(), String> {
    system_item_execute(&target, params.as_deref())
}

#[tauri::command]
pub async fn launcher_open_file_location(path: String) -> Result<(), String> {
    let p = Path::new(&path);
    if p.exists() {
        let param = format!("/select,\"{}\"", path);
        shell_execute("open", "explorer.exe", &param, None)
    } else {
        Err(format!("文件不存在: {}", path))
    }
}

#[tauri::command]
pub async fn launcher_extract_icon(path: String) -> Result<Option<String>, String> {
    Ok(extract_file_icon(&path))
}

#[tauri::command]
pub async fn launcher_resolve_shortcut(path: String) -> Result<Option<ShortcutInfo>, String> {
    Ok(resolve_shortcut(&path))
}

#[tauri::command]
pub async fn launcher_fetch_url_info(url: String) -> Result<UrlMetadata, String> {
    fetch_url_metadata(&url).await
}

#[tauri::command]
pub async fn launcher_scan_start_menu() -> Result<Vec<ScannedProgram>, String> {
    Ok(scan_start_menu())
}

#[tauri::command]
pub async fn launcher_scan_appx() -> Result<Vec<AppxItem>, String> {
    Ok(scan_appx_list())
}

#[tauri::command]
pub async fn launcher_scan_folder(
    path: String,
    hidden_items: Option<String>,
    show_only: Option<String>,
) -> Result<Vec<ScannedProgram>, String> {
    Ok(scan_associated_folder(&path, hidden_items.as_deref(), show_only.as_deref()))
}

#[tauri::command]
pub async fn launcher_get_settings() -> Result<LauncherSetting, String> {
    db::get_settings()
}

#[tauri::command]
pub async fn launcher_save_settings(
    app: AppHandle,
    settings: LauncherSetting,
) -> Result<(), String> {
    let old = db::get_settings().unwrap_or_default();
    db::save_settings(&settings)?;

    if old.show_hide_shortcut_key != settings.show_hide_shortcut_key
        && !settings.show_hide_shortcut_key.trim().is_empty()
    {
        let _ = register_global_hotkey(app, &settings.show_hide_shortcut_key);
    }
    Ok(())
}

#[tauri::command]
pub async fn launcher_register_hotkey(app: AppHandle, hotkey: String) -> Result<(), String> {
    register_global_hotkey(app, &hotkey)
}

#[tauri::command]
pub async fn launcher_import_browser_bookmarks(
    browser: String,
    custom_path: Option<String>,
) -> Result<usize, String> {
    super::windows::import_browser_bookmarks(&browser, custom_path.as_deref())
}

#[tauri::command]
pub async fn launcher_export_backup() -> Result<String, String> {
    db::export_backup()
}

#[tauri::command]
pub async fn launcher_import_backup(json_str: String) -> Result<(), String> {
    db::import_backup(&json_str)
}
