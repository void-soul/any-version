use crate::commands::ai_registry::registry;
use std::process::Stdio;


#[tauri::command]
pub async fn upgrade_ai_tool(tool_id: String) -> Result<String, String> {
    let reg = registry();
    let (_, paths) = reg.get_tool(&tool_id).ok_or("未知工具")?;
    let install_cmd = &paths.install_cmd;
    let output = tokio::process::Command::new("cmd")
        .args(["/c", install_cmd])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("升级失败: {}", e))?;

    if output.status.success() {
        Ok("升级成功".to_string())
    } else {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if err.is_empty() { "升级失败".to_string() } else { err })
    }
}
