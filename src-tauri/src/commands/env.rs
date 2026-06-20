use std::fs;
use std::path::Path;
use serde::{Serialize, Deserialize};
use winreg::enums::*;
use winreg::RegKey;
use crate::commands::config::load_config;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct DiagnosticProblem {
    pub id: String,
    pub problem_type: String, // "dead_env_path" | "conflict_env" | "residue_files" | "c_drive_cache"
    pub description: String,
    pub detail: String,
    pub severity: String,     // "严重" | "警告" | "建议"
    pub fix_type: String,     // "remove_path" | "set_env" | "delete_folder" | "migrate_cache"
    pub fix_target: String,

    // ---- 检测依据（透明化：告诉用户"为什么"判定为问题）----
    /// 检测来源：读取了哪个配置文件 / 注册表项 / 环境变量
    pub evidence_source: String,
    /// 检测内容：在该来源里读到的实际值 / 关键字段
    pub evidence_content: String,
    /// 检测逻辑：依据什么规则判定为问题
    pub evidence_reason: String,

    // ---- 修复方案（透明化：告诉用户"将要怎么做、做什么"）----
    /// 修复计划的自然语言描述
    pub fix_plan: String,
    /// 将被修改的文件 / 注册表项（若有）
    pub fix_file: String,
    /// 迁移类操作的源路径（若有）
    pub fix_source_path: String,
    /// 迁移类操作的目标路径（若有）
    pub fix_dest_path: String,
}

#[cfg(windows)]
pub fn broadcast_setting_change() {
    use std::os::windows::ffi::OsStrExt;
    use std::ffi::OsStr;

    type SendMessageTimeoutW = unsafe extern "system" fn(
        hwnd: *mut std::ffi::c_void,
        msg: u32,
        wparam: usize,
        lparam: *const u16,
        flags: u32,
        timeout: u32,
        result: *mut usize,
    ) -> isize;

    unsafe {
        let module_name: Vec<u16> = OsStr::new("user32.dll").encode_wide().chain(std::iter::once(0)).collect();
        let handle = LoadLibraryW(module_name.as_ptr());
        if !handle.is_null() {
            let proc_name = std::ffi::CString::new("SendMessageTimeoutW").unwrap();
            let proc_addr = GetProcAddress(handle, proc_name.as_ptr() as *const u8);
            if !proc_addr.is_null() {
                let send_msg_timeout: SendMessageTimeoutW = std::mem::transmute(proc_addr);
                let env_str: Vec<u16> = OsStr::new("Environment").encode_wide().chain(std::iter::once(0)).collect();
                let mut result = 0;
                send_msg_timeout(
                    0xffff as *mut std::ffi::c_void, // HWND_BROADCAST
                    0x001a, // WM_SETTINGCHANGE
                    0,
                    env_str.as_ptr(),
                    0x0002, // SMTO_ABORTIFHUNG
                    5000,
                    &mut result,
                );
            }
            FreeLibrary(handle);
        }
    }
}

#[cfg(windows)]
extern "system" {
    fn LoadLibraryW(lpLibFileName: *const u16) -> *mut std::ffi::c_void;
    fn GetProcAddress(hModule: *mut std::ffi::c_void, lpProcName: *const u8) -> *mut std::ffi::c_void;
    fn FreeLibrary(hLibModule: *mut std::ffi::c_void) -> i32;
}

#[cfg(not(windows))]
pub fn broadcast_setting_change() {}

pub fn get_registry_env(name: &str) -> Option<String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(env_key) = hkcu.open_subkey("Environment") {
        if let Ok(val) = env_key.get_value::<String, _>(name) {
            return Some(val);
        }
    }
    None
}

/// 读取系统级(HKLM)环境变量
pub fn get_system_registry_env(name: &str) -> Option<String> {
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    if let Ok(env_key) = hklm.open_subkey("SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment") {
        if let Ok(val) = env_key.get_value::<String, _>(name) {
            return Some(val);
        }
    }
    None
}

/// 同时检查用户级和系统级注册表，返回值及其来源
pub fn get_registry_env_any(name: &str) -> Option<(String, &'static str)> {
    // 用户级优先
    if let Some(val) = get_registry_env(name) {
        if !val.is_empty() {
            return Some((val, "HKCU"));
        }
    }
    // 系统级
    if let Some(val) = get_system_registry_env(name) {
        if !val.is_empty() {
            return Some((val, "HKLM"));
        }
    }
    None
}

pub fn set_registry_env(name: &str, value: &str) -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (env_key, _) = hkcu.create_subkey("Environment").map_err(|e| e.to_string())?;
    if value.is_empty() {
        let _ = env_key.delete_value(name);
    } else {
        env_key.set_value(name, &value).map_err(|e| e.to_string())?;
    }
    broadcast_setting_change();
    Ok(())
}

fn md5_hash(input: &str) -> String {
    format!("{:x}", md5::compute(input.as_bytes()))
}

#[tauri::command]
pub fn scan_environment() -> Result<Vec<DiagnosticProblem>, String> {
    let config = load_config();
    let links_dir = Path::new(&config.links_dir).to_path_buf();
    let mut problems = Vec::new();

    // 1. Incorrect Environment Variables / Dead PATH entries
    // Retrieve PATH variable
    if let Some(user_path) = get_registry_env("PATH") {
        let parts = std::env::split_paths(&user_path).collect::<Vec<_>>();
        for p in parts {
            if p.as_os_str().is_empty() {
                continue;
            }
            let p_str = p.to_string_lossy().to_string();
            // Check if path exists. Exclude Any-Version links dir.
            if !p_str.to_lowercase().contains(&links_dir.to_string_lossy().to_lowercase()) {
                if !p.exists() {
                    problems.push(DiagnosticProblem {
                        id: md5_hash(&format!("dead_path:{}", p_str)),
                        problem_type: "dead_env_path".to_string(),
                        description: format!("PATH 环境变量中包含不存在的路径: {}", p_str),
                        detail: p_str.clone(),
                        severity: "严重".to_string(),
                        fix_type: "remove_path".to_string(),
                        fix_target: p_str.clone(),
                        evidence_source: "注册表 HKEY_CURRENT_USER\\Environment 中的 PATH 值".to_string(),
                        evidence_content: format!("PATH 中包含路径片段: {}", p_str),
                        evidence_reason: "该路径在磁盘上不存在（已被删除或移动），属于无效的死链 PATH 条目，会拖慢命令查找并可能引发错误。".to_string(),
                        fix_plan: format!("从用户 PATH 中删除这一条无效路径「{}」，其余路径保持不变。", p_str),
                        fix_file: "注册表 HKEY_CURRENT_USER\\Environment\\PATH".to_string(),
                        fix_source_path: String::new(),
                        fix_dest_path: String::new(),
                    });
                }
            }
        }
    }

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    //  检测类型 2 + 3：环境变量 + 外部 SDK（注册表驱动，支持 HKCU + HKLM）
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    use super::project::registry;

    let links_lower = links_dir.to_string_lossy().to_lowercase();
    let mut reported_paths: std::collections::HashSet<String> = std::collections::HashSet::new();

    // 遍历注册表中所有 SDK，检查其关联的环境变量
    for sdk_def in project::registry::registry() {
        let sdk_name = &sdk_def.display_name;

        for var_info in &sdk_def.env_vars {
            let var_name = &var_info.name;
            let desc = &var_info.desc;
            let check_type = &var_info.check_type;

            // 同时检查用户级(HKCU)和系统级(HKLM)
            if let Some((val, source)) = get_registry_env_any(var_name) {
                // 值指向 AnyVersion 链接目录：正常，跳过
                if val.to_lowercase().contains(&links_lower) {
                    continue;
                }

                match check_type.as_str() {
                    "path" => {
                        let val_path = Path::new(&val);
                        if !val_path.exists() {
                            // 路径不存在 → 无效环境变量
                            let reg_path = if source == "HKCU" {
                                format!("HKCU\\Environment\\{}", var_name)
                            } else {
                                format!("HKLM\\SYSTEM\\...\\Environment\\{}", var_name)
                            };
                            problems.push(DiagnosticProblem {
                                id: md5_hash(&format!("dead_var:{}:{}", source, var_name)),
                                problem_type: "dead_env_path".to_string(),
                                description: format!("[{}] {} = {} 路径不存在", sdk_name, var_name, val),
                                detail: format!("{}={}", var_name, val),
                                severity: "严重".to_string(),
                                fix_type: "set_env".to_string(),
                                fix_target: var_name.to_string(),
                                evidence_source: format!("注册表 {}", reg_path),
                                evidence_content: format!("{} = {}", var_name, val),
                                evidence_reason: format!("{}：{}。来源: {}。路径在磁盘上不存在。", sdk_name, desc, source),
                                fix_plan: format!("清空环境变量 {}。", var_name),
                                fix_file: reg_path,
                                fix_source_path: String::new(),
                                fix_dest_path: String::new(),
                            });
                        } else if !reported_paths.contains(&val.to_lowercase()) {
                            // 路径存在但不属于 AnyVersion 管理 → 未管理的 SDK
                            let is_managed = project::registry::find_by_id(&sdk_def.id).is_some()
                                && links_dir.join(&sdk_def.id).exists();

                            if !is_managed {
                                reported_paths.insert(val.to_lowercase());
                                let reg_path = if source == "HKCU" {
                                    format!("HKCU\\Environment\\{}", var_name)
                                } else {
                                    format!("HKLM\\SYSTEM\\...\\Environment\\{}", var_name)
                                };
                                problems.push(DiagnosticProblem {
                                    id: md5_hash(&format!("unmanaged_sdk_env:{}:{}:{}", source, sdk_name, var_name)),
                                    problem_type: "unmanaged_sdk".to_string(),
                                    description: format!("{}：{} 已设置（来源: {}），未被 AnyVersion 管理", sdk_name, var_name, source),
                                    detail: format!("{}={}", var_name, val),
                                    severity: "信息".to_string(),
                                    fix_type: "remove_path".to_string(),
                                    fix_target: var_name.to_string(),
                                    evidence_source: format!("注册表 {}", reg_path),
                                    evidence_content: format!("{} = {}", var_name, val),
                                    evidence_reason: format!("{}：{}。来源: {}。不在 AnyVersion 管理范围内。", sdk_name, desc, source),
                                    fix_plan: format!("如需管理 {}，可在 SDK 版本管理中安装；如已不再使用，可清空此变量。", sdk_name),
                                    fix_file: reg_path,
                                    fix_source_path: String::new(),
                                    fix_dest_path: String::new(),
                                });
                            }
                        }
                    }
                    _ => { /* nonempty 类型：仅记录 */ }
                }
            }
        }

        // PATH 中的外部 SDK 路径扫描（使用注册表的 find_rules）
        for rule in &sdk_def.find_rules {
            if let super::project::types::ResolvePattern::PathContains { path_key: pattern, exe_name: exe_hint } = &rule.pattern {
                let pattern = pattern.as_str();
                let exe_hint = exe_hint.as_str();
                // 扫描用户级 PATH
                let path_sources: Vec<(&str, Option<String>)> = vec![
                    ("HKCU", get_registry_env("PATH")),
                    ("HKLM", get_system_registry_env("PATH")),
                ];

                for (path_source, path_val) in path_sources {
                    let path_val = match path_val {
                        Some(v) => v,
                        None => continue,
                    };
                    let parts = std::env::split_paths(&path_val).collect::<Vec<_>>();

                    for p in &parts {
                        if p.as_os_str().is_empty() {
                            continue;
                        }
                        let p_str = p.to_string_lossy().to_string();
                        let p_lower = p_str.to_lowercase();

                        // 跳过 AnyVersion 管理的目录
                        if p_lower.contains(&links_lower) {
                            continue;
                        }
                        // 去重：如果已经报过了，就跳过
                        if reported_paths.contains(&p_lower) {
                            continue;
                        }

                        if !p_lower.contains(pattern) {
                            continue;
                        }

                        // 模式匹配，检查目录是否存在
                        let has_exe = p.join(exe_hint).exists()
                            || p.join("bin").join(exe_hint).exists();
                        let dir_exists = p.exists();

                        if has_exe || dir_exists {
                            reported_paths.insert(p_lower.clone());

                            let is_managed = links_dir.join(&sdk_def.id).exists();
                            let (severity, fix_desc) = if is_managed {
                                ("警告".to_string(),
                                 format!("已由 AnyVersion 管理 {}，此外部路径可能造成版本冲突。", sdk_name))
                            } else {
                                ("信息".to_string(),
                                 format!("未被 AnyVersion 管理。可在 SDK 版本管理中安装 {}。", sdk_name))
                            };

                            problems.push(DiagnosticProblem {
                                id: md5_hash(&format!("external_sdk:{}:{}:{}", path_source, sdk_name, p_str)),
                                problem_type: "unmanaged_sdk".to_string(),
                                description: format!("PATH（{}）中存在 {} 路径", path_source, sdk_name),
                                detail: p_str.clone(),
                                severity,
                                fix_type: "remove_path".to_string(),
                                fix_target: p_str.clone(),
                                evidence_source: format!("注册表 {}\\Environment\\PATH", path_source),
                                evidence_content: format!("PATH 包含: {}", p_str),
                                evidence_reason: format!("匹配模式「{}」，{} 存在。", pattern, exe_hint),
                                fix_plan: format!("{}从 PATH 中移除此条目：{}", fix_desc, p_str),
                                fix_file: format!("注册表 {}\\Environment\\PATH", path_source),
                                fix_source_path: String::new(),
                                fix_dest_path: String::new(),
                            });
                            break;
                        }
                    }
                }
            }
        }
    }

    // 3. Leftover folders of databases/services
    let database_residues = vec![
        ("MySQL", vec!["C:\\ProgramData\\MySQL", "C:\\Program Files\\MySQL"]),
        ("MongoDB", vec!["C:\\data\\db"]),
        ("PostgreSQL", vec!["C:\\Program Files\\PostgreSQL"]),
    ];

    for (db_name, folders) in database_residues {
        for folder in folders {
            let path = Path::new(folder);
            if path.exists() {
                // If the folder exists, check if there is MySQL / Mongo / Postgres in PATH
                // Or if it's not managed. Since it's a residue, we prompt to safe delete.
                problems.push(DiagnosticProblem {
                    id: md5_hash(&format!("residue:{}", folder)),
                    problem_type: "residue_files".to_string(),
                    description: format!("检测到残留的 {} 数据库数据目录 (无相应服务运行)", db_name),
                    detail: folder.to_string(),
                    severity: "建议".to_string(),
                    fix_type: "delete_folder".to_string(),
                    fix_target: folder.to_string(),
                    evidence_source: format!("文件系统扫描固定路径: {}", folder),
                    evidence_content: format!("目录「{}」存在于磁盘上。", folder),
                    evidence_reason: format!("这是 {} 常见的默认数据/安装目录，但当前并未检测到对应服务在运行，可能是卸载后残留，会占用磁盘空间。", db_name),
                    fix_plan: format!("将残留目录「{}」移动到系统回收站（不会永久删除，可随时还原），以释放磁盘空间。", folder),
                    fix_file: String::new(),
                    fix_source_path: folder.to_string(),
                    fix_dest_path: "系统回收站 (Recycle Bin)".to_string(),
                });
            }
        }
    }

    // 4. Package manager caches located on the C-drive
    let cache_paths = vec![
        ("npm", crate::commands::cache::get_npm_cache_path()),
        ("yarn", crate::commands::cache::get_yarn_cache_path()),
        ("pnpm", crate::commands::cache::get_pnpm_cache_path()),
        ("pip", crate::commands::cache::get_pip_cache_path()),
        ("mvn", crate::commands::cache::get_maven_cache_path()),
        ("nuget", crate::commands::cache::get_nuget_cache_path()),
    ];

    // 预先计算迁移目标盘符，用于在"修复方案"里向用户透明展示目标路径
    let target_drive = pick_non_c_drive();

    for (name, path) in cache_paths {
        if path.exists() {
            let path_str = path.to_string_lossy().to_string();
            if path_str.starts_with("C:") || path_str.starts_with("c:") {
                // Check if it's already a link / symlink / directory junction to another drive
                let is_symlink = fs::symlink_metadata(&path).map(|m| m.file_type().is_symlink()).unwrap_or(false);
                let is_redirected = if let Ok(canonical) = fs::canonicalize(&path) {
                    let canonical_lower = canonical.to_string_lossy().to_lowercase();
                    !canonical_lower.starts_with(r"\\?\c:") && !canonical_lower.starts_with("c:")
                } else {
                    false
                };

                if !is_symlink && !is_redirected {
                    let dest = format!("{}any-version-caches\\{}", target_drive, name);
                    let (cfg_source, cfg_content) = cache_detection_evidence(name, &path_str);
                    problems.push(DiagnosticProblem {
                        id: md5_hash(&format!("c_drive_cache:{}", name)),
                        problem_type: "c_drive_cache".to_string(),
                        description: format!("{} 全局包缓存存储在 C 盘，占用 C 盘空间", name.to_uppercase()),
                        detail: path_str.clone(),
                        severity: "建议".to_string(),
                        fix_type: "migrate_cache".to_string(),
                        fix_target: name.to_string(),
                        evidence_source: cfg_source,
                        evidence_content: cfg_content,
                        evidence_reason: format!("{} 的全局缓存目录「{}」位于系统盘 C 盘，且尚未做重定向，长期使用会持续占用宝贵的 C 盘空间。", name.to_uppercase(), path_str),
                        fix_plan: format!("把缓存目录从「{}」整体迁移到「{}」，并在原位置创建一个 NTFS 目录联接（Junction）。这样所有工具仍按原路径访问，但实际文件存放在非 C 盘，使用上完全无感。", path_str, dest),
                        fix_file: "NTFS 目录联接 (mklink /J)".to_string(),
                        fix_source_path: path_str,
                        fix_dest_path: dest,
                    });
                }
            }
        }
    }

    Ok(problems)
}

/// 选择一个非 C 盘的可用盘符作为缓存迁移目标（与 resolve_problems 中逻辑保持一致）。
fn pick_non_c_drive() -> String {
    for drive in b'D'..=b'Z' {
        let drive_path = format!("{}:\\", drive as char);
        if Path::new(&drive_path).exists() {
            return drive_path;
        }
    }
    "D:\\".to_string()
}

/// 返回某个缓存路径是"通过哪个配置文件/命令"检测到的，用于向用户透明展示检测依据。
fn cache_detection_evidence(name: &str, resolved: &str) -> (String, String) {
    let app_data = std::env::var("APPDATA").unwrap_or_default();
    let user_profile = std::env::var("USERPROFILE").unwrap_or_default();
    match name {
        "npm" => (
            "命令 `npm config get cache` 的输出".to_string(),
            format!("npm 报告的缓存目录为: {}", resolved),
        ),
        "yarn" => (
            "命令 `yarn cache dir` 的输出".to_string(),
            format!("yarn 报告的缓存目录为: {}", resolved),
        ),
        "pnpm" => (
            "命令 `pnpm store path` 的输出".to_string(),
            format!("pnpm 报告的存储目录为: {}", resolved),
        ),
        "pip" => (
            format!("环境变量 PIP_CACHE_DIR，或配置文件 {}\\pip\\pip.ini 中的 cache-dir 项", app_data),
            format!("解析得到的 pip 缓存目录为: {}", resolved),
        ),
        "mvn" => (
            format!("配置文件 {}\\.m2\\settings.xml 中的 <localRepository> 节点（或全局 settings.xml）", user_profile),
            format!("解析得到的 Maven 本地仓库为: {}", resolved),
        ),
        "nuget" => (
            "环境变量 NUGET_PACKAGES（未设置时回退到 %USERPROFILE%\\.nuget\\packages）".to_string(),
            format!("解析得到的 NuGet 全局包目录为: {}", resolved),
        ),
        _ => (
            "包管理器默认缓存路径".to_string(),
            format!("检测到的缓存目录为: {}", resolved),
        ),
    }
}

fn get_sdk_bin_paths(sdk_id: &str, link_dir: &str) -> Vec<String> {
    match sdk_id {
        "go" | "java" | "flutter" | "maven" | "gradle" | "harmony" | "cuda" | "ffmpeg" => {
            vec![format!("{}\\bin", link_dir)]
        }
        "python" => {
            vec![link_dir.to_string(), format!("{}\\Scripts", link_dir)]
        }
        "rust" => {
            vec![format!("{}\\.cargo\\bin", link_dir)]
        }
        "android" => {
            vec![
                format!("{}\\cmdline-tools\\latest\\bin", link_dir),
                format!("{}\\platform-tools", link_dir),
            ]
        }
        "nodejs" | "bun" | "yarn" | "pnpm" | "nginx" | "redis" => {
            vec![link_dir.to_string()]
        }
        "mysql" | "mongodb" | "postgresql" => {
            vec![format!("{}\\bin", link_dir)]
        }
        _ => vec![],
    }
}

pub fn add_to_user_path(paths: &[String]) -> Result<(), String> {
    if let Some(user_path) = get_registry_env("PATH") {
        let mut parts = std::env::split_paths(&user_path)
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        
        let mut modified = false;
        for path in paths {
            let path_lower = path.to_lowercase();
            if !parts.iter().any(|p| p.to_lowercase() == path_lower) {
                parts.push(path.clone());
                modified = true;
            }
        }
        
        if modified {
            let new_path = std::env::join_paths(parts.iter().map(Path::new))
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .to_string();
            set_registry_env("PATH", &new_path)?;
        }
    } else {
        let new_path = std::env::join_paths(paths.iter().map(Path::new))
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .to_string();
        set_registry_env("PATH", &new_path)?;
    }
    Ok(())
}

pub fn remove_from_user_path(paths: &[String]) -> Result<(), String> {
    if let Some(user_path) = get_registry_env("PATH") {
        let parts = std::env::split_paths(&user_path)
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        
        let initial_len = parts.len();
        let new_parts = parts.into_iter()
            .filter(|p| {
                let p_lower = p.to_lowercase();
                !paths.iter().any(|target| target.to_lowercase() == p_lower)
            })
            .collect::<Vec<_>>();
        
        if new_parts.len() != initial_len {
            let new_path = std::env::join_paths(new_parts.iter().map(Path::new))
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .to_string();
            set_registry_env("PATH", &new_path)?;
        }
    }
    Ok(())
}

/// 自动配置 SDK 相关环境变量（注册表驱动）。
/// 新增 SDK 时只需在 projects.json 中定义 env_vars，此函数自动生效。
///
/// 设计原则：
///   - 所有 *_HOME 类变量指向 link_dir（版本切换只需重定向 junction）
///   - CARGO_HOME / RUSTUP_HOME 指向 link_dir 下的子目录
///   - ANDROID_SDK_HOME 指向 link_dir 下的 .android 子目录
pub fn configure_sdk_env_vars(sdk_id: &str, link_dir: &str, _version_dir: &str) -> Result<(), String> {
    use super::project::registry;

    let config = load_config();
    if !config.managed_items.contains(sdk_id) {
        return Ok(());
    }

    let sdk_def = match project::registry::find_by_id(sdk_id) {
        Some(d) => d,
        None => return Ok(()),
    };

    for var_info in &sdk_def.env_vars {
        let var_name = &var_info.name;
        // 对不同变量使用不同的值策略
        let value = match var_name.as_str() {
            // 特殊子目录映射
            "CARGO_HOME"      => format!("{}\\.cargo", link_dir),
            "RUSTUP_HOME"     => format!("{}\\.rustup", link_dir),
            "ANDROID_SDK_HOME" => format!("{}\\.android", link_dir),
            "NPM_CONFIG_PREFIX" => format!("{}\\node_modules", link_dir),
            "PGDATA"          => format!("{}\\data", link_dir),
            // 其他变量统一指向 link_dir
            _                 => link_dir.to_string(),
        };
        let _ = set_registry_env(var_name, &value);
    }

    // 自动将可执行目录添加到用户 PATH 变量中
    let bin_paths = get_sdk_bin_paths(sdk_id, link_dir);
    let _ = add_to_user_path(&bin_paths);

    Ok(())
}

/// 移除 SDK 相关的环境变量（注册表驱动）。
/// 当卸载某 SDK 最后一个版本时调用。
pub fn remove_sdk_env_vars(sdk_id: &str) -> Result<(), String> {
    use super::project::registry;

    let sdk_def = match project::registry::find_by_id(sdk_id) {
        Some(d) => d,
        None => return Ok(()),
    };

    for var_info in &sdk_def.env_vars {
        let _ = set_registry_env(&var_info.name, "");
    }

    // 从用户 PATH 中移除该 SDK 的可执行目录
    let config = load_config();
    let junction_path = Path::new(&config.links_dir).join(sdk_id);
    let link_str = junction_path.to_string_lossy().to_string();
    let bin_paths = get_sdk_bin_paths(sdk_id, &link_str);
    let _ = remove_from_user_path(&bin_paths);

    Ok(())
}

#[tauri::command]
pub fn resolve_problems(problems: Vec<DiagnosticProblem>) -> Result<(), String> {
    for p in problems {
        match p.fix_type.as_str() {
            "remove_path" => {
                if let Some(user_path) = get_registry_env("PATH") {
                    let parts = std::env::split_paths(&user_path).collect::<Vec<_>>();
                    let new_parts = parts.into_iter()
                        .filter(|x| x.to_string_lossy().to_string() != p.fix_target)
                        .collect::<Vec<_>>();
                    let new_path = std::env::join_paths(new_parts)
                        .map_err(|e| e.to_string())?
                        .to_string_lossy()
                        .to_string();
                    set_registry_env("PATH", &new_path)?;
                }
            }
            "set_env" => {
                // Reset or remove the env var in registry
                set_registry_env(&p.fix_target, "")?;
            }
            "delete_folder" => {
                // Move folder to Recycle Bin using the trash crate
                let target_path = Path::new(&p.fix_target);
                if target_path.exists() {
                    trash::delete(target_path).map_err(|e| format!("移至回收站失败: {}", e))?;
                }
            }
            "migrate_cache" => {
                // Migrate cache: redirect to a non-C drive (consistent with scan_environment's plan)
                let target_drive = pick_non_c_drive();
                let cache_name = p.fix_target.clone();
                let target_cache_dir = format!("{}any-version-caches\\{}", target_drive, cache_name);
                super::cache::migrate_cache_path(cache_name, target_cache_dir)?;
            }
            _ => return Err(format!("不支持的修复方式: {}", p.fix_type)),
        }
    }
    Ok(())
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EnvBackup {
    pub id: String,
    pub timestamp: String,
    pub description: String,
    pub user_vars: std::collections::HashMap<String, String>,
    pub sys_vars: std::collections::HashMap<String, String>,
}

#[tauri::command]
pub fn create_env_backup(description: String) -> Result<EnvBackup, String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let mut user_vars = std::collections::HashMap::new();
    if let Ok(env_key) = hkcu.open_subkey("Environment") {
        for name in env_key.enum_values().filter_map(|x| x.ok()).map(|(n, _)| n) {
            if let Ok(val) = env_key.get_value::<String, _>(&name) {
                user_vars.insert(name, val);
            }
        }
    }

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let mut sys_vars = std::collections::HashMap::new();
    if let Ok(env_key) = hklm.open_subkey("SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment") {
        for name in env_key.enum_values().filter_map(|x| x.ok()).map(|(n, _)| n) {
            if let Ok(val) = env_key.get_value::<String, _>(&name) {
                sys_vars.insert(name, val);
            }
        }
    }

    let id = format!("{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0));

    // Get simple local timestamp using powershell date
    let timestamp = std::process::Command::new("powershell")
        .args(&["-Command", "Get-Date -Format 'yyyy-MM-dd HH:mm:ss'"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "Unknown Time".to_string());

    let backup = EnvBackup {
        id,
        timestamp,
        description,
        user_vars,
        sys_vars,
    };

    let base_dir = crate::commands::config::get_base_dir();
    let backups_dir = base_dir.join("backups");
    fs::create_dir_all(&backups_dir).map_err(|e| e.to_string())?;

    let backup_file = backups_dir.join(format!("env_backup_{}.json", backup.id));
    let data = serde_json::to_string_pretty(&backup).map_err(|e| e.to_string())?;
    fs::write(backup_file, data).map_err(|e| e.to_string())?;

    Ok(backup)
}

#[tauri::command]
pub fn list_env_backups() -> Result<Vec<EnvBackup>, String> {
    let base_dir = crate::commands::config::get_base_dir();
    let backups_dir = base_dir.join("backups");
    if !backups_dir.exists() {
        return Ok(Vec::new());
    }

    let mut list = Vec::new();
    if let Ok(entries) = fs::read_dir(backups_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            if entry.path().extension().map(|s| s == "json").unwrap_or(false) {
                if let Ok(content) = fs::read_to_string(entry.path()) {
                    if let Ok(backup) = serde_json::from_str::<EnvBackup>(&content) {
                        list.push(backup);
                    }
                }
            }
        }
    }

    // Sort backups by timestamp descending
    list.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(list)
}

#[tauri::command]
pub fn delete_env_backup(id: String) -> Result<(), String> {
    let base_dir = crate::commands::config::get_base_dir();
    let backup_file = base_dir.join("backups").join(format!("env_backup_{}.json", id));
    if backup_file.exists() {
        fs::remove_file(backup_file).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn restore_env_backup(id: String) -> Result<(), String> {
    let base_dir = crate::commands::config::get_base_dir();
    let backup_file = base_dir.join("backups").join(format!("env_backup_{}.json", id));
    if !backup_file.exists() {
        return Err("备份文件不存在".to_string());
    }

    let content = fs::read_to_string(backup_file).map_err(|e| e.to_string())?;
    let backup = serde_json::from_str::<EnvBackup>(&content).map_err(|e| e.to_string())?;

    // 1. Restore User Variables
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (user_key, _) = hkcu.create_subkey("Environment").map_err(|e| e.to_string())?;

    // Delete keys not in backup
    let existing_user_keys: Vec<String> = user_key.enum_values().filter_map(|x| x.ok()).map(|(n, _)| n).collect();
    for name in existing_user_keys {
        if !backup.user_vars.contains_key(&name) {
            let _ = user_key.delete_value(&name);
        }
    }

    // Restore keys from backup
    for (name, val) in &backup.user_vars {
        user_key.set_value(name, val).map_err(|e| e.to_string())?;
    }

    // 2. Restore System Variables (try, but don't fail if we lack permissions)
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let mut system_restore_msg = String::new();
    match hklm.open_subkey_with_flags("SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment", KEY_ALL_ACCESS) {
        Ok(sys_key) => {
            let existing_sys_keys: Vec<String> = sys_key.enum_values().filter_map(|x| x.ok()).map(|(n, _)| n).collect();
            for name in existing_sys_keys {
                if !backup.sys_vars.contains_key(&name) {
                    let _ = sys_key.delete_value(&name);
                }
            }
            for (name, val) in &backup.sys_vars {
                let _ = sys_key.set_value(name, val);
            }
        }
        Err(_) => {
            system_restore_msg = "\n注意：系统级环境变量恢复失败（权限不足，请以管理员身份运行此程序进行完整恢复）。用户级环境变量已成功恢复！".to_string();
        }
    }

    broadcast_setting_change();

    if !system_restore_msg.is_empty() {
        return Err(system_restore_msg);
    }

    Ok(())
}

#[tauri::command]
pub fn toggle_item_management(id: String, enable: bool, cache_dest: Option<String>) -> Result<(), String> {
    use super::project::registry;
    let sdk_def = project::registry::find_by_id(&id)
        .ok_or_else(|| format!("未找到该标识符对应的配置: {}", id))?;

    let mut config = load_config();

    if enable {
        // 1. Add to managed_items
        config.managed_items.insert(id.clone());

        // 2. Backup conflicting environment variables
        for var_info in &sdk_def.env_vars {
            let var_name = &var_info.name;
            if let Some((val, _source)) = get_registry_env_any(var_name) {
                if !val.to_lowercase().contains(&config.links_dir.to_lowercase()) {
                    config.original_envs.entry(var_name.to_string()).or_insert(val);
                }
            }
        }

        // 3. Backup conflicting PATH entries from HKCU
        let mut original_paths_to_save = Vec::new();
        if let Some(user_path) = get_registry_env("PATH") {
            let parts = std::env::split_paths(&user_path)
                .map(|p| p.to_string_lossy().to_string())
                .collect::<Vec<_>>();
            
            let mut matched_entries = Vec::new();
            let mut remaining_entries = Vec::new();

            for p_str in parts {
                if p_str.is_empty() {
                    continue;
                }
                
                let mut matches = false;
                let p_lower = p_str.to_lowercase();
                
                if !p_lower.contains(&config.links_dir.to_lowercase()) {
                    for rule in &sdk_def.find_rules {
                        match &rule.pattern {
                            super::project::types::ResolvePattern::PathContains { path_key: pattern, .. } => {
                                if p_lower.contains(&pattern.to_lowercase()) {
                                    matches = true;
                                    break;
                                }
                            }
                            super::project::types::ResolvePattern::FixedPath { path: fixed_path, .. } => {
                                if p_lower.contains(&fixed_path.to_lowercase()) {
                                    matches = true;
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                }

                if matches {
                    matched_entries.push(p_str.clone());
                    original_paths_to_save.push(p_str);
                } else {
                    remaining_entries.push(p_str);
                }
            }

            if !matched_entries.is_empty() {
                config.original_paths.entry(id.clone()).or_insert_with(Vec::new).extend(matched_entries);
                
                let new_path = std::env::join_paths(remaining_entries.iter().map(Path::new))
                    .map_err(|e| e.to_string())?
                    .to_string_lossy()
                    .to_string();
                set_registry_env("PATH", &new_path)?;
            }
        }

        // Save config changes so far
        crate::commands::config::save_config(&config)?;

        // 4. Configure SDK env vars if a version is active
        let junction_path = Path::new(&config.links_dir).join(&id);
        if junction_path.exists() {
            let link_str = junction_path.to_string_lossy().to_string();
            let dest_str = fs::canonicalize(&junction_path)
                .map(|p| p.to_string_lossy().to_string().trim_start_matches(r"\\?\").to_string())
                .unwrap_or_default();
            let _ = configure_sdk_env_vars(&id, &link_str, &dest_str);
        }

        // 5. Migrate cache if requested
        if let Some(dest) = cache_dest {
            if !dest.is_empty() {
                let _ = crate::commands::cache::migrate_cache_path(id.clone(), dest);
            }
        }
    } else {
        // Disable management
        config.managed_items.remove(&id);

        // 1. Remove AnyVersion environment variables for this SDK
        let _ = remove_sdk_env_vars(&id);

        // 2. Restore original environment variables from backup
        for var_info in &sdk_def.env_vars {
            let var_name = &var_info.name;
            if let Some(orig_val) = config.original_envs.remove(var_name) {
                let _ = set_registry_env(var_name, &orig_val);
            }
        }

        // 3. Restore original PATH entries
        if let Some(orig_paths) = config.original_paths.remove(&id) {
            let _ = add_to_user_path(&orig_paths);
        }

        // Save updated config
        crate::commands::config::save_config(&config)?;
    }

    Ok(())
}

