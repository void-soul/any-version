use super::*;

pub(super) fn process_name_by_pid(pid: &str) -> Option<String> {
    let task_output = crate::commands::hidden_cmd::hidden_cmd("tasklist")
        .args(&["/fi", &format!("pid eq {}", pid), "/fo", "csv", "/nh"])
        .output()
        .ok()?;
    let task_text = String::from_utf8_lossy(&task_output.stdout)
        .trim()
        .to_string();
    if task_text.is_empty() {
        return None;
    }
    task_text
        .split(',')
        .next()
        .map(|p| p.trim_matches('"').to_string())
}

pub(super) fn is_service_project(def: &ProjectDef) -> bool {
    def.category == ProjectCategory::Service || def.is_service
}

pub(super) fn expand_path_template(template: &str, install_root: Option<&Path>) -> String {
    let mut path = crate::commands::utils::expand_home(template);
    if let Some(root) = install_root {
        path = path.replace("{install_root}", &root.to_string_lossy());
        path = path.replace("{dir}", &root.to_string_lossy());
    }
    path
}

pub(super) fn service_exe_names(def: &ProjectDef) -> Vec<String> {
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

pub(super) fn normalize_exe_name(name: &str) -> String {
    let lower = name.to_lowercase();
    if lower.ends_with(".exe") {
        lower
    } else {
        format!("{}.exe", lower)
    }
}

pub(super) fn process_matches_def(process_name: &str, def: &ProjectDef) -> bool {
    let process = normalize_exe_name(process_name);
    service_exe_names(def)
        .iter()
        .map(|n| normalize_exe_name(n))
        .any(|n| n == process)
}

pub(super) fn service_processes(def: &ProjectDef) -> Vec<ProcessInfo> {
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
        let output = crate::commands::hidden_cmd::hidden_cmd("tasklist")
            .args(&[
                "/fi",
                &format!("IMAGENAME eq {}", name),
                "/fo",
                "csv",
                "/nh",
            ])
            .output();
        let Ok(out) = output else {
            continue;
        };
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
            let Ok(pid) = fields[1].parse::<u32>() else {
                continue;
            };
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
pub(super) fn exe_path_by_pid(pid: u32) -> Option<String> {
    let output = crate::commands::hidden_cmd::hidden_cmd("wmic")
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
#[allow(dead_code)]
pub(super) fn detect_service_install_via_sc(def: &ProjectDef) -> Option<PathBuf> {
    if def.service_names.is_empty() {
        return None;
    }
    let patterns: Vec<regex::Regex> = def
        .service_names
        .iter()
        .filter_map(|p| {
            regex::RegexBuilder::new(p)
                .case_insensitive(true)
                .build()
                .ok()
        })
        .collect();
    if patterns.is_empty() {
        return None;
    }

    // sc query type= service state= all —— 列出所有服务名
    let output = crate::commands::hidden_cmd::hidden_cmd("sc")
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

#[allow(dead_code)]
pub(super) fn sc_service_install_root(service: &str) -> Option<PathBuf> {
    let output = crate::commands::hidden_cmd::hidden_cmd("sc")
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

pub(super) fn detect_install_root(
    def: &ProjectDef,
    config: &Config,
    version: Option<&str>,
) -> Result<Option<PathBuf>, String> {
    let delegation =
        crate::commands::project::scanner::get_project_delegation(config, &def.id, def);
    let use_symlink = delegation.create_symlink;

    if use_symlink {
        if let Some(version) = version.filter(|v| !v.trim().is_empty()) {
            let version_dir = PathBuf::from(&config.versions_dir)
                .join(&def.id)
                .join(version);
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

    let (_, local_root) = crate::commands::project::scanner::detect_install_source(def);
    if let Some(root) = local_root {
        return Ok(Some(PathBuf::from(root)));
    }

    Ok(None)
}

pub(super) fn resolve_config_file(
    def: &ProjectDef,
    install_root: Option<&Path>,
) -> Option<PathBuf> {
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

pub(super) fn resolve_port(def: &ProjectDef, config_file: Option<&Path>) -> Option<u16> {
    if let Some(conf) = config_file {
        let conf_name = conf
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
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

pub(crate) fn resolve_service_runtime(
    def: &ProjectDef,
    version: Option<&str>,
) -> Result<ServiceRuntime, String> {
    let config = load_config();
    let install_root = detect_install_root(def, &config, version)?;
    let config_file = resolve_config_file(def, install_root.as_deref());
    let port = resolve_port(def, config_file.as_deref());
    let mut data_dirs = Vec::new();

    for dir_def in &def.data_dirs {
        data_dirs.push(resolve_data_dir(
            def,
            dir_def,
            &config,
            install_root.as_deref(),
        ));
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

    Ok(ServiceRuntime {
        install_root,
        config_file,
        port,
        data_dirs,
        data_dir,
        log_dir,
    })
}
