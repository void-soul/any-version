use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::sync::LazyLock;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
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

/// 提取文件/程序/快捷方式/目录的高清图标并转为 Base64 PNG 数据 URL
pub fn extract_file_icon(path: &str) -> Option<String> {
    let wpath = to_wide_chars(path);
    unsafe {
        let mut shfi: SHFILEINFOW = std::mem::zeroed();
        let flags = SHGFI_ICON | SHGFI_LARGEICON;
        let res = SHGetFileInfoW(
            wpath.as_ptr(),
            0,
            &mut shfi,
            std::mem::size_of::<SHFILEINFOW>() as u32,
            flags,
        );

        let hicon = if res != 0 && !shfi.hIcon.is_null() {
            shfi.hIcon
        } else {
            // fallback: 尝试默认文件属性
            let mut shfi_fallback: SHFILEINFOW = std::mem::zeroed();
            let res2 = SHGetFileInfoW(
                wpath.as_ptr(),
                FILE_ATTRIBUTE_NORMAL,
                &mut shfi_fallback,
                std::mem::size_of::<SHFILEINFOW>() as u32,
                SHGFI_ICON | SHGFI_LARGEICON | SHGFI_USEFILEATTRIBUTES,
            );
            if res2 != 0 && !shfi_fallback.hIcon.is_null() {
                shfi_fallback.hIcon
            } else {
                return None;
            }
        };

        let result = hicon_to_png_base64(hicon);
        DestroyIcon(hicon);
        result
    }
}

/// 将 HICON 转换为 Base64 PNG 字符串
unsafe fn hicon_to_png_base64(hicon: HICON) -> Option<String> {
    let mut icon_info: ICONINFO = std::mem::zeroed();
    if GetIconInfo(hicon, &mut icon_info) == 0 {
        return None;
    }

    let hbm_color = icon_info.hbmColor;
    let hbm_mask = icon_info.hbmMask;

    let hdc_screen: HDC = GetDC(std::ptr::null_mut());
    let hdc_mem: HDC = CreateCompatibleDC(hdc_screen);

    let mut bm: BITMAP = std::mem::zeroed();
    let target_bm = if !hbm_color.is_null() { hbm_color } else { hbm_mask };
    GetObjectW(
        target_bm as _,
        std::mem::size_of::<BITMAP>() as i32,
        &mut bm as *mut _ as _,
    );

    let width = bm.bmWidth;
    let height = if hbm_color.is_null() { bm.bmHeight / 2 } else { bm.bmHeight };

    if width <= 0 || height <= 0 {
        if !hbm_color.is_null() { DeleteObject(hbm_color as _); }
        if !hbm_mask.is_null() { DeleteObject(hbm_mask as _); }
        DeleteDC(hdc_mem);
        ReleaseDC(std::ptr::null_mut(), hdc_screen);
        return None;
    }

    let mut bi: BITMAPINFO = std::mem::zeroed();
    bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bi.bmiHeader.biWidth = width;
    bi.bmiHeader.biHeight = -height; // top-down
    bi.bmiHeader.biPlanes = 1;
    bi.bmiHeader.biBitCount = 32;
    bi.bmiHeader.biCompression = BI_RGB as u32;

    let buf_size = (width * height * 4) as usize;
    let mut pixel_buf: Vec<u8> = vec![0; buf_size];

    let old_bmp = SelectObject(hdc_mem, target_bm as _);
    GetDIBits(
        hdc_mem,
        target_bm as _,
        0,
        height as u32,
        pixel_buf.as_mut_ptr() as _,
        &mut bi,
        DIB_RGB_COLORS,
    );
    SelectObject(hdc_mem, old_bmp);

    // BGRA -> RGBA 转换，并检测是否有有效 Alpha 通道
    let mut has_alpha = false;
    for chunk in pixel_buf.chunks_exact_mut(4) {
        let b = chunk[0];
        let r = chunk[2];
        let a = chunk[3];
        chunk[0] = r;
        chunk[2] = b;
        if a > 0 {
            has_alpha = true;
        }
    }

    // 如果所有 alpha 都为 0，将透明度置为 255
    if !has_alpha && !hbm_color.is_null() {
        for chunk in pixel_buf.chunks_exact_mut(4) {
            chunk[3] = 255;
        }
    }

    if !hbm_color.is_null() { DeleteObject(hbm_color as _); }
    if !hbm_mask.is_null() { DeleteObject(hbm_mask as _); }
    DeleteDC(hdc_mem);
    ReleaseDC(std::ptr::null_mut(), hdc_screen);

    let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_raw(width as u32, height as u32, pixel_buf)?;

    let mut png_bytes = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png_bytes, image::ImageFormat::Png).ok()?;

    let encoded = general_purpose::STANDARD.encode(png_bytes.into_inner());
    Some(format!("data:image/png;base64,{}", encoded))
}

/// 解析 Windows .lnk 快捷方式
pub fn resolve_shortcut(lnk_path: &str) -> Option<ShortcutInfo> {
    let path_obj = Path::new(lnk_path);
    let stem = path_obj.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();

    // 如果本身不是 lnk，直接返回该文件自身信息
    if !lnk_path.to_lowercase().ends_with(".lnk") {
        let is_dir = path_obj.is_dir();
        let working_dir = if is_dir {
            lnk_path.to_string()
        } else {
            path_obj.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default()
        };
        let icon_base64 = extract_file_icon(lnk_path);
        return Some(ShortcutInfo {
            name: stem,
            target_path: lnk_path.to_string(),
            arguments: String::new(),
            working_dir,
            icon_path: None,
            is_dir,
            icon_base64,
        });
    }

    // 解析 .lnk 文件：通过 PowerShell 快速安全解析 WScript.Shell
    let escaped_path = lnk_path.replace('\'', "''");
    let script = format!(
        r#"
$sh = New-Object -ComObject WScript.Shell
$sc = $sh.CreateShortcut('{}')
[PSCustomObject]@{{
    Target = $sc.TargetPath
    Args = $sc.Arguments
    WorkDir = $sc.WorkingDirectory
    IconLoc = $sc.IconLocation
}} | ConvertTo-Json -Compress
"#,
        escaped_path
    );

    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags_hidden()
        .output()
        .ok()?;

    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) {
            let target_path = val["Target"].as_str().unwrap_or("").to_string();
            let arguments = val["Args"].as_str().unwrap_or("").to_string();
            let mut working_dir = val["WorkDir"].as_str().unwrap_or("").to_string();
            let icon_loc = val["IconLoc"].as_str().map(|s| s.to_string());

            let target_obj = Path::new(&target_path);
            let is_dir = target_obj.is_dir();
            if working_dir.is_empty() {
                if is_dir {
                    working_dir = target_path.clone();
                } else if let Some(parent) = target_obj.parent() {
                    working_dir = parent.to_string_lossy().to_string();
                }
            }

            // 优先提取目标实际文件的图标，如无则提取 lnk 本身
            let icon_base64 = if !target_path.is_empty() && target_obj.exists() {
                extract_file_icon(&target_path).or_else(|| extract_file_icon(lnk_path))
            } else {
                extract_file_icon(lnk_path)
            };

            return Some(ShortcutInfo {
                name: stem,
                target_path: if target_path.is_empty() { lnk_path.to_string() } else { target_path },
                arguments,
                working_dir,
                icon_path: icon_loc,
                is_dir,
                icon_base64,
            });
        }
    }

    // fallback
    let icon_base64 = extract_file_icon(lnk_path);
    Some(ShortcutInfo {
        name: stem,
        target_path: lnk_path.to_string(),
        arguments: String::new(),
        working_dir: String::new(),
        icon_path: None,
        is_dir: false,
        icon_base64,
    })
}

/// 执行文件/命令（支持 open / runas 管理员提权 / explore 资源管理器 / 任意文件关联程序打开）
pub fn shell_execute(
    operation: &str,
    file: &str,
    params: &str,
    start_location: Option<&str>,
) -> Result<(), String> {
    let p = Path::new(file);
    let is_dir = operation == "explore" || p.is_dir();
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let is_exe = ext == "exe" || ext == "bat" || ext == "cmd" || ext == "ps1" || ext == "com";

    // 只有可执行程序才支持 runas 管理员提权，其它文档/文件夹降级为 open
    let actual_op = if operation == "runas" && !is_exe {
        "open"
    } else {
        operation
    };

    let work_dir = if let Some(dir) = start_location {
        if !dir.trim().is_empty() {
            dir.to_string()
        } else {
            get_default_work_dir(file)
        }
    } else {
        get_default_work_dir(file)
    };

    let w_op = to_wide_chars(if is_dir && actual_op == "explore" { "open" } else { actual_op });
    let w_file = to_wide_chars(if is_dir && actual_op == "explore" { "explorer.exe" } else { file });
    let dir_param_str = if is_dir && actual_op == "explore" {
        format!("\"{}\"", file)
    } else {
        params.to_string()
    };
    let w_params = to_wide_chars(&dir_param_str);
    let w_dir = to_wide_chars(&work_dir);

    unsafe {
        // 初始化当前线程 COM 接口
        let _ = windows_sys::Win32::System::Com::CoInitializeEx(
            std::ptr::null_mut(),
            windows_sys::Win32::System::Com::COINIT_APARTMENTTHREADED as u32,
        );

        let res = ShellExecuteW(
            std::ptr::null_mut(),
            w_op.as_ptr(),
            w_file.as_ptr(),
            if dir_param_str.is_empty() { std::ptr::null() } else { w_params.as_ptr() },
            if work_dir.is_empty() { std::ptr::null() } else { w_dir.as_ptr() },
            SW_SHOWNORMAL as i32,
        );

        if (res as isize) <= 32 {
            if is_exe {
                let mut cmd = std::process::Command::new(file);
                if !params.trim().is_empty() {
                    cmd.raw_arg(params);
                }
                if !work_dir.is_empty() {
                    cmd.current_dir(&work_dir);
                }
                cmd.spawn().map_err(|e| format!("启动程序失败 (ShellExecute code {}): {}", res as isize, e))?;
            } else {
                // 文档/文本文件/目录通过 cmd /c start 打开系统默认关联程序
                let mut cmd = std::process::Command::new("cmd.exe");
                let mut cmd_args = vec!["/c".to_string(), "start".to_string(), "".to_string(), file.to_string()];
                if !params.trim().is_empty() {
                    cmd_args.push(params.to_string());
                }
                cmd.args(&cmd_args).creation_flags_hidden();
                if !work_dir.is_empty() {
                    cmd.current_dir(&work_dir);
                }
                cmd.spawn().map_err(|e| format!("打开文件失败 (ShellExecute code {}): {}", res as isize, e))?;
            }
        }
    }
    Ok(())
}

fn get_default_work_dir(file: &str) -> String {
    let p = Path::new(file);
    if p.is_dir() {
        file.to_string()
    } else if let Some(parent) = p.parent() {
        parent.to_string_lossy().to_string()
    } else {
        String::new()
    }
}

/// 执行系统级内置命令
pub fn system_item_execute(target: &str, params: Option<&str>) -> Result<(), String> {
    match target {
        "static:TurnOffMonitor" => {
            unsafe {
                // 关闭显示器
                SendMessageW(
                    HWND_BROADCAST as _,
                    WM_SYSCOMMAND,
                    SC_MONITORPOWER as usize,
                    2 as isize,
                );
            }
            Ok(())
        }
        "static:EmptyRecycleBin" => {
            unsafe {
                SHEmptyRecycleBinW(std::ptr::null_mut(), std::ptr::null(), SHERB_NOSOUND);
            }
            Ok(())
        }
        "static:LockWorkstation" => {
            let _ = std::process::Command::new("rundll32.exe")
                .args(["user32.dll,LockWorkStation"])
                .spawn();
            Ok(())
        }
        "static:RestartExplorer" => {
            thread::spawn(|| {
                let _ = std::process::Command::new("taskkill")
                    .args(["/f", "/im", "explorer.exe"])
                    .creation_flags_hidden()
                    .output();
                thread::sleep(std::time::Duration::from_millis(500));
                let _ = std::process::Command::new("explorer.exe").spawn();
            });
            Ok(())
        }
        _ => {
            let actual_file = if !target.starts_with("shell:") && !target.contains('\\') && !target.contains('/') {
                search_path(target).unwrap_or_else(|| target.to_string())
            } else {
                target.to_string()
            };

            let p_str = params.unwrap_or("");
            shell_execute("open", &actual_file, p_str, None)
        }
    }
}

/// 查找系统路径中的可执行文件全路径
pub fn search_path(file: &str) -> Option<String> {
    let w_file = to_wide_chars(file);
    let mut buf = [0u16; MAX_PATH as usize];
    let mut file_part: *mut u16 = std::ptr::null_mut();
    unsafe {
        let len = SearchPathW(
            std::ptr::null(),
            w_file.as_ptr(),
            std::ptr::null(),
            MAX_PATH,
            buf.as_mut_ptr(),
            &mut file_part,
        );
        if len > 0 {
            Some(from_wide_ptr(buf.as_ptr()))
        } else {
            None
        }
    }
}

/// 扫描 Windows 开始菜单（公共与当前用户）
pub fn scan_start_menu() -> Vec<ScannedProgram> {
    let mut results = Vec::new();
    let mut seen_paths = HashSet::new();

    let mut search_dirs = Vec::new();
    if let Ok(appdata) = std::env::var("APPDATA") {
        search_dirs.push(PathBuf::from(appdata).join(r"Microsoft\Windows\Start Menu\Programs"));
    }
    if let Ok(programdata) = std::env::var("PROGRAMDATA") {
        search_dirs.push(PathBuf::from(programdata).join(r"Microsoft\Windows\Start Menu\Programs"));
    }

    for dir in search_dirs {
        if !dir.exists() {
            continue;
        }
        for entry in walkdir::WalkDir::new(&dir)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("lnk")).unwrap_or(false) {
                let path_str = path.to_string_lossy().to_string();
                if seen_paths.insert(path_str.to_lowercase()) {
                    if let Some(info) = resolve_shortcut(&path_str) {
                        let target_lower = info.target_path.to_lowercase();
                        // 过滤掉卸载程序和纯文档
                        if target_lower.contains("uninstall") || target_lower.contains("卸载") {
                            continue;
                        }
                        results.push(ScannedProgram {
                            name: info.name,
                            target: info.target_path,
                            params: if info.arguments.is_empty() { None } else { Some(info.arguments) },
                            icon: info.icon_base64,
                            category: entry.path().parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()).map(|s| s.to_string()),
                            is_dir: info.is_dir,
                        });
                    }
                }
            }
        }
    }

    // 按名称排序
    results.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    results
}

/// 扫描 Windows 10/11 安装的 Appx / UWP 应用
pub fn scan_appx_list() -> Vec<AppxItem> {
    let script = r#"
Get-AppxPackage | Where-Object { -not $_.IsFramework -and $_.NonRemovable -ne $true } | ForEach-Object {
    $manifest = Get-AppxPackageManifest $_ -ErrorAction SilentlyContinue
    $app = $manifest.Package.Applications.Application | Select-Object -First 1
    $appName = if ($app.VisualElements.DisplayName) { $app.VisualElements.DisplayName } else { $_.Name }
    if ($appName -match '^ms-resource:') {
        $appName = $_.Name
    }
    [PSCustomObject]@{
        DisplayName = $appName
        FamilyName = $_.PackageFamilyName
        AppId = if ($app.Id) { $app.Id } else { "App" }
        InstalledPath = $_.InstallLocation
        Logo = if ($app.VisualElements.Square44x44Logo) { Join-Path $_.InstallLocation $app.VisualElements.Square44x44Logo } else { "" }
    }
} | ConvertTo-Json -Compress
"#;

    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags_hidden()
        .output();

    let mut results = Vec::new();
    if let Ok(out) = output {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(&text) {
                for item in arr {
                    let display_name = item["DisplayName"].as_str().unwrap_or("").to_string();
                    let family_name = item["FamilyName"].as_str().unwrap_or("").to_string();
                    let app_id = item["AppId"].as_str().unwrap_or("App").to_string();
                    let logo = item["Logo"].as_str().unwrap_or("").to_string();
                    let installed_path = item["InstalledPath"].as_str().map(|s| s.to_string());

                    if !family_name.is_empty() {
                        let icon_base64 = if !logo.is_empty() && Path::new(&logo).exists() {
                            extract_file_icon(&logo)
                        } else {
                            None
                        };
                        results.push(AppxItem {
                            display_name: if display_name.is_empty() { family_name.clone() } else { display_name },
                            family_name,
                            app_id,
                            logo: icon_base64,
                            installed_path,
                        });
                    }
                }
            }
        }
    }
    results
}

/// 扫描关联文件夹内容
pub fn scan_associated_folder(
    folder_path: &str,
    hidden_items: Option<&str>,
    show_only: Option<&str>,
) -> Vec<ScannedProgram> {
    let mut results = Vec::new();
    let dir = Path::new(folder_path);
    if !dir.is_dir() {
        return results;
    }

    let hidden_set: HashSet<String> = hidden_items
        .unwrap_or("")
        .split(|c| c == ',' || c == ';')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();

    let show_mode = show_only.unwrap_or("default");

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().to_string();

            // 隐藏点文件或配置的过滤项
            if file_name.starts_with('.') || hidden_set.contains(&file_name.to_lowercase()) {
                continue;
            }

            let is_dir = path.is_dir();
            if show_mode == "file" && is_dir {
                continue;
            }
            if show_mode == "folder" && !is_dir {
                continue;
            }

            let full_path = path.to_string_lossy().to_string();
            let icon = extract_file_icon(&full_path);

            results.push(ScannedProgram {
                name: file_name,
                target: full_path,
                params: None,
                icon,
                category: None,
                is_dir,
            });
        }
    }

    // 文件夹优先，其次按名称排序
    results.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    results
}

/// 抓取网页标题和高清 Favicon
pub async fn fetch_url_metadata(url_str: &str) -> Result<UrlMetadata, String> {
    fetch_url_metadata_with_timeout(url_str, std::time::Duration::from_secs(6)).await
}

/// 抓取网页标题和高清 Favicon（可自定义超时，用于检测时使用更短超时）
pub async fn fetch_url_metadata_with_timeout(
    url_str: &str,
    timeout: std::time::Duration,
) -> Result<UrlMetadata, String> {
    let parsed_url = reqwest::Url::parse(url_str).map_err(|e| format!("无效的网址: {}", e))?;
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .build()
        .map_err(|e| e.to_string())?;

    let res = client.get(parsed_url.clone()).send().await.map_err(|e| format!("请求失败: {}", e))?;
    let body = res.text().await.unwrap_or_default();

    // 解析 Title
    let title = if let Some(start) = body.to_lowercase().find("<title") {
        if let Some(tag_end) = body[start..].find('>') {
            let content_start = start + tag_end + 1;
            if let Some(end) = body[content_start..].to_lowercase().find("</title>") {
                body[content_start..content_start + end].trim().to_string()
            } else {
                parsed_url.host_str().unwrap_or(url_str).to_string()
            }
        } else {
            parsed_url.host_str().unwrap_or(url_str).to_string()
        }
    } else {
        parsed_url.host_str().unwrap_or(url_str).to_string()
    };

    // 尝试获取 Favicon
    let mut icon_url = None;
    // 1. 从 link rel="icon" 或 rel="shortcut icon" 查找
    let lower_body = body.to_lowercase();
    if let Some(link_idx) = lower_body.find("rel=\"icon\"").or_else(|| lower_body.find("rel=\"shortcut icon\"")) {
        let tag_start = body[..link_idx].rfind('<').unwrap_or(0);
        let tag_end = body[link_idx..].find('>').map(|i| link_idx + i).unwrap_or(body.len());
        let tag_str = &body[tag_start..tag_end];
        if let Some(href_idx) = tag_str.to_lowercase().find("href=\"") {
            let val_start = href_idx + 6;
            if let Some(quote_end) = tag_str[val_start..].find('"') {
                let href_val = &tag_str[val_start..val_start + quote_end];
                if let Ok(joined) = parsed_url.join(href_val) {
                    icon_url = Some(joined);
                }
            }
        }
    }

    if icon_url.is_none() {
        if let Ok(default_favicon) = parsed_url.join("/favicon.ico") {
            icon_url = Some(default_favicon);
        }
    }

    let mut icon_base64 = None;
    if let Some(fav_url) = icon_url {
        if let Ok(fav_res) = client.get(fav_url).send().await {
            if fav_res.status().is_success() {
                if let Ok(bytes) = fav_res.bytes().await {
                    if !bytes.is_empty() {
                        let mime = "image/x-icon";
                        let encoded = general_purpose::STANDARD.encode(&bytes);
                        icon_base64 = Some(format!("data:{};base64,{}", mime, encoded));
                    }
                }
            }
        }
    }

    Ok(UrlMetadata {
        title,
        icon: icon_base64,
        url: url_str.to_string(),
    })
}

/// 解析热键组合字符串，如 "Alt+Space", "Ctrl+Shift+L", "Ctrl+Alt+S", "Alt+Q"
pub fn parse_hotkey(hotkey: &str) -> Option<(u32, u32)> {
    let parts: Vec<&str> = hotkey.split('+').map(|s| s.trim()).collect();
    if parts.is_empty() {
        return None;
    }

    let mut modifiers = 0u32;
    let mut key_code: u32 = 0;

    for part in parts {
        match part.to_lowercase().as_str() {
            "alt" => modifiers |= MOD_ALT,
            "ctrl" | "control" => modifiers |= MOD_CONTROL,
            "shift" => modifiers |= MOD_SHIFT,
            "win" | "meta" => modifiers |= MOD_WIN,
            "space" => key_code = VK_SPACE as u32,
            "tab" => key_code = VK_TAB as u32,
            "enter" | "return" => key_code = VK_RETURN as u32,
            "esc" | "escape" => key_code = VK_ESCAPE as u32,
            "backspace" => key_code = 0x08,
            "delete" | "del" => key_code = 0x2E,
            "insert" => key_code = 0x2D,
            "home" => key_code = 0x24,
            "end" => key_code = 0x23,
            "pageup" => key_code = 0x21,
            "pagedown" => key_code = 0x22,
            "up" => key_code = 0x26,
            "down" => key_code = 0x28,
            "left" => key_code = 0x25,
            "right" => key_code = 0x27,
            "f1" => key_code = VK_F1 as u32,
            "f2" => key_code = VK_F2 as u32,
            "f3" => key_code = VK_F3 as u32,
            "f4" => key_code = VK_F4 as u32,
            "f5" => key_code = VK_F5 as u32,
            "f6" => key_code = VK_F6 as u32,
            "f7" => key_code = VK_F7 as u32,
            "f8" => key_code = VK_F8 as u32,
            "f9" => key_code = VK_F9 as u32,
            "f10" => key_code = VK_F10 as u32,
            "f11" => key_code = VK_F11 as u32,
            "f12" => key_code = VK_F12 as u32,
            k if k.len() == 1 => {
                let c = k.chars().next()?.to_ascii_uppercase();
                if c.is_ascii_alphanumeric() {
                    key_code = c as u32;
                } else if c == '`' || c == '~' {
                    key_code = 0xC0; // VK_OEM_3
                }
            }
            _ => {}
        }
    }

    if key_code != 0 {
        Some((modifiers, key_code))
    } else {
        None
    }
}

static CURRENT_HOTKEY_THREAD_RUNNING: AtomicBool = AtomicBool::new(false);
static HOTKEY_THREAD_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// 已注册热键的 id 列表，供下次注册前同步卸载（避免旧线程退出竞态导致新热键被误注销）。
static REGISTERED_HOTKEY_IDS: Mutex<Vec<i32>> = Mutex::new(Vec::new());
/// 前端当前激活的顶级模块 id（由前端在切换页面时通过 "launcher-active-page" 事件上报）。
/// 用于模块专属热键的「显示/隐藏」来回切换判定：当窗口已显示且正显示该模块时，按热键则隐藏。
static CURRENT_PAGE: LazyLock<Mutex<String>> =
    LazyLock::new(|| Mutex::new(String::from("launcher")));

/// 供前端上报当前激活模块（在 setup 中注册 listen 调用）。
pub(crate) fn set_current_page(page: &str) {
    let mut g = CURRENT_PAGE.lock().unwrap();
    *g = page.to_string();
}

/// 注册全局快捷键并启动消息循环监听线程。
///
/// - `show_hide_str`：主热键（全局切换窗口显示，唤起时打开「启动」模块）。
/// - `module_hotkeys`：各顶级模块独立热键（module_id -> 热键字符串），按下后唤起窗口并打开对应模块。
///
/// 主热键按一次显示、再按一次隐藏；模块热键唤起并切换到该模块（已可见时直接切换，不隐藏）。
pub fn register_global_hotkeys(
    app: AppHandle,
    show_hide_str: &str,
    module_hotkeys: &HashMap<String, String>,
) -> Result<(), String> {
    // 构造 (id, 目标模块id可选, (mod, key)) 列表。
    // 主热键固定 id 0x9001（目标 None -> 启动模块）；模块热键从 0x9100 起分配。
    let mut entries: Vec<(i32, Option<String>, (u32, u32))> = Vec::new();
    let main_hotkey = parse_hotkey(show_hide_str);
    if main_hotkey.is_some() {
        entries.push((0x9001, None, main_hotkey.unwrap()));
    }
    let mut mid = 0x9100;
    for (mod_id, hk) in module_hotkeys.iter() {
        if let Some(mk) = parse_hotkey(hk) {
            if !mod_id.is_empty() {
                entries.push((mid, Some(mod_id.clone()), mk));
                mid += 1;
            }
        }
    }
    // 去重：相同 (mod,key) 只注册一次（保留先出现者）。
    let mut seen = std::collections::HashSet::new();
    entries.retain(|(_, _, mk)| seen.insert(*mk));

    if entries.is_empty() {
        // 没有可用热键：卸载旧热键并结束。
        unregister_previous_hotkeys();
        return Ok(());
    }

    // 同步卸载上一次注册的热键（在当前线程执行，避免旧监听线程退出竞态误注销新热键）。
    unregister_previous_hotkeys();

    let app_clone = app.clone();
    let new_ids: Vec<i32> = entries.iter().map(|(id, _, _)| *id).collect();
    let actions: HashMap<i32, Option<String>> =
        entries.iter().map(|(id, mod_id, _)| (*id, mod_id.clone())).collect();

    // 记录本次 id，供下次卸载。
    *REGISTERED_HOTKEY_IDS.lock().unwrap() = new_ids.clone();

    thread::spawn(move || {
        CURRENT_HOTKEY_THREAD_RUNNING.store(true, Ordering::SeqCst);
        let tid = unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() };
        HOTKEY_THREAD_ID.store(tid, Ordering::SeqCst);

        unsafe {
            let mut registered_any = false;
            for (id, _, (modi, key)) in &entries {
                let mut res = RegisterHotKey(std::ptr::null_mut(), *id, *modi | MOD_NOREPEAT, *key);
                if res == 0 {
                    res = RegisterHotKey(std::ptr::null_mut(), *id, *modi, *key);
                }
                if res != 0 {
                    registered_any = true;
                } else {
                    tracing::warn!("注册全局热键 id={:#x} 失败 (可能被占用)", *id);
                }
            }

            if !registered_any {
                CURRENT_HOTKEY_THREAD_RUNNING.store(false, Ordering::SeqCst);
                return;
            }

            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                if msg.message == WM_HOTKEY {
                    let wparam = msg.wParam as i32;
                    if let Some(target) = actions.get(&wparam) {
                        if let Some(window) = app_clone.get_webview_window("main") {
                            let is_visible = window.is_visible().unwrap_or(false);
                            let is_minimized = window.is_minimized().unwrap_or(false);
                            // 本次是否将「唤起窗口」而非「隐藏窗口」。若是唤起，
                            // 提前记录当前前台窗口，供剪贴板模块「一键粘贴」使用。
                            let will_show = match target {
                                None => !(is_visible && !is_minimized),
                                Some(module) => {
                                    !(is_visible && !is_minimized
                                        && CURRENT_PAGE.lock().unwrap().as_str() == module)
                                }
                            };
                            if will_show {
                                crate::commands::clipboard::monitor_remember_window();
                            }
                            match target {
                                // 主热键：可见则隐藏，否则唤起并显示「启动」模块。
                                None => {
                                    if is_visible && !is_minimized {
                                        let _ = window.hide();
                                    } else {
                                        crate::tray::focus_main_window(&window);
                                    }
                                }
                                // 模块热键：与主热键一致，显示/隐藏来回切换。
                                // 已显示且正显示该模块 -> 隐藏；否则显示并切到该模块。
                                Some(module) => {
                                    if is_visible && !is_minimized {
                                        if CURRENT_PAGE.lock().unwrap().as_str() == module {
                                            let _ = window.hide();
                                        } else {
                                            let _ = window.emit("launcher-open-module", module.clone());
                                        }
                                    } else {
                                        crate::tray::show_and_open_module(&window, module);
                                    }
                                }
                            }
                        }
                    }
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }

            for id in &new_ids {
                UnregisterHotKey(std::ptr::null_mut(), *id);
            }
        }
        CURRENT_HOTKEY_THREAD_RUNNING.store(false, Ordering::SeqCst);
    });

    Ok(())
}

/// 同步卸载上一轮注册的全部热键（在当前线程调用，进程级生效）。
fn unregister_previous_hotkeys() {
    let ids = std::mem::take(&mut *REGISTERED_HOTKEY_IDS.lock().unwrap());
    for id in ids {
        unsafe {
            let _ = UnregisterHotKey(std::ptr::null_mut(), id);
        }
    }
}

pub fn register_global_hotkey(app: AppHandle, hotkey_str: &str) -> Result<(), String> {
    let empty: HashMap<String, String> = HashMap::new();
    register_global_hotkeys(app, hotkey_str, &empty)
}

/// 辅助扩展：在 Windows 上创建隐藏命令行子进程
pub trait CommandExtHidden {
    fn creation_flags_hidden(&mut self) -> &mut Self;
}

impl CommandExtHidden for std::process::Command {
    fn creation_flags_hidden(&mut self) -> &mut Self {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            self.creation_flags(CREATE_NO_WINDOW);
        }
        self
    }
}

/// 导入 Edge / Chrome 浏览器收藏夹
pub fn import_browser_bookmarks(browser: &str, custom_path: Option<&str>) -> Result<super::models::BrowserImportResult, String> {
    use super::db;
    use super::models::{BrowserImportResult, Classification, ClassificationData};

    let path = if let Some(p) = custom_path {
        PathBuf::from(p)
    } else {
        let local_app_data = std::env::var("LOCALAPPDATA").map_err(|_| "无法读取 LOCALAPPDATA 环境变量".to_string())?;
        match browser.to_lowercase().as_str() {
            "edge" => PathBuf::from(local_app_data)
                .join("Microsoft")
                .join("Edge")
                .join("User Data")
                .join("Default")
                .join("Bookmarks"),
            "chrome" => PathBuf::from(local_app_data)
                .join("Google")
                .join("Chrome")
                .join("User Data")
                .join("Default")
                .join("Bookmarks"),
            _ => return Err("不支持的浏览器类型，仅支持 edge 或 chrome".to_string()),
        }
    };

    if !path.exists() {
        return Err(format!("未找到浏览器收藏夹文件: {}", path.to_string_lossy()));
    }

    let content = std::fs::read_to_string(&path).map_err(|e| format!("读取收藏夹文件失败: {}", e))?;
    let json: serde_json::Value = serde_json::from_str(&content).map_err(|e| format!("解析收藏夹 JSON 失败: {}", e))?;

    let roots = json.get("roots").ok_or_else(|| "无效的收藏夹格式: 缺少 roots 根节点".to_string())?;

    let root_cls_name = match browser.to_lowercase().as_str() {
        "edge" => "Edge 收藏夹",
        "chrome" => "Chrome 收藏夹",
        _ => "浏览器收藏夹",
    };

    let parent_cls = Classification {
        id: 0,
        parent_id: None,
        name: root_cls_name.to_string(),
        classification_type: 0,
        data: ClassificationData {
            icon: Some("🌐".to_string()),
            ..Default::default()
        },
        shortcut_key: None,
        global_shortcut_key: false,
        order: 99,
        child_list: None,
        item_count: None,
    };

    let parent_id = db::save_classification(&parent_cls)?;
    let mut imported_count = 0usize;

    if let Some(obj) = roots.as_object() {
        for (root_key, root_val) in obj {
            let section_name = match root_key.as_str() {
                "bookmark_bar" => "书签栏",
                "other" => "其他书签",
                "synced" => "移动设备书签",
                other => other,
            };
            if let Some(children) = root_val.get("children").and_then(|c| c.as_array()) {
                if !children.is_empty() {
                    let sub_cls = Classification {
                        id: 0,
                        parent_id: Some(parent_id),
                        name: section_name.to_string(),
                        classification_type: 0,
                        data: ClassificationData {
                            icon: Some("📑".to_string()),
                            ..Default::default()
                        },
                        shortcut_key: None,
                        global_shortcut_key: false,
                        order: 0,
                        child_list: None,
                        item_count: None,
                    };
                    let sub_cls_id = db::save_classification(&sub_cls)?;
                    import_bookmark_children(children, sub_cls_id, &mut imported_count)?;
                }
            }
        }
    }

    Ok(BrowserImportResult {
        count: imported_count,
        category_id: parent_id,
    })
}

fn import_bookmark_children(
    children: &[serde_json::Value],
    current_cls_id: i64,
    imported_count: &mut usize,
) -> Result<(), String> {
    use super::db;
    use super::models::{Classification, ClassificationData, Item, ItemData};

    for node in children {
        let node_type = node.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let name = node.get("name").and_then(|n| n.as_str()).unwrap_or("未命名").trim();

        if node_type == "folder" {
            // 文件夹挂到当前层级（书签栏/父文件夹）之下，保持层级结构
            let folder_cls = Classification {
                id: 0,
                parent_id: Some(current_cls_id),
                name: if name.is_empty() { "文件夹" } else { name }.to_string(),
                classification_type: 0,
                data: ClassificationData {
                    icon: Some("📁".to_string()),
                    ..Default::default()
                },
                shortcut_key: None,
                global_shortcut_key: false,
                order: 0,
                child_list: None,
                item_count: None,
            };
            let new_sub_id = db::save_classification(&folder_cls)?;
            if let Some(sub_children) = node.get("children").and_then(|c| c.as_array()) {
                import_bookmark_children(sub_children, new_sub_id, imported_count)?;
            }
        } else if node_type == "url" {
            let url = node.get("url").and_then(|u| u.as_str()).unwrap_or("");
            if !url.is_empty() {
                let item = Item {
                    id: 0,
                    classification_id: current_cls_id,
                    name: if name.is_empty() { url } else { name }.to_string(),
                    item_type: 2,
                    data: ItemData {
                        target: Some(url.to_string()),
                        run_as_admin: false,
                        ..Default::default()
                    },
                    shortcut_key: None,
                    global_shortcut_key: false,
                    order: *imported_count as i32,
                };
                db::save_item(&item)?;
                *imported_count += 1;
            }
        }
    }
    Ok(())
}

/// 处理拖入的文件或目录列表，解析并生成 Item 列表
pub fn process_dropped_paths(paths: Vec<String>, classification_id: i64) -> Result<Vec<Item>, String> {
    use super::db;
    use super::models::{Item, ItemData};

    let mut created_items = Vec::new();

    for path_str in paths {
        let p = Path::new(&path_str);
        if !p.exists() {
            continue;
        }

        let is_dir = p.is_dir();
        let file_name = p.file_name().and_then(|n| n.to_str()).unwrap_or(&path_str).to_string();
        let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or(&file_name).to_string();
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();

        let item = if ext == "lnk" {
            // 快捷方式
            if let Some(info) = resolve_shortcut(&path_str) {
                Item {
                    id: 0,
                    classification_id,
                    name: if !info.name.is_empty() { info.name } else { stem },
                    item_type: if info.is_dir { 1 } else { 0 },
                    data: ItemData {
                        target: Some(info.target_path),
                        params: if !info.arguments.is_empty() { Some(info.arguments) } else { None },
                        start_location: if !info.working_dir.is_empty() { Some(info.working_dir) } else { None },
                        run_as_admin: false,
                        icon: info.icon_base64,
                        ..Default::default()
                    },
                    shortcut_key: None,
                    global_shortcut_key: false,
                    order: 0,
                }
            } else {
                let icon = extract_file_icon(&path_str);
                Item {
                    id: 0,
                    classification_id,
                    name: stem,
                    item_type: 0,
                    data: ItemData {
                        target: Some(path_str.clone()),
                        run_as_admin: false,
                        icon,
                        ..Default::default()
                    },
                    shortcut_key: None,
                    global_shortcut_key: false,
                    order: 0,
                }
            }
        } else if ext == "url" {
            // 网页快捷方式 .url 文件
            let content = std::fs::read_to_string(p).unwrap_or_default();
            let mut url_target = String::new();
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.to_lowercase().starts_with("url=") {
                    url_target = trimmed[4..].trim().to_string();
                    break;
                }
            }
            if url_target.is_empty() {
                url_target = path_str.clone();
            }
            Item {
                id: 0,
                classification_id,
                name: stem,
                item_type: 2,
                data: ItemData {
                    target: Some(url_target),
                    run_as_admin: false,
                    ..Default::default()
                },
                shortcut_key: None,
                global_shortcut_key: false,
                order: 0,
            }
        } else if is_dir {
            // 目录 / 文件夹
            let icon = extract_file_icon(&path_str);
            Item {
                id: 0,
                classification_id,
                name: file_name,
                item_type: 1, // 文件夹
                data: ItemData {
                    target: Some(path_str.clone()),
                    start_location: Some(path_str.clone()),
                    run_as_admin: false,
                    icon,
                    ..Default::default()
                },
                shortcut_key: None,
                global_shortcut_key: false,
                order: 0,
            }
        } else {
            // 任意文件 / .exe / .bat / .cmd / .ps1 / .pdf / .docx / .png 等
            let icon = extract_file_icon(&path_str);
            let display_name = if ext == "exe" || ext == "bat" || ext == "cmd" || ext == "ps1" {
                stem
            } else {
                file_name
            };
            let parent_dir = p.parent().map(|parent| parent.to_string_lossy().to_string());
            Item {
                id: 0,
                classification_id,
                name: display_name,
                item_type: 0,
                data: ItemData {
                    target: Some(path_str.clone()),
                    start_location: parent_dir,
                    run_as_admin: false,
                    icon,
                    ..Default::default()
                },
                shortcut_key: None,
                global_shortcut_key: false,
                order: 0,
            }
        };

        let item_id = db::save_item(&item)?;
        let mut saved_item = item;
        saved_item.id = item_id;
        created_items.push(saved_item);
    }

    Ok(created_items)
}
