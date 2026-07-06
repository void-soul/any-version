use std::fs;
use std::path::Path;
use serde::{Serialize, Deserialize};
use winreg::enums::*;
use winreg::RegKey;
use crate::commands::config::load_config;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct EnvBackup {
    pub id: String,
    pub timestamp: String,
    pub description: String,
    pub user_vars: std::collections::HashMap<String, String>,
    pub sys_vars: std::collections::HashMap<String, String>,
}



#[cfg(windows)]
pub fn broadcast_setting_change() {
    use std::os::windows::ffi::OsStrExt;
    use std::ffi::OsStr;

    type SendMessageTimeoutW = unsafe extern "system" fn(
        hwnd: *mut std::ffi::c_void,
        msg: u32,
        wparam: usize,
        lparam: *const u16,
        flags: u32,
        timeout: u32,
        result: *mut usize,
    ) -> isize;

    unsafe {
        let module_name: Vec<u16> = OsStr::new("user32.dll").encode_wide().chain(std::iter::once(0)).collect();
        let handle = LoadLibraryW(module_name.as_ptr());
        if !handle.is_null() {
            let proc_name = std::ffi::CString::new("SendMessageTimeoutW").unwrap();
            let proc_addr = GetProcAddress(handle, proc_name.as_ptr() as *const u8);
            if !proc_addr.is_null() {
                let send_msg_timeout: SendMessageTimeoutW = std::mem::transmute(proc_addr);
                let env_str: Vec<u16> = OsStr::new("Environment").encode_wide().chain(std::iter::once(0)).collect();
                let mut result = 0;
                send_msg_timeout(
                    0xffff as *mut std::ffi::c_void, // HWND_BROADCAST
                    0x001a, // WM_SETTINGCHANGE
                    0,
                    env_str.as_ptr(),
                    0x0002, // SMTO_ABORTIFHUNG
                    5000,
                    &mut result,
                );
            }
            FreeLibrary(handle);
        }
    }
}

#[cfg(windows)]
extern "system" {
    fn LoadLibraryW(lpLibFileName: *const u16) -> *mut std::ffi::c_void;
    fn GetProcAddress(hModule: *mut std::ffi::c_void, lpProcName: *const u8) -> *mut std::ffi::c_void;
    fn FreeLibrary(hLibModule: *mut std::ffi::c_void) -> i32;
}

#[cfg(not(windows))]
pub fn broadcast_setting_change() {}

pub fn get_registry_env(name: &str) -> Option<String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(env_key) = hkcu.open_subkey("Environment") {
        if let Ok(val) = env_key.get_value::<String, _>(name) {
            return Some(val);
        }
    }
    None
}

/// 读取系统级(HKLM)环境变量
pub fn get_system_registry_env(name: &str) -> Option<String> {
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    if let Ok(env_key) = hklm.open_subkey("SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment") {
        if let Ok(val) = env_key.get_value::<String, _>(name) {
            return Some(val);
        }
    }
    None
}

/// 同时检查用户级和系统级注册表，返回值及其来源
pub fn get_registry_env_any(name: &str) -> Option<(String, &'static str)> {
    // 用户级优先
    if let Some(val) = get_registry_env(name) {
        if !val.is_empty() {
            return Some((val, "HKCU"));
        }
    }
    // 系统级
    if let Some(val) = get_system_registry_env(name) {
        if !val.is_empty() {
            return Some((val, "HKLM"));
        }
    }
    None
}

/// 智能写入注册表环境变量：
/// - 值包含 % 时使用 REG_EXPAND_SZ（支持 %SystemRoot% 等展开）
/// - 普通字符串使用 REG_SZ
#[cfg(windows)]
fn set_registry_value_smart(key: &RegKey, name: &str, value: &str) -> Result<(), String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    let is_expandable = value.contains('%');
    let reg_type = if is_expandable { REG_EXPAND_SZ } else { REG_SZ };

    // 将字符串编码为 UTF-16LE + null terminator，再转为 &[u8]
    let wide: Vec<u16> = OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))  // null terminator
        .collect();
    let bytes = unsafe {
        std::slice::from_raw_parts(
            wide.as_ptr() as *const u8,
            wide.len() * std::mem::size_of::<u16>(),
        )
    };
    let reg_value = winreg::RegValue {
        vtype: reg_type,
        bytes: bytes.to_vec(),
    };
    key.set_raw_value(name, &reg_value).map_err(|e| e.to_string())
}

pub fn set_registry_env(name: &str, value: &str) -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (env_key, _) = hkcu.create_subkey("Environment").map_err(|e| e.to_string())?;
    if value.is_empty() {
        let _ = env_key.delete_value(name);
    } else {
        set_registry_value_smart(&env_key, name, value)?;
    }
    broadcast_setting_change();
    Ok(())
}

pub fn set_system_registry_env(name: &str, value: &str) -> Result<(), String> {
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let (env_key, _) = hklm.create_subkey("SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment").map_err(|e| e.to_string())?;
    if value.is_empty() {
        let _ = env_key.delete_value(name);
    } else {
        set_registry_value_smart(&env_key, name, value)?;
    }
    broadcast_setting_change();
    Ok(())
}


pub fn add_to_user_path(paths: &[String]) -> Result<(), String> {
    let known_tools = super::project::registry::all_ids();

    if let Some(user_path) = get_registry_env("PATH") {
        let mut parts = std::env::split_paths(&user_path)
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        
        let mut modified = false;
        // 倒序遍历插入，以保持传入的 paths 之间的相对顺序在最前列
        for path in paths.iter().rev() {
            let path_lower = path.to_lowercase();

            // 防御检查：是否包含重复的工具名（如 ...nodejs\nodejs）
            for tool in &known_tools {
                let double = format!("{}{}", tool, tool);
                let double_sep = format!("{}\\{}", tool, tool);
                if path_lower.ends_with(&double) || path_lower.ends_with(&double_sep) {
                    return Err(format!("PATH 条目疑似损坏（重复的工具名）: {}", path));
                }
            }

            // 无论当前 PATH 中是否已存在，我们都先将其移除并插入到最前面，以保证最高优先级
            parts.retain(|p| p.to_lowercase() != path_lower);
            parts.insert(0, path.clone());
            modified = true;
        }
        
        if modified {
            let new_path = std::env::join_paths(parts.iter().map(Path::new))
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .to_string();
            set_registry_env("PATH", &new_path)?;
        }
    } else {
        let new_path = std::env::join_paths(paths.iter().map(Path::new))
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .to_string();
        set_registry_env("PATH", &new_path)?;
    }
    Ok(())
}

pub fn remove_from_user_path(paths: &[String]) -> Result<(), String> {
    if let Some(user_path) = get_registry_env("PATH") {
        let parts = std::env::split_paths(&user_path)
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        
        let initial_len = parts.len();
        let new_parts = parts.into_iter()
            .filter(|p| {
                let p_lower = p.to_lowercase();
                !paths.iter().any(|target| target.to_lowercase() == p_lower)
            })
            .collect::<Vec<_>>();
        
        if new_parts.len() != initial_len {
            let new_path = std::env::join_paths(new_parts.iter().map(Path::new))
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .to_string();
            set_registry_env("PATH", &new_path)?;
        }
    }
    Ok(())
}


/// 自动配置 SDK 相关环境变量（注册表驱动）。
/// 新增 SDK 时只需在 projects/<id>/config.json 中定义 env_vars，此函数自动生效。
///
/// 设计原则：
///   - 所有 *_HOME 类变量指向 link_dir（版本切换只需重定向 junction）
///   - CARGO_HOME / RUSTUP_HOME 指向 link_dir 下的子目录
///   - ANDROID_SDK_HOME 指向 link_dir 下的 .android 子目录
pub fn configure_sdk_env_vars(sdk_id: &str, link_dir: &str, _version_dir: &str) -> Result<(), String> {
    use super::project::registry;

    let config = load_config();
    if !config.managed_items.contains(sdk_id) {
        return Ok(());
    }

    let sdk_def = match registry::find_by_id(sdk_id) {
        Some(d) => d,
        None => return Ok(()),
    };

    for var_info in &sdk_def.env_vars {
        // Skip compat/discovery tier vars - only scan, not manage
        if let Some(ref tier) = var_info.tier {
            if *tier == super::project::types::EnvVarTier::Compat {
                continue;
            }
            if *tier == super::project::types::EnvVarTier::Clear {
                let _ = set_registry_env(&var_info.name, "");
                continue;
            }
        }

        let var_name = &var_info.name;
        // 值策略由 EnvVarDef.sub_dir 驱动：有 sub_dir 则拼接触到 link_dir 后，否则直接用 link_dir
        let value = if let Some(ref sub) = var_info.sub_dir {
            format!("{}\\{}", link_dir, sub)
        } else {
            link_dir.to_string()
        };
        let _ = set_registry_env(var_name, &value);
    }

    // 自动将可执行目录添加到用户 PATH 变量中
    let bin_paths = crate::commands::project::scanner::get_bin_paths(sdk_id, link_dir);
    let _ = add_to_user_path(&bin_paths);

    Ok(())
}

/// 移除 SDK 相关的环境变量（注册表驱动）。
/// 当卸载某 SDK 最后一个版本时调用。
pub fn remove_sdk_env_vars(sdk_id: &str) -> Result<(), String> {
    use super::project::registry;

    let sdk_def = match registry::find_by_id(sdk_id) {
        Some(d) => d,
        None => return Ok(()),
    };

    for var_info in &sdk_def.env_vars {
        // Only remove vars we would have set (core + package), skip compat
        if let Some(ref tier) = var_info.tier {
            if *tier == super::project::types::EnvVarTier::Compat {
                continue;
            }
        }
        let _ = set_registry_env(&var_info.name, "");
    }

    // 从用户 PATH 中移除该 SDK 的可执行目录
    let config = load_config();
    let junction_path = Path::new(&config.links_dir).join(sdk_id);
    let link_str = junction_path.to_string_lossy().to_string();
    let bin_paths = crate::commands::project::scanner::get_bin_paths(sdk_id, &link_str);
    let _ = remove_from_user_path(&bin_paths);

    Ok(())
}

/// 获取指定项目可配置的运行时环境变量（user_configurable_vars）的当前值
#[tauri::command]
pub fn get_user_configurable_vars(project_id: String) -> Result<Vec<serde_json::Value>, String> {
    use super::project::registry;
    let def = registry::find_by_id(&project_id)
        .ok_or_else(|| format!("未找到项目: {}", project_id))?;

    let mut results = Vec::new();
    for var in &def.user_configurable_vars {
        let (current_value, source) = get_registry_env_any(&var.name)
            .map(|(v, s)| (Some(v), s.to_string()))
            .unwrap_or((None, "未设置".to_string()));

        results.push(serde_json::json!({
            "name": var.name,
            "desc": var.desc,
            "placeholder": var.placeholder,
            "options": var.options,
            "var_type": var.var_type,
            "current_value": current_value,
            "source": source,
        }));
    }
    Ok(results)
}

/// 设置用户自定义环境变量（运行时参数）
#[tauri::command]
pub fn set_user_configurable_var(name: String, value: String) -> Result<(), String> {
    set_registry_env(&name, &value)
}

/// 删除用户自定义环境变量
#[tauri::command]
pub fn delete_user_configurable_var(name: String) -> Result<(), String> {
    set_registry_env(&name, "")
}

#[tauri::command]
pub fn toggle_item_management(id: String, enable: bool, cache_dest: Option<String>) -> Result<(), String> {
    use super::project::registry;
    let sdk_def = registry::find_by_id(&id)
        .ok_or_else(|| format!("未找到该标识符对应的配置: {}", id))?;

    let mut config = load_config();

    if enable {
        // 1. Add to managed_items
        config.managed_items.insert(id.clone());

        // 2. Backup conflicting environment variables
        for var_info in &sdk_def.env_vars {
            let var_name = &var_info.name;
            if let Some((val, _source)) = get_registry_env_any(var_name) {
                if !val.to_lowercase().contains(&config.links_dir.to_lowercase()) {
                    config.original_envs.entry(var_name.to_string()).or_insert(val);
                }
            }
        }

        // 3. Backup conflicting PATH entries from HKCU
        let mut original_paths_to_save = Vec::new();
        if let Some(user_path) = get_registry_env("PATH") {
            let parts = std::env::split_paths(&user_path)
                .map(|p| p.to_string_lossy().to_string())
                .collect::<Vec<_>>();
            
            let mut matched_entries = Vec::new();
            let mut remaining_entries = Vec::new();

            for p_str in parts {
                if p_str.is_empty() {
                    continue;
                }
                
                let mut matches = false;
                let p_lower = p_str.to_lowercase();
                
                if !p_lower.contains(&config.links_dir.to_lowercase()) {
                    for rule in &sdk_def.find_rules {
                        match &rule.pattern {
                            super::project::types::ResolvePattern::PathContains { path_key: pattern, .. } => {
                                if p_lower.contains(&pattern.to_lowercase()) {
                                    matches = true;
                                    break;
                                }
                            }
                            super::project::types::ResolvePattern::FixedPath { path: fixed_path, .. } => {
                                if p_lower.contains(&fixed_path.to_lowercase()) {
                                    matches = true;
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                }

                if matches {
                    matched_entries.push(p_str.clone());
                    original_paths_to_save.push(p_str);
                } else {
                    remaining_entries.push(p_str);
                }
            }

            if !matched_entries.is_empty() {
                config.original_paths.entry(id.clone()).or_insert_with(Vec::new).extend(matched_entries);
                
                let new_path = std::env::join_paths(remaining_entries.iter().map(Path::new))
                    .map_err(|e| e.to_string())?
                    .to_string_lossy()
                    .to_string();
                set_registry_env("PATH", &new_path)?;
            }
        }

        // Save config changes so far
        crate::commands::config::save_config(&config)?;

        // 4. Configure SDK env vars if a version is active
        let junction_path = Path::new(&config.links_dir).join(&id);
        if junction_path.exists() {
            let link_str = junction_path.to_string_lossy().to_string();
            let dest_str = fs::canonicalize(&junction_path)
                .map(|p| p.to_string_lossy().to_string().trim_start_matches(r"\\?\").to_string())
                .unwrap_or_default();
            let _ = configure_sdk_env_vars(&id, &link_str, &dest_str);
        }

        // 5. Migrate cache if requested
        if let Some(dest) = cache_dest {
            if !dest.is_empty() {
                let _ = crate::commands::cache::migrate_cache_path(id.clone(), dest);
            }
        }
    } else {
        // Disable management
        config.managed_items.remove(&id);

        // 1. Remove AnyVersion environment variables for this SDK
        let _ = remove_sdk_env_vars(&id);

        // 2. Restore original environment variables from backup
        for var_info in &sdk_def.env_vars {
            let var_name = &var_info.name;
            if let Some(orig_val) = config.original_envs.remove(var_name) {
                let _ = set_registry_env(var_name, &orig_val);
            }
        }

        // 3. Restore original PATH entries
        if let Some(orig_paths) = config.original_paths.remove(&id) {
            let _ = add_to_user_path(&orig_paths);
        }

        // Save updated config
        crate::commands::config::save_config(&config)?;
    }

    Ok(())
}

#[tauri::command]
pub fn create_env_backup(description: String) -> Result<EnvBackup, String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let mut user_vars = std::collections::HashMap::new();
    if let Ok(env_key) = hkcu.open_subkey("Environment") {
        for name in env_key.enum_values().filter_map(|x| x.ok()).map(|(n, _)| n) {
            if let Ok(val) = env_key.get_value::<String, _>(&name) {
                user_vars.insert(name, val);
            }
        }
    }

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let mut sys_vars = std::collections::HashMap::new();
    if let Ok(env_key) = hklm.open_subkey("SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment") {
        for name in env_key.enum_values().filter_map(|x| x.ok()).map(|(n, _)| n) {
            if let Ok(val) = env_key.get_value::<String, _>(&name) {
                sys_vars.insert(name, val);
            }
        }
    }

    let id = format!("{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0));

    // Get simple local timestamp using powershell date
    let timestamp = super::hidden_cmd::hidden_cmd("powershell")
        .args(&["-Command", "Get-Date -Format 'yyyy-MM-dd HH:mm:ss'"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "Unknown Time".to_string());

    let backup = EnvBackup {
        id,
        timestamp,
        description,
        user_vars,
        sys_vars,
    };

    let base_dir = crate::commands::config::get_base_dir();
    let backup_dir = base_dir.join("backup");
    fs::create_dir_all(&backup_dir).map_err(|e| e.to_string())?;
    let backup_file = backup_dir.join(format!("env_backup_{}.json", backup.id));
    let data = serde_json::to_string_pretty(&backup).map_err(|e| e.to_string())?;
    fs::write(backup_file, data).map_err(|e| e.to_string())?;

    Ok(backup)
}

#[tauri::command]
pub fn list_env_backups() -> Result<Vec<EnvBackup>, String> {
    let base_dir = crate::commands::config::get_base_dir();
    let backup_dir = base_dir.join("backup");
    if !backup_dir.exists() {
        return Ok(Vec::new());
    }

    let mut list = Vec::new();
    if let Ok(entries) = fs::read_dir(&backup_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            if name.starts_with("env_backup_") && name.ends_with(".json") {
                if let Ok(content) = fs::read_to_string(entry.path()) {
                    if let Ok(backup) = serde_json::from_str::<EnvBackup>(&content) {
                        list.push(backup);
                    }
                }
            }
        }
    }

    // Sort backups by timestamp descending
    list.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(list)
}

#[tauri::command]
pub fn delete_env_backup(id: String) -> Result<(), String> {
    let base_dir = crate::commands::config::get_base_dir();
    let backup_file = base_dir.join("backup").join(format!("env_backup_{}.json", id));
    if backup_file.exists() {
        fs::remove_file(backup_file).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn restore_env_backup(id: String) -> Result<(), String> {
    let base_dir = crate::commands::config::get_base_dir();
    let backup_file = base_dir.join("backup").join(format!("env_backup_{}.json", id));
    if !backup_file.exists() {
        return Err("备份文件不存在".to_string());
    }

    let content = fs::read_to_string(backup_file).map_err(|e| e.to_string())?;
    let backup = serde_json::from_str::<EnvBackup>(&content).map_err(|e| e.to_string())?;

    // 1. Restore User Variables
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (user_key, _) = hkcu.create_subkey("Environment").map_err(|e| e.to_string())?;

    // Delete keys not in backup
    let existing_user_keys: Vec<String> = user_key.enum_values().filter_map(|x| x.ok()).map(|(n, _)| n).collect();
    for name in existing_user_keys {
        if !backup.user_vars.contains_key(&name) {
            let _ = user_key.delete_value(&name);
        }
    }

    // Restore keys from backup
    for (name, val) in &backup.user_vars {
        set_registry_value_smart(&user_key, name, val)?;
    }

    // 2. Restore System Variables (try, but don't fail if we lack permissions)
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let mut system_restore_msg = String::new();
    match hklm.open_subkey_with_flags("SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment", KEY_ALL_ACCESS) {
        Ok(sys_key) => {
            let existing_sys_keys: Vec<String> = sys_key.enum_values().filter_map(|x| x.ok()).map(|(n, _)| n).collect();
            for name in existing_sys_keys {
                if !backup.sys_vars.contains_key(&name) {
                    let _ = sys_key.delete_value(&name);
                }
            }
            for (name, val) in &backup.sys_vars {
                let _ = set_registry_value_smart(&sys_key, name, val);
            }
        }
        Err(_) => {
            system_restore_msg = "\n注意：系统级环境变量恢复失败（权限不足，请以管理员身份运行此程序进行完整恢复）。用户级环境变量已成功恢复！".to_string();
        }
    }

    broadcast_setting_change();

    if !system_restore_msg.is_empty() {
        return Err(system_restore_msg);
    }

    Ok(())
}

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

fn expand_env_vars(path_str: &str) -> String {
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

fn list_executables_in_dir(dir_path: &str) -> Vec<String> {
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
                    if name_lower.ends_with(".exe") || name_lower.ends_with(".cmd") || name_lower.ends_with(".bat") {
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
            if path_str.is_empty() { continue; }
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
            if path_str.is_empty() { continue; }
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
pub fn save_path_directories(user_paths: Vec<String>, system_paths: Vec<String>, save_system: bool) -> Result<(), String> {
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


