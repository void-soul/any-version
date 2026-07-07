
use std::path::{Path, PathBuf};
use std::fs;
use std::sync::OnceLock;
use super::project::types::PackageManagerDef;

/// 获取用户主目录（统一入口，避免各模块重复实现）
pub fn get_home_dir() -> PathBuf {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let home = if home.is_empty() {
        std::env::var("HOME").unwrap_or_default()
    } else {
        home
    };
    if home.is_empty() {
        PathBuf::from(".")
    } else {
        PathBuf::from(home)
    }
}

/// 获取全局共享的 HTTP Client 单例，避免每次请求都重建连接池
pub fn get_http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent("Any-Version-Manager")
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

/// 在 PATH 中查找可执行文件的绝对路径（Windows 自动补齐 .exe/.cmd/.bat 后缀）
pub fn find_in_path(exe_name: &str) -> Option<PathBuf> {
    let names: Vec<String> = {
        let lower = exe_name.to_lowercase();
        if lower.ends_with(".exe") || lower.ends_with(".cmd") || lower.ends_with(".bat") {
            vec![exe_name.to_string()]
        } else {
            vec![
                exe_name.to_string(),
                format!("{}.exe", exe_name),
                format!("{}.cmd", exe_name),
            ]
        }
    };
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            for name in &names {
                let full = dir.join(name);
                if full.is_file() {
                    return Some(full);
                }
            }
        }
    }
    None
}

/// Expand {home} placeholder in path strings
pub fn expand_home(path: &str) -> String {
    if path.contains("{home}") {
        path.replace("{home}", &get_home_dir().to_string_lossy())
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
    find_in_path(name).is_some()
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

/// 在 XML 配置文件中原地替换指定 key 的 value 属性值。
///
/// 匹配形如：`<add key="KEY" value="OLD_VALUE" />`
/// 或：`<add key="KEY" value="OLD_VALUE"/>`
/// 将 value 替换为 `new_value`，其余内容保持不变。
///
/// 如果文件不存在或 key 不存在，则静默跳过（不报错）。
pub fn write_xml_config_key(config_path: &str, key: &str, new_value: &str) -> Result<(), String> {
    let path = std::path::Path::new(config_path);
    if !path.exists() {
        return Ok(()); // 文件不存在，静默跳过
    }

    let content = fs::read_to_string(path).map_err(|e| format!("读取 {} 失败: {}", config_path, e))?;

    // 构造匹配目标 key 的正则，替换其 value 属性
    let pattern = format!(
        r#"(<add\s+key\s*=\s*"{}"\s+value\s*=\s*")[^"]*(")"#,
        regex::escape(key)
    );
    let re = regex::Regex::new(&pattern).map_err(|e| e.to_string())?;

    if !re.is_match(&content) {
        return Ok(()); // key 不存在，静默跳过
    }

    let new_content = re.replace_all(&content, |caps: &regex::Captures| {
        format!("{}{}{}", &caps[1], new_value, &caps[2])
    }).to_string();

    if new_content != content {
        fs::write(path, new_content).map_err(|e| format!("写入 {} 失败: {}", config_path, e))?;
    }

    Ok(())
}

/// 根据 PackageManagerDef 中的 `cache_config_source.write_keys` 定义，
/// 批量将迁移后的目标路径写回对应的配置文件。
///
/// `base_path`：迁移目标的根目录（junction 指向的目录）。
/// 每个 write_key 的实际值 = base_path / value_suffix（或直接 base_path）。
pub fn apply_cache_config_writes(pm: &crate::commands::project::types::PackageManagerDef, base_path: &str) {
    let source = match &pm.cache_config_source {
        Some(s) => s,
        None => return,
    };
    let write_keys = match &source.write_keys {
        Some(k) if !k.is_empty() => k,
        _ => return,
    };

    let user_home = expand_home("{home}");

    // 找到第一个存在的配置文件路径
    let config_path = source.paths.iter()
        .map(|p| p.replace("{home}", &user_home))
        .find(|p| std::path::Path::new(p).exists());

    let config_path = match config_path {
        Some(p) => p,
        None => return, // 配置文件不存在，跳过
    };

    for wk in write_keys {
        let value = match &wk.value_suffix {
            Some(suffix) => std::path::Path::new(base_path)
                .join(suffix)
                .to_string_lossy()
                .to_string(),
            None => base_path.to_string(),
        };
        let _ = write_xml_config_key(&config_path, &wk.key, &value);
    }
}

static RESOURCE_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 设置打包环境下的静态资源目录
pub fn set_resource_dir(path: PathBuf) {
    let _ = RESOURCE_DIR.set(path);
}

/// 获取打包环境下的静态资源目录
pub fn get_resource_dir() -> Option<PathBuf> {
    RESOURCE_DIR.get().cloned()
}

