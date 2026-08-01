use std::fs;
use std::path::PathBuf;
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MirrorInfo {
    pub tool: String,
    pub display_name: String,
    pub current: String,
    pub mirror_name: String,
    pub options: Vec<super::project::types::MirrorOption>,
    pub config_file_desc: Option<String>,
}

fn classify_mirror(url_str: &str, _tool: &str) -> String {
    let url_lower = url_str.to_lowercase();
    if url_lower.contains("npmmirror.com") || url_lower.contains("aliyun.com") || url_lower.contains("taobao.org") {
        return "Aliyun / Taobao".to_string();
    }
    if url_lower.contains("tsinghua.edu.cn") {
        return "Tsinghua".to_string();
    }
    if url_lower.contains("tencent.com") || url_lower.contains("tencentcloud") {
        return "Tencent".to_string();
    }
    if url_lower.contains("rsproxy.cn") {
        return "Rsproxy".to_string();
    }
    if url_lower.contains("npmjs.org")
        || url_lower.contains("pypi.org")
        || url_lower.contains("golang.org")
        || url_lower.contains("crates.io-index")
        || url_lower.contains("maven.org")
        || url_lower.contains("apache.org")
    {
        return "Official".to_string();
    }
    "Custom".to_string()
}

#[tauri::command]
pub fn get_mirrors_list() -> Result<Vec<MirrorInfo>, String> {
    let registry = super::project::registry::registry();
    let mut mirrors = Vec::new();
    let user_profile = std::env::var("USERPROFILE").unwrap_or_default();
    let app_data = std::env::var("APPDATA").unwrap_or_default();

    for proj in &registry {
        for pm in &proj.package_managers {
            if let Some(ref options) = pm.mirror_options {
                if !options.is_empty() {
                    let mut current = String::new();
                    let mut mirror_name;

                    // 1. Try detect cmd
                    if let Some(ref detect_cmd) = pm.mirror_detect_cmd {
                        let pm_exe = super::pkg::find_pm_executable(&pm.id, &proj.id);
                        let resolved_cmd = if detect_cmd.starts_with(&pm.id) {
                            detect_cmd.replacen(&pm.id, &pm_exe, 1)
                        } else {
                            detect_cmd.clone()
                        };
                        if let Ok(out) = super::pkg::execute_command_string(&resolved_cmd) {
                            let trimmed = out.trim().to_string();
                            if !trimmed.is_empty() {
                                current = trimmed;
                            }
                        }
                    }

                    // 2. Try file-based detection
                    if current.is_empty() {
                        if let Some(ref config_file_tpl) = pm.mirror_config_file {
                            let config_path_str = config_file_tpl
                                .replace("{home}", &user_profile)
                                .replace("{appdata}", &app_data);
                            let config_path = PathBuf::from(&config_path_str);
                            if config_path.exists() {
                                if let Ok(content) = fs::read_to_string(&config_path) {
                                    if let Some(ref regex_str) = pm.mirror_detect_file_regex {
                                        if let Ok(re) = regex::Regex::new(regex_str) {
                                            if let Some(cap) = re.captures(&content) {
                                                if let Some(m) = cap.get(1) {
                                                    current = m.as_str().trim().to_string();
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // If still empty, use default official url
                    if current.is_empty() {
                        if let Some(first_opt) = options.first() {
                            current = first_opt.url.clone();
                        }
                    }

                    // Classify mirror name
                    mirror_name = classify_mirror(&current, &pm.id);

                    // Find if it matches any mirror option precisely
                    for opt in options {
                        if opt.url.to_lowercase() == current.to_lowercase()
                            || current.to_lowercase().contains(&opt.url.to_lowercase())
                            || opt.mirror_type.to_lowercase() == mirror_name.to_lowercase()
                        {
                            mirror_name = opt.name.clone();
                            break;
                        }
                    }

                    mirrors.push(MirrorInfo {
                        tool: pm.id.clone(),
                        display_name: pm.display_name.clone(),
                        current,
                        mirror_name,
                        options: options.clone(),
                        config_file_desc: pm.mirror_config_desc.clone(),
                    });
                }
            }
        }
    }

    Ok(mirrors)
}

#[tauri::command]
pub fn set_mirror(tool: String, mirror_type: String) -> Result<(), String> {
    let registry = super::project::registry::registry();
    let user_profile = std::env::var("USERPROFILE").unwrap_or_default();
    let app_data = std::env::var("APPDATA").unwrap_or_default();

    for proj in &registry {
        for pm in &proj.package_managers {
            if pm.id.to_lowercase() == tool.to_lowercase() || proj.pkg_manager.as_deref().map(|s| s.to_lowercase()) == Some(tool.to_lowercase()) {
                if let Some(ref options) = pm.mirror_options {
                    for opt in options {
                        if opt.mirror_type.to_lowercase() == mirror_type.to_lowercase() {
                            // 1. Try file-based configuration
                            if let Some(ref config_file_tpl) = pm.mirror_config_file {
                                let config_path_str = config_file_tpl
                                    .replace("{home}", &user_profile)
                                    .replace("{appdata}", &app_data);
                                let config_path = PathBuf::from(&config_path_str);
                                
                                if let Some(ref content) = opt.config_content {
                                    if !content.is_empty() {
                                        if let Some(parent) = config_path.parent() {
                                            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                                        }
                                        fs::write(&config_path, content).map_err(|e| e.to_string())?;
                                    } else {
                                        let _ = fs::remove_file(&config_path);
                                    }
                                } else {
                                    let _ = fs::remove_file(&config_path);
                                }
                                
                                // Specific cleanup for cargo config old file
                                if pm.id == "cargo" || pm.id == "rust" {
                                    let cargo_config_old = PathBuf::from(&user_profile).join(".cargo").join("config");
                                    let _ = fs::remove_file(&cargo_config_old);
                                }
                                return Ok(());
                            }

                            // 2. Try command-based configuration
                            if let Some(ref tpl) = pm.mirror_cmd_template {
                                let cmd = tpl.replace("{url}", &opt.url);
                                let output = super::hidden_cmd::hidden_cmd("cmd")
                                     .args(&["/c", &cmd])
                                     .output();
                                match output {
                                     Ok(out) => {
                                         if out.status.success() {
                                             return Ok(());
                                         } else {
                                             let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                                             let err_msg = if stderr.is_empty() {
                                                 format!("命令执行失败 (exit code: {})", out.status.code().unwrap_or(-1))
                                             } else {
                                                 stderr
                                             };
                                             return Err(format!("配置镜像失败: {}", err_msg));
                                         }
                                     }
                                     Err(e) => return Err(format!("执行配置命令失败: {}", e)),
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Err(format!("未配置支持该镜像的工具: {}", tool))
}
