use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::Mutex;
use std::thread;

use base64::{engine::general_purpose, Engine as _};
use image::{ImageBuffer, Rgba};
use tauri::{AppHandle, Emitter, Manager};
use windows_sys::Win32::Foundation::MAX_PATH;
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, SelectObject, BITMAP,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HDC,
};
use windows_sys::Win32::Storage::FileSystem::{SearchPathW, FILE_ATTRIBUTE_NORMAL};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

extern "system" {
    fn GetDC(hwnd: *mut std::ffi::c_void) -> HDC;
    fn ReleaseDC(hwnd: *mut std::ffi::c_void, hdc: HDC) -> i32;
}

use super::models::{AppxItem, Item, ScannedProgram, ShortcutInfo, UrlMetadata};

/// 将 &str 转为 null-terminated wide string
pub fn to_wide_chars(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// 将 null-terminated u16 slice 转为 Rust String
pub fn from_wide_ptr(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut len = 0;
    unsafe {
        while *ptr.add(len) != 0 {
            len += 1;
        }
        let slice = std::slice::from_raw_parts(ptr, len);
        String::from_utf16_lossy(slice)
    }
}

/// 使用 .NET WinForms 文件对话框选择文件，返回原始选中路径（**不跟随 .lnk 快捷方式**）。
/// 相比 tauri-plugin-dialog 的 open（Windows 下会跟随 .lnk 返回其目标，如 wscript.exe），
/// 此命令能拿到 .lnk 原始路径，供 resolve_shortcut 正确解析快捷方式的目标/参数/图标。
/// 通过 `powershell -STA` 的 OpenFileDialog 实现（STA 线程必需，且不解析快捷方式）。
pub fn pick_file() -> Option<String> {
    let ps = r#"
Add-Type -AssemblyName System.Windows.Forms
$d = New-Object System.Windows.Forms.OpenFileDialog
$d.Filter = '应用程序 (*.exe)|*.exe|快捷方式 (*.lnk)|*.lnk|所有文件 (*.*)|*.*'
$d.Multiselect = $false
if ($d.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { Write-Output $d.FileName }
"#;
    let out = crate::commands::hidden_cmd::hidden_cmd("powershell")
        .args(["-NoProfile", "-NonInteractive", "-STA", "-Command", ps])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// 前端当前激活的顶级模块 id（由前端在切换页面时通过 "launcher-active-page" 事件上报）。
/// 用于模块专属热键的「显示/隐藏」来回切换判定：当窗口已显示且正显示该模块时，按热键则隐藏。
static CURRENT_PAGE: LazyLock<Mutex<String>> =
    LazyLock::new(|| Mutex::new(String::from("launcher")));

/// 分类全局快捷键请求聚焦的分类 id（一次性）。面板唤起后取走并定位。
static PENDING_FOCUS_CATEGORY: LazyLock<Mutex<Option<i64>>> = LazyLock::new(|| Mutex::new(None));

/// 热键命令通道：向单一持久热键线程发送「重新注册」命令。
/// 热键的注册与注销必须在同一线程进行（Windows 限制），否则旧热键无法被可靠
/// 注销，导致「换成新快捷键后新旧都生效」。因此用单一常驻线程统一处理注册/注销。
static HOTKEY_CMD_TX: Mutex<
    Option<std::sync::mpsc::Sender<Vec<(i32, Option<String>, (u32, u32))>>>,
> = Mutex::new(None);

/// 辅助扩展：在 Windows 上创建隐藏命令行子进程
pub trait CommandExtHidden {
    fn creation_flags_hidden(&mut self) -> &mut Self;
}

impl CommandExtHidden for std::process::Command {
    fn creation_flags_hidden(&mut self) -> &mut Self {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // 仅 CREATE_NO_WINDOW：隐藏子进程窗口即可。
            // 切勿在此加 CREATE_BREAKAWAY_FROM_JOB——若 AnyVersion 被放入一个
            // 不允许 breakaway 的 Job Object（如由某些父进程/CI 启动），该标志会让
            // 子进程 spawn 直接失败（os error 5 拒绝访问），导致 resolve_shortcut 等
            // 同步查询（.output()）拿不到结果而静默降级（如 .lnk 被当成文件本身）。
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            self.creation_flags(CREATE_NO_WINDOW);
        }
        self
    }
}

/// 让子进程彻底脱离 AnyVersion 的控制台与生命周期：
/// - `DETACHED_PROCESS`：不继承父进程控制台（子进程有自己的控制台或 GUI 窗口）。
/// - `CREATE_BREAKAWAY_FROM_JOB`：脱离父进程 Job Object。
/// - `CREATE_NEW_PROCESS_GROUP`：独立进程组，Ctrl+C 等信号不波及。
///
/// 注意：本方法不设置 NO_WINDOW，保留目标程序自身的窗口（用于启动用户可见的应用）。
pub trait CommandExtDetached {
    fn creation_flags_detached(&mut self) -> &mut Self;
}

impl CommandExtDetached for std::process::Command {
    fn creation_flags_detached(&mut self) -> &mut Self {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const DETACHED_PROCESS: u32 = 0x00000008;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
            const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x01000000;
            self.creation_flags(
                DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB,
            );
        }
        self
    }
}

mod icons;
pub use icons::*;
mod execution;
pub use execution::*;
mod applications;
pub use applications::*;
mod metadata;
pub use metadata::*;
mod hotkeys;
pub use hotkeys::*;
mod drop;
pub use drop::*;
