use crate::commands::ai_registry::registry;
use std::process::Stdio;
use std::collections::HashMap;
use std::sync::Mutex;

/// 跟踪正在进行中的 升级/安装/卸载 操作，作为“进行中”状态的权威来源。
/// 前端在切换 Agent、切换页面或组件重新挂载后，仍可从 detect / versions 结果中
/// 读取到 busy 标记，从而持续显示“升级中/安装中/卸载中”，而不会被回退为“可升级 + 升级按钮”。
static TOOL_OPS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

pub fn set_tool_busy(tool_id: &str, op: &str) {
    if let Ok(mut m) = TOOL_OPS.lock() {
        m.get_or_insert_with(HashMap::new)
            .insert(tool_id.to_string(), op.to_string());
    }
}

pub fn clear_tool_busy(tool_id: &str) {
    if let Ok(mut m) = TOOL_OPS.lock() {
        if let Some(map) = m.as_mut() {
            map.remove(tool_id);
        }
    }
}

/// 供 detect / versions 命令读取某工具是否正在进行中操作
pub fn get_tool_busy(tool_id: &str) -> Option<String> {
    TOOL_OPS
        .lock()
        .ok()
        .and_then(|m| m.as_ref().and_then(|map| map.get(tool_id).cloned()))
}

/// 借用型守卫：离开作用域（含提前 return / ? 错误返回）时自动清除 busy 标记
struct ToolBusyGuard {
    id: String,
}

impl Drop for ToolBusyGuard {
    fn drop(&mut self) {
        clear_tool_busy(&self.id);
    }
}

#[tauri::command]
pub async fn install_ai_tool(tool_id: String) -> Result<String, String> {
    let reg = registry();
    let (_, paths) = reg.get_tool(&tool_id).ok_or("未知工具")?;
    let _busy_guard = ToolBusyGuard { id: tool_id.clone() };
    set_tool_busy(&tool_id, "installing");
    let install_cmd = &paths.install_cmd;
    let mut cmd = tokio::process::Command::new("cmd");
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW：禁止弹出命令提示符黑框
    let output = cmd
        .args(["/c", install_cmd])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("安装失败: {}", e))?;

    if output.status.success() {
        Ok("安装成功".to_string())
    } else {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if err.is_empty() {
            "安装失败".to_string()
        } else {
            err
        })
    }
}

#[tauri::command]
pub async fn upgrade_ai_tool(tool_id: String) -> Result<String, String> {
    let reg = registry();
    let (config, paths) = reg.get_tool(&tool_id).ok_or("未知工具")?;
    let _busy_guard = ToolBusyGuard { id: tool_id.clone() };
    set_tool_busy(&tool_id, "upgrading");
    let pkg_name = config.pkg_name.as_deref().unwrap_or(&config.id);
    let upgrade_cmd = match config.pkg_manager.as_deref() {
        Some("npm") => format!("npm install -g {}@latest", pkg_name),
        Some("pip") => format!("pip install --upgrade {}", pkg_name),
        _ => paths.install_cmd.clone(),
    };
    let mut cmd = tokio::process::Command::new("cmd");
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW：禁止弹出命令提示符黑框
    let output = cmd
        .args(["/c", &upgrade_cmd])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("升级失败: {}", e))?;

    if output.status.success() {
        Ok("升级成功".to_string())
    } else {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if err.is_empty() {
            "升级失败".to_string()
        } else {
            err
        })
    }
}

#[tauri::command]
pub async fn uninstall_ai_tool(tool_id: String) -> Result<String, String> {
    let reg = registry();
    let (config, paths) = reg.get_tool(&tool_id).ok_or("未知工具")?;
    let _busy_guard = ToolBusyGuard { id: tool_id.clone() };
    set_tool_busy(&tool_id, "uninstalling");
    let pkg_name = config.pkg_name.as_deref().unwrap_or(&config.id);
    let uninstall_cmd = match config.pkg_manager.as_deref() {
        Some("npm") => format!("npm uninstall -g {}", pkg_name),
        Some("pip") => format!("pip uninstall -y {}", pkg_name),
        _ => match &paths.uninstall_cmd {
            Some(c) => c.clone(),
            None => return Err("该工具未配置卸载命令".to_string()),
        },
    };
    let mut cmd = tokio::process::Command::new("cmd");
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW：禁止弹出命令提示符黑框
    let output = cmd
        .args(["/c", &uninstall_cmd])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("卸载失败: {}", e))?;

    if output.status.success() {
        Ok("卸载成功".to_string())
    } else {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if err.is_empty() {
            "卸载失败".to_string()
        } else {
            err
        })
    }
}
