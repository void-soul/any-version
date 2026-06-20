//! 项目注册表 — 从外部 projects.json 文件加载项目定义。
//!
//! 文件查找顺序：
//!   1. 可执行文件同目录/projects.json（打包后用户可见可编辑）
//!   2. %USERPROFILE%/.any-version/projects.json
//!
//! 如果文件不存在，返回空列表。用户可自行创建或从 AnyVersion 官方获取。

use serde_json;
use super::types::ProjectDef;

/// 全局项目注册表
pub fn registry() -> Vec<ProjectDef> {
    load_registry()
}

/// 从外部 projects.json 文件加载项目注册表
pub fn load_registry() -> Vec<ProjectDef> {
    // 优先从可执行文件同目录读取 projects.json
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_default();

    let candidates = vec![
        exe_dir.join("projects.json"),
        crate::commands::config::get_base_dir().join("projects.json"),
    ];

    for path in &candidates {
        if path.exists() {
            if let Ok(data) = std::fs::read_to_string(path) {
                if let Ok(list) = serde_json::from_str::<Vec<ProjectDef>>(&data) {
                    return list;
                }
            }
        }
    }

    // 所有路径都失败，返回空列表
    Vec::new()
}

/// 根据 id 查找项目定义
pub fn find_by_id(id: &str) -> Option<ProjectDef> {
    registry().into_iter().find(|s| s.id == id)
}

/// 返回所有项目 id 列表
pub fn all_ids() -> Vec<String> {
    registry().into_iter().map(|s| s.id).collect()
}
