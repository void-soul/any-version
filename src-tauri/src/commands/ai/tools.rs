use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tauri::AppHandle;
use tauri::Emitter;
use crate::commands::ai_registry::{registry, AiToolDefDto, ToolConfig, PathConfig};
use crate::commands::config::get_base_dir;
use crate::commands::tool_version::is_newer;
use crate::commands::hidden_cmd;
use crate::commands::cache::{get_dir_size, format_bytes, create_junction, migrate_pkg_storage_impl, clean_pkg_cache_impl};
use super::models::*;


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
