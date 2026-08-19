use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use tauri::{AppHandle, Emitter, Manager};

use super::db;
use super::models::{
    AppxItem, CheckProgress, Classification, Item, ItemCheckResult, LauncherSetting, ScannedProgram,
    ShortcutInfo, UrlMetadata,
};
use super::windows::{
    extract_file_icon, fetch_url_metadata, fetch_url_metadata_with_timeout, resolve_shortcut,
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
    let run_admin = item.data.run_as_admin;
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

    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
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

/// 停止检测：置位停止标志，正在运行的检测循环会在下一项前终止。
static CHECK_STOP_REQUESTED: AtomicBool = AtomicBool::new(false);

/// 请求停止当前正在进行的检测。
#[tauri::command]
pub fn launcher_stop_check() {
    CHECK_STOP_REQUESTED.store(true, Ordering::SeqCst);
}

/// 批量检测启动项是否存在：
/// - 网页类(item_type=2)：探测可访问性（短超时），并自动更新链接图标（favicon）与标题；
/// - 其余类型：检查本地路径是否存在，不存在时前端以红色边框标识。
/// 每检测完一项 emit 一次进度事件（含该项结果），前端实时高亮；
/// 同时把检测结果（exists/checkedAt）持久化到数据库，重启后仍保留。
#[tauri::command]
pub async fn launcher_check_items(
    app: AppHandle,
    items: Vec<Item>,
) -> Result<Vec<ItemCheckResult>, String> {
    let total = items.len();
    let mut results = Vec::with_capacity(total);
    CHECK_STOP_REQUESTED.store(false, Ordering::SeqCst);
    let stopped = || CHECK_STOP_REQUESTED.load(Ordering::SeqCst);

    // 进度事件：每检测完一项向前端上报一次（含该项结果，便于实时呈现）
    let report_progress = |done: usize, res: &Option<&ItemCheckResult>| {
        let payload = match res {
            Some(r) => CheckProgress {
                done,
                total,
                item_id: r.item_id,
                name: r.name.clone(),
                exists: r.exists,
                icon: r.icon.clone(),
                title: r.title.clone(),
                stopped: false,
            },
            None => CheckProgress {
                done,
                total,
                item_id: 0,
                name: String::new(),
                exists: true,
                icon: None,
                title: None,
                stopped: false,
            },
        };
        let _ = app.emit("launcher-check-progress", payload);
    };

    report_progress(0, &None);

    for (idx, item) in items.into_iter().enumerate() {
        // 用户点击停止：立即终止剩余检测
        if stopped() {
            let _ = app.emit(
                "launcher-check-progress",
                CheckProgress {
                    done: idx,
                    total,
                    item_id: 0,
                    name: String::new(),
                    exists: true,
                    icon: None,
                    title: None,
                    stopped: true,
                },
            );
            break;
        }

        let mut result = ItemCheckResult {
            item_id: item.id,
            exists: true,
            icon: None,
            title: None,
            name: item.name.clone(),
        };

        let mut updated_icon: Option<String> = None;
        let mut updated_title: Option<String> = None;

        match item.item_type {
            // 网页类：短超时探测可访问性并抓取 favicon
            2 => {
                let target = item.data.target.clone().unwrap_or_default();
                if target.trim().is_empty() {
                    result.exists = false;
                } else {
                    // 检测用短超时（2.5s），快速跳过超时的网络项目
                    match fetch_url_metadata_with_timeout(
                        &target,
                        std::time::Duration::from_millis(2500),
                    )
                    .await
                    {
                        Ok(meta) => {
                            result.exists = true;
                            if !item.data.fixed_icon && meta.icon.is_some() {
                                result.icon = meta.icon.clone();
                                updated_icon = meta.icon.clone();
                            }
                            if !meta.title.is_empty() && meta.title != item.name {
                                result.title = Some(meta.title.clone());
                                updated_title = Some(meta.title);
                            }
                        }
                        Err(_) => {
                            result.exists = false;
                        }
                    }
                }
            }
            // 系统/Appx：无法（或无需）做路径检测，视为存在
            3 | 4 => {
                result.exists = true;
            }
            // 文件/文件夹/多项目：检查本地路径是否存在
            _ => {
                result.exists = check_local_target_exists(&item);
            }
        }

        // 持久化检测结果（exists/checkedAt 与网页图标/标题更新）
        let mut item_to_save = item.clone();
        item_to_save.data.exists = Some(result.exists);
        item_to_save.data.checked_at = Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
        );
        if let Some(icon) = updated_icon {
            item_to_save.data.icon = Some(icon);
        }
        if let Some(title) = updated_title {
            item_to_save.name = title;
        }
        let _ = db::save_item(&item_to_save);

        results.push(result);
        report_progress(idx + 1, &Some(&results.last().unwrap()));
    }
    Ok(results)
}

/// 检查本地文件/文件夹目标是否存在；多项目检查首个条目。
fn check_local_target_exists(item: &Item) -> bool {
    if item.item_type == 5 {
        let first = item
            .data
            .multi_items
            .as_ref()
            .and_then(|v| v.first())
            .map(|m| m.target.as_str())
            .unwrap_or_default();
        return !first.trim().is_empty() && Path::new(first).exists();
    }
    let target = item.data.target.clone().unwrap_or_default();
    if target.trim().is_empty() {
        return false;
    }
    // 去掉常见的引号包裹
    let target = target.trim_matches('"');
    let p = Path::new(target);
    if p.exists() {
        return true;
    }
    // 目标可能是 .lnk 指向的真实程序：.lnk 自身存在即算存在
    let lower = target.to_lowercase();
    if lower.ends_with(".lnk") && p.exists() {
        return true;
    }
    false
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
    db::save_settings(&settings)?;
    let _ = super::windows::register_global_hotkeys(
        app,
        &settings.show_hide_shortcut_key,
        &settings.module_hotkeys,
    );
    Ok(())
}

#[tauri::command]
pub async fn launcher_register_hotkey(app: AppHandle, hotkey: String) -> Result<(), String> {
    let empty: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    super::windows::register_global_hotkeys(app, &hotkey, &empty)
}

#[tauri::command]
pub async fn launcher_process_dropped_paths(
    paths: Vec<String>,
    classification_id: i64,
) -> Result<Vec<Item>, String> {
    super::windows::process_dropped_paths(paths, classification_id)
}

#[tauri::command]
pub async fn launcher_import_browser_bookmarks(
    browser: String,
    custom_path: Option<String>,
) -> Result<super::models::BrowserImportResult, String> {
    super::windows::import_browser_bookmarks(&browser, custom_path.as_deref())
}

#[tauri::command]
pub async fn launcher_export_backup() -> Result<String, String> {
    db::export_backup()
}

#[tauri::command]
pub async fn launcher_import_backup(json_str: String) -> Result<(), String> {
    super::importers::import_dawn_or_any_json(&json_str).map(|_| ())
}

#[tauri::command]
pub async fn launcher_import_backup_file(file_path: String) -> Result<usize, String> {
    let path = Path::new(&file_path);
    if !path.exists() {
        return Err(format!("文件不存在: {}", file_path));
    }

    let bytes = std::fs::read(&file_path)
        .map_err(|e| format!("读取备份文件失败: {}", e))?;

    // 标准 SQLite 或 sqleet(chacha20) 加密数据库 → 走数据库导入（内部自动解密）
    if bytes.starts_with(b"SQLite format 3\0") {
        super::importers::import_dawn_or_any_db(&file_path)
    } else if let Ok(content) = String::from_utf8(bytes) {
        // 有效 UTF-8 文本 → 按 JSON 备份导入
        super::importers::import_dawn_or_any_json(&content)
    } else {
        // 非文本、非标准 SQLite → 按数据库导入（支持 sqleet 加密的 Dawn Launcher Data.db）
        super::importers::import_dawn_or_any_db(&file_path)
    }
}
