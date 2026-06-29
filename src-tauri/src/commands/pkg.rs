use serde::{Serialize, Deserialize};
use std::path::PathBuf;
use crate::commands::config::load_config;
use crate::commands::project::registry;

#[derive(Serialize, Clone, Debug)]
pub struct PackageInfo {
    pub name: String,
    pub current_version: String,
    pub latest_version: String,
    pub status: String, // "latest" | "outdated"
    pub homepage: String,
}

// Structs for parsing different JSON list formats
#[derive(Deserialize)]
struct NpmList {
    dependencies: Option<std::collections::HashMap<String, NpmDep>>,
}

#[derive(Deserialize)]
struct NpmDep {
    version: Option<String>,
}

#[derive(Deserialize)]
struct NpmOutdatedItem {
    latest: String,
}

type NpmOutdated = std::collections::HashMap<String, NpmOutdatedItem>;

#[derive(Deserialize)]
struct PipPackage {
    name: String,
    version: String,
}

#[derive(Deserialize)]
struct PipOutdated {
    name: String,
    latest_version: String,
}

pub fn find_pm_executable(pm_id: &str, project_id: &str) -> String {
    let config = load_config();
    let link_dir = PathBuf::from(&config.links_dir).join(project_id);
    
    if let Some(project) = registry::find_by_id(project_id) {
        if let Some(pm) = project.package_managers.iter().find(|m| m.id == pm_id) {
            if let Some(ref run_args) = pm.run_via_runtime_args {
                let runtime_exe_name = project.version_exe.as_deref().unwrap_or(project_id);
                let active_runtime = if cfg!(windows) {
                    if !runtime_exe_name.ends_with(".exe") {
                        link_dir.join(format!("{}.exe", runtime_exe_name))
                    } else {
                        link_dir.join(runtime_exe_name)
                    }
                } else {
                    link_dir.join(runtime_exe_name)
                };
                if active_runtime.exists() {
                    return format!("\"{}\" {}", active_runtime.to_string_lossy(), run_args.join(" "));
                }
            }
        }
    }

    let active_exe = if cfg!(windows) {
        link_dir.join(format!("{}.cmd", pm_id))
    } else {
        link_dir.join(pm_id)
    };
    if active_exe.exists() {
        active_exe.to_string_lossy().to_string()
    } else {
        pm_id.to_string()
    }
}

pub fn execute_command_string(cmd_str: &str) -> Result<String, String> {
    // Correctly split command string respecting quotes (like "C:\path to python\python.exe" -m pip list)
    let parts = if cmd_str.contains('"') {
        let mut result = Vec::new();
        let mut in_quotes = false;
        let mut current = String::new();
        for c in cmd_str.chars() {
            if c == '"' {
                in_quotes = !in_quotes;
            } else if c == ' ' && !in_quotes {
                if !current.is_empty() {
                    result.push(current.clone());
                    current.clear();
                }
            } else {
                current.push(c);
            }
        }
        if !current.is_empty() {
            result.push(current);
        }
        result
    } else {
        cmd_str.split_whitespace().map(|s| s.to_string()).collect()
    };

    if parts.is_empty() {
        return Err("空命令".to_string());
    }
    
    let mut cmd = super::hidden_cmd::hidden_cmd(&parts[0]);
    if parts.len() > 1 {
        cmd.args(&parts[1..]);
    }
    let output = cmd.output().map_err(|e| format!("执行命令失败: {}", e))?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[tauri::command]
pub fn get_global_packages(sdk_name: String) -> Result<Vec<PackageInfo>, String> {
    let (project, pm) = registry::registry().into_iter()
        .find_map(|p| {
            if p.id.eq_ignore_ascii_case(&sdk_name) {
                p.package_managers.iter().find(|m| m.pkg_list_cmd.is_some()).map(|m| (p.clone(), m.clone()))
            } else {
                p.package_managers.iter().find(|m| m.id.eq_ignore_ascii_case(&sdk_name)).map(|m| (p.clone(), m.clone()))
            }
        })
        .ok_or_else(|| format!("未找到项目或包管理器: {}", sdk_name))?;

    let list_cmd = pm.pkg_list_cmd.as_ref()
        .ok_or_else(|| format!("{} 不支持全局包管理（未配置 pkg_list_cmd）", pm.display_name))?;

    let pm_exe = find_pm_executable(&pm.id, &project.id);
    let resolved_list_cmd = if list_cmd.starts_with(&pm.id) {
        list_cmd.replacen(&pm.id, &pm_exe, 1)
    } else {
        list_cmd.clone()
    };

    let stdout = execute_command_string(&resolved_list_cmd)?;
    let format = pm.pkg_list_format.as_deref().unwrap_or("text_lines");

    let mut list = Vec::new();
    let mut outdated_map = std::collections::HashMap::new();

    if let Some(ref outdated_cmd) = pm.pkg_outdated_cmd {
        let resolved_outdated_cmd = if outdated_cmd.starts_with(&pm.id) {
            outdated_cmd.replacen(&pm.id, &pm_exe, 1)
        } else {
            outdated_cmd.clone()
        };
        if let Ok(outdated_stdout) = execute_command_string(&resolved_outdated_cmd) {
            let out_format = pm.pkg_outdated_format.as_deref().unwrap_or("");
            if out_format == "npm_outdated_json" {
                if let Some(start_idx) = outdated_stdout.find('{') {
                    let json_slice = &outdated_stdout[start_idx..];
                    if let Ok(parsed) = serde_json::from_str::<NpmOutdated>(json_slice) {
                        for (name, item) in parsed {
                            outdated_map.insert(name.to_lowercase(), item.latest);
                        }
                    }
                }
            } else if out_format == "pip_outdated_json" {
                if let Ok(parsed) = serde_json::from_str::<Vec<PipOutdated>>(&outdated_stdout) {
                    for op in parsed {
                        outdated_map.insert(op.name.to_lowercase(), op.latest_version);
                    }
                }
            }
        }
    }

    let homepage_template = pm.pkg_homepage_template.as_deref().unwrap_or("https://www.npmjs.com/package/{pkg}");

    match format {
        "npm_json" => {
            if let Some(start_idx) = stdout.find('{') {
                let json_slice = &stdout[start_idx..];
                if let Ok(parsed) = serde_json::from_str::<NpmList>(json_slice) {
                    if let Some(deps) = parsed.dependencies {
                        for (name, dep) in deps {
                            let current = dep.version.unwrap_or_else(|| "unknown".to_string());
                            let mut latest = current.clone();
                            let mut status = "latest".to_string();
                            if let Some(lv) = outdated_map.get(&name.to_lowercase()) {
                                latest = lv.clone();
                                status = "outdated".to_string();
                            }
                            list.push(PackageInfo {
                                name: name.clone(),
                                current_version: current,
                                latest_version: latest,
                                status,
                                homepage: homepage_template.replace("{pkg}", &name),
                            });
                        }
                    }
                }
            }
        }
        "pip_json" => {
            if let Ok(pkgs) = serde_json::from_str::<Vec<PipPackage>>(&stdout) {
                for p in pkgs {
                    let current = p.version;
                    let mut latest = current.clone();
                    let mut status = "latest".to_string();
                    if let Some(lv) = outdated_map.get(&p.name.to_lowercase()) {
                        latest = lv.clone();
                        status = "outdated".to_string();
                    }
                    list.push(PackageInfo {
                        name: p.name.clone(),
                        current_version: current,
                        latest_version: latest,
                        status,
                        homepage: homepage_template.replace("{pkg}", &p.name),
                    });
                }
            }
        }
        "yarn_json" => {
            for line in stdout.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() { continue; }
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
                    if val.get("type").and_then(|v| v.as_str()) == Some("info") {
                        if let Some(data) = val.get("data").and_then(|v| v.as_str()) {
                            let clean = data.trim_matches('"').trim_end_matches('\n');
                            let mut parts = clean.rsplitn(2, '@');
                            let ver = parts.next().unwrap_or("unknown").to_string();
                            let name = parts.next().unwrap_or("").to_string();
                            if !name.is_empty() {
                                list.push(PackageInfo {
                                    name: name.clone(),
                                    current_version: ver.clone(),
                                    latest_version: ver,
                                    status: "latest".to_string(),
                                    homepage: homepage_template.replace("{pkg}", &name),
                                });
                            }
                        }
                    }
                }
            }
        }
        "pnpm_json" => {
            if let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(&stdout) {
                for item in &parsed {
                    if let Some(deps) = item.get("dependencies").and_then(|v| v.as_object()) {
                        for (name, dep) in deps {
                            let ver = dep.get("version")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown")
                                .to_string();
                            list.push(PackageInfo {
                                name: name.clone(),
                                current_version: ver.clone(),
                                latest_version: ver,
                                status: "latest".to_string(),
                                homepage: homepage_template.replace("{pkg}", &name),
                            });
                        }
                    }
                }
            }
        }
        _ => {
            for line in stdout.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() { continue; }
                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                if parts.len() >= 2 {
                    let name = parts[0].to_string();
                    let ver = parts[1].to_string();
                    list.push(PackageInfo {
                        name: name.clone(),
                        current_version: ver.clone(),
                        latest_version: ver,
                        status: "latest".to_string(),
                        homepage: homepage_template.replace("{pkg}", &name),
                    });
                }
            }
        }
    }

    Ok(list)
}

#[tauri::command]
pub fn upgrade_global_package(sdk_name: String, pkg_name: String) -> Result<(), String> {
    if pkg_name.trim().is_empty() {
        return Err("包名不能为空".to_string());
    }

    let (project, pm) = registry::registry().into_iter()
        .find_map(|p| {
            if p.id.eq_ignore_ascii_case(&sdk_name) {
                p.package_managers.iter().find(|m| m.pkg_upgrade_cmd_template.is_some()).map(|m| (p.clone(), m.clone()))
            } else {
                p.package_managers.iter().find(|m| m.id.eq_ignore_ascii_case(&sdk_name)).map(|m| (p.clone(), m.clone()))
            }
        })
        .ok_or_else(|| format!("未找到项目或包管理器: {}", sdk_name))?;

    let upgrade_template = pm.pkg_upgrade_cmd_template.as_ref()
        .ok_or_else(|| format!("{} 不支持升级包（未配置 upgrade_cmd）", pm.display_name))?;

    let pm_exe = find_pm_executable(&pm.id, &project.id);
    let resolved_upgrade_template = if upgrade_template.starts_with(&pm.id) {
        upgrade_template.replacen(&pm.id, &pm_exe, 1)
    } else {
        upgrade_template.clone()
    };

    let final_cmd = resolved_upgrade_template.replace("{pkg}", pkg_name.trim());
    let _output = execute_command_string(&final_cmd)?;
    
    // Simple check: if command executed without panic or tauri error, consider success
    Ok(())
}
