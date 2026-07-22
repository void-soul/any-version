use crate::commands::ai_registry::registry;
use std::process::Stdio;

#[tauri::command]
pub async fn install_ai_tool(tool_id: String) -> Result<String, String> {
    let reg = registry();
    let (_, paths) = reg.get_tool(&tool_id).ok_or("未知工具")?;
    let install_cmd = &paths.install_cmd;
    let output = tokio::process::Command::new("cmd")
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
    let pkg_name = config.pkg_name.as_deref().unwrap_or(&config.id);
    let upgrade_cmd = match config.pkg_manager.as_deref() {
        Some("npm") => format!("npm install -g {}@latest", pkg_name),
        Some("pip") => format!("pip install --upgrade {}", pkg_name),
        _ => paths.install_cmd.clone(),
    };
    let output = tokio::process::Command::new("cmd")
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
    let pkg_name = config.pkg_name.as_deref().unwrap_or(&config.id);
    let uninstall_cmd = match config.pkg_manager.as_deref() {
        Some("npm") => format!("npm uninstall -g {}", pkg_name),
        Some("pip") => format!("pip uninstall -y {}", pkg_name),
        _ => match &paths.uninstall_cmd {
            Some(c) => c.clone(),
            None => return Err("该工具未配置卸载命令".to_string()),
        },
    };
    let output = tokio::process::Command::new("cmd")
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
