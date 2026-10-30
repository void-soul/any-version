use super::*;

/// 获取所有项目列表及运行时状态
#[tauri::command]
pub async fn project_list() -> Result<Vec<ProjectStatus>, String> {
    tokio::task::spawn_blocking(|| scanner::list_projects())
        .await
        .map_err(|e| format!("任务执行失败: {}", e))?
}

/// 快速获取项目列表（跳过缓存大小计算，用于前端初次加载）
#[tauri::command]
pub async fn project_list_fast() -> Result<Vec<ProjectStatus>, String> {
    tokio::task::spawn_blocking(|| scanner::list_projects_fast())
        .await
        .map_err(|e| format!("任务执行失败: {}", e))?
}

/// 获取单个项目运行时状态
#[tauri::command]
pub async fn project_status(id: String) -> Result<ProjectStatus, String> {
    tokio::task::spawn_blocking(move || scanner::get_project_status(&id))
        .await
        .map_err(|e| format!("任务执行失败: {}", e))?
}

/// 获取项目详情（定义 + 状态）
#[tauri::command]
pub async fn project_detail(id: String) -> Result<ProjectDetail, String> {
    tokio::task::spawn_blocking(move || scanner::get_project_detail(&id))
        .await
        .map_err(|e| format!("任务执行失败: {}", e))?
}

/// 预览托管操作步骤
#[tauri::command]
pub fn project_preview_manage(
    id: String,
    delegation: crate::commands::config::ProjectDelegation,
) -> Result<ManagePreview, String> {
    scanner::preview_manage(&id, delegation)
}

/// PATH 里的一条目录是不是「这个项目装在别处的副本」—— 是的话托管时会把它摘掉。
///
/// 清理是**破坏性**操作（改的是用户的 PATH），所以这里刻意保守，两种情况一律不算：
/// - **MSIX 型**（如 WinGet）：它的 exe 在 WindowsApps（应用执行别名），而它的
///   find_rules 正好命中那个目录 —— 摘掉会连所有应用执行别名（winget、Store 版
///   python…）一起废掉；
/// - **`path_key` 为空的规则**：`contains("")` 恒为真，等于「PATH 里任何目录只要有
///   这个 exe 就删」。宽匹配规则不该触发删除。
///
/// 其余情况必须在**这个目录里真的找到**可执行文件才算（避免匹配到同名无关目录）。
pub(super) fn is_foreign_path_entry(
    entry: &str,
    def: &crate::commands::project::types::ProjectDef,
    links_dir: &str,
) -> bool {
    if def.is_msix() {
        return false;
    }
    let entry_lower = entry.to_lowercase();
    // 已经是我们自己的 links 目录：那是托管路径，不是「外部副本」
    if !links_dir.is_empty() && entry_lower.contains(&links_dir.to_lowercase()) {
        return false;
    }

    for rule in &def.find_rules {
        let exe_name = match &rule.pattern {
            crate::commands::project::types::ResolvePattern::PathContains {
                path_key,
                exe_name,
            } => {
                if path_key.is_empty() {
                    continue;
                }
                if !entry_lower.contains(&path_key.to_lowercase()) {
                    continue;
                }
                exe_name
            }
            crate::commands::project::types::ResolvePattern::FixedPath { path, exe_name } => {
                if !entry_lower.contains(&path.to_lowercase()) {
                    continue;
                }
                exe_name
            }
            _ => continue,
        };

        let mut names = vec![exe_name.clone()];
        #[cfg(windows)]
        {
            let lower = exe_name.to_lowercase();
            if !lower.ends_with(".exe") && !lower.ends_with(".cmd") && !lower.ends_with(".bat") {
                names.push(format!("{exe_name}.exe"));
                names.push(format!("{exe_name}.cmd"));
                names.push(format!("{exe_name}.bat"));
            }
        }
        if names
            .iter()
            .any(|name| std::path::Path::new(entry).join(name).exists())
        {
            return true;
        }
    }
    false
}

/// 托管项目（将项目纳入 AnyVersion 管理）
#[tauri::command]
pub fn project_manage(
    app: tauri::AppHandle,
    id: String,
    delegation: crate::commands::config::ProjectDelegation,
) -> Result<(), String> {
    use crate::commands::config::{load_config, save_config};
    use crate::commands::env::{
        add_to_system_path, add_to_user_path, get_registry_env_any, set_registry_env,
    };
    use crate::commands::project::registry;
    use std::path::Path;

    let def = registry::find_by_id(&id).ok_or_else(|| format!("未找到项目: {}", id))?;

    let mut config = load_config();

    // 1. 备份勾选的环境变量
    for var_def in &def.env_vars {
        if let Some(tier) = &var_def.tier {
            if *tier == crate::commands::project::types::EnvVarTier::Compat {
                continue;
            }
        }
        if delegation.env_vars.contains(&var_def.name) {
            if let Some((val, _)) = get_registry_env_any(&var_def.name) {
                if !val
                    .to_lowercase()
                    .contains(&config.links_dir.to_lowercase())
                {
                    config
                        .original_envs
                        .entry(id.clone())
                        .or_default()
                        .entry(var_def.name.clone())
                        .or_insert(val);
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
                    if p_str.is_empty() {
                        continue;
                    }
                    if is_foreign_path_entry(&p_str, &def, &config.links_dir) {
                        matched_entries.push(p_str.clone());
                    } else {
                        remaining_entries.push(p_str);
                    }
                }

                if !matched_entries.is_empty() {
                    config
                        .original_paths
                        .entry(id.clone())
                        .or_insert_with(Vec::new)
                        .extend(matched_entries);
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
    config
        .project_delegations
        .insert(id.clone(), delegation.clone());
    save_config(&config)?;

    // 4. 设置被勾选的项目环境变量
    let link_dir = Path::new(&config.links_dir).join(&id);
    let link_str = link_dir.to_string_lossy().to_string();

    for var_def in &def.env_vars {
        if let Some(tier) = &var_def.tier {
            if *tier == crate::commands::project::types::EnvVarTier::Compat {
                continue;
            }
            if *tier == crate::commands::project::types::EnvVarTier::Clear {
                if delegation.env_vars.contains(&var_def.name) {
                    let _ = set_registry_env(&var_def.name, "");
                }
                continue;
            }
        }
        if delegation.env_vars.contains(&var_def.name) {
            let value = crate::commands::env::sdk_env_var_value(&id, &link_str, var_def);
            let _ = set_registry_env(&var_def.name, &value);
            std::env::set_var(&var_def.name, &value);
        }
    }

    // 5. 添加被勾选的 bin 路径到 PATH
    //
    //    MSIX 型（如 WinGet）**不写**：它是系统级安装，不做 junction，
    //    `links_dir/<id>` 永远不会存在 —— 写进去就是一条永远解析不到的条目。
    //    它的可执行文件由 Windows 的应用执行别名（WindowsApps）提供，本来就不需要 PATH。
    if !def.is_msix() {
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
                if def.requires_system_path {
                    // 构建工具（如 Flutter 启动的 cmake 子进程）只读取系统级 PATH 视图，
                    // 必须写入 HKLM PATH 才能被 find_program 找到。需要管理员权限。
                    add_to_system_path(&add_paths).map_err(|e| {
                        format!(
                            "已将 {} 添加到用户级 PATH，但写入系统级 PATH 失败（{}）。\n请「以管理员身份运行」Kira 后重新托管该项目，否则 Flutter/cmake 等构建子进程将无法找到其可执行文件。",
                            id, e
                        )
                    })?;
                } else {
                    let _ = add_to_user_path(&add_paths);
                }
            }
        }
    }

    // 5.5 安装根布局（scoop 这类把「本体位置」写死在目录结构里的工具：
    //     `<root>\apps\<id>\current` junction + `<root>\shims\<name>.cmd` 包装）。
    //     还没装版本时内部会跳过，安装完成后再补一次（见 versions::do_install）。
    if let Err(e) = crate::commands::project::layout::apply(&def, &link_dir) {
        eprintln!("[manage] 补齐 {} 的安装根布局失败: {}", id, e);
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
    use crate::commands::config::load_config;
    use crate::commands::project::registry;
    use std::path::Path;

    let def = registry::find_by_id(&id).ok_or_else(|| format!("未找到项目: {}", id))?;
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
        .map(|p| {
            p.to_string_lossy()
                .to_string()
                .trim_start_matches(r"\\?\")
                .to_string()
        })
        .unwrap_or_else(|_| link_str.clone());

    crate::commands::env::configure_sdk_env_vars(&id, &link_str, &dest_str)?;
    // 修复环境变量后同样要保证安装根布局存在（scoop 的 apps\scoop\current / shims 包装）。
    if let Err(e) = crate::commands::project::layout::apply(&def, &link_dir) {
        eprintln!("[repair] 补齐 {} 的安装根布局失败: {}", id, e);
    }
    crate::sync_process_path();
    Ok(())
}

/// 取消托管项目（从 AnyVersion 管理中还原并移除）
#[tauri::command]
pub fn project_unmanage(app: tauri::AppHandle, id: String) -> Result<(), String> {
    use crate::commands::config::{load_config, save_config};
    use crate::commands::env::{
        add_to_user_path, remove_from_system_path, remove_from_user_path, set_registry_env,
    };
    use crate::commands::project::registry;
    use std::path::Path;

    let def = registry::find_by_id(&id).ok_or_else(|| format!("未找到项目: {}", id))?;

    let mut config = load_config();
    let delegation = scanner::get_project_delegation(&config, &id, &def);

    // 0. 先撤掉 root_layout 生成的 junction / shim 包装 —— 必须在清空 SCOOP 之前，
    //    否则读不到根路径，留下的 junction 会指向可能已不存在的版本链接目录。
    crate::commands::project::layout::remove(&def, &Path::new(&config.links_dir).join(&id));

    // 1. 移除被托管的项目环境变量
    for var_def in &def.env_vars {
        if let Some(tier) = &var_def.tier {
            if *tier == crate::commands::project::types::EnvVarTier::Compat {
                continue;
            }
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
            // 若曾注册到系统级 PATH，也一并清理（失败静默忽略，避免卸载受阻）
            if def.requires_system_path {
                let _ = remove_from_system_path(&remove_paths);
            }
        }
    }

    // 3. 还原原始环境变量
    for var_def in &def.env_vars {
        if let Some(tier) = &var_def.tier {
            if *tier == crate::commands::project::types::EnvVarTier::Compat {
                continue;
            }
        }
        if delegation.env_vars.contains(&var_def.name) {
            let orig_val = config
                .original_envs
                .get_mut(&id)
                .and_then(|m| m.remove(&var_def.name));
            if let Some(orig_val) = orig_val {
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

    // 5. 从 managed_items and project_delegations and active_versions and auto_start_services 中移除
    config.managed_items.remove(&id);
    config.simple_managed_items.remove(&id);
    config.project_delegations.remove(&id);
    config.active_versions.remove(&id);
    config.auto_start_services.remove(&id);
    save_config(&config)?;

    // 6. 同步进程环境并重建菜单
    crate::sync_process_path();
    let _ = crate::tray::rebuild_tray_menu(&app);

    Ok(())
}

/// 将托管时的旧版安装数据和环境备份保存到独立文件，便于用户查看和管理
pub(super) fn save_manage_backup(
    id: &str,
    def: &crate::commands::project::types::ProjectDef,
    config: &crate::commands::config::Config,
) {
    use crate::commands::project::scanner;
    use std::collections::HashMap;

    let base_dir = crate::commands::config::get_data_dir();
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
            crate::commands::project::versions::detect_version_from_path(id, path)
        } else {
            None
        }
    });

    // 收集备份的环境变量（仅与此项目相关的）
    let mut backed_env_vars: HashMap<String, String> = HashMap::new();
    for var_def in &def.env_vars {
        if let Some(val) = config
            .original_envs
            .get(&def.id)
            .and_then(|m| m.get(&var_def.name))
        {
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
    use crate::commands::project::registry;

    let def = registry::find_by_id(&id).ok_or_else(|| format!("未找到项目: {}", id))?;

    let config = load_config();

    if !config.managed_items.contains(id.as_str()) {
        return Err("该项目尚未托管".to_string());
    }

    let is_simple = def.simple_mode || config.simple_managed_items.contains(&id);

    if is_simple {
        let steps = vec![crate::commands::project::types::ManageStep {
            action: "simple_unmanage".to_string(),
            description: "取消托管后，Kira 将不再管理该项目的缓存、代理和镜像配置".to_string(),
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
    let compat_vars: Vec<String> = def
        .env_vars
        .iter()
        .filter(|v| {
            v.tier.as_ref().map_or(false, |t| {
                *t == crate::commands::project::types::EnvVarTier::Compat
            })
        })
        .map(|v| v.name.clone())
        .collect();

    let managed_vars: Vec<String> = def
        .env_vars
        .iter()
        .filter(|v| {
            !v.tier.as_ref().map_or(false, |t| {
                *t == crate::commands::project::types::EnvVarTier::Compat
            })
        })
        .map(|v| v.name.clone())
        .collect();

    if !managed_vars.is_empty() {
        steps.push(crate::commands::project::types::ManageStep {
            action: "clear_env".to_string(),
            description: format!("清除 Kira 设置的 {} 个环境变量", managed_vars.len()),
            target: managed_vars.join(", "),
        });
    }

    // 步骤 2: 将从 PATH 中移除的条目
    let bin_paths = scanner::get_bin_paths(&def.id, &link_str);
    if !bin_paths.is_empty() {
        steps.push(crate::commands::project::types::ManageStep {
            action: "remove_path".to_string(),
            description: format!("从 PATH 中移除 {} 个条目", bin_paths.len()),
            target: bin_paths.join("\n"),
        });
    }

    // 步骤 3: 将要还原的环境变量
    let mut restore_count = 0;
    let mut restore_list = Vec::new();
    for var_def in &def.env_vars {
        if var_def.tier.as_ref().map_or(false, |t| {
            *t == crate::commands::project::types::EnvVarTier::Compat
        }) {
            continue;
        }
        if let Some(orig_val) = config
            .original_envs
            .get(&id)
            .and_then(|m| m.get(&var_def.name))
        {
            restore_count += 1;
            restore_list.push(format!("{} = {}", var_def.name, orig_val));
        }
    }
    if restore_count > 0 {
        steps.push(crate::commands::project::types::ManageStep {
            action: "restore_env".to_string(),
            description: format!("还原 {} 个环境变量的原始值", restore_count),
            target: restore_list.join("\n"),
        });
    }

    // 步骤 4: 将要还原的 PATH 条目
    if let Some(orig_paths) = config.original_paths.get(&id) {
        if !orig_paths.is_empty() {
            steps.push(crate::commands::project::types::ManageStep {
                action: "restore_path".to_string(),
                description: format!("向 PATH 恢复 {} 个原始条目", orig_paths.len()),
                target: orig_paths.join("\n"),
            });
        }
    }

    // 步骤 5: 说明 Compat 层变量不会被修改
    if !compat_vars.is_empty() {
        steps.push(crate::commands::project::types::ManageStep {
            action: "skip_compat".to_string(),
            description: format!(
                "以下 {} 个兼容层变量保持不变（不属于 Kira 管理）: {}",
                compat_vars.len(),
                compat_vars.join(", ")
            ),
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

pub(super) async fn install_pip_internal(project_id: &str) -> Result<String, String> {
    let config = crate::commands::config::load_config();
    let link_dir = std::path::PathBuf::from(&config.links_dir).join(project_id);
    if !link_dir.exists() {
        return Err("未找到 Python 链接目录，请先安装或启用 Python 版本。".to_string());
    }

    // 1. 尝试修改 ._pth 文件以启用 site-packages 导入
    let canonical_dir =
        std::fs::canonicalize(&link_dir).map_err(|e| format!("无法解析 Python 路径: {}", e))?;

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
        .args(&[
            "-c",
            "import sys; print(f'{sys.version_info.major}.{sys.version_info.minor}')",
        ])
        .current_dir(&canonical_dir)
        .output()
        .map_err(|e| format!("获取 Python 版本失败: {}", e))?;

    let py_ver = String::from_utf8_lossy(&ver_output.stdout)
        .trim()
        .to_string();
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

    let res = client
        .get(&download_url)
        .send()
        .await
        .map_err(|e| format!("请求 get-pip.py 失败 (URL: {}): {}", download_url, e))?;

    if !res.status().is_success() {
        return Err(format!(
            "下载 get-pip.py 返回错误状态码: {} (URL: {})",
            res.status(),
            download_url
        ));
    }

    let bytes = res
        .bytes()
        .await
        .map_err(|e| format!("读取 get-pip.py 内容失败: {}", e))?;
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
        Err(format!(
            "安装 pip 失败!\nStdout: {}\nStderr: {}",
            stdout, stderr
        ))
    }
}
