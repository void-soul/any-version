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

// ─── 真实可执行文件定位（检测兜底 / 启动时 PATH 前置） ───

/// 展开工具路径模板：`%VAR%`（Windows 环境变量）与 `~`（用户主目录）。
///
/// 打包的 `paths.json` 用的是 `%APPDATA%/npm/claude.cmd` 这种写法，而
/// `utils::expand_home`（认 `{根名}`）与 `dirs::expand` 都不认 `%VAR%`。
/// 未定义的变量**原样保留**，避免拼出一条看起来合法其实错误的路径。
pub fn expand_tool_path(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('%') else {
            out.push('%');
            rest = after;
            continue;
        };
        let name = &after[..end];
        // 空名字或含分隔符 → 不是环境变量占位符
        if name.is_empty() || name.contains('/') || name.contains('\\') {
            out.push('%');
            rest = after;
            continue;
        }
        match std::env::var(name) {
            Ok(value) => {
                out.push_str(&value);
                rest = &after[end + 1..];
            }
            Err(_) => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);

    let normalized = if cfg!(windows) {
        out.replace('/', "\\")
    } else {
        out
    };

    if normalized == "~" {
        return get_home()
            .map(|home| home.to_string_lossy().to_string())
            .unwrap_or(normalized);
    }
    if let Some(tail) = normalized
        .strip_prefix("~/")
        .or_else(|| normalized.strip_prefix("~\\"))
    {
        if let Some(home) = get_home() {
            return home.join(tail).to_string_lossy().to_string();
        }
    }
    normalized
}

/// 该工具在当前 OS 的候选路径：默认路径 + 用户覆盖（用户路径优先）。
///
/// 用户覆盖来自 `~/.any-version/tool-paths.json`（`tool_paths` 里的自愈文件）。
pub fn effective_tool_paths(tool_id: &str, declared: &HashMap<String, Vec<String>>) -> Vec<String> {
    let mut paths = get_current_os_paths(declared);
    let overrides = load_user_path_overrides();
    let extra = overrides.get(tool_id).cloned().unwrap_or_default();
    apply_user_path_overrides(&mut paths, &extra);
    paths
}

/// 命令名的候选后缀（Windows 上同一个命令可能是 .exe / .cmd / .bat）。
fn command_suffixes() -> &'static [&'static str] {
    if cfg!(windows) {
        &[".exe", ".cmd", ".bat", ".ps1", ""]
    } else {
        &[""]
    }
}

/// 在工具声明的路径列表里找**磁盘上真实存在**的可执行文件。
///
/// 抄作业自 EchoBird `f86fe961`（`command_exists || binary detected on disk`），两个用途：
/// - **检测**：`claude --version` 跑不起来 ≠ 没安装 —— curl/scoop/choco 装完 `setx` 只对
///   **之后**启动的进程生效，本进程 PATH 里没有该目录，会误报「未安装」；
/// - **启动**：把命中的目录前置进子进程 PATH，裸命令（`cmd /k claude`）才能被解析。
///
/// 路径项既可能是文件本身（`%APPDATA%/npm/claude.cmd`），也可能是安装目录。
pub fn find_declared_exe(
    tool_id: &str,
    declared: &HashMap<String, Vec<String>>,
    command: &str,
) -> Option<PathBuf> {
    // 命令名**可以为空**：桌面应用的 paths.json 就是 `startCommand: ""`（它们靠 exe 路径启动）。
    // 以前这里直接 `？` 早退，导致这类工具无论默认路径还是用户手填的路径都不去磁盘上看一眼，
    // 永远显示「未安装」——用户设了路径也没用。
    let command = command.split_whitespace().next().unwrap_or("").trim();
    for raw in effective_tool_paths(tool_id, declared) {
        let expanded = expand_tool_path(&raw);
        if expanded.is_empty() {
            continue;
        }
        let path = PathBuf::from(&expanded);
        if path.is_file() {
            return Some(path);
        }
        if path.is_dir() {
            if !command.is_empty() {
                for suffix in command_suffixes() {
                    let candidate = path.join(format!("{}{}", command, suffix));
                    if candidate.is_file() {
                        return Some(candidate);
                    }
                }
            } else if let Some(guessed) = guess_exe_in_dir(&path, tool_id) {
                // 没有命令名可拼：目录里的 exe 猜一个（见函数注释，猜不到就返回 None）
                return Some(guessed);
            }
        }
    }
    None
}

/// 在没有命令名的情况下，从目录里猜可执行文件。
///
/// 两种情形才认（都要足够确定，宁可返回 None 让上层报「未安装」，也不要指到一个错的文件）：
/// 1. 文件名与工具 id 同名（忽略大小写、连字符与扩展名）：`workbuddy` → `WorkBuddy.exe`；
/// 2. 目录里**只有一个**像可执行文件的东西（Windows 看 .exe/.cmd/.bat/.ps1，其它平台不设限）。
fn guess_exe_in_dir(dir: &std::path::Path, tool_id: &str) -> Option<PathBuf> {
    let files: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    if files.is_empty() {
        return None;
    }
    let normalize = |s: &str| s.to_ascii_lowercase().replace(['-', '_'], "");
    let needle = normalize(tool_id);
    if let Some(hit) = files.iter().find(|p| {
        p.file_stem()
            .and_then(|s| s.to_str())
            .map(|s| normalize(s) == needle)
            .unwrap_or(false)
    }) {
        return Some(hit.clone());
    }

    let exe_like: Vec<&PathBuf> = if cfg!(windows) {
        files
            .iter()
            .filter(|p| {
                matches!(
                    p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
                    Some("exe") | Some("cmd") | Some("bat") | Some("ps1")
                )
            })
            .collect()
    } else {
        files.iter().collect()
    };
    if exe_like.len() == 1 {
        return Some(exe_like[0].clone());
    }
    None
}

// ─── 手动指定路径（用户在界面上填的那一条） ───

/// 注册表里该工具在当前 OS 的默认路径（用于把「用户自己填的」从文件里认出来）。
fn registry_default_paths(tool_id: &str) -> Vec<String> {
    match crate::commands::ai_registry::registry().get_path_config(tool_id) {
        Some(pc) => get_current_os_paths(&pc.paths),
        None => Vec::new(),
    }
}

/// 读取用户在界面上手动指定的路径。
///
/// 覆盖文件里默认路径与用户路径混在一起（`apply_user_path_overrides` 需要这个形态），
/// 所以「哪条是用户填的」只能靠与注册表默认路径做差集得出。
pub fn custom_path_for(tool_id: &str) -> Option<String> {
    let defaults = registry_default_paths(tool_id);
    let saved = load_user_path_overrides().get(tool_id).cloned()?;
    saved.into_iter().find(|p| !defaults.contains(p))
}

/// 写入（或清除）用户手动指定的路径。
///
/// 保留注册表默认路径垫在后面：手动填错时还能靠默认路径认出已安装的工具，
/// 不至于「填一次错的就再也检测不到」。传 `None` / 空串 = 清除，回到纯默认。
pub fn set_custom_path(tool_id: &str, path: Option<&str>) -> Result<(), String> {
    let home = get_home().ok_or("无法获取用户 HOME 目录")?;
    let dir = home.join(".any-version");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建 .any-version 目录失败: {}", e))?;
    let file = dir.join("tool-paths.json");

    // 现有内容优先；文件不存在/坏掉时从注册表种子重建（其余工具的条目不丢）
    let mut map: serde_json::Map<String, serde_json::Value> = match std::fs::read_to_string(&file)
        .ok()
        .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
        .and_then(|v| match v {
            serde_json::Value::Object(m) => Some(m),
            _ => None,
        }) {
        Some(m) => m,
        None => {
            let mut m = serde_json::Map::new();
            for (id, paths) in default_override_seed() {
                m.insert(
                    id,
                    serde_json::Value::Array(
                        paths.into_iter().map(serde_json::Value::String).collect(),
                    ),
                );
            }
            m
        }
    };

    let defaults = registry_default_paths(tool_id);
    let mut merged: Vec<String> = Vec::new();
    if let Some(p) = path.map(str::trim).filter(|p| !p.is_empty()) {
        merged.push(p.to_string());
    }
    for d in &defaults {
        if !merged.contains(d) {
            merged.push(d.clone());
        }
    }

    if merged.is_empty() {
        map.remove(tool_id);
    } else {
        map.insert(
            tool_id.to_string(),
            serde_json::Value::Array(merged.into_iter().map(serde_json::Value::String).collect()),
        );
    }

    let content = serde_json::to_string_pretty(&serde_json::Value::Object(map))
        .map_err(|e| format!("序列化覆盖文件失败: {}", e))?;
    std::fs::write(&file, format!("{}\n", content))
        .map_err(|e| format!("写入覆盖文件失败: {}", e))?;
    Ok(())
}

// ─── Tauri 命令 ───

/// 手动指定某工具的安装路径（可执行文件本身或安装目录皆可）。
///
/// 传空串/null 表示清除，回到注册表默认路径。
#[tauri::command]
pub fn ai_set_tool_custom_path(tool_id: String, path: Option<String>) -> Result<(), String> {
    if tool_id.trim().is_empty() {
        return Err("工具 id 不能为空".to_string());
    }
    set_custom_path(&tool_id, path.as_deref())
}

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

#[cfg(test)]
mod tests {
    use super::{expand_tool_path, find_declared_exe};
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn probe_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!("anyver-toolpaths-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn declared_all_platforms(entries: &[String]) -> HashMap<String, Vec<String>> {
        let mut map = HashMap::new();
        for key in ["win32", "darwin", "linux"] {
            map.insert(key.to_string(), entries.to_vec());
        }
        map
    }

    /// `%VAR%` 能展开，未定义的变量原样保留（不能拼出一条假的绝对路径）。
    #[test]
    fn expand_tool_path_resolves_env_and_keeps_unknown() {
        let unknown = expand_tool_path("%ANYVER_DEFINITELY_NOT_SET_98765%/npm/claude.cmd");
        assert!(
            unknown.starts_with('%'),
            "未定义变量应原样保留: {}",
            unknown
        );
        assert!(unknown.ends_with("claude.cmd"));

        // 用必然存在的变量（PATH）验证展开。注意占位符两端都要有 `%`
        let value = std::env::var("PATH").expect("PATH 必然存在");
        let out = expand_tool_path("%PATH%/probe/x");
        assert!(
            out.contains(&value),
            "占位符未展开: out={:?} value={:?}",
            out,
            value
        );
        if !value.contains('%') {
            assert!(!out.contains('%'), "占位符应被完全展开: {:?}", out);
        }
    }

    /// 路径项是目录 → 拼命令名；是文件本身 → 直接用；都找不到 → None。
    #[test]
    fn find_declared_exe_matches_files_directories_and_gives_up() {
        let dir = probe_dir("find");
        let file_name = if cfg!(windows) { "probe.cmd" } else { "probe" };
        let exe = dir.join(file_name);
        std::fs::write(&exe, "").unwrap();

        let by_dir = declared_all_platforms(&[dir.to_string_lossy().to_string()]);
        assert_eq!(
            find_declared_exe("__probe_test__", &by_dir, "probe").expect("目录项应命中"),
            exe
        );

        let by_file = declared_all_platforms(&[exe.to_string_lossy().to_string()]);
        assert_eq!(
            find_declared_exe("__probe_test__", &by_file, "probe").expect("文件项应命中"),
            exe
        );

        // start_command 可能带参数（如 "mimo ."）→ 只取命令名
        assert!(find_declared_exe("__probe_test__", &by_dir, "probe --verbose").is_some());

        // 路径列表为空（或都不存在）→ None，不要瞎猜
        assert!(find_declared_exe("__probe_test__", &HashMap::new(), "probe").is_none());
        let missing = declared_all_platforms(&[dir.join("nope").to_string_lossy().to_string()]);
        assert!(find_declared_exe("__probe_test__", &missing, "probe").is_none());
    }

    /// 桌面应用（`startCommand: ""`）的回归：命令名为空时**也要认路径**。
    ///
    /// 旧实现见不到命令名就直接返回 None，于是「默认路径 / 用户手填路径」都不查，
    /// 这类工具永远显示未安装（WorkBuddy 就是这样）。
    #[test]
    fn find_declared_exe_works_without_a_command_name() {
        // 用假 id：真实 id（如 workbuddy）可能被用户在 tool-paths.json 里配了路径，
        // 那些用户路径优先级更高，会把测试指向别的目录。
        const TOOL: &str = "__probe_no_cmd__";
        let dir = probe_dir("no-command");
        let exe_name = if cfg!(windows) { "__probe_no_cmd__.exe" } else { "__probe_no_cmd__" };
        let exe = dir.join(exe_name);
        std::fs::write(&exe, "").unwrap();

        // ① 路径项是文件本身 → 不需要命令名
        let by_file = declared_all_platforms(&[exe.to_string_lossy().to_string()]);
        assert_eq!(find_declared_exe(TOOL, &by_file, "").expect("文件项应命中"), exe);

        // ② 路径项是目录 + 无命令名 → 按 id 猜同名 exe
        let by_dir = declared_all_platforms(&[dir.to_string_lossy().to_string()]);
        assert_eq!(
            find_declared_exe(TOOL, &by_dir, "").expect("目录项应猜中同名 exe"),
            exe
        );

        // ③ 目录里还有别的可执行文件时，与 id 不同名就**不猜**（宁可不认，也不认错）
        let other = dir.join(if cfg!(windows) { "unins000.exe" } else { "unins000" });
        std::fs::write(&other, "").unwrap();
        assert!(
            find_declared_exe("__probe_other__", &by_dir, "").is_none(),
            "有多个候选时不应该瞎猜"
        );
    }
}
