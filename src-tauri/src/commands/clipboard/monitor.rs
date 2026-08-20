//! Windows 剪贴板后台监控线程
//!
//! 采用与 CopyQ 相同的 `AddClipboardFormatListener` 方案：
//! 创建 message-only 隐藏窗口并注册剪贴板监听，收到 `WM_CLIPBOARDUPDATE`
//! 后异步读取剪贴板内容（文本/图片），去重后写入历史 DB 并向前端发送
//! `clipboard-updated` 事件。

use std::sync::Arc;
use tauri::{Emitter, Manager};

use super::db;
use super::images;

// ---------------------------------------------------------------------------
// 全局：唤起窗口前记录前台窗口（供一键粘贴）
// ---------------------------------------------------------------------------

static PREVIOUS_FOREGROUND: std::sync::atomic::AtomicIsize =
    std::sync::atomic::AtomicIsize::new(0);

/// 记录当前前台窗口（在全局热键唤起主窗口之前调用，供一键粘贴使用）
pub(crate) fn remember_previous_window() {
    let hwnd = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
    PREVIOUS_FOREGROUND.store(hwnd as isize, std::sync::atomic::Ordering::SeqCst);
}

/// 读取记录的「唤起前的前台窗口」
pub(crate) fn take_previous_window() -> Option<windows_sys::Win32::Foundation::HWND> {
    let v = PREVIOUS_FOREGROUND.swap(0, std::sync::atomic::Ordering::SeqCst);
    if v == 0 {
        return None;
    }
    let hwnd = v as isize as *mut core::ffi::c_void;
    // 确认窗口仍有效
    let alive = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::IsWindow(hwnd) } != 0;
    if alive {
        Some(hwnd)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// 窗口过程
// ---------------------------------------------------------------------------

/// 创建 message-only 监听窗口并启动消息循环。
/// 该线程与主线程完全独立，AppHandle 由窗口 userdata 携带。
pub fn spawn_monitor(app: tauri::AppHandle) {
    std::thread::Builder::new()
        .name("clipboard-monitor".into())
        .spawn(move || {
            if let Err(e) = run_monitor_loop(app.clone()) {
                tracing::warn!("剪贴板监控线程退出: {}", e);
            }
        })
        .ok();
}

fn run_monitor_loop(app: tauri::AppHandle) -> Result<(), String> {
    use windows_sys::Win32::System::DataExchange::{AddClipboardFormatListener, RemoveClipboardFormatListener};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    let instance = unsafe { GetModuleHandleW(std::ptr::null()) };

    // 注册窗口类
    let class_name: Vec<u16> = "AnyVersionClipboardMonitor\0".encode_utf16().collect();
    let wc = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(monitor_wnd_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance,
        hIcon: std::ptr::null_mut(),
        hCursor: std::ptr::null_mut(),
        hbrBackground: std::ptr::null_mut(),
        lpszMenuName: std::ptr::null(),
        lpszClassName: class_name.as_ptr(),
    };
    let atom = unsafe { RegisterClassW(&wc) };
    if atom == 0 {
        return Err(format!("注册剪贴板监听窗口类失败: {}", std::io::Error::last_os_error()));
    }

    // 创建 message-only 窗口
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            class_name.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        )
    };
    if hwnd.is_null() {
        return Err(format!("创建剪贴板监听窗口失败: {}", std::io::Error::last_os_error()));
    }

    // 通过窗口 userdata 携带 AppHandle，供 WndProc 使用
    let app_box: Box<Arc<tauri::AppHandle>> = Box::new(Arc::new(app.clone()));
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(app_box) as isize);
    }

    // 注册剪贴板监听
    let ok = unsafe { AddClipboardFormatListener(hwnd) };
    if ok == 0 {
        return Err(format!("AddClipboardFormatListener 失败: {}", std::io::Error::last_os_error()));
    }

    tracing::info!("剪贴板监控已启动 (hwnd={:p})", hwnd);

    // 消息循环
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    unsafe {
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        RemoveClipboardFormatListener(hwnd);
        // 释放 userdata
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Arc<tauri::AppHandle>;
        if !ptr.is_null() {
            drop(Box::from_raw(ptr));
        }
        DestroyWindow(hwnd);
    }
    Ok(())
}

unsafe extern "system" fn monitor_wnd_proc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    lparam: windows_sys::Win32::Foundation::LPARAM,
) -> windows_sys::Win32::Foundation::LRESULT {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    if msg == WM_CLIPBOARDUPDATE {
        // 取出 AppHandle 处理剪贴板更新（不阻塞消息循环太久，读取较快）
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Arc<tauri::AppHandle>;
        if !ptr.is_null() {
            let app = &**ptr;
            handle_clipboard_update(app);
        }
        return 0;
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

// ---------------------------------------------------------------------------
// 剪贴板更新处理
// ---------------------------------------------------------------------------

fn handle_clipboard_update(app: &tauri::AppHandle) {
    let state = match app.try_state::<super::ClipboardState>() {
        Some(s) => s.clone(),
        None => return,
    };
    let settings = {
        let conn = state.db.lock().unwrap();
        match db::load_settings(&conn) {
            Ok(s) => s,
            Err(_) => super::ClipboardSettings::default(),
        }
    };
    if !settings.enabled {
        return;
    }

    // 读取来源程序（尽力而为：剪贴板更新事件触发时前台窗口通常是复制发起者）
    let source_app = get_foreground_app_name();

    // 读取剪贴板内容（优先文本，其次图片）
    let captured = read_clipboard_content(app, &settings, source_app.as_deref());

    let Ok(Some((formats, extra, content))) = captured else {
        return;
    };

    // 计算去重 hash（Png/Rgba 统一基于像素，同一张图无论以 PNG 还是 DIB 捕获都命中同一 hash）
    let hash = match &content {
        Capture::Text(t) => format!("text:{:x}", md5::compute(t.as_bytes())),
        Capture::Png(_, rgba, w, h) | Capture::Rgba(rgba, w, h) => {
            let mut c = md5::Context::new();
            c.consume(&(w ^ h).to_le_bytes());
            c.consume(rgba);
            format!("image:{:x}", c.compute())
        }
    };
    let src = source_app.unwrap_or_default();

    let conn = state.db.lock().unwrap();
    let now = chrono::Utc::now().timestamp();

    match db::find_by_hash(&conn, &hash) {
        Ok(Some((id, pinned))) => {
            if !pinned {
                // 非置顶：更新来源与时间（移到最前）
                let _ = conn.execute(
                    "UPDATE clipboard_items SET source_app=?1, created_at=?2 WHERE id=?3",
                    rusqlite::params![src, now, id],
                );
            }
        }
        Ok(None) => {
            let insert_result = match &content {
                Capture::Text(t) => db::insert_item(
                    &conn,
                    "text",
                    Some(t),
                    None,
                    None,
                    0,
                    0,
                    &src,
                    &hash,
                    now,
                    &formats,
                ),
                Capture::Png(png, _, w, h) => {
                    let stem = images::new_stem();
                    let dir = super::images_dir();
                    match images::save_png_bytes(&dir, &stem, png, 256) {
                        Ok((full_rel, thumb_rel)) => db::insert_item(
                            &conn,
                            "image",
                            None,
                            Some(&full_rel),
                            Some(&thumb_rel),
                            *w as i64,
                            *h as i64,
                            &src,
                            &hash,
                            now,
                            &formats,
                        ),
                        Err(e) => {
                            tracing::warn!("保存剪贴板图片失败: {}", e);
                            return;
                        }
                    }
                }
                Capture::Rgba(rgba, w, h) => {
                    let stem = images::new_stem();
                    let dir = super::images_dir();
                    match images::save_rgba_png(&dir, &stem, *w, *h, rgba, 256) {
                        Ok((full_rel, thumb_rel)) => db::insert_item(
                            &conn,
                            "image",
                            None,
                            Some(&full_rel),
                            Some(&thumb_rel),
                            *w as i64,
                            *h as i64,
                            &src,
                            &hash,
                            now,
                            &formats,
                        ),
                        Err(e) => {
                            tracing::warn!("保存剪贴板图片失败: {}", e);
                            return;
                        }
                    }
                }
            };
            if let Ok(id) = insert_result {
                // 保存附加格式数据（HTML/RTF），粘贴时按原格式写回
                if !extra.is_empty() {
                    let _ = db::insert_item_formats(&conn, id, &extra);
                }
                // 裁剪历史到上限（保留置顶），并清理被裁剪掉的孤儿图片文件与格式数据
                if settings.max_items > 0 {
                    let _ = db::trim_history(&conn, settings.max_items);
                    let _ = db::cleanup_orphan_images(&conn, &super::images_dir());
                    let _ = db::cleanup_orphan_formats(&conn);
                }
            }
        }
        Err(_) => return,
    }
    drop(conn);

    let _ = app.emit("clipboard-updated", ());
}

// ---------------------------------------------------------------------------
// 剪贴板读取
// ---------------------------------------------------------------------------

enum Capture {
    Text(String),
    /// 原始 PNG 字节 + rgba 像素 + 宽高（剪贴板里原本就是 PNG 时使用，粘贴可原样还原）
    Png(Vec<u8>, Vec<u8>, u32, u32),
    Rgba(Vec<u8>, u32, u32), // rgba, width, height
}

fn read_clipboard_content(
    _app: &tauri::AppHandle,
    settings: &super::ClipboardSettings,
    source_app: Option<&str>,
) -> Result<Option<(Vec<String>, Vec<(String, Vec<u8>)>, Capture)>, String> {
    use windows_sys::Win32::System::DataExchange::*;

    // 忽略规则检查（来源程序被忽略则跳过）
    if let Some(app_name) = source_app {
        let state = _app
            .try_state::<super::ClipboardState>()
            .map(|s| s.clone())
            .ok_or("无剪贴板状态")?;
        let conn = state.db.lock().unwrap();
        if db::is_app_ignored(&conn, app_name).unwrap_or(false) {
            return Ok(None);
        }
    }

    // 尝试打开剪贴板（可能被其他程序占用，最多重试几次）
    let mut hwnd_opened = false;
    for _ in 0..5 {
        if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
            hwnd_opened = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    if !hwnd_opened {
        return Ok(None);
    }

    let result = (|| -> Result<Option<(Vec<String>, Vec<(String, Vec<u8>)>, Capture)>, String> {
        // 记录这次复制时剪贴板包含的全部格式（CopyQ 式）
        let formats = super::formats::enumerate_clipboard_formats();

        // 1. 文本优先
        let text_available = unsafe { IsClipboardFormatAvailable(super::CF_UNICODETEXT) } != 0;
        if text_available {
            if let Some(text) = read_unicode_text()? {
                let text = normalize_text(&text);
                if !text.is_empty() {
                    // 空白过滤
                    if settings.ignore_blank && text.trim().is_empty() {
                        return Ok(None);
                    }
                    if settings.ignore_short && text.trim().len() <= 2 {
                        return Ok(None);
                    }
                    // 附加格式：HTML / RTF（粘贴时按原格式写回，保留富文本样式）
                    let mut extra = Vec::new();
                    if let Some(d) = read_registered_format("HTML Format") {
                        extra.push(("text/html".to_string(), d));
                    }
                    if let Some(d) = read_registered_format("Rich Text Format") {
                        extra.push(("application/rtf".to_string(), d));
                    }
                    return Ok(Some((formats, extra, Capture::Text(text))));
                }
            }
        }

        // 2. 图片（需开启 store_images）
        //    a) 剪贴板里原本有 PNG → 原样保留（粘贴可还原 PNG，透明/质量无损）
        //    b) 否则读 CF_DIBV5 / CF_DIB 转成 PNG
        if settings.store_images {
            if let Some(png_bytes) = read_registered_format("PNG") {
                if let Ok(img) = image::load_from_memory(&png_bytes) {
                    let rgba = img.to_rgba8();
                    let (w, h) = rgba.dimensions();
                    return Ok(Some((formats, Vec::new(), Capture::Png(
                        png_bytes, rgba.to_vec(), w, h,
                    ))));
                }
            }

            let dib_available = unsafe { IsClipboardFormatAvailable(super::CF_DIB) } != 0
                || unsafe { IsClipboardFormatAvailable(super::CF_DIBV5) } != 0;
            if dib_available {
                if let Some((w, raw_h, bpp, compression, masks, pixels)) = read_dib_data()? {
                    if let Some((w2, h2, rgba)) =
                        images::dib_to_rgba(w, raw_h, bpp, compression, masks, &pixels)
                    {
                        return Ok(Some((formats, Vec::new(), Capture::Rgba(rgba, w2, h2))));
                    }
                }
            }
        }

        Ok(None)
    })();

    unsafe {
        CloseClipboard();
    }
    result
}

/// 读取 CF_UNICODETEXT 文本
fn read_unicode_text() -> Result<Option<String>, String> {
    use windows_sys::Win32::System::DataExchange::GetClipboardData;
    use windows_sys::Win32::System::Memory::*;

    let h = unsafe { GetClipboardData(super::CF_UNICODETEXT) };
    if h.is_null() {
        return Ok(None);
    }
    unsafe {
        let ptr = GlobalLock(h);
        if ptr.is_null() {
            return Ok(None);
        }
        let size = GlobalSize(h) as usize;
        if size < 2 {
            GlobalUnlock(h);
            return Ok(None);
        }
        let u16_len = size / 2;
        let buf = std::slice::from_raw_parts(ptr as *const u16, u16_len);
        // 去掉结尾 null
        let mut end = buf.len();
        while end > 0 && buf[end - 1] == 0 {
            end -= 1;
        }
        let s = String::from_utf16_lossy(&buf[..end]);
        GlobalUnlock(h);
        Ok(Some(s))
    }
}

/// 读取注册格式的原始数据（如 "HTML Format"、"Rich Text Format"、"PNG"）。
/// 必须在 OpenClipboard 之后调用。
fn read_registered_format(name: &str) -> Option<Vec<u8>> {
    use windows_sys::Win32::System::DataExchange::{GetClipboardData, RegisterClipboardFormatW};
    use windows_sys::Win32::System::Memory::*;

    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let fmt = unsafe { RegisterClipboardFormatW(wide.as_ptr()) };
    if fmt == 0 {
        return None;
    }
    let h = unsafe { GetClipboardData(fmt) };
    if h.is_null() {
        return None;
    }
    unsafe {
        let ptr = GlobalLock(h);
        if ptr.is_null() {
            return None;
        }
        let size = GlobalSize(h) as usize;
        if size == 0 {
            GlobalUnlock(h);
            return None;
        }
        let data = std::slice::from_raw_parts(ptr as *const u8, size).to_vec();
        GlobalUnlock(h);
        Some(data)
    }
}

/// 读取位图数据：优先 CF_DIBV5（微信/截图软件主格式），回退 CF_DIB
///
/// 返回 (width, raw_height, bpp, compression, [r,g,b,a]掩码, pixels)
fn read_dib_data() -> Result<Option<(i32, i32, u16, u32, [u32; 4], Vec<u8>)>, String> {
    use windows_sys::Win32::System::DataExchange::IsClipboardFormatAvailable;

    for fmt in [super::CF_DIBV5, super::CF_DIB] {
        if unsafe { IsClipboardFormatAvailable(fmt) } != 0 {
            if let Some(v) = read_dib_format(fmt)? {
                return Ok(Some(v));
            }
        }
    }
    Ok(None)
}

/// 读取指定 DIB 格式的数据并解析头部
fn read_dib_format(fmt: u32) -> Result<Option<(i32, i32, u16, u32, [u32; 4], Vec<u8>)>, String> {
    use windows_sys::Win32::System::DataExchange::GetClipboardData;
    use windows_sys::Win32::System::Memory::*;

    let h = unsafe { GetClipboardData(fmt) };
    if h.is_null() {
        return Ok(None);
    }
    unsafe {
        let ptr = GlobalLock(h);
        if ptr.is_null() {
            return Ok(None);
        }
        let size = GlobalSize(h) as usize;
        if size < 40 {
            GlobalUnlock(h);
            return Ok(None);
        }
        let data = std::slice::from_raw_parts(ptr as *const u8, size);
        // 可能以 BITMAPFILEHEADER（"BM"）开头，跳过 14 字节
        let mut off = 0usize;
        if size >= 14 && data[0] == b'B' && data[1] == b'M' {
            off = 14;
        }
        if size - off < 40 {
            GlobalUnlock(h);
            return Ok(None);
        }
        let d = &data[off..];
        let header_size = u32::from_le_bytes([d[0], d[1], d[2], d[3]]) as usize;
        if header_size < 40 {
            GlobalUnlock(h);
            return Ok(None);
        }
        let width = i32::from_le_bytes([d[4], d[5], d[6], d[7]]);
        let raw_height = i32::from_le_bytes([d[8], d[9], d[10], d[11]]);
        let bpp = u16::from_le_bytes([d[14], d[15]]);
        let compression = u32::from_le_bytes([d[16], d[17], d[18], d[19]]);

        // BI_BITFIELDS=3：解析通道掩码
        // - BITMAPV5HEADER（header_size>=52，实际为 108/124）：掩码在头内 offset 40..56
        // - BITMAPINFOHEADER（40 字节）：掩码紧跟头部，像素从 off+40+12 开始
        let mut masks = [0u32; 4];
        let mut pixel_start = off + header_size;
        if compression == 3 {
            if header_size >= 52 && d.len() >= 56 {
                masks[0] = u32::from_le_bytes([d[40], d[41], d[42], d[43]]);
                masks[1] = u32::from_le_bytes([d[44], d[45], d[46], d[47]]);
                masks[2] = u32::from_le_bytes([d[48], d[49], d[50], d[51]]);
                masks[3] = u32::from_le_bytes([d[52], d[53], d[54], d[55]]);
            } else if data.len() >= off + 40 + 12 {
                masks[0] = u32::from_le_bytes(data[off + 40..off + 44].try_into().unwrap());
                masks[1] = u32::from_le_bytes(data[off + 44..off + 48].try_into().unwrap());
                masks[2] = u32::from_le_bytes(data[off + 48..off + 52].try_into().unwrap());
                pixel_start = off + 40 + 12;
            }
        }

        if pixel_start > data.len() {
            GlobalUnlock(h);
            return Ok(None);
        }
        let pixels = data[pixel_start..].to_vec();
        GlobalUnlock(h);
        Ok(Some((width, raw_height, bpp, compression, masks, pixels)))
    }
}

/// 文本规范化：去首尾空白后若仍有换行等保留原样；纯空白返回空
fn normalize_text(s: &str) -> String {
    let t = s.trim();
    if t.is_empty() {
        String::new()
    } else {
        s.to_string()
    }
}

/// 获取当前前台窗口的进程可执行文件名（如 "chrome.exe"）
fn get_foreground_app_name() -> Option<String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return None;
        }
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == 0 {
            return None;
        }
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut len);
        CloseHandle(handle);
        if ok == 0 {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        Some(
            std::path::Path::new(&path)
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or(path),
        )
    }
}
