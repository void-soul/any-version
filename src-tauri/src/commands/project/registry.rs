//! 项目注册表 — 从 projects/ 目录（每个子目录一个 config.json，零代码扩展）
//! 加载项目定义；兼容旧版单文件 projects.json。

use super::types::{EnvVarDef, FindRule, PackageManagerDef, ProjectDef};

use std::sync::RwLock;

/// 命令字符串白名单字符集。
/// `allow_shell = true` 时额外允许 shell 重定向/管道字符（`> < & | ;`）。
///
/// 安全说明：这些命令最终都会经 `std::process::Command` 直接 spawn（不经 shell 解释），
/// 重定向/管道字符只是字面参数，不会被解释执行，因此不存在注入风险。
/// 但「只读检测命令」（version_cmd / cache_detect_cmd 等）常需 `2>&1`、管道等合法写法
/// （如 `java -version 2>&1`），故对它们放宽；写操作命令保持严格。
fn is_safe_cmd(s: &str, allow_shell: bool) -> bool {
    let mut base = vec![
        ' ', '.', '/', '\\', '-', '_', ':', '{', '}', '=', '@', '%', '+', ',', '[', ']',
    ];
    if allow_shell {
        base.extend(['>', '<', '&', '|', ';']);
    }
    s.chars().all(|c| c.is_ascii_alphanumeric() || base.contains(&c))
}

/// 校验一个项目定义内的所有可执行命令字段，防止注册表注入任意 shell 命令。
/// 遇到含危险字符的命令，记录告警并将该字段置为 None（拒绝执行）。
/// 只读检测命令放宽重定向字符，写操作命令保持严格。
fn sanitize_project_cmds(def: &mut ProjectDef) {
    let check = |field: &str, val: &mut Option<String>, allow_shell: bool| {
        if let Some(ref s) = val {
            if !is_safe_cmd(s, allow_shell) {
                eprintln!(
                    "[registry] 拒绝危险命令字段 {}.{} = {:?}（含非法字符，疑似注入）",
                    def.id, field, s
                );
                *val = None;
            }
        }
    };
    // 只读检测命令：允许 `2>&1`、管道等合法写法
    check("version_cmd", &mut def.version_cmd, true);
    check("cache_detect_cmd", &mut def.cache_detect_cmd, true);
    check("cache_default_path", &mut def.cache_default_path, true);
    for pm in &mut def.package_managers {
        check("install_cmd", &mut pm.install_cmd, false);
        check("version_cmd", &mut pm.version_cmd, true);
        check("cache_detect_cmd", &mut pm.cache_detect_cmd, true);
        check("pkg_list_cmd", &mut pm.pkg_list_cmd, true);
        check("cache_set_cmd_template", &mut pm.cache_set_cmd_template, false);
        check("proxy_clear_cmd", &mut pm.proxy_clear_cmd, false);
        // 注意：PowerShell 类模板（proxy_set_cmd_template / mirror_cmd_template）
        // 本身合法包含 `$` `'` `;` `(` 等元字符，不能在此处严格白名单（会误杀 vcpkg 等）。
        // 注入防护改在替换点：用户输入值经 utils::validate_subst_value 校验后再拼入模板。
    }
}

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

    // 优先在 Tauri 2 打包后的官方资源目录下查找
    if let Some(res_dir) = crate::commands::utils::get_resource_dir() {
        search_dirs.push(res_dir);
    }

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
        for candidate in [dir.join("projects"), dir.join("_up_").join("projects")] {
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
                        Ok(mut list) => {
                            for def in &mut list {
                                sanitize_project_cmds(def);
                            }
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
                        sanitize_project_cmds(&mut def);
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
