//! 项目注册表 — 从 projects/ 目录（每个子目录一个 config.json，零代码扩展）
//! 加载项目定义；兼容旧版单文件 projects.json。

use super::types::{EnvVarDef, FindRule, PackageManagerDef, ProjectDef};

use std::sync::RwLock;

static REGISTRY_CACHE: RwLock<Option<Vec<ProjectDef>>> = RwLock::new(None);

pub fn registry() -> Vec<ProjectDef> {
    {
        let read_guard = REGISTRY_CACHE.read().unwrap();
        if let Some(ref list) = *read_guard {
            return list.clone();
        }
    }

    let list = load_registry();
    let mut write_guard = REGISTRY_CACHE.write().unwrap();
    *write_guard = Some(list.clone());
    list
}

pub fn clear_registry_cache() {
    let mut write_guard = REGISTRY_CACHE.write().unwrap();
    *write_guard = None;
}

pub fn load_registry() -> Vec<ProjectDef> {
    let base_dir = crate::commands::config::get_base_dir();
    let mut search_dirs: Vec<std::path::PathBuf> = Vec::new();

    // 1. exe 同目录及向上 5 层
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

    // 4. 优先从 projects/ 目录加载（每个子目录一个 config.json，零代码扩展）
    for dir in &search_dirs {
        for candidate in [dir.join("_up_").join("projects"), dir.join("projects")] {
            if let Some(list) = load_from_dir(&candidate) {
                if !list.is_empty() {
                    eprintln!("[registry] 从目录加载 {} 个项目: {}", list.len(), candidate.display());
                    return list;
                }
            }
        }
    }

    // 5. 兼容旧版单文件 projects.json
    for dir in &search_dirs {
        let up_dir = dir.join("_up_");
        let candidates = [up_dir.as_path(), dir.as_path()];
        for candidate in &candidates {
            let path = candidate.join("projects.json");
            if path.exists() {
                match std::fs::read_to_string(&path) {
                    Ok(data) => match serde_json::from_str::<Vec<ProjectDef>>(&data) {
                        Ok(list) => {
                            if !list.is_empty() {
                                eprintln!("[registry] 从 projects.json 加载 {} 个项目: {}", list.len(), path.display());
                                return list;
                            }
                        }
                        Err(e) => eprintln!("[registry] JSON 解析失败: {}", e),
                    },
                    Err(e) => eprintln!("[registry] 读取失败: {}", e),
                }
            }
        }
    }

    eprintln!("[registry] 未找到 projects 配置（projects/ 目录与 projects.json 均不存在或为空）");
    Vec::new()
}

/// 从 projects/ 子目录逐个加载 config.json，聚合为项目定义列表。
/// 每个子目录对应一个 SDK，目录名即 SDK 位置（以 config.json 内的 id 为准）。
/// 复杂的数组/对象字段（env_vars / find_rules / package_managers /
/// remote_versions_config）可拆分到同名独立文件中，加载时按文件优先覆盖内联值。
fn load_from_dir(dir: &std::path::Path) -> Option<Vec<ProjectDef>> {
    if !dir.exists() || !dir.is_dir() {
        return None;
    }
    let mut list: Vec<ProjectDef> = Vec::new();
    let mut found = false;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let cfg = path.join("config.json");
            if !cfg.exists() {
                continue;
            }
            match std::fs::read_to_string(&cfg) {
                Ok(s) => match serde_json::from_str::<ProjectDef>(&s) {
                    Ok(mut def) => {
                        // 从拆分出的独立文件覆盖复杂字段（文件优先于 config.json 内联值）
                        if let Some(v) = read_json_file::<Vec<EnvVarDef>>(&path.join("env_vars.json")) {
                            def.env_vars = v;
                        }
                        if let Some(v) = read_json_file::<Vec<FindRule>>(&path.join("find_rules.json")) {
                            def.find_rules = v;
                        }
                        if let Some(v) = read_json_file::<Vec<PackageManagerDef>>(&path.join("package_managers.json")) {
                            def.package_managers = v;
                        }
                        if let Some(v) = read_json_file::<serde_json::Value>(&path.join("remote_versions_config.json")) {
                            def.remote_versions_config = Some(v);
                        }
                        list.push(def);
                        found = true;
                    }
                    Err(e) => eprintln!("[registry] 解析失败 {}: {}", cfg.display(), e),
                },
                Err(e) => eprintln!("[registry] 读取失败 {}: {}", cfg.display(), e),
            }
        }
    }
    if found { Some(list) } else { None }
}

/// 读取并解析一个 JSON 文件为指定类型；文件不存在或解析失败返回 None。
fn read_json_file<T: serde::de::DeserializeOwned>(path: &std::path::Path) -> Option<T> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

pub fn find_by_id(id: &str) -> Option<ProjectDef> {
    registry().into_iter().find(|s| s.id == id)
}

pub fn all_ids() -> Vec<String> {
    registry().iter().map(|s| s.id.clone()).collect()
}
