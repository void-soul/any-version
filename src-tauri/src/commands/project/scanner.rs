//! 项目扫描器 — 扫描本机项目状态的核心逻辑。
//!
//! 负责遍历注册表定义，实时检测每个项目在本机的安装状态、
//! 环境变量状态、缓存状态、服务状态等信息。

use std::fs;
use std::path::{Path, PathBuf};

use super::types::{
    ProjectDef, ProjectStatus, ProjectDetail,
    EnvVarStatus, EnvVarTier, CacheStatus, ServiceStatus,
    ManagePreview, ManageStep, ResolvePattern,
};
use super::registry;
use crate::commands::config::load_config;
use crate::commands::env::get_registry_env_any;
use crate::commands::sdk_resolver::{find_sdk_root, FindRule, ResolvePattern as ResolverPattern};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  公开接口
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 列出所有项目及其运行时状态
pub fn list_projects() -> Result<Vec<ProjectStatus>, String> {
    let defs = registry::registry();
    let config = load_config();
    let mut results = Vec::with_capacity(defs.len());

    for def in &defs {
        let status = build_project_status(def, &config)?;
        results.push(status);
    }

    Ok(results)
}

/// 获取单个项目运行时状态
pub fn get_project_status(id: &str) -> Result<ProjectStatus, String> {
    let def = registry::find_by_id(id)
        .ok_or_else(|| format!("未找到项目: {}", id))?;
    let config = load_config();
    build_project_status(&def, &config)
}

/// 获取项目详情（定义 + 状态）
pub fn get_project_detail(id: &str) -> Result<ProjectDetail, String> {
    let def = registry::find_by_id(id)
        .ok_or_else(|| format!("未找到项目: {}", id))?;
    let config = load_config();
    let status = build_project_status(&def, &config)?;

    Ok(ProjectDetail {
        def,
        status,
    })
}

/// 预览托管操作步骤
pub fn preview_manage(id: &str) -> Result<ManagePreview, String> {
    let def = registry::find_by_id(id)
        .ok_or_else(|| format!("未找到项目: {}", id))?;

    let mut steps = Vec::new();

    // 检测本地安装
    let (local_install_root, local_install_source) = detect_install_source(&def);
    let has_local = local_install_root.is_some();

    if has_local {
        steps.push(ManageStep {
            action: "found_local".to_string(),
            description: format!("检测到本地已安装版本: {} (来源: {})",
                local_install_root.as_deref().unwrap_or("未知"),
                local_install_source.as_deref().unwrap_or("未知")),
            target: local_install_root.clone().unwrap_or_default(),
        });
    }

    // 步骤 1: 备份当前环境变量
    let env_count = def.env_vars.len();
    if env_count > 0 {
        steps.push(ManageStep {
            action: "backup_env".to_string(),
            description: format!("备份 {} 个环境变量的当前值", env_count),
            target: def.env_vars.iter().map(|v| v.name.as_str()).collect::<Vec<_>>().join(", "),
        });
    }

    // 步骤 2: 清理外部 PATH 条目
    let config = load_config();
    let links_dir = Path::new(&config.links_dir);
    let link_dir = links_dir.join(&id);
    if link_dir.exists() {
        steps.push(ManageStep {
            action: "clean_path".to_string(),
            description: "清理 PATH 中的外部 SDK 条目，替换为 AnyVersion 管理路径".to_string(),
            target: id.to_string(),
        });
    }

    // 步骤 3: 设置环境变量
    for var in &def.env_vars {
        if var.tier.as_ref().map_or(false, |t| *t == EnvVarTier::Compat) {
            continue;
        }
        if var.tier.as_ref().map_or(false, |t| *t == EnvVarTier::Clear) {
            steps.push(ManageStep {
                action: "clear_env".to_string(),
                description: format!("清除注册表中的环境变量 {}（托管后由 anyversion 托管）", var.name),
                target: var.name.clone(),
            });
            continue;
        }
        let link_str = links_dir.join(&id).to_string_lossy().to_string();
        let value = if let Some(ref sub) = var.sub_dir {
            format!("{}\\{}", link_str, sub)
        } else {
            link_str.clone()
        };
        steps.push(ManageStep {
            action: "set_env".to_string(),
            description: format!("设置环境变量 {} = {}", var.name, value),
            target: var.name.clone(),
        });
    }

    // 步骤 4: 添加 PATH
    let bin_paths = get_bin_paths(&def.id, &link_dir.to_string_lossy());
    for bp in &bin_paths {
        steps.push(ManageStep {
            action: "add_path".to_string(),
            description: format!("将 {} 添加到用户 PATH", bp),
            target: bp.clone(),
        });
    }

    // 步骤 5: 缓存管理
    if def.has_cache {
        steps.push(ManageStep {
            action: "manage_cache".to_string(),
            description: "检测并管理缓存目录（可选迁移）".to_string(),
            target: id.to_string(),
        });
    }

    // 步骤 6: 镜像配置
    if def.has_mirror {
        steps.push(ManageStep {
            action: "configure_mirror".to_string(),
            description: "配置国内镜像加速".to_string(),
            target: id.to_string(),
        });
    }

    Ok(ManagePreview {
        steps,
        has_local_install: has_local,
        local_install_root,
        local_install_source,
    })
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  内部实现
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 构建单个项目的运行时状态
fn build_project_status(def: &ProjectDef, config: &crate::commands::config::Config) -> Result<ProjectStatus, String> {
    let id = &def.id;
    let versions_dir = Path::new(&config.versions_dir).join(id);
    let links_dir = Path::new(&config.links_dir);

    // 扫描已安装版本
    let installed_versions = scan_installed_versions(&versions_dir);

    // 检测激活版本（通过 junction link 解析）
    let junction_path = links_dir.join(id);
    let active_version = resolve_active_version(&junction_path);

    // 是否被 AnyVersion 托管
    let managed = config.managed_items.contains(id.as_str());

    // 安装来源检测（使用 sdk_resolver）—— 仅未托管时报告，托管后旧版信息在"旧版数据"选项卡展示
    let (install_source, install_root) = if managed {
        (None, None)
    } else {
        detect_install_source(def)
    };

    // 判断是否已安装（AnyVersion 版本目录 或 外部安装均可）
    let mut installed = !installed_versions.is_empty() || active_version.is_some() || install_root.is_some();
    let mut active_version = active_version;

    // 二次验证：未托管的项目通过 version_exe 在 PATH 中确认可执行文件真实存在
    // 防止残留的版本目录/junction 或无效的 find_rules 匹配导致误判为"已安装"
    if installed && !managed {
        if let Some(ref exe) = def.version_exe {
            let found = which_in_path(exe);
            if !found {
                installed = false;
                active_version = None;
            }
        }
    }

    // 环境变量状态
    let env_vars_status = build_env_vars_status(def, &config.links_dir, config, managed);

    // 缓存状态
    let cache_status = if def.has_cache {
        build_cache_status(def)
    } else {
        None
    };

    // 解析当前实际的安装根路径 (AnyVersion 托管链接 或 外部检测路径)
    let active_install_root = if junction_path.exists() || junction_path.is_symlink() {
        Some(junction_path.to_string_lossy().to_string())
    } else if let Some(ref root) = install_root {
        Some(root.clone())
    } else {
        None
    };

    // 数据目录状态
    let data_dirs_status = build_data_dirs_status(def, active_install_root.as_deref());

    // 服务状态
    let service_status = if def.is_service {
        build_service_status(def, &junction_path, install_root.as_deref(), &data_dirs_status)
    } else {
        None
    };

    Ok(ProjectStatus {
        id: def.id.clone(),
        display_name: def.display_name.clone(),
        category: def.category.clone(),
        installed,
        active_version,
        installed_versions,
        install_source,
        install_root,
        managed,
        env_vars_status,
        cache_status,
        service_status,
        data_dirs_status,
    })
}

/// 扫描已安装版本列表
///
/// Windows 上通过「注册本地版本」创建的条目是 junction（reparse point），
/// `is_dir()` 返回 false，必须同时检查 `is_symlink()` 才能识别。
fn scan_installed_versions(versions_dir: &Path) -> Vec<String> {
    let mut versions = Vec::new();
    if versions_dir.exists() {
        if let Ok(entries) = fs::read_dir(versions_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let name = entry.file_name().to_string_lossy().to_string();
                // 跳过隐藏目录
                if name.starts_with('.') {
                    continue;
                }
                let ft = entry.file_type();
                let is_dir_or_junction = ft.as_ref()
                    .map(|t| t.is_dir() || t.is_symlink())
                    .unwrap_or(false);
                if is_dir_or_junction {
                    versions.push(name);
                }
            }
        }
    }
    // 按版本号排序
    versions.sort();
    versions
}

/// 通过 junction link 解析当前激活版本
fn resolve_active_version(junction_path: &Path) -> Option<String> {
    if !junction_path.exists() && !junction_path.is_symlink() {
        return None;
    }

    // 尝试 canonicalize 解析 junction 目标
    if let Ok(target) = fs::canonicalize(junction_path) {
        let target_str = target.to_string_lossy().to_string()
            .trim_start_matches(r"\\?\").to_string();
        let target_path = Path::new(&target_str);
        // 取目标的最后一级目录名作为版本号
        if let Some(name) = target_path.file_name() {
            let version = name.to_string_lossy().to_string();
            if !version.is_empty() {
                return Some(version);
            }
        }
    }

    None
}

/// 检测安装来源（通过 sdk_resolver）
pub fn detect_install_source(def: &ProjectDef) -> (Option<String>, Option<String>) {
    // 转换 types::ResolvePattern -> sdk_resolver::ResolvePattern
    let resolver_rules = to_resolver_rules(&def.find_rules);

    if let Some(location) = find_sdk_root(&def.id, &resolver_rules) {
        let source = location.source.clone();
        let root = location.root.to_string_lossy().to_string();
        (Some(source), Some(root))
    } else {
        (None, None)
    }
}

/// 将 project::types::FindRule 转换为 sdk_resolver::FindRule
fn to_resolver_rules(rules: &[super::types::FindRule]) -> Vec<FindRule> {
    rules.iter().map(|r| {
        let pattern = match &r.pattern {
            ResolvePattern::PathContains { path_key, exe_name } => {
                ResolverPattern::PathContains {
                    keyword: path_key.clone(),
                    exe: exe_name.clone(),
                }
            }
            ResolvePattern::EnvBin { env_var, bin_sub, exe_name } => {
                ResolverPattern::EnvBin {
                    env: env_var.clone(),
                    bin_sub: bin_sub.clone(),
                    exe: exe_name.clone(),
                }
            }
            ResolvePattern::FixedPath { path, exe_name } => {
                ResolverPattern::FixedPath {
                    path: path.clone(),
                    exe: exe_name.clone(),
                }
            }
        };
        FindRule {
            pattern,
            source_label: r.source_label.clone(),
            priority: r.priority,
            root_offset: r.root_offset,
        }
    }).collect()
}

/// 构建环境变量状态列表
fn build_env_vars_status(
    def: &ProjectDef,
    links_dir: &str,
    config: &crate::commands::config::Config,
    managed: bool,
) -> Vec<EnvVarStatus> {
    let links_lower = links_dir.to_lowercase();
    let mut statuses = Vec::new();

    for var_def in &def.env_vars {
        // 跳过兼容层变量（NODE_PATH/NVM_HOME/VOLTA_HOME 等），
        // 它们属于其他工具的检测线索，与 AnyVersion 管理无关
        if var_def.tier.as_ref().map_or(false, |t| *t == EnvVarTier::Compat) {
            continue;
        }
        let name = &var_def.name;
        let (value, source, exists, in_anyversion) = if managed && var_def.tier.as_ref().map_or(false, |t| *t == EnvVarTier::Clear) {
            // 如果已经被托管且是 Clear 级别的变量，我们展示已清空并托管的状态，并显示备份值
            if let Some(backup_val) = config.original_envs.get(name) {
                (Some(format!("已清空并托管 (备份值: {})", backup_val)), "备份管理".to_string(), true, true)
            } else {
                (Some("已清空并托管".to_string()), "托管中".to_string(), true, true)
            }
        } else if let Some((val, src)) = get_registry_env_any(name) {
            let val_path = Path::new(&val);
            let path_exists = if var_def.check_type == "path" {
                val_path.exists()
            } else {
                true
            };
            let in_av = val.to_lowercase().contains(&links_lower);
            (Some(val), src.to_string(), path_exists, in_av)
        } else {
            (None, "未设置".to_string(), false, false)
        };

        statuses.push(EnvVarStatus {
            name: name.clone(),
            desc: var_def.desc.clone(),
            value,
            source,
            exists,
            in_anyversion,
            tier: var_def.tier.clone(),
        });
    }

    statuses
}

/// 构建缓存状态
fn build_cache_status(def: &ProjectDef) -> Option<CacheStatus> {
    use crate::commands::cache::get_dir_size;
    use crate::commands::cache::format_bytes;
    use crate::commands::utils::{expand_home, get_cmd_output, resolve_custom_cache_path};

    // Find the first package manager under this project that has cache settings configured
    let pm = def.package_managers.iter().find(|pm| pm.cache_detect_cmd.is_some() || pm.cache_default_path.is_some() || pm.cache_config_source.is_some())?;
    
    // Resolve path: try custom config resolver first, then cmd, then default_path
    let mut resolved_path = resolve_custom_cache_path(pm).unwrap_or_default();
    
    if resolved_path.is_empty() {
        if let Some(ref cmd) = pm.cache_detect_cmd {
            let parts: Vec<&str> = cmd.split_whitespace().collect();
            if !parts.is_empty() {
                let out = get_cmd_output(parts[0], &parts[1..]);
                if !out.is_empty() && out != "undefined" && out != "null" {
                    resolved_path = out;
                }
            }
        }
    }
    
    if resolved_path.is_empty() {
        if let Some(ref default_path) = pm.cache_default_path {
            resolved_path = expand_home(default_path);
        }
    }
    
    if resolved_path.is_empty() {
        return None;
    }
    
    let cache_path = PathBuf::from(&resolved_path);
    if !cache_path.exists() {
        return None;
    }

    // 检测是否为 junction/symlink
    let mut is_link = false;
    let mut real_target = String::new();
    if let Ok(metadata) = fs::symlink_metadata(&cache_path) {
        if metadata.file_type().is_symlink() {
            if let Ok(target) = fs::read_link(&cache_path) {
                is_link = true;
                real_target = target.to_string_lossy().to_string();
            }
        }
    }

    let size_path = if !real_target.is_empty() {
        PathBuf::from(&real_target)
    } else {
        cache_path.clone()
    };
    let size_bytes = get_dir_size(&size_path);
    let size_str = format_bytes(size_bytes);

    let detect_source = if pm.cache_detect_cmd.is_some() {
        format!("{} config (cmd)", pm.id)
    } else {
        format!("{} config (default)", pm.id)
    };

    Some(CacheStatus {
        path: cache_path.to_string_lossy().to_string(),
        size: size_str,
        is_link,
        real_target,
        detect_source,
    })
}

/// 构建服务状态
fn build_service_status(
    def: &ProjectDef,
    junction_path: &Path,
    local_install_root: Option<&str>,
    data_dirs_status: &[crate::commands::project::types::DataDirStatus]
) -> Option<ServiceStatus> {
    use crate::commands::service::find_port_owner_simple;

    let default_port = def.default_port.unwrap_or(0);
    let port_str = default_port.to_string();

    // 检查端口是否被占用
    let port_owner = find_port_owner_simple(&port_str);
    let running = port_owner.is_some();
    let pid = port_owner
        .and_then(|o| o.pid.parse::<u32>().ok());

    // 解析数据目录和日志目录
    let base_path = if junction_path.exists() || junction_path.is_symlink() {
        junction_path.to_path_buf()
    } else if let Some(root) = local_install_root {
        PathBuf::from(root)
    } else {
        PathBuf::new()
    };

    let mut data_dir = def.data_dir.as_deref()
        .map(|d| base_path.join(d).to_string_lossy().to_string())
        .unwrap_or_default();

    // 优先使用实际检测到的已存在的数据文件夹作为 data_dir 反馈给前端
    if data_dir.is_empty() {
        if let Some(first_dir) = data_dirs_status.iter().find(|d| d.exists) {
            data_dir = first_dir.path.clone();
        } else if let Some(first_def) = data_dirs_status.first() {
            data_dir = first_def.path.clone();
        }
    }

    let log_dir = def.log_dir.as_deref()
        .map(|d| base_path.join(d).to_string_lossy().to_string())
        .unwrap_or_default();

    Some(ServiceStatus {
        running,
        port: if default_port > 0 { Some(default_port) } else { None },
        pid,
        data_dir,
        log_dir,
    })
}

/// 获取 SDK 的可执行目录列表（用于 PATH 管理）
/// 优先使用 projects.json 中由 Scoop 更新或手动定义的 bin_dirs 字段
pub fn get_bin_paths(sdk_id: &str, link_dir: &str) -> Vec<String> {
    // ── 优先从 ProjectDef.bin_dirs 读取 ──
    if let Some(def) = registry::find_by_id(sdk_id) {
        if let Some(ref bin_dirs) = def.bin_dirs {
            if !bin_dirs.is_empty() {
                return bin_dirs.iter()
                    .map(|d| if d.is_empty() { link_dir.to_string() } else { format!("{}\\{}", link_dir, d) })
                    .collect();
            }
        }
    }

    // Generic fallback if bin_dirs is not defined
    let bin_path = format!("{}\\bin", link_dir);
    if std::path::Path::new(&bin_path).exists() {
        vec![bin_path]
    } else {
        vec![link_dir.to_string()]
    }
}

/// 在 PATH 中搜索可执行文件（Windows 兼容 .exe/.cmd/.bat）
fn which_in_path(name: &str) -> bool {
    let mut check_names = vec![name.to_string()];
    #[cfg(windows)]
    {
        let name_lower = name.to_lowercase();
        if !name_lower.ends_with(".exe") && !name_lower.ends_with(".cmd") && !name_lower.ends_with(".bat") {
            check_names.push(format!("{}.exe", name));
            check_names.push(format!("{}.cmd", name));
            check_names.push(format!("{}.bat", name));
        }
    }

    if let Ok(paths) = std::env::var("PATH") {
        for dir in std::env::split_paths(&paths) {
            for check_name in &check_names {
                let full = dir.join(check_name);
                if full.exists() {
                    return true;
                }
            }
        }
    }
    false
}

/// 扫描数据目录状态
fn build_data_dirs_status(def: &ProjectDef, active_install_root: Option<&str>) -> Vec<crate::commands::project::types::DataDirStatus> {
    use crate::commands::cache::get_dir_size;
    use crate::commands::cache::format_bytes;
    use crate::commands::utils::expand_home;
    use crate::commands::project::types::DataDirStatus;

    let mut statuses = Vec::new();

    for dir_def in &def.data_dirs {
        let mut paths_to_check = Vec::new();

        // 1. 优先检查环境变量路径
        if let Some(ref env_var) = dir_def.env_var {
            if let Some(env_val) = crate::commands::env::get_registry_env(env_var) {
                if !env_val.is_empty() {
                    paths_to_check.push(env_val);
                }
            }
        }

        // 2. 添加 possible_paths 并做变量替换与拓展
        for p in &dir_def.possible_paths {
            let mut resolved = expand_home(p);
            if let Some(root) = active_install_root {
                resolved = resolved.replace("{install_root}", root);
            }
            if !paths_to_check.contains(&resolved) {
                paths_to_check.push(resolved);
            }
        }

        // 3. 补充 default_path 确保有备选
        let mut default_resolved = expand_home(&dir_def.default_path);
        if let Some(root) = active_install_root {
            default_resolved = default_resolved.replace("{install_root}", root);
        }
        if !paths_to_check.contains(&default_resolved) {
            paths_to_check.push(default_resolved.clone());
        }

        let mut found_any = false;
        for path_str in paths_to_check {
            let path = Path::new(&path_str);
            if path.exists() {
                found_any = true;
                let mut is_link = false;
                let mut real_target = String::new();

                if let Ok(metadata) = fs::symlink_metadata(path) {
                    if metadata.file_type().is_symlink() || metadata.file_type().is_dir() {
                        if let Ok(eval_path) = fs::read_link(path) {
                            is_link = true;
                            real_target = eval_path.to_string_lossy().to_string();
                        } else if let Ok(eval_path) = fs::canonicalize(path) {
                            let canonical = eval_path.to_string_lossy().to_string();
                            let canonical_clean = canonical.trim_start_matches(r"\\?\").to_string();
                            if canonical_clean != path.to_string_lossy().to_string() {
                                is_link = true;
                                real_target = canonical_clean;
                            }
                        }
                    }
                }

                let size_path = if is_link && !real_target.is_empty() { Path::new(&real_target) } else { path };
                let size_bytes = get_dir_size(size_path);
                let size_str = format_bytes(size_bytes);

                statuses.push(DataDirStatus {
                    id: dir_def.id.clone(),
                    display_name: dir_def.display_name.clone(),
                    path: path_str,
                    size: size_str,
                    is_link,
                    real_target,
                    exists: true,
                });
            }
        }

        // 若全部不存在，添加默认路径的占位，标记为不存在
        if !found_any {
            statuses.push(DataDirStatus {
                id: dir_def.id.clone(),
                display_name: dir_def.display_name.clone(),
                path: default_resolved,
                size: "0 B".to_string(),
                is_link: false,
                real_target: String::new(),
                exists: false,
            });
        }
    }

    statuses
}
