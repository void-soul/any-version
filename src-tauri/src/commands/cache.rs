use std::fs;
use std::path::{Path, PathBuf};
use serde::{Serialize, Deserialize};
use walkdir::WalkDir;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CacheInfo {
    pub name: String,
    pub installed: bool,
    pub path: String,
    pub size: String,
    pub is_link: bool,
    pub real_target: String,
    /// 检测依据：该缓存路径是通过哪个配置文件 / 命令得到的
    pub detect_source: String,
    /// 检测依据：读到的实际内容
    pub detect_content: String,
}

pub fn is_installed(cli: &str) -> bool {
    std::process::Command::new("cmd")
        .args(&["/c", "where", cli])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn get_dir_size(path: &Path) -> u64 {
    let mut total_size = 0;
    for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
        if let Ok(metadata) = entry.metadata() {
            if metadata.is_file() {
                total_size += metadata.len();
            }
        }
    }
    total_size
}

pub fn format_bytes(bytes: u64) -> String {
    const UNIT: u64 = 1024;
    if bytes < UNIT {
        return format!("{} B", bytes);
    }
    let mut div = UNIT;
    let mut exp = 0;
    let mut n = bytes / UNIT;
    while n >= UNIT {
        div *= UNIT;
        exp += 1;
        n /= UNIT;
    }
    let suffix = match exp {
        0 => "KiB",
        1 => "MiB",
        2 => "GiB",
        3 => "TiB",
        _ => "PiB",
    };
    format!("{:.2} {}", (bytes as f64) / (div as f64), suffix)
}

pub fn create_junction(link_path: &Path, target_path: &Path) -> Result<(), String> {
    if link_path.exists() || link_path.is_symlink() {
        // Junctions are directory reparse points on Windows.
        // fs::remove_dir removes the junction itself without deleting target contents.
        // fs::remove_file would fail with Access Denied (os error 5) on a junction.
        let _ = fs::remove_dir(link_path);
        // Fallback: if remove_dir failed (e.g. it's a real dir, not a junction)
        if link_path.exists() {
            fs::remove_dir_all(link_path).map_err(|e| format!("删除旧链接失败: {}", e))?;
        }
    }
    if let Some(parent) = link_path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::create_dir_all(target_path).map_err(|e| e.to_string())?;
    let output = std::process::Command::new("cmd")
        .args(&[
            "/c",
            "mklink",
            "/J",
            &link_path.to_string_lossy(),
            &target_path.to_string_lossy(),
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }
    Ok(())
}

fn get_cmd_output(cmd: &str, args: &[&str]) -> String {
    std::process::Command::new(cmd)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

pub fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> std::io::Result<()> {
    fs::create_dir_all(&dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(entry.path(), dst.as_ref().join(entry.file_name()))?;
        } else {
            fs::copy(entry.path(), dst.as_ref().join(entry.file_name()))?;
        }
    }
    Ok(())
}

pub fn get_npm_cache_path() -> PathBuf {
    let local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let path_str = get_cmd_output("cmd", &["/c", "npm", "config", "get", "cache"]);
    if path_str.is_empty() {
        PathBuf::from(&local_app_data).join("npm-cache")
    } else {
        PathBuf::from(path_str)
    }
}

pub fn get_yarn_cache_path() -> PathBuf {
    let local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let path_str = get_cmd_output("cmd", &["/c", "yarn", "cache", "dir"]);
    if path_str.is_empty() {
        PathBuf::from(&local_app_data).join("Yarn").join("Cache")
    } else {
        PathBuf::from(path_str)
    }
}

pub fn get_pnpm_cache_path() -> PathBuf {
    let local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let path_str = get_cmd_output("cmd", &["/c", "pnpm", "store", "path"]);
    if path_str.is_empty() {
        PathBuf::from(&local_app_data).join("pnpm").join("store")
    } else {
        PathBuf::from(path_str)
    }
}

pub fn get_pip_cache_path() -> PathBuf {
    let local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let app_data = std::env::var("APPDATA").unwrap_or_default();
    let mut pip_path = std::env::var("PIP_CACHE_DIR").unwrap_or_default();
    if pip_path.is_empty() {
        let pip_ini = PathBuf::from(&app_data).join("pip").join("pip.ini");
        if pip_ini.exists() {
            if let Ok(data) = fs::read_to_string(pip_ini) {
                for line in data.lines() {
                    let line_trimmed = line.trim();
                    if line_trimmed.to_lowercase().starts_with("cache-dir") {
                        let parts = line_trimmed.splitn(2, '=').collect::<Vec<_>>();
                        if parts.len() == 2 {
                            pip_path = parts[1].trim().to_string();
                            break;
                        }
                    }
                }
            }
        }
    }
    if pip_path.is_empty() {
        PathBuf::from(&local_app_data).join("pip").join("Cache")
    } else {
        PathBuf::from(pip_path)
    }
}

pub fn get_maven_cache_path() -> PathBuf {
    let user_profile = std::env::var("USERPROFILE").unwrap_or_default();
    let mut mvn_path = String::new();

    // 1. User settings.xml
    let m2_settings = PathBuf::from(&user_profile).join(".m2").join("settings.xml");
    if m2_settings.exists() {
        if let Ok(content) = fs::read_to_string(&m2_settings) {
            if let Some(start) = content.find("<localRepository>") {
                if let Some(end) = content.find("</localRepository>") {
                    if end > start + 17 {
                        let potential_path = content[start + 17..end].trim().to_string();
                        let is_commented = match content[..start].rfind("<!--") {
                            Some(comment_start) => {
                                match content[comment_start..start].find("-->") {
                                    None => true,
                                    Some(_) => false,
                                }
                            }
                            None => false,
                        };
                        if !is_commented {
                            mvn_path = potential_path;
                        }
                    }
                }
            }
        }
    }

    // 2. Global settings.xml from MAVEN_HOME or M2_HOME
    if mvn_path.is_empty() {
        for env_var in &["MAVEN_HOME", "M2_HOME"] {
            if let Ok(val) = std::env::var(env_var) {
                let global_settings = Path::new(&val).join("conf").join("settings.xml");
                if global_settings.exists() {
                    if let Ok(content) = fs::read_to_string(&global_settings) {
                        if let Some(start) = content.find("<localRepository>") {
                            if let Some(end) = content.find("</localRepository>") {
                                if end > start + 17 {
                                    let potential_path = content[start + 17..end].trim().to_string();
                                    let is_commented = match content[..start].rfind("<!--") {
                                        Some(comment_start) => {
                                            match content[comment_start..start].find("-->") {
                                                None => true,
                                                Some(_) => false,
                                            }
                                        }
                                        None => false,
                                    };
                                    if !is_commented {
                                        mvn_path = potential_path;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // 3. Global settings.xml from Maven resolved via path
    if mvn_path.is_empty() {
        if let Ok(output) = std::process::Command::new("cmd")
            .args(&["/c", "where", "mvn"])
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if let Some(first_line) = stdout.lines().next() {
                    let mvn_bin = Path::new(first_line.trim());
                    if let Some(maven_home) = mvn_bin.parent().and_then(|p| p.parent()) {
                        let global_settings = maven_home.join("conf").join("settings.xml");
                        if global_settings.exists() {
                            if let Ok(content) = fs::read_to_string(&global_settings) {
                                if let Some(start) = content.find("<localRepository>") {
                                    if let Some(end) = content.find("</localRepository>") {
                                        if end > start + 17 {
                                            let potential_path = content[start + 17..end].trim().to_string();
                                            let is_commented = match content[..start].rfind("<!--") {
                                                Some(comment_start) => {
                                                    match content[comment_start..start].find("-->") {
                                                        None => true,
                                                        Some(_) => false,
                                                    }
                                                }
                                                None => false,
                                            };
                                            if !is_commented {
                                                mvn_path = potential_path;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if mvn_path.is_empty() {
        PathBuf::from(&user_profile).join(".m2").join("repository")
    } else {
        PathBuf::from(mvn_path)
    }
}

pub fn get_nuget_cache_path() -> PathBuf {
    let user_profile = std::env::var("USERPROFILE").unwrap_or_default();
    let nuget_path = std::env::var("NUGET_PACKAGES").unwrap_or_default();
    if nuget_path.is_empty() {
        PathBuf::from(&user_profile).join(".nuget").join("packages")
    } else {
        PathBuf::from(nuget_path)
    }
}

/// 返回某缓存路径的"检测依据"：来源说明 + 实际读到的内容。
/// 用于在界面上向用户透明展示"我们是怎么知道这个缓存在哪里的"。
pub fn cache_detect_evidence(name: &str, resolved: &str) -> (String, String) {
    let app_data = std::env::var("APPDATA").unwrap_or_default();
    let user_profile = std::env::var("USERPROFILE").unwrap_or_default();
    match name {
        "npm" => (
            "命令 `npm config get cache` 的输出".to_string(),
            format!("npm 报告的缓存目录: {}", resolved),
        ),
        "yarn" => (
            "命令 `yarn cache dir` 的输出".to_string(),
            format!("yarn 报告的缓存目录: {}", resolved),
        ),
        "pnpm" => (
            "命令 `pnpm store path` 的输出".to_string(),
            format!("pnpm 报告的存储目录: {}", resolved),
        ),
        "pip" => (
            format!("环境变量 PIP_CACHE_DIR，或配置文件 {}\\pip\\pip.ini 的 cache-dir 项", app_data),
            format!("解析得到的 pip 缓存目录: {}", resolved),
        ),
        "mvn" => (
            format!("配置文件 {}\\.m2\\settings.xml 的 <localRepository> 节点（或 Maven 全局 settings.xml）", user_profile),
            format!("解析得到的 Maven 本地仓库: {}", resolved),
        ),
        "nuget" => (
            "环境变量 NUGET_PACKAGES（未设置时回退到 %USERPROFILE%\\.nuget\\packages）".to_string(),
            format!("解析得到的 NuGet 全局包目录: {}", resolved),
        ),
        _ => (
            "包管理器默认缓存路径".to_string(),
            format!("检测到的缓存目录: {}", resolved),
        ),
    }
}

#[tauri::command]
pub fn get_caches_list() -> Result<Vec<CacheInfo>, String> {
    let npm_installed = is_installed("npm");
    let npm_path = get_npm_cache_path().to_string_lossy().to_string();

    let yarn_installed = is_installed("yarn");
    let yarn_path = get_yarn_cache_path().to_string_lossy().to_string();

    let pnpm_installed = is_installed("pnpm");
    let pnpm_path = get_pnpm_cache_path().to_string_lossy().to_string();

    let pip_installed = is_installed("pip");
    let pip_path = get_pip_cache_path().to_string_lossy().to_string();

    let mvn_installed = is_installed("mvn");
    let mvn_path = get_maven_cache_path().to_string_lossy().to_string();

    let nuget_installed = is_installed("dotnet") || is_installed("nuget");
    let nuget_path = get_nuget_cache_path().to_string_lossy().to_string();

    let raw_caches = vec![
        ("npm", npm_installed, npm_path),
        ("yarn", yarn_installed, yarn_path),
        ("pnpm", pnpm_installed, pnpm_path),
        ("pip", pip_installed, pip_path),
        ("mvn", mvn_installed, mvn_path),
        ("nuget", nuget_installed, nuget_path),
    ];

    let mut list = Vec::new();
    for (name, installed, raw_path) in raw_caches {
        let clean_path = Path::new(&raw_path);
        let mut is_link = false;
        let mut real_target = String::new();

        if let Ok(metadata) = fs::symlink_metadata(clean_path) {
            if metadata.file_type().is_symlink() || metadata.file_type().is_dir() {
                if let Ok(eval_path) = fs::read_link(clean_path) {
                    is_link = true;
                    real_target = eval_path.to_string_lossy().to_string();
                } else if let Ok(eval_path) = fs::canonicalize(clean_path) {
                    // Strips the Windows UNC prefix \\?\ if it exists
                    let canonical = eval_path.to_string_lossy().to_string();
                    let canonical_clean = canonical.trim_start_matches(r"\\?\").to_string();
                    if canonical_clean != clean_path.to_string_lossy().to_string() {
                        is_link = true;
                        real_target = canonical_clean;
                    }
                }
            }
        }

        let size_path = if is_link { Path::new(&real_target) } else { clean_path };
        let size_bytes = get_dir_size(size_path);
        let size_str = format_bytes(size_bytes);

        let (detect_source, detect_content) = cache_detect_evidence(name, &raw_path);

        list.push(CacheInfo {
            name: name.to_string(),
            installed,
            path: clean_path.to_string_lossy().to_string(),
            size: size_str,
            is_link,
            real_target,
            detect_source,
            detect_content,
        });
    }

    Ok(list)
}

#[tauri::command]
pub fn migrate_cache_path(name: String, new_path: String) -> Result<(), String> {
    let list = get_caches_list()?;
    let cache_info = list.iter().find(|c| c.name == name)
        .ok_or_else(|| format!("未找到缓存: {}", name))?;

    let orig_path = Path::new(&cache_info.path);
    let target_path = Path::new(&new_path);

    if orig_path == target_path {
        return Err("原路径与目标路径相同，无需迁移".to_string());
    }

    // Ensure target directory exists
    fs::create_dir_all(target_path).map_err(|e| format!("无法创建目标目录: {}", e))?;

    // Check if original path is already a junction/symlink
    let is_symlink = fs::symlink_metadata(orig_path).map(|m| m.file_type().is_symlink()).unwrap_or(false);

    if is_symlink {
        // Just remove old junction link
        fs::remove_file(orig_path).map_err(|e| format!("无法移除已有的旧链接: {}", e))?;
    } else {
        // Move files
        if orig_path.exists() {
            copy_dir_all(orig_path, target_path).map_err(|e| format!("复制缓存文件失败: {}", e))?;
            fs::remove_dir_all(orig_path).map_err(|e| format!("清空原缓存目录失败: {}", e))?;
        }
    }

    // Create Junction
    create_junction(orig_path, target_path)?;

    Ok(())
}

/// 迁移缓存目录（接受原始路径，用于包管理器缓存迁移）
pub fn migrate_cache_path_raw(orig_path_str: &str, new_path_str: &str) -> Result<(), String> {
    let orig_path = Path::new(orig_path_str);
    let target_path = Path::new(new_path_str);

    if orig_path == target_path {
        return Err("原路径与目标路径相同，无需迁移".to_string());
    }

    fs::create_dir_all(target_path).map_err(|e| format!("无法创建目标目录: {}", e))?;

    let is_symlink = fs::symlink_metadata(orig_path).map(|m| m.file_type().is_symlink()).unwrap_or(false);

    if is_symlink {
        fs::remove_file(orig_path).map_err(|e| format!("无法移除已有的旧链接: {}", e))?;
    } else {
        if orig_path.exists() {
            copy_dir_all(orig_path, target_path).map_err(|e| format!("复制缓存文件失败: {}", e))?;
            fs::remove_dir_all(orig_path).map_err(|e| format!("清空原缓存目录失败: {}", e))?;
        }
    }

    create_junction(orig_path, target_path)
}
