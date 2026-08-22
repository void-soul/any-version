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
    crate::exit_log::exit_log(&format!("launcher_execute_item: op = {:?}, run_admin = {}", op, run_admin));

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

/// 读取本地图片文件并转为 Base64 data URL 作为项目图标（支持 png/jpg/jpeg/gif/webp/bmp/ico/svg）。
/// 用于「修改图标 → 上传本地图片」。
#[tauri::command]
pub async fn launcher_load_image_as_icon(path: String) -> Result<String, String> {
    use base64::{engine::general_purpose, Engine as _};

    let p = Path::new(&path);
    if !p.exists() {
        return Err(format!("文件不存在: {}", path));
    }
    let bytes = std::fs::read(p).map_err(|e| format!("读取图片失败: {}", e))?;
    if bytes.is_empty() {
        return Err("图片文件为空".to_string());
    }
    let mime = match p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("bmp") => "image/bmp",
        Some("ico") => "image/x-icon",
        Some("svg") => "image/svg+xml",
        _ => "image/png",
    };
    let encoded = general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:{};base64,{}", mime, encoded))
}

#[tauri::command]
pub async fn launcher_pick_file() -> Result<Option<String>, String> {
    Ok(super::windows::pick_file())
}

#[tauri::command]
pub async fn launcher_resolve_shortcut(path: String) -> Result<Option<ShortcutInfo>, String> {
    let info = resolve_shortcut(&path);
    if let Some(i) = &info {
        crate::exit_log!(
            "[resolve-shortcut] path={} => name={} target={} args={} workdir={} icon={}",
            path,
            i.name,
            i.target_path,
            i.arguments,
            i.working_dir,
            i.icon_base64.as_ref().map(|b| b.len() > 0).unwrap_or(false)
        );
    } else {
        crate::exit_log!("[resolve-shortcut] path={} => None（解析失败）", path);
    }
    Ok(info)
}

#[tauri::command]
pub async fn launcher_fetch_url_info(url: String) -> Result<UrlMetadata, String> {
    fetch_url_metadata(&url).await
}

/// 下载远程图片（如网页 favicon / 任意图片链接）并转为 Base64 data URL 作为项目图标。
/// 参考 DawnLauncher 的网络图标（NetworkIcon）功能。
/// SVG 保持原样，其余位图格式统一用 image crate 重编码为 PNG，避免 WebView2 对
/// `image/x-icon` 等 data URL 显示不稳定导致「图标下载了但显示不出来」。
#[tauri::command]
pub async fn launcher_download_image(url: String) -> Result<String, String> {
    use base64::{engine::general_purpose, Engine as _};
    use std::io::Cursor;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .build()
        .map_err(|e| e.to_string())?;

    let res = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("下载图片失败: {}", e))?;
    if !res.status().is_success() {
        return Err(format!("下载图片失败，HTTP {}", res.status()));
    }

    // 在消费 body 前先读取 Content-Type，避免 res 被 bytes() move 后无法访问
    let mime = res
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(';').next().unwrap_or("").trim().to_lowercase())
        .filter(|s| s.starts_with("image/"))
        .unwrap_or_else(|| "image/x-icon".to_string())
        .to_string();

    let bytes = res.bytes().await.map_err(|e| format!("读取图片失败: {}", e))?;
    if bytes.is_empty() {
        return Err("图片内容为空".to_string());
    }

    // SVG 保持原样（image crate 不支持 svg）
    if mime == "image/svg+xml" {
        let encoded = general_purpose::STANDARD.encode(&bytes);
        return Ok(format!("data:image/svg+xml;base64,{}", encoded));
    }

    // 其余位图：解码并重编码为 PNG，保证在 WebView 稳定显示
    if let Ok(img) = image::load_from_memory(&bytes) {
        let mut png = Cursor::new(Vec::new());
        if img.write_to(&mut png, image::ImageFormat::Png).is_ok() {
            let encoded = general_purpose::STANDARD.encode(png.into_inner());
            return Ok(format!("data:image/png;base64,{}", encoded));
        }
    }

    // 解码失败（如未知格式）：回退原始字节，按原 mime 输出
    let encoded = general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:{};base64,{}", mime, encoded))
}

/// 批量移动子分类：把 source 分类下的所有直接子分类（连同各自子树与项目，保留层级）整体移动到 target 分类之下。
#[tauri::command]
pub async fn launcher_move_subcategories_to_classification(
    source_id: i64,
    target_id: i64,
) -> Result<usize, String> {
    db::move_subcategories_to_classification(source_id, target_id)
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
    let mut hotkeys = settings.module_hotkeys.clone();
    if !settings.selection_translate_hotkey.trim().is_empty() {
        hotkeys.insert(
            "selection-translate".to_string(),
            settings.selection_translate_hotkey.clone(),
        );
    }
    let _ = super::windows::register_global_hotkeys(app, &hotkeys);
    Ok(())
}

#[tauri::command]
pub async fn launcher_register_hotkey(app: AppHandle, hotkey: String) -> Result<(), String> {
    let mut map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    map.insert("launcher".to_string(), hotkey);
    super::windows::register_global_hotkeys(app, &map)
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
