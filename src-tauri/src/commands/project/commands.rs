//! 项目管理模块 — Tauri 命令层。
//!
//! 暴露给前端的所有 project_* 命令，内部调用 scanner 模块执行实际逻辑。

use tauri::Emitter;
use super::types::{ProjectStatus, ProjectDetail, ManagePreview};
use super::scanner;

/// 获取所有项目列表及运行时状态
#[tauri::command]
pub fn project_list() -> Result<Vec<ProjectStatus>, String> {
    scanner::list_projects()
}

/// 获取单个项目运行时状态
#[tauri::command]
pub fn project_status(id: String) -> Result<ProjectStatus, String> {
    scanner::get_project_status(&id)
}

/// 获取项目详情（定义 + 状态）
#[tauri::command]
pub fn project_detail(id: String) -> Result<ProjectDetail, String> {
    scanner::get_project_detail(&id)
}

/// 预览托管操作步骤
#[tauri::command]
pub fn project_preview_manage(id: String) -> Result<ManagePreview, String> {
    scanner::preview_manage(&id)
}

/// 托管项目（将项目纳入 AnyVersion 管理）
///
/// 执行步骤：
/// 1. 备份当前环境变量
/// 2. 清理 PATH 中的外部条目
/// 3. 设置项目环境变量（指向 AnyVersion 管理目录）
/// 4. 添加 bin 路径到 PATH
/// 5. 将项目 ID 添加到 managed_items
/// 6. 保存配置和管理备份
#[tauri::command]
pub fn project_manage(id: String) -> Result<(), String> {
    use crate::commands::config::load_config;
    use crate::commands::env::{get_registry_env_any, set_registry_env, add_to_user_path};
    use super::registry;

    let def = registry::find_by_id(&id)
        .ok_or_else(|| format!("未找到项目: {}", id))?;

    let mut config = load_config();

    // 如果已托管，直接返回
    if config.managed_items.contains(id.as_str()) {
        return Ok(());
    }

    // 1. 备份当前环境变量
    for var_def in &def.env_vars {
        // Only backup core + package tier vars
        if let Some(tier) = &var_def.tier {
            if *tier == super::types::EnvVarTier::Compat { continue; }
        }
        if let Some((val, _)) = get_registry_env_any(&var_def.name) {
            if !val.to_lowercase().contains(&config.links_dir.to_lowercase()) {
                config.original_envs.entry(var_def.name.clone()).or_insert(val);
            }
        }
    }

    // 2. 清理 PATH 中的外部条目
    use std::path::Path;
    if let Some(user_path) = crate::commands::env::get_registry_env("PATH") {
        let parts = std::env::split_paths(&user_path)
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        let mut matched_entries = Vec::new();
        let mut remaining_entries = Vec::new();

        for p_str in parts {
            if p_str.is_empty() {
                continue;
            }
            let p_lower = p_str.to_lowercase();

            if !p_lower.contains(&config.links_dir.to_lowercase()) {
                let mut matches = false;
                for rule in &def.find_rules {
                    match &rule.pattern {
                        super::types::ResolvePattern::PathContains { path_key, .. } => {
                            if p_lower.contains(&path_key.to_lowercase()) {
                                matches = true;
                                break;
                            }
                        }
                        super::types::ResolvePattern::FixedPath { path: fixed_path, .. } => {
                            if p_lower.contains(&fixed_path.to_lowercase()) {
                                matches = true;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                if matches {
                    matched_entries.push(p_str.clone());
                    continue;
                }
            }
            remaining_entries.push(p_str);
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

    // 3. 添加到 managed_items
    config.managed_items.insert(id.clone());
    crate::commands::config::save_config(&config)?;

    // 4. 设置项目环境变量（仅语言本身的变量，包管理器变量独立管理）
    let link_dir = Path::new(&config.links_dir).join(&id);
    let link_str = link_dir.to_string_lossy().to_string();
    for var_def in &def.env_vars {
        // Skip compat/discovery tier vars - only manage core + package
        if let Some(tier) = &var_def.tier {
            if *tier == super::types::EnvVarTier::Compat { continue; }
        }
        // 值策略由 EnvVarDef.sub_dir 驱动
        let value = if let Some(ref sub) = var_def.sub_dir {
            format!("{}\\{}", link_str, sub)
        } else {
            link_str.clone()
        };
        let _ = set_registry_env(&var_def.name, &value);
    }

    // 5. 添加 bin 路径到 PATH
    let bin_paths = scanner::get_bin_paths(&def.id, &link_str);
    if !bin_paths.is_empty() {
        let _ = add_to_user_path(&bin_paths);
    }

    // 6. 同步当前进程环境变量，使子进程（如 run_cmd_capture）能立即使用新值
    if let Some(user_path) = crate::commands::env::get_registry_env("PATH") {
        std::env::set_var("PATH", &user_path);
    }
    // 同步所有项目环境变量到当前进程
    for var_def in &def.env_vars {
        if let Some(tier) = &var_def.tier {
            if *tier == super::types::EnvVarTier::Compat { continue; }
        }
        let value = if let Some(ref sub) = var_def.sub_dir {
            format!("{}\\{}", link_str, sub)
        } else {
            link_str.clone()
        };
        std::env::set_var(&var_def.name, &value);
    }

    // 7. 备份旧版数据到独立备份目录（便于用户查看和管理）
    save_manage_backup(&id, &def, &config);

    Ok(())
}

/// 取消托管项目（从 AnyVersion 管理中移除）
///
/// 执行步骤：
/// 1. 移除项目环境变量
/// 2. 从 PATH 中移除 AnyVersion 添加的条目
/// 3. 还原原始环境变量
/// 4. 还原原始 PATH 条目
/// 5. 从 managed_items 中移除
/// 6. 保存配置
#[tauri::command]
pub fn project_unmanage(id: String) -> Result<(), String> {
    use crate::commands::config::load_config;
    use crate::commands::env::{set_registry_env, remove_from_user_path, add_to_user_path};
    use super::registry;

    let def = registry::find_by_id(&id)
        .ok_or_else(|| format!("未找到项目: {}", id))?;

    let mut config = load_config();

    // 如果未托管，直接返回
    if !config.managed_items.contains(id.as_str()) {
        return Ok(());
    }

    // 1. 移除项目环境变量（env_vars 已包含 NPM_CONFIG_PREFIX 等托管变量）
    for var_def in &def.env_vars {
        // Skip compat/discovery tier vars - we did not set them
        if let Some(tier) = &var_def.tier {
            if *tier == super::types::EnvVarTier::Compat { continue; }
        }
        let _ = set_registry_env(&var_def.name, "");
    }

    // 2. 从 PATH 中移除 AnyVersion 添加的条目
    let link_dir = std::path::Path::new(&config.links_dir).join(&id);
    let link_str = link_dir.to_string_lossy().to_string();
    let bin_paths = scanner::get_bin_paths(&def.id, &link_str);
    if !bin_paths.is_empty() {
        let _ = remove_from_user_path(&bin_paths);
    }

    // 3. 还原原始环境变量
    for var_def in &def.env_vars {
        // Skip compat/discovery tier vars - we did not set them
        if let Some(tier) = &var_def.tier {
            if *tier == super::types::EnvVarTier::Compat { continue; }
        }
        if let Some(orig_val) = config.original_envs.remove(&var_def.name) {
            let _ = set_registry_env(&var_def.name, &orig_val);
        }
    }

    // 4. 还原原始 PATH 条目
    if let Some(orig_paths) = config.original_paths.remove(&id) {
        let _ = add_to_user_path(&orig_paths);
    }

    // 5. 从 managed_items 中移除
    config.managed_items.remove(&id);
    crate::commands::config::save_config(&config)?;

    // 6. 同步当前进程环境变量
    if let Some(user_path) = crate::commands::env::get_registry_env("PATH") {
        std::env::set_var("PATH", &user_path);
    }

    Ok(())
}

/// 将托管时的旧版安装数据和环境备份保存到独立文件，便于用户查看和管理
fn save_manage_backup(
    id: &str,
    def: &super::types::ProjectDef,
    config: &crate::commands::config::Config,
) {
    use std::collections::HashMap;
    use super::scanner;

    let base_dir = crate::commands::config::get_base_dir();
    let backup_dir = base_dir.join("backup");
    if let Err(e) = std::fs::create_dir_all(&backup_dir) {
        eprintln!("[manage_backup] 创建备份目录失败: {}", e);
        return;
    }

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // 检测旧版安装信息（版本号、来源、路径）
    let (legacy_source, legacy_root) = scanner::detect_install_source(def);
    let legacy_version = legacy_root.as_ref().and_then(|root| {
        let path = std::path::Path::new(root);
        if path.exists() {
            // 使用版本检测命令获取版本号
            super::versions::detect_version_from_path(id, path)
        } else {
            None
        }
    });

    // 收集备份的环境变量（仅与此项目相关的）
    let mut backed_env_vars: HashMap<String, String> = HashMap::new();
    for var_def in &def.env_vars {
        if let Some(val) = config.original_envs.get(&var_def.name) {
            backed_env_vars.insert(var_def.name.clone(), val.clone());
        }
    }

    let backup_data = serde_json::json!({
        "project_id": id,
        "action": "manage",
        "timestamp": ts,
        "legacy_install_source": legacy_source,
        "legacy_install_root": legacy_root,
        "legacy_version": legacy_version,
        "backed_env_vars": backed_env_vars,
        "backed_path_entries": config.original_paths.get(id),
    });

    let backup_file = backup_dir.join(format!("manage_{}_{}.json", id, ts));
    let data = serde_json::to_string_pretty(&backup_data).unwrap_or_default();
    if let Err(e) = std::fs::write(&backup_file, &data) {
        eprintln!("[manage_backup] 写入备份文件失败: {}", e);
    }
}

/// 取消托管预演 — 展示将要执行的操作步骤
#[tauri::command]
pub fn project_preview_unmanage(id: String) -> Result<ManagePreview, String> {
    use crate::commands::config::load_config;
    use super::registry;

    let def = registry::find_by_id(&id)
        .ok_or_else(|| format!("未找到项目: {}", id))?;

    let config = load_config();

    if !config.managed_items.contains(id.as_str()) {
        return Err("该项目尚未托管".to_string());
    }

    let mut steps = Vec::new();

    let link_dir = std::path::Path::new(&config.links_dir).join(&id);
    let link_str = link_dir.to_string_lossy().to_string();

    // 步骤 1: 将要清除的环境变量
    let compat_vars: Vec<String> = def.env_vars.iter()
        .filter(|v| v.tier.as_ref().map_or(false, |t| *t == super::types::EnvVarTier::Compat))
        .map(|v| v.name.clone())
        .collect();

    let managed_vars: Vec<String> = def.env_vars.iter()
        .filter(|v| !v.tier.as_ref().map_or(false, |t| *t == super::types::EnvVarTier::Compat))
        .map(|v| v.name.clone())
        .collect();

    if !managed_vars.is_empty() {
        steps.push(super::types::ManageStep {
            action: "clear_env".to_string(),
            description: format!("清除 AnyVersion 设置的 {} 个环境变量", managed_vars.len()),
            target: managed_vars.join(", "),
        });
    }

    // 步骤 2: 将从 PATH 中移除的条目
    let bin_paths = scanner::get_bin_paths(&def.id, &link_str);
    if !bin_paths.is_empty() {
        steps.push(super::types::ManageStep {
            action: "remove_path".to_string(),
            description: format!("从 PATH 中移除 {} 个条目", bin_paths.len()),
            target: bin_paths.join("\n"),
        });
    }

    // 步骤 3: 将要还原的环境变量
    let mut restore_count = 0;
    let mut restore_list = Vec::new();
    for var_def in &def.env_vars {
        if var_def.tier.as_ref().map_or(false, |t| *t == super::types::EnvVarTier::Compat) {
            continue;
        }
        if let Some(orig_val) = config.original_envs.get(&var_def.name) {
            restore_count += 1;
            restore_list.push(format!("{} = {}", var_def.name, orig_val));
        }
    }
    if restore_count > 0 {
        steps.push(super::types::ManageStep {
            action: "restore_env".to_string(),
            description: format!("还原 {} 个环境变量的原始值", restore_count),
            target: restore_list.join("\n"),
        });
    }

    // 步骤 4: 将要还原的 PATH 条目
    if let Some(orig_paths) = config.original_paths.get(&id) {
        if !orig_paths.is_empty() {
            steps.push(super::types::ManageStep {
                action: "restore_path".to_string(),
                description: format!("向 PATH 恢复 {} 个原始条目", orig_paths.len()),
                target: orig_paths.join("\n"),
            });
        }
    }

    // 步骤 5: 说明 Compat 层变量不会被修改
    if !compat_vars.is_empty() {
        steps.push(super::types::ManageStep {
            action: "skip_compat".to_string(),
            description: format!("以下 {} 个兼容层变量保持不变（不属于 AnyVersion 管理）: {}", compat_vars.len(), compat_vars.join(", ")),
            target: String::new(),
        });
    }

    Ok(ManagePreview {
        steps,
        has_local_install: false,
        local_install_root: None,
        local_install_source: None,
    })
}

/// 执行 shell 命令并捕获输出（用于包管理器版本检测、镜像切换等）
#[tauri::command]
pub fn run_cmd_capture(cmd: String) -> Result<String, String> {
    let mut command = super::super::hidden_cmd::hidden_cmd("cmd");
    command.args(&["/c", &cmd]);

    // 注入 NPM_CONFIG_PREFIX，确保 npm install -g 安装到正确的 link_dir
    // （当前进程环境变量可能未同步注册表，这里兜底保证子进程用对 prefix）
    if let Ok(prefix) = std::env::var("NPM_CONFIG_PREFIX") {
        command.env("NPM_CONFIG_PREFIX", &prefix);
    }

    let output = command
        .output()
        .map_err(|e| format!("执行命令失败: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if output.status.success() {
        Ok(stdout)
    } else if !stderr.is_empty() {
        Err(stderr)
    } else {
        Err(format!("命令执行失败 (exit code: {})", output.status.code().unwrap_or(-1)))
    }
}

/// 获取项目的旧版安装备份数据（供"旧版数据"选项卡展示）
#[derive(serde::Serialize, Clone, Debug)]
pub struct LegacyBackupInfo {
    pub project_id: String,
    pub install_source: Option<String>,
    pub install_root: Option<String>,
    pub version: Option<String>,
    pub backed_env_vars: std::collections::HashMap<String, String>,
    pub removed_path_entries: Vec<String>,
    pub timestamp: u64,
}

#[tauri::command]
pub fn get_legacy_backup(id: String) -> Result<Option<LegacyBackupInfo>, String> {
    let base_dir = crate::commands::config::get_base_dir();
    let backup_dir = base_dir.join("backup");
    if !backup_dir.exists() {
        return Ok(None);
    }

    // 查找该项目的备份文件
    let mut best: Option<(u64, std::path::PathBuf)> = None;
    if let Ok(entries) = std::fs::read_dir(&backup_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with(&format!("manage_{}_", id)) && name_str.ends_with(".json") {
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    if let Ok(data) = serde_json::from_str::<serde_json::Value>(&content) {
                        let ts = data.get("timestamp").and_then(|v| v.as_u64()).unwrap_or(0);
                        if best.is_none() || ts > best.as_ref().unwrap().0 {
                            best = Some((ts, entry.path()));
                        }
                    }
                }
            }
        }
    }

    match best {
        Some((_ts, path)) => {
            let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let data: serde_json::Value = serde_json::from_str(&content).map_err(|e| e.to_string())?;

            let backed_env_vars = data.get("backed_env_vars")
                .and_then(|v| v.as_object())
                .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect())
                .unwrap_or_default();

            let removed_path_entries = data.get("backed_path_entries")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();

            Ok(Some(LegacyBackupInfo {
                project_id: id,
                install_source: data.get("legacy_install_source").and_then(|v| v.as_str().map(String::from)),
                install_root: data.get("legacy_install_root").and_then(|v| v.as_str().map(String::from)),
                version: data.get("legacy_version").and_then(|v| v.as_str().map(String::from)),
                backed_env_vars,
                removed_path_entries,
                timestamp: data.get("timestamp").and_then(|v| v.as_u64()).unwrap_or(0),
            }))
        }
        None => Ok(None),
    }
}

/// 获取包管理器缓存信息
#[derive(serde::Serialize, Clone, Debug)]
pub struct PkgCacheInfo {
    pub path: String,
    pub size: String,
    pub is_link: bool,
    pub real_target: String,
    /// 如果父目录是 junction（如 C:\Users\...\Yarn → D:\Yarn），则返回父目录信息
    pub parent_link: Option<ParentLinkInfo>,
}

#[derive(serde::Serialize, Clone, Debug)]
pub struct ParentLinkInfo {
    /// 父目录路径（如 C:\Users\...\Yarn）
    pub parent_path: String,
    /// 父目录 junction 实际目标（如 D:\Yarn）
    pub parent_target: String,
    /// 父目录下缓存子目录的相对路径（如 Cache）
    pub child_rel: String,
}

fn check_parent_junction(cache_path: &str) -> Option<ParentLinkInfo> {
    let p = std::path::Path::new(cache_path);
    let parent = p.parent()?;
    if let Ok(meta) = std::fs::symlink_metadata(parent) {
        if meta.file_type().is_symlink() {
            let target = std::fs::read_link(parent)
                .map(|t| t.to_string_lossy().to_string())
                .unwrap_or_default();
            let child_rel = p.file_name()?.to_string_lossy().to_string();
            return Some(ParentLinkInfo {
                parent_path: parent.to_string_lossy().to_string(),
                parent_target: target,
                child_rel,
            });
        }
    }
    None
}

#[tauri::command]
pub fn get_pkg_cache_info(cmd: String) -> Result<PkgCacheInfo, String> {
    let cache_path = run_cmd_capture(cmd)?;
    // 过滤 Yarn 等工具返回的 "undefined" / "null" 字符串
    let trimmed = cache_path.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("undefined") || trimmed.eq_ignore_ascii_case("null") {
        return Err("缓存路径为空".to_string());
    }
    let cache_path = trimmed.to_string();

    let path = std::path::Path::new(&cache_path);
    let (is_link, real_target) = if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            let target = std::fs::read_link(path)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            (true, target)
        } else {
            (false, String::new())
        }
    } else {
        (false, String::new())
    };

    // 如果缓存路径本身不是链接，检查父目录是否是 junction（整体链接模式）
    let parent_link = if !is_link {
        check_parent_junction(&cache_path)
    } else {
        None
    };

    let size = crate::commands::cache::get_dir_size(path);
    let size_str = crate::commands::cache::format_bytes(size);

    Ok(PkgCacheInfo {
        path: cache_path,
        size: size_str,
        is_link,
        real_target,
        parent_link,
    })
}

/// 迁移缓存/数据 — 统一的存储迁移命令。
/// - storage_kind: "cache" | "data"（cache 可先删再迁、data 必须拷贝）
/// - delete_old_first: 是否先删除旧目录再建链接（仅 cache 类型生效）
/// - orig_path: 源路径（可选，不传则用 cache_detect_cmd 自动检测）
#[tauri::command]
pub fn migrate_pkg_storage(
    app_handle: tauri::AppHandle,
    cache_detect_cmd: String,
    new_path: String,
    storage_kind: String,
    delete_old_first: Option<bool>,
    orig_path: Option<String>,
) -> Result<(), String> {
    let src_path = if let Some(ref op) = orig_path {
        op.clone()
    } else {
        let p = run_cmd_capture(cache_detect_cmd)?;
        if p.is_empty() { return Err("未能检测到存储路径".to_string()); }
        p
    };
    let delete_old = delete_old_first.unwrap_or(false);
    crate::commands::cache::migrate_pkg_storage_impl(
        &app_handle, &src_path, &new_path, &storage_kind, delete_old,
    )
}

/// 指向模式下处理旧文件 — 在修改配置指向新路径前，处理旧目录中的文件。
/// - action: "move" 拷贝到新路径后删除旧文件，"delete" 直接删除旧文件，"keep" 不做操作。
#[tauri::command]
pub fn handle_point_storage_files(
    app_handle: tauri::AppHandle,
    old_path: String,
    new_path: String,
    action: String,
) -> Result<(), String> {
    if action == "keep" {
        return Ok(());
    }
    let old = std::path::Path::new(&old_path);
    if !old.exists() {
        return Ok(()); // 旧路径不存在，无需处理
    }
    if action == "delete" {
        std::fs::remove_dir_all(old).map_err(|e| format!("删除旧文件失败: {}", e))?;
        let _ = app_handle.emit("migrate-storage-progress", crate::commands::cache::MigrateStorageProgress {
            stage: "已删除旧文件".to_string(),
            current: 1,
            total: 1,
            file_name: String::new(),
        });
    } else if action == "move" {
        crate::commands::cache::copy_dir_all_with_progress(
            old, std::path::Path::new(&new_path), Some(&app_handle),
        ).map_err(|e| e.to_string())?;
        std::fs::remove_dir_all(old).map_err(|e| format!("删除旧文件失败: {}", e))?;
        let _ = app_handle.emit("migrate-storage-progress", crate::commands::cache::MigrateStorageProgress {
            stage: "已完成移动".to_string(),
            current: 1,
            total: 1,
            file_name: String::new(),
        });
    }
    Ok(())
}

/// 清理包管理器缓存 — 只清缓存文件，不跟随 junction，保留目录结构。
#[tauri::command]
pub fn clean_pkg_cache(
    app_handle: tauri::AppHandle,
    cache_detect_cmd: String,
    cache_path: Option<String>,
) -> Result<(), String> {
    let resolved = if let Some(ref cp) = cache_path {
        cp.clone()
    } else {
        let p = run_cmd_capture(cache_detect_cmd)?;
        if p.is_empty() { return Err("未能检测到缓存路径".to_string()); }
        p
    };
    crate::commands::cache::clean_pkg_cache_impl(&app_handle, &resolved)
}
