//! 剪贴板格式枚举与名称映射（CopyQ 式 MIME 名称）
//!
//! 在剪贴板已打开（`OpenClipboard`）后调用 `enumerate_clipboard_formats()`
//! 列出当时剪贴板包含的全部格式，将标准格式映射为 MIME 风格名称
//! （如 `text/plain;charset=utf-8`、`image/bmp`），注册格式按名字保留
//! （如 `PNG` → `image/png`、`HTML Format` → `text/html`）。

use windows_sys::Win32::System::DataExchange::{EnumClipboardFormats, GetClipboardFormatNameW};

// 标准剪贴板格式
const CF_TEXT: u32 = 1;
const CF_BITMAP: u32 = 2;
const CF_METAFILEPICT: u32 = 3;
const CF_SYLK: u32 = 4;
const CF_DIF: u32 = 5;
const CF_TIFF: u32 = 6;
const CF_OEMTEXT: u32 = 7;
const CF_DIB: u32 = 8;
const CF_PALETTE: u32 = 9;
const CF_PENDATA: u32 = 10;
const CF_RIFF: u32 = 11;
const CF_WAVE: u32 = 12;
const CF_UNICODETEXT: u32 = 13;
const CF_ENHMETAFILE: u32 = 14;
const CF_HDROP: u32 = 15;
const CF_LOCALE: u32 = 16;
const CF_DIBV5: u32 = 17;
const CF_OWNERDISPLAY: u32 = 0x0080;

/// 枚举剪贴板当前包含的全部格式，返回去重后的 MIME 风格名称列表。
/// 必须在 `OpenClipboard` 之后、`CloseClipboard` 之前调用。
pub fn enumerate_clipboard_formats() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur: u32 = 0;
    loop {
        cur = unsafe { EnumClipboardFormats(cur) };
        if cur == 0 {
            break;
        }
        if let Some(name) = format_name(cur) {
            if !out.contains(&name) {
                out.push(name);
            }
        }
    }
    // 稳定排序：text/plain 系最前，其次 text/html，再图片，其余按枚举顺序
    out.sort_by_key(|n| priority(n));
    out
}

/// 展示优先级（越小越靠前）
fn priority(name: &str) -> u8 {
    if name.starts_with("text/plain") {
        0
    } else if name == "text/html" {
        1
    } else if name.starts_with("image/") {
        2
    } else {
        3
    }
}

/// 将格式 ID 映射为 MIME 风格名称；不关心的格式返回 None
fn format_name(fmt: u32) -> Option<String> {
    if fmt >= 0xC000 {
        return registered_format_name(fmt);
    }
    let std_name = match fmt {
        CF_TEXT => "text/plain;charset=iso-8859-1",
        CF_OEMTEXT => "text/plain;charset=oem",
        CF_UNICODETEXT => "text/plain;charset=utf-8",
        CF_BITMAP | CF_DIB | CF_DIBV5 => "image/bmp",
        CF_TIFF => "image/tiff",
        CF_HDROP => "text/uri-list",
        CF_ENHMETAFILE => "image/x-emf",
        CF_METAFILEPICT => "image/x-windows-metafile",
        CF_PALETTE => "application/x-color",
        CF_RIFF => "audio/x-riff",
        CF_WAVE => "audio/x-wav",
        CF_LOCALE => "text/locale",
        CF_SYLK => "application/x-sylk",
        CF_DIF => "application/x-dif",
        CF_PENDATA => "application/x-pendata",
        CF_OWNERDISPLAY..=0x008F => return None, // owner-display / 显示私有格式，跳过
        _ => return None,
    };
    Some(std_name.to_string())
}

/// 读取注册格式（>= 0xC000）的名字，并映射常见格式
fn registered_format_name(fmt: u32) -> Option<String> {
    let mut buf = [0u16; 256];
    let len = unsafe { GetClipboardFormatNameW(fmt, buf.as_mut_ptr(), 256) };
    if len == 0 {
        return None;
    }
    let raw = String::from_utf16_lossy(&buf[..len as usize]);
    let mapped = match raw.as_str() {
        "HTML Format" => "text/html".to_string(),
        "PNG" => "image/png".to_string(),
        "JFIF" => "image/jpeg".to_string(),
        "Rich Text Format" => "application/rtf".to_string(),
        _ => raw,
    };
    Some(mapped)
}
