use super::*;

#[tauri::command]
pub fn is_admin() -> bool {
    #[cfg(windows)]
    {
        use winreg::enums::*;
        use winreg::RegKey;

        // 尝试以写权限打开 HKEY_LOCAL_MACHINE\SOFTWARE（只有提权的管理员才能成功打开）
        RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey_with_flags("SOFTWARE", KEY_WRITE)
            .is_ok()
    }
    #[cfg(not(windows))]
    {
        true
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PathDirectoryInfo {
    pub path: String,
    pub source: String, // "HKCU" or "HKLM"
    pub exists: bool,
    pub executables: Vec<String>,
}

pub(super) fn expand_env_vars(path_str: &str) -> String {
    let mut expanded = path_str.to_string();
    let mut i = 0;
    while let Some(start) = expanded[i..].find('%') {
        let abs_start = i + start;
        if let Some(end) = expanded[abs_start + 1..].find('%') {
            let abs_end = abs_start + 1 + end;
            let var_name = &expanded[abs_start + 1..abs_end];
            let var_val = std::env::var(var_name).unwrap_or_else(|_| format!("%{}%", var_name));
            expanded.replace_range(abs_start..=abs_end, &var_val);
            i = abs_start + var_val.len();
        } else {
            break;
        }
    }
    expanded
}

pub(super) fn list_executables_in_dir(dir_path: &str) -> Vec<String> {
    let path = Path::new(dir_path);
    if !path.exists() || !path.is_dir() {
        return Vec::new();
    }
    let mut exes = Vec::new();
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.filter_map(|e| e.ok()) {
            if let Ok(file_type) = entry.file_type() {
                if file_type.is_file() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let name_lower = name.to_lowercase();
                    if name_lower.ends_with(".exe")
                        || name_lower.ends_with(".cmd")
                        || name_lower.ends_with(".bat")
                    {
                        exes.push(name);
                    }
                }
            }
            if exes.len() >= 100 {
                break;
            }
        }
    }
    exes.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
    exes
}

#[tauri::command]
pub fn get_path_directories() -> Result<Vec<PathDirectoryInfo>, String> {
    let mut result = Vec::new();

    // 1. Read HKCU PATH
    if let Some(user_path) = get_registry_env("PATH") {
        for part in std::env::split_paths(&user_path) {
            let path_str = part.to_string_lossy().to_string();
            if path_str.is_empty() {
                continue;
            }
            let expanded = expand_env_vars(&path_str);
            let exists = Path::new(&expanded).exists();
            let executables = if exists {
                list_executables_in_dir(&expanded)
            } else {
                Vec::new()
            };
            result.push(PathDirectoryInfo {
                path: path_str,
                source: "HKCU".to_string(),
                exists,
                executables,
            });
        }
    }

    // 2. Read HKLM PATH
    if let Some(sys_path) = get_system_registry_env("PATH") {
        for part in std::env::split_paths(&sys_path) {
            let path_str = part.to_string_lossy().to_string();
            if path_str.is_empty() {
                continue;
            }
            let expanded = expand_env_vars(&path_str);
            let exists = Path::new(&expanded).exists();
            let executables = if exists {
                list_executables_in_dir(&expanded)
            } else {
                Vec::new()
            };
            result.push(PathDirectoryInfo {
                path: path_str,
                source: "HKLM".to_string(),
                exists,
                executables,
            });
        }
    }

    Ok(result)
}

#[tauri::command]
pub fn save_path_directories(
    user_paths: Vec<String>,
    system_paths: Vec<String>,
    save_system: bool,
) -> Result<(), String> {
    // 1. Create a backup first
    let time_str = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let _ = create_env_backup(format!("Reorder PATH - {}", time_str));

    // 2. Set User PATH
    let user_path_val = std::env::join_paths(user_paths.iter().map(Path::new))
        .map_err(|e| e.to_string())?
        .to_string_lossy()
        .to_string();
    set_registry_env("PATH", &user_path_val)?;

    // 3. Set System PATH
    if save_system && !system_paths.is_empty() {
        let sys_path_val = std::env::join_paths(system_paths.iter().map(Path::new))
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .to_string();

        // Try setting system PATH, which may fail due to privilege restrictions
        set_system_registry_env("PATH", &sys_path_val)?;
    }

    // 4. Sync process PATH
    crate::sync_process_path();

    Ok(())
}
