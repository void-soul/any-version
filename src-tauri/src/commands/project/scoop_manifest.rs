//! Scoop Manifest 集成模块
//!
//! 从 ScoopInstaller 仓库获取 manifest JSON，自动推导安装参数：
//! - 下载地址（含版本号替换）
//! - 文件扩展名
//! - 解压子目录
//! - PATH 目录列表（bin_dirs）
//! - 远程版本检测 URL

use serde::{Deserialize, Serialize};

// ── Scoop Manifest 反序列化结构体 ──

#[derive(Deserialize, Debug, Clone)]
pub struct ScoopManifest {
    pub version: Option<String>,
    pub architecture: Option<ScoopArchitecture>,
    pub bin: Option<serde_json::Value>,
    pub env_add_path: Option<serde_json::Value>,
    pub env_set: Option<serde_json::Value>,
    pub extract_dir: Option<ScoopExtractDir>,
    pub persist: Option<Vec<String>>,
    pub checkver: Option<ScoopCheckver>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ScoopArchitecture {
    #[serde(rename = "64bit")]
    pub x64: Option<ScoopArchEntry>,
    #[serde(rename = "32bit")]
    pub x86: Option<ScoopArchEntry>,
    pub arm64: Option<ScoopArchEntry>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ScoopArchEntry {
    pub url: String,
    pub hash: Option<String>,
    pub extract_dir: Option<String>,
}

/// extract_dir 字段：可以是全局字符串或按架构区分
#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum ScoopExtractDir {
    String(String),
    Architecture { x64: Option<String>, x86: Option<String>, arm64: Option<String> },
}

#[derive(Deserialize, Debug, Clone)]
pub struct ScoopCheckver {
    pub url: Option<String>,
    pub regex: Option<String>,
    pub jsonpath: Option<String>,
}

// ── 推导出的安装信息 ──

#[derive(Debug, Clone)]
pub struct DerivedInstallInfo {
    /// 下载 URL 模板（{version} 为占位符）
    pub download_url_template: String,
    /// 文件扩展名：zip / tar.gz / msi / 7z / exe
    pub file_ext: String,
    /// 解压后需要进入的子目录
    pub extract_subdir: Option<String>,
    /// 需要添加到 PATH 的目录列表（相对于安装根目录）
    pub bin_dirs: Vec<String>,
    /// 文件哈希（暂未使用，可用于完整性校验）
    #[allow(dead_code)]
    pub hash: Option<String>,
    /// 远程版本检测 URL
    pub remote_versions_url: Option<String>,
    /// 远程版本检测配置
    pub remote_versions_config: Option<serde_json::Value>,
}

// ── 核心逻辑 ──

/// 获取 Scoop manifest 的 URL
pub fn manifest_url(bucket: &str, name: &str) -> String {
    format!(
        "https://raw.githubusercontent.com/ScoopInstaller/{}/master/bucket/{}.json",
        bucket, name
    )
}

/// 从 JSON 字符串解析 Scoop manifest
pub fn parse_manifest(json: &str) -> Result<ScoopManifest, String> {
    serde_json::from_str::<ScoopManifest>(json)
        .map_err(|e| format!("Scoop manifest 解析失败: {}", e))
}

/// 异步获取 Scoop manifest（带本地缓存）
pub async fn fetch_manifest(bucket: &str, name: &str) -> Result<ScoopManifest, String> {
    let base_dir = crate::commands::config::get_base_dir();
    let cache_dir = base_dir.join("scoop_cache");
    let cache_file = cache_dir.join(format!("{}_{}.json", bucket, name));

    // 如果缓存存在且不超过 24 小时，直接使用
    if cache_file.exists() {
        if let Ok(meta) = std::fs::metadata(&cache_file) {
            if let Ok(modified) = meta.modified() {
                if let Ok(elapsed) = modified.elapsed() {
                    if elapsed.as_secs() < 86400 {
                        let content = std::fs::read_to_string(&cache_file)
                            .map_err(|e| format!("读取缓存失败: {}", e))?;
                        return parse_manifest(&content);
                    }
                }
            }
        }
    }

    // 下载 manifest
    let url = manifest_url(bucket, name);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

    let resp = client.get(&url)
        .header("User-Agent", "any-version/1.0")
        .send()
        .await
        .map_err(|e| format!("获取 Scoop manifest 失败: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("Scoop manifest 不存在 (HTTP {}): {}", resp.status(), url));
    }

    let text = resp.text().await
        .map_err(|e| format!("读取响应失败: {}", e))?;

    // 缓存到本地
    let _ = std::fs::create_dir_all(&cache_dir);
    let _ = std::fs::write(&cache_file, &text);

    parse_manifest(&text)
}

/// 从 serde_json::Value 数组中提取首个条目字符串（兼容 String / [String, ...] 混合）
fn extract_first_entries(arr: &serde_json::Value) -> Vec<String> {
    let mut result = Vec::new();
    if let Some(items) = arr.as_array() {
        for item in items {
            match item {
                serde_json::Value::String(s) => result.push(s.clone()),
                serde_json::Value::Array(a) => {
                    if let Some(first) = a.first().and_then(|v| v.as_str()) {
                        result.push(first.to_string());
                    }
                }
                _ => {}
            }
        }
    }
    result
}

/// 从 Scoop manifest 推导安装信息（用实际版本号填充 URL）
pub fn derive_install_info(manifest: &ScoopManifest, version: &str) -> Result<DerivedInstallInfo, String> {
    let arch_entry = manifest.architecture.as_ref()
        .and_then(|a| a.x64.as_ref())
        .ok_or_else(|| "Scoop manifest 缺少 64-bit 架构定义".to_string())?;

    let file_ext = derive_file_ext(&arch_entry.url);

    let version_clean = version.trim_start_matches('v');
    let major_version = version_clean.split('.').next().unwrap_or("0");
    let download_url = arch_entry.url
        .replace("$version", version_clean)
        .replace("$majorVersion", major_version)
        .replace("{version}", version_clean);

    let (extract_subdir, bin_dirs, remote_versions_url, remote_versions_config) =
        derive_common_fields(manifest, arch_entry);

    Ok(DerivedInstallInfo {
        download_url_template: download_url,
        file_ext,
        extract_subdir,
        bin_dirs,
        hash: arch_entry.hash.clone(),
        remote_versions_url,
        remote_versions_config,
    })
}

/// 从 Scoop manifest 推导安装模板（保留 {version} 占位符，供写入 projects.json）
pub fn derive_install_template(manifest: &ScoopManifest) -> Result<DerivedInstallInfo, String> {
    let arch_entry = manifest.architecture.as_ref()
        .and_then(|a| a.x64.as_ref())
        .ok_or_else(|| "Scoop manifest 缺少 64-bit 架构定义".to_string())?;

    let file_ext = derive_file_ext(&arch_entry.url);

    // 将 Scoop 占位符转为我们统一的 {version} / {majorVersion}
    let url_template = arch_entry.url
        .replace("$version", "{version}")
        .replace("$majorVersion", "{majorVersion}");

    let (extract_subdir, bin_dirs, remote_versions_url, remote_versions_config) =
        derive_common_fields(manifest, arch_entry);

    Ok(DerivedInstallInfo {
        download_url_template: url_template,
        file_ext,
        extract_subdir,
        bin_dirs,
        hash: arch_entry.hash.clone(),
        remote_versions_url,
        remote_versions_config,
    })
}

/// 从 manifest 提取 extract_subdir / bin_dirs / 版本检测（与版本无关的共用逻辑）
fn derive_common_fields(
    manifest: &ScoopManifest,
    arch_entry: &ScoopArchEntry,
) -> (
    Option<String>,
    Vec<String>,
    Option<String>,
    Option<serde_json::Value>,
) {
    // extract_subdir
    let extract_subdir = arch_entry.extract_dir.clone()
        .or_else(|| match &manifest.extract_dir {
            Some(ScoopExtractDir::String(s)) => Some(s.clone()),
            Some(ScoopExtractDir::Architecture { x64, .. }) => x64.clone(),
            None => None,
        });

    // bin_dirs
    let mut bin_dirs: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    if let Some(ref env_path) = manifest.env_add_path {
        for d in extract_first_entries(env_path) {
            let normalized = d.trim_matches('\\').trim_matches('/').replace('\\', "/");
            if !normalized.is_empty() && seen.insert(normalized.clone()) {
                bin_dirs.push(normalized);
            }
        }
    }

    if let Some(ref bin) = manifest.bin {
        for entry in extract_first_entries(bin) {
            if let Some(pos) = entry.rfind('\\').or_else(|| entry.rfind('/')) {
                let dir = &entry[..pos];
                let normalized = dir.trim_matches('\\').trim_matches('/').replace('\\', "/");
                if !normalized.is_empty() && seen.insert(normalized.clone()) {
                    bin_dirs.push(normalized);
                }
            }
        }
    }

    // 版本检测
    let (remote_versions_url, remote_versions_config) = if let Some(ref cv) = manifest.checkver {
        let url = cv.url.clone().or_else(|| {
            cv.jsonpath.as_ref().map(|_| String::new())
        });

        let mut config = serde_json::Map::new();

        if let Some(ref jsonpath) = cv.jsonpath {
            config.insert("type".to_string(), serde_json::Value::String("json_api".to_string()));
            config.insert("response_type".to_string(), serde_json::Value::String("array".to_string()));
            config.insert("version_field".to_string(), serde_json::Value::String(jsonpath.clone()));
            config.insert("regex".to_string(), serde_json::Value::String(
                cv.regex.clone().unwrap_or_else(|| "v(.*)".to_string())
            ));
        } else {
            config.insert("type".to_string(), serde_json::Value::String("scrape".to_string()));
            if let Some(ref regex) = cv.regex {
                config.insert("regex".to_string(), serde_json::Value::String(regex.clone()));
            }
        }

        (url, Some(serde_json::Value::Object(config)))
    } else {
        (None, None)
    };

    (extract_subdir, bin_dirs, remote_versions_url, remote_versions_config)
}

/// 从 URL 后缀推导文件扩展名
fn derive_file_ext(url: &str) -> String {
    let url_lower = url.to_lowercase();
    let base = url_lower.split('?').next().unwrap_or(&url_lower);
    if base.ends_with(".tar.gz") || base.ends_with(".tgz") {
        "tar.gz".to_string()
    } else if base.ends_with(".tar.xz") {
        "tar.xz".to_string()
    } else if base.ends_with(".tar.bz2") {
        "tar.bz2".to_string()
    } else if base.ends_with(".7z") {
        "7z".to_string()
    } else if base.ends_with(".msi") {
        "msi".to_string()
    } else if base.ends_with(".exe") {
        "exe".to_string()
    } else if base.ends_with(".zip") {
        "zip".to_string()
    } else if base.ends_with(".nupkg") {
        "nupkg".to_string()
    } else {
        "zip".to_string()
    }
}

// ── 从 Scoop 更新 projects.json ──

/// 查找 projects.json，搜索逻辑与 load_registry() 保持一致
fn find_projects_json() -> Option<std::path::PathBuf> {
    let base_dir = crate::commands::config::get_base_dir();
    let mut search_dirs: Vec<std::path::PathBuf> = Vec::new();

    // 1. exe 同目录及向上 5 层
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            search_dirs.push(exe_dir.to_path_buf());
            let mut dir = exe_dir.to_path_buf();
            for _ in 0..5 {
                if let Some(parent) = dir.parent() {
                    dir = parent.to_path_buf();
                    search_dirs.push(dir.clone());
                }
            }
        }
    }

    // 2. 当前工作目录
    if let Ok(cwd) = std::env::current_dir() {
        search_dirs.push(cwd);
    }

    // 3. 用户配置目录
    search_dirs.push(base_dir);

    for dir in &search_dirs {
        let up_dir = dir.join("_up_");
        let candidates = [up_dir.as_path(), dir.as_path()];
        for candidate in &candidates {
            let path = candidate.join("projects.json");
            if path.exists() {
                eprintln!("[scoop_update] 找到 projects.json: {}", path.display());
                return Some(path);
            }
        }
    }

    eprintln!("[scoop_update] 所有路径均未找到 projects.json");
    None
}

/// 从 Scoop manifest 批量更新 projects.json 中的安装参数。
///
/// 遍历所有含 `scoop_ref` 的项目，拉取对应 manifest，
/// 推导 download_url / file_ext / extract_subdir / bin_dirs / 版本检测配置，
/// 写回 projects.json。后续安装/扫描直接读取 projects.json，无需访问 Scoop。
#[tauri::command]
pub async fn update_projects_from_scoop() -> Result<ScoopUpdateReport, String> {
    // 复用与 load_registry() 相同的路径搜索逻辑
    let projects_path = find_projects_json()
        .ok_or_else(|| "projects.json 不存在，无法更新".to_string())?;

    let content = std::fs::read_to_string(&projects_path)
        .map_err(|e| format!("读取 projects.json 失败: {}", e))?;

    let mut projects: Vec<super::types::ProjectDef> = serde_json::from_str(&content)
        .map_err(|e| format!("解析 projects.json 失败: {}", e))?;

    let now = chrono::Utc::now().to_rfc3339();
    let mut report = ScoopUpdateReport::default();

    for project in &mut projects {
        let Some(ref scoop_ref) = project.scoop_ref else { continue };

        report.total += 1;

        // 拉取 Scoop manifest
        let manifest = match fetch_manifest(&scoop_ref.bucket, &scoop_ref.name).await {
            Ok(m) => m,
            Err(e) => {
                report.failed.push(ScoopUpdateEntry {
                    id: project.id.clone(),
                    display_name: project.display_name.clone(),
                    error: e,
                });
                continue;
            }
        };

        // 推导模板信息（URL 中保留 {version} 占位符）
        let info = match derive_install_template(&manifest) {
            Ok(i) => i,
            Err(e) => {
                report.failed.push(ScoopUpdateEntry {
                    id: project.id.clone(),
                    display_name: project.display_name.clone(),
                    error: e,
                });
                continue;
            }
        };

        // 填充推导出的字段到 ProjectDef（保留手动定义的不覆盖）
        let mut updated = false;

        if project.download_url_template.is_none() || info.download_url_template != project.download_url_template.as_deref().unwrap_or("") {
            project.download_url_template = Some(info.download_url_template);
            updated = true;
        }
        if project.download_file_ext.is_none() {
            project.download_file_ext = Some(info.file_ext);
            updated = true;
        }
        if project.extract_subdir.is_none() && info.extract_subdir.is_some() {
            project.extract_subdir = info.extract_subdir;
            updated = true;
        }
        if project.bin_dirs.is_none() && !info.bin_dirs.is_empty() {
            project.bin_dirs = Some(info.bin_dirs);
            updated = true;
        }
        if project.remote_versions_url.is_none() && info.remote_versions_url.is_some() {
            project.remote_versions_url = info.remote_versions_url;
            project.remote_versions_config = info.remote_versions_config;
            updated = true;
        }

        if updated {
            project.scoop_updated_at = Some(now.clone());
            report.updated.push(ScoopUpdateEntry {
                id: project.id.clone(),
                display_name: project.display_name.clone(),
                error: String::new(),
            });
        } else {
            project.scoop_updated_at = Some(now.clone());
            report.skipped.push(ScoopUpdateEntry {
                id: project.id.clone(),
                display_name: project.display_name.clone(),
                error: String::new(),
            });
        }
    }

    // 写回 projects.json
    let new_content = serde_json::to_string_pretty(&projects)
        .map_err(|e| format!("序列化 projects.json 失败: {}", e))?;

    std::fs::write(&projects_path, &new_content)
        .map_err(|e| format!("写入 projects.json 失败: {}", e))?;

    // 清理项目注册表缓存以使新写入的内容生效
    super::registry::clear_registry_cache();

    report.success = true;
    Ok(report)
}

/// 更新报告
#[derive(Serialize, Clone, Debug, Default)]
pub struct ScoopUpdateReport {
    pub success: bool,
    /// 更新的项目数
    pub total: u32,
    /// 已更新的项目（字段有变化）
    pub updated: Vec<ScoopUpdateEntry>,
    /// 跳过的项目（字段未变化）
    pub skipped: Vec<ScoopUpdateEntry>,
    /// 失败的项目
    pub failed: Vec<ScoopUpdateEntry>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ScoopUpdateEntry {
    pub id: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub error: String,
}
