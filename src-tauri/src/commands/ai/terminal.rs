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


// ─── 终端检测 ───

fn find_terminal(id: &str, name: &str, exe_names: &[&str]) -> Option<TerminalInfo> {
    for exe in exe_names {
        let output = hidden_cmd::hidden_cmd("cmd")
            .args(["/c", "where", exe])
            .output()
            .ok()?;
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let first_line = path.lines().next().unwrap_or(&path).to_string();
            if !first_line.is_empty() {
                return Some(TerminalInfo {
                    id: id.to_string(),
                    name: name.to_string(),
                    exe_path: first_line,
                });
            }
        }
    }
    None
}

/// 判断终端是否"外部终端"（带 launch_args 的即为外部终端，如 wezterm/alacritty/tabby）
pub(crate) fn is_ext_terminal(terminal_id: &str) -> bool {
    registry().terminals().terminals.get(terminal_id)
        .and_then(|t| t.launch_args.as_ref())
        .is_some()
}

/// 从 terminals.json 配置获取终端 exe 名称
pub(crate) fn get_terminal_exe_cfg(terminal_id: &str) -> String {
    registry().terminals().terminals.get(terminal_id)
        .and_then(|t| t.exe_names.first())
        .map(|s| s.clone())
        .unwrap_or_else(|| "cmd.exe".to_string())
}

#[tauri::command]
pub fn detect_terminals() -> Result<Vec<TerminalInfo>, String> {
    let mut terminals = Vec::new();

    // 从 terminals.json 驱动终端检测
    for (id, def) in registry().terminal_defs() {
        let exe_names: Vec<&str> = def.exe_names.iter().map(|s| s.as_str()).collect();
        if let Some(term) = find_terminal(id, &def.name, &exe_names) {
            terminals.push(term);
        }
    }

    // CMD 作为 fallback 总是可用
    if !terminals.iter().any(|t| t.id == "cmd") {
        terminals.push(TerminalInfo {
            id: "cmd".to_string(),
            name: "命令提示符".to_string(),
            exe_path: "cmd.exe".to_string(),
        });
    }

    Ok(terminals)
}
