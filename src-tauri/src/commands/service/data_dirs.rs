use super::*;

pub(super) fn read_value_from_ini_or_conf(content: &str, key: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        let lower = trimmed.to_lowercase();
        if lower.starts_with(key) {
            let rest = &trimmed[key.len()..];
            if rest.starts_with('=')
                || rest.starts_with(':')
                || rest.starts_with(' ')
                || rest.starts_with('\t')
            {
                let splitter = if rest.contains('=') {
                    Some('=')
                } else if rest.contains(':') {
                    Some(':')
                } else {
                    None
                };
                if let Some(sp) = splitter {
                    if let Some(pos) = trimmed.find(sp) {
                        let val = trimmed[pos + 1..]
                            .trim()
                            .trim_matches('"')
                            .trim_matches('\'')
                            .to_string();
                        if !val.is_empty() {
                            return Some(val);
                        }
                    }
                } else {
                    // Space separated
                    let fields = trimmed.split_whitespace().collect::<Vec<_>>();
                    if fields.len() >= 2 && fields[0].to_lowercase() == key {
                        let val = fields[1].trim_matches('"').trim_matches('\'').to_string();
                        if !val.is_empty() {
                            return Some(val);
                        }
                    }
                }
            }
        }
    }
    None
}

pub(super) fn read_path_from_config_file(
    conf_path: &Path,
    project_id: &str,
    dir_id: &str,
) -> Option<String> {
    if !conf_path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(conf_path).ok()?;
    match project_id {
        "mysql" => {
            if dir_id == "data" {
                if let Some(val) = read_value_from_ini_or_conf(&content, "datadir") {
                    return Some(val);
                }
                if let Some(val) = read_value_from_ini_or_conf(&content, "data-dir") {
                    return Some(val);
                }
            }
        }
        "postgresql" => {
            if dir_id == "data" {
                if let Some(val) = read_value_from_ini_or_conf(&content, "data_directory") {
                    return Some(val);
                }
            }
        }
        "redis" => {
            if dir_id == "data" {
                if let Some(val) = read_value_from_ini_or_conf(&content, "dir") {
                    return Some(val);
                }
            }
        }
        "mongodb" => {
            if dir_id == "data" {
                if let Some(val) = read_value_from_ini_or_conf(&content, "dbpath") {
                    return Some(val);
                }
            }
        }
        _ => {}
    }
    None
}

pub(crate) fn resolve_data_dir(
    def: &ProjectDef,
    dir_def: &DataDirDef,
    config: &Config,
    install_root: Option<&Path>,
) -> ResolvedDataDir {
    // 1. 优先使用 AnyVersion 托管的自定义数据目录路径
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

    // 2. 其次尝试从配置文件中读取
    if let Some(config_file) = resolve_config_file(def, install_root) {
        if let Some(path_from_config) =
            read_path_from_config_file(&config_file, &def.id, &dir_def.id)
        {
            let expanded = expand_path_template(&path_from_config, install_root);
            let path_buf = PathBuf::from(&expanded);
            let final_path = if path_buf.is_absolute() {
                path_buf
            } else if let Some(root) = install_root {
                root.join(path_buf)
            } else {
                path_buf
            };
            return ResolvedDataDir {
                id: dir_def.id.clone(),
                display_name: dir_def.display_name.clone(),
                path: final_path.to_string_lossy().to_string(),
                kind: dir_def.kind.clone(),
                source: "config".to_string(),
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
