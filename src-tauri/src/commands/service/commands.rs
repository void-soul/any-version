use super::*;

#[tauri::command]
pub fn get_running_services() -> Result<Vec<ServiceInfo>, String> {
    use crate::commands::project::registry;

    let all_projects = registry::registry();
    let mut result = Vec::new();

    for def in all_projects.iter().filter(|p| is_service_project(p)) {
        let status = service_status_for_def(def);
        result.push(ServiceInfo {
            name: def.id.clone(),
            status: status
                .status
                .clone()
                .unwrap_or_else(|| if status.running { "running" } else { "stopped" }.to_string()),
            active_version: status.install_root.clone().unwrap_or_default(),
            port: status.port.map(|p| p.to_string()).unwrap_or_default(),
            pid: status.pid.map(|p| p as i32).unwrap_or(0),
        });
    }

    Ok(result)
}

#[tauri::command]
pub fn start_service(
    app: tauri::AppHandle,
    name: String,
    version: Option<String>,
) -> Result<(), String> {
    start_service_inner(name.clone(), version)?;
    let _ = crate::tray::rebuild_tray_menu(&app);
    // 主动通知前端服务状态变化，避免依赖 3s 轮询的缓存空窗导致刷新不及时。
    let _ = app.emit("service-status-changed", &name);
    Ok(())
}

pub(crate) fn start_service_inner(name: String, version: Option<String>) -> Result<(), String> {
    use crate::commands::project::registry;
    let def = registry::find_by_id(&name).ok_or_else(|| format!("未找到服务定义: {}", name))?;
    if !is_service_project(&def) {
        return Err(format!("{} 不是服务项目", name));
    }

    let status = service_status_for_def(&def);
    if status.running {
        return Err(format!(
            "服务 {} 已经运行中 (PID: {})",
            name,
            status.pid.unwrap_or(0)
        ));
    }
    if status.status.as_deref() == Some("port_conflict") {
        return Err(format!(
            "端口 {} 已被 {} 占用，请先处理端口冲突",
            status.port.map(|p| p.to_string()).unwrap_or_default(),
            status
                .process_name
                .unwrap_or_else(|| "其他进程".to_string())
        ));
    }

    let runtime = resolve_service_runtime(&def, version.as_deref())?;
    let install_root = runtime
        .install_root
        .clone()
        .ok_or_else(|| "未检测到本地安装，请先手动指定安装目录。".to_string())?;
    let start_cmd_template = def.start_cmd.as_deref().unwrap_or("");
    if start_cmd_template.is_empty() {
        return Err(format!("服务 {} 未配置启动命令", name));
    }

    for dir in &runtime.data_dirs {
        let path = Path::new(&dir.path);
        if !path.exists() {
            if dir.auto_create {
                fs::create_dir_all(path)
                    .map_err(|e| format!("创建{}失败: {}", dir.display_name, e))?;
            } else if dir.required_for_start {
                return Err(format!(
                    "{}不存在: {}。请先初始化或设置正确路径。",
                    dir.display_name, dir.path
                ));
            }
        }
    }

    let cmd_str = render_command(start_cmd_template, &runtime);
    let detached = def.service_start_mode.as_deref() == Some("detached");
    run_service_command(&cmd_str, Some(&install_root), detached)?;

    // 启动命令已发出（detached 进程在后台初始化；wait 模式则 run_service_command 已阻塞到
    // 命令结束）。mysqld/redis 等 detached 启动后仍需初始化数据目录、绑定端口，往往比
    // run_service_command 内部的 800ms 更久才真正 ready。若这里直接返回 Ok，前端立即刷新
    // 会探测到「端口尚未 LISTENING」而显示成 stopped，造成「启动中→未启动→刷新才 running」的
    // 闪烁。因此主动轮询等待服务真正就绪后再返回。
    wait_for_service_running(&def)?;
    // 启动成功：失效服务状态快照，避免前端轮询读到 TTL 内过期状态。
    invalidate_service_status_cache();
    Ok(())
}

#[tauri::command]
pub fn stop_service(app: tauri::AppHandle, name: String) -> Result<(), String> {
    stop_service_inner(name.clone())?;
    let _ = crate::tray::rebuild_tray_menu(&app);
    let _ = app.emit("service-status-changed", &name);
    Ok(())
}

pub(crate) fn stop_service_inner(name: String) -> Result<(), String> {
    use crate::commands::project::registry;

    let def = registry::find_by_id(&name).ok_or_else(|| format!("未找到服务定义: {}", name))?;
    if !is_service_project(&def) {
        return Err(format!("{} 不是服务项目", name));
    }

    let status = service_status_for_def(&def);
    if status.external || status.status.as_deref() == Some("external_running") {
        return Err(format!(
            "服务 {} 正由外部进程运行，Kira 不会接管或停止该进程",
            def.display_name
        ));
    }
    if !status.running {
        if status.status.as_deref() == Some("port_conflict") {
            return Err(format!(
                "端口被 {} 占用，但它不是 {} 服务进程，已拒绝停止。",
                status
                    .process_name
                    .unwrap_or_else(|| "其他进程".to_string()),
                def.display_name
            ));
        }
        return Err(format!("服务 {} 未运行", name));
    }

    // 检测是否作为 Windows 系统服务在运行，如果是则直接提示并阻断，不主动接管停止
    #[cfg(windows)]
    {
        if !def.service_names.is_empty() {
            if let Some(active_service_name) = find_running_system_service(&def) {
                return Err(format!(
                    "检测到 {} 当前正作为 Windows 系统服务（服务名: {}）在后台运行。\n为了保障系统安全性，Any Version 不会主动操作 Windows 系统服务。请你先在 Windows 服务管理器 (services.msc) 中手动停止该服务。",
                    def.display_name, active_service_name
                ));
            }
        }
    }

    let runtime = resolve_service_runtime(&def, None)?;
    let install_root = runtime.install_root.clone();
    let mut stop_error = None;

    if let Some(stop_cmd_template) = def.stop_cmd.as_deref().filter(|s| !s.trim().is_empty()) {
        let cmd_str = render_command(stop_cmd_template, &runtime);
        if let Err(err) = run_service_command(&cmd_str, install_root.as_deref(), false) {
            stop_error = Some(err);
        }
    } else {
        stop_error = Some("未配置安全停止命令".to_string());
    }

    if let Some(err) = stop_error {
        if def.service_allow_force_kill {
            if let Some(pid) = status.pid {
                let proc_name = status.process_name.as_deref().unwrap_or_default();
                if !proc_name.is_empty() && !process_matches_def(proc_name, &def) {
                    return Err(format!(
                        "停止命令失败，且进程 {} 不匹配服务定义，已拒绝强制终止: {}",
                        proc_name, err
                    ));
                }
                let output = crate::commands::hidden_cmd::hidden_cmd("taskkill")
                    .args(&["/f", "/pid", &pid.to_string()])
                    .output()
                    .map_err(|e| format!("强制终止服务失败: {}", e))?;
                if !output.status.success() {
                    let err_msg = String::from_utf8_lossy(&output.stderr).to_string();
                    return Err(format!(
                        "正常关闭失败，且无法强制终止进程 (PID: {}): {}",
                        pid, err_msg
                    ));
                }
            }
        } else {
            return Err(format!(
                "无法安全停止 {}：{}。为避免数据损坏，未执行强制终止。",
                def.display_name, err
            ));
        }
    }

    // 停止成功：失效服务状态快照，避免前端轮询读到 TTL 内过期状态。
    invalidate_service_status_cache();
    Ok(())
}

/// 用户在前端确认后强制终止服务进程（taskkill /f /t）。
/// 严格校验进程名属于服务定义，避免误杀；不再受 service_allow_force_kill 限制。
#[tauri::command]
pub fn force_stop_service(app: tauri::AppHandle, name: String) -> Result<(), String> {
    force_stop_service_inner(name.clone())?;
    let _ = crate::tray::rebuild_tray_menu(&app);
    let _ = app.emit("service-status-changed", &name);
    Ok(())
}

pub(crate) fn force_stop_service_inner(name: String) -> Result<(), String> {
    use crate::commands::project::registry;

    let def = registry::find_by_id(&name).ok_or_else(|| format!("未找到服务定义: {}", name))?;
    if !is_service_project(&def) {
        return Err(format!("{} 不是服务项目", name));
    }

    let status = service_status_for_def(&def);
    if status.external || status.status.as_deref() == Some("external_running") {
        return Err(format!(
            "服务 {} 正由外部进程运行，Kira 不会强制终止该进程",
            def.display_name
        ));
    }
    let Some(pid) = status.pid else {
        return Err(format!(
            "未检测到 {} 服务进程，无法强制终止",
            def.display_name
        ));
    };

    // 二次校验进程名属于该服务，防止误杀同 PID 复用情况
    let proc_name = status.process_name.as_deref().unwrap_or_default();
    if proc_name.is_empty() || !process_matches_def(proc_name, &def) {
        return Err(format!(
            "PID {} 对应的进程 {} 与 {} 服务定义不匹配，已拒绝强制终止",
            pid,
            if proc_name.is_empty() {
                "<unknown>"
            } else {
                proc_name
            },
            def.display_name
        ));
    }

    let output = crate::commands::hidden_cmd::hidden_cmd("taskkill")
        .args(&["/f", "/t", "/pid", &pid.to_string()])
        .output()
        .map_err(|e| format!("强制终止服务失败: {}", e))?;
    if !output.status.success() {
        let err_msg = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!("强制终止进程失败 (PID: {}): {}", pid, err_msg));
    }
    // 强制终止成功：失效服务状态快照。
    invalidate_service_status_cache();
    Ok(())
}

#[tauri::command]
pub fn read_service_config(name: String) -> Result<String, String> {
    use crate::commands::config::load_config;
    use crate::commands::project::registry;

    let def = registry::find_by_id(&name).ok_or_else(|| format!("未找到服务定义: {}", name))?;
    if !is_service_project(&def) {
        return Err(format!("{} 不是服务项目", name));
    }

    let config = load_config();
    let install_root = detect_install_root(&def, &config, None)?;
    let config_file = resolve_config_file(&def, install_root.as_deref())
        .ok_or_else(|| "未找到配置文件。请确保已启用版本或指定了安装目录。".to_string())?;

    if !config_file.exists() {
        return Err(format!("配置文件不存在: {}", config_file.display()));
    }

    std::fs::read_to_string(&config_file).map_err(|e| format!("读取配置文件失败: {}", e))
}

#[tauri::command]
pub fn write_service_config(name: String, content: String) -> Result<(), String> {
    use crate::commands::config::load_config;
    use crate::commands::project::registry;

    let def = registry::find_by_id(&name).ok_or_else(|| format!("未找到服务定义: {}", name))?;
    if !is_service_project(&def) {
        return Err(format!("{} 不是服务项目", name));
    }

    let config = load_config();
    let install_root = detect_install_root(&def, &config, None)?;
    let config_file = resolve_config_file(&def, install_root.as_deref())
        .ok_or_else(|| "未找到配置文件。请确保已启用版本或指定了安装目录。".to_string())?;

    if let Some(parent) = config_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    std::fs::write(&config_file, content).map_err(|e| format!("写入配置文件失败: {}", e))
}
