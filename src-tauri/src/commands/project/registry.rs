//! 项目注册表 — 从外部 projects.json 文件加载项目定义。
//!
//! 文件查找顺序（按优先级）：
//!   1. Tauri 资源目录 (打包后与 .exe 同目录)
//!   2. 可执行文件同目录
//!   3. 可执行文件的父目录 (开发模式 target/debug/ → 项目根)
//!   4. 当前工作目录
//!   5. %USERPROFILE%/.any-version/

use super::types::ProjectDef;

/// 全局项目注册表
pub fn registry() -> Vec<ProjectDef> {
    load_registry()
}

/// 从外部 projects.json 动态加载注册表
pub fn load_registry() -> Vec<ProjectDef> {
    let base_dir = crate::commands::config::get_base_dir();
    
    let candidates = build_search_paths(&base_dir);
    
    for path in &candidates {
        if path.exists() {
            if let Ok(data) = std::fs::read_to_string(path) {
                if let Ok(list) = serde_json::from_str::<Vec<ProjectDef>>(&data) {
                    if !list.is_empty() {
                        return list;
                    }
                }
            }
        }
    }
    
    Vec::new()
}

/// 构建搜索路径列表
fn build_search_paths(base_dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    
    // 1. Tauri 资源目录 (打包后)
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            // exe 同目录
            paths.push(exe_dir.join("projects.json"));
            
            // 开发模式: target/debug/ -> 项目根目录 (向上找 3 层)
            let mut ancestor = exe_dir.to_path_buf();
            for _ in 0..3 {
                if let Some(parent) = ancestor.parent() {
                    ancestor = parent.to_path_buf();
                    let candidate = ancestor.join("projects.json");
                    if candidate.exists() {
                        paths.push(candidate);
                    }
                }
            }
        }
    }
    
    // 2. 当前工作目录
    if let Ok(cwd) = std::env::current_dir() {
        paths.push(cwd.join("projects.json"));
    }
    
    // 3. 用户配置目录
    paths.push(base_dir.join("projects.json"));
    
    paths
}

/// 根据 id 查找项目定义
pub fn find_by_id(id: &str) -> Option<ProjectDef> {
    registry().into_iter().find(|s| s.id == id)
}

/// 返回所有项目 id 列表
pub fn all_ids() -> Vec<String> {
    registry().iter().map(|s| s.id.clone()).collect()
}
