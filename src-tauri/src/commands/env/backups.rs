use super::*;

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
    if let Ok(env_key) =
        hklm.open_subkey("SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment")
    {
        for name in env_key.enum_values().filter_map(|x| x.ok()).map(|(n, _)| n) {
            if let Ok(val) = env_key.get_value::<String, _>(&name) {
                sys_vars.insert(name, val);
            }
        }
    }

    let id = format!(
        "{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    );

    // Get simple local timestamp using powershell date
    let timestamp = crate::commands::hidden_cmd::hidden_cmd("powershell")
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

    let base_dir = crate::commands::config::get_data_dir();
    let backup_dir = base_dir.join("backup");
    fs::create_dir_all(&backup_dir).map_err(|e| e.to_string())?;
    let backup_file = backup_dir.join(format!("env_backup_{}.json", backup.id));
    let data = serde_json::to_string_pretty(&backup).map_err(|e| e.to_string())?;
    fs::write(backup_file, data).map_err(|e| e.to_string())?;

    Ok(backup)
}

#[tauri::command]
pub fn list_env_backups() -> Result<Vec<EnvBackup>, String> {
    let base_dir = crate::commands::config::get_data_dir();
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
    let base_dir = crate::commands::config::get_data_dir();
    let backup_file = base_dir
        .join("backup")
        .join(format!("env_backup_{}.json", id));
    if backup_file.exists() {
        fs::remove_file(backup_file).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn restore_env_backup(id: String) -> Result<(), String> {
    let base_dir = crate::commands::config::get_data_dir();
    let backup_file = base_dir
        .join("backup")
        .join(format!("env_backup_{}.json", id));
    if !backup_file.exists() {
        return Err("备份文件不存在".to_string());
    }

    let content = fs::read_to_string(backup_file).map_err(|e| e.to_string())?;
    let backup = serde_json::from_str::<EnvBackup>(&content).map_err(|e| e.to_string())?;

    // 1. Restore User Variables
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (user_key, _) = hkcu
        .create_subkey("Environment")
        .map_err(|e| e.to_string())?;

    // Delete keys not in backup
    let existing_user_keys: Vec<String> = user_key
        .enum_values()
        .filter_map(|x| x.ok())
        .map(|(n, _)| n)
        .collect();
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
    match hklm.open_subkey_with_flags(
        "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
        KEY_ALL_ACCESS,
    ) {
        Ok(sys_key) => {
            let existing_sys_keys: Vec<String> = sys_key
                .enum_values()
                .filter_map(|x| x.ok())
                .map(|(n, _)| n)
                .collect();
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
