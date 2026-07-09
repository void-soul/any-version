use serde::Serialize;
use serde_json::Value as JsonValue;
use std::path::PathBuf;
use std::sync::OnceLock;
use crate::commands::ai_registry::{registry, AiToolDefDto, ToolConfig, PathConfig};
use crate::commands::tool_version::is_newer;
use crate::commands::hidden_cmd;
use crate::commands::utils::{find_in_path, get_http_client};

use super::config::DetectedAiTool;

/// 全局缓存的 semver 提取正则表达式（避免每次调用都重新编译）
static SEMVER_RE: OnceLock<regex::Regex> = OnceLock::new();

fn detect_single_tool(config: &ToolConfig, paths: &PathConfig) -> DetectedAiTool {
    eprintln!("[detect] ========== {} ({}) ==========", config.display_name, config.id);

    let upgrade_cmd = match config.pkg_manager.as_deref() {
        Some("npm") => format!("npm install -g {}@latest", config.pkg_name.as_deref().unwrap_or(&config.id)),
        Some("pip") => format!("pip install --upgrade {}", config.pkg_name.as_deref().unwrap_or(&config.id)),
        _ => paths.install_cmd.clone(),
    };

    let not_found = AiToolDefDto {
        id: config.id.clone(),
        display_name: config.display_name.clone(),
        installed: false,
        version: None,
        latest_version_cmd: None,
        install_cmd: paths.install_cmd.clone(),
        upgrade_cmd,
        website: config.website.clone(),
        api_protocol: config.api_protocol.clone(),
        supports_model: config.support_model,
        supports_fallback_model: config.support_fallback_model,
        resume_cmd: config.resume_cmd.clone(),
        continue_cmd: config.continue_cmd.clone(),
        cache_dirs: config.cache_dirs.clone(),
        category: config.category.clone(),
        support_one_m_context: config.support_one_m_context,
        supports_openai: config.supports_openai,
        supports_anthropic: config.supports_anthropic,
        supports_google: config.supports_google,
        builtin_models: config.builtin_models.clone(),
        supports_optimizer: config.supports_optimizer,
        supports_rectifier: config.supports_rectifier,
    };

    // 策略 1：按 PM 类型精准查询版本
    if let Some(pm) = config.pkg_manager.as_deref() {
        if let Some(pkg) = config.pkg_name.as_deref() {
            eprintln!("[detect]   [策略 1] PM={}, pkg={}", pm, pkg);
            if let Some(ver) = detect_via_pm(pm, pkg) {
                eprintln!("[detect]   [策略 1] ✓ 成功 → version={}", ver);
                return DetectedAiTool {
                    installed: true,
                    version: Some(ver),
                    ..not_found
                };
            } else {
                eprintln!("[detect]   [策略 1] ✗ PM 查询失败");
            }
        }
    }

    // 策略 2：回退到 detect_cmd（调用工具自身的 --version）
    let detect_cmd = &paths.detect_cmd;
    eprintln!("[detect]   [策略 2] detect_cmd=\"{}\"", detect_cmd);
    if let Some(ver) = detect_via_cmd(detect_cmd) {
        eprintln!("[detect]   [策略 2] ✓ 成功 → version={}", ver);
        return DetectedAiTool {
            installed: true,
            version: Some(ver),
            ..not_found
        };
    }
    eprintln!("[detect]   [策略 2] ✗ 失败");

    eprintln!("[detect] ✗ 未检测到安装");
    not_found
}

/// 通过包管理器查询已安装版本（npm / pip）
fn detect_via_pm(pm: &str, pkg_name: &str) -> Option<String> {
    match pm {
        "npm" => {
            // npm ls {pkg} -g --json
            let npm_args = &["ls", pkg_name, "-g", "--depth=0", "--json"];
            eprintln!("[detect]     npm cmd: npm {}", npm_args.join(" "));
            let (stdout, stderr) = run_cmd_output_full("cmd", &["/c", "npm", "ls", pkg_name, "-g", "--depth=0", "--json"]);
            let stdout = stdout?;
            eprintln!("[detect]     npm stdout: {}", safe_slice(&stdout, 500));
            if !stderr.is_empty() {
                eprintln!("[detect]     npm stderr: {}", safe_slice(&stderr, 500));
            }
            // 解析 JSON：{"dependencies": {"@scope/pkg": {"version": "1.2.3"}}}
            match serde_json::from_str::<JsonValue>(&stdout) {
                Ok(val) => {
                    eprintln!("[detect]     npm JSON parsed OK");
                    // 兼容两种格式：dependencies 为对象 或 直接为空
                    if let Some(deps) = val.get("dependencies") {
                        // 先精确匹配
                        if let Some(pkg_info) = deps.get(pkg_name) {
                            if let Some(ver) = pkg_info.get("version").and_then(|v| v.as_str()) {
                                eprintln!("[detect]     npm found {}@{} (exact)", pkg_name, ver);
                                return Some(ver.to_string());
                            }
                        }
                        // 再尝试 scoped package 的可能命名变体
                        eprintln!("[detect]     npm deps keys: {:?}", deps.as_object().map(|o| o.keys().collect::<Vec<_>>()));
                        for (k, v) in deps.as_object()? {
                            let vinfo = v.get("version").and_then(|v| v.as_str());
                            eprintln!("[detect]     npm dep: {}={}", k, vinfo.unwrap_or("?"));
                        }
                    } else {
                        eprintln!("[detect]     npm JSON has no 'dependencies' key, keys: {:?}", val.as_object().map(|o| o.keys().collect::<Vec<_>>()));
                    }
                }
                Err(e) => {
                    eprintln!("[detect]     npm JSON parse error: {}", e);
                }
            }
            None
        }
        "pip" => {
            let pip_args = &["show", pkg_name];
            eprintln!("[detect]     pip cmd: pip {}", pip_args.join(" "));
            let (stdout, stderr) = run_cmd_output_full("cmd", &["/c", "pip", "show", pkg_name]);
            let stdout = stdout?;
            eprintln!("[detect]     pip stdout: {}", safe_slice(&stdout, 300));
            if !stderr.is_empty() {
                eprintln!("[detect]     pip stderr: {}", safe_slice(&stderr, 300));
            }
            // 输出包含 "Version: 1.2.3" 行
            for line in stdout.lines() {
                if let Some(ver) = line.strip_prefix("Version:").or_else(|| line.strip_prefix("version:")) {
                    let v = ver.trim().to_string();
                    eprintln!("[detect]     pip found version: {}", v);
                    return Some(v);
                }
            }
            eprintln!("[detect]     pip 'Version:' line not found");
            None
        }
        _ => None,
    }
}

/// 通过 detect_cmd 回退检测（执行工具自身的 --version 命令）
fn detect_via_cmd(detect_cmd: &str) -> Option<String> {
    let parts: Vec<&str> = detect_cmd.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }

    // 先在 PATH 中找可执行文件
    let exe = find_in_path_local(parts[0])?;
    eprintln!("[detect]     find_in_path({}) → {:?}", parts[0], exe);

    let output = {
        let mut cmd = hidden_cmd::hidden_cmd(&exe);
        if parts.len() > 1 {
            cmd.args(&parts[1..]);
        }
        match cmd.output() {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                eprintln!("[detect]     cmd exit_code={}", out.status.code().map_or(-1, |c| c));
                if !stdout.is_empty() {
                    eprintln!("[detect]     cmd stdout: {}", trimmed_first_line(&stdout, 500));
                }
                if !stderr.is_empty() {
                    eprintln!("[detect]     cmd stderr: {}", trimmed_first_line(&stderr, 500));
                }
                if out.status.success() {
                    Some(format!("{}{}", stdout, stderr))
                } else {
                    None
                }
            }
            Err(e) => {
                eprintln!("[detect]     cmd spawn error: {}", e);
                None
            }
        }
    }?;

    let stdout = output.trim().to_string();
    if stdout.is_empty() {
        eprintln!("[detect]     empty output → (installed)");
        return Some("(installed)".to_string());
    }

    // 用正则提取纯净的 semver 版本号
    let ver = extract_semver(&stdout);
    eprintln!("[detect]     extract_semver({}) → {:?}", trimmed_first_line(&stdout, 100), ver);
    ver
}

/// 在 PATH 中查找可执行文件的绝对路径
fn find_in_path_local(exe_name: &str) -> Option<PathBuf> {
    find_in_path(exe_name)
}

/// 从字符串中提取 semver 版本号（如 1.2.3, 0.45.0-alpha）
fn extract_semver(text: &str) -> Option<String> {
    let re = SEMVER_RE.get_or_init(|| {
        regex::Regex::new(r"(\d+\.\d+\.\d+(?:-[a-zA-Z0-9.]+)?)").unwrap_or_else(|_| regex::Regex::new(r"\d+\.\d+\.\d+").unwrap())
    });
    re.captures(text)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// 执行命令并返回 (stdout, stderr)，返回 None 时表示失败
fn run_cmd_output_full(exe: &str, args: &[&str]) -> (Option<String>, String) {
    match hidden_cmd::hidden_cmd(exe).args(args).output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            eprintln!("[detect]     run_cmd: {} {}, exit={}",
                exe, args.join(" "),
                output.status.code().map_or(-1, |c| c));
            if output.status.success() {
                (if stdout.is_empty() { None } else { Some(stdout) }, stderr)
            } else {
                (None, stderr)
            }
        }
        Err(e) => {
            eprintln!("[detect]     run_cmd FAILED: {} {} → {}", exe, args.join(" "), e);
            (None, String::new())
        }
    }
}

/// 安全截取字符串，避免在 UTF-8 字符边界中间切片导致 Panic
fn safe_slice(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        return s;
    }
    let mut end = max_len;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn trimmed_first_line(s: &str, max_len: usize) -> &str {
    let line = s.lines().next().unwrap_or(s);
    safe_slice(line, max_len)
}

#[tauri::command]
pub async fn detect_ai_tools() -> Result<Vec<DetectedAiTool>, String> {
    let tools_reg = registry();
    let tool_ids: Vec<String> = tools_reg.tool_ids().into_iter().map(|s| s.clone()).collect();
    let mut handles = Vec::with_capacity(tool_ids.len());
    for id in &tool_ids {
        let id = id.clone();
        let handle = tokio::task::spawn_blocking(move || {
            let reg = registry();
            let (config, paths) = match reg.get_tool(&id) {
                Some(t) => (t.0.clone(), t.1.clone()),
                None => return None,
            };
            Some(detect_single_tool(&config, &paths))
        });
        handles.push(handle);
    }

    let mut results = Vec::with_capacity(handles.len());
    for handle in handles {
        match handle.await {
            Ok(Some(result)) => results.push(result),
            Ok(None) => {},
            Err(e) => eprintln!("[detect_ai_tools] task join error: {}", e),
        }
    }
    Ok(results)
}

/// AI 工具版本状态
#[derive(Serialize, Clone, Debug)]
pub struct AiToolVersionStatus {
    pub tool_id: String,
    pub display_name: String,
    pub current_version: Option<String>,
    pub latest_version: Option<String>,
    pub status: String,
}

/// 检查所有 AI 工具的最新版本（npm/pip 在线查询）
#[tauri::command]
pub async fn check_ai_tool_versions() -> Result<Vec<AiToolVersionStatus>, String> {
    let tools_reg = registry();
    let tool_ids: Vec<String> = tools_reg.tool_ids().into_iter().map(|s| s.clone()).collect();

    let mut handles = Vec::with_capacity(tool_ids.len());
    for id in &tool_ids {
        let id = id.clone();
        let handle = tokio::task::spawn_blocking(move || {
            let reg = registry();
            let (config, paths) = match reg.get_tool(&id) {
                Some(t) => (t.0.clone(), t.1.clone()),
                None => return None,
            };
            let result = detect_single_tool(&config, &paths);
            Some((id, config.display_name.clone(), result))
        });
        handles.push(handle);
    }

    let mut tools: Vec<(String, String, DetectedAiTool)> = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(Some((id, name, result))) => tools.push((id, name, result)),
            Ok(None) => {},
            Err(e) => eprintln!("[ai_ver] task join error: {}", e),
        }
    }

    // 只对已安装且有 pkg_manager 的工具查最新版本——并发请求
    let mut version_tasks = Vec::new();
    for (id, name, tool) in &tools {
        let tools_reg = registry();
        let pkg_manager = tools_reg.get_tool_config(id).and_then(|c| c.pkg_manager.clone());
        let pkg_name = tools_reg.get_tool_config(id).and_then(|c| c.pkg_name.clone());

        if tool.installed && pkg_manager.is_some() {
            let id = id.clone();
            let name = name.clone();
            let pm = pkg_manager.unwrap();
            let pn = pkg_name.unwrap_or_default();
            version_tasks.push(tokio::spawn(async move {
                let latest = match pm.as_str() {
                    "npm" => fetch_npm_latest_version(&pn).await,
                    "pip" => fetch_pypi_latest_version(&pn).await,
                    _ => None,
                };
                (id, name, latest)
            }));
        }
    }

    let mut latest_map: std::collections::HashMap<String, Option<String>> = std::collections::HashMap::new();
    for task in version_tasks {
        if let Ok((id, _name, latest)) = task.await {
            latest_map.insert(id, latest);
        }
    }

    let mut results = Vec::new();
    for (id, name, tool) in &tools {
        let latest = latest_map.get(id).cloned().flatten();

        let status = match (&tool.version, &latest) {
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

        eprintln!("[ai_ver] {}: installed={}, current={:?}, latest={:?}, status={}",
            id, tool.installed, tool.version, latest, status);

        results.push(AiToolVersionStatus {
            tool_id: id.to_string(),
            display_name: name.to_string(),
            current_version: tool.version.clone(),
            latest_version: latest,
            status,
        });
    }

    Ok(results)
}

/// 查询 npm registry 获取最新版本号
async fn fetch_npm_latest_version(package: &str) -> Option<String> {
    let client = get_http_client();
    let url = format!("https://registry.npmjs.org/{}/latest", package);
    eprintln!("[ai_ver] npm fetch: {}", url);

    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[ai_ver] npm request failed: {}", e);
            return None;
        }
    };

    let body = match resp.text().await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[ai_ver] npm read body failed: {}", e);
            return None;
        }
    };

    if let Ok(info) = serde_json::from_str::<JsonValue>(&body) {
        if let Some(ver) = info.get("version").and_then(|v| v.as_str()) {
            eprintln!("[ai_ver] npm latest for {}: {}", package, ver);
            return Some(ver.to_string());
        }
    }
    eprintln!("[ai_ver] npm response for {} had no 'version' field", package);
    None
}

/// 查询 PyPI 获取最新版本号
async fn fetch_pypi_latest_version(package: &str) -> Option<String> {
    let client = get_http_client();
    let url = format!("https://pypi.org/pypi/{}/json", package);
    eprintln!("[ai_ver] pypi fetch: {}", url);

    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[ai_ver] pypi request failed: {}", e);
            return None;
        }
    };

    let body = match resp.text().await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[ai_ver] pypi read body failed: {}", e);
            return None;
        }
    };

    if let Ok(info) = serde_json::from_str::<JsonValue>(&body) {
        if let Some(ver) = info
            .get("info")
            .and_then(|i| i.get("version"))
            .and_then(|v| v.as_str())
        {
            eprintln!("[ai_ver] pypi latest for {}: {}", package, ver);
            return Some(ver.to_string());
        }
    }
    eprintln!("[ai_ver] pypi response for {} had no 'info.version' field", package);
    None
}

// ─── skills / usage 文件路径 ───

