//! Tool version detection & upgrade module.
//!
//! Detects installed tool/runtime versions by executing `version_cmd`,
//! fetches latest versions from npm / GitHub / PyPI / custom URLs,
//! and provides upgrade capability.
//!
//! Ported from cc-switch's `get_tool_versions` + `run_tool_lifecycle_action` patterns.

use serde::{Serialize, Deserialize};
use std::path::PathBuf;
use tauri_plugin_opener::OpenerExt;
use crate::commands::config::load_config;
use crate::commands::project::registry;
use crate::commands::project::types::ProjectDef;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  数据结构
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Runtime version status for one tool.
#[derive(Serialize, Clone, Debug)]
pub struct ToolVersionStatus {
    pub project_id: String,
    pub display_name: String,
    pub current_version: Option<String>,
    pub latest_version: Option<String>,
    /// "latest" | "outdated" | "unknown" (no remote data) | "not_installed"
    pub status: String,
    pub error: Option<String>,
    pub official_website: String,
    /// If true, the tool supports remote version check
    pub has_remote_check: bool,
}

/// Result of a tool upgrade operation.
#[derive(Serialize, Clone, Debug)]
pub struct ToolUpgradeResult {
    pub project_id: String,
    pub success: bool,
    pub message: String,
}

#[derive(Deserialize)]
struct NpmPackageInfo {
    version: Option<String>,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
}

#[derive(Deserialize)]
struct PypiInfo {
    info: PypiInfoInner,
}
#[derive(Deserialize)]
struct PypiInfoInner {
    version: String,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  Version parsing
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Extract version string from command output using configured regex or default.
fn extract_version(project_id: &str, output: &str) -> Option<String> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return None;
    }
    let custom_regex = registry::find_by_id(project_id)
        .and_then(|def| def.version_parse_regex.clone());
    let pattern = custom_regex.as_deref().unwrap_or(r"\b\d+\.\d+(?:\.\d+)*(?:\-[a-zA-Z0-9.]+)?\b");
    let re = regex::Regex::new(pattern).ok()?;
    if let Some(captures) = re.captures(trimmed) {
        if captures.len() > 1 {
            captures.get(1).map(|m| m.as_str().to_string())
        } else {
            captures.get(0).map(|m| m.as_str().to_string())
        }
    } else {
        None
    }
}

/// Parse version into comparable numeric parts.
fn parse_version_parts(v: &str) -> Vec<u64> {
    let mut parts = Vec::new();
    let mut current = 0u64;
    let mut has_digit = false;
    for c in v.chars() {
        if c.is_ascii_digit() {
            current = current * 10 + (c as u64 - '0' as u64);
            has_digit = true;
        } else if has_digit {
            parts.push(current);
            current = 0;
            has_digit = false;
        }
    }
    if has_digit {
        parts.push(current);
    }
    parts
}

/// Compare two semver-like version strings. Returns Ordering.
fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let pa = parse_version_parts(a);
    let pb = parse_version_parts(b);
    let len = pa.len().max(pb.len());
    for i in 0..len {
        let va = pa.get(i).copied().unwrap_or(0);
        let vb = pb.get(i).copied().unwrap_or(0);
        match va.cmp(&vb) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

/// Returns true if `candidate` is a newer version than `current`.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    compare_versions(candidate, current) == std::cmp::Ordering::Greater
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  Current version detection
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Get the current installed version of a tool by executing its `version_cmd`.
fn detect_current_version(def: &ProjectDef) -> Option<String> {
    let exe_name = def.version_exe.as_ref()?;
    let version_cmd = def.version_cmd.as_ref()?;

    // Resolve the executable from the links directory first, then PATH
    let config = load_config();
    let link_dir = PathBuf::from(&config.links_dir).join(&def.id);

    let parts: Vec<&str> = version_cmd.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }

    // Build candidate exe paths
    let mut candidates = Vec::new();
    candidates.push(link_dir.join(exe_name));
    #[cfg(windows)]
    {
        let exe_lower = exe_name.to_lowercase();
        if !exe_lower.ends_with(".exe") && !exe_lower.ends_with(".cmd") && !exe_lower.ends_with(".bat") {
            candidates.push(link_dir.join(format!("{}.exe", exe_name)));
            candidates.push(link_dir.join(format!("{}.cmd", exe_name)));
        }
    }

    for candidate in &candidates {
        if !candidate.exists() {
            continue;
        }
        let mut cmd = crate::commands::hidden_cmd::hidden_cmd(candidate);
        // Clear env vars that might interfere
        for var_def in &def.env_vars {
            if let Err(_) = std::env::var(&var_def.name) {
                cmd.env_remove(&var_def.name);
            }
        }
        if parts.len() > 1 {
            cmd.args(&parts[1..]);
        }
        if let Ok(output) = cmd.output() {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let combined = if stdout.trim().is_empty() { stderr } else { stdout };
            if let Some(ver) = extract_version(&def.id, &combined) {
                return Some(ver);
            }
        }
    }

    None
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  Remote latest version fetching
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn build_http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("Any-Version-Manager")
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())
}

/// Fetch latest version from the configured source.
async fn fetch_latest_version(def: &ProjectDef) -> Result<Option<String>, String> {
    let config = def.remote_versions_config.as_ref();
    let latest_config = config.and_then(|c| c.get("latest_version"));
    
    // If no latest_version sub-config, try to use the remote versions list
    let source = latest_config
        .and_then(|c| c.get("source").and_then(|v| v.as_str()))
        .unwrap_or("from_list");

    match source {
        "npm" => {
            let package = latest_config
                .and_then(|c| c.get("package").and_then(|v| v.as_str()))
                .ok_or("npm source missing 'package' field")?;
            fetch_npm_latest(package).await
        }
        "github_release" => {
            let repo = latest_config
                .and_then(|c| c.get("repo").and_then(|v| v.as_str()))
                .ok_or("github_release source missing 'repo' field")?;
            fetch_github_latest(repo).await
        }
        "pypi" => {
            let package = latest_config
                .and_then(|c| c.get("package").and_then(|v| v.as_str()))
                .ok_or("pypi source missing 'package' field")?;
            fetch_pypi_latest(package).await
        }
        "url" => {
            let url = latest_config
                .and_then(|c| c.get("url").and_then(|v| v.as_str()))
                .ok_or("url source missing 'url' field")?;
            let field = latest_config
                .and_then(|c| c.get("version_field").and_then(|v| v.as_str()))
                .unwrap_or("version");
            fetch_url_latest(url, field).await
        }
        "from_list" | _ => {
            fetch_latest_from_version_list(def).await
        }
    }
}

/// Fetch latest version from npm registry.
async fn fetch_npm_latest(package: &str) -> Result<Option<String>, String> {
    let client = build_http_client()?;
    let url = format!("https://registry.npmjs.org/{}/latest", package);
    let resp = client.get(&url).send().await.map_err(|e| format!("npm 请求失败: {}", e))?;
    let info: NpmPackageInfo = resp.json().await.map_err(|e| format!("npm 解析失败: {}", e))?;
    Ok(info.version)
}

/// Fetch latest version from GitHub releases.
async fn fetch_github_latest(repo: &str) -> Result<Option<String>, String> {
    let client = build_http_client()?;
    let url = format!("https://api.github.com/repos/{}/releases/latest", repo);
    let resp = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|e| format!("GitHub 请求失败: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("GitHub API 返回 {}", resp.status()));
    }
    let release: GithubRelease = resp.json().await.map_err(|e| format!("GitHub 解析失败: {}", e))?;
    // Strip "v" prefix if present
    let version = release.tag_name.trim_start_matches('v').to_string();
    Ok(Some(version))
}

/// Fetch latest version from PyPI.
async fn fetch_pypi_latest(package: &str) -> Result<Option<String>, String> {
    let client = build_http_client()?;
    let url = format!("https://pypi.org/pypi/{}/json", package);
    let resp = client.get(&url).send().await.map_err(|e| format!("PyPI 请求失败: {}", e))?;
    let info: PypiInfo = resp.json().await.map_err(|e| format!("PyPI 解析失败: {}", e))?;
    Ok(Some(info.info.version))
}

/// Fetch latest version from a custom URL endpoint.
async fn fetch_url_latest(url: &str, field: &str) -> Result<Option<String>, String> {
    let client = build_http_client()?;
    let resp = client.get(url).send().await.map_err(|e| format!("请求失败: {}", e))?;
    let body: serde_json::Value = resp.json().await.map_err(|e| format!("JSON 解析失败: {}", e))?;

    // Support dot notation nesting like "info.version"
    let mut current = &body;
    for part in field.split('.') {
        current = current.get(part).ok_or_else(|| format!("缺少字段: {}", part))?;
    }
    if let Some(v) = current.as_str() {
        Ok(Some(v.to_string()))
    } else {
        Ok(Some(current.to_string()))
    }
}

/// Fallback: get latest version from the existing `remote_versions_config` list.
async fn fetch_latest_from_version_list(def: &ProjectDef) -> Result<Option<String>, String> {
    // Reuse existing remote version list logic
    let result = crate::commands::project::versions::project_list_remote_versions(
        def.id.clone(),
        true, // force refresh to get latest
    )
    .await
    .map_err(|e| format!("获取版本列表失败: {}", e))?;

    // Filter out non-version labels (like "LTS" suffixes, vendor prefixes)
    let versions: Vec<String> = result
        .versions
        .into_iter()
        .filter_map(|v| {
            // Try to extract just the version number
            let re = regex::Regex::new(r"(\d+\.\d+\.\d+)").ok()?;
            re.captures(&v)
                .and_then(|c| c.get(1))
                .map(|m| m.as_str().to_string())
        })
        .collect();

    // Sort and take the latest
    let mut sorted = versions;
    sorted.sort_by(|a, b| {
        let pa = parse_version_parts(a);
        let pb = parse_version_parts(b);
        for i in 0..pa.len().max(pb.len()) {
            let va = pa.get(i).copied().unwrap_or(0);
            let vb = pb.get(i).copied().unwrap_or(0);
            match va.cmp(&vb) {
                std::cmp::Ordering::Equal => continue,
                other => return other,
            }
        }
        std::cmp::Ordering::Equal
    });
    Ok(sorted.pop())
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  Tauri commands
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Check version status for all managed tools that have `version_cmd` configured.
#[tauri::command]
pub async fn check_all_tool_versions() -> Result<Vec<ToolVersionStatus>, String> {
    let defs = registry::registry();
    let mut results = Vec::with_capacity(defs.len());

    for def in &defs {
        // Only check tools that have version detection configured
        if def.version_cmd.is_none() || def.version_exe.is_none() {
            continue;
        }

        let current = detect_current_version(def);
        let (latest, error, has_remote) = if def.remote_versions_config.is_some() {
            match fetch_latest_version(def).await {
                Ok(Some(ver)) => (Some(ver), None, true),
                Ok(None) => (None, None, false),
                Err(e) => (None, Some(e), true),
            }
        } else {
            (None, None, false)
        };

        let status = match (&current, &latest) {
            (None, _) => "not_installed".to_string(),
            (Some(_), None) => "unknown".to_string(),
            (Some(cur), Some(ver)) => {
                if is_newer(ver, cur) {
                    "outdated".to_string()
                } else {
                    "latest".to_string()
                }
            }
        };

        results.push(ToolVersionStatus {
            project_id: def.id.clone(),
            display_name: def.display_name.clone(),
            current_version: current,
            latest_version: latest,
            status,
            error,
            official_website: def.official_website.clone(),
            has_remote_check: has_remote,
        });
    }

    Ok(results)
}

/// Check version status for a single tool.
#[tauri::command]
pub async fn check_tool_version(project_id: String) -> Result<ToolVersionStatus, String> {
    let def = registry::find_by_id(&project_id)
        .ok_or_else(|| format!("未找到项目: {}", project_id))?;

    if def.version_cmd.is_none() || def.version_exe.is_none() {
        return Err(format!("{} 未配置版本检测", def.display_name));
    }

    let current = detect_current_version(&def);
    let (latest, error, has_remote) = if def.remote_versions_config.is_some() {
        match fetch_latest_version(&def).await {
            Ok(Some(ver)) => (Some(ver), None, true),
            Ok(None) => (None, None, false),
            Err(e) => (None, Some(e), true),
        }
    } else {
        (None, None, false)
    };

    let status = match (&current, &latest) {
        (None, _) => "not_installed".to_string(),
        (Some(_), None) => "unknown".to_string(),
        (Some(cur), Some(ver)) => {
            if is_newer(ver, cur) {
                "outdated".to_string()
            } else {
                "latest".to_string()
            }
        }
    };

    Ok(ToolVersionStatus {
        project_id: def.id.clone(),
        display_name: def.display_name.clone(),
        current_version: current,
        latest_version: latest,
        status,
        error,
        official_website: def.official_website.clone(),
        has_remote_check: has_remote,
    })
}

/// Upgrade a tool/runtime.
/// Strategy: 1) use configured upgrade command, 2) open official download page.
#[tauri::command]
pub async fn upgrade_tool(project_id: String, app: tauri::AppHandle) -> Result<ToolUpgradeResult, String> {
    let def = registry::find_by_id(&project_id)
        .ok_or_else(|| format!("未找到项目: {}", project_id))?;

    // Strategy 1: Check if there's an upgrade_cmd configured for the tool's built-in PM
    if let Some(builtin_pm) = def.package_managers.iter().find(|pm| pm.built_in) {
        if let Some(upgrade_cmd) = builtin_pm.pkg_upgrade_cmd_template.as_ref() {
            let final_cmd = upgrade_cmd.replace("{pkg}", &def.id);
            match crate::commands::pkg::execute_command_string(&final_cmd) {
                Ok(_) => {
                    return Ok(ToolUpgradeResult {
                        project_id: def.id.clone(),
                        success: true,
                        message: format!("{} 升级命令已执行", def.display_name),
                    });
                }
                Err(e) => {
                    eprintln!("PM upgrade failed for {}: {}", def.id, e);
                }
            }
        }
    }

    // Strategy 2: Check if tool has download_url_template - try to install latest version
    if def.download_url_template.is_some() && def.remote_versions_config.is_some() {
        if let Ok(Some(version)) = fetch_latest_version(&def).await {
            if let Some(current_ver) = detect_current_version(&def) {
                if is_newer(&version, &current_ver) {
                    // Install the latest version using existing install command
                    match crate::commands::project::versions::project_install_version(
                        app.clone(),
                        def.id.clone(),
                        version.clone(),
                    )
                    .await
                    {
                        Ok(()) => {
                            return Ok(ToolUpgradeResult {
                                project_id: def.id.clone(),
                                success: true,
                                message: format!("{} 正在安装最新版本 v{}", def.display_name, version),
                            });
                        }
                        Err(e) => {
                            eprintln!("Install latest for {} failed: {}", def.id, e);
                        }
                    }
                }
            }
        }
    }

    // Strategy 3: Open official website for manual download
    let _ = app.opener().open_url(
        &def.official_website,
        None::<&str>,
    );

    Ok(ToolUpgradeResult {
        project_id: def.id.clone(),
        success: true,
        message: format!("已打开 {} 下载页面，请手动下载安装最新版本", def.display_name),
    })
}
