use std::fs;
use std::path::PathBuf;

fn get_hosts_path() -> PathBuf {
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
    PathBuf::from(system_root).join("System32\\drivers\\etc\\hosts")
}

#[tauri::command]
pub fn read_hosts() -> Result<String, String> {
    let path = get_hosts_path();
    fs::read_to_string(&path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn write_hosts(content: String) -> Result<(), String> {
    let path = get_hosts_path();
    fs::write(&path, content).map_err(|e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            "无修改权限。请以管理员身份运行此程序以修改 hosts 文件。".to_string()
        } else {
            e.to_string()
        }
    })
}
