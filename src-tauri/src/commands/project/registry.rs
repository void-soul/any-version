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
///
/// 目录下没有 config.json 的子目录会被跳过，不会成为项目。
pub(crate) fn load_from_dir(dir: &std::path::Path) -> Option<Vec<ProjectDef>> {
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

#[cfg(test)]
mod tests {
    use super::super::types::ResolvePattern;
    use super::*;
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};

    /// 仓库内 `projects/` 目录。
    /// `cargo test` 的工作目录是本 crate（`src-tauri`），故取 CARGO_MANIFEST_DIR 的上一级，
    /// **不受打包产物 `src-tauri/_up_/projects` 影响**，测到的一定是仓库里的真配置。
    fn repo_projects_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("src-tauri 应有上级目录")
            .join("projects")
    }

    fn load_all() -> Vec<ProjectDef> {
        let dir = repo_projects_dir();
        assert!(dir.is_dir(), "未找到 projects 目录: {}", dir.display());
        load_from_dir(&dir).unwrap_or_else(|| {
            panic!("{} 未解析出任何项目（是否存在 JSON 语法错误？）", dir.display())
        })
    }

    /// 目录类字段允许出现的占位符：共享目录根 + 通用模板变量。
    fn allowed_placeholders() -> Vec<String> {
        let mut names = crate::commands::project::dirs::root_names();
        names.extend(
            ["install_root", "home", "data_dir", "version"]
                .iter()
                .map(|s| s.to_string()),
        );
        names
    }

    fn unknown_placeholders(text: &str, allowed: &[String]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut rest = text;
        while let Some(start) = rest.find('{') {
            let after = &rest[start + 1..];
            let Some(end) = after.find('}') else { break };
            let name = &after[..end];
            if !allowed.iter().any(|a| a == name) && !out.iter().any(|o| o == name) {
                out.push(name.to_string());
            }
            rest = &after[end + 1..];
        }
        out
    }

    /// 任何一份 config.json 出错都会让该项目从界面上静默消失，必须由测试挡住。
    #[test]
    fn every_project_dir_parses() {
        let dir = repo_projects_dir();
        let expected = std::fs::read_dir(&dir)
            .expect("读取 projects 目录失败")
            .flatten()
            .filter(|e| e.path().is_dir() && e.path().join("config.json").exists())
            .count();
        let list = load_all();
        assert!(
            expected >= 20,
            "projects/ 下带 config.json 的子目录只有 {} 个，疑似配置被误删",
            expected
        );
        assert_eq!(
            list.len(),
            expected,
            "{} 个目录存在 config.json，却只解析出 {} 个项目",
            expected,
            list.len()
        );
    }

    /// 拆分出来的独立配置文件（env_vars / find_rules / package_managers /
    /// remote_versions_config）解析失败时，`load_from_dir` 会**静默回退**到 config.json
    /// 的内联值（通常为空），项目本身仍能加载成功。因此必须单独验证它们能解析：
    /// 改坏一份 find_rules.json 只会表现为「这个项目的安装来源检测全部失效」，
    /// 不会有任何报错。
    #[test]
    fn split_config_files_parse() {
        let dir = repo_projects_dir();
        let mut checked = 0usize;
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if !path.is_dir() || !path.join("config.json").exists() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();

            let fr = path.join("find_rules.json");
            if fr.exists() {
                let list: Vec<super::super::types::FindRule> = read_json_file(&fr)
                    .unwrap_or_else(|| panic!("{} 的 find_rules.json 无法解析", name));
                assert!(!list.is_empty(), "{} 的 find_rules.json 为空", name);
                checked += 1;
            }
            let pm = path.join("package_managers.json");
            if pm.exists() {
                let _: Vec<super::super::types::PackageManagerDef> = read_json_file(&pm)
                    .unwrap_or_else(|| panic!("{} 的 package_managers.json 无法解析", name));
                checked += 1;
            }
            let ev = path.join("env_vars.json");
            if ev.exists() {
                let _: Vec<super::super::types::EnvVarDef> = read_json_file(&ev)
                    .unwrap_or_else(|| panic!("{} 的 env_vars.json 无法解析", name));
                checked += 1;
            }
            let rv = path.join("remote_versions_config.json");
            if rv.exists() {
                let _: serde_json::Value = read_json_file(&rv)
                    .unwrap_or_else(|| panic!("{} 的 remote_versions_config.json 无法解析", name));
                checked += 1;
            }
        }
        assert!(checked >= 60, "扫描到的拆分配置文件数量异常偏少: {}", checked);
    }

    /// 目录名必须等于 config.json 里的 id，且 id 全局唯一（id 是配置索引键）。
    #[test]
    fn project_ids_unique_and_match_dir_name() {
        let dir = repo_projects_dir();
        let mut seen: HashSet<String> = HashSet::new();
        for def in load_all() {
            assert!(seen.insert(def.id.clone()), "项目 id 重复: {}", def.id);
        }
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if !path.is_dir() || !path.join("config.json").exists() {
                continue;
            }
            let dir_name = path.file_name().unwrap().to_string_lossy().to_string();
            assert!(
                seen.contains(&dir_name),
                "目录 {} 未被解析为项目，或 id 与目录名不一致",
                dir_name
            );
        }
    }

    /// 目录类字段只能引用「共享目录根」或通用模板变量。
    /// 直接写死 `C:\Program Files` / `$HOME\AppData\Local` 之类的机器路径会导致
    /// 同特征目录在多处重复维护（改一处漏一处 / 系统盘与 Program Files 被重定向时失效）。
    #[test]
    fn dir_fields_only_use_shared_roots() {
        let allowed = allowed_placeholders();
        let mut violations: Vec<String> = Vec::new();

        let mut check = |owner: &str, field: &str, text: &str| {
            for name in unknown_placeholders(text, &allowed) {
                violations.push(format!("{} 的 {} 含未知占位符 {{{}}}: {}", owner, field, name, text));
            }
        };

        for def in load_all() {
            for (i, rule) in def.find_rules.iter().enumerate() {
                let owner = format!("{}.find_rules[{}]", def.id, i);
                match &rule.pattern {
                    ResolvePattern::PathContains { path_key, .. } => check(&owner, "path_key", path_key),
                    ResolvePattern::EnvBin { bin_sub, .. } => check(&owner, "bin_sub", bin_sub),
                    ResolvePattern::FixedPath { path, .. } => check(&owner, "path", path),
                }
            }
            for (i, candidate) in def.config_file_candidates.iter().enumerate() {
                check(&def.id, &format!("config_file_candidates[{}]", i), candidate);
            }
            for dir_def in &def.data_dirs {
                for p in &dir_def.possible_paths {
                    check(&def.id, &format!("data_dirs[{}].possible_paths", dir_def.id), p);
                }
                check(
                    &def.id,
                    &format!("data_dirs[{}].default_path", dir_def.id),
                    &dir_def.default_path,
                );
            }
            for pm in &def.package_managers {
                if let Some(p) = &pm.cache_default_path {
                    check(&def.id, &format!("{}.cache_default_path", pm.id), p);
                }
                if let Some(p) = &pm.data_default_path {
                    check(&def.id, &format!("{}.data_default_path", pm.id), p);
                }
                for extra in &pm.extra_caches {
                    if let Some(p) = &extra.default_path {
                        check(&def.id, &format!("{}.{}", pm.id, extra.id), p);
                    }
                }
            }
            for cm in &def.conflict_managers {
                if let Some(p) = &cm.cache_default_path {
                    check(&def.id, &format!("{}.cache_default_path", cm.id), p);
                }
            }
        }

        assert!(
            violations.is_empty(),
            "目录配置存在未登记的占位符（应改用 dirs::ROOTS 中的共享目录根）:\n{}",
            violations.join("\n")
        );
    }

    /// 服务项目必须声明数据目录，否则「数据文件」页签与启动前的目录检查都无从落地。
    #[test]
    fn services_declare_data_dirs() {
        for def in load_all() {
            if !def.is_service {
                continue;
            }
            assert!(
                !def.data_dirs.is_empty(),
                "服务 {} 未声明任何 data_dirs",
                def.id
            );
            let mut ids: HashSet<&str> = HashSet::new();
            for d in &def.data_dirs {
                assert!(ids.insert(d.id.as_str()), "{} 的 data_dirs id 重复: {}", def.id, d.id);
            }
        }
    }

    /// 「同一组配置」不变式：所有服务的日志目录必须用同一份默认路径与创建语义；
    /// 数据目录的创建语义只能取「不自动创建 + 启动前必须存在」或「自动创建」两种，
    /// 不允许出现 auto_create=true 同时 required_for_start=true 这种自相矛盾的组合。
    #[test]
    fn dir_groups_stay_consistent() {
        let mut log_default: Option<(String, String)> = None;
        for def in load_all() {
            for d in &def.data_dirs {
                match d.kind.as_deref() {
                    Some("log") => {
                        let key = (d.default_path.clone(), d.kind.clone().unwrap_or_default());
                        match &log_default {
                            None => log_default = Some(key),
                            Some(prev) => assert_eq!(
                                prev.0, key.0,
                                "服务 {} 的日志目录默认路径与同组不一致（应为 {:?}）",
                                def.id, prev.0
                            ),
                        }
                        assert!(
                            !(d.auto_create.unwrap_or(false) && d.required_for_start),
                            "{} 的日志目录 auto_create 与 required_for_start 同时为真",
                            def.id
                        );
                    }
                    Some("data") => {
                        assert!(
                            !(d.auto_create.unwrap_or(false) && d.required_for_start),
                            "{} 的数据目录 auto_create 与 required_for_start 同时为真（矛盾组合）",
                            def.id
                        );
                    }
                    _ => {}
                }
            }
        }
        assert_eq!(
            log_default.map(|(p, _)| p).as_deref(),
            Some("{install_root}\\logs"),
            "服务日志目录的共享默认路径应为 {{install_root}}\\logs"
        );
    }
}
