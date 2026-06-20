use std::fs;
use std::path::PathBuf;
use std::process::Command;
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MirrorInfo {
    pub tool: String,
    pub current: String,
    pub mirror_name: String,
}

fn get_cmd_output(cmd: &str, args: &[&str]) -> String {
    Command::new(cmd)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
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
    let user_profile = std::env::var("USERPROFILE").unwrap_or_default();
    let app_data = std::env::var("APPDATA").unwrap_or_default();

    // 1. npm
    let mut npm_reg = get_cmd_output("cmd", &["/c", "npm", "config", "get", "registry"]);
    if npm_reg.is_empty() {
        npm_reg = "https://registry.npmjs.org/".to_string();
    }
    let npm_name = classify_mirror(&npm_reg, "npm");

    // 2. pip
    let mut pip_reg = String::new();
    let pip_ini = PathBuf::from(&app_data).join("pip").join("pip.ini");
    if pip_ini.exists() {
        if let Ok(data) = fs::read_to_string(&pip_ini) {
            for line in data.lines() {
                let line_trimmed = line.trim();
                if line_trimmed.to_lowercase().starts_with("index-url") {
                    let parts = line_trimmed.splitn(2, '=').collect::<Vec<_>>();
                    if parts.len() == 2 {
                        pip_reg = parts[1].trim().to_string();
                        break;
                    }
                }
            }
        }
    }
    if pip_reg.is_empty() {
        pip_reg = "https://pypi.org/simple".to_string();
    }
    let pip_name = classify_mirror(&pip_reg, "pip");

    // 3. maven
    let mut mvn_reg = "https://repo.maven.apache.org/maven2".to_string();
    let m2_settings = PathBuf::from(&user_profile).join(".m2").join("settings.xml");
    if m2_settings.exists() {
        if let Ok(content) = fs::read_to_string(&m2_settings) {
            if content.contains("maven.aliyun.com") {
                mvn_reg = "https://maven.aliyun.com/repository/public".to_string();
            }
        }
    }
    let mvn_name = classify_mirror(&mvn_reg, "maven");

    // 4. go
    let mut go_proxy = get_cmd_output("cmd", &["/c", "go", "env", "GOPROXY"]);
    if go_proxy.is_empty() {
        go_proxy = "https://proxy.golang.org,direct".to_string();
    }
    let go_name = classify_mirror(&go_proxy, "go");

    // 5. rust
    let mut rust_reg = "https://github.com/rust-lang/crates.io-index".to_string();
    let mut cargo_config = PathBuf::from(&user_profile).join(".cargo").join("config.toml");
    if !cargo_config.exists() {
        cargo_config = PathBuf::from(&user_profile).join(".cargo").join("config");
    }
    if cargo_config.exists() {
        if let Ok(content) = fs::read_to_string(&cargo_config) {
            if content.contains("rsproxy.cn") {
                rust_reg = "https://rsproxy.cn".to_string();
            } else if content.contains("ustc.edu.cn") {
                rust_reg = "https://mirrors.ustc.edu.cn/crates.io-index".to_string();
            }
        }
    }
    let rust_name = classify_mirror(&rust_reg, "rust");

    Ok(vec![
        MirrorInfo { tool: "npm".to_string(), current: npm_reg, mirror_name: npm_name },
        MirrorInfo { tool: "pip".to_string(), current: pip_reg, mirror_name: pip_name },
        MirrorInfo { tool: "maven".to_string(), current: mvn_reg, mirror_name: mvn_name },
        MirrorInfo { tool: "go".to_string(), current: go_proxy, mirror_name: go_name },
        MirrorInfo { tool: "rust".to_string(), current: rust_reg, mirror_name: rust_name },
    ])
}

#[tauri::command]
pub fn set_mirror(tool: String, mirror_type: String) -> Result<(), String> {
    let user_profile = std::env::var("USERPROFILE").unwrap_or_default();
    let app_data = std::env::var("APPDATA").unwrap_or_default();
    let m_type = mirror_type.to_lowercase();
    let t_name = tool.to_lowercase();

    match t_name.as_str() {
        "npm" => {
            let url_val = match m_type.as_str() {
                "aliyun" => "https://registry.npmmirror.com/",
                "tencent" => "https://mirrors.cloud.tencent.com/npm/",
                _ => "https://registry.npmjs.org/",
            };
            let _ = Command::new("cmd").args(&["/c", "npm", "config", "set", "registry", url_val]).output();
            if crate::commands::cache::is_installed("yarn") {
                let _ = Command::new("cmd").args(&["/c", "yarn", "config", "set", "registry", url_val]).output();
            }
            if crate::commands::cache::is_installed("pnpm") {
                let _ = Command::new("cmd").args(&["/c", "pnpm", "config", "set", "registry", url_val]).output();
            }
        }
        "pip" => {
            let pip_ini = PathBuf::from(&app_data).join("pip").join("pip.ini");
            if m_type == "official" {
                let _ = fs::remove_file(&pip_ini);
            } else {
                let (url_val, host_val) = match m_type.as_str() {
                    "aliyun" => ("https://mirrors.aliyun.com/pypi/simple/", "mirrors.aliyun.com"),
                    "tsinghua" => ("https://pypi.tuna.tsinghua.edu.cn/simple", "pypi.tuna.tsinghua.edu.cn"),
                    _ => ("https://pypi.org/simple", "pypi.org"),
                };
                let _ = fs::create_dir_all(pip_ini.parent().unwrap());
                let content = format!("[global]\nindex-url = {}\ntrusted-host = {}\n", url_val, host_val);
                fs::write(&pip_ini, content).map_err(|e| e.to_string())?;
            }
        }
        "maven" => {
            let m2_settings = PathBuf::from(&user_profile).join(".m2").join("settings.xml");
            if m_type == "official" {
                let _ = fs::remove_file(&m2_settings);
            } else if m_type == "aliyun" {
                let _ = fs::create_dir_all(m2_settings.parent().unwrap());
                let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<settings xmlns="http://maven.apache.org/SETTINGS/1.0.0"
          xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
          xsi:schemaLocation="http://maven.apache.org/SETTINGS/1.0.0 https://maven.apache.org/xsd/settings-1.0.0.xsd">
  <mirrors>
    <mirror>
      <id>aliyunmaven</id>
      <mirrorOf>central</mirrorOf>
      <name>aliyun maven</name>
      <url>https://maven.aliyun.com/repository/public</url>
    </mirror>
  </mirrors>
</settings>"#;
                fs::write(&m2_settings, xml).map_err(|e| e.to_string())?;
            }
        }
        "go" => {
            let url_val = match m_type.as_str() {
                "aliyun" => "https://mirrors.aliyun.com/goproxy/,direct",
                "tsinghua" | "goproxy" => "https://goproxy.cn,direct",
                _ => "https://proxy.golang.org,direct",
            };
            if crate::commands::cache::is_installed("go") {
                let _ = Command::new("cmd").args(&["/c", "go", "env", "-w", &format!("GOPROXY={}", url_val)]).output();
            }
        }
        "rust" => {
            let cargo_config = PathBuf::from(&user_profile).join(".cargo").join("config.toml");
            let cargo_config_old = PathBuf::from(&user_profile).join(".cargo").join("config");
            if m_type == "official" {
                let _ = fs::remove_file(&cargo_config);
                let _ = fs::remove_file(&cargo_config_old);
            } else {
                let _ = fs::create_dir_all(cargo_config.parent().unwrap());
                let config_content = match m_type.as_str() {
                    "rsproxy" => r#"[source.crates-io]
replace-with = 'rsproxy'

[source.rsproxy]
registry = "https://rsproxy.cn/crates.io-index"

[source.rsproxy-sparse]
registry = "sparse+https://rsproxy.cn/index/"

[net]
git-fetch-with-cli = true
"#,
                    "tsinghua" => r#"[source.crates-io]
replace-with = 'tsinghua'

[source.tsinghua]
registry = "https://mirrors.tuna.tsinghua.edu.cn/git/crates.io-index"
"#,
                    _ => "",
                };
                if !config_content.is_empty() {
                    fs::write(&cargo_config, config_content).map_err(|e| e.to_string())?;
                    let _ = fs::remove_file(&cargo_config_old);
                }
            }
        }
        _ => return Err(format!("未知的工具: {}", tool)),
    }

    Ok(())
}
