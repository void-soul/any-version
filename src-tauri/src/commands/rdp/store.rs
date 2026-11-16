use super::models::RdpConnection;
use std::path::PathBuf;
use std::sync::Mutex;

static STORE_LOCK: Mutex<()> = Mutex::new(());

fn store_path() -> PathBuf {
    crate::commands::config::get_data_dir()
        .join("rdp")
        .join("connections.json")
}

/// 文件不存在 = 空列表；文件损坏 = 报错且不覆盖原文件。
pub fn load() -> Result<Vec<RdpConnection>, String> {
    let _guard = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = store_path();
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("读取远程桌面连接失败 {}: {e}", path.display())),
    };
    serde_json::from_slice(&bytes)
        .map_err(|e| format!("远程桌面连接文件无效，未覆盖原文件 {}: {e}", path.display()))
}

pub fn save(items: &[RdpConnection]) -> Result<(), String> {
    let _guard = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let bytes = serde_json::to_vec_pretty(items).map_err(|e| e.to_string())?;
    crate::commands::foundation::atomic_write_file(&store_path(), &bytes)
}
