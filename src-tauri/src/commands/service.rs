use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use serde::{Serialize, Deserialize};
use crate::commands::config::{load_config, Config};
use crate::commands::project::types::{DataDirDef, ProjectCategory, ProjectDef, ServiceStatus};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ServiceInfo {
    pub name: String,
    pub status: String, // "running" | "stopped" | "not_installed" | "port_conflict"
    pub active_version: String,
    pub port: String,
    pub pid: i32,
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedDataDir {
    pub(crate) id: String,
    pub(crate) display_name: String,
    pub(crate) path: String,
    pub(crate) kind: Option<String>,
    pub(crate) source: String,
    pub(crate) auto_create: bool,
    pub(crate) required_for_start: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceRuntime {
    pub(crate) install_root: Option<PathBuf>,
    pub(crate) config_file: Option<PathBuf>,
    pub(crate) port: Option<u16>,
    pub(crate) data_dirs: Vec<ResolvedDataDir>,
    pub(crate) data_dir: String,
    pub(crate) log_dir: String,
}

#[derive(Clone, Debug)]
struct ProcessInfo {
    pid: u32,
    name: String,
    exe_path: Option<String>,
}

pub(crate) fn read_port_from_ini(ini_path: &Path, key: &str) -> String {
    if let Ok(content) = fs::read_to_string(ini_path) {
        for line in content.lines() {
            let line_trimmed = line.trim();
            if line_trimmed.starts_with('#') || line_trimmed.starts_with(';') {
                continue;
            }
            if line_trimmed.to_lowercase().starts_with(key) {
                let parts = line_trimmed.splitn(2, '=').collect::<Vec<_>>();
                if parts.len() == 2 {
                    return parts[1].trim().trim_matches('"').to_string();
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
            if line_trimmed.starts_with('#') || line_trimmed.starts_with(';') {
                continue;
            }
            if line_trimmed.to_lowercase().starts_with(key) {
                if line_trimmed.contains('=') {
                    let parts = line_trimmed.splitn(2, '=').collect::<Vec<_>>();
                    if parts.len() == 2 {
                        return parts[1].trim().trim_matches('"').trim_matches('\'').to_string();
                    }
                }
                let fields = line_trimmed.split_whitespace().collect::<Vec<_>>();
                if fields.len() >= 2 && fields[0].to_lowercase() == key {
                    return fields[1].trim_matches('"').trim_matches('\'').to_string();
                }
            }
        }
    }
    String::new()
}

pub(crate) fn read_nginx_port(conf_path: &Path) -> String {
    if let Ok(content) = fs::read_to_string(conf_path) {
        for line in content.lines() {
            let line_trimmed = line.trim();
            if line_trimmed.starts_with('#') || !line_trimmed.starts_with("listen") {
                continue;
            }
            let listen_line = line_trimmed
                .trim_start_matches("listen")
                .trim()
                .trim_end_matches(';')
                .trim();
            if let Some(first) = listen_line.split_whitespace().next() {
                let port = first.rsplit(':').next().unwrap_or(first);
                return port.trim().to_string();
            }
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

        let row_port = if local_addr.contains(']') {
            local_addr.split("]:").nth(1).unwrap_or("")
        } else {
            local_addr.rsplit(':').next().unwrap_or("")
        };

        if row_port == port_str && state == "LISTENING" {
            pid = row_pid.to_string();
            break;
        }
    }

    if pid.is_empty() {
        return None;
    }

    let process_name = process_name_by_pid(&pid).unwrap_or_else(|| "Unknown".to_string());
    Some(PortOwnerSimple { pid, process_name })
}

fn process_name_by_pid(pid: &str) -> Option<String> {
    let task_output = super::hidden_cmd::hidden_cmd("tasklist")
        .args(&["/fi", &format!("pid eq {}", pid), "/fo", "csv", "/nh"])
        .output()
        .ok()?;
    let task_text = String::from_utf8_lossy(&task_output.stdout).trim().to_string();
    if task_text.is_empty() {
        return None;
    }
    task_text
        .split(',')
        .next()
        .map(|p| p.trim_matches('"').to_string())
}

fn is_service_project(def: &ProjectDef) -> bool {
    def.category == ProjectCategory::Service || def.is_service
}

fn expand_path_template(template: &str, install_root: Option<&Path>) -> String {
    let mut path = crate::commands::utils::expand_home(template);
    if let Some(root) = install_root {
        path = path.replace("{install_root}", &root.to_string_lossy());
        path = path.replace("{dir}", &root.to_string_lossy());
    }
    path
}

fn service_exe_names(def: &ProjectDef) -> Vec<String> {
    let mut names = def.service_process_exes.clone();
    if names.is_empty() {
        if let Some(exe) = &def.version_exe {
            names.push(exe.clone());
        } else {
            names.push(def.id.clone());
        }
    }
    names
}

fn normalize_exe_name(name: &str) -> String {
    let lower = name.to_lowercase();
    if lower.ends_with(".exe") {
        lower
    } else {
        format!("{}.exe", lower)
    }
}

fn process_matches_def(process_name: &str, def: &ProjectDef) -> bool {
    let process = normalize_exe_name(process_name);
    service_exe_names(def)
        .iter()
        .map(|n| normalize_exe_name(n))
        .any(|n| n == process)
}

fn service_processes(def: &ProjectDef) -> Vec<ProcessInfo> {
    let names: Vec<String> = service_exe_names(def)
        .iter()
        .map(|n| normalize_exe_name(n))
        .collect();
    if names.is_empty() {
        return Vec::new();
    }

    let mut result = Vec::new();
    let mut seen_pids = std::collections::HashSet::new();

    for name in &names {
        // tasklist /fo csv 输出："Image Name","PID","Session Name","Session#","Mem Usage"
        // 头一行是表头，后续每行用引号包裹各列。
        let output = super::hidden_cmd::hidden_cmd("tasklist")
            .args(&["/fi", &format!("IMAGENAME eq {}", name), "/fo", "csv", "/nh"])
            .output();
        let Ok(out) = output else { continue; };
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with("INFO:") {
                continue;
            }
            let fields: Vec<String> = line
                .split("\",\"")
                .map(|s| s.trim_matches('"').to_string())
                .collect();
            if fields.len() < 2 {
                continue;
            }
            let image = fields[0].clone();
            let Ok(pid) = fields[1].parse::<u32>() else { continue; };
            if !seen_pids.insert(pid) {
                continue;
            }
            let exe_path = exe_path_by_pid(pid);
            result.push(ProcessInfo {
                pid,
                name: image,
                exe_path,
            });
        }
    }

    result
}

/// 通过 wmic 查询某 PID 的 ExecutablePath，便于据此推断 install_root。
fn exe_path_by_pid(pid: u32) -> Option<String> {
    let output = super::hidden_cmd::hidden_cmd("wmic")
        .args(&[
            "process",
            "where",
            &format!("ProcessId={}", pid),
            "get",
            "ExecutablePath",
            "/value",
        ])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("ExecutablePath=") {
            let value = rest.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// 通过 sc query 获取与该服务定义匹配的 Windows 服务，并解析 BINARY_PATH_NAME。
/// 返回的 PathBuf 是服务 EXE 所在目录（去掉 EXE 文件名）。
fn detect_service_install_via_sc(def: &ProjectDef) -> Option<PathBuf> {
    if def.service_names.is_empty() {
        return None;
    }
    let patterns: Vec<regex::Regex> = def
        .service_names
        .iter()
        .filter_map(|p| regex::RegexBuilder::new(p).case_insensitive(true).build().ok())
        .collect();
    if patterns.is_empty() {
        return None;
    }

    // sc query type= service state= all —— 列出所有服务名
    let output = super::hidden_cmd::hidden_cmd("sc")
        .args(&["query", "type=", "service", "state=", "all"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);

    let mut matched_services: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("SERVICE_NAME:") {
            let name = rest.trim();
            if patterns.iter().any(|re| re.is_match(name)) {
                matched_services.push(name.to_string());
            }
        }
    }

    for service in matched_services {
        if let Some(root) = sc_service_install_root(&service) {
            return Some(root);
        }
    }
    None
}

fn sc_service_install_root(service: &str) -> Option<PathBuf> {
    let output = super::hidden_cmd::hidden_cmd("sc")
        .args(&["qc", service])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("BINARY_PATH_NAME") {
            // 形如 "BINARY_PATH_NAME   : "C:\Path\bin\foo.exe" --service ..."
            let value = rest.trim_start_matches(':').trim();
            // 提取第一段路径：处理可选引号 + 多余参数
            let exe_path = if let Some(stripped) = value.strip_prefix('"') {
                stripped.split('"').next().unwrap_or("").to_string()
            } else {
                value.split_whitespace().next().unwrap_or("").to_string()
            };
            let exe_path = exe_path.trim();
            if exe_path.is_empty() {
                return None;
            }
            let path = Path::new(exe_path);
            return path.parent().map(|p| p.to_path_buf());
        }
    }
    None
}

fn detect_install_root(def: &ProjectDef, config: &Config, version: Option<&str>) -> Result<Option<PathBuf>, String> {
    let fully_managed = config.managed_items.contains(&def.id)
        && !config.simple_managed_items.contains(&def.id)
        && !def.simple_mode;

    if fully_managed {
        if let Some(version) = version.filter(|v| !v.trim().is_empty()) {
            let version_dir = PathBuf::from(&config.versions_dir).join(&def.id).join(version);
            if !version_dir.exists() {
                return Err(format!("服务版本 {} 未安装", version));
            }
            let junction_path = PathBuf::from(&config.links_dir).join(&def.id);
            crate::commands::cache::create_junction(&junction_path, &version_dir)?;
            return Ok(Some(junction_path));
        }

        let junction_path = PathBuf::from(&config.links_dir).join(&def.id);
        if junction_path.exists() || junction_path.is_symlink() {
            return Ok(Some(junction_path));
        }
        return Err("请先启用一个版本，然后才能启动服务".to_string());
    }

    if let Some(custom) = config.custom_install_paths.get(&def.id) {
        return Ok(Some(PathBuf::from(custom)));
    }

    let (_, local_root) = super::project::scanner::detect_install_source(def);
    if let Some(root) = local_root {
        return Ok(Some(PathBuf::from(root)));
    }

    // find_rules 未命中时，兜底用 sc query 解析 BINARY_PATH_NAME
    if let Some(root) = detect_service_install_via_sc(def) {
        return Ok(Some(root));
    }

    Ok(None)
}

fn resolve_config_file(def: &ProjectDef, install_root: Option<&Path>) -> Option<PathBuf> {
    let mut candidates = def.config_file_candidates.clone();
    if let Some(config_file) = &def.config_file {
        candidates.push(config_file.clone());
    }

    for candidate in candidates {
        let resolved = expand_path_template(&candidate, install_root);
        let path = PathBuf::from(&resolved);
        let path = if path.is_absolute() {
            path
        } else if let Some(root) = install_root {
            root.join(path)
        } else {
            path
        };
        if path.exists() {
            return Some(path);
        }
    }

    None
}

fn resolve_port(def: &ProjectDef, config_file: Option<&Path>) -> Option<u16> {
    if let Some(conf) = config_file {
        let conf_name = conf.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        let port = if def.id == "nginx" || conf_name.contains("nginx") {
            read_nginx_port(conf)
        } else if conf_name.ends_with(".ini") || conf_name.ends_with(".cnf") {
            read_port_from_ini(conf, "port")
        } else {
            read_port_from_conf(conf, "port")
        };
        if let Ok(parsed) = port.trim().parse::<u16>() {
            return Some(parsed);
        }
    }
    def.default_port
}

pub(crate) fn resolve_service_runtime(def: &ProjectDef, version: Option<&str>) -> Result<ServiceRuntime, String> {
    let config = load_config();
    let install_root = detect_install_root(def, &config, version)?;
    let config_file = resolve_config_file(def, install_root.as_deref());
    let port = resolve_port(def, config_file.as_deref());
    let mut data_dirs = Vec::new();

    for dir_def in &def.data_dirs {
        data_dirs.push(resolve_data_dir(def, dir_def, &config, install_root.as_deref()));
    }

    let data_dir = data_dirs
        .iter()
        .find(|d| d.kind.as_deref().unwrap_or("data") == "data")
        .or_else(|| data_dirs.first())
        .map(|d| d.path.clone())
        .or_else(|| def.data_dir.as_ref().map(|d| expand_path_template(d, install_root.as_deref())))
        .unwrap_or_default();

    let log_dir = data_dirs
        .iter()
        .find(|d| d.kind.as_deref() == Some("log"))
        .map(|d| d.path.clone())
        .or_else(|| def.log_dir.as_ref().map(|d| expand_path_template(d, install_root.as_deref())))
        .unwrap_or_default();

    Ok(ServiceRuntime {
        install_root,
        config_file,
        port,
        data_dirs,
        data_dir,
        log_dir,
    })
}

pub(crate) fn resolve_data_dir(
    def: &ProjectDef,
    dir_def: &DataDirDef,
    config: &Config,
    install_root: Option<&Path>,
) -> ResolvedDataDir {
    if let Some(project_paths) = config.custom_data_paths.get(&def.id) {
        if let Some(path) = project_paths.get(&dir_def.id) {
            return ResolvedDataDir {
                id: dir_def.id.clone(),
                display_name: dir_def.display_name.clone(),
                path: expand_path_template(path, install_root),
                kind: dir_def.kind.clone(),
                source: "custom".to_string(),
                auto_create: dir_def.auto_create.unwrap_or(def.service_auto_create_dirs),
                required_for_start: dir_def.required_for_start,
            };
        }
    }

    if let Some(env_var) = &dir_def.env_var {
        if let Some(value) = crate::commands::env::get_registry_env(env_var) {
            if !value.trim().is_empty() {
                return ResolvedDataDir {
                    id: dir_def.id.clone(),
                    display_name: dir_def.display_name.clone(),
                    path: expand_path_template(&value, install_root),
                    kind: dir_def.kind.clone(),
                    source: format!("env:{}", env_var),
                    auto_create: dir_def.auto_create.unwrap_or(def.service_auto_create_dirs),
                    required_for_start: dir_def.required_for_start,
                };
            }
        }
    }

    for candidate in &dir_def.possible_paths {
        let resolved = expand_path_template(candidate, install_root);
        if Path::new(&resolved).exists() {
            return ResolvedDataDir {
                id: dir_def.id.clone(),
                display_name: dir_def.display_name.clone(),
                path: resolved,
                kind: dir_def.kind.clone(),
                source: "detected".to_string(),
                auto_create: dir_def.auto_create.unwrap_or(def.service_auto_create_dirs),
                required_for_start: dir_def.required_for_start,
            };
        }
    }

    ResolvedDataDir {
        id: dir_def.id.clone(),
        display_name: dir_def.display_name.clone(),
        path: expand_path_template(&dir_def.default_path, install_root),
        kind: dir_def.kind.clone(),
        source: "default".to_string(),
        auto_create: dir_def.auto_create.unwrap_or(def.service_auto_create_dirs),
        required_for_start: dir_def.required_for_start,
    }
}

pub(crate) fn service_status_for_def(def: &ProjectDef) -> ServiceStatus {
    let mut runtime = resolve_service_runtime(def, None).ok();
    let mut install_root = runtime.as_ref().and_then(|r| r.install_root.clone());
    let processes = service_processes(def);

    // 进程兜底：find_rules / sc 都未给出 install_root 时，把运行中进程的 EXE 目录视作安装根
    if install_root.is_none() {
        if let Some(exe) = processes
            .iter()
            .find_map(|p| p.exe_path.as_ref().and_then(|s| Path::new(s).parent().map(|p| p.to_path_buf())))
        {
            // 用兜底 install_root 重跑 runtime（数据目录/配置文件/端口都能跟着对齐）
            let config = load_config();
            let config_file = resolve_config_file(def, Some(exe.as_path()));
            let port = resolve_port(def, config_file.as_deref());
            let mut data_dirs = Vec::new();
            for dir_def in &def.data_dirs {
                data_dirs.push(resolve_data_dir(def, dir_def, &config, Some(exe.as_path())));
            }
            let data_dir = data_dirs
                .iter()
                .find(|d| d.kind.as_deref().unwrap_or("data") == "data")
                .or_else(|| data_dirs.first())
                .map(|d| d.path.clone())
                .unwrap_or_default();
            let log_dir = data_dirs
                .iter()
                .find(|d| d.kind.as_deref() == Some("log"))
                .map(|d| d.path.clone())
                .unwrap_or_default();
            install_root = Some(exe.clone());
            runtime = Some(ServiceRuntime {
                install_root: Some(exe),
                config_file,
                port,
                data_dirs,
                data_dir,
                log_dir,
            });
        }
    }

    let port = runtime.as_ref().and_then(|r| r.port).or(def.default_port);

    let mut running = false;
    let mut status = if install_root.is_some() { "stopped" } else { "not_installed" }.to_string();
    let mut pid = None;
    let mut process_name = None;

    if let Some(process) = processes.first() {
        running = true;
        status = "running".to_string();
        pid = Some(process.pid);
        process_name = Some(process.name.clone());
    } else if let Some(port) = port {
        if let Some(owner) = find_port_owner_simple(&port.to_string()) {
            let owner_matches = process_matches_def(&owner.process_name, def);
            if let Ok(owner_pid) = owner.pid.parse::<u32>() {
                pid = Some(owner_pid);
            }
            process_name = Some(owner.process_name.clone());
            if owner_matches {
                running = true;
                status = "running".to_string();
            } else {
                status = "port_conflict".to_string();
            }
        }
    }

    ServiceStatus {
        running,
        port,
        pid,
        data_dir: runtime.as_ref().map(|r| r.data_dir.clone()).unwrap_or_default(),
        log_dir: runtime.as_ref().map(|r| r.log_dir.clone()).unwrap_or_default(),
        status: Some(status),
        process_name,
        install_root: install_root.map(|p| p.to_string_lossy().to_string()),
        config_file: runtime
            .as_ref()
            .and_then(|r| r.config_file.as_ref())
            .map(|p| p.to_string_lossy().to_string()),
    }
}

fn render_command(template: &str, runtime: &ServiceRuntime) -> String {
    let install_root = runtime.install_root.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
    let config_file = runtime.config_file.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
    let port = runtime.port.map(|p| p.to_string()).unwrap_or_default();

    template
        .replace("{dir}", &install_root)
        .replace("{install_root}", &install_root)
        .replace("{config_file}", &config_file)
        .replace("{port}", &port)
        .replace("{data_dir}", &runtime.data_dir)
        .replace("{log_dir}", &runtime.log_dir)
}

fn run_service_command(cmd_str: &str, current_dir: Option<&Path>, detached: bool) -> Result<(), String> {
    let mut command = super::hidden_cmd::hidden_cmd("cmd");
    command.args(&["/c", cmd_str]);
    if let Some(dir) = current_dir {
        command.current_dir(dir);
    }

    if detached {
        command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        command.spawn().map_err(|e| format!("启动服务失败: {}", e))?;
        std::thread::sleep(Duration::from_millis(800));
        Ok(())
    } else {
        let output = command.output().map_err(|e| format!("执行服务命令失败: {}", e))?;
        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !stderr.is_empty() {
                Err(stderr)
            } else if !stdout.is_empty() {
                Err(stdout)
            } else {
                Err(format!("服务命令执行失败 (exit code: {})", output.status.code().unwrap_or(-1)))
            }
        }
    }
}

#[tauri::command]
pub fn get_running_services() -> Result<Vec<ServiceInfo>, String> {
    use super::project::registry;

    let all_projects = registry::registry();
    let mut result = Vec::new();

    for def in all_projects.iter().filter(|p| is_service_project(p)) {
        let status = service_status_for_def(def);
        result.push(ServiceInfo {
            name: def.id.clone(),
            status: status.status.clone().unwrap_or_else(|| if status.running { "running" } else { "stopped" }.to_string()),
            active_version: status.install_root.clone().unwrap_or_default(),
            port: status.port.map(|p| p.to_string()).unwrap_or_default(),
            pid: status.pid.map(|p| p as i32).unwrap_or(0),
        });
    }

    Ok(result)
}

#[tauri::command]
pub fn start_service(app: tauri::AppHandle, name: String, version: Option<String>) -> Result<(), String> {
    start_service_inner(name, version)?;
    let _ = crate::tray::rebuild_tray_menu(&app);
    Ok(())
}

pub(crate) fn start_service_inner(name: String, version: Option<String>) -> Result<(), String> {
    use super::project::registry;

    let def = registry::find_by_id(&name)
        .ok_or_else(|| format!("未找到服务定义: {}", name))?;
    if !is_service_project(&def) {
        return Err(format!("{} 不是服务项目", name));
    }

    let status = service_status_for_def(&def);
    if status.running {
        return Err(format!("服务 {} 已经运行中 (PID: {})", name, status.pid.unwrap_or(0)));
    }
    if status.status.as_deref() == Some("port_conflict") {
        return Err(format!("端口 {} 已被 {} 占用，请先处理端口冲突", status.port.map(|p| p.to_string()).unwrap_or_default(), status.process_name.unwrap_or_else(|| "其他进程".to_string())));
    }

    let runtime = resolve_service_runtime(&def, version.as_deref())?;
    let install_root = runtime.install_root.clone().ok_or_else(|| "未检测到本地安装，请先手动指定安装目录。".to_string())?;
    let start_cmd_template = def.start_cmd.as_deref().unwrap_or("");
    if start_cmd_template.is_empty() {
        return Err(format!("服务 {} 未配置启动命令", name));
    }

    for dir in &runtime.data_dirs {
        let path = Path::new(&dir.path);
        if !path.exists() {
            if dir.auto_create {
                fs::create_dir_all(path).map_err(|e| format!("创建{}失败: {}", dir.display_name, e))?;
            } else if dir.required_for_start {
                return Err(format!("{}不存在: {}。请先初始化或设置正确路径。", dir.display_name, dir.path));
            }
        }
    }

    let cmd_str = render_command(start_cmd_template, &runtime);
    let detached = def.service_start_mode.as_deref() == Some("detached");
    run_service_command(&cmd_str, Some(&install_root), detached)?;
    Ok(())
}

#[tauri::command]
pub fn stop_service(app: tauri::AppHandle, name: String) -> Result<(), String> {
    stop_service_inner(name)?;
    let _ = crate::tray::rebuild_tray_menu(&app);
    Ok(())
}

pub(crate) fn stop_service_inner(name: String) -> Result<(), String> {
    use super::project::registry;

    let def = registry::find_by_id(&name)
        .ok_or_else(|| format!("未找到服务定义: {}", name))?;
    if !is_service_project(&def) {
        return Err(format!("{} 不是服务项目", name));
    }

    let status = service_status_for_def(&def);
    if !status.running {
        if status.status.as_deref() == Some("port_conflict") {
            return Err(format!("端口被 {} 占用，但它不是 {} 服务进程，已拒绝停止。", status.process_name.unwrap_or_else(|| "其他进程".to_string()), def.display_name));
        }
        return Err(format!("服务 {} 未运行", name));
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
                    return Err(format!("停止命令失败，且进程 {} 不匹配服务定义，已拒绝强制终止: {}", proc_name, err));
                }
                let output = super::hidden_cmd::hidden_cmd("taskkill")
                    .args(&["/f", "/pid", &pid.to_string()])
                    .output()
                    .map_err(|e| format!("强制终止服务失败: {}", e))?;
                if !output.status.success() {
                    let err_msg = String::from_utf8_lossy(&output.stderr).to_string();
                    return Err(format!("正常关闭失败，且无法强制终止进程 (PID: {}): {}", pid, err_msg));
                }
            }
        } else {
            return Err(format!("无法安全停止 {}：{}。为避免数据损坏，未执行强制终止。", def.display_name, err));
        }
    }

    Ok(())
}

/// 用户在前端确认后强制终止服务进程（taskkill /f /t）。
/// 严格校验进程名属于服务定义，避免误杀；不再受 service_allow_force_kill 限制。
#[tauri::command]
pub fn force_stop_service(app: tauri::AppHandle, name: String) -> Result<(), String> {
    force_stop_service_inner(name)?;
    let _ = crate::tray::rebuild_tray_menu(&app);
    Ok(())
}

pub(crate) fn force_stop_service_inner(name: String) -> Result<(), String> {
    use super::project::registry;

    let def = registry::find_by_id(&name)
        .ok_or_else(|| format!("未找到服务定义: {}", name))?;
    if !is_service_project(&def) {
        return Err(format!("{} 不是服务项目", name));
    }

    let status = service_status_for_def(&def);
    let Some(pid) = status.pid else {
        return Err(format!("未检测到 {} 服务进程，无法强制终止", def.display_name));
    };

    // 二次校验进程名属于该服务，防止误杀同 PID 复用情况
    let proc_name = status.process_name.as_deref().unwrap_or_default();
    if proc_name.is_empty() || !process_matches_def(proc_name, &def) {
        return Err(format!(
            "PID {} 对应的进程 {} 与 {} 服务定义不匹配，已拒绝强制终止",
            pid,
            if proc_name.is_empty() { "<unknown>" } else { proc_name },
            def.display_name
        ));
    }

    let output = super::hidden_cmd::hidden_cmd("taskkill")
        .args(&["/f", "/t", "/pid", &pid.to_string()])
        .output()
        .map_err(|e| format!("强制终止服务失败: {}", e))?;
    if !output.status.success() {
        let err_msg = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!("强制终止进程失败 (PID: {}): {}", pid, err_msg));
    }
    Ok(())
}
