use std::fs;
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use crate::commands::config::load_config;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ServiceInfo {
    pub name: String,
    pub status: String, // "running" | "stopped" | "not_installed"
    pub active_version: String,
    pub port: String,
    pub pid: i32,
}

pub(crate) fn read_port_from_ini(ini_path: &Path, key: &str) -> String {
    if let Ok(content) = fs::read_to_string(ini_path) {
        for line in content.lines() {
            let line_trimmed = line.trim();
            if line_trimmed.to_lowercase().starts_with(key) {
                let parts = line_trimmed.splitn(2, '=').collect::<Vec<_>>();
                if parts.len() == 2 {
                    return parts[1].trim().to_string();
                }
            }
        }
    }
    String::new()
}

pub(crate) fn read_port_from_conf(conf_path: &Path, key: &str) -> String {
    if let Ok(content) = fs::read_to_string(conf_path) {
        for line in content.lines() {
            let line_trimmed = line.trim();
            if line_trimmed.to_lowercase().starts_with(key) {
                let fields = line_trimmed.split_whitespace().collect::<Vec<_>>();
                if fields.len() >= 2 && fields[0].to_lowercase() == key {
                    return fields[1].to_string();
                }
            }
        }
    }
    String::new()
}

pub(crate) fn read_nginx_port(conf_path: &Path) -> String {
    if let Ok(content) = fs::read_to_string(conf_path) {
        if let Some(idx) = content.find("listen") {
            let sub = &content[idx..];
            if let Some(semi_idx) = sub.find(';') {
                let listen_line = &sub[6..semi_idx];
                let fields = listen_line.split_whitespace().collect::<Vec<_>>();
                if !fields.is_empty() {
                    return fields[0].trim().to_string();
                }
            }
        }
    }
    String::new()
}

fn extract_version_from_path(path: &str, name: &str) -> String {
    let path_clean = path.replace('/', "\\");
    let parts = path_clean.split('\\').collect::<Vec<_>>();
    for (i, &part) in parts.iter().enumerate() {
        if part.to_lowercase() == name && i + 1 < parts.len() {
            return parts[i + 1].to_string();
        }
    }
    String::new()
}

pub(crate) struct PortOwnerSimple {
    pub(crate) pid: String,
    pub(crate) process_name: String,
}

pub(crate) fn find_port_owner_simple(port_str: &str) -> Option<PortOwnerSimple> {
    let output = super::hidden_cmd::hidden_cmd("netstat")
        .args(&["-ano", "-p", "tcp"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let mut pid = String::new();

    for line in text.lines() {
        let line_trimmed = line.trim();
        if line_trimmed.is_empty() || !line_trimmed.to_uppercase().starts_with("TCP") {
            continue;
        }
        let fields: Vec<&str> = line_trimmed.split_whitespace().collect();
        if fields.len() < 5 {
            continue;
        }
        let local_addr = fields[1];
        let state = fields[3];
        let row_pid = fields[4];

        let mut row_port = "";
        if local_addr.contains(']') {
            let parts: Vec<&str> = local_addr.split("]:").collect();
            if parts.len() == 2 {
                row_port = parts[1];
            }
        } else {
            let parts: Vec<&str> = local_addr.split(':').collect();
            if !parts.is_empty() {
                row_port = parts[parts.len() - 1];
            }
        }

        if row_port == port_str && state == "LISTENING" {
            pid = row_pid.to_string();
            break;
        }
    }

    if pid.is_empty() {
        return None;
    }

    // tasklist to find process name
    let task_output = super::hidden_cmd::hidden_cmd("tasklist")
        .args(&["/fi", &format!("pid eq {}", pid), "/fo", "csv", "/nh"])
        .output()
        .ok()?;
    let task_text = String::from_utf8_lossy(&task_output.stdout).trim().to_string();
    let mut process_name = "Unknown".to_string();
    if !task_text.is_empty() {
        let parts: Vec<&str> = task_text.split(',').collect();
        if !parts.is_empty() {
            process_name = parts[0].trim_matches('"').to_string();
        }
    }

    Some(PortOwnerSimple {
        pid,
        process_name,
    })
}

#[tauri::command]
pub fn get_running_services() -> Result<Vec<ServiceInfo>, String> {
    use super::project::registry;

    let config = load_config();
    let all_projects = registry::registry();
    let mut service_names = Vec::new();

    for p in &all_projects {
        if p.category == crate::commands::project::types::ProjectCategory::Service || p.is_service {
            let port_str = p.default_port.unwrap_or(0).to_string();
            service_names.push((p.id.clone(), port_str));
        }
    }

    let mut services = HashMap::new();
    for (name, default_port) in &service_names {
        services.insert(name.clone(), ServiceInfo {
            name: name.clone(),
            status: "stopped".to_string(),
            active_version: String::new(),
            port: default_port.clone(),
            pid: 0,
        });
    }

    // Check version details
    for (name, svc) in services.iter_mut() {
        let sdk_dir = PathBuf::from(&config.versions_dir).join(name);
        let mut has_installed = false;
        if sdk_dir.exists() {
            if let Ok(entries) = fs::read_dir(&sdk_dir) {
                for entry in entries.filter_map(|e| e.ok()) {
                    if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        has_installed = true;
                        break;
                    }
                }
            }
        }
        // If not in AnyVersion versions_dir, check if detected locally via scanner
        if !has_installed {
            if let Some(def) = registry::find_by_id(name) {
                let (_, install_root) = super::project::scanner::detect_install_source(&def);
                if install_root.is_some() {
                    has_installed = true;
                }
            }
        }

        if !has_installed {
            svc.status = "not_installed".to_string();
        }

        let junction_path = PathBuf::from(&config.links_dir).join(name);
        let active_dir = if junction_path.exists() || junction_path.is_symlink() {
            fs::canonicalize(&junction_path).ok()
                .map(|p| p.to_string_lossy().to_string().trim_start_matches(r"\\?\").to_string())
        } else {
            if let Some(def) = registry::find_by_id(name) {
                let (_, install_root) = super::project::scanner::detect_install_source(&def);
                install_root
            } else {
                None
            }
        };

        if let Some(active_dir_path) = active_dir {
            let active_path = Path::new(&active_dir_path);
            let v_name = active_path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            svc.active_version = v_name;

            // Resolve port from config
            if let Some(def) = registry::find_by_id(name) {
                if let Some(ref conf_file) = def.config_file {
                    let conf_path = active_path.join(conf_file);
                    if conf_path.exists() {
                        let port = if name == "nginx" {
                            read_nginx_port(&conf_path)
                        } else if conf_file.ends_with(".ini") {
                            read_port_from_ini(&conf_path, "port")
                        } else {
                            read_port_from_conf(&conf_path, "port")
                        };
                        if !port.is_empty() {
                            svc.port = port;
                        }
                    }
                }
            }
        }
    }

    // Use WMIC command to query running processes
    let output = super::hidden_cmd::hidden_cmd("wmic")
        .args(&["process", "get", "ExecutablePath,ProcessId"])
        .output();

    if let Ok(out) = output {
        let text = String::from_utf8_lossy(&out.stdout);

        for line in text.lines() {
            let line_trimmed = line.trim();
            if line_trimmed.is_empty() || line_trimmed.to_lowercase().starts_with("executablepath") {
                continue;
            }

            if let Some(last_space_idx) = line_trimmed.rfind(' ') {
                let path_part = line_trimmed[..last_space_idx].trim().to_string();
                let pid_part = line_trimmed[last_space_idx..].trim().to_string();

                let path_clean = path_part.to_lowercase().replace('/', "\\");
                if let Ok(pid) = pid_part.parse::<i32>() {
                    let mut matched_id = None;
                    let mut matched_bin = None;

                    for p in &all_projects {
                        if p.category == crate::commands::project::types::ProjectCategory::Service || p.is_service {
                            let exe_name = p.version_exe.as_deref().unwrap_or(&p.id);
                            let exe_suffix = format!("{}.exe", exe_name.to_lowercase());
                            if path_clean.ends_with(&exe_suffix) {
                                matched_id = Some(p.id.clone());
                                matched_bin = Some(exe_name.to_string());
                                break;
                            }
                        }
                    }

                    if let (Some(svc_key), Some(bin_name)) = (matched_id, matched_bin) {
                        if let Some(svc) = services.get_mut(&svc_key) {
                            svc.status = "running".to_string();
                            svc.pid = pid;
                            let version = extract_version_from_path(&path_part, &bin_name);
                            if !version.is_empty() {
                                svc.active_version = version;
                            }
                        }
                    }
                }
            }
        }
    }

    // Fallback: Check port occupancy for services not detected as running
    for svc in services.values_mut() {
        if svc.status != "running" {
            if let Some(owner) = find_port_owner_simple(&svc.port) {
                svc.status = "running".to_string();
                if let Ok(pid_val) = owner.pid.parse::<i32>() {
                    svc.pid = pid_val;
                }
                if svc.active_version.is_empty() {
                    svc.active_version = format!("系统/外部进程 ({})", owner.process_name);
                }
            }
        }
    }

    let order: Vec<String> = all_projects.iter().map(|p| p.id.clone()).collect();
    let mut result: Vec<ServiceInfo> = services.into_values().collect();
    result.sort_by_key(|s| order.iter().position(|x| x == &s.name).unwrap_or(99));
    Ok(result)
}

#[tauri::command]
pub fn start_service(name: String, version: String) -> Result<(), String> {
    use super::project::registry;

    let services = get_running_services()?;
    let svc = services.iter().find(|s| s.name == name)
        .ok_or_else(|| format!("未知的服务: {}", name))?;

    if svc.status == "running" {
        return Err(format!("服务 {} 已经运行中 (PID: {})", name, svc.pid));
    }

    let def = registry::find_by_id(&name)
        .ok_or_else(|| format!("未找到服务定义: {}", name))?;

    let config = load_config();
    let dir = if config.managed_items.contains(&name) {
        let d = PathBuf::from(&config.versions_dir).join(&name).join(&version);
        if !d.exists() {
            return Err(format!("服务版本 {} 未安装", version));
        }
        // Set junction link
        let junction_path = PathBuf::from(&config.links_dir).join(&name);
        let _ = crate::commands::cache::create_junction(&junction_path, &d);
        junction_path
    } else {
        let (_, local_root) = super::project::scanner::detect_install_source(&def);
        if let Some(local_root) = local_root {
            PathBuf::from(local_root)
        } else {
            return Err("未检测到本地安装，且服务未纳入托管。请先安装或托管。".to_string());
        }
    };

    let start_cmd_template = def.start_cmd.as_deref().unwrap_or("");
    if start_cmd_template.is_empty() {
        return Err(format!("服务 {} 未配置启动命令", name));
    }

    // Resolve data dir and log dir
    let data_dir = if let Some(first_dir) = def.data_dirs.first() {
        let mut p = first_dir.default_path.clone();
        p = p.replace("{install_root}", &dir.to_string_lossy());
        p = crate::commands::utils::expand_home(&p);
        p
    } else {
        String::new()
    };

    let log_dir = def.log_dir.as_deref()
        .map(|d| dir.join(d).to_string_lossy().to_string())
        .unwrap_or_default();

    if !data_dir.is_empty() {
        let _ = fs::create_dir_all(&data_dir);
    }

    let cmd_str = start_cmd_template
        .replace("{dir}", &dir.to_string_lossy())
        .replace("{install_root}", &dir.to_string_lossy())
        .replace("{port}", &svc.port)
        .replace("{data_dir}", &data_dir)
        .replace("{log_dir}", &log_dir);

    let output = super::hidden_cmd::hidden_cmd("cmd")
        .args(&["/c", &cmd_str])
        .current_dir(&dir)
        .output()
        .map_err(|e| format!("启动服务失败: {}", e))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }

    Ok(())
}

#[tauri::command]
pub fn stop_service(name: String) -> Result<(), String> {
    use super::project::registry;

    let services = get_running_services()?;
    let svc = services.iter().find(|s| s.name == name)
        .ok_or_else(|| format!("未知的服务: {}", name))?;

    if svc.status == "stopped" {
        return Err(format!("服务 {} 未运行", name));
    }

    let is_external = svc.active_version.contains("系统/外部进程");

    if is_external {
        if svc.pid > 0 {
            let output = super::hidden_cmd::hidden_cmd("taskkill")
                .args(&["/f", "/pid", &svc.pid.to_string()])
                .output()
                .map_err(|e| format!("停止外部服务失败: {}", e))?;
            if !output.status.success() {
                let err_msg = String::from_utf8_lossy(&output.stderr).to_string();
                return Err(format!("无法终止外部服务进程 (PID: {}): {}", svc.pid, err_msg));
            }
            return Ok(());
        }
        return Err("外部服务未检测到有效的 PID".to_string());
    }

    let def = registry::find_by_id(&name)
        .ok_or_else(|| format!("未找到服务定义: {}", name))?;

    let config = load_config();
    let dir = if config.managed_items.contains(&name) {
        PathBuf::from(&config.links_dir).join(&name)
    } else {
        let (_, local_root) = super::project::scanner::detect_install_source(&def);
        if let Some(local_root) = local_root {
            PathBuf::from(local_root)
        } else {
            PathBuf::new()
        }
    };

    let mut shutdown_err = false;

    if let Some(ref stop_cmd_template) = def.stop_cmd {
        if !stop_cmd_template.is_empty() && !dir.as_os_str().is_empty() {
            // Resolve data dir and log dir
            let data_dir = if let Some(first_dir) = def.data_dirs.first() {
                let mut p = first_dir.default_path.clone();
                p = p.replace("{install_root}", &dir.to_string_lossy());
                p = crate::commands::utils::expand_home(&p);
                p
            } else {
                String::new()
            };

            let log_dir = def.log_dir.as_deref()
                .map(|d| dir.join(d).to_string_lossy().to_string())
                .unwrap_or_default();

            let cmd_str = stop_cmd_template
                .replace("{dir}", &dir.to_string_lossy())
                .replace("{install_root}", &dir.to_string_lossy())
                .replace("{port}", &svc.port)
                .replace("{data_dir}", &data_dir)
                .replace("{log_dir}", &log_dir);

            let output = super::hidden_cmd::hidden_cmd("cmd")
                .args(&["/c", &cmd_str])
                .current_dir(&dir)
                .output();

            match output {
                Ok(out) if out.status.success() => {},
                _ => { shutdown_err = true; }
            }
        } else {
            shutdown_err = true;
        }
    } else {
        shutdown_err = true;
    }

    if shutdown_err && svc.pid > 0 {
        let output = super::hidden_cmd::hidden_cmd("taskkill")
            .args(&["/f", "/pid", &svc.pid.to_string()])
            .output()
            .map_err(|e| format!("强制终止服务失败: {}", e))?;
        if !output.status.success() {
            let err_msg = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(format!("正常关闭失败，且无法强制终止进程 (PID: {}): {}", svc.pid, err_msg));
        }
    }

    Ok(())
}
