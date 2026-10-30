use super::*;

#[cfg(windows)]
pub(super) fn cursor_position() -> Option<(i32, i32)> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut pt = POINT { x: 0, y: 0 };
    if unsafe { GetCursorPos(&mut pt) } != 0 {
        Some((pt.x, pt.y))
    } else {
        None
    }
}

#[cfg(windows)]
pub(super) fn cursor_work_area() -> Option<(i32, i32, i32, i32)> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    let (cx, cy) = cursor_position()?;
    let hmon = unsafe { MonitorFromPoint(POINT { x: cx, y: cy }, MONITOR_DEFAULTTONEAREST) };
    if hmon.is_null() {
        return None;
    }
    let mut mi: MONITORINFO = unsafe { std::mem::zeroed() };
    mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    if unsafe { GetMonitorInfoW(hmon, &mut mi) } == 0 {
        return None;
    }
    let r = mi.rcWork;
    Some((r.left, r.top, r.right, r.bottom))
}

/// 悬浮窗定位：鼠标所在显示器水平居中、垂直位于上方 16%（Spotlight 式）。
#[cfg(windows)]
pub(super) fn focused_position(win_w: i32, win_h: i32) -> Option<(i32, i32)> {
    let (left, top, right, bottom) = cursor_work_area()?;
    let x = left + ((right - left) - win_w) / 2;
    let y = top + ((bottom - top) as f64 * 0.16) as i32;
    Some((x.max(left), y.max(top).min((bottom - win_h).max(top))))
}

#[cfg(not(windows))]
pub(super) fn focused_position(_win_w: i32, _win_h: i32) -> Option<(i32, i32)> {
    None
}

pub(super) fn ensure_popup(app: &tauri::AppHandle) -> Result<tauri::WebviewWindow, String> {
    if let Some(win) = app.get_webview_window(POPUP_LABEL) {
        return Ok(win);
    }
    let mut builder = tauri::WebviewWindowBuilder::new(
        app,
        POPUP_LABEL,
        tauri::WebviewUrl::App("index.html?popup=file-search".into()),
    )
    .title("文件搜索")
    .inner_size(POPUP_W, POPUP_H)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(true)
    .maximizable(false)
    .minimizable(false)
    .shadow(false);
    if let Some((x, y)) = focused_position(POPUP_W as i32, POPUP_H as i32) {
        builder = builder.position(x as f64, y as f64);
    }
    builder
        .build()
        .map_err(|e| format!("创建文件搜索悬浮窗失败: {}", e))
}

/// 呼出 / 收起文件搜索悬浮窗（已聚焦时再按热键则隐藏）。
pub fn open_search_popup(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window(POPUP_LABEL) {
        if win.is_focused().unwrap_or(false) {
            let _ = win.hide();
            return Ok(());
        }
    }
    ensure_popup(&app)?;
    let app_main = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = app_main.get_webview_window(POPUP_LABEL) {
            let _ = w.show();
            #[cfg(windows)]
            if let Some((x, y)) = focused_position(POPUP_W as i32, POPUP_H as i32) {
                let _ = w.set_position(tauri::PhysicalPosition::new(x, y));
            }
            let _ = w.set_focus();
            let webview: &tauri::Webview<tauri::Wry> = w.as_ref();
            let _ = webview.set_focus();
            // 通知前端重新聚焦输入框（窗口复用时不会重新挂载组件）。
            let _ = w.emit("file-search-focus", ());
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn open_file_search_popup(app: tauri::AppHandle) -> Result<(), String> {
    open_search_popup(app)
}

#[tauri::command]
pub async fn hide_file_search_popup(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window(POPUP_LABEL) {
        let _ = win.hide();
    }
    Ok(())
}
