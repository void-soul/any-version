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
pub fn project_preview_manage(id: String, delegation: crate::commands::config::ProjectDelegation) -> Result<ManagePreview, String> {
    scanner::preview_manage(&id, delegation)
}

/// 托管项目（将项目纳入 AnyVersion 管理）
#[tauri::command]
pub fn project_manage(app: tauri::AppHandle, id: String, delegation: crate::commands::config::ProjectDelegation) -> Result<(), String> {
    use crate::commands::config::{load_config, save_config};
    use crate::commands::env::{get_registry_env_any, set_registry_env, add_to_user_path};
    use super::registry;
    use std::path::Path;

    let def = registry::find_by_id(&id)
        .ok_or_else(|| format!("未找到项目: {}", id))?;

    let mut config = load_config();

    // 1. 备份勾选的环境变量
    for var_def in &def.env_vars {
        if let Some(tier) = &var_def.tier {
            if *tier == super::types::EnvVarTier::Compat { continue; }
        }
        if delegation.env_vars.contains(&var_def.name) {
            if let Some((val, _)) = get_registry_env_any(&var_def.name) {
                if !val.to_lowercase().contains(&config.links_dir.to_lowercase()) {
                    config.original_envs.entry(var_def.name.clone()).or_insert(val);
                }
            }
        }
    }

    // 2. 处理 PATH 变量中勾选的目录
    if let Some(ref dirs) = def.bin_dirs {
        let has_path_delegated = dirs.iter().any(|d| delegation.path_vars.contains(d));

        if has_path_delegated {
            if let Some(user_path) = crate::commands::env::get_registry_env("PATH") {
                let parts = std::env::split_paths(&user_path)
                    .map(|p| p.to_string_lossy().to_string())
                    .collect::<Vec<_>>();

                let mut matched_entries = Vec::new();
                let mut remaining_entries = Vec::new();

                for p_str in parts {
                    if p_str.is_empty() { continue; }
                    let p_lower = p_str.to_lowercase();

                    if !p_lower.contains(&config.links_dir.to_lowercase()) {
                        let mut matches = false;
                        for rule in &def.find_rules {
                            match &rule.pattern {
                                super::types::ResolvePattern::PathContains { path_key, exe_name } => {
                                    if p_lower.contains(&path_key.to_lowercase()) {
                                        let mut exists = false;
                                        let mut check_names = vec![exe_name.clone()];
                                        #[cfg(windows)]
                                        {
                                            let exe_lower = exe_name.to_lowercase();
                                            if !exe_lower.ends_with(".exe") && !exe_lower.ends_with(".cmd") && !exe_lower.ends_with(".bat") {
                                                check_names.push(format!("{}.exe", exe_name));
                                                check_names.push(format!("{}.cmd", exe_name));
                                                check_names.push(format!("{}.bat", exe_name));
                                            }
                                        }
                                        for name in check_names {
                                            if Path::new(&p_str).join(&name).exists() {
                                                exists = true;
                                                break;
                                            }
                                        }
                                        if exists {
                                            matches = true;
                                            break;
                                        }
                                    }
                                }
                                super::types::ResolvePattern::FixedPath { path: fixed_path, exe_name } => {
                                    if p_lower.contains(&fixed_path.to_lowercase()) {
                                        let mut exists = false;
                                        let mut check_names = vec![exe_name.clone()];
                                        #[cfg(windows)]
                                        {
                                            let exe_lower = exe_name.to_lowercase();
                                            if !exe_lower.ends_with(".exe") && !exe_lower.ends_with(".cmd") && !exe_lower.ends_with(".bat") {
                                                check_names.push(format!("{}.exe", exe_name));
                                                check_names.push(format!("{}.cmd", exe_name));
                                                check_names.push(format!("{}.bat", exe_name));
                                            }
                                        }
                                        for name in check_names {
                                            if Path::new(&p_str).join(&name).exists() {
                                                exists = true;
                                                break;
                                            }
                                        }
                                        if exists {
                                            matches = true;
                                            break;
                                        }
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
        }
    }

    // 3. 保存配置
    config.managed_items.insert(id.clone());
    if !delegation.version_control {
        config.simple_managed_items.insert(id.clone());
    } else {
        config.simple_managed_items.remove(&id);
    }
    config.project_delegations.insert(id.clone(), delegation.clone());
    save_config(&config)?;

    // 4. 设置被勾选的项目环境变量
    let link_dir = Path::new(&config.links_dir).join(&id);
    let link_str = link_dir.to_string_lossy().to_string();

    for var_def in &def.env_vars {
        if let Some(tier) = &var_def.tier {
            if *tier == super::types::EnvVarTier::Compat { continue; }
            if *tier == super::types::EnvVarTier::Clear {
                if delegation.env_vars.contains(&var_def.name) {
                    let _ = set_registry_env(&var_def.name, "");
                }
                continue;
            }
        }
        if delegation.env_vars.contains(&var_def.name) {
            let value = if let Some(ref sub) = var_def.sub_dir {
                format!("{}\\{}", link_str, sub)
            } else {
                link_str.clone()
            };
            let _ = set_registry_env(&var_def.name, &value);
            std::env::set_var(&var_def.name, &value);
        }
    }

    // 5. 添加被勾选的 bin 路径到 PATH
    if let Some(ref dirs) = def.bin_dirs {
        let mut add_paths = Vec::new();
        for bin_dir in dirs {
            if delegation.path_vars.contains(bin_dir) {
                let path_val = if bin_dir.is_empty() {
                    link_str.clone()
                } else {
                    format!("{}\\{}", link_str, bin_dir)
                };
                add_paths.push(path_val);
            }
        }
        if !add_paths.is_empty() {
            let _ = add_to_user_path(&add_paths);
        }
    }

    // 6. 保存旧版数据备份
    save_manage_backup(&id, &def, &config);

    // 7. 同步当前进程环境 & 重建托盘
    crate::sync_process_path();
    let _ = crate::tray::rebuild_tray_menu(&app);

    Ok(())
}

/// 修复项目环境变量和 PATH，使其重新指向 AnyVersion 的稳定 links 路径。
#[tauri::command]
pub fn project_repair_env_vars(id: String) -> Result<(), String> {
    use std::path::Path;
    use crate::commands::config::load_config;
    use super::registry;

    let def = registry::find_by_id(&id)
        .ok_or_else(|| format!("未找到项目: {}", id))?;
    let config = load_config();

    if !config.managed_items.contains(id.as_str()) {
        return Err("该项目尚未托管".to_string());
    }
    let delegation = scanner::get_project_delegation(&config, &id, &def);
    if !delegation.version_control {
        return Err("简单托管项目不需要修复版本环境变量".to_string());
    }

    let link_dir = Path::new(&config.links_dir).join(&id);
    if !link_dir.exists() && !link_dir.is_symlink() {
        return Err("请先安装或启用一个版本，然后再修复环境变量".to_string());
    }

    let link_str = link_dir.to_string_lossy().to_string();
    let dest_str = std::fs::canonicalize(&link_dir)
        .map(|p| p.to_string_lossy().to_string().trim_start_matches(r"\\?\").to_string())
        .unwrap_or_else(|_| link_str.clone());

    crate::commands::env::configure_sdk_env_vars(&id, &link_str, &dest_str)?;
    crate::sync_process_path();
    Ok(())
}

/// 取消托管项目（从 AnyVersion 管理中还原并移除）
#[tauri::command]
pub fn project_unmanage(app: tauri::AppHandle, id: String) -> Result<(), String> {
    use crate::commands::config::{load_config, save_config};
    use crate::commands::env::{set_registry_env, remove_from_user_path, add_to_user_path};
    use super::registry;
    use std::path::Path;

    let def = registry::find_by_id(&id)
        .ok_or_else(|| format!("未找到项目: {}", id))?;

    let mut config = load_config();
    let delegation = scanner::get_project_delegation(&config, &id, &def);

    // 1. 移除被托管的项目环境变量
    for var_def in &def.env_vars {
        if let Some(tier) = &var_def.tier {
            if *tier == super::types::EnvVarTier::Compat { continue; }
        }
        if delegation.env_vars.contains(&var_def.name) {
            let _ = set_registry_env(&var_def.name, "");
        }
    }

    // 2. 从 PATH 中移除 AnyVersion 添加 of 条目
    let link_dir = Path::new(&config.links_dir).join(&id);
    let link_str = link_dir.to_string_lossy().to_string();

    if let Some(ref dirs) = def.bin_dirs {
        let mut remove_paths = Vec::new();
        for bin_dir in dirs {
            if delegation.path_vars.contains(bin_dir) {
                let path_val = if bin_dir.is_empty() {
                    link_str.clone()
                } else {
                    format!("{}\\{}", link_str, bin_dir)
                };
                remove_paths.push(path_val);
            }
        }
        if !remove_paths.is_empty() {
            let _ = remove_from_user_path(&remove_paths);
        }
    }

    // 3. 还原原始环境变量
    for var_def in &def.env_vars {
        if let Some(tier) = &var_def.tier {
            if *tier == super::types::EnvVarTier::Compat { continue; }
        }
        if delegation.env_vars.contains(&var_def.name) {
            if let Some(orig_val) = config.original_envs.remove(&var_def.name) {
                let _ = set_registry_env(&var_def.name, &orig_val);
                std::env::set_var(&var_def.name, &orig_val);
            } else {
                let _ = set_registry_env(&var_def.name, "");
                std::env::remove_var(&var_def.name);
            }
        }
    }

    // 4. 还原原始 PATH 条目
    if let Some(orig_paths) = config.original_paths.remove(&id) {
        let _ = add_to_user_path(&orig_paths);
    }

    // 5. 从 managed_items and project_delegations and active_versions 中移除
    config.managed_items.remove(&id);
    config.simple_managed_items.remove(&id);
    config.project_delegations.remove(&id);
    config.active_versions.remove(&id);
    save_config(&config)?;

    // 6. 同步进程环境并重建菜单
    crate::sync_process_path();
    let _ = crate::tray::rebuild_tray_menu(&app);

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

    let is_simple = def.simple_mode || config.simple_managed_items.contains(&id);

    if is_simple {
        let steps = vec![super::types::ManageStep {
            action: "simple_unmanage".to_string(),
            description: "取消托管后，AnyVersion 将不再管理该项目的缓存、代理和镜像配置".to_string(),
            target: id.clone(),
        }];
        return Ok(ManagePreview {
            steps,
            has_local_install: false,
            local_install_root: None,
            local_install_source: None,
        });
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

async fn install_pip_internal(project_id: &str) -> Result<String, String> {
    let config = crate::commands::config::load_config();
    let link_dir = std::path::PathBuf::from(&config.links_dir).join(project_id);
    if !link_dir.exists() {
        return Err("未找到 Python 链接目录，请先安装或启用 Python 版本。".to_string());
    }

    // 1. 尝试修改 ._pth 文件以启用 site-packages 导入
    let canonical_dir = std::fs::canonicalize(&link_dir)
        .map_err(|e| format!("无法解析 Python 路径: {}", e))?;
    
    let mut pth_file = None;
    if let Ok(entries) = std::fs::read_dir(&canonical_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_file() {
                let file_name = path.file_name().unwrap_or_default().to_string_lossy();
                if file_name.ends_with("._pth") || file_name.ends_with(".pth") {
                    pth_file = Some(path);
                    break;
                }
            }
        }
    }

    if let Some(pth_path) = pth_file {
        if let Ok(content) = std::fs::read_to_string(&pth_path) {
            let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
            let mut has_import_site = false;
            let mut modified = false;

            for line in &mut lines {
                let trimmed = line.trim();
                if trimmed == "import site" {
                    has_import_site = true;
                } else if trimmed == "#import site" || trimmed == "# import site" {
                    *line = "import site".to_string();
                    has_import_site = true;
                    modified = true;
                }
            }

            if !has_import_site {
                lines.push("import site".to_string());
                modified = true;
            }

            if modified {
                let new_content = lines.join("\r\n");
                let _ = std::fs::write(&pth_path, new_content);
            }
        }
    }

    // 2. 获取 Python 版本以匹配合适的 get-pip.py 下载链接
    let python_exe = link_dir.join("python.exe");
    if !python_exe.exists() {
        return Err("在链接目录中未找到 python.exe".to_string());
    }

    let ver_output = crate::commands::hidden_cmd::hidden_cmd(&python_exe)
        .args(&["-c", "import sys; print(f'{sys.version_info.major}.{sys.version_info.minor}')"])
        .current_dir(&canonical_dir)
        .output()
        .map_err(|e| format!("获取 Python 版本失败: {}", e))?;

    let py_ver = String::from_utf8_lossy(&ver_output.stdout).trim().to_string();
    let parts: Vec<&str> = py_ver.split('.').collect();
    let is_old = if parts.len() >= 2 {
        if let (Ok(major), Ok(minor)) = (parts[0].parse::<i32>(), parts[1].parse::<i32>()) {
            major < 3 || (major == 3 && minor < 10)
        } else {
            false
        }
    } else {
        false
    };

    let download_url = if is_old && !py_ver.is_empty() {
        format!("https://bootstrap.pypa.io/pip/{}/get-pip.py", py_ver)
    } else {
        "https://bootstrap.pypa.io/get-pip.py".to_string()
    };

    // 3. 下载 get-pip.py
    let temp_dir = std::env::temp_dir();
    let get_pip_path = temp_dir.join("get-pip.py");
    
    let client = reqwest::Client::builder()
        .user_agent("Any-Version-Manager")
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let res = client.get(&download_url)
        .send()
        .await
        .map_err(|e| format!("请求 get-pip.py 失败 (URL: {}): {}", download_url, e))?;

    if !res.status().is_success() {
        return Err(format!("下载 get-pip.py 返回错误状态码: {} (URL: {})", res.status(), download_url));
    }

    let bytes = res.bytes().await.map_err(|e| format!("读取 get-pip.py 内容失败: {}", e))?;
    std::fs::write(&get_pip_path, bytes).map_err(|e| format!("保存 get-pip.py 失败: {}", e))?;

    // 4. 执行 python.exe get-pip.py
    let output = crate::commands::hidden_cmd::hidden_cmd(&python_exe)
        .arg(&get_pip_path)
        .current_dir(&canonical_dir)
        .output()
        .map_err(|e| {
            let _ = std::fs::remove_file(&get_pip_path);
            format!("执行 get-pip.py 失败: {}", e)
        })?;

    // 5. 清理 get-pip.py
    let _ = std::fs::remove_file(&get_pip_path);

    if output.status.success() {
        Ok("pip 安装成功！".to_string())
    } else {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(format!("安装 pip 失败!\nStdout: {}\nStderr: {}", stdout, stderr))
    }
}

fn resolve_custom_exe(cmd: &str, config: &crate::commands::config::Config) -> String {
    let mut parts = cmd.split_whitespace();
    if let Some(first_word) = parts.next() {
        let registry = super::registry::registry();
        for proj in &registry {
            let matches_proj = proj.id == first_word 
                || proj.version_exe.as_ref().map(|s| s.as_str()) == Some(first_word);
            if matches_proj {
                if let Some(custom_path) = config.custom_install_paths.get(&proj.id) {
                    let path = std::path::Path::new(custom_path);
                    let exe = proj.version_exe.as_ref().map(|s| s.as_str()).unwrap_or(&proj.id);
                    let mut candidates = Vec::new();
                    candidates.push(path.join(exe));
                    candidates.push(path.join("bin").join(exe));
                    #[cfg(windows)]
                    {
                        let exe_lower = exe.to_lowercase();
                        if !exe_lower.ends_with(".exe") && !exe_lower.ends_with(".cmd") && !exe_lower.ends_with(".bat") {
                            candidates.push(path.join(format!("{}.exe", exe)));
                            candidates.push(path.join(format!("{}.cmd", exe)));
                            candidates.push(path.join(format!("{}.bat", exe)));
                            candidates.push(path.join("bin").join(format!("{}.exe", exe)));
                            candidates.push(path.join("bin").join(format!("{}.cmd", exe)));
                            candidates.push(path.join("bin").join(format!("{}.bat", exe)));
                        }
                    }
                    for candidate in candidates {
                        if candidate.exists() {
                            let resolved_first = format!("\"{}\"", candidate.to_string_lossy());
                            let rest = &cmd[first_word.len()..];
                            return format!("{}{}", resolved_first, rest);
                        }
                    }
                }
            }
        }
    }
    cmd.to_string()
}

/// 执行 shell 命令并捕获输出（用于包管理器版本检测、镜像切换等）
#[tauri::command]
pub fn run_cmd_capture(cmd: String, project_id: Option<String>) -> Result<String, String> {
    if cmd == "install_pip" {
        return tauri::async_runtime::block_on(async {
            install_pip_internal(project_id.as_deref().unwrap_or("python")).await
        });
    }

    let config = crate::commands::config::load_config();
    let mut resolved_cmd = resolve_custom_exe(&cmd, &config);

    // Generic resolution of package manager command execution (like pip run via python)
    let registry = super::registry::registry();
    for proj in &registry {
        for pm in &proj.package_managers {
            if let Some(ref run_args) = pm.run_via_runtime_args {
                let pm_prefix = format!("{} ", pm.id);
                if resolved_cmd == pm.id || resolved_cmd.starts_with(&pm_prefix) {
                    let link_dir = std::path::PathBuf::from(&config.links_dir).join(&proj.id);
                    let runtime_exe_name = proj.version_exe.as_deref().unwrap_or(&proj.id);
                    let active_runtime = if cfg!(windows) {
                        if !runtime_exe_name.ends_with(".exe") {
                            link_dir.join(format!("{}.exe", runtime_exe_name))
                        } else {
                            link_dir.join(runtime_exe_name)
                        }
                    } else {
                        link_dir.join(runtime_exe_name)
                    };
                    if active_runtime.exists() {
                        let pm_run_str = format!(
                            "\"{}\" {}",
                            active_runtime.to_string_lossy(),
                            run_args.join(" ")
                        );
                        if resolved_cmd == pm.id {
                            resolved_cmd = pm_run_str;
                        } else {
                            resolved_cmd = resolved_cmd.replacen(&pm.id, &pm_run_str, 1);
                        }
                    }
                }
            }
        }
    }

    // PowerShell 命令直接调用 powershell.exe，避免 cmd /c 的引号嵌套问题
    let is_powershell = cfg!(windows) && {
        let lower = resolved_cmd.trim_start().to_lowercase();
        lower.starts_with("powershell ") || lower.starts_with("powershell.exe ")
    };

    let mut command = if is_powershell {
        // 解析 "powershell -Command ..." 结构，提取脚本内容
        // 直接调用 powershell.exe，脚本作为 -Command 的参数传递
        let trimmed = resolved_cmd.trim_start();
        let after_exe = if trimmed.to_lowercase().starts_with("powershell.exe ") {
            trimmed["powershell.exe ".len()..].trim_start()
        } else {
            trimmed["powershell ".len()..].trim_start()
        };
        // 提取 -Command 后面的脚本
        let script = if after_exe.starts_with("-Command ") {
            after_exe["-Command ".len()..].trim()
        } else if after_exe.starts_with("-command ") {
            after_exe["-command ".len()..].trim()
        } else {
            after_exe
        };
        let mut c = super::super::hidden_cmd::hidden_cmd("powershell.exe");
        c.args(&["-Command", script]);
        c
    } else {
        let mut c = super::super::hidden_cmd::hidden_cmd("cmd");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.raw_arg(&format!("/c \"chcp 65001 >nul && {}\"", resolved_cmd));
        }
        #[cfg(not(windows))]
        {
            c.args(&["/c", &resolved_cmd]);
        }
        c
    };

    // 注入 NPM_CONFIG_PREFIX，确保 npm install -g 安装到正确的 link_dir
    // （当前进程环境变量可能未同步注册表，这里兜底保证子进程用对 prefix）
    if let Ok(prefix) = std::env::var("NPM_CONFIG_PREFIX") {
        command.env("NPM_CONFIG_PREFIX", &prefix);
    }

    let output = command
        .output()
        .map_err(|e| format!("执行命令失败: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().trim_matches('"').trim_matches('\'').trim().to_string();
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

fn resolve_pkg_storage_path_dynamic(
    project_id: &str,
    pm_id: &str,
    storage_kind: &str,
) -> Result<String, String> {
    use crate::commands::project::registry;
    use crate::commands::utils::{expand_home, resolve_custom_cache_path};

    let project = registry::registry().into_iter()
        .find(|p| p.id.eq_ignore_ascii_case(project_id))
        .ok_or_else(|| format!("未找到项目: {}", project_id))?;

    let pm = project.package_managers.iter()
        .find(|m| m.id.eq_ignore_ascii_case(pm_id))
        .ok_or_else(|| format!("在项目 {} 中未找到包管理器: {}", project_id, pm_id))?;

    let mut resolved_path = String::new();

    if storage_kind == "cache" {
        resolved_path = resolve_custom_cache_path(pm).unwrap_or_default();
        if resolved_path.is_empty() {
            if let Some(ref cmd) = pm.cache_detect_cmd {
                if let Ok(out) = run_cmd_capture(cmd.clone(), Some(project_id.to_string())) {
                    let trimmed = out.trim();
                    if !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("undefined") && !trimmed.eq_ignore_ascii_case("null") {
                        resolved_path = trimmed.to_string();
                    }
                }
            }
        }
        if resolved_path.is_empty() {
            if let Some(ref env_var) = pm.cache_env_var {
                if let Ok(val) = std::env::var(env_var) {
                    if !val.trim().is_empty() {
                        resolved_path = val.trim().to_string();
                    }
                }
            }
        }
        if resolved_path.is_empty() {
            if let Some(ref default_path) = pm.cache_default_path {
                resolved_path = expand_home(default_path);
            }
        }
    } else if storage_kind == "data" {
        if let Some(ref cmd) = pm.data_detect_cmd {
            if let Ok(out) = run_cmd_capture(cmd.clone(), Some(project_id.to_string())) {
                let trimmed = out.trim();
                if !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("undefined") && !trimmed.eq_ignore_ascii_case("null") {
                    resolved_path = trimmed.to_string();
                }
            }
        }
        if resolved_path.is_empty() {
            if let Some(ref env_var) = pm.data_env_var {
                if let Ok(val) = std::env::var(env_var) {
                    if !val.trim().is_empty() {
                        resolved_path = val.trim().to_string();
                    }
                }
            }
        }
        if resolved_path.is_empty() {
            if let Some(ref default_path) = pm.data_default_path {
                resolved_path = expand_home(default_path);
            }
        }
    } else {
        return Err(format!("不支持的存储类型: {}", storage_kind));
    }

    if resolved_path.is_empty() {
        return Err(format!("未能检测到 {} 存储路径", storage_kind));
    }

    let cleaned = resolved_path.trim_matches('"').trim_matches('\'').trim().to_string();
    if cleaned.is_empty() {
        return Err(format!("未能检测到 {} 存储路径", storage_kind));
    }

    Ok(cleaned)
}

#[tauri::command]
pub fn get_pkg_cache_info(
    project_id: String,
    pm_id: String,
    storage_kind: String,
) -> Result<PkgCacheInfo, String> {
    let cache_path = resolve_pkg_storage_path_dynamic(&project_id, &pm_id, &storage_kind)?;
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
    project_id: String,
    pm_id: String,
    new_path: String,
    storage_kind: String,
    delete_old_first: Option<bool>,
    orig_path: Option<String>,
) -> Result<(), String> {
    let src_path = if let Some(ref op) = orig_path {
        op.clone()
    } else {
        resolve_pkg_storage_path_dynamic(&project_id, &pm_id, &storage_kind)?
    };
    let delete_old = delete_old_first.unwrap_or(false);
    crate::commands::cache::migrate_pkg_storage_impl(
        &app_handle, &src_path, &new_path, &storage_kind, delete_old,
    )?;

    // 迁移完成后，同步更新配置文件中的缓存路径（如 NuGet.Config）
    if storage_kind == "cache" {
        if let Some(def) = super::registry::find_by_id(&project_id) {
            if let Some(pm) = def.package_managers.iter().find(|p| p.id == pm_id) {
                crate::commands::utils::apply_cache_config_writes(pm, &new_path);
            }
        }
    }

    Ok(())
}

/// 指向模式下设置缓存目录。
/// 如果包管理器配有 cache_env_var 且检测到该环境变量已被配置：
/// - 若存在设置命令（例如 go env -w）且旧环境变量在 HKCU（用户级），则主动在注册表及进程中将其清空，使命令托管生效；
fn set_maven_local_repository(new_path: &str) -> Result<(), String> {
    use crate::commands::utils::expand_home;
    let config = crate::commands::config::load_config();
    let user_home = expand_home("{home}");
    let links_dir = config.links_dir;
    let maven_home = std::env::var("MAVEN_HOME").unwrap_or_default();

    let paths = vec![
        std::path::PathBuf::from(&user_home).join(".m2").join("settings.xml"),
        std::path::PathBuf::from(&maven_home).join("conf").join("settings.xml"),
        std::path::PathBuf::from(&links_dir).join("maven").join("conf").join("settings.xml"),
    ];

    let mut target_path = None;
    for path in &paths {
        if path.exists() {
            target_path = Some(path.clone());
            break;
        }
    }

    // Default to {home}/.m2/settings.xml if none exists
    let settings_path = target_path.unwrap_or_else(|| {
        let p = std::path::PathBuf::from(&user_home).join(".m2").join("settings.xml");
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        p
    });

    if settings_path.exists() {
        let content = std::fs::read_to_string(&settings_path)
            .map_err(|e| format!("无法读取 settings.xml: {}", e))?;

        let re_repo = regex::Regex::new(r"(?s)<localRepository>.*?</localRepository>")
            .map_err(|e| e.to_string())?;

        let new_tag = format!("<localRepository>{}</localRepository>", new_path);

        let new_content = if re_repo.is_match(&content) {
            // Replace existing localRepository tag
            re_repo.replace(&content, new_tag.as_str()).to_string()
        } else {
            // Find <settings> tag and insert it after
            let re_settings = regex::Regex::new(r"(?i)<settings\b[^>]*>")
                .map_err(|e| e.to_string())?;

            if let Some(mat) = re_settings.find(&content) {
                let insert_idx = mat.end();
                let mut c = content.clone();
                c.insert_str(insert_idx, &format!("\n  {}", new_tag));
                c
            } else {
                return Err("未在 settings.xml 中找到 <settings> 标签。".to_string());
            }
        };

        std::fs::write(&settings_path, new_content)
            .map_err(|e| format!("写入 settings.xml 失败: {}", e))?;
    } else {
        // Create a new settings.xml with the localRepository
        let new_content = format!(
            r#"<settings xmlns="http://maven.apache.org/SETTINGS/1.0.0"
          xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
          xsi:schemaLocation="http://maven.apache.org/SETTINGS/1.0.0 https://maven.apache.org/xsd/settings-1.0.0.xsd">
  <localRepository>{}</localRepository>
</settings>"#,
            new_path
        );
        std::fs::write(&settings_path, new_content)
            .map_err(|e| format!("创建 settings.xml 失败: {}", e))?;
    }

    Ok(())
}

/// - 若无设置命令或旧环境变量在 HKLM（系统级，无法删除），则通过在 HKCU 中写入新路径来实现优先级覆盖；
/// - 若执行命令失败且之前删除了 HKCU 环境变量，则进行恢复/兜底设置。
#[tauri::command]
pub fn project_set_cache_path(
    project_id: String,
    pm_id: String,
    new_path: String,
) -> Result<(), String> {
    if pm_id == "maven" {
        return set_maven_local_repository(&new_path);
    }

    use crate::commands::env::{set_registry_env, get_registry_env_any};
    use super::registry;

    let def = registry::find_by_id(&project_id)
        .ok_or_else(|| format!("未找到项目: {}", project_id))?;

    let pm = def.package_managers.iter().find(|p| p.id == pm_id)
        .ok_or_else(|| format!("未找到包管理器: {}", pm_id))?;

    let mut env_var_updated = false;
    let mut hkcu_deleted = false;
    let mut deleted_var_name = String::new();

    if let Some(ref env_var) = pm.cache_env_var {
        if let Some((_, source)) = get_registry_env_any(env_var) {
            if pm.cache_set_cmd_template.is_none() {
                // 没有命令行设置方式，必须修改注册表环境变量
                set_registry_env(env_var, &new_path)?;
                std::env::set_var(env_var, &new_path);
                env_var_updated = true;
            } else if source == "HKLM" {
                // 如果是系统级环境变量，无法删除，只能在 HKCU 中写入新值以覆盖它
                set_registry_env(env_var, &new_path)?;
                std::env::set_var(env_var, &new_path);
                env_var_updated = true;
            } else {
                // 如果是用户级环境变量 (HKCU)，且有命令可以设置（如 go env -w）
                // 我们优先将其从 HKCU 中删除，以便改为由 commands (go env -w) 托管
                set_registry_env(env_var, "")?;
                std::env::remove_var(env_var);
                hkcu_deleted = true;
                deleted_var_name = env_var.clone();
            }
        } else {
            // 如果注册表中完全没有配置该环境变量
            if pm.cache_set_cmd_template.is_none() {
                // 如果没有命令行模板，则直接通过注册表环境变量来设置
                set_registry_env(env_var, &new_path)?;
                std::env::set_var(env_var, &new_path);
                env_var_updated = true;
            }
        }
    }

    if let Some(ref tpl) = pm.cache_set_cmd_template {
        let cmd = tpl.replace("{path}", &new_path);
        if let Err(e) = run_cmd_capture(cmd, Some(project_id.clone())) {
            // 如果命令执行失败
            if hkcu_deleted {
                // 如果之前删除了 HKCU 环境变量，且命令执行失败，则需要恢复/设置 HKCU 环境变量作为兜底
                let _ = set_registry_env(&deleted_var_name, &new_path);
                std::env::set_var(&deleted_var_name, &new_path);
            } else if !env_var_updated {
                // 如果既没删除也没修改过环境变量，则如果有环境变量名，降级/兜底通过修改注册表环境变量来设置它
                //（例如：在 Go 1.13 以下不支持 go env -w 时，直接改环境变量是唯一的设置方式）
                if let Some(ref env_var) = pm.cache_env_var {
                    set_registry_env(env_var, &new_path)?;
                    std::env::set_var(env_var, &new_path);
                } else {
                    return Err(e);
                }
            }
        }
    }

    // 设置完成后，同步更新配置文件中的缓存路径（如 NuGet.Config）
    crate::commands::utils::apply_cache_config_writes(pm, &new_path);

    Ok(())
}


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
    project_id: String,
    pm_id: String,
    cache_path: Option<String>,
) -> Result<(), String> {
    let resolved = if let Some(ref cp) = cache_path {
        cp.clone()
    } else {
        resolve_pkg_storage_path_dynamic(&project_id, &pm_id, "cache")?
    };
    crate::commands::cache::clean_pkg_cache_impl(&app_handle, &resolved)
}

#[tauri::command]
pub fn migrate_data_dir(
    app_handle: tauri::AppHandle,
    project_id: String,
    orig_path: String,
    new_path: String,
) -> Result<(), String> {
    use std::fs;
    use std::path::Path;

    // 1. 确保服务不在运行中
    let status = scanner::get_project_status(&project_id)?;
    if let Some(svc_status) = status.service_status {
        if svc_status.running {
            return Err("服务正在运行中，请先停止服务后再进行数据迁移。".to_string());
        }
    }

    let orig = Path::new(&orig_path);
    let target = Path::new(&new_path);

    if orig == target {
        return Err("原路径与目标路径相同，无需迁移".to_string());
    }
    if !orig.exists() {
        return Err("源路径不存在".to_string());
    }

    // Ensure target parent folder exists
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("无法创建目标父目录: {}", e))?;
    }

    let is_symlink = fs::symlink_metadata(orig).map(|m| m.file_type().is_symlink()).unwrap_or(false);

    if is_symlink {
        // Just remove old junction link
        fs::remove_dir(orig).map_err(|e| format!("无法移除已有的旧链接: {}", e))?;
    } else {
        // Copy directory safely (database data files: delete_old_first = false)
        // We use migrate_pkg_storage_impl with delete_old_first=false, storage_kind="data"
        crate::commands::cache::migrate_pkg_storage_impl(
            &app_handle,
            &orig_path,
            &new_path,
            "data",
            false,
        )?;
    }

    // Create Junction
    crate::commands::cache::create_junction(orig, target)?;

    Ok(())
}

#[tauri::command]
pub fn delete_data_dir(project_id: String, path: String) -> Result<(), String> {
    use std::fs;
    use std::path::Path;

    // 1. 确保服务不在运行中
    let status = scanner::get_project_status(&project_id)?;
    if let Some(svc_status) = status.service_status {
        if svc_status.running {
            return Err("服务正在运行中，请先停止服务后再删除其数据目录。".to_string());
        }
    }

    let p = Path::new(&path);
    if !p.exists() {
        return Err("路径不存在".to_string());
    }

    let is_symlink = fs::symlink_metadata(p).map(|m| m.file_type().is_symlink()).unwrap_or(false);

    if is_symlink {
        if let Ok(target) = fs::read_link(p) {
            let _ = fs::remove_dir(p);
            if target.exists() {
                fs::remove_dir_all(&target).map_err(|e| format!("删除链接目标目录失败: {}", e))?;
            }
        } else {
            fs::remove_dir(p).map_err(|e| format!("删除链接失败: {}", e))?;
        }
    } else {
        fs::remove_dir_all(p).map_err(|e| format!("删除数据目录失败: {}", e))?;
    }

    Ok(())
}

#[tauri::command]
pub fn project_set_data_dir(project_id: String, dir_id: String, new_path: String) -> Result<(), String> {
    use crate::commands::config::{load_config, save_config};
    use super::registry;

    let def = registry::find_by_id(&project_id)
        .ok_or_else(|| format!("未找到项目: {}", project_id))?;
    if !def.data_dirs.iter().any(|d| d.id == dir_id) {
        return Err(format!("项目 {} 未定义数据目录: {}", project_id, dir_id));
    }

    let status = scanner::get_project_status(&project_id)?;
    if let Some(svc_status) = status.service_status {
        if svc_status.running {
            return Err("服务正在运行中，请先停止服务后再修改数据/日志路径。".to_string());
        }
    }

    let mut config = load_config();
    if new_path.trim().is_empty() {
        if let Some(paths) = config.custom_data_paths.get_mut(&project_id) {
            paths.remove(&dir_id);
            if paths.is_empty() {
                config.custom_data_paths.remove(&project_id);
            }
        }
    } else {
        let p = std::path::Path::new(&new_path);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("无法创建父目录: {}", e))?;
        }
        config
            .custom_data_paths
            .entry(project_id)
            .or_default()
            .insert(dir_id, new_path);
    }

    save_config(&config)
}
#[tauri::command]
pub fn project_set_custom_path(id: String, path: String) -> Result<(), String> {
    use crate::commands::config::{load_config, save_config};
    let mut config = load_config();
    if path.is_empty() {
        config.custom_install_paths.remove(&id);
    } else {
        let p = std::path::Path::new(&path);
        if !p.exists() {
            return Err("指定的路径不存在".to_string());
        }
        config.custom_install_paths.insert(id, path);
    }
    save_config(&config)?;
    Ok(())
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct GitRepoStatus {
    pub is_git: bool,
    pub has_exe: bool,
    pub has_update: bool,
    pub current_commit: String,
    pub latest_commit: String,
}

fn run_git_cmd(dir: &std::path::Path, args: &[&str]) -> Result<String, String> {
    let mut cmd = std::process::Command::new("git");
    cmd.current_dir(dir);
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let output = cmd.output().map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

#[tauri::command]
pub fn check_git_repo_status(path: String, exe_name: String, bootstrap_cmd: Option<String>) -> Result<GitRepoStatus, String> {
    let path_buf = std::path::Path::new(&path);
    if !path_buf.exists() {
        return Ok(GitRepoStatus {
            is_git: false,
            has_exe: false,
            has_update: false,
            current_commit: String::new(),
            latest_commit: String::new(),
        });
    }

    let has_bootstrap = if let Some(ref b_cmd) = bootstrap_cmd {
        path_buf.join(b_cmd).exists() || {
            if b_cmd.ends_with(".bat") || b_cmd.ends_with(".cmd") {
                let sh_version = b_cmd.replace(".bat", ".sh").replace(".cmd", ".sh");
                path_buf.join(sh_version).exists()
            } else {
                false
            }
        }
    } else {
        false
    };

    let is_git = path_buf.join(".git").exists() && (has_bootstrap || bootstrap_cmd.is_none());

    let full_exe_name = if cfg!(windows) {
        let lower = exe_name.to_lowercase();
        if !lower.ends_with(".exe") && !lower.ends_with(".cmd") && !lower.ends_with(".bat") {
            format!("{}.exe", exe_name)
        } else {
            exe_name.clone()
        }
    } else {
        exe_name.clone()
    };
    let has_exe = path_buf.join(&full_exe_name).exists() || path_buf.join("bin").join(&full_exe_name).exists();

    let mut has_update = false;
    let mut current_commit = String::new();
    let mut latest_commit = String::new();

    if is_git {
        if let Ok(curr) = run_git_cmd(path_buf, &["rev-parse", "--short", "HEAD"]) {
            current_commit = curr;
        }

        let _ = run_git_cmd(path_buf, &["fetch"]);

        let upstream = run_git_cmd(path_buf, &["rev-parse", "--short", "@{u}"])
            .or_else(|_| run_git_cmd(path_buf, &["rev-parse", "--short", "origin/master"]))
            .or_else(|_| run_git_cmd(path_buf, &["rev-parse", "--short", "origin/main"]));

        if let Ok(up) = upstream {
            latest_commit = up.clone();
            if !current_commit.is_empty() && current_commit != latest_commit {
                if let Ok(merge_base) = run_git_cmd(path_buf, &["merge-base", "HEAD", &latest_commit]) {
                    if merge_base == current_commit {
                        has_update = true;
                    }
                }
            }
        }
    }

    Ok(GitRepoStatus {
        is_git,
        has_exe,
        has_update,
        current_commit,
        latest_commit,
    })
}

#[tauri::command]
pub fn bootstrap_git_repo(path: String, cmd: String) -> Result<(), String> {
    let dir = std::path::Path::new(&path);
    if !dir.exists() {
        return Err("目录不存在".to_string());
    }

    let cmd_to_run = if cfg!(windows) {
        cmd.clone()
    } else {
        if cmd.ends_with(".bat") || cmd.ends_with(".cmd") {
            let sh_version = cmd.replace(".bat", ".sh").replace(".cmd", ".sh");
            if dir.join(&sh_version).exists() {
                format!("./{}", sh_version)
            } else {
                cmd.clone()
            }
        } else {
            cmd.clone()
        }
    };

    let mut process_cmd = if cfg!(windows) {
        let mut c = std::process::Command::new("cmd");
        c.current_dir(dir);
        c.args(&["/c", &cmd_to_run]);
        c
    } else {
        let mut c = std::process::Command::new("sh");
        c.current_dir(dir);
        if cmd_to_run.starts_with("./") {
            c.args(&[&cmd_to_run[2..]]);
        } else {
            c.args(&[&cmd_to_run]);
        }
        c
    };

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        process_cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let output = process_cmd.output().map_err(|e| format!("执行初始化脚本失败: {}", e))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Err(format!("初始化脚本执行失败!\nStderr: {}\nStdout: {}", stderr, stdout))
    }
}

#[tauri::command]
pub fn update_git_repo(path: String, bootstrap_cmd: Option<String>) -> Result<(), String> {
    let dir = std::path::Path::new(&path);
    if !dir.exists() {
        return Err("目录不存在".to_string());
    }

    let mut pull_cmd = std::process::Command::new("git");
    pull_cmd.current_dir(dir);
    pull_cmd.args(&["pull"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        pull_cmd.creation_flags(0x08000000);
    }
    let pull_output = pull_cmd.output().map_err(|e| format!("git pull 失败: {}", e))?;
    if !pull_output.status.success() {
        let err = String::from_utf8_lossy(&pull_output.stderr).trim().to_string();
        return Err(format!("Git pull 失败: {}", err));
    }

    if let Some(ref b_cmd) = bootstrap_cmd {
        bootstrap_git_repo(path, b_cmd.clone())
    } else {
        Ok(())
    }
}
