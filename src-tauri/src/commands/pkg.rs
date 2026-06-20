use std::path::PathBuf;
use serde::{Serialize, Deserialize};
use crate::commands::config::load_config;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PackageInfo {
    pub name: String,
    pub current_version: String,
    pub latest_version: String,
    pub status: String,
    /// 该包的官网/主页地址（npm 或 PyPI），方便用户点击查看文档与源代码。
    pub homepage: String,
}

// NPM list JSON mapping
#[derive(Deserialize)]
struct NpmListDep {
    version: Option<String>,
}

#[derive(Deserialize)]
struct NpmList {
    dependencies: Option<std::collections::HashMap<String, NpmListDep>>,
}

// NPM outdated JSON mapping
#[derive(Deserialize)]
struct NpmOutdatedItem {
    latest: String,
}

type NpmOutdated = std::collections::HashMap<String, NpmOutdatedItem>;

// PIP list mapping
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

fn get_npm_path() -> String {
    let config = load_config();
    let active_npm = PathBuf::from(&config.links_dir).join("nodejs").join("npm.cmd");
    if active_npm.exists() {
        active_npm.to_string_lossy().to_string()
    } else {
        "npm".to_string()
    }
}

fn get_python_path() -> String {
    let config = load_config();
    let active_python = PathBuf::from(&config.links_dir).join("python").join("python.exe");
    if active_python.exists() {
        active_python.to_string_lossy().to_string()
    } else {
        "python".to_string()
    }
}

fn get_global_npm_packages() -> Result<Vec<PackageInfo>, String> {
    let npm = get_npm_path();

    // 1. Run npm list -g --depth=0 --json
    let list_out = super::hidden_cmd::hidden_cmd(&npm)
        .args(&["list", "-g", "--depth=0", "--json"])
        .output()
        .map_err(|e| format!("运行 npm list 失败: {}", e))?;

    let stdout_bytes = list_out.stdout;
    let mut list_data = NpmList { dependencies: None };

    if !stdout_bytes.is_empty() {
        // Skip potential leading warnings/text before JSON
        if let Some(start_idx) = stdout_bytes.iter().position(|&b| b == b'{') {
            let json_slice = &stdout_bytes[start_idx..];
            if let Ok(parsed) = serde_json::from_slice::<NpmList>(json_slice) {
                list_data = parsed;
            }
        }
    }

    // 2. Run npm outdated -g --json
    let outdated_out = super::hidden_cmd::hidden_cmd(&npm)
        .args(&["outdated", "-g", "--json"])
        .output()
        .map_err(|e| format!("运行 npm outdated 失败: {}", e))?;

    let mut outdated_data = NpmOutdated::new();
    let out_bytes = outdated_out.stdout;
    if !out_bytes.is_empty() {
        if let Some(start_idx) = out_bytes.iter().position(|&b| b == b'{') {
            let json_slice = &out_bytes[start_idx..];
            let _ = serde_json::from_slice::<NpmOutdated>(json_slice).map(|parsed| {
                outdated_data = parsed;
            });
        }
    }

    let mut list = Vec::new();
    if let Some(deps) = list_data.dependencies {
        for (name, dep) in deps {
            let current = dep.version.unwrap_or_else(|| "unknown".to_string());
            let mut latest = current.clone();
            let mut status = "latest".to_string();

            if let Some(out_info) = outdated_data.get(&name) {
                latest = out_info.latest.clone();
                status = "outdated".to_string();
            }

            list.push(PackageInfo {
                name: name.clone(),
                current_version: current,
                latest_version: latest,
                status,
                homepage: format!("https://www.npmjs.com/package/{}", name),
            });
        }
    }

    Ok(list)
}

fn get_global_pip_packages() -> Result<Vec<PackageInfo>, String> {
    let python = get_python_path();

    // 1. Run python -m pip list --format=json
    let list_out = super::hidden_cmd::hidden_cmd(&python)
        .args(&["-m", "pip", "list", "--format=json"])
        .output()
        .map_err(|e| format!("运行 pip list 失败: {}", e))?;

    if !list_out.status.success() {
        return Err(format!("pip list exit with error: {}", String::from_utf8_lossy(&list_out.stderr)));
    }

    let pkgs: Vec<PipPackage> = serde_json::from_slice(&list_out.stdout)
        .map_err(|e| format!("解析 pip list JSON 失败: {}", e))?;

    // 2. Run python -m pip list --outdated --format=json
    let outdated_out = super::hidden_cmd::hidden_cmd(&python)
        .args(&["-m", "pip", "list", "--outdated", "--format=json"])
        .output()
        .map_err(|e| format!("运行 pip list --outdated 失败: {}", e))?;

    let mut outdated_pkgs: Vec<PipOutdated> = Vec::new();
    if outdated_out.status.success() && !outdated_out.stdout.is_empty() {
        let _ = serde_json::from_slice::<Vec<PipOutdated>>(&outdated_out.stdout).map(|parsed| {
            outdated_pkgs = parsed;
        });
    }

    let mut outdated_map = std::collections::HashMap::new();
    for op in outdated_pkgs {
        outdated_map.insert(op.name.to_lowercase(), op.latest_version);
    }

    let mut list = Vec::new();
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
            homepage: format!("https://pypi.org/project/{}/", p.name),
        });
    }

    Ok(list)
}

#[tauri::command]
pub fn get_global_packages(sdk_name: String) -> Result<Vec<PackageInfo>, String> {
    let name_lower = sdk_name.to_lowercase();
    if name_lower == "nodejs" || name_lower == "npm" {
        get_global_npm_packages()
    } else if name_lower == "python" || name_lower == "pip" {
        get_global_pip_packages()
    } else {
        Err(format!("不支持的包管理器: {}", sdk_name))
    }
}

#[tauri::command]
pub fn upgrade_global_package(sdk_name: String, pkg_name: String) -> Result<(), String> {
    let name_lower = sdk_name.to_lowercase();
    if pkg_name.trim().is_empty() {
        return Err("包名不能为空".to_string());
    }

    let output = if name_lower == "nodejs" || name_lower == "npm" {
        let npm = get_npm_path();
        super::hidden_cmd::hidden_cmd(npm)
            .args(&["install", "-g", &format!("{}@latest", pkg_name.trim())])
            .output()
    } else if name_lower == "python" || name_lower == "pip" {
        let python = get_python_path();
        super::hidden_cmd::hidden_cmd(python)
            .args(&["-m", "pip", "install", "--upgrade", pkg_name.trim()])
            .output()
    } else {
        return Err(format!("不支持的包管理器: {}", sdk_name));
    }.map_err(|e| format!("执行命令失败: {}", e))?;

    if !output.status.success() {
        return Err(format!("升级失败: {}", String::from_utf8_lossy(&output.stderr)));
    }

    Ok(())
}
