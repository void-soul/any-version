use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tauri::AppHandle;
use tauri::Emitter;
use crate::commands::ai_registry::{registry, AiToolDefDto, ToolConfig, PathConfig};
use crate::commands::config::get_base_dir;
use crate::commands::tool_version::is_newer;
use crate::commands::hidden_cmd;
use crate::commands::cache::{get_dir_size, format_bytes, create_junction, migrate_pkg_storage_impl, clean_pkg_cache_impl};
use super::models::*;

use std::collections::HashSet;
use tokio::task;
use super::config::{load_sessions, save_sessions_to_file};

// ─── AI 会话管理 ───

#[tauri::command]
pub fn get_ai_sessions() -> Result<AiSessionsFile, String> {
    Ok(load_sessions())
}

#[tauri::command]
pub fn remove_ai_session(tool_id: String, project_path: String, session_id: Option<String>) -> Result<(), String> {
    let mut sessions = load_sessions();
    sessions.sessions.retain(|s| {
        !(s.tool_id == tool_id && s.project_path == project_path && s.session_id == session_id)
    });
    save_sessions_to_file(&sessions)
}
// ─── 工具会话扫描 ───

/// 扫描工具会话（由 config.json 的 sessions 字段驱动）
#[tauri::command]
pub fn scan_tool_sessions(tool_id: String) -> Result<Vec<ToolSession>, String> {
    let mut sessions = Vec::new();
    let home = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default());
    let home = if home.as_os_str().is_empty() {
        PathBuf::from(std::env::var("HOME").unwrap_or_default())
    } else {
        home
    };
    let config_home = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".config"));

    // 从 registry 获取工具配置
    let session_def = match registry().get_tool_config(&tool_id).and_then(|c| c.sessions.as_ref()) {
        Some(def) => def,
        None => return Ok(sessions),
    };

    for dir_pattern in &session_def.dirs {
        // 解析路径中的 ~ 和 XDG_CONFIG_HOME
        let dir = if dir_pattern.starts_with("~/.config/") {
            let relative = dir_pattern.strip_prefix("~/.config/").unwrap_or("");
            config_home.join(relative)
        } else if dir_pattern.starts_with("~/") {
            home.join(&dir_pattern[2..])
        } else {
            PathBuf::from(dir_pattern)
        };

        if !dir.exists() {
            continue;
        }

        match session_def.scan_type.as_str() {
            "claude_projects" => {
                let projects_dir = dir.join("projects");
                if projects_dir.exists() {
                    scan_claude_sessions_enhanced(&projects_dir, &mut sessions);
                }
            }
            "jsonl" => {
                if dir.is_file() {
                    scan_codex_sessions(&dir, &mut sessions);
                } else {
                    // 也可能是目录，找 sessions.jsonl
                    let f = dir.join("sessions.jsonl");
                    if f.exists() {
                        scan_codex_sessions(&f, &mut sessions);
                    }
                }
            }
            "opencode_style" => {
                scan_opencode_sessions(&dir, &mut sessions);
            }
            _ => {}
        }
    }

    sessions.sort_by(|a, b| b.last_used.cmp(&a.last_used));

    // 为每个 session 填充 resume_cmd（从工具配置的模板 + session_id 拼接）
    fill_resume_cmds(&mut sessions, &tool_id);

    Ok(sessions)
}

/// 扫描 Codex sessions（JSONL 格式，参考 cc-switch）
fn scan_codex_sessions(file_path: &PathBuf, sessions: &mut Vec<ToolSession>) {
    if let Ok(content) = fs::read_to_string(file_path) {
        for line in content.lines() {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
                let session_id = val["session_id"].as_str().unwrap_or("").to_string();
                let project_path = val["project_path"].as_str().unwrap_or("").to_string();
                let last_used = val["last_used"].as_str().unwrap_or("").to_string();
                let summary = val["summary"].as_str().map(|s| s.to_string());
                if !session_id.is_empty() {
                    sessions.push(ToolSession {
                        session_id,
                        project_path,
                        last_used,
                        summary,
                        resume_cmd: None,
                    });
                }
            }
        }
    }
}

/// 扫描 OpenCode sessions（文件系统遍历，参考 EchoBird）
fn scan_opencode_sessions(opencode_dir: &PathBuf, sessions: &mut Vec<ToolSession>) {
    // OpenCode 可能在 sessions/ 子目录或直接在根目录存储
    let session_dir = opencode_dir.join("sessions");
    let scan_dir = if session_dir.exists() { session_dir } else { opencode_dir.clone() };

    // 尝试 SQLite 数据库
    let db_path = scan_dir.join("sessions.db");
    if db_path.exists() {
        // 简单读取 SQLite（启发式，非严格 SQL）
        if let Ok(data) = fs::read(&db_path) {
            // 读取包含 "CREATE TABLE" 确认是 SQLite
            if data.starts_with(b"SQLite format 3\0") {
                eprintln!("[scan_opencode] 发现 SQLite 数据，跳过（需要 rusqlite）");
            }
        }
    }

    // 遍历文件系统查找 session 文件
    if let Ok(entries) = walk_dir_for_sessions(&scan_dir, 3) {
        for (path, modified, _size) in entries {
            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // 尝试 JSONL 格式
            if path.extension().map_or(false, |e| e == "jsonl") {
                // 用 Claude 增强版解析器处理
                parse_jsonl_session(&content, sessions);
            } else if path.extension().map_or(false, |e| e == "json") {
                // JSON 格式
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                    extract_session_from_value(&val, &path, sessions, &modified);
                }
            } else {
                // 纯文本格式（可能是聊天记录）
                let sid = path.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                if !sid.is_empty() {
                    let summary = content.lines().next().map(|s| s.chars().take(200).collect());
                    sessions.push(ToolSession {
                        session_id: sid,
                        project_path: path.parent().unwrap_or(&path).to_string_lossy().to_string(),
                        last_used: modified.clone(),
                        summary,
                        resume_cmd: None,
                    });
                }
            }
        }
    }
}

/// 递归遍历目录查找 session 文件，限制最大深度
fn walk_dir_for_sessions(dir: &PathBuf, max_depth: u32) -> Result<Vec<(PathBuf, String, u64)>, std::io::Error> {
    let mut results = Vec::new();
    walk_dir_recursive(dir, max_depth, &mut results)?;
    Ok(results)
}

fn walk_dir_recursive(dir: &PathBuf, depth: u32, results: &mut Vec<(PathBuf, String, u64)>) -> Result<(), std::io::Error> {
    if depth == 0 { return Ok(()); }
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let modified = entry.metadata()
            .map(|m| {
                if let Ok(t) = m.modified() {
                    chrono::DateTime::<chrono::Local>::from(t)
                        .format("%Y-%m-%dT%H:%M:%S").to_string()
                } else {
                    String::new()
                }
            })
            .unwrap_or_default();

        if path.is_dir() {
            walk_dir_recursive(&path, depth - 1, results)?;
        } else if path.is_file() {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            // 跳过非 session 文件
            if name.starts_with("agent-") || name == "meta.json" || name == "config.json" {
                continue;
            }
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            if size > 0 && size < 10 * 1024 * 1024 { // 跳过超过 10MB 的文件
                results.push((path, modified, size));
            }
        }
    }
    Ok(())
}

/// 解析 JSONL 格式的 session 文件（兼容 Claude Code 格式）
fn parse_jsonl_session(content: &str, sessions: &mut Vec<ToolSession>) {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() { return; }

    let mut session_id = String::new();
    let mut project_path = String::new();
    let mut last_used = String::new();
    let mut summary: Option<String> = None;
    let mut title: Option<String> = None;

    // 头行提取 session_id / cwd / 标题
    for line in lines.iter().take(15) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
            if session_id.is_empty() {
                session_id = val["session_id"].as_str().unwrap_or("").to_string();
            }
            if project_path.is_empty() {
                project_path = val["cwd"].as_str()
                    .or(val["project_path"].as_str())
                    .unwrap_or("").to_string();
            }
            // 从 message 中提取第一条用户输入作为标题
            if let Some(msg) = val.get("message") {
                if msg.get("role").and_then(|r| r.as_str()) == Some("user") {
                    let text = extract_message_text(msg);
                    if !text.is_empty() && title.is_none() {
                        title = Some(text.chars().take(80).collect());
                    }
                }
            }
        }
    }

    // 尾行提取最后时间戳和摘要
    let tail_start = if lines.len() > 30 { lines.len() - 30 } else { 0 };
    for line in &lines[tail_start..] {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(ts) = val["timestamp"].as_str() {
                if ts > last_used.as_str() { last_used = ts.to_string(); }
            }
            if let Some(msg) = val.get("message") {
                if msg.get("role").and_then(|r| r.as_str()) == Some("assistant") {
                    let text = extract_message_text(msg);
                    if !text.is_empty() {
                        summary = Some(text.chars().take(200).collect());
                    }
                }
            }
        }
    }

    if !session_id.is_empty() {
        sessions.push(ToolSession {
            session_id,
            project_path,
            last_used,
            summary: summary.or(title),
            resume_cmd: None,
        });
    }
}

/// 从 JSON Value 直接提取 session 信息
fn extract_session_from_value(val: &serde_json::Value, _path: &PathBuf, sessions: &mut Vec<ToolSession>, modified: &str) {
    let session_id = val.get("session_id").and_then(|v| v.as_str())
        .or(val.get("id").and_then(|v| v.as_str()))
        .or(val.get("uuid").and_then(|v| v.as_str()))
        .unwrap_or("").to_string();
    if session_id.is_empty() { return; }

    let project_path = val.get("cwd").and_then(|v| v.as_str())
        .or(val.get("project_path").and_then(|v| v.as_str()))
        .or(val.get("projectDir").and_then(|v| v.as_str()))
        .unwrap_or("").to_string();
    let last_used = val.get("last_used").and_then(|v| v.as_str())
        .or(val.get("timestamp").and_then(|v| v.as_str()))
        .or(val.get("updatedAt").and_then(|v| v.as_str()))
        .unwrap_or(modified)
        .to_string();
    let summary = val.get("summary").and_then(|v| v.as_str())
        .or(val.get("title").and_then(|v| v.as_str()))
        .or(val.get("name").and_then(|v| v.as_str()))
        .map(|s| s.chars().take(200).collect());

    sessions.push(ToolSession {
        session_id,
        project_path,
        last_used,
        summary,
        resume_cmd: None,
    });
}

/// 从 message JSON 中提取文本内容
fn extract_message_text(msg: &serde_json::Value) -> String {
    if let Some(content) = msg.get("content") {
        if let Some(arr) = content.as_array() {
            for item in arr {
                if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                    return text.to_string();
                }
            }
        } else if let Some(s) = content.as_str() {
            return s.to_string();
        }
    }
    String::new()
}
// ─── 改进的会话扫描（并行，参考 cc-switch Provider Adapter 模式）───

#[tauri::command]
pub async fn scan_tool_sessions_parallel(tool_ids: Vec<String>) -> Result<std::collections::HashMap<String, Vec<ToolSession>>, String> {
    let mut results = std::collections::HashMap::new();

    let mut handles = Vec::new();
    for tool_id in tool_ids {
        let handle = tokio::task::spawn_blocking(move || {
            match scan_tool_sessions(tool_id.clone()) {
                Ok(sessions) => (tool_id, sessions),
                Err(e) => {
                    eprintln!("[scan_tool_sessions_parallel] {} error: {}", tool_id, e);
                    (tool_id, Vec::new())
                }
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        match handle.await {
            Ok((id, sessions)) => { results.insert(id, sessions); }
            Err(e) => eprintln!("[scan_tool_sessions_parallel] task join error: {}", e),
        }
    }

    Ok(results)
}

/// 增强版 Claude Session 扫描：头 10 行提取 title，尾 30 行提取 summary
/// 增强版 Claude 风格 session 扫描（兼容 Claude Code / Gemini CLI / KiloCode 等）
///
/// 目录结构（Claude Code v2.x 实测）：
///   .claude/projects/E--pro-my-any-version/
///     ├── {uuid1}/           ← UUID 子目录（可能包含派生数据）
///     ├── {uuid2}/           
///     ├── {uuid1}.jsonl      ← 实际 session 数据（与子目录同级的文件！）
///     ├── {uuid2}.jsonl
///     └── memory/            ← 特殊目录，跳过
///
/// JSONL 每行格式（顶层字段，注意驼峰命名）：
///   - "sessionId": "uuid"        ← 注意是驼峰 sessionId，不是 session_id
///   - "cwd": "E:\\pro\\..."       ← 工作目录
///   - "timestamp": "2026-..."     ← 每行都有时间戳
///   - "type": "ai-title", "aiTitle": "标题"   ← 会话标题事件
///   - "message": {"role": "user"|"assistant", "content": [...]}
fn scan_claude_sessions_enhanced(dir: &PathBuf, sessions: &mut Vec<ToolSession>) {
    if !dir.is_dir() { return; }

    let read_dir = match fs::read_dir(dir) {
        Ok(d) => d,
        Err(_) => return,
    };

    // 收集所有 .jsonl 文件（在项目根目录下直接扫描，不只是子目录里）
    let mut jsonl_files: Vec<PathBuf> = Vec::new();
    for entry in read_dir.flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();

        // 跳过 memory 目录、agent- 子代理文件
        if name == "memory" || name.starts_with("agent-") {
            continue;
        }

        // jsonl 文件直接在项目目录下（同级）
        if path.is_file() && path.extension().map_or(false, |e| e == "jsonl") {
            jsonl_files.push(path);
            continue;
        }

        // 同时兼容旧格式：UUID 子目录里的 jsonl 文件
        if path.is_dir() && name.len() > 8 && name.contains('-') {
            if let Ok(sub_entries) = fs::read_dir(&path) {
                for sub in sub_entries.flatten() {
                    let sp = sub.path();
                    let sn = sub.file_name().to_string_lossy().to_string();
                    if sn.starts_with("agent-") { continue; }
                    if sp.is_file() && sp.extension().map_or(false, |e| e == "jsonl") {
                        jsonl_files.push(sp);
                    }
                }
            }
        }
    }

    // 去重（同名文件可能被两种扫描路径都收集到）
    jsonl_files.sort();
    jsonl_files.dedup();

    for file_path in &jsonl_files {
        let content = match fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let lines: Vec<&str> = content.lines().collect();
        if lines.is_empty() { continue; }

        let mut session_id = String::new();
        let mut project_path = String::new();
        let mut last_used = String::new();
        let mut title: Option<String> = None;
        let mut summary: Option<String> = None;

        for line in &lines {
            let val = match serde_json::from_str::<serde_json::Value>(line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // 提取 sessionId（驼峰命名）
            if session_id.is_empty() {
                session_id = val["sessionId"].as_str().unwrap_or("").to_string();
            }

            // 提取 cwd（顶层字段）
            if project_path.is_empty() {
                if let Some(cwd) = val["cwd"].as_str() {
                    project_path = cwd.to_string();
                }
            }

            // 提取 timestamp（取最大的）
            if let Some(ts) = val["timestamp"].as_str() {
                if ts > last_used.as_str() {
                    last_used = ts.to_string();
                }
            }

            // 提取 ai-title 作为标题（优先于 message 内容）
            if val["type"].as_str() == Some("ai-title") {
                if let Some(at) = val["aiTitle"].as_str() {
                    if !at.is_empty() {
                        title = Some(at.to_string());
                    }
                }
            }

            // 从 message 中提取内容（作为 summary 后备）
            if let Some(msg) = val.get("message") {
                let role = msg["role"].as_str().unwrap_or("");
                let text = extract_msg_content(msg);

                if role == "user" && !text.is_empty() && title.is_none() {
                    title = Some(text.chars().take(80).collect());
                }
                if role == "assistant" && !text.is_empty() {
                    summary = Some(text.chars().take(200).collect());
                }
            }
        }

        if !session_id.is_empty() {
            if project_path.is_empty() {
                // 从父目录名推断项目路径（如 E--pro-my-any-version → E:/pro/my/any-version）
                if let Some(parent) = file_path.parent() {
                    if let Some(name) = parent.file_name().and_then(|n| n.to_str()) {
                        project_path = decode_project_path(name);
                    }
                }
            }
            sessions.push(ToolSession {
                session_id,
                project_path,
                last_used,
                summary: summary.or(title),
                resume_cmd: None,
            });
        }
    }
}

/// 从 Claude Code 编码的项目目录名还原实际路径
/// "E--pro-my-any-version" → "E:/pro/my/any-version"
fn decode_project_path(encoded: &str) -> String {
    encoded.replace("--", ":").replace('-', "/")
}

/// 从 message JSON 提取文本内容（支持 string 和 array 两种格式）
fn extract_msg_content(msg: &serde_json::Value) -> String {
    let content = msg.get("content");
    if let Some(s) = content.and_then(|v| v.as_str()) {
        return s.to_string();
    }
    if let Some(arr) = content.and_then(|v| v.as_array()) {
        for item in arr {
            if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                return text.to_string();
            }
        }
    }
    String::new()
}

/// 为扫描到的会话填充 resume_cmd（由工具配置的 resume_cmd 模板 + session_id 拼接）
///
/// 模板中的 `{session_id}` 占位符会被替换为实际 session_id。
/// 例如: `opencode -s {session_id}` → `opencode -s abc-123-def`
fn fill_resume_cmds(sessions: &mut Vec<ToolSession>, tool_id: &str) {
    let resume_template = match registry().get_tool_config(tool_id).and_then(|c| c.resume_cmd.as_deref()) {
        Some(cmd) if !cmd.is_empty() => cmd.to_string(),
        _ => return,
    };

    for session in sessions.iter_mut() {
        if session.resume_cmd.is_some() {
            continue; // 已填充，跳过
        }
        session.resume_cmd = Some(
            resume_template.replace("{session_id}", &session.session_id)
        );
    }
}
