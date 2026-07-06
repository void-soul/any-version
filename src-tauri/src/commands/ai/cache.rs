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


// ─── AI 工具缓存管理 ───

/// 获取 AI 工具的缓存/数据信息（复用 SDK 统一缓存管理，仅支持 Junction）
#[derive(Serialize, Clone, Debug)]
pub struct AiToolCacheInfo {
    pub tool_id: String,
    pub dir_name: String,
    pub full_path: String,
    pub size: String,
    pub size_bytes: u64,
    pub is_junction: bool,
    pub junction_target: String,
    pub exists: bool,
}

#[tauri::command]
pub fn get_ai_tool_cache_info() -> Result<Vec<AiToolCacheInfo>, String> {
    let home = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default());
    let mut results = Vec::new();

    for (tool_id, (config, _)) in registry().tool_iter() {
        for cache_dir in &config.cache_dirs {
            let full_path = home.join(cache_dir);
            if !full_path.exists() {
                results.push(AiToolCacheInfo {
                    tool_id: tool_id.to_string(),
                    dir_name: cache_dir.to_string(),
                    full_path: full_path.to_string_lossy().to_string(),
                    size: "0 B".to_string(),
                    size_bytes: 0,
                    is_junction: false,
                    junction_target: String::new(),
                    exists: false,
                });
                continue;
            }

            let is_junction = fs::symlink_metadata(&full_path)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false);
            let junction_target = if is_junction {
                fs::read_link(&full_path)
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default()
            } else {
                String::new()
            };

            let size_bytes = get_dir_size(&full_path);
            let size_str = format_bytes(size_bytes);

            results.push(AiToolCacheInfo {
                tool_id: tool_id.to_string(),
                dir_name: cache_dir.to_string(),
                full_path: full_path.to_string_lossy().to_string(),
                size: size_str,
                size_bytes,
                is_junction,
                junction_target,
                exists: true,
            });
        }
    }

    results.sort_by(|a, b| {
        b.size_bytes.cmp(&a.size_bytes)
    });
    Ok(results)
}

/// 迁移 AI 工具缓存目录（复用 SDK 统一缓存管理，仅支持 Junction，禁止直接指向目录）
#[tauri::command]
pub fn migrate_ai_tool_cache(
    app: AppHandle,
    _tool_id: String,
    dir_name: String,
    new_path: String,
) -> Result<(), String> {
    let home = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default());
    let orig_path = home.join(&dir_name);
    let target_path = PathBuf::from(&new_path);

    if orig_path.to_string_lossy() == target_path.to_string_lossy() {
        return Err("原路径与目标路径相同".to_string());
    }

    // 禁止直接指向 C 盘：必须迁移到非 C 盘以释放 C 盘空间
    if new_path.to_lowercase().starts_with("c:") {
        return Err("AI 工具缓存只能迁移到非 C 盘（例如 D:\\...），禁止直接指向 C 盘目录".to_string());
    }

    let orig_path_str = orig_path.to_string_lossy().to_string();
    let target_path_str = target_path.to_string_lossy().to_string();

    // 复用 SDK 统一缓存迁移：安全拷贝模式（不可先删），且始终创建 Junction
    migrate_pkg_storage_impl(&app, &orig_path_str, &target_path_str, "cache", false)
}

/// 清理 AI 工具缓存目录
#[tauri::command]
pub fn clean_ai_tool_cache(
    app: AppHandle,
    _tool_id: String,
    dir_name: String,
) -> Result<(), String> {
    let home = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default());
    let cache_path = home.join(&dir_name);
    let cache_path_str = cache_path.to_string_lossy().to_string();
    crate::commands::cache::clean_pkg_cache_impl(&app, &cache_path_str)
}

/// 在资源管理器中打开工具缓存目录（按 dir_name）
#[tauri::command]
pub fn open_ai_tool_cache_dir(dir_name: String) -> Result<(), String> {
    let home = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default());
    let cache_path = home.join(&dir_name);
    if cache_path.exists() {
        std::process::Command::new("explorer")
            .arg(&cache_path)
            .spawn()
            .map_err(|e| format!("打开目录失败: {}", e))?;
    }
    Ok(())
}

/// 在资源管理器中打开工具缓存目录（按 full_path）
#[tauri::command]
pub fn open_ai_tool_cache_dir_path(full_path: String) -> Result<(), String> {
    let cache_path = PathBuf::from(&full_path);
    if cache_path.exists() {
        std::process::Command::new("explorer")
            .arg(&cache_path)
            .spawn()
            .map_err(|e| format!("打开目录失败: {}", e))?;
    }
    Ok(())
}
