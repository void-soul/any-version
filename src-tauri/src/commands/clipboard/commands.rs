//! 剪贴板模块 Tauri 命令层

use base64::Engine;
use tauri::{AppHandle, Manager, State};

use super::db;
use super::paste;
use super::{ClipboardItem, ClipboardSettings, ClipboardState};

// ---------------------------------------------------------------------------
// 历史列表
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn clipboard_get_items(
    state: State<'_, ClipboardState>,
    keyword: Option<String>,
    kind: Option<String>,
    pinned_only: Option<bool>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<serde_json::Value, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let items = db::query_items(
        &conn,
        keyword.as_deref().unwrap_or(""),
        kind.as_deref().unwrap_or(""),
        pinned_only.unwrap_or(false),
        limit.unwrap_or(200).min(1000),
        offset.unwrap_or(0).max(0),
    )?;
    let total = db::count_items(
        &conn,
        keyword.as_deref().unwrap_or(""),
        kind.as_deref().unwrap_or(""),
        pinned_only.unwrap_or(false),
    )?;
    Ok(serde_json::json!({ "items": items, "total": total }))
}

#[tauri::command]
pub fn clipboard_delete_item(state: State<'_, ClipboardState>, id: i64) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::delete_item_with_files(&conn, id, &state.data_dir)?;
    Ok(())
}

#[tauri::command]
pub fn clipboard_pin_item(state: State<'_, ClipboardState>, id: i64, pinned: bool) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::pin_item(&conn, id, pinned)
}

#[tauri::command]
pub fn clipboard_clear_history(state: State<'_, ClipboardState>, keep_pinned: Option<bool>) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let keep = keep_pinned.unwrap_or(true);
    db::clear_items(&conn, keep)?;
    // 清理图片文件（保留置顶条目引用的图片）
    let dir = super::images_dir();
    if keep {
        let keep_paths = db::pinned_image_paths(&conn, &state.data_dir)?;
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let p = e.path();
                if !keep_paths.contains(&p) {
                    let _ = std::fs::remove_file(&p);
                }
            }
        }
    } else {
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
    }
    // 清理被删除条目残留的格式数据
    let _ = db::cleanup_orphan_formats(&conn);
    Ok(())
}

// ---------------------------------------------------------------------------
// 复制 / 粘贴
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn clipboard_copy_item(state: State<'_, ClipboardState>, id: i64) -> Result<(), String> {
    // 剪贴板写入是阻塞操作（OpenClipboard/SetClipboardData、大图解码），放到后台线程执行，
    // 避免在主线程卡死 UI（复制时程序无响应）。
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = st.db.lock().map_err(|e| e.to_string())?;
        let item = db::get_item(&conn, id)?.ok_or("条目不存在")?;
        copy_item_to_clipboard(&st, &item)
    })
    .await
    .map_err(|e| format!("复制任务异常: {}", e))?
}

#[tauri::command]
pub async fn clipboard_paste_item(
    app: AppHandle,
    state: State<'_, ClipboardState>,
    id: i64,
) -> Result<(), String> {
    // 1. 复制到剪贴板（阻塞操作放后台线程）
    let st = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = st.db.lock().map_err(|e| e.to_string())?;
        let item = db::get_item(&conn, id)?.ok_or("条目不存在")?;
        copy_item_to_clipboard(&st, &item)
    })
    .await
    .map_err(|e| format!("复制任务异常: {}", e))??;

    // 2. 隐藏主窗口
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }

    // 3. 模拟 Ctrl+V 粘贴到之前的前台窗口（若已失效则当前前台）
    let target = super::monitor_take_previous_window();
    paste::simulate_paste(target)
}

/// 将一条历史项写入剪贴板（CopyQ 式：还原复制时的全部格式）
fn copy_item_to_clipboard(state: &ClipboardState, item: &ClipboardItem) -> Result<(), String> {
    match item.kind.as_str() {
        "text" => {
            let text = item.content.clone().unwrap_or_default();
            let mut entries: Vec<(u32, Vec<u8>)> = Vec::new();
            // 主格式：纯文本
            entries.push((super::CF_UNICODETEXT, paste::text_to_utf16_bytes(&text)));
            // 附加格式：HTML / RTF 原样写回（富文本粘贴保持样式）
            let conn = state.db.lock().map_err(|e| e.to_string())?;
            let extra = db::get_item_formats(&conn, item.id)?;
            drop(conn);
            for (mime, data) in extra {
                if let Some(fmt) = paste::format_id_for_mime(&mime) {
                    entries.push((fmt, data));
                }
            }
            paste::write_multi(&entries)
        }
        "image" => {
            let rel = item.image_path.clone().ok_or("图片路径缺失")?;
            let full = db::image_file_path(&state.data_dir, &rel);
            if !full.exists() {
                return Err("图片文件不存在".into());
            }
            let png_bytes = std::fs::read(&full).map_err(|e| format!("读取图片失败: {}", e))?;
            let img = image::open(&full).map_err(|e| format!("读取图片失败: {}", e))?;
            let rgba = img.to_rgba8();
            let (w, h) = (rgba.width(), rgba.height());
            let mut entries: Vec<(u32, Vec<u8>)> = Vec::new();
            // 主格式：PNG 注册格式（目标程序优先取用，透明/质量无损）
            let png_fmt = paste::register_format("PNG");
            if png_fmt != 0 {
                entries.push((png_fmt, png_bytes));
            }
            // 兼容格式：CF_DIB（老程序只认 DIB 位图）
            entries.push((super::CF_DIB, paste::rgba_to_dib(&rgba, w, h)));
            paste::write_multi(&entries)
        }
        _ => Err(format!("未知条目类型: {}", item.kind)),
    }
}

// ---------------------------------------------------------------------------
// 设置
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn clipboard_get_settings(state: State<'_, ClipboardState>) -> Result<ClipboardSettings, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::load_settings(&conn)
}

#[tauri::command]
pub fn clipboard_save_settings(
    state: State<'_, ClipboardState>,
    settings: ClipboardSettings,
) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::save_settings(&conn, &settings)
}

// ---------------------------------------------------------------------------
// 忽略规则
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn clipboard_get_ignored_apps(state: State<'_, ClipboardState>) -> Result<Vec<String>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::list_ignored_apps(&conn)
}

#[tauri::command]
pub fn clipboard_add_ignored_app(state: State<'_, ClipboardState>, app: String) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::add_ignored_app(&conn, &app)
}

#[tauri::command]
pub fn clipboard_remove_ignored_app(state: State<'_, ClipboardState>, app: String) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::remove_ignored_app(&conn, &app)
}

// ---------------------------------------------------------------------------
// 其他
// ---------------------------------------------------------------------------

/// 唤起主窗口时由前端调用：记录当前前台窗口（供一键粘贴）
#[tauri::command]
pub fn clipboard_remember_window() -> Result<(), String> {
    super::monitor_remember_window();
    Ok(())
}

/// 读取图片为 base64 data-url（供前端预览）
#[tauri::command]
pub fn clipboard_get_image(
    state: State<'_, ClipboardState>,
    id: i64,
    thumb: Option<bool>,
) -> Result<String, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let item = db::get_item(&conn, id)?.ok_or("条目不存在")?;
    if item.kind != "image" {
        return Err("非图片条目".into());
    }
    let rel = if thumb.unwrap_or(true) {
        item.thumb_path.clone().or(item.image_path.clone())
    } else {
        item.image_path.clone()
    }
    .ok_or("图片路径缺失")?;
    let full = db::image_file_path(&state.data_dir, &rel);
    let bytes = std::fs::read(&full).map_err(|e| format!("读取图片失败: {}", e))?;
    Ok(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    ))
}
