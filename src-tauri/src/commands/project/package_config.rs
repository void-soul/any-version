//! SDK package-manager configuration adapters.

use super::types::PackageManagerDef;
use crate::commands::foundation::expand_path as expand_home;
use crate::commands::utils::write_xml_config_key;
use std::{fs, path::Path};

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
                        resolved = Path::new(&resolved)
                            .join(suffix)
                            .to_string_lossy()
                            .to_string();
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
        let mut expanded = raw_path
            .replace("{home}", &user_home)
            .replace("{links_dir}", &links_dir);

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
                                    resolved_val = caps
                                        .get(1)
                                        .map(|m| m.as_str().to_string())
                                        .unwrap_or_default();
                                }
                            }
                        }
                    }
                } else if source.parser_type.eq_ignore_ascii_case("properties") {
                    for line in content.lines() {
                        let trimmed = line.trim();
                        if trimmed.starts_with('#')
                            || trimmed.starts_with(';')
                            || trimmed.is_empty()
                        {
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

                let mut resolved = resolved_val
                    .trim_matches('"')
                    .trim_matches('\'')
                    .trim()
                    .to_string();
                if !resolved.is_empty() {
                    // Apply replacements
                    for (from, to) in &source.replacements {
                        let to_expanded = to
                            .replace("{home}", &user_home)
                            .replace("{links_dir}", &links_dir);
                        resolved = resolved.replace(from, &to_expanded);
                    }
                    // Apply suffix if specified
                    if let Some(ref suffix) = source.suffix {
                        if !resolved.ends_with(suffix) {
                            resolved = Path::new(&resolved)
                                .join(suffix)
                                .to_string_lossy()
                                .to_string();
                        }
                    }
                    return Some(resolved);
                }
            }
        }
    }

    None
}

/// Dynamic description builder for package manager caches
pub fn cache_detect_evidence_dynamic(
    _pm_id: &str,
    resolved: &str,
    pm_def: &PackageManagerDef,
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

/// 根据 PackageManagerDef 中的 `cache_config_source.write_keys` 定义，
/// 批量将迁移后的目标路径写回对应的配置文件。
///
/// `base_path`：迁移目标的根目录（junction 指向的目录）。
/// 每个 write_key 的实际值 = base_path / value_suffix（或直接 base_path）。
pub fn apply_cache_config_writes(pm: &PackageManagerDef, base_path: &str) {
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
    let config_path = source
        .paths
        .iter()
        .map(|p| p.replace("{home}", &user_home))
        .find(|p| std::path::Path::new(p).exists());

    let config_path = match config_path {
        Some(p) => p,
        None => {
            crate::exit_log::exit_log(&format!(
                "[apply_cache_config_writes] {}: 配置文件不存在，跳过缓存路径写入",
                pm.id
            ));
            return;
        }
    };

    for wk in write_keys {
        let value = match &wk.value_suffix {
            Some(suffix) => std::path::Path::new(base_path)
                .join(suffix)
                .to_string_lossy()
                .to_string(),
            None => base_path.to_string(),
        };
        if let Err(e) = write_xml_config_key(&config_path, &wk.key, &value) {
            crate::exit_log::exit_log(&format!(
                "[apply_cache_config_writes] {}: 写入 {} 失败: {}",
                pm.id, wk.key, e
            ));
        }
    }
}
