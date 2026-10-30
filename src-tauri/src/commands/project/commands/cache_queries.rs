use super::*;

/// 获取项目的旧版安装备份数据（供"旧版数据"选项卡展示）
#[derive(serde::Serialize, Clone, Debug)]
pub struct LegacyBackupInfo {
    pub project_id: String,
    pub install_source: Option<String>,
    pub install_root: Option<String>,
    pub version: Option<String>,
    pub backed_env_vars: std::collections::HashMap<String, String>,
    pub removed_path_entries: Vec<String>,
    pub timestamp: u64,
}

#[tauri::command]
pub fn get_legacy_backup(id: String) -> Result<Option<LegacyBackupInfo>, String> {
    let base_dir = crate::commands::config::get_data_dir();
    let backup_dir = base_dir.join("backup");
    if !backup_dir.exists() {
        return Ok(None);
    }

    // 查找该项目的备份文件
    let mut best: Option<(u64, std::path::PathBuf)> = None;
    if let Ok(entries) = std::fs::read_dir(&backup_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with(&format!("manage_{}_", id)) && name_str.ends_with(".json") {
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    if let Ok(data) = serde_json::from_str::<serde_json::Value>(&content) {
                        let ts = data.get("timestamp").and_then(|v| v.as_u64()).unwrap_or(0);
                        if best.is_none() || ts > best.as_ref().unwrap().0 {
                            best = Some((ts, entry.path()));
                        }
                    }
                }
            }
        }
    }

    match best {
        Some((_ts, path)) => {
            let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let data: serde_json::Value =
                serde_json::from_str(&content).map_err(|e| e.to_string())?;

            let backed_env_vars = data
                .get("backed_env_vars")
                .and_then(|v| v.as_object())
                .map(|obj| {
                    obj.iter()
                        .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                        .collect()
                })
                .unwrap_or_default();

            let removed_path_entries = data
                .get("backed_path_entries")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            Ok(Some(LegacyBackupInfo {
                project_id: id,
                install_source: data
                    .get("legacy_install_source")
                    .and_then(|v| v.as_str().map(String::from)),
                install_root: data
                    .get("legacy_install_root")
                    .and_then(|v| v.as_str().map(String::from)),
                version: data
                    .get("legacy_version")
                    .and_then(|v| v.as_str().map(String::from)),
                backed_env_vars,
                removed_path_entries,
                timestamp: data.get("timestamp").and_then(|v| v.as_u64()).unwrap_or(0),
            }))
        }
        None => Ok(None),
    }
}

/// 获取包管理器缓存信息
#[derive(serde::Serialize, Clone, Debug)]
pub struct PkgCacheInfo {
    pub path: String,
    pub size: String,
    pub is_link: bool,
    pub real_target: String,
    /// 如果父目录是 junction（如 C:\Users\...\Yarn → D:\Yarn），则返回父目录信息
    pub parent_link: Option<ParentLinkInfo>,
}

#[derive(serde::Serialize, Clone, Debug)]
pub struct ParentLinkInfo {
    /// 父目录路径（如 C:\Users\...\Yarn）
    pub parent_path: String,
    /// 父目录 junction 实际目标（如 D:\Yarn）
    pub parent_target: String,
    /// 父目录下缓存子目录的相对路径（如 Cache）
    pub child_rel: String,
}

pub(super) fn check_parent_junction(cache_path: &str) -> Option<ParentLinkInfo> {
    let p = std::path::Path::new(cache_path);
    let parent = p.parent()?;
    if let Ok(meta) = std::fs::symlink_metadata(parent) {
        if meta.file_type().is_symlink() {
            let target = std::fs::read_link(parent)
                .map(|t| t.to_string_lossy().to_string())
                .unwrap_or_default();
            let child_rel = p.file_name()?.to_string_lossy().to_string();
            return Some(ParentLinkInfo {
                parent_path: parent.to_string_lossy().to_string(),
                parent_target: target,
                child_rel,
            });
        }
    }
    None
}

/// 解析附加缓存目录路径（pnpm.cache 等）。优先执行 detect_cmd，再回退到 default_path。
pub(super) fn resolve_extra_cache_path_dynamic(
    project_id: &str,
    extra: &crate::commands::project::types::ExtraCacheDef,
) -> Result<String, String> {
    use crate::commands::utils::{expand_home, resolve_detected_path, run_simple_command_checked};

    let mut resolved = String::new();
    if let Some(ref cmd) = extra.detect_cmd {
        let out = if cmd.starts_with("pnpm config get") {
            run_simple_command_checked(cmd)
        } else {
            run_cmd_capture(cmd.clone(), Some(project_id.to_string()))
        }
        .map_err(|e| format!("附加缓存检测失败 ({}): {}", extra.display_name, e))?;
        resolved = resolve_detected_path(&out, extra.detect_json_path.as_deref())
            .ok_or_else(|| format!("附加缓存检测未返回有效路径: {}", extra.display_name))?;
    } else if let Some(ref dp) = extra.default_path {
        resolved = expand_home(dp);
    }
    let cleaned = resolved
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_string();
    if cleaned.is_empty() {
        return Err(format!("未能检测到附加缓存目录: {}", extra.display_name));
    }
    Ok(cleaned)
}

pub(super) fn resolve_pkg_storage_path_dynamic(
    project_id: &str,
    pm_id: &str,
    storage_kind: &str,
    extra_cache_id: Option<&str>,
) -> Result<String, String> {
    use crate::commands::project::registry;
    use crate::commands::utils::{expand_home, resolve_custom_cache_path};

    let project = registry::registry()
        .into_iter()
        .find(|p| p.id.eq_ignore_ascii_case(project_id))
        .ok_or_else(|| format!("未找到项目: {}", project_id))?;

    let pm = project
        .package_managers
        .iter()
        .find(|m| m.id.eq_ignore_ascii_case(pm_id))
        .ok_or_else(|| format!("在项目 {} 中未找到包管理器: {}", project_id, pm_id))?;

    // 附加缓存目录（pnpm.cache 等）：优先按 extra_cache_id 解析
    if let Some(extra_id) = extra_cache_id {
        if let Some(extra) = pm.extra_caches.iter().find(|e| e.id == extra_id) {
            return resolve_extra_cache_path_dynamic(project_id, extra);
        }
        return Err(format!("包管理器 {} 中未找到附加缓存: {}", pm_id, extra_id));
    }

    let mut resolved_path = String::new();

    if storage_kind == "cache" {
        resolved_path = resolve_custom_cache_path(pm).unwrap_or_default();
        if resolved_path.is_empty() {
            if let Some(ref cmd) = pm.cache_detect_cmd {
                if let Ok(out) = run_cmd_capture(cmd.clone(), Some(project_id.to_string())) {
                    if let Some(path) =
                        resolve_detected_path(&out, pm.cache_detect_json_path.as_deref())
                    {
                        resolved_path = path;
                    }
                }
            }
        }
        if resolved_path.is_empty() {
            if let Some(ref env_var) = pm.cache_env_var {
                if let Ok(val) = std::env::var(env_var) {
                    if !val.trim().is_empty() {
                        resolved_path = val.trim().to_string();
                    }
                }
            }
        }
        if resolved_path.is_empty() {
            if let Some(ref default_path) = pm.cache_default_path {
                resolved_path = expand_home(default_path);
            }
        }
    } else if storage_kind == "data" {
        if let Some(ref cmd) = pm.data_detect_cmd {
            if let Ok(out) = run_cmd_capture(cmd.clone(), Some(project_id.to_string())) {
                if let Some(path) = resolve_detected_path(&out, None) {
                    resolved_path = path;
                }
            }
        }
        if resolved_path.is_empty() {
            if let Some(ref env_var) = pm.data_env_var {
                if let Ok(val) = std::env::var(env_var) {
                    if !val.trim().is_empty() {
                        resolved_path = val.trim().to_string();
                    }
                }
            }
        }
        if resolved_path.is_empty() {
            if let Some(ref default_path) = pm.data_default_path {
                resolved_path = expand_home(default_path);
            }
        }
    } else {
        return Err(format!("不支持的存储类型: {}", storage_kind));
    }

    if resolved_path.is_empty() {
        return Err(format!("未能检测到 {} 存储路径", storage_kind));
    }

    let cleaned = resolved_path
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_string();
    if cleaned.is_empty() {
        return Err(format!("未能检测到 {} 存储路径", storage_kind));
    }

    Ok(cleaned)
}

#[tauri::command]
pub fn get_pkg_cache_info(
    project_id: String,
    pm_id: String,
    storage_kind: String,
    extra_cache_id: Option<String>,
) -> Result<PkgCacheInfo, String> {
    let cache_path = resolve_pkg_storage_path_dynamic(
        &project_id,
        &pm_id,
        &storage_kind,
        extra_cache_id.as_deref(),
    )?;
    // 过滤 Yarn 等工具返回的 "undefined" / "null" 字符串
    let trimmed = cache_path.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("undefined")
        || trimmed.eq_ignore_ascii_case("null")
    {
        return Err("缓存路径为空".to_string());
    }
    let cache_path = trimmed.to_string();

    let path = std::path::Path::new(&cache_path);
    let (is_link, real_target) = if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            let target = std::fs::read_link(path)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            (true, target)
        } else {
            (false, String::new())
        }
    } else {
        (false, String::new())
    };

    // 如果缓存路径本身不是链接，检查父目录是否是 junction（整体链接模式）
    let parent_link = if !is_link {
        check_parent_junction(&cache_path)
    } else {
        None
    };

    let size = crate::commands::cache::get_dir_size(path);
    let size_str = crate::commands::cache::format_bytes(size);

    Ok(PkgCacheInfo {
        path: cache_path,
        size: size_str,
        is_link,
        real_target,
        parent_link,
    })
}

/// 迁移缓存/数据 — 统一的存储迁移命令。
/// - storage_kind: "cache" | "data"（cache 可先删再迁、data 必须拷贝）
/// - delete_old_first: 是否先删除旧目录再建链接（仅 cache 类型生效）
/// - orig_path: 源路径（可选，不传则用 cache_detect_cmd 自动检测）
#[tauri::command]
pub fn migrate_pkg_storage(
    app_handle: tauri::AppHandle,
    project_id: String,
    pm_id: String,
    new_path: String,
    storage_kind: String,
    delete_old_first: Option<bool>,
    orig_path: Option<String>,
    extra_cache_id: Option<String>,
) -> Result<(), String> {
    // 防注入：new_path 会替换进 set_cmd_template 后经 shell/PowerShell 执行
    crate::commands::utils::validate_subst_value(&new_path)?;
    // 后端复检：目标路径禁止 C 盘（前端已有校验，此处为信任边界兜底）
    if new_path.to_ascii_lowercase().starts_with("c:") {
        return Err(
            "目标路径必须位于非 C 盘（C: 盘迁移无法腾出空间，且系统盘写入风险高）".to_string(),
        );
    }
    let src_path = if let Some(ref op) = orig_path {
        op.clone()
    } else {
        resolve_pkg_storage_path_dynamic(
            &project_id,
            &pm_id,
            &storage_kind,
            extra_cache_id.as_deref(),
        )?
    };
    let delete_old = delete_old_first.unwrap_or(false);
    crate::commands::cache::migrate_pkg_storage_impl(
        &app_handle,
        &src_path,
        &new_path,
        &storage_kind,
        delete_old,
    )?;

    // 迁移完成后，同步更新配置中的缓存路径
    if storage_kind == "cache" {
        if let Some(def) = crate::commands::project::registry::find_by_id(&project_id) {
            if let Some(pm) = def.package_managers.iter().find(|p| p.id == pm_id) {
                if let Some(extra_id) = &extra_cache_id {
                    // 附加缓存（pnpm.cache 等）：执行其 set_cmd_template 指向新路径
                    if let Some(extra) = pm.extra_caches.iter().find(|e| &e.id == extra_id) {
                        if let Some(ref tpl) = extra.set_cmd_template {
                            // 清除进程中继承的旧值，避免它覆盖 pnpm 配置文件中的新路径。
                            let old_env = extra.env_var.as_deref().and_then(|name| {
                                std::env::var(name)
                                    .ok()
                                    .map(|value| (name.to_string(), value))
                            });
                            if let Some((name, _)) = &old_env {
                                std::env::remove_var(name);
                            }
                            let cmd = tpl.replace("{path}", &new_path);
                            if let Err(e) = run_cmd_capture(cmd, Some(project_id.clone())) {
                                if let Some((name, value)) = old_env {
                                    std::env::set_var(&name, &value);
                                }
                                return Err(format!("更新附加缓存配置失败: {}", e));
                            }
                        }
                    }
                } else {
                    // 主缓存：同步更新配置文件中的缓存路径（如 NuGet.Config）
                    crate::commands::utils::apply_cache_config_writes(pm, &new_path);
                }
            }
        }
    }

    Ok(())
}

/// 指向模式下设置缓存目录。
/// 如果包管理器配有 cache_env_var 且检测到该环境变量已被配置：
/// - 若存在设置命令（例如 go env -w）且旧环境变量在 HKCU（用户级），则主动在注册表及进程中将其清空，使命令托管生效；
pub(super) fn set_maven_local_repository(new_path: &str) -> Result<(), String> {
    use crate::commands::utils::expand_home;
    // new_path 会写入 settings.xml 的 <localRepository> 标签，须防 XML/命令注入
    crate::commands::utils::validate_subst_value(new_path)?;
    let config = crate::commands::config::load_config();
    let user_home = expand_home("{home}");
    let links_dir = config.links_dir;
    let maven_home = std::env::var("MAVEN_HOME").unwrap_or_default();

    let paths = vec![
        std::path::PathBuf::from(&user_home)
            .join(".m2")
            .join("settings.xml"),
        std::path::PathBuf::from(&maven_home)
            .join("conf")
            .join("settings.xml"),
        std::path::PathBuf::from(&links_dir)
            .join("maven")
            .join("conf")
            .join("settings.xml"),
    ];

    let mut target_path = None;
    for path in &paths {
        if path.exists() {
            target_path = Some(path.clone());
            break;
        }
    }

    // Default to {home}/.m2/settings.xml if none exists
    let settings_path = target_path.unwrap_or_else(|| {
        let p = std::path::PathBuf::from(&user_home)
            .join(".m2")
            .join("settings.xml");
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        p
    });

    if settings_path.exists() {
        let content = std::fs::read_to_string(&settings_path)
            .map_err(|e| format!("无法读取 settings.xml: {}", e))?;

        let re_repo = regex::Regex::new(r"(?s)<localRepository>.*?</localRepository>")
            .map_err(|e| e.to_string())?;

        let new_tag = format!("<localRepository>{}</localRepository>", new_path);

        let new_content = if re_repo.is_match(&content) {
            // Replace existing localRepository tag
            re_repo.replace(&content, new_tag.as_str()).to_string()
        } else {
            // Find <settings> tag and insert it after
            let re_settings =
                regex::Regex::new(r"(?i)<settings\b[^>]*>").map_err(|e| e.to_string())?;

            if let Some(mat) = re_settings.find(&content) {
                let insert_idx = mat.end();
                let mut c = content.clone();
                c.insert_str(insert_idx, &format!("\n  {}", new_tag));
                c
            } else {
                return Err("未在 settings.xml 中找到 <settings> 标签。".to_string());
            }
        };

        std::fs::write(&settings_path, new_content)
            .map_err(|e| format!("写入 settings.xml 失败: {}", e))?;
    } else {
        // Create a new settings.xml with the localRepository
        let new_content = format!(
            r#"<settings xmlns="http://maven.apache.org/SETTINGS/1.0.0"
          xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
          xsi:schemaLocation="http://maven.apache.org/SETTINGS/1.0.0 https://maven.apache.org/xsd/settings-1.0.0.xsd">
  <localRepository>{}</localRepository>
</settings>"#,
            new_path
        );
        std::fs::write(&settings_path, new_content)
            .map_err(|e| format!("创建 settings.xml 失败: {}", e))?;
    }

    Ok(())
}
