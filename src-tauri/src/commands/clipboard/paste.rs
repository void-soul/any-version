//! 剪贴板写入与一键粘贴
//!
//! - `write_multi`：CopyQ 式多格式写回（一次写入多种格式，目标程序按需取用）
//! - `simulate_paste`：SendInput 模拟 Ctrl+V 到指定（或当前前台）窗口

/// 将多组 (格式ID, 原始数据) 一次性写入剪贴板（CopyQ 式）。
/// 先 EmptyClipboard 再逐个 SetClipboardData；单个格式失败不影响其余格式。
pub fn write_multi(entries: &[(u32, Vec<u8>)]) -> Result<(), String> {
    use windows_sys::Win32::System::DataExchange::*;
    use windows_sys::Win32::System::Memory::*;

    if entries.is_empty() {
        return Ok(());
    }
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::GetDesktopWindow;
        // OpenClipboard 是阻塞调用，若剪贴板被其它程序（含本应用自己的监听线程）
        // 占用会失败。重试最多 30 次、每次间隔 10ms（共 ~300ms），重试前先尝试
        // CloseClipboard 清理本进程可能残留的打开状态；仍失败则返回明确错误（含
        // GetLastError，便于区分"被占用"与"权限不足/拒绝访问"）。
        let mut opened = false;
        let mut last_err = 0u32;
        for _ in 0..30 {
            // 优先用 NULL owner；管理员高完整性场景下有时需要指定 owner 窗口
            if OpenClipboard(std::ptr::null_mut()) != 0 {
                opened = true;
                break;
            }
            last_err = windows_sys::Win32::Foundation::GetLastError();
            // 清理本进程可能残留的打开状态，给监听线程让出剪贴板
            let _ = CloseClipboard();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if !opened {
            // 兜底：用桌面窗口作为 owner 再试一次（极少数会话下 NULL 会失败）
            if OpenClipboard(GetDesktopWindow()) != 0 {
                opened = true;
            } else {
                last_err = windows_sys::Win32::Foundation::GetLastError();
            }
        }
        if !opened {
            crate::exit_log::exit_log(&format!(
                "write_multi OpenClipboard 失败: last_err={} ({})",
                last_err,
                std::io::Error::from_raw_os_error(last_err as i32)
            ));
            return Err(format!(
                "打开剪贴板失败(被占用或权限不足): {} (code {})",
                std::io::Error::from_raw_os_error(last_err as i32),
                last_err
            ));
        }
        if EmptyClipboard() == 0 {
            crate::exit_log::exit_log(&format!(
                "write_multi EmptyClipboard 失败: {}",
                std::io::Error::last_os_error()
            ));
        }
        for (fmt, data) in entries {
            let h = GlobalAlloc(GMEM_MOVEABLE, data.len());
            if h.is_null() {
                CloseClipboard();
                return Err("分配剪贴板内存失败".into());
            }
            let ptr = GlobalLock(h);
            if ptr.is_null() {
                // 失败路径不释放：GlobalFree 在 Windows 10+ 已是 no-op
                CloseClipboard();
                return Err("锁定剪贴板内存失败".into());
            }
            std::ptr::copy_nonoverlapping(data.as_ptr(), ptr as *mut u8, data.len());
            GlobalUnlock(h);
            // SetClipboardData 失败时内存归系统管理（Win10+ GlobalFree 为 no-op），
            // 记录警告并继续尝试其余格式
            if SetClipboardData(*fmt, h).is_null() {
                tracing::warn!(
                    "设置剪贴板格式 {} 失败: {}",
                    *fmt,
                    std::io::Error::last_os_error()
                );
            }
        }
        CloseClipboard();
    }
    Ok(())
}

/// 文本 → UTF-16LE 字节（带结尾 null），用于 CF_UNICODETEXT
pub fn text_to_utf16_bytes(text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len() * 2 + 2);
    for u in text.encode_utf16().chain(std::iter::once(0)) {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    bytes
}

/// 注册剪贴板格式名 → 格式 ID（"HTML Format"、"Rich Text Format"、"PNG" 等）
pub fn register_format(name: &str) -> u32 {
    use windows_sys::Win32::System::DataExchange::RegisterClipboardFormatW;
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe { RegisterClipboardFormatW(wide.as_ptr()) }
}

/// 将 mime 名映射为写回用的格式 ID（仅支持捕获时保存过的格式）
pub fn format_id_for_mime(mime: &str) -> Option<u32> {
    match mime {
        "text/html" => Some(register_format("HTML Format")),
        "application/rtf" => Some(register_format("Rich Text Format")),
        _ => None,
    }
}

/// 将 RGBA 像素构造为 DIB 字节（32bpp BI_RGB，top-down），用于 CF_DIB
pub fn rgba_to_dib(rgba: &image::RgbaImage, w: u32, h: u32) -> Vec<u8> {
    let header_size = 40usize;
    let row_size = w as usize * 4;
    let pixel_size = row_size * h as usize;
    let total = header_size + pixel_size;
    let mut dib = Vec::<u8>::with_capacity(total);
    dib.resize(total, 0);
    dib[0..4].copy_from_slice(&(40u32).to_le_bytes());
    dib[4..8].copy_from_slice(&(w as i32).to_le_bytes());
    dib[8..12].copy_from_slice(&(-(h as i32)).to_le_bytes()); // top-down
    dib[12..14].copy_from_slice(&1u16.to_le_bytes()); // planes
    dib[14..16].copy_from_slice(&32u16.to_le_bytes()); // bpp
    dib[16..20].copy_from_slice(&0u32.to_le_bytes()); // BI_RGB
    dib[20..24].copy_from_slice(&(pixel_size as u32).to_le_bytes());
    for y in 0..h as usize {
        for x in 0..w as usize {
            let dst = header_size + y * row_size + x * 4;
            let px = rgba.get_pixel(x as u32, y as u32);
            dib[dst] = px[2]; // B
            dib[dst + 1] = px[1]; // G
            dib[dst + 2] = px[0]; // R
            dib[dst + 3] = px[3]; // A
        }
    }
    dib
}

/// 模拟 Ctrl+V 粘贴到指定窗口（若无则当前前台窗口）
pub fn simulate_paste(target: Option<windows_sys::Win32::Foundation::HWND>) -> Result<(), String> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, SetForegroundWindow};

    unsafe {
        // 若提供了目标窗口，先尝试将其置为前台
        if let Some(hwnd) = target {
            if !hwnd.is_null() {
                let fg = GetForegroundWindow();
                if fg != hwnd {
                    // 通过模拟 Alt 键解决 SetForegroundWindow 限制
                    let mut alt: INPUT = std::mem::zeroed();
                    alt.r#type = INPUT_KEYBOARD;
                    alt.Anonymous.ki.wVk = VK_MENU as u16;
                    SendInput(1, &alt, std::mem::size_of::<INPUT>() as i32);
                    SetForegroundWindow(hwnd);
                    let mut alt_up: INPUT = std::mem::zeroed();
                    alt_up.r#type = INPUT_KEYBOARD;
                    alt_up.Anonymous.ki.wVk = VK_MENU as u16;
                    alt_up.Anonymous.ki.dwFlags = KEYEVENTF_KEYUP;
                    SendInput(1, &alt_up, std::mem::size_of::<INPUT>() as i32);
                }
            }
        }

        // 发送 Ctrl+V
        let mut inputs: Vec<INPUT> = Vec::with_capacity(4);
        // Ctrl down
        let mut ctrl: INPUT = std::mem::zeroed();
        ctrl.r#type = INPUT_KEYBOARD;
        ctrl.Anonymous.ki.wVk = VK_CONTROL as u16;
        inputs.push(ctrl);
        // V down
        let mut v_down: INPUT = std::mem::zeroed();
        v_down.r#type = INPUT_KEYBOARD;
        v_down.Anonymous.ki.wVk = b'V' as u16;
        inputs.push(v_down);
        // V up
        let mut v_up: INPUT = std::mem::zeroed();
        v_up.r#type = INPUT_KEYBOARD;
        v_up.Anonymous.ki.wVk = b'V' as u16;
        v_up.Anonymous.ki.dwFlags = KEYEVENTF_KEYUP;
        inputs.push(v_up);
        // Ctrl up
        let mut ctrl_up: INPUT = std::mem::zeroed();
        ctrl_up.r#type = INPUT_KEYBOARD;
        ctrl_up.Anonymous.ki.wVk = VK_CONTROL as u16;
        ctrl_up.Anonymous.ki.dwFlags = KEYEVENTF_KEYUP;
        inputs.push(ctrl_up);

        let sent = SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
        if sent == 0 {
            return Err(format!("发送粘贴按键失败: {}", std::io::Error::last_os_error()));
        }
    }
    Ok(())
}
