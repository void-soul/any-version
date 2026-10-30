use super::*;

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
    let target_bm = if !hbm_color.is_null() {
        hbm_color
    } else {
        hbm_mask
    };
    GetObjectW(
        target_bm as _,
        std::mem::size_of::<BITMAP>() as i32,
        &mut bm as *mut _ as _,
    );

    let width = bm.bmWidth;
    let height = if hbm_color.is_null() {
        bm.bmHeight / 2
    } else {
        bm.bmHeight
    };

    if width <= 0 || height <= 0 {
        if !hbm_color.is_null() {
            DeleteObject(hbm_color as _);
        }
        if !hbm_mask.is_null() {
            DeleteObject(hbm_mask as _);
        }
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

    if !hbm_color.is_null() {
        DeleteObject(hbm_color as _);
    }
    if !hbm_mask.is_null() {
        DeleteObject(hbm_mask as _);
    }
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
    let stem = path_obj
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    // 如果本身不是 lnk，直接返回该文件自身信息
    if !lnk_path.to_lowercase().ends_with(".lnk") {
        let is_dir = path_obj.is_dir();
        let working_dir = if is_dir {
            lnk_path.to_string()
        } else {
            path_obj
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default()
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
                target_path: if target_path.is_empty() {
                    lnk_path.to_string()
                } else {
                    target_path
                },
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

/// 从快捷方式的 IconLocation（如 `C:\path\icon.ico,0` 或 `%USERPROFILE%\x.ico`）提取自定义图标。
/// 仅当快捷方式显式指定了自定义图标时才有值；为空或提取失败时返回 None，
/// 由调用方回退到「目标文件图标」。这样特殊快捷方式可保留自定义 .ico，又不影响普通快捷方式。
pub(super) fn extract_shortcut_custom_icon(icon_loc: &str) -> Option<String> {
    let icon_loc = icon_loc.trim();
    if icon_loc.is_empty() {
        return None;
    }

    // 1. 展开环境变量（如 %USERPROFILE%）——纯 std 方式，遍历当前进程环境变量逐项替换
    let mut expanded = icon_loc.to_string();
    for (k, v) in std::env::vars() {
        let pat = format!("%{}%", k);
        if expanded.contains(&pat) {
            expanded = expanded.replace(&pat, &v);
        }
    }

    // 2. 去掉 `,索引` 后缀（IconLocation 形如 `path,index`；从右往左找最后一个逗号）
    let path_part = match expanded.rfind(',') {
        Some(idx) => &expanded[..idx],
        None => expanded.as_str(),
    };
    let path_part = path_part.trim();
    if path_part.is_empty() {
        return None;
    }

    // 3. 仅当指向的图标文件确实存在时才提取；否则回退目标图标
    let p = Path::new(path_part);
    if !p.exists() {
        return None;
    }

    extract_file_icon(path_part)
}
