//! 项目注册表 — 从外部 projects.json 文件加载项目定义。

use super::types::ProjectDef;

pub fn registry() -> Vec<ProjectDef> {
    load_registry()
}

pub fn load_registry() -> Vec<ProjectDef> {
    let base_dir = crate::commands::config::get_base_dir();
    let mut search_dirs: Vec<std::path::PathBuf> = Vec::new();

    // 1. exe 同目录及向上 3 层
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            search_dirs.push(exe_dir.to_path_buf());
            let mut dir = exe_dir.to_path_buf();
            for _ in 0..5 {
                if let Some(parent) = dir.parent() {
                    dir = parent.to_path_buf();
                    search_dirs.push(dir.clone());
                }
            }
        }
    }

    // 2. 当前工作目录
    if let Ok(cwd) = std::env::current_dir() {
        search_dirs.push(cwd);
    }

    // 3. 用户配置目录
    search_dirs.push(base_dir);

    for dir in &search_dirs {
        // 优先检查 _up_ 子目录（Tauri 打包后 bundled resources 在此）
        let up_dir = dir.join("_up_");
        let candidates = [up_dir.as_path(), dir.as_path()];
        for candidate in &candidates {
            let path = candidate.join("projects.json");
            eprintln!("[registry] 检查: {} -> 存在: {}", path.display(), path.exists());
            if path.exists() {
                match std::fs::read_to_string(&path) {
                    Ok(data) => {
                        match serde_json::from_str::<Vec<ProjectDef>>(&data) {
                            Ok(list) => {
                                eprintln!("[registry] 成功加载 {} 个项目 from {}", list.len(), path.display());
                                if !list.is_empty() {
                                    return list;
                                }
                            }
                            Err(e) => eprintln!("[registry] JSON 解析失败: {}", e),
                        }
                    }
                    Err(e) => eprintln!("[registry] 读取失败: {}", e),
                }
            }
        }
    }

    eprintln!("[registry] 所有路径均未找到 projects.json");
    Vec::new()
}

pub fn find_by_id(id: &str) -> Option<ProjectDef> {
    registry().into_iter().find(|s| s.id == id)
}

pub fn all_ids() -> Vec<String> {
    registry().iter().map(|s| s.id.clone()).collect()
}
