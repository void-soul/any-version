//! 项目版本管理模块 -- 远程版本列表、安装、卸载、切换。
//!
//! 从已删除的 sdk.rs 迁移而来，适配新的项目托管架构。
//! 使用 project_id（原 sdk_name）标识项目，通过 load_config() 获取 versions_dir/links_dir，
//! 通过 junction 实现版本切换。
//!
//! 注意：本地版本注册/导入功能已移除。托管时自动将旧版数据（版本、来源、路径、备份环境变量）
//! 保存到 backup 目录供"旧版数据"选项卡展示。

use std::fs;
use std::path::{Path, PathBuf};
use std::collections::HashSet;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use crate::commands::config::{load_config, get_base_dir};
use crate::commands::sdk_resolver::{FindRule, ResolvePattern as ResolverPattern};
use super::types::ResolvePattern;

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
//  Tauri 命令
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 获取远程版本列表（从 projects.json 的 remote_versions_config 读取配置）
#[tauri::command]
pub async fn project_list_remote_versions(id: String) -> Result<Vec<String>, String> {
    let def = super::registry::find_by_id(&id)
        .ok_or_else(|| format!("未找到项目: {}", id))?;

    let config = def.remote_versions_config.as_ref()
        .ok_or_else(|| format!("未配置远程版本: {}", id))?;

    // connect_timeout: 仅限制建立连接的超时（15s），不限制请求总时长
    // 对 GitHub API 等有时候很慢的接口更友好
    let client = reqwest::Client::builder()
        .user_agent("Any-Version-Manager")
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;

    let config_type = config.get("type").and_then(|v| v.as_str()).unwrap_or("static");

    match config_type {
        "static" => {
            let versions = config.get("versions")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            Ok(versions)
        }
        "json_api" => fetch_json_api(&client, config, def.remote_versions_url.as_deref()).await,
        "multi_source" => fetch_multi_source(&client, config).await,
        _ => Err(format!("不支持的远程版本类型: {}", config_type)),
    }
}

async fn fetch_json_api(client: &reqwest::Client, config: &serde_json::Value, url_override: Option<&str>) -> Result<Vec<String>, String> {
    let url = if let Some(u) = url_override {
        u
    } else if let Some(u) = config.get("url").and_then(|v| v.as_str()) {
        u
    } else {
        return Err("缺少 url 配置".to_string());
    };
    let max_count = config.get("max_count").and_then(|v| v.as_u64()).unwrap_or(100) as usize;
    let response_type = config.get("response_type").and_then(|v| v.as_str()).unwrap_or("array");
    let version_field = config.get("version_field").and_then(|v| v.as_str()).unwrap_or("version");
    let version_transform = config.get("version_transform").and_then(|v| v.as_str()).unwrap_or("");
    let filter_field = config.get("filter_field").and_then(|v| v.as_str());
    let filter_value = config.get("filter_value");
    let filter_contains_not = config.get("filter_contains_not").and_then(|v| v.as_str());
    let reverse = config.get("reverse").and_then(|v| v.as_bool()).unwrap_or(false);
    let extra_field = config.get("extra_field").and_then(|v| v.as_str());

    // 带重试的请求（最多 3 次，指数退避：1s, 2s, 4s）
    let mut last_error = String::new();
    for attempt in 0..3 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(1 << (attempt - 1))).await;
        }
        let resp = match client.get(url).send().await {
            Ok(r) => r,
            Err(e) => {
                last_error = format!("网络请求失败: {}", e);
                continue;
            }
        };
        // 检查 HTTP 状态码
        let status = resp.status();
        if !status.is_success() {
            last_error = format!("HTTP {} ({}), 可能是 API 限流或网络问题", status.as_u16(), status.canonical_reason().unwrap_or("未知"));
            continue;
        }
        // 读取响应文本，便于在解析失败时输出诊断信息
        let resp_text = match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                last_error = format!("读取响应体失败: {}", e);
                continue;
            }
        };
        let body: serde_json::Value = match serde_json::from_str(&resp_text) {
            Ok(v) => v,
            Err(e) => {
                // 截取前 200 字符帮助诊断
                let preview: String = resp_text.chars().take(200).collect();
                last_error = format!("JSON 解析失败: {}，响应预览: {}", e, preview);
                continue;
            }
        };

        let items: Vec<serde_json::Value> = match response_type {
            "array" => body.as_array().cloned().unwrap_or_default(),
            "object_with_array" => {
                let arr_field = config.get("array_field").and_then(|v| v.as_str()).unwrap_or("versions");
                body.get(arr_field).and_then(|v| v.as_array()).cloned().unwrap_or_default()
            }
            "object_with_nested_array" => {
                let arr_field = config.get("array_field").and_then(|v| v.as_str()).unwrap_or("releases");
                body.get(arr_field).and_then(|v| v.as_array()).cloned().unwrap_or_default()
            }
            _ => {
                last_error = format!("不支持的 response_type: {}", response_type);
                continue;
            }
        };

        let mut versions: Vec<String> = Vec::new();
        for item in &items {
            if let Some(ff) = filter_field {
                if let Some(fv) = filter_value {
                    let item_val = item.get(ff);
                    if let Some(fv_bool) = fv.as_bool() {
                        if item_val.and_then(|v| v.as_bool()).unwrap_or(false) != fv_bool { continue; }
                    } else if let Some(fv_str) = fv.as_str() {
                        if item_val.and_then(|v| v.as_str()).unwrap_or("") != fv_str { continue; }
                    }
                }
            }

            let raw_version = if response_type == "object_with_array" {
                // items may be objects (Adoptium API) or plain strings
                item.get(version_field).and_then(|v| v.as_str().map(String::from))
                    .or_else(|| item.as_str().map(String::from))
                    .unwrap_or_default()
            } else {
                item.get(version_field).and_then(|v| {
                    if v.is_string() { v.as_str().map(String::from) }
                    else if v.is_array() {
                        Some(v.as_array().unwrap().iter().map(|n| n.to_string()).collect::<Vec<_>>().join("."))
                    } else { Some(v.to_string()) }
                }).unwrap_or_default()
            };

            if raw_version.is_empty() { continue; }
            if let Some(fc) = filter_contains_not {
                if raw_version.contains(fc) { continue; }
            }

            let mut ver = apply_transform(&raw_version, version_transform);

            if let Some(ef) = extra_field {
                if let Some(extra_val) = item.get(ef) {
                    let extra_format = config.get("extra_format").and_then(|v| v.as_str()).unwrap_or("");
                    if extra_format == "lts_label" {
                        if extra_val.is_boolean() && extra_val.as_bool().unwrap_or(false) {
                            ver = format!("{} (LTS)", ver);
                        } else if extra_val.is_string() {
                            let lts_name = extra_val.as_str().unwrap_or("");
                            if !lts_name.is_empty() && lts_name != "false" {
                                ver = format!("{} (LTS: {})", ver, lts_name);
                            }
                        }
                    }
                }
            }
            // version_label: prepend vendor/distribution label for display
            if let Some(label) = config.get("version_label").and_then(|v| v.as_str()) {
                ver = format!("{} {}", label, ver);
            }
            versions.push(ver);
        }

        if reverse { versions.reverse(); }
        // max_count=0 表示不截取，展示全部版本
        if max_count > 0 {
            versions.truncate(max_count);
        }
        return Ok(versions);
    }

    Err(last_error)
}

async fn fetch_multi_source(client: &reqwest::Client, config: &serde_json::Value) -> Result<Vec<String>, String> {
    let mut all_versions: Vec<String> = Vec::new();

    if let Some(statics) = config.get("static_versions").and_then(|v| v.as_array()) {
        for v in statics {
            if let Some(s) = v.as_str() { all_versions.push(s.to_string()); }
        }
    }

    if let Some(sources) = config.get("sources").and_then(|v| v.as_array()) {
        let futures: Vec<_> = sources.iter().map(|source| {
            let c = client.clone();
            let source = source.clone();
            async move {
                let url = if let Some(url_template) = source.get("url_template").and_then(|v| v.as_str()) {
                    let versions = source.get("versions").and_then(|v| v.as_array());
                    if let Some(vers) = versions {
                        let mut results = Vec::new();
                        for v in vers {
                            if let Some(ver_str) = v.as_str() {
                                let next: i32 = ver_str.parse().unwrap_or(0) + 1;
                                let u = url_template.replace("{major}", ver_str).replace("{next}", &next.to_string());
                                if let Some(r) = fetch_single_source(&c, &source, &u).await {
                                    results.extend(r);
                                }
                            }
                        }
                        return results;
                    }
                    return Vec::new();
                } else if let Some(u) = source.get("url").and_then(|v| v.as_str()) {
                    u.to_string()
                } else {
                    return Vec::new();
                };
                fetch_single_source(&c, &source, &url).await.unwrap_or_default()
            }
        }).collect();

        let results = futures_util::future::join_all(futures).await;
        for mut r in results { all_versions.append(&mut r); }
    }

    Ok(all_versions)
}

async fn fetch_single_source(client: &reqwest::Client, source: &serde_json::Value, url: &str) -> Option<Vec<String>> {
    let resp = client.get(url).send().await.ok()?;
    let body: serde_json::Value = resp.json().await.ok()?;

    let response_type = source.get("response_type").and_then(|v| v.as_str()).unwrap_or("array");
    let max_per = source.get("max_per_source").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
    let version_transform = source.get("version_transform").and_then(|v| v.as_str()).unwrap_or("");

    let items: Vec<serde_json::Value> = match response_type {
        "array" => body.as_array().cloned().unwrap_or_default(),
        "object_with_array" => {
            let arr_field = source.get("array_field").and_then(|v| v.as_str()).unwrap_or("releases");
            body.get(arr_field).and_then(|v| v.as_array()).cloned().unwrap_or_default()
        }
        _ => return None,
    };

    let mut versions = Vec::new();
    for item in items.iter().take(max_per) {
        if let Some(ff) = source.get("filter_field").and_then(|v| v.as_str()) {
            if let Some(fc) = source.get("filter_contains").and_then(|v| v.as_str()) {
                let val = item.get(ff).and_then(|v| v.as_str()).unwrap_or("");
                if !val.contains(fc) { continue; }
            }
        }
        let raw = if response_type == "object_with_array" {
            item.as_str().map(String::from).unwrap_or_default()
        } else {
            let vf = source.get("version_field").and_then(|v| v.as_str()).unwrap_or("version");
            item.get(vf).and_then(|v| {
                if v.is_string() { v.as_str().map(String::from) }
                else if v.is_array() {
                    let fmt = source.get("version_format").and_then(|f| f.as_str()).unwrap_or("");
                    if fmt == "join_dots" {
                        Some(v.as_array().unwrap().iter().map(|n| n.to_string()).collect::<Vec<_>>().join("."))
                    } else { Some(v.to_string()) }
                } else { Some(v.to_string()) }
            }).unwrap_or_default()
        };
        if !raw.is_empty() { versions.push(apply_transform(&raw, version_transform)); }
    }
    Some(versions)
}

fn apply_transform(version: &str, transform: &str) -> String {
    let mut ver = version.to_string();
    for op in transform.split(';') {
        let op = op.trim();
        if let Some(suffix) = op.strip_prefix("trim_suffix:") {
            ver = ver.strip_suffix(suffix).unwrap_or(&ver).to_string();
        } else if let Some(prefix) = op.strip_prefix("trim_prefix:") {
            ver = ver.strip_prefix(prefix).unwrap_or(&ver).to_string();
        } else if let Some(prefix) = op.strip_prefix("prefix:") {
            ver = format!("{}{}", prefix, ver);
        } else if let Some(repl) = op.strip_prefix("replace:") {
            // format: replace:old:new
            let parts: Vec<&str> = repl.split(':').collect();
            if parts.len() >= 2 {
                ver = ver.replace(parts[0], parts[1]);
            }
        }
    }
    ver
}


/// 安装指定版本（下载 -> 解压 -> 安装到 versions_dir -> 创建 junction -> 配置环境变量）
#[tauri::command]
pub async fn project_install_version(app: AppHandle, id: String, version: String) -> Result<(), String> {
    let def = super::registry::find_by_id(&id)
        .ok_or_else(|| format!("未找到项目: {}", id))?;
    let config = load_config();

    let dl_info = get_download_url(&id, &version)?;
    let download_url = dl_info.url;
    let file_ext = dl_info.file_ext;

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
    let ext_result = match file_ext.as_str() {
        "tar.gz" | "tgz" => extract_tar_gz(&archive_path, &extract_dir),
        "tar.xz" => extract_tar_xz(&archive_path, &extract_dir),
        "tar.bz2" => extract_tar_bz2(&archive_path, &extract_dir),
        "msi" => extract_msi(&archive_path, &extract_dir),
        "exe" => {
            fs::create_dir_all(&extract_dir).map_err(|e| e.to_string())?;
            fs::copy(&archive_path, extract_dir.join(format!("{}.exe", id)))
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
        _ => unzip_file(&archive_path, &extract_dir),
    };

    if let Err(e) = ext_result {
        cleanup();
        return Err(format!("解压失败: {}", e));
    }

    // 4. 安装到 versions_dir
    let dest_dir = Path::new(&config.versions_dir).join(&id).join(&version);

    // 使用 scoop 推导的 extract_subdir 或 JSON 配置
    let extract_subdir = dl_info.extract_subdir.as_deref().unwrap_or("");
    let src_dir = if !extract_subdir.is_empty() {
        extract_dir.join(extract_subdir)
    } else {
        extract_dir
    };

    if let Err(e) = move_extract_to_dest(&src_dir, &dest_dir) {
        cleanup();
        return Err(format!("安装失败: {}", e));
    }

    cleanup();

    // 5. 后置配置（从 JSON post_install 读取）
    if let Some(ref post_install) = def.post_install {
        if post_install.get("generate_config").and_then(|v| v.as_bool()).unwrap_or(false) {
            if let Some(tpl) = post_install.get("config_template").and_then(|v| v.as_str()) {
                let ini_path = dest_dir.join("my.ini");
                let data_dir = dest_dir.join("data");
                let content = tpl
                    .replace("{basedir}", &dest_dir.to_string_lossy().replace("\\", "/"))
                    .replace("{datadir}", &data_dir.to_string_lossy().replace("\\", "/"));
                let _ = fs::write(&ini_path, content);
            }
        }
        if let Some(init_cmd) = post_install.get("init_command").and_then(|v| v.as_str()) {
            let parts: Vec<&str> = init_cmd.splitn(2, ' ').collect();
            let exe = dest_dir.join(parts[0]);
            let args: Vec<&str> = if parts.len() > 1 { parts[1].split(' ').collect() } else { vec![] };
            let _ = super::super::hidden_cmd::hidden_cmd(exe).args(&args).current_dir(&dest_dir).output();
        }
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

/// 从本地路径自动检测版本号（公开函数，供 save_manage_backup 等使用）。
pub fn detect_version_from_path(project_id: &str, path: &Path) -> Option<String> {
    let def = super::registry::find_by_id(project_id)?;

    // 通过 version_cmd + version_exe 检测
    if let (Some(ref cmd), Some(ref exe)) = (&def.version_cmd, &def.version_exe) {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        if !parts.is_empty() {
            let exe_path = path.join(format!("{}.exe", exe));
            let bin_exe = path.join("bin").join(format!("{}.exe", exe));
            for candidate in &[exe_path, bin_exe] {
                if candidate.exists() {
                    if let Ok(output) = super::super::hidden_cmd::hidden_cmd(candidate)
                        .args(&parts[1..])
                        .output()
                    {
                        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                        let combined = if stdout.trim().is_empty() { stderr } else { stdout };
                        if let Some(ver) = parse_version_from_output(project_id, &combined) {
                            return Some(ver);
                        }
                    }
                }
            }
        }
    }
    None
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

/// 提取 tar.xz 归档（需要 xz2 依赖）
fn extract_tar_xz(src: &Path, dest: &Path) -> Result<(), String> {
    let file = fs::File::open(src).map_err(|e| e.to_string())?;
    let decoder = xz2::read::XzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    archive.unpack(dest).map_err(|e| e.to_string())?;
    Ok(())
}

/// 提取 tar.bz2 归档（需要 bzip2 依赖）
fn extract_tar_bz2(src: &Path, dest: &Path) -> Result<(), String> {
    let file = fs::File::open(src).map_err(|e| e.to_string())?;
    let decoder = bzip2::read::BzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    archive.unpack(dest).map_err(|e| e.to_string())?;
    Ok(())
}

/// 提取 MSI 安装包（使用 msiexec /a 行政安装）
fn extract_msi(src: &Path, dest: &Path) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let dest_str = dest.to_string_lossy().to_string();
    let src_str = src.to_string_lossy().to_string();

    let output = crate::commands::hidden_cmd::hidden_cmd("msiexec")
        .args(["/a", &src_str, "/qn", &format!("TARGETDIR={}", dest_str)])
        .output()
        .map_err(|e| format!("msiexec 执行失败: {}", e))?;

    if !output.status.success() {
        return Err(format!("MSI 提取失败，退出码: {:?}", output.status.code()));
    }
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
    // 下载大文件（如 Rust 200MB+）时不应有短超时
    // connect_timeout 仅限制建立连接的时间，不限制下载总时长
    let client = reqwest::Client::builder()
        .user_agent("Any-Version-Manager")
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let res = client.get(url).send().await.map_err(|e| format!("下载请求失败: {}", e))?;
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
struct DownloadInfo {
    url: String,
    file_ext: String,
    extract_subdir: Option<String>,
}

fn get_download_url(project_id: &str, version: &str) -> Result<DownloadInfo, String> {
    let def = super::registry::find_by_id(project_id)
        .ok_or_else(|| format!("未找到项目: {}", project_id))?;

    // 如果版本字符串带有 version_label 前缀，先剥离
    let version_stripped = if let Some(ref cfg) = def.remote_versions_config {
        if let Some(label) = cfg.get("version_label").and_then(|v| v.as_str()) {
            version.strip_prefix(&format!("{} ", label)).unwrap_or(version)
        } else {
            version
        }
    } else {
        version
    };
    let version_clean = version_stripped.trim_start_matches('v').split(' ').next().unwrap_or(version_stripped);
    // Clean version for download URL: strip "-LTS" suffix, encode "+" as "%2B"
    let version_url = version_clean.trim_end_matches("-LTS").replace('+', "%2B");
    let file_ext = def.download_file_ext.clone().unwrap_or_else(|| "zip".to_string());

    // 1. 优先按版本前缀映射（如 java: adoptium-/microsoft-/oracle-/zulu-）
    if let Some(ref prefix_map) = def.version_prefix_map {
        let major_version = version_clean.split('.').next().unwrap_or("0");
        for (prefix, template) in prefix_map {
            if version_stripped.starts_with(prefix) {
                let ver = version_stripped.trim_start_matches(prefix).trim_end_matches("-LTS").replace('+', "%2B");
                let url = template
                    .replace("{ver}", &ver)
                    .replace("{version}", &version_url)
                    .replace("{majorVersion}", major_version);
                return Ok(DownloadInfo { url, file_ext, extract_subdir: def.extract_subdir.clone() });
            }
        }
    }

    // 2. 按版本号前缀映射（如 mysql: 5.7/8.0/8.4）
    if let Some(ref url_prefix_map) = def.version_url_prefix_map {
        let major_version = version_clean.split('.').next().unwrap_or("0");
        for (ver_prefix, template) in url_prefix_map {
            if version_clean.starts_with(ver_prefix) {
                let url = template
                    .replace("{version}", &version_url)
                    .replace("{majorVersion}", major_version);
                return Ok(DownloadInfo { url, file_ext, extract_subdir: def.extract_subdir.clone() });
            }
        }
    }

    // 3. 使用通用 download_url_template（由 Scoop 更新或手动定义）
    if let Some(ref template) = def.download_url_template {
        let major_version = version_clean.split('.').next().unwrap_or("0");
        let url = template
            .replace("{version}", &version_url)
            .replace("{majorVersion}", major_version);
        return Ok(DownloadInfo { url, file_ext, extract_subdir: def.extract_subdir.clone() });
    }

    Err(format!("未配置下载地址: {}", project_id))
}


