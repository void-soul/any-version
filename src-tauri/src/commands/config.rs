use std::fs;
use std::path::{Path, PathBuf};
use serde::{Serialize, Deserialize};
use tauri::Emitter;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProjectMenuConfig {
    #[serde(default = "default_true")]
    pub show_version: bool,
    #[serde(default = "default_true")]
    pub show_service: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ProjectDelegation {
    #[serde(default)]
    pub env_vars: std::collections::HashSet<String>,
    #[serde(default)]
    pub path_vars: std::collections::HashSet<String>,
    #[serde(default = "default_false")]
    pub version_control: bool,
    #[serde(default = "default_false")]
    pub create_symlink: bool,
    #[serde(default = "default_false")]
    pub manage_install_dir: bool,
    #[serde(default = "default_false")]
    pub manage_data_dir: bool,
    #[serde(default = "default_false")]
    pub manage_cache_dir: bool,
    #[serde(default)]
    pub manage_optional_tools: std::collections::HashSet<String>,
}

fn default_false() -> bool {
    false
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    pub versions_dir: String,
    pub links_dir: String,
    pub managed_items: std::collections::HashSet<String>,
    #[serde(default)]
    pub simple_managed_items: std::collections::HashSet<String>,
    #[serde(default)]
    pub custom_install_paths: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub custom_data_paths: std::collections::HashMap<String, std::collections::HashMap<String, String>>,
    #[serde(default)]
    pub project_menu_configs: std::collections::HashMap<String, ProjectMenuConfig>,
    #[serde(default)]
    pub project_delegations: std::collections::HashMap<String, ProjectDelegation>,
    #[serde(default)]
    pub active_versions: std::collections::HashMap<String, String>,
    pub original_envs: std::collections::HashMap<String, String>,
    pub original_paths: std::collections::HashMap<String, Vec<String>>,
    #[serde(default)]
    pub rss_sources: Vec<String>,
    /// RSS 订阅源自定义名称：url -> name。为空时前端回退使用 feed 标题。
    #[serde(default)]
    pub rss_source_names: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub has_run_before: bool,
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
        simple_managed_items: std::collections::HashSet::new(),
        custom_install_paths: std::collections::HashMap::new(),
        custom_data_paths: std::collections::HashMap::new(),
        project_menu_configs: std::collections::HashMap::new(),
        project_delegations: std::collections::HashMap::new(),
        active_versions: std::collections::HashMap::new(),
        original_envs: std::collections::HashMap::new(),
        original_paths: std::collections::HashMap::new(),
        rss_sources: vec![
            "https://36kr.com/feed".to_string(),
            "http://www.ruanyifeng.com/blog/atom.xml".to_string(),
        ],
        rss_source_names: default_rss_source_names(),
        has_run_before: false,
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

// ─── Backup storage (separate file) ───

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct BackupStore {
    /// project_id -> env var name -> original value
    pub env_vars: std::collections::HashMap<String, std::collections::HashMap<String, String>>,
    /// project_id -> list of PATH entries that were removed
    pub path_entries: std::collections::HashMap<String, Vec<String>>,
}

fn backup_path() -> PathBuf {
    get_base_dir().join("backups.json")
}

pub fn load_backups() -> BackupStore {
    let path = backup_path();
    if path.exists() {
        if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(store) = serde_json::from_str::<BackupStore>(&data) {
                return store;
            }
        }
    }
    BackupStore::default()
}

pub fn save_backups(store: &BackupStore) -> Result<(), String> {
    let path = backup_path();
    let data = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
    fs::write(path, data).map_err(|e| e.to_string())?;
    Ok(())
}

/// Migrate old backups from config.json into backups.json (one-time)
pub fn migrate_backups_from_config(config: &mut Config) {
    if !config.original_envs.is_empty() || !config.original_paths.is_empty() {
        let mut store = load_backups();
        for (k, v) in config.original_envs.drain() {
            // store under a generic key since old format didn't track per-project
            store.env_vars.entry("__migrated__".to_string()).or_default().insert(k, v);
        }
        for (k, v) in config.original_paths.drain() {
            store.path_entries.entry(k).or_default().extend(v);
        }
        let _ = save_backups(&store);
        let _ = save_config(config);
    }
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
    /// 迁移后仍在原位置的旧目录（供前端询问是否删除）
    pub old_dirs_remain: Vec<String>,
}

/// 迁移进度事件（通过 Tauri emit 发送）
#[derive(Serialize, Clone, Debug)]
pub struct MigrateProgress {
    pub stage: String,       // 当前阶段描述
    pub current: usize,
    pub total: usize,
    pub file_name: String,   // 当前正在处理的文件/目录名
}

/// 执行存储路径迁移：移动文件、更新 junction、更新环境变量/PATH
/// app_handle 用于发射进度事件（可选，传 None 则跳过）
pub fn do_migrate_storage(
    old_versions_dir: &str,
    new_versions_dir: &str,
    old_links_dir: &str,
    new_links_dir: &str,
    config: &Config,
    app_handle: Option<&tauri::AppHandle>,
) -> MigrateResult {
    let emit_progress = |stage: &str, current: usize, total: usize, file_name: &str| {
        if let Some(handle) = app_handle {
            let _ = handle.emit("migrate-progress", MigrateProgress {
                stage: stage.to_string(),
                current,
                total,
                file_name: file_name.to_string(),
            });
        }
    };

    let mut result = MigrateResult {
        moved_versions: false,
        moved_links: false,
        recreated_junctions: Vec::new(),
        updated_env_vars: Vec::new(),
        updated_path_entries: Vec::new(),
        errors: Vec::new(),
        old_dirs_remain: Vec::new(),
    };

    let fully_managed_items: Vec<String> = config.managed_items.iter()
        .filter_map(|item_id| {
            if config.simple_managed_items.contains(item_id) {
                return None;
            }
            match crate::commands::project::registry::find_by_id(item_id) {
                Some(def) if !def.simple_mode => Some(item_id.clone()),
                _ => None,
            }
        })
        .collect();

    let versions_changed = normalize(old_versions_dir) != normalize(new_versions_dir);
    let links_changed = normalize(old_links_dir) != normalize(new_links_dir);

    if !versions_changed && !links_changed {
        return result;
    }

    // ── 1. 迁移 links_dir ──
    if links_changed {
        let old_links = Path::new(old_links_dir);
        let new_links = Path::new(new_links_dir);

        if old_links.exists() {
            if let Err(e) = fs::create_dir_all(new_links) {
                result.errors.push(format!("创建新 links 目录失败: {}", e));
            } else {
                // 收集条目列表
                let entries: Vec<_> = if let Ok(rd) = fs::read_dir(old_links) {
                    rd.filter_map(|e| e.ok())
                        .filter(|e| e.path().is_dir() || e.path().is_symlink())
                        .collect()
                } else {
                    Vec::new()
                };
                let total = entries.len();

                // 移动 junction 目录
                for (i, entry) in entries.iter().enumerate() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy().to_string();
                    emit_progress("移动链接目录", i + 1, total, &name_str);

                    let old_path = old_links.join(&name);
                    let new_path = new_links.join(&name);
                    if new_path.exists() {
                        let _ = fs::remove_dir_all(&new_path);
                    }
                    if fs::rename(&old_path, &new_path).is_err() {
                        if let Err(e) = crate::commands::cache::copy_dir_all_with_progress(
                            &old_path, &new_path, app_handle,
                        ) {
                            result.errors.push(format!("复制链接目录 {:?} 失败: {}", e, name_str));
                        } else {
                            let _ = fs::remove_dir_all(&old_path);
                        }
                    }
                }
                result.moved_links = true;

                // 重建所有 junction
                let managed_vec: Vec<_> = fully_managed_items.iter().collect();
                let total_j = managed_vec.len();
                for (i, item_id) in managed_vec.iter().enumerate() {
                    emit_progress("重建 junction 链接", i + 1, total_j, item_id);
                    let link_path = new_links.join(item_id);
                    if let Ok(canonical) = fs::canonicalize(&link_path) {
                        let target_str = canonical.to_string_lossy().to_string()
                            .trim_start_matches(r"\\?\").to_string();
                        let new_target = if versions_changed {
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
                        result.recreated_junctions.push(item_id.to_string());
                    }
                }

                // ── 重写 PATH 环境变量：删除所有旧 links_dir 相关条目，重新写入所有托管 SDK 的路径 ──
                if let Some(user_path) = crate::commands::env::get_registry_env("PATH") {
                    let parts: Vec<String> = std::env::split_paths(&user_path)
                        .map(|p| p.to_string_lossy().to_string())
                        .collect();

                    let old_links_lower = normalize(old_links_dir);
                    let new_links_lower = normalize(new_links_dir);
                    let mut remaining_parts: Vec<String> = Vec::new();
                    let mut removed_count = 0usize;

                    // 删除所有与软件相关的 PATH 条目（旧 links_dir 和新 links_dir 的都删掉，确保干净重写）
                    for part in &parts {
                        let part_lower = part.to_lowercase();
                        if part_lower.contains(&old_links_lower) || part_lower.contains(&new_links_lower) {
                            result.updated_path_entries.push(format!("删除: {}", part));
                            removed_count += 1;
                        } else {
                            remaining_parts.push(part.clone());
                        }
                    }

                    // 重新写入：根据当前完全托管 SDK 列表生成新的 PATH 条目
                    use crate::commands::project::scanner;
                    for item_id in &fully_managed_items {
                        let link_dir = format!("{}\\{}", new_links_dir, item_id);
                        let bin_paths = scanner::get_bin_paths(item_id, &link_dir);
                        for bp in bin_paths.iter().rev() {
                            remaining_parts.insert(0, bp.clone());
                            result.updated_path_entries.push(format!("添加: {}", bp));
                        }
                    }

                    if removed_count > 0 || !fully_managed_items.is_empty() {
                        emit_progress("重写 PATH 环境变量", 1, 1, "");
                        if let Ok(new_path) = std::env::join_paths(remaining_parts.iter().map(Path::new)) {
                            let _ = crate::commands::env::set_registry_env(
                                "PATH",
                                &new_path.to_string_lossy().to_string(),
                            );
                        }
                    }
                }

                // ── 重写注册表环境变量：仅处理完全托管项目，并尊重 env tier ──
                use crate::commands::project::registry;
                use crate::commands::project::types::EnvVarTier;
                for item_id in &fully_managed_items {
                    if let Some(sdk_def) = registry::find_by_id(item_id) {
                        let link_dir = format!("{}\\{}", new_links_dir, item_id);
                        for var_info in &sdk_def.env_vars {
                            if var_info.tier.as_ref().map_or(false, |t| *t == EnvVarTier::Compat) {
                                continue;
                            }
                            if var_info.tier.as_ref().map_or(false, |t| *t == EnvVarTier::Clear) {
                                let _ = crate::commands::env::set_registry_env(&var_info.name, "");
                                result.updated_env_vars.push(format!("{} => <清空>", var_info.name));
                                continue;
                            }
                            let value = if let Some(ref sub) = var_info.sub_dir {
                                format!("{}\\{}", link_dir, sub)
                            } else {
                                link_dir.clone()
                            };
                            let _ = crate::commands::env::set_registry_env(&var_info.name, &value);
                            result.updated_env_vars.push(format!("{} => {}", var_info.name, value));
                        }
                    }
                }
            }
        }

        // 记录残留的旧目录
        if old_links.exists() {
            result.old_dirs_remain.push(old_links_dir.to_string());
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
                let entries: Vec<_> = if let Ok(rd) = fs::read_dir(old_versions) {
                    rd.filter_map(|e| e.ok())
                        .filter(|e| e.path().is_dir())
                        .collect()
                } else {
                    Vec::new()
                };
                let total = entries.len();

                for (i, entry) in entries.iter().enumerate() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy().to_string();
                    emit_progress("移动版本目录", i + 1, total, &name_str);

                    let old_path = old_versions.join(&name);
                    let new_path = new_versions.join(&name);
                    if new_path.exists() {
                        let _ = fs::remove_dir_all(&new_path);
                    }
                    if fs::rename(&old_path, &new_path).is_err() {
                        if let Err(e) = crate::commands::cache::copy_dir_all_with_progress(
                            &old_path, &new_path, app_handle,
                        ) {
                            result.errors.push(format!("复制版本目录 {:?} 失败: {}", e, name_str));
                        } else {
                            let _ = fs::remove_dir_all(&old_path);
                        }
                    }
                }
                result.moved_versions = true;

                // 如果 links_dir 没变，需要单独重建 junction
                if !links_changed {
                    let links_path = Path::new(new_links_dir);
                    let managed_vec: Vec<_> = fully_managed_items.iter().collect();
                    let total_j = managed_vec.len();
                    for (i, item_id) in managed_vec.iter().enumerate() {
                        emit_progress("重建 junction 链接", i + 1, total_j, item_id);
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
                                result.recreated_junctions.push(item_id.to_string());
                            }
                        }
                    }
                }
            }
        }

        // 记录残留的旧目录
        if old_versions.exists() {
            result.old_dirs_remain.push(old_versions_dir.to_string());
        }
    }

    // 发送完成事件
    emit_progress("完成", 1, 1, "");
    result
}

/// 删除旧的存储目录（迁移后调用）
#[tauri::command]
pub fn delete_old_storage_dirs(dirs: Vec<String>) -> Result<Vec<String>, String> {
    let mut deleted = Vec::new();
    let mut errors = Vec::new();
    for dir in &dirs {
        let path = Path::new(dir);
        if path.exists() {
            match fs::remove_dir_all(path) {
                Ok(()) => deleted.push(dir.clone()),
                Err(e) => errors.push(format!("删除 {} 失败: {}", dir, e)),
            }
        } else {
            deleted.push(format!("{} (已不存在)", dir));
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    Ok(deleted)
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
pub fn update_config(app_handle: tauri::AppHandle, versions_dir: String, links_dir: String) -> Result<MigrateResult, String> {
    let old_config = load_config();
    let old_versions_dir = old_config.versions_dir.clone();
    let old_links_dir = old_config.links_dir.clone();

    let mut config = old_config;

    let result = do_migrate_storage(
        &old_versions_dir,
        &versions_dir,
        &old_links_dir,
        &links_dir,
        &config,
        Some(&app_handle),
    );

    if !result.errors.is_empty() {
        return Err(result.errors.join("\n"));
    }

    config.versions_dir = versions_dir;
    config.links_dir = links_dir;
    save_config(&config)?;

    Ok(result)
}

#[tauri::command]
pub fn get_project_menu_config(id: String) -> Result<ProjectMenuConfig, String> {
    let config = load_config();
    Ok(config.project_menu_configs.get(&id).cloned().unwrap_or_else(|| ProjectMenuConfig {
        show_version: true,
        show_service: true,
    }))
}

#[tauri::command]
pub fn update_project_menu_config(app_handle: tauri::AppHandle, id: String, show_version: bool, show_service: bool) -> Result<(), String> {
    let mut config = load_config();
    config.project_menu_configs.insert(id, ProjectMenuConfig {
        show_version,
        show_service,
    });
    save_config(&config)?;
    let _ = crate::tray::rebuild_tray_menu(&app_handle);
    Ok(())
}

/// 单个 RSS 订阅源（含自定义名称）
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RssSourceDto {
    pub url: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RssConfig {
    pub sources: Vec<RssSourceDto>,
    pub is_first_launch: bool,
}

/// 内置默认 RSS 源的名称映射
fn default_rss_source_names() -> std::collections::HashMap<String, String> {
    let mut m = std::collections::HashMap::new();
    m.insert("https://36kr.com/feed".to_string(), "36氪".to_string());
    m.insert(
        "http://www.ruanyifeng.com/blog/atom.xml".to_string(),
        "阮一峰的网络日志".to_string(),
    );
    m
}

/// 将 config 中的 url 列表与名称映射合并为带名称的源列表
fn build_rss_sources(config: &Config) -> Vec<RssSourceDto> {
    config
        .rss_sources
        .iter()
        .map(|url| RssSourceDto {
            url: url.clone(),
            name: config.rss_source_names.get(url).cloned().unwrap_or_default(),
        })
        .collect()
}

#[tauri::command]
pub fn get_rss_config() -> Result<RssConfig, String> {
    let mut config = load_config();
    let is_first_launch = !config.has_run_before;
    if is_first_launch {
        config.has_run_before = true;
        if config.rss_sources.is_empty() {
            config.rss_sources = vec![
                "https://36kr.com/feed".to_string(),
                "http://www.ruanyifeng.com/blog/atom.xml".to_string(),
            ];
        }
        if config.rss_source_names.is_empty() {
            config.rss_source_names = default_rss_source_names();
        }
        save_config(&config)?;
    }
    Ok(RssConfig {
        sources: build_rss_sources(&config),
        is_first_launch,
    })
}

#[tauri::command]
pub fn set_rss_sources(sources: Vec<RssSourceDto>) -> Result<(), String> {
    let mut config = load_config();
    config.rss_sources = sources.iter().map(|s| s.url.clone()).collect();
    config.rss_source_names = sources
        .into_iter()
        .filter(|s| !s.name.trim().is_empty())
        .map(|s| (s.url, s.name.trim().to_string()))
        .collect();
    save_config(&config)?;
    Ok(())
}

#[tauri::command]
pub async fn fetch_rss_feed(url: String) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .connect_timeout(std::time::Duration::from_secs(3))
        .build()
        .map_err(|e| e.to_string())?;

    let response = client.get(&url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    let text = response.text()
        .await
        .map_err(|e| format!("读取内容失败: {}", e))?;

    Ok(text)
}

