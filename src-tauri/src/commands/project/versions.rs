//! 项目版本管理模块 -- 远程版本列表、安装、卸载、切换、本地注册。
//!
//! 从已删除的 sdk.rs 迁移而来，适配新的项目托管架构。
//! 使用 project_id（原 sdk_name）标识项目，通过 load_config() 获取 versions_dir/links_dir，
//! 通过 junction 实现版本切换。

use std::fs;
use std::path::{Path, PathBuf};
use serde::{Serialize, Deserialize};
use tauri::{AppHandle, Emitter};
use crate::commands::config::{load_config, get_base_dir};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  数据结构
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 下载进度事件
#[derive(Serialize, Clone)]
pub struct DownloadProgress {
    pub sdk: String,
    pub downloaded: u64,
    pub total: u64,
    pub pct: u8,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  远程版本 API 响应结构
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Deserialize)]
struct GoRelease {
    version: String,
    stable: bool,
}

#[derive(Deserialize)]
struct NodeRelease {
    version: String,
    lts: serde_json::Value,
}

#[derive(Deserialize)]
struct NugetVersions {
    versions: Vec<String>,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
}

#[derive(Deserialize)]
struct AdoptiumReleases {
    releases: Vec<String>,
}

#[derive(Deserialize)]
struct ZuluPackage {
    download_url: String,
    java_version: Vec<i32>,
    name: String,
}

#[derive(Deserialize)]
struct FlutterReleaseJSON {
    releases: Vec<FlutterRelease>,
}

#[derive(Deserialize)]
struct FlutterRelease {
    version: String,
    channel: String,
    archive: String,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  Tauri 命令
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 获取远程版本列表
#[tauri::command]
pub async fn project_list_remote_versions(id: String) -> Result<Vec<String>, String> {
    let client = reqwest::Client::builder()
        .user_agent("Any-Version-Manager")
        .build()
        .map_err(|e| e.to_string())?;

    match id.as_str() {
        "go" => {
            let releases: Vec<GoRelease> = client.get("https://go.dev/dl/?mode=json&include=all")
                .send().await.map_err(|e| e.to_string())?
                .json().await.map_err(|e| e.to_string())?;
            let versions = releases.into_iter()
                .filter(|r| r.stable)
                .map(|r| r.version.trim_start_matches("go").to_string())
                .take(100)
                .collect();
            Ok(versions)
        }
        "nodejs" => {
            let releases: Vec<NodeRelease> = client.get("https://nodejs.org/dist/index.json")
                .send().await.map_err(|e| e.to_string())?
                .json().await.map_err(|e| e.to_string())?;
            let versions = releases.into_iter()
                .map(|r| {
                    let v = r.version.trim_start_matches('v').to_string();
                    let lts_label = if r.lts.is_boolean() && r.lts.as_bool().unwrap_or(false) {
                        " (LTS)".to_string()
                    } else if r.lts.is_string() {
                        format!(" (LTS: {})", r.lts.as_str().unwrap_or_default())
                    } else {
                        "".to_string()
                    };
                    format!("{}{}", v, lts_label)
                })
                .take(120)
                .collect();
            Ok(versions)
        }
        "python" => {
            let data: NugetVersions = client.get("https://api.nuget.org/v3-flatcontainer/python/index.json")
                .send().await.map_err(|e| e.to_string())?
                .json().await.map_err(|e| e.to_string())?;
            let mut versions: Vec<String> = data.versions.into_iter()
                .filter(|v| !v.contains('-'))
                .collect();
            versions.reverse();
            versions.truncate(100);
            Ok(versions)
        }
        "bun" => {
            let releases: Vec<GithubRelease> = client.get("https://api.github.com/repos/oven-sh/bun/releases")
                .send().await.map_err(|e| e.to_string())?
                .json().await.map_err(|e| e.to_string())?;
            let versions = releases.into_iter()
                .map(|r| r.tag_name.trim_start_matches("bun-v").trim_start_matches('v').to_string())
                .collect();
            Ok(versions)
        }
        "rust" => {
            let releases: Vec<GithubRelease> = client.get("https://api.github.com/repos/rust-lang/rust/releases")
                .send().await.map_err(|e| e.to_string())?
                .json().await.map_err(|e| e.to_string())?;
            let versions = releases.into_iter()
                .filter(|r| !r.tag_name.contains('-'))
                .map(|r| r.tag_name.clone())
                .collect();
            Ok(versions)
        }
        "flutter" => {
            let data: FlutterReleaseJSON = client.get("https://storage.googleapis.com/flutter_infra_release/releases/releases_windows.json")
                .send().await.map_err(|e| e.to_string())?
                .json().await.map_err(|e| e.to_string())?;
            let versions = data.releases.into_iter()
                .filter(|r| r.channel == "stable")
                .map(|r| r.version)
                .collect();
            Ok(versions)
        }
        "java" => {
            let mut versions = Vec::new();
            // Adoptium 版本
            for major in &["21", "17", "11", "8"] {
                let adopt_url = format!("https://api.adoptium.net/v3/info/release_names?project=jdk&release_type=ga&os=windows&architecture=x64&image_type=jdk&version=[{},{})", major, major.parse::<i32>().unwrap() + 1);
                if let Ok(res) = client.get(&adopt_url).send().await {
                    if let Ok(data) = res.json::<AdoptiumReleases>().await {
                        for r in data.releases.into_iter().take(5) {
                            versions.push(format!("adoptium-{}", r.trim_start_matches("jdk-")));
                        }
                    }
                }
            }
            // Azul Zulu
            let zulu_url = "https://api.azul.com/metadata/v1/zulu/packages/?os=windows&arch=amd64&archive_type=zip&java_package_type=jdk&release_status=ga&latest=true&page_size=20";
            if let Ok(res) = client.get(zulu_url).send().await {
                if let Ok(pkgs) = res.json::<Vec<ZuluPackage>>().await {
                    for pkg in pkgs {
                        if pkg.name.contains("-ca-jdk") {
                            let v = pkg.java_version.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(".");
                            versions.push(format!("zulu-{}", v));
                        }
                    }
                }
            }
            versions.push("microsoft-21".to_string());
            versions.push("microsoft-17".to_string());
            versions.push("oracle-21".to_string());
            versions.push("oracle-17".to_string());
            Ok(versions)
        }
        "android" => {
            // Android 命令行工具：常见稳定构建号
            Ok(vec![
                "13114758".to_string(),
                "11076708".to_string(),
                "10406996".to_string(),
                "9477386".to_string(),
                "8512546".to_string(),
            ])
        }
        "harmony" => {
            // OpenHarmony / 鸿蒙：常见发行版本号
            Ok(vec![
                "5.0.5".to_string(),
                "5.0.3".to_string(),
                "4.1.0".to_string(),
                "4.0.0".to_string(),
            ])
        }
        "cuda" => {
            // CUDA Toolkit：常用版本供参考
            Ok(vec!["12.6.3".to_string(), "12.5.1".to_string(), "12.4.1".to_string(), "12.2.2".to_string(), "11.8.0".to_string()])
        }
        "ffmpeg" => {
            // FFmpeg Windows 构建来自 gyan.dev 或 BtbN
            Ok(vec!["7.1.1".to_string(), "7.1".to_string(), "7.0.2".to_string(), "6.1.2".to_string(), "6.0".to_string()])
        }
        "nginx" => Ok(vec!["1.26.1".to_string(), "1.26.0".to_string(), "1.24.0".to_string(), "1.22.1".to_string()]),
        "redis" => Ok(vec!["5.0.14.1".to_string(), "3.0.504".to_string()]),
        "mysql" => Ok(vec!["8.0.36".to_string(), "8.4.0".to_string(), "5.7.44".to_string()]),
        "mongodb" => Ok(vec!["7.0.5".to_string(), "6.0.13".to_string(), "5.0.24".to_string()]),
        "postgresql" => Ok(vec!["16.2".to_string(), "15.6".to_string(), "14.11".to_string()]),
        "maven" => Ok(vec!["3.9.6".to_string(), "3.8.8".to_string(), "3.6.3".to_string()]),
        "gradle" => Ok(vec!["8.6".to_string(), "8.5".to_string(), "7.6.4".to_string()]),
        "yarn" => Ok(vec!["1.22.19".to_string(), "3.8.1".to_string()]),
        "pnpm" => Ok(vec!["9.0.5".to_string(), "8.15.4".to_string()]),
        _ => Err(format!("不支持的项目类别: {}", id)),
    }
}

/// 安装指定版本（下载 -> 解压 -> 安装到 versions_dir -> 创建 junction -> 配置环境变量）
#[tauri::command]
pub async fn project_install_version(app: AppHandle, id: String, version: String) -> Result<(), String> {
    let config = load_config();
    let (download_url, file_ext) = get_download_url(&id, &version)?;

    // 1. 创建临时目录
    let (temp_dir, cleanup) = setup_temp_dir(&id)?;
    let archive_path = temp_dir.join(format!("archive.{}", file_ext));

    // 2. 下载（带进度事件）
    let id_cap = id.clone();
    let app_handle = app.clone();
    let dl_result = download_with_progress(&download_url, &archive_path, move |downloaded, total| {
        let pct = if total > 0 { (downloaded * 100 / total) as u8 } else { 0 };
        let _ = app_handle.emit("download-progress", DownloadProgress {
            sdk: id_cap.clone(),
            downloaded,
            total,
            pct,
        });
    }).await;

    if let Err(e) = dl_result {
        cleanup();
        return Err(format!("下载失败: {}", e));
    }

    // 3. 解压
    let extract_dir = temp_dir.join("extracted");
    let ext_result = if file_ext == "tar.gz" {
        extract_tar_gz(&archive_path, &extract_dir)
    } else if file_ext == "exe" {
        fs::create_dir_all(&extract_dir).map_err(|e| e.to_string())?;
        fs::copy(&archive_path, extract_dir.join(format!("{}.exe", id)))
            .map(|_| ())
            .map_err(|e| e.to_string())
    } else {
        unzip_file(&archive_path, &extract_dir)
    };

    if let Err(e) = ext_result {
        cleanup();
        return Err(format!("解压失败: {}", e));
    }

    // 4. 安装到 versions_dir
    let dest_dir = Path::new(&config.versions_dir).join(&id).join(&version);

    // python 特殊处理：解压后的目录结构不同
    let src_dir = if id == "python" {
        extract_dir.join("tools")
    } else {
        extract_dir
    };

    if let Err(e) = move_extract_to_dest(&src_dir, &dest_dir) {
        cleanup();
        return Err(format!("安装失败: {}", e));
    }

    cleanup();

    // 5. 后置配置（如 mysql 初始化）
    if id == "mysql" {
        let my_ini_path = dest_dir.join("my.ini");
        let data_dir = dest_dir.join("data");
        let clean_base = dest_dir.to_string_lossy().replace("\\", "/");
        let clean_data = data_dir.to_string_lossy().replace("\\", "/");

        let my_ini_content = format!(
            "[mysqld]\nport=3306\nbasedir={}\ndatadir={}\nmax_connections=200\ncharacter-set-server=utf8mb4\ndefault-storage-engine=INNODB\ndefault_authentication_plugin=mysql_native_password\n\n[mysql]\ndefault-character-set=utf8mb4\n\n[client]\nport=3306\ndefault-character-set=utf8mb4\n",
            clean_base, clean_data
        );
        let _ = fs::write(&my_ini_path, my_ini_content);

        // 初始化 MySQL
        let mysql_daemon = dest_dir.join("bin").join("mysqld.exe");
        let _ = std::process::Command::new(mysql_daemon)
            .args(&["--defaults-file", &my_ini_path.to_string_lossy(), "--initialize-insecure"])
            .output();
    }

    // 6. 首次安装时自动创建 junction
    let junction_path = Path::new(&config.links_dir).join(&id);
    if !junction_path.exists() {
        let _ = crate::commands::cache::create_junction(&junction_path, &dest_dir);
    }

    // 7. 自动配置环境变量（指向 links 目录下的稳定路径）
    let link_str = junction_path.to_string_lossy().to_string();
    let dest_str = dest_dir.to_string_lossy().to_string();
    let _ = crate::commands::env::configure_sdk_env_vars(&id, &link_str, &dest_str);

    Ok(())
}

/// 卸载指定版本
#[tauri::command]
pub fn project_uninstall_version(id: String, version: String) -> Result<(), String> {
    let config = load_config();
    let dest_dir = Path::new(&config.versions_dir).join(&id).join(&version);
    if !dest_dir.exists() {
        return Err(format!("版本 {} 的 {} 未安装", version, id));
    }

    // 如果当前正在使用该版本，先断开 junction
    let junction_path = Path::new(&config.links_dir).join(&id);
    let active_dir = fs::canonicalize(&junction_path)
        .map(|p| p.to_string_lossy().to_string().trim_start_matches(r"\\?\").to_string().to_lowercase())
        .unwrap_or_default();
    let dest_dir_clean = dest_dir.to_string_lossy().to_string().to_lowercase();

    if active_dir == dest_dir_clean {
        let _ = fs::remove_file(&junction_path);
    }

    fs::remove_dir_all(&dest_dir).map_err(|e| e.to_string())?;

    // 如果这是该项目最后一个版本，自动清理环境变量
    let sdk_dir = Path::new(&config.versions_dir).join(&id);
    let has_other_versions = fs::read_dir(&sdk_dir)
        .ok()
        .map(|entries| entries.filter_map(|e| e.ok()).any(|e| e.path() != dest_dir))
        .unwrap_or(false);

    if !has_other_versions {
        let _ = crate::commands::env::remove_sdk_env_vars(&id);
    }

    Ok(())
}

/// 切换到指定版本（创建 junction 指向目标版本目录）
#[tauri::command]
pub fn project_use_version(id: String, version: String) -> Result<(), String> {
    let config = load_config();
    let dest_dir = Path::new(&config.versions_dir).join(&id).join(&version);
    if !dest_dir.exists() {
        return Err(format!("版本 {} 的 {} 未安装", version, id));
    }

    let junction_path = Path::new(&config.links_dir).join(&id);
    crate::commands::cache::create_junction(&junction_path, &dest_dir)?;

    // 切换版本后，重新确认环境变量指向正确
    let link_str = junction_path.to_string_lossy().to_string();
    let dest_str = dest_dir.to_string_lossy().to_string();
    let _ = crate::commands::env::configure_sdk_env_vars(&id, &link_str, &dest_str);

    Ok(())
}

/// 注册本地版本（复制到 versions_dir -> 创建 junction）
///
/// 当用户指定一个本地路径时，自动扫描该目录下的可执行文件，
/// 判断它是什么版本并自动识别版本号。
#[tauri::command]
pub fn project_register_local(id: String, version: String, local_path: String) -> Result<(), String> {
    let config = load_config();
    let src = Path::new(&local_path);
    if !src.exists() {
        return Err("本地路径不存在".to_string());
    }

    // 自动识别版本号：如果用户没有指定版本，则尝试从可执行文件获取
    let effective_version = if version.trim().is_empty() {
        detect_version_from_path(&id, src)
            .unwrap_or_else(|| "unknown".to_string())
    } else {
        version.trim().to_string()
    };

    if effective_version == "unknown" {
        return Err("无法自动识别版本号，请手动指定版本号".to_string());
    }

    let dest_dir = Path::new(&config.versions_dir).join(&id).join(&effective_version);
    if dest_dir.exists() {
        return Err(format!("版本 {} 已存在，无需重复添加", effective_version));
    }

    crate::commands::cache::copy_dir_all(src, &dest_dir).map_err(|e| e.to_string())?;

    // 首次安装时自动创建 junction
    let junction_path = Path::new(&config.links_dir).join(&id);
    if !junction_path.exists() {
        let _ = crate::commands::cache::create_junction(&junction_path, &dest_dir);
    }

    let link_str = junction_path.to_string_lossy().to_string();
    let dest_str = dest_dir.to_string_lossy().to_string();
    let _ = crate::commands::env::configure_sdk_env_vars(&id, &link_str, &dest_str);

    Ok(())
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  版本自动识别（问题 4）
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 从本地路径自动检测版本号。
///
/// 根据项目 ID 执行对应的版本检测命令（如 `go version`、`node --version` 等），
/// 解析输出并提取版本号。
fn detect_version_from_path(project_id: &str, path: &Path) -> Option<String> {
    // 根据项目 ID 确定可执行文件名和参数
    let (exe_name, args): (&str, &[&str]) = match project_id {
        "go"        => ("go", &["version"]),
        "nodejs"    => ("node", &["--version"]),
        "python"    => ("python", &["--version"]),
        "bun"       => ("bun", &["--version"]),
        "rust"      => ("rustc", &["--version"]),
        "java"      => ("java", &["-version"]),
        "flutter"   => ("flutter", &["--version"]),
        "maven"     => ("mvn", &["--version"]),
        "gradle"    => ("gradle", &["--version"]),
        "nginx"     => ("nginx", &["-v"]),
        "redis"     => ("redis-server", &["-v"]),
        "mysql"     => ("mysql", &["--version"]),
        "mongodb"   => ("mongod", &["--version"]),
        "postgresql" => ("psql", &["--version"]),
        "yarn"      => ("yarn", &["--version"]),
        "pnpm"      => ("pnpm", &["--version"]),
        "android"   => return None, // Android 无法简单检测
        "harmony"   => return None, // 鸿蒙无法简单检测
        "cuda"      => return None, // CUDA 无法简单检测
        "ffmpeg"    => return None, // FFmpeg 无法简单检测
        _           => return None,
    };

    // 在 bin 子目录或根目录中查找可执行文件
    let exe_candidates = if project_id == "python" {
        vec![path.join("python.exe"), path.join("Scripts").join("python.exe")]
    } else if project_id == "rust" {
        vec![path.join(".cargo").join("bin").join(format!("{}.exe", exe_name))]
    } else if project_id == "android" {
        vec![]
    } else {
        vec![
            path.join(format!("{}.exe", exe_name)),
            path.join("bin").join(format!("{}.exe", exe_name)),
        ]
    };

    let mut exe_path = None;
    for candidate in &exe_candidates {
        if candidate.exists() {
            exe_path = Some(candidate.clone());
            break;
        }
    }

    let exe_path = exe_path?;

    // 执行版本检测命令
    let output = std::process::Command::new(&exe_path)
        .args(args)
        .output()
        .ok()?;

    // 合并 stdout 和 stderr（某些工具如 java 输出到 stderr）
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let combined = if stdout.trim().is_empty() {
        stderr
    } else {
        stdout
    };

    // 解析版本号
    parse_version_from_output(project_id, &combined)
}

/// 从命令输出中解析版本号
fn parse_version_from_output(project_id: &str, output: &str) -> Option<String> {
    let trimmed = output.trim();
    match project_id {
        "go" => {
            // "go version go1.22.0 windows/amd64" -> "1.22.0"
            trimmed.split_whitespace()
                .find(|w| w.starts_with("go"))
                .map(|w| w.trim_start_matches("go").to_string())
        }
        "nodejs" => {
            // "v18.16.0" -> "18.16.0"
            Some(trimmed.trim_start_matches('v').to_string())
        }
        "python" => {
            // "Python 3.12.1" -> "3.12.1"
            trimmed.split_whitespace()
                .nth(1)
                .map(|v| v.to_string())
        }
        "bun" => {
            // "1.1.0" -> "1.1.0"
            Some(trimmed.to_string())
        }
        "rust" => {
            // "rustc 1.76.0 (07dca489a 2024-02-04)" -> "1.76.0"
            trimmed.split_whitespace()
                .nth(1)
                .map(|v| v.to_string())
        }
        "java" => {
            // 'openjdk version "21.0.2" 2024-01-16' -> "21.0.2"
            trimmed.split('"')
                .nth(1)
                .map(|v| v.to_string())
        }
        "flutter" => {
            // "Flutter 3.19.0 ..." -> "3.19.0"
            trimmed.split_whitespace()
                .nth(1)
                .map(|v| v.to_string())
        }
        "maven" => {
            // "Apache Maven 3.9.6 ..." -> "3.9.6"
            trimmed.split_whitespace()
                .find(|w| w.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false))
                .map(|v| v.to_string())
        }
        "gradle" => {
            // "Gradle 8.6" -> "8.6"
            trimmed.split_whitespace()
                .find(|w| w.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false))
                .map(|v| v.to_string())
        }
        "nginx" => {
            // "nginx version: nginx/1.26.1" -> "1.26.1"
            trimmed.split('/')
                .last()
                .map(|v| v.to_string())
        }
        "redis" => {
            // "Redis server v=5.0.14.1 ..." -> "5.0.14.1"
            trimmed.split("v=")
                .nth(1)
                .and_then(|s| s.split_whitespace().next())
                .map(|v| v.to_string())
        }
        "mysql" => {
            // "mysql  Ver 8.0.36 ..." -> "8.0.36"
            trimmed.split_whitespace()
                .find(|w| w.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false))
                .map(|v| v.to_string())
        }
        "mongodb" => {
            // "db version v7.0.5" -> "7.0.5"
            trimmed.split_whitespace()
                .last()
                .map(|v| v.trim_start_matches('v').to_string())
        }
        "postgresql" => {
            // "psql (PostgreSQL) 16.2" -> "16.2"
            trimmed.split_whitespace()
                .find(|w| w.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false))
                .map(|v| v.to_string())
        }
        "yarn" => {
            // "1.22.19" -> "1.22.19"
            Some(trimmed.to_string())
        }
        "pnpm" => {
            // "9.0.5" -> "9.0.5"
            Some(trimmed.to_string())
        }
        _ => None,
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  内部工具函数（从 sdk.rs 迁移）
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 创建临时目录，返回 (路径, 清理闭包)
fn setup_temp_dir(prefix: &str) -> Result<(PathBuf, Box<dyn FnOnce() + Send>), String> {
    let base_dir = get_base_dir();
    let temp_root = base_dir.join(".tmp");
    fs::create_dir_all(&temp_root).map_err(|e| e.to_string())?;

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let temp_dir = temp_root.join(format!("{}_{}", prefix, timestamp));
    fs::create_dir_all(&temp_dir).map_err(|e| e.to_string())?;

    let cleanup_path = temp_dir.clone();
    let cleanup = move || {
        let _ = fs::remove_dir_all(cleanup_path);
    };

    Ok((temp_dir, Box::new(cleanup)))
}

/// 解压 zip 文件
fn unzip_file(src: &Path, dest: &Path) -> Result<(), String> {
    let file = fs::File::open(src).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        let outpath = match file.enclosed_name() {
            Some(path) => dest.join(path.to_owned()),
            None => continue,
        };

        if file.name().ends_with('/') {
            fs::create_dir_all(&outpath).map_err(|e| e.to_string())?;
        } else {
            if let Some(p) = outpath.parent() {
                if !p.exists() {
                    fs::create_dir_all(&p).map_err(|e| e.to_string())?;
                }
            }
            let mut outfile = fs::File::create(&outpath).map_err(|e| e.to_string())?;
            std::io::copy(&mut file, &mut outfile).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// 解压 tar.gz 文件
fn extract_tar_gz(src: &Path, dest: &Path) -> Result<(), String> {
    let file = fs::File::open(src).map_err(|e| e.to_string())?;
    let tar_gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(tar_gz);
    archive.unpack(dest).map_err(|e| e.to_string())?;
    Ok(())
}

/// 将解压后的内容移动到目标目录
fn move_extract_to_dest(extracted_dir: &Path, dest_dir: &Path) -> Result<(), String> {
    let entries = fs::read_dir(extracted_dir).map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .collect::<Vec<_>>();

    let mut src_dir = extracted_dir.to_path_buf();
    if entries.len() == 1 && entries[0].file_type().map(|t| t.is_dir()).unwrap_or(false) {
        src_dir = entries[0].path();
    }

    if dest_dir.exists() {
        fs::remove_dir_all(dest_dir).map_err(|e| e.to_string())?;
    }
    fs::create_dir_all(dest_dir).map_err(|e| e.to_string())?;

    let sub_entries = fs::read_dir(&src_dir).map_err(|e| e.to_string())?
        .filter_map(|e| e.ok());

    for entry in sub_entries {
        let old_path = entry.path();
        let new_path = dest_dir.join(entry.file_name());

        if fs::rename(&old_path, &new_path).is_err() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                crate::commands::cache::copy_dir_all(&old_path, &new_path).map_err(|e| e.to_string())?;
            } else {
                fs::copy(&old_path, &new_path).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

/// 带进度回调的下载
async fn download_with_progress<F>(url: &str, dest: &Path, on_progress: F) -> Result<(), String>
where
    F: Fn(u64, u64),
{
    use futures_util::StreamExt;
    let client = reqwest::Client::builder()
        .user_agent("Any-Version-Manager")
        .build()
        .map_err(|e| e.to_string())?;

    let res = client.get(url).send().await.map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("HTTP 请求失败，状态码: {}", res.status()));
    }

    let total = res.content_length().unwrap_or(0);
    let mut file = fs::File::create(dest).map_err(|e| e.to_string())?;
    let mut stream = res.bytes_stream();
    let mut downloaded = 0u64;

    while let Some(item) = stream.next().await {
        let chunk = item.map_err(|e| e.to_string())?;
        std::io::Write::write_all(&mut file, &chunk).map_err(|e| e.to_string())?;
        downloaded += chunk.len() as u64;
        on_progress(downloaded, total);
    }

    Ok(())
}

/// 获取下载 URL 和文件扩展名
fn get_download_url(project_id: &str, version: &str) -> Result<(String, String), String> {
    let version_clean = version.trim_start_matches('v').split(' ').next().unwrap_or(version).to_string();
    let download_url: String;
    let mut file_ext = "zip".to_string();

    match project_id {
        "go" => {
            download_url = format!("https://go.dev/dl/go{}.windows-amd64.zip", version_clean);
        }
        "nodejs" => {
            download_url = format!("https://nodejs.org/dist/v{}/node-v{}-win-x64.zip", version_clean, version_clean);
        }
        "python" => {
            download_url = format!("https://www.nuget.org/api/v2/package/python/{}", version_clean);
            file_ext = "nupkg".to_string();
        }
        "bun" => {
            download_url = format!("https://github.com/oven-sh/bun/releases/download/bun-v{}/bun-windows-x64.zip", version_clean);
        }
        "android" => {
            download_url = format!("https://dl.google.com/android/repository/commandlinetools-win-{}_latest.zip", version_clean);
        }
        "harmony" => {
            return Err("鸿蒙(HarmonyOS)命令行工具需在华为开发者官网登录后下载，暂无免登录直链。\n请前往官网下载：https://developer.huawei.com/consumer/cn/download/ （选择 Command Line Tools）。\n下载解压后，使用本页下方的『注册本地版本』功能，填入版本号和解压目录即可纳入版本管理。".to_string());
        }
        "rust" => {
            download_url = format!("https://static.rust-lang.org/dist/rust-{}-x86_64-pc-windows-msvc.tar.gz", version_clean);
            file_ext = "tar.gz".to_string();
        }
        "java" => {
            let mut resolved_ver = version.to_string();
            if !version.starts_with("adoptium-") && !version.starts_with("microsoft-") && !version.starts_with("oracle-") && !version.starts_with("zulu-") {
                resolved_ver = format!("adoptium-{}", version);
            }

            if resolved_ver.starts_with("adoptium-") {
                let v = resolved_ver.trim_start_matches("adoptium-");
                download_url = format!("https://api.adoptium.net/v3/binary/version/jdk-{}/windows/x64/jdk/hotspot/normal/eclipse?project=jdk", v);
            } else if resolved_ver.starts_with("microsoft-") {
                let v = resolved_ver.trim_start_matches("microsoft-");
                download_url = format!("https://aka.ms/download-jdk/microsoft-jdk-{}-windows-x64.zip", v);
            } else if resolved_ver.starts_with("oracle-") {
                let v = resolved_ver.trim_start_matches("oracle-");
                download_url = format!("https://download.oracle.com/java/{}/latest/jdk-{}_windows-x64_bin.zip", v, v);
            } else {
                let v = resolved_ver.trim_start_matches("zulu-");
                download_url = format!("https://api.adoptium.net/v3/binary/latest/{}/ga/windows/x64/jdk/hotspot/normal/eclipse?project=jdk", v);
            }
        }
        "flutter" => {
            download_url = format!("https://storage.googleapis.com/flutter_infra_release/releases/stable/windows/flutter_windows_{}-stable.zip", version_clean);
        }
        "nginx" => {
            download_url = format!("https://nginx.org/download/nginx-{}.zip", version_clean);
        }
        "redis" => {
            if version_clean == "3.0.504" {
                download_url = "https://github.com/microsoftarchive/redis/releases/download/win-3.0.504/Redis-x64-3.0.504.zip".to_string();
            } else {
                download_url = format!("https://github.com/tporadowski/redis/releases/download/v{}/Redis-x64-{}.zip", version_clean, version_clean);
            }
        }
        "mysql" => {
            if version_clean.starts_with("5.7") {
                download_url = format!("https://cdn.mysql.com/Downloads/MySQL-5.7/mysql-{}-winx64.zip", version_clean);
            } else if version_clean.starts_with("8.0") {
                download_url = format!("https://cdn.mysql.com/Downloads/MySQL-8.0/mysql-{}-winx64.zip", version_clean);
            } else if version_clean.starts_with("8.4") {
                download_url = format!("https://cdn.mysql.com/Downloads/MySQL-8.4/mysql-{}-winx64.zip", version_clean);
            } else {
                download_url = format!("https://cdn.mysql.com/Downloads/MySQL-8.0/mysql-{}-winx64.zip", version_clean);
            }
        }
        "mongodb" => {
            download_url = format!("https://fastdl.mongodb.org/windows/mongodb-windows-x86_64-{}.zip", version_clean);
        }
        "postgresql" => {
            download_url = format!("https://get.enterprisedb.com/postgresql/postgresql-{}-1-windows-x64-binaries.zip", version_clean);
        }
        "maven" => {
            download_url = format!("https://archive.apache.org/dist/maven/maven-3/{}/binaries/apache-maven-{}-bin.zip", version_clean, version_clean);
        }
        "gradle" => {
            download_url = format!("https://services.gradle.org/distributions/gradle-{}-bin.zip", version_clean);
        }
        "yarn" => {
            download_url = format!("https://github.com/yarnpkg/yarn/releases/download/v{}/yarn-v{}.tar.gz", version_clean, version_clean);
            file_ext = "tar.gz".to_string();
        }
        "pnpm" => {
            download_url = format!("https://github.com/pnpm/pnpm/releases/download/v{}/pnpm-win-x64.exe", version_clean);
            file_ext = "exe".to_string();
        }
        "cuda" => {
            return Err("CUDA Toolkit 需从 NVIDIA 官网下载（需登录）：https://developer.nvidia.com/cuda-toolkit-archive 。下载后使用「注册本地版本」功能导入。".to_string());
        }
        "ffmpeg" => {
            download_url = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip".to_string();
        }
        _ => return Err(format!("不支持自动下载的项目: {}", project_id)),
    }

    Ok((download_url, file_ext))
}
