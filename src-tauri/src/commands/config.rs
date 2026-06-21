use std::fs;
use std::path::{Path, PathBuf};
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    pub versions_dir: String,
    pub links_dir: String,
    pub managed_items: std::collections::HashSet<String>,
    pub original_envs: std::collections::HashMap<String, String>,
    pub original_paths: std::collections::HashMap<String, Vec<String>>,
}

pub fn get_base_dir() -> PathBuf {
    let user_profile = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| {
            let drive = std::env::var("HOMEDRIVE").unwrap_or_default();
            let path = std::env::var("HOMEPATH").unwrap_or_default();
            if drive.is_empty() && path.is_empty() {
                "C:\\any-version".to_string()
            } else {
                format!("{}{}", drive, path)
            }
        });
    let mut path = PathBuf::from(user_profile);
    if path.as_os_str().is_empty() || path == PathBuf::from("C:\\any-version") {
        PathBuf::from("C:\\any-version")
    } else {
        path.push(".any-version");
        path
    }
}

pub fn load_config() -> Config {
    let base_dir = get_base_dir();
    let config_path = base_dir.join("config.json");
    if config_path.exists() {
        if let Ok(data) = fs::read_to_string(&config_path) {
            if let Ok(config) = serde_json::from_str::<Config>(&data) {
                return config;
            }
        }
    }
    let default_config = Config {
        versions_dir: base_dir.join("versions").to_string_lossy().to_string(),
        links_dir: base_dir.join("links").to_string_lossy().to_string(),
        managed_items: std::collections::HashSet::new(),
        original_envs: std::collections::HashMap::new(),
        original_paths: std::collections::HashMap::new(),
    };
    let _ = fs::create_dir_all(&base_dir);
    let _ = save_config(&default_config);
    default_config
}

pub fn save_config(config: &Config) -> Result<(), String> {
    let base_dir = get_base_dir();
    let config_path = base_dir.join("config.json");
    let data = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    fs::write(config_path, data).map_err(|e| e.to_string())?;
    Ok(())
}

fn normalize(path: &str) -> String {
    let p = path.trim_end_matches('\\').trim_end_matches('/');
    p.to_lowercase()
}

/// 路径迁移结果（供前端展示）
#[derive(Serialize, Clone, Debug)]
pub struct MigrateResult {
    pub moved_versions: bool,
    pub moved_links: bool,
    pub recreated_junctions: Vec<String>,
    pub updated_env_vars: Vec<String>,
    pub updated_path_entries: Vec<String>,
    pub errors: Vec<String>,
}

/// 执行存储路径迁移：移动文件、更新 junction、更新环境变量/PATH
pub fn do_migrate_storage(
    old_versions_dir: &str,
    new_versions_dir: &str,
    old_links_dir: &str,
    new_links_dir: &str,
    managed_items: &std::collections::HashSet<String>,
) -> MigrateResult {
    let mut result = MigrateResult {
        moved_versions: false,
        moved_links: false,
        recreated_junctions: Vec::new(),
        updated_env_vars: Vec::new(),
        updated_path_entries: Vec::new(),
        errors: Vec::new(),
    };

    let versions_changed = normalize(old_versions_dir) != normalize(new_versions_dir);
    let links_changed = normalize(old_links_dir) != normalize(new_links_dir);

    if !versions_changed && !links_changed {
        return result;
    }

    // ── 1. 迁移 links_dir ──
    if links_changed {
        let old_links = Path::new(old_links_dir);
        let new_links = Path::new(new_links_dir);

        // 移动所有 junction 目录
        if old_links.exists() {
            if let Err(e) = fs::create_dir_all(new_links) {
                result.errors.push(format!("创建新 links 目录失败: {}", e));
            } else {
                // 先取消所有旧 junction 的 env vars 和 PATH 指向
                let mut old_path_entries = Vec::new();
                for item_id in managed_items {
                    // 记录需要更新的 PATH 条目
                    let link_path = old_links.join(item_id);
                    let link_str = link_path.to_string_lossy().to_string();
                    let bin_paths = crate::commands::env::get_sdk_bin_paths(item_id, &link_str);
                    for bp in &bin_paths {
                        old_path_entries.push(bp.clone());
                    }
                }

                // 移动 junction 目录
                if let Ok(entries) = fs::read_dir(old_links) {
                    for entry in entries.filter_map(|e| e.ok()) {
                        let name = entry.file_name();
                        let old_path = old_links.join(&name);
                        let new_path = new_links.join(&name);
                        if old_path.is_dir() || old_path.is_symlink() {
                            if new_path.exists() {
                                let _ = fs::remove_dir_all(&new_path);
                            }
                            if fs::rename(&old_path, &new_path).is_err() {
                                // 跨盘符时 rename 会失败，改用复制
                                if let Err(e) = crate::commands::cache::copy_dir_all(&old_path, &new_path) {
                                    result.errors.push(format!("复制链接目录 {:?} 失败: {}", e, name));
                                } else {
                                    let _ = fs::remove_dir_all(&old_path);
                                }
                            }
                        }
                    }
                }
                result.moved_links = true;

                // 重建所有 junction（因为目标路径可能也变了）
                for item_id in managed_items {
                    let link_path = new_links.join(item_id);
                    if let Ok(canonical) = fs::canonicalize(&link_path) {
                        // 获取 junction 实际指向的版本目录名
                        let target_str = canonical.to_string_lossy().to_string()
                            .trim_start_matches(r"\\?\").to_string();
                        // 如果 versions_dir 也变了，重新计算目标路径
                        let new_target = if versions_changed {
                            // 从旧目标路径中提取 project_id/version 部分
                            let old_ver_prefix = format!("{}\\{}", normalize(old_versions_dir), item_id);
                            let target_lower = target_str.to_lowercase();
                            if let Some(rel_pos) = target_lower.find(&old_ver_prefix) {
                                let rel = &target_str[rel_pos + old_ver_prefix.len()..];
                                let rel = rel.trim_start_matches('\\').trim_start_matches('/');
                                format!("{}\\{}\\{}", new_versions_dir, item_id, rel)
                            } else {
                                target_str.clone()
                            }
                        } else {
                            target_str
                        };

                        let _ = crate::commands::cache::create_junction(&link_path, Path::new(&new_target));
                        result.recreated_junctions.push(item_id.clone());
                    }
                }

                // 更新 PATH 中的旧链接路径
                if let Some(user_path) = crate::commands::env::get_registry_env("PATH") {
                    let parts: Vec<String> = std::env::split_paths(&user_path)
                        .map(|p| p.to_string_lossy().to_string())
                        .collect();

                    let old_links_lower = normalize(old_links_dir);
                    let mut new_parts = Vec::new();
                    let mut updated = false;

                    for part in &parts {
                        let part_lower = part.to_lowercase();
                        if part_lower.contains(&old_links_lower) {
                            let new_part = part_lower.replacen(
                                &old_links_lower,
                                &normalize(new_links_dir),
                                1,
                            );
                            // 保持原始大小写
                            let mut rebuilt = String::new();
                            let mut old_chars = part.chars();
                            let mut new_chars = new_part.chars();
                            loop {
                                match (old_chars.next(), new_chars.next()) {
                                    (Some(_), Some(nc)) => rebuilt.push(nc),
                                    (Some(oc), None) => rebuilt.push(oc),
                                    _ => break,
                                }
                            }
                            result.updated_path_entries.push(rebuilt.clone());
                            new_parts.push(rebuilt);
                            updated = true;
                        } else {
                            new_parts.push(part.clone());
                        }
                    }

                    if updated {
                        let new_path = std::env::join_paths(new_parts.iter().map(Path::new))
                            .map(|e| e.to_string())
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let _ = crate::commands::env::set_registry_env("PATH", &new_path);
                    }
                }

                // 更新注册表中的环境变量
                use crate::commands::project::registry;
                for item_id in managed_items {
                    if let Some(sdk_def) = registry::find_by_id(item_id) {
                        for var_info in &sdk_def.env_vars {
                            if let Some((val, _)) = crate::commands::env::get_registry_env_any(&var_info.name) {
                                let val_lower = val.to_lowercase();
                                if val_lower.contains(&normalize(old_links_dir)) {
                                    let new_val = val_lower.replacen(
                                        &normalize(old_links_dir),
                                        &normalize(new_links_dir),
                                        1,
                                    );
                                    // 恢复原始大小写模式
                                    let rebuilt = val.replacen(old_links_dir, new_links_dir, 1);
                                    let _ = crate::commands::env::set_registry_env(&var_info.name, &rebuilt);
                                    result.updated_env_vars.push(var_info.name.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // ── 2. 迁移 versions_dir ──
    if versions_changed {
        let old_versions = Path::new(old_versions_dir);
        let new_versions = Path::new(new_versions_dir);

        if old_versions.exists() {
            if let Err(e) = fs::create_dir_all(new_versions) {
                result.errors.push(format!("创建新 versions 目录失败: {}", e));
            } else {
                // 移动所有项目版本目录
                if let Ok(entries) = fs::read_dir(old_versions) {
                    for entry in entries.filter_map(|e| e.ok()) {
                        let name = entry.file_name();
                        let old_path = old_versions.join(&name);
                        let new_path = new_versions.join(&name);
                        if old_path.is_dir() {
                            if new_path.exists() {
                                let _ = fs::remove_dir_all(&new_path);
                            }
                            if fs::rename(&old_path, &new_path).is_err() {
                                if let Err(e) = crate::commands::cache::copy_dir_all(&old_path, &new_path) {
                                    result.errors.push(format!("复制版本目录 {:?} 失败: {}", e, name));
                                } else {
                                    let _ = fs::remove_dir_all(&old_path);
                                }
                            }
                        }
                    }
                }
                result.moved_versions = true;

                // 如果 links_dir 没变，需要单独重建 junction
                if !links_changed {
                    let links_path = Path::new(new_links_dir);
                    for item_id in managed_items {
                        let link_path = links_path.join(item_id);
                        if let Ok(canonical) = fs::canonicalize(&link_path) {
                            let target_str = canonical.to_string_lossy().to_string()
                                .trim_start_matches(r"\\?\").to_string();
                            let old_ver_prefix = format!("{}\\{}", normalize(old_versions_dir), item_id);
                            let target_lower = target_str.to_lowercase();
                            if let Some(rel_pos) = target_lower.find(&old_ver_prefix) {
                                let rel = &target_str[rel_pos + old_ver_prefix.len()..];
                                let rel = rel.trim_start_matches('\\').trim_start_matches('/');
                                let new_target = format!("{}\\{}\\{}", new_versions_dir, item_id, rel);
                                let _ = crate::commands::cache::create_junction(&link_path, Path::new(&new_target));
                                result.recreated_junctions.push(item_id.clone());
                            }
                        }
                    }
                }
            }
        }
    }

    result
}

#[tauri::command]
pub fn get_config() -> Result<Config, String> {
    Ok(load_config())
}

/// 获取应用版本号（从 Cargo.toml）
#[tauri::command]
pub fn get_app_version() -> Result<String, String> {
    Ok(env!("CARGO_PKG_VERSION").to_string())
}

/// 更新配置并自动迁移存储路径。
/// 如果 versions_dir 或 links_dir 发生变化，自动执行：
///   1. 将旧目录下的已安装版本文件移动到新目录
///   2. 更新所有 junction 链接的指向
///   3. 更新 PATH 环境变量中的旧路径为新路径
#[tauri::command]
pub fn update_config(versions_dir: String, links_dir: String) -> Result<MigrateResult, String> {
    let old_config = load_config();
    let old_versions_dir = old_config.versions_dir.clone();
    let old_links_dir = old_config.links_dir.clone();

    let mut config = old_config;

    let result = do_migrate_storage(
        &old_versions_dir,
        &versions_dir,
        &old_links_dir,
        &links_dir,
        &config.managed_items,
    );

    if !result.errors.is_empty() {
        return Err(result.errors.join("\n"));
    }

    config.versions_dir = versions_dir;
    config.links_dir = links_dir;
    save_config(&config)?;

    Ok(result)
}
