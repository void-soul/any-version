use super::*;

/// 取消正在进行的安装（中止下载/解压任务，RAII guard 自动清理临时文件）
#[tauri::command]
pub fn project_cancel_install(id: String) -> Result<(), String> {
    let mut map = get_active_downloads().lock().unwrap();
    if let Some(active) = map.remove(&id) {
        active.abort_handle.abort();
        Ok(())
    } else {
        Err(format!("没有正在进行的安装任务: {}", id))
    }
}

/// 查询当前正在进行中的安装列表（供前端挂载时恢复进度条/取消按钮）。
#[tauri::command]
pub fn project_get_active_installs() -> Vec<ActiveInstall> {
    get_active_downloads()
        .lock()
        .unwrap()
        .values()
        .map(|e| e.active.clone())
        .collect()
}

/// 卸载指定版本
#[tauri::command]
pub fn project_uninstall_version(
    app: AppHandle,
    id: String,
    version: String,
) -> Result<(), String> {
    let config = load_config();
    if !config.managed_items.contains(&id) {
        return Err("项目尚未开启托管，无法卸载版本".to_string());
    }
    let def = crate::commands::project::registry::find_by_id(&id)
        .ok_or_else(|| format!("未找到项目: {}", id))?;
    let delegation = crate::commands::project::scanner::get_project_delegation(&config, &id, &def);
    if !delegation.version_control {
        return Err("项目尚未开启“版本控制与下载”功能，无法卸载版本".to_string());
    }

    // MSIX 型（如 WinGet）：卸载走 Remove-AppxPackage，它没有 versions_dir 里的版本目录
    if def.install_mode.as_deref() == Some("msix") {
        let pkg = def
            .msix_package_name
            .clone()
            .ok_or_else(|| "该项目的 msix_package_name 未配置，无法卸载".to_string())?;
        let script = format!(
            "Get-AppxPackage -Name '{}' | Remove-AppxPackage",
            pkg.replace('\'', "''")
        );
        let out = crate::commands::hidden_cmd::hidden_cmd("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .map_err(|e| format!("执行 Remove-AppxPackage 失败: {}", e))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let stdout = String::from_utf8_lossy(&out.stdout);
            return Err(format!(
                "卸载失败: {}",
                if stderr.trim().is_empty() {
                    stdout.trim()
                } else {
                    stderr.trim()
                }
            ));
        }
        let mut cfg = load_config();
        if cfg.active_versions.remove(&id).is_some() || cfg.auto_start_services.remove(&id) {
            let _ = crate::commands::config::save_config(&cfg);
        }
        // 同步失效扫描器的 MSIX 缓存，否则列表里还留着已卸载的版本
        crate::commands::project::scanner::invalidate_msix_version_cache(&pkg);
        let _ = crate::tray::rebuild_tray_menu(&app);
        return Ok(());
    }

    let dest_dir = Path::new(&config.versions_dir).join(&id).join(&version);
    if !dest_dir.exists() {
        return Err(format!("版本 {} 的 {} 未安装", version, id));
    }

    // 如果当前正在使用该版本，先断开 junction
    let junction_path = Path::new(&config.links_dir).join(&id);
    let active_dir = fs::canonicalize(&junction_path)
        .map(|p| {
            p.to_string_lossy()
                .to_string()
                .trim_start_matches("\\\\?\\")
                .to_string()
                .to_lowercase()
        })
        .unwrap_or_default();
    let dest_dir_clean = dest_dir.to_string_lossy().to_string().to_lowercase();

    if active_dir == dest_dir_clean {
        let _ = fs::remove_dir(&junction_path);
    }

    fs::remove_dir_all(&dest_dir).map_err(|e| e.to_string())?;

    // 若无剩余已安装版本且无自定义路径，自动从自启列表中清除
    let versions_parent = Path::new(&config.versions_dir).join(&id);
    let has_remaining = versions_parent
        .read_dir()
        .map(|mut it| it.next().is_some())
        .unwrap_or(false);
    if !has_remaining && !config.custom_install_paths.contains_key(&id) {
        use crate::commands::config::save_config;
        let mut cfg = load_config();
        if cfg.auto_start_services.remove(&id) {
            let _ = save_config(&cfg);
        }
    }

    let _ = crate::tray::rebuild_tray_menu(&app);
    Ok(())
}

/// 切换到指定版本（创建 junction 指向目标版本目录）
pub(super) fn project_use_version_impl(id: String, version: String) -> Result<(), String> {
    use crate::commands::config::{load_config, save_config};
    let mut config = load_config();
    if !config.managed_items.contains(&id) {
        return Err("项目尚未开启托管，无法切换版本".to_string());
    }
    let def = crate::commands::project::registry::find_by_id(&id)
        .ok_or_else(|| format!("未找到项目: {}", id))?;
    let delegation = crate::commands::project::scanner::get_project_delegation(&config, &id, &def);
    if !delegation.version_control {
        return Err("项目尚未开启“版本控制与下载”功能，无法切换版本".to_string());
    }
    // MSIX 型是系统级覆盖安装（同名包只能有一份），不存在「多个版本目录可切换」
    if def.install_mode.as_deref() == Some("msix") {
        return Err("该 SDK 为系统级覆盖安装（装新版即替换旧版），不支持多版本切换".to_string());
    }
    let dest_dir = Path::new(&config.versions_dir).join(&id).join(&version);
    if !dest_dir.exists() {
        return Err(format!("版本 {} 的 {} 未安装", version, id));
    }

    let def = crate::commands::project::registry::find_by_id(&id)
        .ok_or_else(|| format!("未找到项目: {}", id))?;
    let delegation = crate::commands::project::scanner::get_project_delegation(&config, &id, &def);

    if delegation.create_symlink {
        let junction_path = Path::new(&config.links_dir).join(&id);
        crate::commands::cache::create_junction(&junction_path, &dest_dir)?;
    }

    config.active_versions.insert(id, version);
    save_config(&config)?;

    Ok(())
}

/// 切换到指定版本（创建 junction 指向目标版本目录）
pub fn project_use_version_inner(id: &str, version: &str) -> Result<(), String> {
    project_use_version_impl(id.to_string(), version.to_string())
}

#[tauri::command]
pub fn project_use_version(app: AppHandle, id: String, version: String) -> Result<(), String> {
    project_use_version_impl(id, version)?;
    let _ = crate::tray::rebuild_tray_menu(&app);
    Ok(())
}

/// 从命令输出中解析版本号
pub(super) fn parse_version_from_output(project_id: &str, output: &str) -> Option<String> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Find version_parse_regex from the project registry definition
    let custom_regex = crate::commands::project::registry::find_by_id(project_id)
        .and_then(|def| def.version_parse_regex.clone());

    let pattern = custom_regex
        .as_deref()
        .unwrap_or(r"\b\d+\.\d+(?:\.\d+)*(?:\-[a-zA-Z0-9.]+)?\b");
    if let Ok(re) = regex::Regex::new(pattern) {
        if let Some(captures) = re.captures(trimmed) {
            // If there's a capture group (index 1), return it. Otherwise return the whole match (index 0).
            if captures.len() > 1 {
                return captures.get(1).map(|m| m.as_str().to_string());
            } else {
                return captures.get(0).map(|m| m.as_str().to_string());
            }
        }
    }
    None
}

/// 从本地路径自动检测版本号（公开函数，供 save_manage_backup 等使用）。
pub fn detect_version_from_path(project_id: &str, path: &Path) -> Option<String> {
    let def = crate::commands::project::registry::find_by_id(project_id)?;

    // 通过 version_cmd + version_exe 检测
    if let (Some(ref cmd), Some(ref exe)) = (&def.version_cmd, &def.version_exe) {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        if !parts.is_empty() {
            let mut candidates = Vec::new();
            candidates.push(path.join(exe));
            candidates.push(path.join("bin").join(exe));
            #[cfg(windows)]
            {
                let exe_lower = exe.to_lowercase();
                if !exe_lower.ends_with(".exe")
                    && !exe_lower.ends_with(".cmd")
                    && !exe_lower.ends_with(".bat")
                {
                    candidates.push(path.join(format!("{}.exe", exe)));
                    candidates.push(path.join(format!("{}.cmd", exe)));
                    candidates.push(path.join(format!("{}.bat", exe)));
                    candidates.push(path.join("bin").join(format!("{}.exe", exe)));
                    candidates.push(path.join("bin").join(format!("{}.cmd", exe)));
                    candidates.push(path.join("bin").join(format!("{}.bat", exe)));
                }
            }
            for candidate in &candidates {
                if candidate.exists() {
                    let mut command = crate::commands::hidden_cmd::hidden_cmd(candidate);
                    for var_def in &def.env_vars {
                        command.env_remove(&var_def.name);
                    }
                    if let Ok(output) = command.args(&parts[1..]).output() {
                        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                        let combined = if stdout.trim().is_empty() {
                            stderr
                        } else {
                            stdout
                        };
                        if let Some(ver) = parse_version_from_output(project_id, &combined) {
                            return Some(ver);
                        }
                    }
                }
            }
        }
    }
    None
}
