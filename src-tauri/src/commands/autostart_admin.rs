//! 以管理员身份开机自启（任务计划程序方案）。
//!
//! Windows 的 UAC 不允许普通注册表 Run 键 / 启动文件夹项自动继承管理员令牌，
//! 但「任务计划程序」创建的任务可勾选「使用最高权限 (/RL HIGHEST)」，在用户登录时
//! 静默以管理员身份拉起目标进程，且不会弹出 UAC 确认框。
//!
//! 这里用当前用户 (/RU <当前用户名>) + ONLOGON 触发器 + /RL HIGHEST，
//! 既能在用户桌面会话里运行（托盘/剪贴板可用），又具备管理员权限。
//!
//! 注意：创建/删除任务本身需要管理员权限（schtasks 写任务计划程序库），
//! 因此调用 enable 前应先判断 is_admin()，非管理员时让前端提示用管理员重启。

use std::process::Command;

const TASK_NAME: &str = "AnyVersionAdminAutostart";

/// 返回当前 exe 的完整路径。
#[cfg(windows)]
fn current_exe_path() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// 返回当前登录用户名（用于 /RU，确保任务在用户会话里运行）。
#[cfg(windows)]
fn current_user_name() -> String {
    std::env::var("USERNAME").unwrap_or_else(|_| "Administrator".to_string())
}

/// 用 schtasks 创建高权限登录自启任务。
/// 已存在同名任务时先删除再重建，保证参数（路径/参数）最新。
#[tauri::command]
pub fn enable_admin_autostart(_app: tauri::AppHandle) -> Result<String, String> {
    #[cfg(not(windows))]
    {
        let _ = _app;
        return Err("仅 Windows 支持管理员自启".into());
    }

    #[cfg(windows)]
    {
        let exe = current_exe_path();
        if exe.is_empty() {
            return Err("无法获取当前可执行文件路径".into());
        }
        let user = current_user_name();

        // 先尝试删除可能存在的旧任务
        let _ = Command::new("schtasks")
            .args(["/Delete", "/TN", TASK_NAME, "/F"])
            .output();

        // 创建 ONLOGON + 最高权限任务，目标 exe 带 --minimized 静默驻留托盘
        let status = Command::new("schtasks")
            .args([
                "/Create",
                "/TN",
                TASK_NAME,
                "/TR",
                &format!("\"{}\" --minimized", exe),
                "/SC",
                "ONLOGON",
                "/RL",
                "HIGHEST",
                "/RU",
                &user,
                "/F",
            ])
            .output()
            .map_err(|e| format!("创建任务计划失败: {}", e))?;

        if !status.status.success() {
            let msg = String::from_utf8_lossy(&status.stderr).to_string();
            return Err(format!(
                "创建管理员自启任务失败（可能需要以管理员身份运行 Kira）: {}",
                msg.trim()
            ));
        }

        Ok("已创建管理员开机自启任务".into())
    }
}

/// 删除高权限登录自启任务。
#[tauri::command]
pub fn disable_admin_autostart() -> Result<String, String> {
    #[cfg(not(windows))]
    {
        return Ok("非 Windows，无需操作".into());
    }

    #[cfg(windows)]
    {
        let status = Command::new("schtasks")
            .args(["/Delete", "/TN", TASK_NAME, "/F"])
            .output()
            .map_err(|e| format!("删除任务计划失败: {}", e))?;

        // 任务不存在也视为成功（exit code 非 0 但 stderr 可能只是 "找不到"）
        let stderr = String::from_utf8_lossy(&status.stderr).to_string();
        if status.status.success() || stderr.contains("找不到") || stderr.contains("cannot find") {
            Ok("已移除管理员开机自启任务".into())
        } else {
            Err(format!("移除管理员自启任务失败: {}", stderr.trim()))
        }
    }
}

/// 查询高权限登录自启任务是否存在（反映系统真实状态）。
#[tauri::command]
pub fn is_admin_autostart_enabled() -> bool {
    #[cfg(not(windows))]
    {
        return false;
    }

    #[cfg(windows)]
    {
        let status = Command::new("schtasks")
            .args(["/Query", "/TN", TASK_NAME])
            .output();
        match status {
            Ok(out) => out.status.success(),
            Err(_) => false,
        }
    }
}
