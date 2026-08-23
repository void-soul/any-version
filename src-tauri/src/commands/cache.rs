use std::fs;
use std::path::{Path, PathBuf};
use serde::{Serialize, Deserialize};
use walkdir::WalkDir;
use tauri::Emitter;

use super::config::{get_data_dir, MigrateProgress};
use super::utils::{expand_home, get_cmd_output};

/// 把技能相关调试日志同时写入文件，便于在打包运行时排查（终端 stderr 不可见）。
pub(crate) fn skill_debug_log(line: &str) {
    let path = get_data_dir().join("skill-debug.log");
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        use std::io::Write;
        let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let _ = writeln!(f, "[{}] {}", ts, line);
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CacheInfo {
    pub name: String,
    pub installed: bool,
    pub path: String,
    pub size: String,
    pub is_link: bool,
    pub real_target: String,
    /// 检测依据：该缓存路径是通过哪个配置文件 / 命令得到的
    pub detect_source: String,
    /// 检测依据：读到的实际内容
    pub detect_content: String,
}

pub fn get_dir_size(path: &Path) -> u64 {
    let mut total_size = 0;
    for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
        if let Ok(metadata) = entry.metadata() {
            if metadata.is_file() {
                total_size += metadata.len();
            }
        }
    }
    total_size
}

pub fn format_bytes(bytes: u64) -> String {
    const UNIT: u64 = 1024;
    if bytes < UNIT {
        return format!("{} B", bytes);
    }
    let mut div = UNIT;
    let mut exp = 0;
    let mut n = bytes / UNIT;
    while n >= UNIT {
        div *= UNIT;
        exp += 1;
        n /= UNIT;
    }
    let suffix = match exp {
        0 => "KiB",
        1 => "MiB",
        2 => "GiB",
        3 => "TiB",
        _ => "PiB",
    };
    format!("{:.2} {}", (bytes as f64) / (div as f64), suffix)
}

/// Windows 的 mklink 无法解析混用 `/` 与 `\` 的路径，统一成反斜杠。
/// 例如 resolve_path 由 skills-scan.json 的 "~/.trae-cn/skills" 生成
/// "C:\Users\...\ .trae-cn/skills"，其中的 `/` 不会被 Path::join 转换，
/// 直接喂给 mklink 会报“无效的参数”。其余平台原样返回。
fn normalize_separators(p: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(p.to_string_lossy().replace('/', "\\"))
    }
    #[cfg(not(windows))]
    {
        p.to_path_buf()
    }
}

/// 解码子进程输出：优先按 UTF-8，非法则回退按 GBK（代码页 936）。
/// 中文 Windows 的 `cmd /c mklink` 输出是 GBK 编码，直接 from_utf8_lossy 会乱码。
pub fn decode_cp_output(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => {
            #[cfg(windows)]
            {
                let (cow, _, _) = encoding_rs::GBK.decode(bytes);
                cow.into_owned()
            }
            #[cfg(not(windows))]
            {
                String::from_utf8_lossy(bytes).into_owned()
            }
        }
    }
}

pub fn create_junction(link_path: &Path, target_path: &Path) -> Result<(), String> {
    // 归一化分隔符（Windows 上 mklink 只能吃反斜杠），避免混合分隔符导致“无效的参数”。
    let link_norm = normalize_separators(link_path);
    let target_norm = normalize_separators(target_path);
    let link_path = link_norm.as_path();
    let target_path = target_norm.as_path();

    // mklink /J 的目标必须是绝对路径：相对目标会以链接父目录解析而失败。
    let target_abs = if target_path.is_absolute() {
        target_path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_default()
            .join(target_path)
    };
    if link_path.exists() || link_path.is_symlink() {
        // 判断是否为 junction（目录重解析点）：symlink_metadata 的 file_type().is_symlink()
        // 对 junction 返回 true（Windows 上 junction 也是 reparse point）。
        let is_reparse = fs::symlink_metadata(link_path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);
        if is_reparse {
            // junction：只用 remove_dir 删除链接本身，绝不跟随删除目标内容。
            let _ = fs::remove_dir(link_path);
            // remove_dir 失败（如占用/权限）时，用 rmdir 兜底（同样只删链接本身）。
            if link_path.exists() {
                let _ = fs::remove_file(link_path);
            }
        } else {
            // 普通目录：递归删除（此时确实是真实目录，可安全 remove_dir_all）。
            fs::remove_dir_all(link_path).map_err(|e| format!("删除旧链接失败: {}", e))?;
        }
    }
    if let Some(parent) = link_path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::create_dir_all(&target_abs).map_err(|e| e.to_string())?;
    let link_str = link_path.to_string_lossy().to_string();
    let target_str = target_abs.to_string_lossy().to_string();
    let pre = format!(
        "create_junction: cmd /c mklink /J \"{}\" \"{}\" | link_exists={} target_exists={}",
        link_str,
        target_str,
        link_path.exists(),
        target_abs.exists()
    );
    eprintln!("[skill] {}", pre);
    skill_debug_log(&pre);
    let output = super::hidden_cmd::hidden_cmd("cmd")
        .args(&["/c", "mklink", "/J", &link_str, &target_str])
        .output()
        .map_err(|e| e.to_string())?;
    let stdout = decode_cp_output(&output.stdout);
    let stderr = decode_cp_output(&output.stderr);
    // 成功时不打印 mklink 的中文 stdout 提示（不同代码页会乱码），只记录英文状态。
    let post = format!(
        "create_junction: ok={} status={:?} link=\"{}\" -> \"{}\"",
        output.status.success(),
        output.status.code(),
        link_str,
        target_str
    );
    eprintln!("[skill] {}", post);
    skill_debug_log(&post);
    if !output.status.success() {
        let err = format!(
            "create_junction failed: stdout={} stderr={} (link=\"{}\", target=\"{}\")",
            stdout, stderr, link_str, target_str
        );
        skill_debug_log(&err);
        return Err(err);
    }
    Ok(())
}



pub fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> std::io::Result<()> {
    fs::create_dir_all(&dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(entry.path(), dst.as_ref().join(entry.file_name()))?;
        } else {
            fs::copy(entry.path(), dst.as_ref().join(entry.file_name()))?;
        }
    }
    Ok(())
}

/// 带进度事件的目录复制
pub fn copy_dir_all_with_progress(
    src: impl AsRef<Path>,
    dst: impl AsRef<Path>,
    app_handle: Option<&tauri::AppHandle>,
) -> std::io::Result<()> {
    // 先统计总文件数
    let total_files = WalkDir::new(&src).into_iter().filter_map(|e| e.ok()).count();
    let mut current = 0usize;

    fs::create_dir_all(&dst)?;
    for entry in WalkDir::new(&src) {
        let entry = entry?;
        let rel_path = entry.path().strip_prefix(&src).unwrap_or(entry.path());
        let dest_path = dst.as_ref().join(rel_path);

        if entry.file_type().is_dir() {
            fs::create_dir_all(&dest_path)?;
        } else {
            current += 1;
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(handle) = app_handle {
                let _ = handle.emit("migrate-progress", MigrateProgress {
                    stage: "复制文件".to_string(),
                    current,
                    total: total_files,
                    file_name: name,
                });
            }
            fs::copy(entry.path(), &dest_path)?;
        }
    }
    Ok(())
}

/// 移动目录（保留进度事件）。
/// 优先对每条目使用 `fs::rename`（同盘原子移动，只改目录项、不读写文件内容，
/// 不受源文件「只读/系统」属性或进程锁定影响，避免 `fs::copy` 在 Windows 上
/// 报 os error 5「拒绝访问」）。仅当条目跨盘 rename 失败时才回退到 copy。
/// 目录本身逐层 rename，最终源目录被清空后可被调用方删除。
pub fn move_dir_with_progress(
    app_handle: &tauri::AppHandle,
    src: impl AsRef<Path>,
    dst: impl AsRef<Path>,
) -> std::io::Result<()> {
    let src = src.as_ref();
    let dst = dst.as_ref();
    let total = WalkDir::new(src).follow_links(false).into_iter().filter_map(|e| e.ok()).count();
    let mut current = 0usize;

    fs::create_dir_all(dst)?;

    // 先 rename 整个顶层（若 src 与 dst 同盘且 src 非 junction，rename 可一次性完成）
    if let Err(_) = fs::rename(src, dst) {
        // rename 失败（跨盘或 src 正在被占用），退化为逐条目移动：先建目标骨架，再逐文件 rename/copy
        for entry in WalkDir::new(src).follow_links(false) {
            let entry = entry?;
            let rel = entry.path().strip_prefix(src).unwrap_or(entry.path());
            let dest = dst.join(rel);

            if entry.file_type().is_dir() {
                fs::create_dir_all(&dest)?;
            } else {
                current += 1;
                let name = entry.file_name().to_string_lossy().to_string();
                let _ = app_handle.emit("migrate-storage-progress", MigrateStorageProgress {
                    stage: "移动文件中".to_string(),
                    current,
                    total,
                    file_name: name,
                });
                if let Err(_) = fs::rename(entry.path(), &dest) {
                    // 跨盘或 rename 失败：回退 copy，再删源文件
                    fs::copy(entry.path(), &dest)?;
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
        // 逐条目移动后删除已清空的源目录结构
        remove_dir_all_forced(src)?;
    } else {
        let _ = app_handle.emit("migrate-storage-progress", MigrateStorageProgress {
            stage: "移动文件中".to_string(),
            current: total,
            total,
            file_name: String::new(),
        });
    }

    Ok(())
}

/// 强制删除目录：先用标准 `fs::remove_dir_all`，若因只读/系统属性文件或进程锁定
/// 报 os error 5（拒绝访问）而失败，则用 `cmd /c rmdir /s /q` 兜底（rmdir 会忽略只读属性）。
pub fn remove_dir_all_forced(path: impl AsRef<Path>) -> std::io::Result<()> {
    let path = path.as_ref();
    if !path.exists() {
        return Ok(());
    }
    match fs::remove_dir_all(path) {
        Ok(_) => Ok(()),
        Err(e) => {
            // 仅在确属权限/拒绝访问类错误时走 rmdir 兜底，避免掩盖其它真实错误
            let is_access_denied = e.raw_os_error() == Some(5)
                || e.kind() == std::io::ErrorKind::PermissionDenied;
            if !is_access_denied {
                return Err(e);
            }
            let p = path.to_string_lossy().to_string();
            let output = super::hidden_cmd::hidden_cmd("cmd")
                .args(&["/c", "rmdir", "/s", "/q", &p])
                .output();
            match output {
                Ok(o) if o.status.success() => Ok(()),
                Ok(o) => Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("rmdir 兜底失败: {}", String::from_utf8_lossy(&o.stderr)),
                )),
                Err(cmd_err) => Err(cmd_err),
            }
        }
    }
}

/// 计算缓存条目信息：检查是否符号链接、计算目录大小。
fn build_cache_info(
    name: &str,
    installed: bool,
    path: &str,
    detect_source: &str,
    detect_content: &str,
) -> CacheInfo {
    let clean_path = Path::new(path);
    let mut is_link = false;
    let mut real_target = String::new();

    if let Ok(metadata) = fs::symlink_metadata(clean_path) {
        if metadata.file_type().is_symlink() {
            if let Ok(eval_path) = fs::read_link(clean_path) {
                is_link = true;
                real_target = eval_path.to_string_lossy().to_string();
            } else if let Ok(eval_path) = fs::canonicalize(clean_path) {
                let canonical = eval_path.to_string_lossy().to_string();
                let canonical_clean = canonical.trim_start_matches(r"\\?\").to_string();
                if canonical_clean != clean_path.to_string_lossy().to_string() {
                    is_link = true;
                    real_target = canonical_clean;
                }
            }
        }
    }

    let size_path = if is_link { Path::new(&real_target) } else { clean_path };
    let size_bytes = get_dir_size(size_path);

    CacheInfo {
        name: name.to_string(),
        installed,
        path: clean_path.to_string_lossy().to_string(),
        size: format_bytes(size_bytes),
        is_link,
        real_target,
        detect_source: detect_source.to_string(),
        detect_content: detect_content.to_string(),
    }
}

/// 解析附加缓存路径：先执行检测命令，再回退到默认路径模板。
/// 返回 (解析后的路径, 检测依据描述, 检测内容)。
fn resolve_extra_cache_path(
    detect_cmd: Option<&str>,
    detect_json_path: Option<&str>,
    default_path: Option<&str>,
    display_name: &str,
) -> Option<(String, String, String)> {
    let mut resolved = String::new();
    let mut source = String::new();
    let mut content = String::new();

    if let Some(cmd) = detect_cmd {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        if !parts.is_empty() {
            let out = get_cmd_output(parts[0], &parts[1..]);
            if let Some(path) = super::utils::resolve_detected_path(&out, detect_json_path) {
                resolved = path;
                source = format!("命令 `{}` 的输出", cmd);
                content = format!("{} 报告的缓存目录为: {}", display_name, resolved);
            }
        }
    }
    if resolved.is_empty() {
        if let Some(dp) = default_path {
            resolved = expand_home(dp);
            source = format!("默认路径配置: {}", dp);
            content = format!("检测到的 {} 缓存目录为: {}", display_name, resolved);
        }
    }

    let trimmed = resolved.trim_matches('"').trim_matches('\'').trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some((trimmed, source, content))
    }
}

#[tauri::command]
pub fn get_caches_list() -> Result<Vec<CacheInfo>, String> {
    use super::project::registry;
    use super::utils::{expand_home, get_cmd_output, is_exe_in_path, cache_detect_evidence_dynamic, resolve_detected_path};
    
    let mut list = Vec::new();
    
    // Load all projects from registry
    for project in registry::registry() {
        for pm in &project.package_managers {
            // Check if this package manager configures cache detection/path
            if pm.cache_detect_cmd.is_some() || pm.cache_default_path.is_some() || pm.cache_config_source.is_some() {
                // Determine if installed by checking if version_exe or id is in PATH
                let exe_name = pm.version_exe.as_deref().unwrap_or(&pm.id);
                let installed = is_exe_in_path(exe_name);
                
                // Resolve path: try custom config resolver first, then cmd, then default_path
                let mut resolved_path = super::utils::resolve_custom_cache_path(pm).unwrap_or_default();
                
                if resolved_path.is_empty() {
                    if let Some(ref cmd) = pm.cache_detect_cmd {
                        let parts: Vec<&str> = cmd.split_whitespace().collect();
                        if !parts.is_empty() {
                            let out = get_cmd_output(parts[0], &parts[1..]);
                            if let Some(path) = resolve_detected_path(&out, pm.cache_detect_json_path.as_deref()) {
                                resolved_path = path;
                            }
                        }
                    }
                }
                
                if resolved_path.is_empty() {
                    if let Some(ref default_path) = pm.cache_default_path {
                        resolved_path = expand_home(default_path);
                    }
                }
                
                let trimmed_path = resolved_path.trim_matches('"').trim_matches('\'').trim().to_string();
                if trimmed_path.is_empty() {
                    continue;
                }

                let (detect_source, detect_content) = cache_detect_evidence_dynamic(&pm.id, &resolved_path, pm);
                
                // Avoid duplicates in the cache list
                let info = build_cache_info(&pm.id, installed, &trimmed_path, &detect_source, &detect_content);
                if !list.iter().any(|c: &CacheInfo| c.path == info.path) {
                    list.push(info);
                }
            }

            // 附加缓存目录（一个包管理器可有多个缓存，如 pnpm 的 store + 元数据 cache-dir）
            if !pm.extra_caches.is_empty() {
                let exe_name = pm.version_exe.as_deref().unwrap_or(&pm.id);
                let installed = is_exe_in_path(exe_name);

                for extra in &pm.extra_caches {
                    if let Some((extra_path, extra_source, extra_content)) = resolve_extra_cache_path(
                        extra.detect_cmd.as_deref(),
                        extra.detect_json_path.as_deref(),
                        extra.default_path.as_deref(),
                        &extra.display_name,
                    ) {
                        let name = format!("{}.{}", pm.id, extra.id);
                        let info = build_cache_info(&name, installed, &extra_path, &extra_source, &extra_content);
                        if !list.iter().any(|c: &CacheInfo| c.path == info.path) {
                            list.push(info);
                        }
                    }
                }
            }
        }
    }
    
    Ok(list)
}

#[tauri::command]
pub fn migrate_cache_path(name: String, new_path: String) -> Result<(), String> {
    let list = get_caches_list()?;
    let cache_info = list.iter().find(|c| c.name == name)
        .ok_or_else(|| format!("未找到缓存: {}", name))?;

    let orig_path = Path::new(&cache_info.path);
    let target_path = Path::new(&new_path);

    if orig_path == target_path {
        return Err("原路径与目标路径相同，无需迁移".to_string());
    }

    // 迁移标记：防止上次中断留下的半拷贝目录被再次当作目标（拷贝非原子）
    const MIGRATE_MARKER: &str = ".anyversion-migrating";
    let marker = target_path.join(MIGRATE_MARKER);
    if marker.exists() {
        return Err(format!(
            "目标目录 {} 存在未完成的迁移标记（{}），可能上次迁移被中断。\n请确认后手动清理该目录再重试。",
            target_path.display(),
            marker.display()
        ));
    }

    // Ensure target directory exists
    fs::create_dir_all(target_path).map_err(|e| format!("无法创建目标目录: {}", e))?;
    fs::write(&marker, "in-progress").map_err(|e| format!("写入迁移标记失败: {}", e))?;

    // Check if original path is already a junction/symlink
    let is_symlink = fs::symlink_metadata(orig_path).map(|m| m.file_type().is_symlink()).unwrap_or(false);

    let result = (|| -> Result<(), String> {
        if is_symlink {
            // Just remove old junction link
            fs::remove_file(orig_path).map_err(|e| format!("无法移除已有的旧链接: {}", e))?;
        } else {
            // Move files
            if orig_path.exists() {
                copy_dir_all(orig_path, target_path).map_err(|e| format!("复制缓存文件失败: {}", e))?;
                fs::remove_dir_all(orig_path).map_err(|e| format!("清空原缓存目录失败: {}", e))?;
            }
        }

        // Create Junction
        create_junction(orig_path, target_path)?;
        Ok(())
    })();

    // 无论成功失败都清理标记（失败时保留半拷贝供用户检查，但不再阻塞后续迁移）
    let _ = fs::remove_file(&marker);
    result
}

/// 存储迁移进度（与 config::MigrateProgress 区分，用于 cache/data 迁移）
#[derive(serde::Serialize, Clone, Debug)]
pub struct MigrateStorageProgress {
    pub stage: String,
    pub current: usize,
    pub total: usize,
    pub file_name: String,
}

/// 迁移缓存/数据目录 — 统一处理 cache 和 data 两种类型。
/// - storage_kind = "cache": 如果 delete_old_first=true，直接删除旧目录再建 junction（快）
///                           如果 delete_old_first=false，先拷贝再建 junction
/// - storage_kind = "data":  必须拷贝，不可先删（安全），拷贝后建 junction
pub fn migrate_pkg_storage_impl(
    app_handle: &tauri::AppHandle,
    orig_path: &str,
    new_path: &str,
    storage_kind: &str,
    delete_old_first: bool,
) -> Result<(), String> {
    let orig = Path::new(orig_path);
    let target = Path::new(new_path);

    if orig == target {
        return Err("原路径与目标路径相同".to_string());
    }
    if !orig.exists() {
        if let Some(parent) = orig.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("创建源目录的父级失败: {}", e))?;
        }
        fs::create_dir_all(target).map_err(|e| format!("创建目标目录失败: {}", e))?;
        create_junction(orig, target)?;

        let _ = app_handle.emit("migrate-storage-progress", MigrateStorageProgress {
            stage: "已完成（源路径不存在，直接创建链接）".to_string(),
            current: 1,
            total: 1,
            file_name: String::new(),
        });
        return Ok(());
    }

    let can_fast_path = storage_kind == "cache" && delete_old_first;

    // --- 预处理 ---
    // 删除旧 junction 链接本身（不删目标内容）
    let is_symlink = fs::symlink_metadata(orig)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);
    if is_symlink {
        let _ = fs::remove_dir(orig);
        if orig.exists() {
            fs::remove_dir_all(orig).map_err(|e| format!("删除旧链接失败: {}", e))?;
        }
    }

    fs::create_dir_all(target).map_err(|e| format!("创建目标目录失败: {}", e))?;

    if can_fast_path {
        // 快路径：删除旧数据后直接建 junction
        if !is_symlink && orig.exists() {
            fs::remove_dir_all(orig).map_err(|e| format!("删除旧缓存目录失败: {}", e))?;
        }
        create_junction(orig, target)?;

        let _ = app_handle.emit("migrate-storage-progress", MigrateStorageProgress {
            stage: "已完成（快速模式）".to_string(),
            current: 1,
            total: 1,
            file_name: String::new(),
        });
    } else {
        // 慢路径：移动到新目录再建 junction（适用于 data 或 cache 但用户选择迁移）
        // 优先用 rename（同盘原子移动，不涉及文件读写，避开只读/系统属性与进程锁定导致的 os error 5），
        // 仅跨盘时 fallback 到带进度的 copy。复制/移动完成后删除原始目录（os error 5 用 rmdir 兜底）。
        let total = WalkDir::new(orig).follow_links(false).into_iter().filter_map(|e| e.ok()).count();
        let _ = app_handle.emit("migrate-storage-progress", MigrateStorageProgress {
            stage: "移动文件中".to_string(),
            current: 0,
            total,
            file_name: String::new(),
        });

        move_dir_with_progress(app_handle, orig, target)
            .map_err(|e| format!("移动文件失败: {}", e))?;

        // 移动完成后删除原始目录（rename 方式已清空，rmdir 兜底处理残留只读/锁定文件）
        if !is_symlink && orig.exists() {
            remove_dir_all_forced(orig)
                .map_err(|e| format!("删除原始目录失败: {}", e))?;
        }

        create_junction(orig, target)?;

        let _ = app_handle.emit("migrate-storage-progress", MigrateStorageProgress {
            stage: "已完成".to_string(),
            current: total,
            total,
            file_name: String::new(),
        });
    }

    Ok(())
}

/// 清理缓存进度
#[derive(serde::Serialize, Clone, Debug)]
pub struct CleanProgress {
    pub stage: String,
    pub current: usize,
    pub total: usize,
    pub file_name: String,
}

/// 清理缓存 — 删除缓存目录中的所有文件，带进度事件。
/// 不跟随 junction（安全），不删除目录本身（保留结构）。
pub fn clean_pkg_cache_impl(app_handle: &tauri::AppHandle, cache_path: &str) -> Result<(), String> {
    let cache = Path::new(&cache_path);

    // 检查路径是否存在
    if !cache.exists() {
        return Err("缓存目录不存在（可能已被清理）".to_string());
    }

    // 如果是 junction，只删除链接本身（不跟随），然后重新创建一个空目录
    if let Ok(meta) = fs::symlink_metadata(cache) {
        if meta.file_type().is_symlink() {
            let _ = fs::remove_dir(cache);
            if cache.exists() {
                fs::remove_dir_all(cache).map_err(|e| format!("删除旧链接失败: {}", e))?;
            }
            fs::create_dir_all(cache).map_err(|e| format!("重新创建目录失败: {}", e))?;

            let _ = app_handle.emit("clean-cache-progress", CleanProgress {
                stage: "清理完成".to_string(),
                current: 1,
                total: 1,
                file_name: String::new(),
            });
            return Ok(());
        }
    }

    // 不跟随符号链接/junction — 防止意外删除链接目标中的文件
    let entries: Vec<_> = WalkDir::new(cache)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .collect();

    let total = entries.iter().filter(|e| e.file_type().is_file() && e.depth() > 0).count();
    if total == 0 {
        let _ = app_handle.emit("clean-cache-progress", CleanProgress {
            stage: "清理完成（无需清理）".to_string(),
            current: 0,
            total: 0,
            file_name: String::new(),
        });
        return Ok(());
    }

    let _ = app_handle.emit("clean-cache-progress", CleanProgress {
        stage: "扫描完成".to_string(),
        current: 0,
        total,
        file_name: String::new(),
    });

    // 从深到浅删除文件
    let mut current = 0usize;
    for entry in entries.iter().rev() {
        if entry.file_type().is_file() && entry.depth() > 0 {
            current += 1;
            let _ = app_handle.emit("clean-cache-progress", CleanProgress {
                stage: "清理中".to_string(),
                current,
                total,
                file_name: entry.file_name().to_string_lossy().to_string(),
            });
            let _ = fs::remove_file(entry.path());
        }
    }

    // 删除空子目录（保留缓存根目录本身）
    for entry in entries.iter().rev() {
        if entry.file_type().is_dir() && entry.depth() > 0 && entry.path() != cache {
            let _ = fs::remove_dir(entry.path());
        }
    }

    let _ = app_handle.emit("clean-cache-progress", CleanProgress {
        stage: "清理完成".to_string(),
        current: total,
        total,
        file_name: String::new(),
    });

    Ok(())
}

/// 保留旧命令别名 — 内部模块可用
pub fn migrate_cache_path_raw(orig_path_str: &str, new_path_str: &str) -> Result<(), String> {
    // 内部调用保持兼容（不发射进度事件）
    let orig = Path::new(orig_path_str);
    let target = Path::new(new_path_str);
    if orig == target { return Err("原路径与目标路径相同".to_string()); }
    if !orig.exists() { return Err("源路径不存在".to_string()); }

    let is_symlink = fs::symlink_metadata(orig).map(|m| m.file_type().is_symlink()).unwrap_or(false);
    if is_symlink {
        let _ = fs::remove_dir(orig);
        if orig.exists() {
            fs::remove_dir_all(orig).map_err(|e| format!("删除旧链接失败: {}", e))?;
        }
    } else {
        fs::remove_dir_all(orig).map_err(|e| format!("删除旧目录失败: {}", e))?;
    }
    fs::create_dir_all(target).map_err(|e| format!("创建目标目录失败: {}", e))?;
    create_junction(orig, target)
}
pub fn move_cache_path_raw(orig_path_str: &str, new_path_str: &str) -> Result<(), String> {
    migrate_cache_path_raw(orig_path_str, new_path_str)
}

#[cfg(test)]
mod format_tests {
    use super::format_bytes;

    #[test]
    fn format_bytes_units() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.00 KiB");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5.00 MiB");
        assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.00 GiB");
        assert_eq!(format_bytes(2 * 1024u64.pow(4)), "2.00 TiB");
    }
}
