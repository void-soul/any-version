//! 工具路径覆盖文件管理（参考 EchoBird 的 tool-paths.json 自愈机制）
//!
//! 用户可在 `~/.any-version/tool-paths.json` 中添加/修改工具的安装路径，
//! 无需编辑打包的 `paths.json`。删除此文件即可恢复纯默认路径。
//!
//! 自愈机制：打开文件时，若文件中缺少某个后续版本新增工具的条目，
//! 自动补全其默认路径，且保留用户已有的编辑。
//!
//! 文件格式：{ "<toolId>": ["path/one", "path/two"], ... }
//!   - 字符串自动转为单元素数组
//!   - `_` 前缀的 key 被忽略（用户可用 `_note` 做注释）

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// 加载用户自定义路径覆盖
///
/// 返回 `HashMap<tool_id, Vec<path>>`，文件不存在或格式错误时静默返回空 Map。
pub fn load_user_path_overrides() -> HashMap<String, Vec<String>> {
    let Some(home) = get_home() else {
        return HashMap::new();
    };
    let file = home.join(".any-version").join("tool-paths.json");

    let content = match fs::read_to_string(&file) {
        Ok(c) => c,
        Err(_) => return HashMap::new(),
    };

    let parsed: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[tool_paths] tool-paths.json 不是有效 JSON，忽略覆盖: {}", e);
            return HashMap::new();
        }
    };
    let Some(obj) = parsed.as_object() else {
        return HashMap::new();
    };

    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for (tool_id, val) in obj {
        if tool_id.starts_with('_') {
            continue;
        }
        let mut paths: Vec<String> = Vec::new();
        match val {
            serde_json::Value::String(s) => {
                if !s.trim().is_empty() {
                    paths.push(s.clone());
                }
            }
            serde_json::Value::Array(arr) => {
                for item in arr {
                    if let Some(s) = item.as_str() {
                        if !s.trim().is_empty() {
                            paths.push(s.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
        if !paths.is_empty() {
            out.insert(tool_id.clone(), paths);
        }
    }

    if !out.is_empty() {
        eprintln!(
            "[tool_paths] 已加载 {} 个工具的用户路径覆盖",
            out.len()
        );
    }
    out
}

/// 将用户自定义路径合并到 PathConfig 的当前 OS 路径列表头部
///
/// 用户路径排在最前面（因为用户明确指定了路径），去重后追加默认路径。
pub fn apply_user_path_overrides(
    default_paths: &mut Vec<String>,
    extra: &[String],
) {
    if extra.is_empty() {
        return;
    }

    let mut merged: Vec<String> = Vec::with_capacity(default_paths.len() + extra.len());
    for p in extra {
        if !merged.contains(p) {
            merged.push(p.clone());
        }
    }
    for p in default_paths.iter() {
        if !merged.contains(p) {
            merged.push(p.clone());
        }
    }
    *default_paths = merged;
}

/// 获取当前 OS 对应的路径列表
pub fn get_current_os_paths(paths: &HashMap<String, Vec<String>>) -> Vec<String> {
    #[cfg(target_os = "windows")]
    let key = "win32";
    #[cfg(target_os = "macos")]
    let key = "darwin";
    #[cfg(target_os = "linux")]
    let key = "linux";
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    let key = "win32";

    paths.get(key).cloned().unwrap_or_default()
}

/// 构建覆盖文件的种子数据（纯默认路径，不含用户编辑）
///
/// 从 ai-tools/ 注册表读取所有工具的 path.json，提取当前 OS 的默认路径。
/// 排序以保证稳定输出。
pub fn default_override_seed() -> Vec<(String, Vec<String>)> {
    let registry = crate::commands::ai_registry::registry();
    let mut tool_ids: Vec<String> = registry.tool_ids().into_iter().cloned().collect();
    tool_ids.sort();

    let mut out = Vec::new();
    for tool_id in &tool_ids {
        let Some(path_config) = registry.get_path_config(tool_id) else {
            continue;
        };
        let paths = get_current_os_paths(&path_config.paths);
        if !paths.is_empty() {
            out.push((tool_id.clone(), paths));
        }
    }
    out
}

/// 自愈合并：将种子中新增的工具条目合并到现有文件内容中
///
/// - `Some(empty object)` → 创建完整种子（文件不存在）
/// - `Some(object)` → 只添加缺失的工具条目（自愈已有文件）
/// - `None` → 文件格式无法解析，不修改（避免破坏用户的编辑）
///
/// 返回 `None` 表示文件已是最新，无需写入。
pub fn merge_override_seed(
    existing: Option<&serde_json::Value>,
    seed: &[(String, Vec<String>)],
) -> Option<serde_json::Map<String, serde_json::Value>> {
    let mut map = match existing? {
        serde_json::Value::Object(m) => m.clone(),
        _ => return None,
    };
    let mut changed = false;
    for (tool_id, paths) in seed {
        if !map.contains_key(tool_id) {
            map.insert(
                tool_id.clone(),
                serde_json::Value::Array(
                    paths
                        .iter()
                        .cloned()
                        .map(serde_json::Value::String)
                        .collect(),
                ),
            );
            changed = true;
        }
    }
    if changed { Some(map) } else { None }
}

/// 获取或创建工具路径覆盖文件（含自愈）
///
/// 返回文件路径和是否为新建/自愈（前端可据此提示用户）。
/// 若文件不存在，创建完整的种子文件。
/// 若工具注册表中新增了工具，自动补全缺失条目。
pub fn ensure_tool_path_override_file() -> Result<(String, bool), String> {
    let home = get_home().ok_or("无法获取用户 HOME 目录")?;
    let dir = home.join(".any-version");
    fs::create_dir_all(&dir).map_err(|e| format!("创建 .any-version 目录失败: {}", e))?;

    let file = dir.join("tool-paths.json");
    let seed = default_override_seed();

    let existing: Option<serde_json::Value> = if file.exists() {
        match fs::read_to_string(&file) {
            Ok(content) => serde_json::from_str(&content).ok(),
            Err(_) => None,
        }
    } else {
        // 文件不存在 → 用空对象触发完整种子
        Some(serde_json::Value::Object(serde_json::Map::new()))
    };

    if let Some(map) = merge_override_seed(existing.as_ref(), &seed) {
        let content = serde_json::to_string_pretty(&serde_json::Value::Object(map))
            .map_err(|e| format!("序列化覆盖文件失败: {}", e))?;
        fs::write(&file, format!("{}\n", content))
            .map_err(|e| format!("写入覆盖文件失败: {}", e))?;
        Ok((file.to_string_lossy().to_string(), true))
    } else {
        Ok((file.to_string_lossy().to_string(), false))
    }
}

fn get_home() -> Option<PathBuf> {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()?;
    if home.is_empty() {
        None
    } else {
        Some(PathBuf::from(home))
    }
}

// ─── Tauri 命令 ───

/// 获取/创建工具路径覆盖文件路径（含自愈）
///
/// 返回 { path: string, autoHealed: bool } — autoHealed 表示是否新增了工具条目。
#[tauri::command]
pub fn get_tool_path_override_file() -> Result<serde_json::Value, String> {
    let (path, auto_healed) = ensure_tool_path_override_file()?;
    Ok(serde_json::json!({
        "path": path,
        "autoHealed": auto_healed,
    }))
}
