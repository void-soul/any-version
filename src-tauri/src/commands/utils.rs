
use std::path::{Path, PathBuf};
use std::fs;
use std::io::Write;
use std::sync::OnceLock;
use tauri::Manager;
use super::project::types::PackageManagerDef;

/// 获取用户主目录（统一入口，避免各模块重复实现）
pub fn get_home_dir() -> PathBuf {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let home = if home.is_empty() {
        std::env::var("HOME").unwrap_or_default()
    } else {
        home
    };
    if home.is_empty() {
        PathBuf::from(".")
    } else {
        PathBuf::from(home)
    }
}

/// 获取全局共享的 HTTP Client 单例，避免每次请求都重建连接池
pub fn get_http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent("Any-Version-Manager")
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

/// 在 PATH 中查找可执行文件的绝对路径（Windows 自动补齐 .exe/.cmd/.bat 后缀）
pub fn find_in_path(exe_name: &str) -> Option<PathBuf> {
    let names: Vec<String> = {
        let lower = exe_name.to_lowercase();
        if lower.ends_with(".exe") || lower.ends_with(".cmd") || lower.ends_with(".bat") {
            vec![exe_name.to_string()]
        } else {
            vec![
                exe_name.to_string(),
                format!("{}.exe", exe_name),
                format!("{}.cmd", exe_name),
            ]
        }
    };
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            for name in &names {
                let full = dir.join(name);
                if full.is_file() {
                    return Some(full);
                }
            }
        }
    }
    None
}

/// Expand {home} placeholder in path strings
pub fn expand_home(path: &str) -> String {
    if path.contains("{home}") {
        path.replace("{home}", &get_home_dir().to_string_lossy())
    } else {
        path.to_string()
    }
}

/// Generic configuration-file based cache resolver
pub fn resolve_custom_cache_path(pm: &PackageManagerDef) -> Option<String> {
    let source = pm.cache_config_source.as_ref()?;
    
    // 1. Check environment variables if specified
    for env_var in &source.env_vars {
        if let Ok(val) = std::env::var(env_var) {
            let val_trimmed = val.trim();
            if !val_trimmed.is_empty() {
                let mut resolved = val_trimmed.to_string();
                if let Some(ref suffix) = source.suffix {
                    if !resolved.ends_with(suffix) {
                        resolved = Path::new(&resolved).join(suffix).to_string_lossy().to_string();
                    }
                }
                return Some(resolved);
            }
        }
    }
    
    // 2. Check the config files in path order
    let user_home = expand_home("{home}");
    let links_dir = crate::commands::config::load_config().links_dir;
    
    for raw_path in &source.paths {
        // Expand standard placeholders
        let mut expanded = raw_path.replace("{home}", &user_home).replace("{links_dir}", &links_dir);
        
        // Match other env vars in path: e.g. {MAVEN_HOME}
        if let Ok(re_var) = regex::Regex::new(r"\{([^}]+)\}") {
            for cap in re_var.captures_iter(raw_path) {
                let var_name = &cap[1];
                if var_name != "home" && var_name != "links_dir" {
                    if let Ok(val) = std::env::var(var_name) {
                        expanded = expanded.replace(&format!("{{{}}}", var_name), &val);
                    }
                }
            }
        }
        
        let file_path = Path::new(&expanded);
        if file_path.exists() {
            if let Ok(content) = fs::read_to_string(file_path) {
                let mut resolved_val = String::new();
                if source.parser_type.eq_ignore_ascii_case("xml") {
                    if let Some(pattern) = source.keys.first() {
                        // Strip XML comments to avoid commented out settings
                        if let Ok(re_comment) = regex::Regex::new(r"(?s)<!--.*?-->") {
                            let clean_content = re_comment.replace_all(&content, "");
                            if let Ok(re_tag) = regex::Regex::new(pattern) {
                                if let Some(caps) = re_tag.captures(&clean_content) {
                                    resolved_val = caps.get(1).map(|m| m.as_str().to_string()).unwrap_or_default();
                                }
                            }
                        }
                    }
                } else if source.parser_type.eq_ignore_ascii_case("properties") {
                    for line in content.lines() {
                        let trimmed = line.trim();
                        if trimmed.starts_with('#') || trimmed.starts_with(';') || trimmed.is_empty() {
                            continue;
                        }
                        if let Some(pos) = trimmed.find('=') {
                            let key = trimmed[..pos].trim();
                            let val = trimmed[pos + 1..].trim();
                            if source.keys.iter().any(|k| k == key) {
                                resolved_val = val.to_string();
                                break;
                            }
                        }
                    }
                }
                
                let mut resolved = resolved_val.trim_matches('"').trim_matches('\'').trim().to_string();
                if !resolved.is_empty() {
                    // Apply replacements
                    for (from, to) in &source.replacements {
                        let to_expanded = to.replace("{home}", &user_home).replace("{links_dir}", &links_dir);
                        resolved = resolved.replace(from, &to_expanded);
                    }
                    // Apply suffix if specified
                    if let Some(ref suffix) = source.suffix {
                        if !resolved.ends_with(suffix) {
                            resolved = Path::new(&resolved).join(suffix).to_string_lossy().to_string();
                        }
                    }
                    return Some(resolved);
                }
            }
        }
    }
    
    None
}

/// Run a command and capture its stdout as a trimmed string
pub fn get_cmd_output(cmd: &str, args: &[&str]) -> String {
    super::hidden_cmd::hidden_cmd(cmd)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Search for an executable in PATH (Windows compatible with .exe/.cmd/.bat)
pub fn is_exe_in_path(name: &str) -> bool {
    find_in_path(name).is_some()
}

/// Dynamic description builder for package manager caches
pub fn cache_detect_evidence_dynamic(
    _pm_id: &str,
    resolved: &str,
    pm_def: &crate::commands::project::types::PackageManagerDef,
) -> (String, String) {
    let display_name = &pm_def.display_name;
    
    if let Some(ref cmd) = pm_def.cache_detect_cmd {
        (
            format!("命令 `{}` 的输出", cmd),
            format!("{} 报告的缓存目录为: {}", display_name, resolved),
        )
    } else if let Some(ref env_var) = pm_def.cache_env_var {
        (
            format!("环境变量 {} 的路径", env_var),
            format!("从环境变量解析的 {} 缓存目录为: {}", display_name, resolved),
        )
    } else if let Some(ref default_path) = pm_def.cache_default_path {
        (
            format!("默认路径配置: {}", default_path),
            format!("检测到的 {} 缓存目录为: {}", display_name, resolved),
        )
    } else {
        (
            "默认配置路径".to_string(),
            format!("检测到的 {} 缓存目录为: {}", display_name, resolved),
        )
    }
}

/// 在 XML 配置文件中原地替换指定 key 的 value 属性值。
///
/// 匹配形如：`<add key="KEY" value="OLD_VALUE" />`
/// 或：`<add key="KEY" value="OLD_VALUE"/>`
/// 将 value 替换为 `new_value`，其余内容保持不变。
///
/// 如果文件不存在或 key 不存在，则静默跳过（不报错）。
pub fn write_xml_config_key(config_path: &str, key: &str, new_value: &str) -> Result<(), String> {
    let path = std::path::Path::new(config_path);
    if !path.exists() {
        return Ok(()); // 文件不存在，静默跳过
    }

    let content = fs::read_to_string(path).map_err(|e| format!("读取 {} 失败: {}", config_path, e))?;

    // 构造匹配目标 key 的正则，替换其 value 属性
    let pattern = format!(
        r#"(<add\s+key\s*=\s*"{}"\s+value\s*=\s*")[^"]*(")"#,
        regex::escape(key)
    );
    let re = regex::Regex::new(&pattern).map_err(|e| e.to_string())?;

    if !re.is_match(&content) {
        return Ok(()); // key 不存在，静默跳过
    }

    let new_content = re.replace_all(&content, |caps: &regex::Captures| {
        format!("{}{}{}", &caps[1], new_value, &caps[2])
    }).to_string();

    if new_content != content {
        fs::write(path, new_content).map_err(|e| format!("写入 {} 失败: {}", config_path, e))?;
    }

    Ok(())
}

/// 根据 PackageManagerDef 中的 `cache_config_source.write_keys` 定义，
/// 批量将迁移后的目标路径写回对应的配置文件。
///
/// `base_path`：迁移目标的根目录（junction 指向的目录）。
/// 每个 write_key 的实际值 = base_path / value_suffix（或直接 base_path）。
pub fn apply_cache_config_writes(pm: &crate::commands::project::types::PackageManagerDef, base_path: &str) {
    let source = match &pm.cache_config_source {
        Some(s) => s,
        None => return,
    };
    let write_keys = match &source.write_keys {
        Some(k) if !k.is_empty() => k,
        _ => return,
    };

    let user_home = expand_home("{home}");

    // 找到第一个存在的配置文件路径
    let config_path = source.paths.iter()
        .map(|p| p.replace("{home}", &user_home))
        .find(|p| std::path::Path::new(p).exists());

    let config_path = match config_path {
        Some(p) => p,
        None => return, // 配置文件不存在，跳过
    };

    for wk in write_keys {
        let value = match &wk.value_suffix {
            Some(suffix) => std::path::Path::new(base_path)
                .join(suffix)
                .to_string_lossy()
                .to_string(),
            None => base_path.to_string(),
        };
        let _ = write_xml_config_key(&config_path, &wk.key, &value);
    }
}

static RESOURCE_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 设置打包环境下的静态资源目录
pub fn set_resource_dir(path: PathBuf) {
    let _ = RESOURCE_DIR.set(path);
}

/// 获取打包环境下的静态资源目录
pub fn get_resource_dir() -> Option<PathBuf> {
    RESOURCE_DIR.get().cloned()
}

/// 查找 bin/ 目录下的可执行文件或关联文件（如 ffmpeg.exe, mediamtx.exe, mediamtx.yml 等）
/// 兼容 Tauri 打包后资源目录中的 _up_/bin/ 与 bin/
pub fn find_bin_file(filename: &str) -> PathBuf {
    let mut candidates = Vec::new();

    // 1. 优先在 Tauri 2 打包后的官方资源目录 (resource_dir/_up_/bin 及 resource_dir/bin) 查找
    if let Some(res_dir) = get_resource_dir() {
        candidates.push(res_dir.join("_up_").join("bin").join(filename));
        candidates.push(res_dir.join("bin").join(filename));
        candidates.push(res_dir.join("..").join("_up_").join("bin").join(filename));
        candidates.push(res_dir.join("..").join("bin").join(filename));
    }

    // 2. exe 同目录及向上 5 层
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            candidates.push(exe_dir.join("_up_").join("bin").join(filename));
            candidates.push(exe_dir.join("bin").join(filename));
            let mut dir = exe_dir.to_path_buf();
            for _ in 0..5 {
                if let Some(parent) = dir.parent() {
                    dir = parent.to_path_buf();
                    candidates.push(dir.join("_up_").join("bin").join(filename));
                    candidates.push(dir.join("bin").join(filename));
                }
            }
        }
    }

    // 3. 当前工作目录下的 _up_/bin/ 与 bin/
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("_up_").join("bin").join(filename));
        candidates.push(cwd.join("bin").join(filename));
        candidates.push(cwd.join("..").join("_up_").join("bin").join(filename));
        candidates.push(cwd.join("..").join("bin").join(filename));
    }

    // 4. 用户配置目录 (~/.any-version/bin)
    let mut base = crate::commands::config::get_base_dir();
    base.push("bin");
    candidates.push(base.join(filename));

    for path in &candidates {
        if path.exists() {
            return path.clone();
        }
    }

    // 5. PATH 系统路径查找
    if let Some(path_exe) = find_in_path(filename) {
        return path_exe;
    }

    PathBuf::from(format!("bin/{}", filename))
}

/// 查找项目 bin/ 目录（存放 ffmpeg.exe、mediamtx.exe、mihomo.exe 等可执行文件）
/// 返回第一个已存在的 bin 目录；若都找不到则回退到相对 "bin"
pub fn get_bin_dir() -> PathBuf {
    let mut candidates = Vec::new();

    if let Some(res_dir) = get_resource_dir() {
        candidates.push(res_dir.join("_up_").join("bin"));
        candidates.push(res_dir.join("bin"));
        candidates.push(res_dir.join("..").join("_up_").join("bin"));
        candidates.push(res_dir.join("..").join("bin"));
    }

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            candidates.push(exe_dir.join("_up_").join("bin"));
            candidates.push(exe_dir.join("bin"));
            let mut dir = exe_dir.to_path_buf();
            for _ in 0..5 {
                if let Some(parent) = dir.parent() {
                    dir = parent.to_path_buf();
                    candidates.push(dir.join("_up_").join("bin"));
                    candidates.push(dir.join("bin"));
                }
            }
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("_up_").join("bin"));
        candidates.push(cwd.join("bin"));
        candidates.push(cwd.join("..").join("_up_").join("bin"));
        candidates.push(cwd.join("..").join("bin"));
    }

    let mut base = crate::commands::config::get_base_dir();
    base.push("bin");
    candidates.push(base);

    for d in &candidates {
        if d.is_dir() {
            return d.clone();
        }
    }

    PathBuf::from("bin")
}

/// 获取所有可能存在 bin 目录的基路径列表（包含 _up_/bin 与 bin）
pub fn get_bin_search_dirs() -> Vec<PathBuf> {
    let mut search_dirs = Vec::new();

    if let Some(res_dir) = get_resource_dir() {
        search_dirs.push(res_dir.join("_up_").join("bin"));
        search_dirs.push(res_dir.join("bin"));
        search_dirs.push(res_dir.join("..").join("_up_").join("bin"));
        search_dirs.push(res_dir.join("..").join("bin"));
    }

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            search_dirs.push(exe_dir.join("_up_").join("bin"));
            search_dirs.push(exe_dir.join("bin"));
            let mut dir = exe_dir.to_path_buf();
            for _ in 0..5 {
                if let Some(parent) = dir.parent() {
                    dir = parent.to_path_buf();
                    search_dirs.push(dir.join("_up_").join("bin"));
                    search_dirs.push(dir.join("bin"));
                }
            }
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        search_dirs.push(cwd.join("_up_").join("bin"));
        search_dirs.push(cwd.join("bin"));
        search_dirs.push(cwd.join("..").join("_up_").join("bin"));
        search_dirs.push(cwd.join("..").join("bin"));
    }

    let mut base = crate::commands::config::get_base_dir();
    base.push("bin");
    search_dirs.push(base);

    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        let mut p = PathBuf::from(manifest);
        p.pop();
        p.push("bin");
        search_dirs.push(p);
    }

    let mut unique = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for d in search_dirs {
        let norm = d.to_string_lossy().replace('\\', "/");
        // 排除 Tauri 打包资源的 _up_/bin 缓存（上次打包 resources 残留，
        // 会误判为「已安装」导致下载对话框永不弹出）。
        // 注意：不要排除 target/debug/bin、target/release/bin——
        // 开发态的 bin 资产就解压在 exe 同级（target/debug/bin）下，必须保留。
        if norm.contains("/_up_/bin") {
            continue;
        }
        if seen.insert(d.clone()) {
            unique.push(d);
        }
    }
    unique
}

/// 在所有候选 bin 目录中查找指定可执行文件，返回第一个存在的完整路径。
/// 用于避免 `get_bin_dir()`（返回第一个存在的 bin 目录）因干扰目录（如 src-tauri/bin 仅含 lego）而误判未安装。
pub fn find_bin_executable(name: &str) -> Option<PathBuf> {
    for d in get_bin_search_dirs() {
        let p = d.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// 返回 bin/<tool>/<tool>.exe 形式的工具可执行文件路径（如 bin/mihomo/mihomo.exe）。
/// 在各候选 bin 目录的 <tool> 子目录中查找，规避 src-tauri/bin 等不含目标工具的干扰目录。
/// 适配 bin 目录按工具分子目录的新布局（mihomo/mediamtx/ffmpeg 等）。
/// 若存在多个匹配，优先选择不在 target/ 下的源码 bin 目录（dev 下 _up_ 资源副本排在前面，
/// 但应以项目根 bin/ 为准）。
pub fn bin_tool_path(tool: &str) -> Option<PathBuf> {
    let exe = format!("{}.exe", tool);
    let mut matches: Vec<(bool, PathBuf)> = Vec::new();
    for d in get_bin_search_dirs() {
        let p = d.join(tool).join(&exe);
        if p.is_file() {
            let in_target = p.to_string_lossy().contains("target");
            matches.push((in_target, p));
        }
    }
    if matches.is_empty() {
        return None;
    }
    // 优先非 target（源码 bin）目录
    matches.sort_by_key(|(in_target, _)| *in_target);
    Some(matches.remove(0).1)
}

// ================= 运行组件（bin 资产）检测与按需下载 =================

/// ModelScope 上的运行组件压缩包（含 ffmpeg/lego/mediamtx/mihomo 四个目录）。
/// 不再随 GitHub 发布，改为首次启动按需下载。
/// 注意：ModelScope 的「网页浏览 URL」(/models/.../files/xxx) 返回的是 HTML 页面，
/// 真正的文件流要走 API 端点 /api/v1/models/{owner}/{repo}/repo?Revision={branch}&FilePath={path}，
/// 该端点返回 application/zip + 正确的 Content-Length。
const BIN_ASSETS_URL: &str =
    "https://modelscope.cn/api/v1/models/qedtcx/any-versions/repo?Revision=master&FilePath=any-version-bin.zip";

/// 需要检测/下载的运行组件工具列表。
const BIN_ASSET_TOOLS: &[&str] = &["ffmpeg", "lego", "mediamtx", "mihomo"];

/// 判断某工具是否已安装：在其 bin 子目录下递归查找任意 .exe 文件。
/// 兼容 lego 的 bin/lego/lego_xxx/lego.exe 多一层版本子目录布局。
pub fn tool_installed(tool: &str) -> bool {
    let dir = get_bin_dir().join(tool);
    if !dir.is_dir() {
        return false;
    }
    has_exe_recursive(&dir, 3)
}

fn has_exe_recursive(dir: &Path, depth: u8) -> bool {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return false,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if let Some(ext) = path.extension() {
                if ext.eq_ignore_ascii_case("exe") {
                    return true;
                }
            }
        } else if depth > 0 {
            if has_exe_recursive(&path, depth - 1) {
                return true;
            }
        }
    }
    false
}

/// 解析运行组件的安装目录：优先复用已存在的 bin 目录（开发态仓库根 bin/），
/// 否则回退到 exe 同目录的 bin/（打包态首次运行新建）。
pub fn resolve_bin_install_dir() -> PathBuf {
    for d in get_bin_search_dirs() {
        if d.is_dir() {
            return d;
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join("bin");
            let _ = std::fs::create_dir_all(&p);
            return p;
        }
    }
    PathBuf::from("bin")
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinAssetsStatus {
    /// 缺失的工具名列表（空表示全部就绪）
    pub missing: Vec<String>,
    /// 是否全部就绪
    pub all_present: bool,
    /// 安装目标目录（用于提示）
    pub install_dir: String,
}

/// 启动时检测运行组件（ffmpeg/lego/mediamtx/mihomo）是否齐全。
#[tauri::command]
pub fn check_bin_assets() -> BinAssetsStatus {
    let missing: Vec<String> = BIN_ASSET_TOOLS
        .iter()
        .filter(|&&t| !tool_installed(t))
        .map(|&t| t.to_string())
        .collect();
    let install_dir = resolve_bin_install_dir();
    BinAssetsStatus {
        all_present: missing.is_empty(),
        missing,
        install_dir: install_dir.to_string_lossy().to_string(),
    }
}

/// 进度回报负载（前端监听 bin-assets-progress 事件）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinAssetsProgress {
    pub downloaded: u64,
    pub total: u64,
    pub speed_str: String,
    pub phase: String,
}

/// 下载并解压运行组件压缩包到安装目录，期间通过事件回报进度。
#[tauri::command]
pub async fn download_bin_assets(app: tauri::AppHandle) -> Result<(), String> {
    use futures_util::StreamExt;
    use tauri::Emitter;

    let install_dir = resolve_bin_install_dir();
    std::fs::create_dir_all(&install_dir).map_err(|e| e.to_string())?;

    // 1) 下载到临时 zip
    let tmp_zip = install_dir.join("any-version-bin.tmp.zip");
    let client = reqwest::Client::builder()
        .user_agent("Any-Version-Manager")
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let _ = app.emit(
        "bin-assets-progress",
        BinAssetsProgress {
            downloaded: 0,
            total: 0,
            speed_str: "".to_string(),
            phase: "connecting".into(),
        },
    );

    let res = client
        .get(BIN_ASSETS_URL)
        .send()
        .await
        .map_err(|e| format!("下载请求失败: {}", e))?;
    if !res.status().is_success() {
        return Err(format!("HTTP 请求失败，状态码: {}", res.status()));
    }
    let total = res.content_length().unwrap_or(0);
    let mut file = std::fs::File::create(&tmp_zip).map_err(|e| e.to_string())?;
    let mut stream = res.bytes_stream();
    let mut downloaded: u64 = 0;
    let start = std::time::Instant::now();

    while let Some(item) = stream.next().await {
        let chunk = item.map_err(|e| e.to_string())?;
        std::io::Write::write_all(&mut file, &chunk).map_err(|e| e.to_string())?;
        downloaded += chunk.len() as u64;
        let elapsed = start.elapsed().as_secs_f64();
        let speed = if elapsed > 0.0 {
            downloaded as f64 / elapsed
        } else {
            0.0
        };
        let _ = app.emit(
            "bin-assets-progress",
            BinAssetsProgress {
                downloaded,
                total,
                speed_str: format_speed(speed),
                phase: "downloading".into(),
            },
        );
    }

    // 2) 解压（流式解压，避免一次性加载 209MB 到内存）
    // 校验下载到的确实是 zip（文件头 PK\x03\x04），否则给出清晰错误，
    // 避免把 HTML 错误页等当成压缩包解压，报出晦涩的 "Could not find EOCD"。
    {
        let mut sig = [0u8; 4];
        if std::fs::File::open(&tmp_zip)
            .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut sig))
            .is_ok()
            && &sig != b"PK\x03\x04"
        {
            let _ = std::fs::remove_file(&tmp_zip);
            return Err(
                "下载到的文件不是有效的 ZIP 压缩包（可能是 ModelScope 返回了网页或鉴权页），请检查下载地址或网络。"
                    .to_string(),
            );
        }
    }
    let _ = app.emit(
        "bin-assets-progress",
        BinAssetsProgress {
            downloaded,
            total,
            speed_str: "".to_string(),
            phase: "extracting".into(),
        },
    );
    unzip_to_dir(&tmp_zip, &install_dir)?;

    // 2.5) 压缩包刚解压出 mihomo 内核，立即把其中的 geo 数据文件
    // （country.mmdb / geoip.metadb / geoip.dat / geosite.dat）同步到
    // mihomo 的数据目录（AppData/Roaming/com.voidsoul.anyversion/mihomo），
    // 避免核心启动因 data_dir 缺文件而联网下载 MMDB（国内常超时）。
    sync_mihomo_geo(&app);

    // 3) 清理临时文件
    let _ = std::fs::remove_file(&tmp_zip);

    let _ = app.emit(
        "bin-assets-progress",
        BinAssetsProgress {
            downloaded,
            total,
            speed_str: "".to_string(),
            phase: "done".into(),
        },
    );
    Ok(())
}

/// 解压 zip 到目标目录（zip 内顶层为 ffmpeg/lego/mediamtx/mihomo 四个目录）。
fn unzip_to_dir(zip_path: &Path, dest: &Path) -> Result<(), String> {
    let file = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("压缩包解析失败: {}", e))?;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("读取条目失败: {}", e))?;
        let name = match entry.enclosed_name() {
            Some(n) => n.to_path_buf(),
            None => continue,
        };
        let out_path = dest.join(&name);
        // 防 zip 穿越攻击
        if !out_path.starts_with(dest) {
            continue;
        }
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut out = std::fs::File::create(&out_path).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// 格式化速度（字节/秒 → 人类可读字符串）。
fn format_speed(bytes_per_sec: f64) -> String {
    let b = bytes_per_sec;
    if b >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} GB/s", b / 1024.0 / 1024.0 / 1024.0)
    } else if b >= 1024.0 * 1024.0 {
        format!("{:.1} MB/s", b / 1024.0 / 1024.0)
    } else if b >= 1024.0 {
        format!("{:.1} KB/s", b / 1024.0)
    } else {
        format!("{:.0} B/s", b)
    }
}

/// 把 bin/mihomo 里的 geo 数据文件按需同步到 mihomo 的数据目录
/// （AppData/Roaming/com.voidsoul.anyversion/mihomo）。
///
/// 语义：检查 data_dir 下 geo 文件是否齐全（存在且非空），**不齐全才复制**，
/// 齐全则跳过。mihomo 核心启动时会读取 data_dir 下的 country.mmdb /
/// geoip.metadb 等，若缺失会联网去 GitHub 下载 MMDB（国内常超时导致启动失败）。
/// 内核启动前调用本函数即可保证 data_dir 有可用副本，绝不联网。
pub fn sync_mihomo_geo(app: &tauri::AppHandle) {
    let data_dir = match app.path().app_data_dir() {
        Ok(d) => d.join("mihomo"),
        Err(_) => return,
    };
    if std::fs::create_dir_all(&data_dir).is_err() {
        return;
    }
    // 日志同时写 stderr 与 data_dir/sync-geo.log（打包态无终端，便于排查）
    let log_path = data_dir.join("sync-geo.log");
    let log = |line: String| {
        eprintln!("[mihomo] {line}");
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
            let _ = std::writeln!(f, "[{ts}] {line}");
        }
    };
    // 源目录优先级：
    // 1) 打包态 exe 同级 bin/mihomo（安装目录，必定含 geo 文件）——
    //    最可靠，不依赖多目录猜测。
    // 2) 回退到 bin_tool_path 的探测结果。
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            candidates.push(exe_dir.join("bin").join("mihomo"));
        }
    }
    if let Some(p) = bin_tool_path("mihomo") {
        candidates.push(p);
    }
    // 取第一个确实存在 geo 文件的源目录
    let Some(bin_mihomo) = candidates.into_iter().find(|d| {
        d.join("country.mmdb").is_file()
            || d.join("geoip.metadb").is_file()
            || d.join("geoip.dat").is_file()
    }) else {
        log(format!(
            "未找到含 geo 文件的 bin/mihomo 源目录，跳过同步（data_dir={}）",
            data_dir.display()
        ));
        return;
    };
    log(format!("geo 源目录: {}", bin_mihomo.display()));
    for f in ["country.mmdb", "geoip.dat", "geoip.metadb", "geosite.dat"] {
        let src = bin_mihomo.join(f);
        let dst = data_dir.join(f);
        if !src.exists() {
            log(format!("源目录缺少 {f}，跳过"));
            continue;
        }
        // 仅当目标缺失或为空（0 字节，可能上次复制中断）时才复制，齐全则跳过。
        let need_copy = match std::fs::metadata(&dst) {
            Ok(m) => m.len() == 0,
            Err(_) => true,
        };
        if need_copy {
            match std::fs::copy(&src, &dst) {
                Ok(_) => log(format!("geo 文件缺失/为空，已复制 {f} -> {}", dst.display())),
                Err(e) => log(format!("复制 geo 文件 {f} 失败: {e}")),
            }
        } else {
            log(format!("geo 文件 {f} 已齐全，跳过"));
        }
    }
}


