use super::*;
use std::collections::HashMap;
use std::io::Read;
use winreg::types::FromRegValue;

const SYSTEM_ENV: &str = "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment";
const MAX_BACKUP_BYTES: u64 = 8 * 1024 * 1024;

pub(super) fn backup_path(root: &Path, id: &str) -> Result<std::path::PathBuf, String> {
    if id.is_empty() || id.len() > 96 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return Err("无效的环境变量备份ID".into());
    }
    Ok(root.join("backup").join(format!("env_backup_{id}.json")))
}

pub(super) fn collect_snapshot(
    values: impl Iterator<Item = std::io::Result<(String, winreg::RegValue)>>,
) -> Result<HashMap<String, String>, String> {
    let mut snapshot = HashMap::new();
    for value in values {
        let (name, raw) = value.map_err(|e| format!("枚举环境变量失败: {e}"))?;
        let value = String::from_reg_value(&raw).map_err(|e| format!("读取环境变量 {name} 失败: {e}"))?;
        snapshot.insert(name, value);
    }
    Ok(snapshot)
}

fn read_snapshot(root: winreg::HKEY, path: &str, missing_ok: bool) -> Result<HashMap<String, String>, String> {
    match RegKey::predef(root).open_subkey(path) {
        Ok(key) => collect_snapshot(key.enum_values()),
        Err(e) if missing_ok && e.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
        Err(e) => Err(format!("读取环境变量注册表失败: {e}")),
    }
}

fn load_backup(path: &Path) -> Result<EnvBackup, String> {
    let file = fs::File::open(path).map_err(|e| format!("读取环境变量备份失败: {e}"))?;
    let mut bytes = Vec::new();
    file.take(MAX_BACKUP_BYTES + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BACKUP_BYTES { return Err("环境变量备份超过8 MiB".into()); }
    serde_json::from_slice(&bytes).map_err(|e| format!("解析环境变量备份失败: {e}"))
}

#[tauri::command]
pub fn create_env_backup(description: String) -> Result<EnvBackup, String> {
    // A failed read is never a valid empty snapshot: restore removes absent values.
    let user_vars = read_snapshot(HKEY_CURRENT_USER, "Environment", true)?;
    let sys_vars = read_snapshot(HKEY_LOCAL_MACHINE, SYSTEM_ENV, false)?;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?.as_nanos();
    let id = format!("{now}-{}", NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let backup = EnvBackup {
        id, timestamp: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(), description, user_vars, sys_vars,
    };
    let path = backup_path(&crate::commands::config::get_data_dir(), &backup.id)?;
    let data = serde_json::to_vec_pretty(&backup).map_err(|e| e.to_string())?;
    if data.len() as u64 > MAX_BACKUP_BYTES { return Err("环境变量备份超过8 MiB".into()); }
    crate::commands::config::atomic_write_file(&path, &data)?;
    Ok(backup)
}

#[tauri::command]
pub fn list_env_backups() -> Result<Vec<EnvBackup>, String> {
    let root = crate::commands::config::get_data_dir();
    let entries = match fs::read_dir(root.join("backup")) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("读取备份目录失败: {e}")),
    };
    let mut list = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(id) = name.strip_prefix("env_backup_").and_then(|name| name.strip_suffix(".json")) {
            let backup = load_backup(&backup_path(&root, id)?)?;
            if backup.id != id { return Err("环境变量备份ID与文件名不一致".into()); }
            list.push(backup);
        }
    }
    list.sort_by(|a, b| b.timestamp.cmp(&a.timestamp).then_with(|| b.id.cmp(&a.id)));
    Ok(list)
}

#[tauri::command]
pub fn delete_env_backup(id: String) -> Result<(), String> {
    let path = backup_path(&crate::commands::config::get_data_dir(), &id)?;
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

pub(super) fn validate_restore(backup: &EnvBackup) -> Result<(), String> {
    if backup.sys_vars.is_empty() { return Err("系统级环境变量备份为空，无法确认备份完整性，已停止恢复".into()); }
    for (name, value) in backup.user_vars.iter().chain(&backup.sys_vars) {
        if name.is_empty() || name.contains(['\0', '=']) || value.contains('\0') {
            return Err("备份包含无效的环境变量名称或内容".into());
        }
    }
    Ok(())
}

fn restore_values(key: &RegKey, values: &HashMap<String, String>) -> Result<(), String> {
    // Enumerate completely before the first deletion; iteration errors must abort.
    let names: Vec<String> = key.enum_values().map(|row| row.map(|(name, _)| name)).collect::<Result<_, _>>().map_err(|e| e.to_string())?;
    for name in names {
        if !values.keys().any(|saved| saved.eq_ignore_ascii_case(&name)) {
            key.delete_value(&name).map_err(|e| format!("删除环境变量 {name} 失败: {e}"))?;
        }
    }
    for (name, value) in values { set_registry_value_smart(key, name, value)?; }
    Ok(())
}

#[tauri::command]
pub fn restore_env_backup(id: String) -> Result<(), String> {
    let backup = load_backup(&backup_path(&crate::commands::config::get_data_dir(), &id)?)?;
    if backup.id != id { return Err("环境变量备份ID与文件名不一致".into()); }
    validate_restore(&backup)?;
    // Require a complete recovery point before deleting or changing registry values.
    create_env_backup(format!("Before restore {id}"))?;
    let result = (|| {
        let (user, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey("Environment").map_err(|e| e.to_string())?;
        restore_values(&user, &backup.user_vars).map_err(|e| format!("用户级环境变量恢复失败，可能已有部分更改: {e}"))?;
        let system = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(SYSTEM_ENV, KEY_ALL_ACCESS)
            .map_err(|e| format!("系统级环境变量恢复失败（用户级已恢复）: {e}"))?;
        restore_values(&system, &backup.sys_vars).map_err(|e| format!("系统级环境变量恢复失败（用户级已恢复，系统级可能已有部分更改）: {e}"))
    })();
    broadcast_setting_change();
    crate::sync_process_path();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use winreg::types::ToRegValue;
    #[test]
    fn failed_or_nonstring_snapshot_is_not_an_empty_backup() {
        assert!(collect_snapshot(std::iter::once(Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)))).is_err());
        assert!(collect_snapshot(std::iter::once(Ok(("bad".into(), 1u32.to_reg_value())))).is_err());
        let snapshot = collect_snapshot(std::iter::once(Ok(("PATH".into(), "%ROOT%\\bin".to_reg_value())))).unwrap();
        assert_eq!(snapshot["PATH"], "%ROOT%\\bin");
    }
    #[test]
    fn backup_ids_cannot_escape_the_backup_directory() {
        for id in ["", "../x", "a/b", "a\\b", "C:secret", "a.b"] { assert!(backup_path(Path::new("root"), id).is_err()); }
        assert!(backup_path(Path::new("root"), "123-4").unwrap().ends_with("env_backup_123-4.json"));
        assert!(backup_path(Path::new("root"), "123").is_ok());
    }
    #[test]
    fn incomplete_or_invalid_legacy_restore_is_rejected_before_mutation() {
        let mut backup = EnvBackup::default();
        assert!(validate_restore(&backup).is_err());
        backup.sys_vars.insert("PATH".into(), "C:\\Windows".into());
        assert!(validate_restore(&backup).is_ok());
        backup.user_vars.insert("BAD=NAME".into(), "value".into());
        assert!(validate_restore(&backup).is_err());
    }
}
