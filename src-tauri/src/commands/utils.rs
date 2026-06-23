
use std::path::Path;
use std::fs;
use super::project::types::PackageManagerDef;

/// Expand {home} placeholder in path strings
pub fn expand_home(path: &str) -> String {
    if path.contains("{home}") {
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_default();
        path.replace("{home}", &home)
    } else {
        path.to_string()
    }
}

/// Generic configuration-file based cache resolver
pub fn resolve_custom_cache_path(pm: &PackageManagerDef) -> Option<String> {
    let source = pm.cache_config_source.as_ref()?;
    
    // 1. Check environment variables if specified
    for env_var in &source.env_vars {
        if let Ok(val) = std::env::var(env_var) {
            let val_trimmed = val.trim();
            if !val_trimmed.is_empty() {
                let mut resolved = val_trimmed.to_string();
                if let Some(ref suffix) = source.suffix {
                    if !resolved.ends_with(suffix) {
                        resolved = Path::new(&resolved).join(suffix).to_string_lossy().to_string();
                    }
                }
                return Some(resolved);
            }
        }
    }
    
    // 2. Check the config files in path order
    let user_home = expand_home("{home}");
    let links_dir = crate::commands::config::load_config().links_dir;
    
    for raw_path in &source.paths {
        // Expand standard placeholders
        let mut expanded = raw_path.replace("{home}", &user_home).replace("{links_dir}", &links_dir);
        
        // Match other env vars in path: e.g. {MAVEN_HOME}
        if let Ok(re_var) = regex::Regex::new(r"\{([^}]+)\}") {
            for cap in re_var.captures_iter(raw_path) {
                let var_name = &cap[1];
                if var_name != "home" && var_name != "links_dir" {
                    if let Ok(val) = std::env::var(var_name) {
                        expanded = expanded.replace(&format!("{{{}}}", var_name), &val);
                    }
                }
            }
        }
        
        let file_path = Path::new(&expanded);
        if file_path.exists() {
            if let Ok(content) = fs::read_to_string(file_path) {
                let mut resolved_val = String::new();
                if source.parser_type.eq_ignore_ascii_case("xml") {
                    if let Some(pattern) = source.keys.first() {
                        // Strip XML comments to avoid commented out settings
                        if let Ok(re_comment) = regex::Regex::new(r"(?s)<!--.*?-->") {
                            let clean_content = re_comment.replace_all(&content, "");
                            if let Ok(re_tag) = regex::Regex::new(pattern) {
                                if let Some(caps) = re_tag.captures(&clean_content) {
                                    resolved_val = caps.get(1).map(|m| m.as_str().to_string()).unwrap_or_default();
                                }
                            }
                        }
                    }
                } else if source.parser_type.eq_ignore_ascii_case("properties") {
                    for line in content.lines() {
                        let trimmed = line.trim();
                        if trimmed.starts_with('#') || trimmed.starts_with(';') || trimmed.is_empty() {
                            continue;
                        }
                        if let Some(pos) = trimmed.find('=') {
                            let key = trimmed[..pos].trim();
                            let val = trimmed[pos + 1..].trim();
                            if source.keys.iter().any(|k| k == key) {
                                resolved_val = val.to_string();
                                break;
                            }
                        }
                    }
                }
                
                let mut resolved = resolved_val.trim_matches('"').trim_matches('\'').trim().to_string();
                if !resolved.is_empty() {
                    // Apply replacements
                    for (from, to) in &source.replacements {
                        let to_expanded = to.replace("{home}", &user_home).replace("{links_dir}", &links_dir);
                        resolved = resolved.replace(from, &to_expanded);
                    }
                    // Apply suffix if specified
                    if let Some(ref suffix) = source.suffix {
                        if !resolved.ends_with(suffix) {
                            resolved = Path::new(&resolved).join(suffix).to_string_lossy().to_string();
                        }
                    }
                    return Some(resolved);
                }
            }
        }
    }
    
    None
}

/// Run a command and capture its stdout as a trimmed string
pub fn get_cmd_output(cmd: &str, args: &[&str]) -> String {
    super::hidden_cmd::hidden_cmd(cmd)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Search for an executable in PATH (Windows compatible with .exe/.cmd/.bat)
pub fn is_exe_in_path(name: &str) -> bool {
    let exe = if cfg!(windows) {
        format!("{}.exe", name)
    } else {
        name.to_string()
    };
    let cmd = if cfg!(windows) {
        format!("{}.cmd", name)
    } else {
        String::new()
    };

    if let Ok(paths) = std::env::var("PATH") {
        for dir in std::env::split_paths(&paths) {
            if dir.join(&exe).exists() {
                return true;
            }
            if !cmd.is_empty() && dir.join(&cmd).exists() {
                return true;
            }
        }
    }
    false
}

/// Dynamic description builder for package manager caches
pub fn cache_detect_evidence_dynamic(
    _pm_id: &str,
    resolved: &str,
    pm_def: &crate::commands::project::types::PackageManagerDef,
) -> (String, String) {
    let display_name = &pm_def.display_name;
    
    if let Some(ref cmd) = pm_def.cache_detect_cmd {
        (
            format!("命令 `{}` 的输出", cmd),
            format!("{} 报告的缓存目录为: {}", display_name, resolved),
        )
    } else if let Some(ref env_var) = pm_def.cache_env_var {
        (
            format!("环境变量 {} 的路径", env_var),
            format!("从环境变量解析的 {} 缓存目录为: {}", display_name, resolved),
        )
    } else if let Some(ref default_path) = pm_def.cache_default_path {
        (
            format!("默认路径配置: {}", default_path),
            format!("检测到的 {} 缓存目录为: {}", display_name, resolved),
        )
    } else {
        (
            "默认配置路径".to_string(),
            format!("检测到的 {} 缓存目录为: {}", display_name, resolved),
        )
    }
}
