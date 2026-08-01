//! 杂项能力：管理员权限、热重载、代理环境变量、日志清理、目录打开
//! 对齐 clash-party 的 checkAdminPrivileges / restartAsAdmin / copyEnv / hotReload 等

use crate::commands::mihomo::manager::{is_admin, reload_config};
use crate::commands::mihomo::MihomoState;
use crate::commands::hidden_cmd::hidden_cmd;
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::{AppHandle, State};

/// 当前进程是否具备管理员权限
#[tauri::command]
pub fn mihomo_check_admin() -> bool {
    is_admin()
}

/// 以管理员身份重启应用（UAC 提权）
#[tauri::command]
pub fn mihomo_restart_as_admin(app: AppHandle) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let path = exe.to_string_lossy().to_string();
    hidden_cmd("powershell")
        .args([
            "-NoProfile",
            "-WindowStyle",
            "Hidden",
            "-Command",
            &format!("Start-Process -FilePath '{}' -Verb RunAs", path.replace('\'', "''")),
        ])
        .spawn()
        .map_err(|e| format!("提权失败: {e}"))?;
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(600));
        app.exit(0);
    });
    Ok(())
}

/// TUN 运行条件自检
#[tauri::command]
pub fn mihomo_check_tun_permissions(state: State<'_, MihomoState>) -> Value {
    let app_config = state.app_config.lock().unwrap().clone();
    let core = crate::commands::mihomo::manager::resolve_core_path(&app_config);
    let wintun = core
        .parent()
        .map(|d| d.join("wintun.dll").exists())
        .unwrap_or(false);
    json!({
        "admin": is_admin(),
        "corePath": core.to_string_lossy(),
        "coreExists": core.exists(),
        "wintun": wintun,
        "ok": is_admin() && core.exists(),
    })
}

/// 热重载配置（不重启内核进程）
#[tauri::command]
pub async fn mihomo_hot_reload(
    app: AppHandle,
    state: State<'_, MihomoState>,
) -> Result<(), String> {
    let inner: Arc<_> = Arc::clone(&state);
    reload_config(&app, inner).await
}

/// 生成代理环境变量命令串（cmd / powershell / bash）
#[tauri::command]
pub fn mihomo_copy_env(state: State<'_, MihomoState>, kind: String) -> String {
    let port = state.app_config.lock().unwrap().mixed_port;
    let host = format!("127.0.0.1:{port}");
    match kind.as_str() {
        "cmd" => format!(
            "set http_proxy=http://{host}\r\nset https_proxy=http://{host}\r\nset all_proxy=socks5://{host}"
        ),
        "powershell" => format!(
            "$env:HTTP_PROXY=\"http://{host}\"; $env:HTTPS_PROXY=\"http://{host}\"; $env:ALL_PROXY=\"socks5://{host}\""
        ),
        _ => format!(
            "export http_proxy=http://{host}\nexport https_proxy=http://{host}\nexport all_proxy=socks5://{host}"
        ),
    }
}

/// 清空内核日志文件
#[tauri::command]
pub fn mihomo_clear_logs(state: State<'_, MihomoState>) -> Result<(), String> {
    std::fs::write(&state.log_file, b"").map_err(|e| e.to_string())
}

/// 打开 mihomo 数据目录 / 内核目录 / 日志文件
#[tauri::command]
pub fn mihomo_open_path(state: State<'_, MihomoState>, kind: String) -> Result<(), String> {
    let app_config = state.app_config.lock().unwrap().clone();
    let target = match kind.as_str() {
        "core" => crate::commands::mihomo::manager::core_dir(&app_config),
        "log" => state.log_file.clone(),
        _ => state.data_dir.clone(),
    };
    if let Some(p) = target.parent() {
        std::fs::create_dir_all(p).ok();
    }
    let arg = target.to_string_lossy().to_string();
    std::process::Command::new("explorer")
        .arg(if target.is_dir() { arg } else { format!("/select,{arg}") })
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 日志文件按保留天数清理（maxLogDays）
#[tauri::command]
pub fn mihomo_cleanup_logs(state: State<'_, MihomoState>) -> Result<u32, String> {
    let days = state
        .app_config
        .lock()
        .unwrap()
        .extra
        .get("maxLogDays")
        .and_then(|v| v.as_u64())
        .unwrap_or(7);
    let dir = state.data_dir.join("logs");
    let mut removed = 0u32;
    // F12 修复：用 checked_sub 防 Duration 下溢 panic
    let now = std::time::SystemTime::now();
    let cutoff = now
        .checked_sub(std::time::Duration::from_secs(days * 86400))
        .unwrap_or(now);
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            if let Ok(meta) = e.metadata() {
                if let Ok(mtime) = meta.modified() {
                    if mtime < cutoff && std::fs::remove_file(e.path()).is_ok() {
                        removed += 1;
                    }
                }
            }
        }
    }
    Ok(removed)
}
