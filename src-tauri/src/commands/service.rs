use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
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

fn read_port_from_ini(ini_path: &Path, key: &str) -> String {
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

fn read_port_from_conf(conf_path: &Path, key: &str) -> String {
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

fn read_nginx_port(conf_path: &Path) -> String {
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

struct PortOwnerSimple {
    pid: String,
    process_name: String,
}

fn find_port_owner_simple(port_str: &str) -> Option<PortOwnerSimple> {
    let output = Command::new("netstat")
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
    let task_output = Command::new("tasklist")
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
    let config = load_config();
    let service_names = vec![
        ("nginx", "80"),
        ("redis", "6379"),
        ("mysql", "3306"),
        ("mongodb", "27017"),
        ("postgresql", "5432"),
    ];

    let mut services = HashMap::new();
    for (name, default_port) in service_names {
        services.insert(name.to_string(), ServiceInfo {
            name: name.to_string(),
            status: "stopped".to_string(),
            active_version: String::new(),
            port: default_port.to_string(),
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
        if !has_installed {
            svc.status = "not_installed".to_string();
        }

        let junction_path = PathBuf::from(&config.links_dir).join(name);
        if let Ok(active_dir_path) = fs::canonicalize(&junction_path) {
            let active_dir = active_dir_path.to_string_lossy().to_string().trim_start_matches(r"\\?\").to_string();
            let v_name = Path::new(&active_dir).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            svc.active_version = v_name;

            if name == "mysql" {
                let port = read_port_from_ini(&Path::new(&active_dir).join("my.ini"), "port");
                if !port.is_empty() {
                    svc.port = port;
                }
            } else if name == "redis" {
                let port = read_port_from_conf(&Path::new(&active_dir).join("redis.windows.conf"), "port");
                if !port.is_empty() {
                    svc.port = port;
                }
            } else if name == "nginx" {
                let port = read_nginx_port(&Path::new(&active_dir).join("conf").join("nginx.conf"));
                if !port.is_empty() {
                    svc.port = port;
                }
            }
        }
    }

    // Use WMIC command to query running processes
    let output = Command::new("wmic")
        .args(&["process", "get", "ExecutablePath,ProcessId"])
        .output();

    if let Ok(out) = output {
        let text = String::from_utf8_lossy(&out.stdout);
        let versions_dir_clean = config.versions_dir.to_lowercase().replace('/', "\\");

        for line in text.lines() {
            let line_trimmed = line.trim();
            if line_trimmed.is_empty() || line_trimmed.to_lowercase().starts_with("executablepath") {
                continue;
            }

            if let Some(last_space_idx) = line_trimmed.rfind(' ') {
                let path_part = line_trimmed[..last_space_idx].trim().to_string();
                let pid_part = line_trimmed[last_space_idx..].trim().to_string();

                let path_clean = path_part.to_lowercase().replace('/', "\\");
                if path_clean.contains(&versions_dir_clean) {
                    if let Ok(pid) = pid_part.parse::<i32>() {
                        let proc_lower = path_clean.to_lowercase();
                        let (svc_key, bin_name) = if proc_lower.ends_with("nginx.exe") {
                            ("nginx", "nginx")
                        } else if proc_lower.ends_with("redis-server.exe") {
                            ("redis", "redis")
                        } else if proc_lower.ends_with("mysqld.exe") {
                            ("mysql", "mysql")
                        } else if proc_lower.ends_with("mongod.exe") {
                            ("mongodb", "mongodb")
                        } else if proc_lower.ends_with("postgres.exe") {
                            ("postgresql", "postgresql")
                        } else {
                            ("", "")
                        };

                        if !svc_key.is_empty() {
                            if let Some(svc) = services.get_mut(svc_key) {
                                svc.status = "running".to_string();
                                svc.pid = pid;
                                let version = extract_version_from_path(&path_part, bin_name);
                                if !version.is_empty() {
                                    svc.active_version = version;
                                }
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

    let order = ["nginx", "redis", "mysql", "mongodb", "postgresql"];
    let mut result: Vec<ServiceInfo> = services.into_values().collect();
    result.sort_by_key(|s| order.iter().position(|&x| x == s.name).unwrap_or(99));
    Ok(result)
}

#[tauri::command]
pub fn start_service(name: String, version: String) -> Result<(), String> {
    let services = get_running_services()?;
    let svc = services.iter().find(|s| s.name == name)
        .ok_or_else(|| format!("未知的服务: {}", name))?;

    if svc.status == "running" {
        return Err(format!("服务 {} 已经运行中 (PID: {})", name, svc.pid));
    }

    let config = load_config();
    let dir = PathBuf::from(&config.versions_dir).join(&name).join(&version);
    if !dir.exists() {
        return Err(format!("服务版本 {} 未安装", version));
    }

    // Set junction link
    let junction_path = PathBuf::from(&config.links_dir).join(&name);
    let _ = crate::commands::cache::create_junction(&junction_path, &dir);

    let output = match name.as_str() {
        "nginx" => {
            Command::new("cmd")
                .args(&["/c", "start", "/b", "nginx.exe"])
                .current_dir(&dir)
                .output()
        }
        "redis" => {
            let conf = if dir.join("redis.windows.conf").exists() { "redis.windows.conf" } else { "" };
            if !conf.is_empty() {
                Command::new("cmd")
                    .args(&["/c", "start", "/b", "redis-server.exe", conf])
                    .current_dir(&dir)
                    .output()
            } else {
                Command::new("cmd")
                    .args(&["/c", "start", "/b", "redis-server.exe"])
                    .current_dir(&dir)
                    .output()
            }
        }
        "mysql" => {
            Command::new("cmd")
                .args(&["/c", "start", "/b", "bin\\mysqld.exe", "--defaults-file=my.ini", "--console"])
                .current_dir(&dir)
                .output()
        }
        "mongodb" => {
            let db_path = dir.join("data");
            let _ = fs::create_dir_all(&db_path);
            Command::new("cmd")
                .args(&["/c", "start", "/b", "bin\\mongod.exe", "--port", "27017", "--dbpath", &db_path.to_string_lossy()])
                .current_dir(&dir)
                .output()
        }
        "postgresql" => {
            let db_path = dir.join("data");
            let log_file = dir.join("logfile");
            Command::new("cmd")
                .args(&["/c", "start", "/b", "bin\\pg_ctl.exe", "-D", &db_path.to_string_lossy(), "-l", &log_file.to_string_lossy(), "start"])
                .current_dir(&dir)
                .output()
        }
        _ => return Err(format!("不支持启动服务: {}", name)),
    }.map_err(|e| format!("启动服务失败: {}", e))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }

    Ok(())
}

#[tauri::command]
pub fn stop_service(name: String) -> Result<(), String> {
    let services = get_running_services()?;
    let svc = services.iter().find(|s| s.name == name)
        .ok_or_else(|| format!("未知的服务: {}", name))?;

    if svc.status == "stopped" {
        return Err(format!("服务 {} 未运行", name));
    }

    let is_external = svc.active_version.contains("系统/外部进程");

    if is_external {
        if svc.pid > 0 {
            let output = Command::new("taskkill")
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

    let config = load_config();
    let dir = PathBuf::from(&config.versions_dir).join(&name).join(&svc.active_version);

    let mut shutdown_err = false;
    match name.as_str() {
        "nginx" => {
            let _ = Command::new(dir.join("nginx.exe"))
                .args(&["-s", "stop"])
                .current_dir(&dir)
                .output()
                .map_err(|_| shutdown_err = true);
        }
        "redis" => {
            let cli_exe = dir.join("redis-cli.exe");
            let output = Command::new(&cli_exe)
                .args(&["-p", &svc.port, "shutdown"])
                .current_dir(&dir)
                .output();
            match output {
                Ok(out) if out.status.success() => {},
                _ => { shutdown_err = true; }
            }
        }
        "mysql" => {
            let admin_exe = dir.join("bin").join("mysqladmin.exe");
            let output = Command::new(&admin_exe)
                .args(&["--port", &svc.port, "-u", "root", "shutdown"])
                .current_dir(&dir)
                .output();
            match output {
                Ok(out) if out.status.success() => {},
                _ => { shutdown_err = true; }
            }
        }
        "postgresql" => {
            let _ = Command::new(dir.join("bin").join("pg_ctl.exe"))
                .args(&["-D", &dir.join("data").to_string_lossy(), "stop"])
                .current_dir(&dir)
                .output()
                .map_err(|_| shutdown_err = true);
        }
        _ => shutdown_err = true,
    }

    if shutdown_err && svc.pid > 0 {
        let output = Command::new("taskkill")
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
