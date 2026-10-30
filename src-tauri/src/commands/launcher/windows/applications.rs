use super::*;

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
            let actual_file =
                if !target.starts_with("shell:") && !target.contains('\\') && !target.contains('/')
                {
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
            if path.is_file()
                && path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.eq_ignore_ascii_case("lnk"))
                    .unwrap_or(false)
            {
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
                            params: if info.arguments.is_empty() {
                                None
                            } else {
                                Some(info.arguments)
                            },
                            icon: info.icon_base64,
                            category: entry
                                .path()
                                .parent()
                                .and_then(|p| p.file_name())
                                .and_then(|n| n.to_str())
                                .map(|s| s.to_string()),
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
                            display_name: if display_name.is_empty() {
                                family_name.clone()
                            } else {
                                display_name
                            },
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
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    results
}
