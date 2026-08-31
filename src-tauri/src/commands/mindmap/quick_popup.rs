//! 悬浮窗「思维导图速记」：独立无边框窗口（`index.html?popup=mindmap`），
//! 从全局快捷键呼出后选择/新建导图，把内容记录为节点（子节点/根节点）或贴纸。
//!
//! 与划词翻译悬浮窗（translate.rs）同一模式：复用窗口、置顶、跳过任务栏、
//! 定位到光标附近；前端以 `?popup=mindmap` 只渲染轻量的 MindmapQuickPopup。

use tauri::Manager;

#[cfg(windows)]
use windows_sys::Win32::Foundation::POINT;
#[cfg(windows)]
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
#[cfg(windows)]
use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

const POPUP_LABEL: &str = "mindmap-quick-popup";
const STICKER_POPUP_LABEL: &str = "mindmap-sticker-popup";
const WIN_W: f64 = 520.0;
const WIN_H: f64 = 900.0;

/// 当前鼠标位置（虚拟屏物理像素坐标，多显示器可为负值）。
#[cfg(windows)]
fn cursor_position() -> Option<(i32, i32)> {
    let mut pt = POINT { x: 0, y: 0 };
    if unsafe { GetCursorPos(&mut pt) } != 0 {
        Some((pt.x, pt.y))
    } else {
        None
    }
}

/// 鼠标所在显示器的工作区（不含任务栏），用于把悬浮窗限制在屏幕内。
#[cfg(windows)]
fn cursor_work_area() -> Option<(i32, i32, i32, i32)> {
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

/// 计算悬浮窗位置：默认鼠标右下方（留 12px 边距），放不下则翻转到左上方，
/// 并始终限制在鼠标所在显示器的工作区内。
#[cfg(windows)]
fn popup_position(win_w: i32, win_h: i32) -> Option<(i32, i32)> {
    let (cx, cy) = cursor_position()?;
    const GAP: i32 = 12;
    let mut x = cx + GAP;
    let mut y = cy + GAP;
    if let Some((left, top, right, bottom)) = cursor_work_area() {
        if x + win_w > right {
            x = cx - GAP - win_w;
        }
        if y + win_h > bottom {
            y = cy - GAP - win_h;
        }
        x = x.clamp(left, (right - win_w).max(left));
        y = y.clamp(top, (bottom - win_h).max(top));
    }
    Some((x, y))
}

/// 把悬浮窗移动到鼠标当前位置（复用窗口时调用）。
#[cfg(windows)]
fn position_popup_at_cursor(win: &tauri::WebviewWindow) {
    let size = win.outer_size().unwrap_or(tauri::PhysicalSize::new(WIN_W as u32, WIN_H as u32));
    if let Some((x, y)) = popup_position(size.width as i32, size.height as i32) {
        let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
    }
}

/// 通用：打开悬浮窗（toggle 逻辑 + 定位 + 捕获选区）。
fn open_popup_generic(app: &tauri::AppHandle, label: &str, capture_sel: bool) -> Result<(), String> {
    // toggle: 已打开且聚焦 → 直接隐藏（再按一次热键收起）
    if let Some(win) = app.get_webview_window(label) {
        if win.is_focused().unwrap_or(false) {
            let _ = win.hide();
            return Ok(());
        }
    }
    // 读取前台窗口选中文本，默认填入「内容」——与划词翻译一致。
    if capture_sel { capture_selection(); }
    // 窗口操作必须在 Tauri 主线程执行
    let app_for_main = app.clone();
    let label = label.to_string();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = app_for_main.get_webview_window(&label) {
            let _ = w.show();
            #[cfg(windows)]
            position_popup_at_cursor(&w);
            let _ = w.set_focus();
            // 聚焦 WebView2 内容（与 translate 悬浮窗一致），保证输入框立即可用
            let webview: &tauri::Webview<tauri::Wry> = w.as_ref();
            let _ = webview.set_focus();
        }
    });
    Ok(())
}

/// 确保悬浮窗存在并返回其句柄（存在则复用）。
fn ensure_quick_popup(app: &tauri::AppHandle) -> Result<tauri::WebviewWindow, String> {
    ensure_popup(app, POPUP_LABEL, "index.html?popup=mindmap-node", "思维导图速记")
}

/// 确保贴纸悬浮窗存在并返回其句柄（存在则复用）。
fn ensure_sticker_popup(app: &tauri::AppHandle) -> Result<tauri::WebviewWindow, String> {
    ensure_popup(app, STICKER_POPUP_LABEL, "index.html?popup=mindmap-sticker", "思维导图贴纸")
}

/// 通用悬浮窗创建/复用。
fn ensure_popup(app: &tauri::AppHandle, label: &str, url: &str, title: &str) -> Result<tauri::WebviewWindow, String> {
    if let Some(win) = app.get_webview_window(label) {
        return Ok(win);
    }
    let mut builder = tauri::WebviewWindowBuilder::new(
        app,
        label,
        tauri::WebviewUrl::App(url.into()),
    )
    .title(title)
    .inner_size(WIN_W, WIN_H)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(true)
    .maximizable(false)
    .minimizable(false)
    .shadow(false);
    // 悬浮窗跟随鼠标：创建时定位到光标附近
    #[cfg(windows)]
    if let Some((x, y)) = popup_position(WIN_W as i32, WIN_H as i32) {
        builder = builder.position(x as f64, y as f64);
    }
    let win = builder
        .build()
        .map_err(|e| format!("创建悬浮窗失败: {}", e))?;
    Ok(win)
}

/// 打开（或隐藏已聚焦的）思维导图速记悬浮窗，定位到鼠标附近。
/// 行为切换（toggle）：窗口已存在且聚焦 → 隐藏；否则显示/聚焦并移到光标旁。
/// 由「思维导图速记」全局快捷键触发（与划词翻译热键同一注册机制）。
/// 最近一次呼出时读到的前台选中文本，供前端挂载/聚焦后拉取默认填入「内容」。
static LAST_SELECTION: std::sync::OnceLock<std::sync::Mutex<Option<String>>> =
    std::sync::OnceLock::new();

/// 读取前台窗口选中文本并暂存，供前端拉取。
fn capture_selection() {
    // 先在后台窗口仍聚焦、选中高亮还没被抢走时模拟 Ctrl+C 读选区。
    crate::commands::ai::translate::simulate_copy_selection();
    let selection = {
        std::thread::sleep(std::time::Duration::from_millis(120));
        crate::commands::ai::translate::read_clipboard_text()
    };
    let slot = LAST_SELECTION.get_or_init(|| std::sync::Mutex::new(None));
    if let Some(txt) = selection {
        if !txt.is_empty() {
            *slot.lock().unwrap() = Some(txt);
        }
    }
    crate::exit_log!("[思维导图速记] 已捕获选区文本");
}

/// 前端拉取最近一次呼出时捕获的选中文本（拉取后清空，避免旧文案反复填入）。
#[tauri::command]
pub fn take_mindmap_quick_selection() -> Option<String> {
    let slot = LAST_SELECTION.get_or_init(|| std::sync::Mutex::new(None));
    slot.lock().unwrap().take()
}

#[tauri::command]
pub fn open_mindmap_quick_popup(app: tauri::AppHandle) -> Result<(), String> {
    ensure_quick_popup(&app)?;
    open_popup_generic(&app, POPUP_LABEL, true)
}

/// 隐藏节点速记悬浮窗（前端记录完成后可自动隐藏；Esc 在此窗口内也走这里）。
#[tauri::command]
pub fn hide_mindmap_quick_popup(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window(POPUP_LABEL) {
        let _ = win.hide();
    }
    Ok(())
}

/// 打开贴纸悬浮窗（toggle 逻辑 + 定位 + 捕获选区）。
#[tauri::command]
pub fn open_mindmap_sticker_popup(app: tauri::AppHandle) -> Result<(), String> {
    ensure_sticker_popup(&app)?;
    open_popup_generic(&app, STICKER_POPUP_LABEL, true)
}

/// 隐藏贴纸悬浮窗。
#[tauri::command]
pub fn hide_mindmap_sticker_popup(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window(STICKER_POPUP_LABEL) {
        let _ = win.hide();
    }
    Ok(())
}
