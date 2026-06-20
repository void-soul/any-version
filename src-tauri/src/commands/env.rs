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
                        fix_file: "注册表: HKEY_CURRENT_USER\\Environment\\PATH".to_string(),
                        fix_source_path: String::new(),
                        fix_dest_path: String::new(),
                    });
                }
            }
        }
    }

    // Check specific environment variables
    let env_vars_to_check = vec![
        ("GOROOT", "Go 安装根目录"),
        ("JAVA_HOME", "Java 安装根目录"),
        ("ANDROID_HOME", "Android SDK 安装目录"),
        ("ANDROID_SDK_ROOT", "Android SDK 根目录"),
        ("MAVEN_HOME", "Maven 安装目录"),
        ("GRADLE_HOME", "Gradle 安装目录"),
        ("NODE_PATH", "Node.js 全局模块路径"),
        ("CARGO_HOME", "Cargo 包管理器目录"),
        ("RUSTUP_HOME", "Rustup 工具链目录"),
    ];

    for (var_name, desc) in env_vars_to_check {
        if let Some(val) = get_registry_env(var_name) {
            if val.is_empty() {
                continue;
            }
            let val_path = Path::new(&val);
            if !val.to_lowercase().contains(&links_dir.to_string_lossy().to_lowercase()) {
                if !val_path.exists() {
                    problems.push(DiagnosticProblem {
                        id: md5_hash(&format!("dead_var:{}", var_name)),
                        problem_type: "dead_env_path".to_string(),
                        description: format!("环境变量 {} ({}) 指向不存在的目录", var_name, desc),
                        detail: format!("{}={}", var_name, val),
                        severity: "严重".to_string(),
                        fix_type: "set_env".to_string(),
                        fix_target: var_name.to_string(),
                        evidence_source: format!("注册表 HKEY_CURRENT_USER\\Environment 中的 {} 值", var_name),
                        evidence_content: format!("{} = {}", var_name, val),
                        evidence_reason: format!("环境变量 {} 指向的目录「{}」在磁盘上不存在，说明对应软件已被卸载或移动，该变量已失效。", var_name, val),
                        fix_plan: format!("清空（删除）失效的环境变量 {}，避免相关工具读取到错误路径。", var_name),
                        fix_file: format!("注册表: HKEY_CURRENT_USER\\Environment\\{}", var_name),
                        fix_source_path: String::new(),
                        fix_dest_path: String::new(),
                    });
                }
            }
        }
    }

    // 2. External conflict development environment variables (non-managed by Any-Version)
    let conflict_exes = vec![
        ("go.exe", "go"),
        ("node.exe", "nodejs"),
        ("python.exe", "python"),
        ("flutter.bat", "flutter"),
        ("rustc.exe", "rust"),
        ("java.exe", "java"),
    ];

    if let Some(user_path) = get_registry_env("PATH") {
        let parts = std::env::split_paths(&user_path).collect::<Vec<_>>();
        for (i, p) in parts.iter().enumerate() {
            let p_str = p.to_string_lossy().to_string();
            if p_str.to_lowercase().contains(&links_dir.to_string_lossy().to_lowercase()) {
                continue;
            }
            for (exe, sdk_name) in &conflict_exes {
                let full_exe = p.join(exe);
                if full_exe.exists() {
                    // Check if Any-Version's link path precedes it
                    let av_link_path = links_dir.join(sdk_name);
                    let av_precedes = parts.iter().take(i).any(|x| {
                        x.to_string_lossy().to_lowercase().contains(&av_link_path.to_string_lossy().to_lowercase())
                    });
                    if !av_precedes {
                        problems.push(DiagnosticProblem {
                            id: md5_hash(&format!("conflict:{}:{}", sdk_name, p_str)),
                            problem_type: "conflict_env".to_string(),
                            description: format!("检测到外部优先的 {} 环境，可能导致 Any-Version 切换不生效", sdk_name),
                            detail: format!("外部路径: {}", p_str),
                            severity: "警告".to_string(),
                            fix_type: "remove_path".to_string(),
                            fix_target: p_str.clone(),
                            evidence_source: "注册表 HKEY_CURRENT_USER\\Environment 中的 PATH 值".to_string(),
                            evidence_content: format!("在 PATH 中发现 {}（位于「{}」），且其顺序排在 Any-Version 链接目录「{}」之前。", exe, p_str, av_link_path.to_string_lossy()),
                            evidence_reason: format!("Windows 按 PATH 顺序查找可执行文件。由于该外部 {} 排在 Any-Version 之前，您在 Any-Version 里切换的 {} 版本不会生效。", exe, sdk_name),
                            fix_plan: format!("将该外部路径「{}」从用户 PATH 中移除，使 Any-Version 管理的 {} 版本成为唯一生效来源。", p_str, sdk_name),
                            fix_file: "注册表: HKEY_CURRENT_USER\\Environment\\PATH".to_string(),
                            fix_source_path: String::new(),
                            fix_dest_path: String::new(),
                        });
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
