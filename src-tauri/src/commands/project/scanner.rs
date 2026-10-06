//! 项目扫描器 — 扫描本机项目状态的核心逻辑。
//!
//! 负责遍历注册表定义，实时检测每个项目在本机的安装状态、
//! 环境变量状态、缓存状态、服务状态等信息。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use super::types::{
    ProjectDef, ProjectStatus, ProjectDetail,
    EnvVarStatus, EnvVarTier, CacheStatus, ServiceStatus,
    ManagePreview, ManageStep,
};
use super::registry;
use crate::commands::config::load_config;
use crate::commands::env::get_registry_env_any;
use crate::commands::sdk_resolver::find_sdk_root;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  公开接口
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 列出所有项目及其运行时状态
pub fn list_projects() -> Result<Vec<ProjectStatus>, String> {
    let defs = registry::registry();
    let config = load_config();
    let mut results = Vec::with_capacity(defs.len());

    for def in &defs {
        let status = build_project_status(def, &config, false, false)?;
        results.push(status);
    }

    Ok(results)
}

/// 快速列出项目状态（跳过缓存大小计算，用于前端列表初次加载）
pub fn list_projects_fast() -> Result<Vec<ProjectStatus>, String> {
    let defs = registry::registry();
    let config = load_config();
    let mut results = Vec::with_capacity(defs.len());

    for def in &defs {
        let status = build_project_status(def, &config, true, false)?;
        results.push(status);
    }

    Ok(results)
}

/// 获取单个项目运行时状态
pub fn get_project_status(id: &str) -> Result<ProjectStatus, String> {
    let def = registry::find_by_id(id)
        .ok_or_else(|| format!("未找到项目: {}", id))?;
    let config = load_config();
    build_project_status(&def, &config, false, true)
}

/// 获取项目详情（定义 + 状态）
pub fn get_project_detail(id: &str) -> Result<ProjectDetail, String> {
    let def = registry::find_by_id(id)
        .ok_or_else(|| format!("未找到项目: {}", id))?;
    let config = load_config();
    let status = build_project_status(&def, &config, false, true)?;

    Ok(ProjectDetail {
        def,
        status,
    })
}

/// 预览托管操作步骤
pub fn preview_manage(id: &str, delegation: crate::commands::config::ProjectDelegation) -> Result<ManagePreview, String> {
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

    // 1. 备份环境变量
    let backup_vars: Vec<&str> = def.env_vars.iter()
        .filter(|v| v.tier.as_ref().map_or(true, |t| *t != EnvVarTier::Compat))
        .filter(|v| delegation.env_vars.contains(&v.name))
        .map(|v| v.name.as_str())
        .collect();

    if !backup_vars.is_empty() {
        steps.push(ManageStep {
            action: "backup_env".to_string(),
            description: format!("备份 {} 个环境变量的当前值", backup_vars.len()),
            target: backup_vars.join(", "),
        });
    }

    // 2. 清理外部 PATH 条目
    let config = load_config();
    let links_dir = Path::new(&config.links_dir);
    let link_dir = links_dir.join(&id);
    
    // MSIX 型（如 WinGet）不参与 PATH：它是系统级安装，没有 link 目录（不做 junction），
    // 而它的 exe 在 WindowsApps（应用执行别名）—— 写/清 PATH 都是错的。
    let has_path_delegated = if def.is_msix() {
        false
    } else if let Some(ref dirs) = def.bin_dirs {
        dirs.iter().any(|d| delegation.path_vars.contains(d))
    } else {
        false
    };

    if has_path_delegated {
        steps.push(ManageStep {
            action: "clean_path".to_string(),
            description: "清理 PATH 中的外部 SDK 条目，替换为 Kira 管理路径".to_string(),
            target: id.to_string(),
        });
    }

    // 3. 设置环境变量
    for var in &def.env_vars {
        if var.tier.as_ref().map_or(false, |t| *t == EnvVarTier::Compat) {
            continue;
        }
        if !delegation.env_vars.contains(&var.name) {
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
        let value = crate::commands::env::sdk_env_var_value(&id, &link_str, var);
        steps.push(ManageStep {
            action: "set_env".to_string(),
            description: format!("设置环境变量 {} = {}", var.name, value),
            target: var.name.clone(),
        });
    }

    // 4. 添加 PATH（MSIX 型跳过，理由同第 2 步）
    if !def.is_msix() {
        if let Some(ref dirs) = def.bin_dirs {
            for bin_dir in dirs {
                if delegation.path_vars.contains(bin_dir) {
                    let link_bin_path = if bin_dir.is_empty() {
                        link_dir.to_string_lossy().to_string()
                    } else {
                        format!("{}\\{}", link_dir.to_string_lossy(), bin_dir)
                    };
                    steps.push(ManageStep {
                        action: "add_path".to_string(),
                        description: format!("将 {} 添加到用户 PATH", link_bin_path),
                        target: link_bin_path,
                    });
                }
            }
        }
    }

    // 5. 目录联接
    if delegation.create_symlink {
        steps.push(ManageStep {
            action: "create_junction".to_string(),
            description: "创建稳定目录联接/映射软链接以接管版本切换".to_string(),
            target: link_dir.to_string_lossy().to_string(),
        });
    }

    // 6. 缓存管理
    if def.has_cache {
        steps.push(ManageStep {
            action: "manage_cache".to_string(),
            description: "检测并管理缓存目录（可选迁移）".to_string(),
            target: id.to_string(),
        });
    }

    // 7. 镜像配置
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

pub fn get_project_delegation(config: &crate::commands::config::Config, id: &str, def: &ProjectDef) -> crate::commands::config::ProjectDelegation {
    if let Some(del) = config.project_delegations.get(id) {
        return del.clone();
    }
    
    // Migration fallback
    if config.managed_items.contains(id) {
        let is_simple = config.simple_managed_items.contains(id) || def.simple_mode;
        if is_simple {
            crate::commands::config::ProjectDelegation {
                env_vars: std::collections::HashSet::new(),
                path_vars: std::collections::HashSet::new(),
                version_control: false,
                create_symlink: false,
                manage_install_dir: true,
                manage_data_dir: true,
                manage_cache_dir: true,
                manage_optional_tools: std::collections::HashSet::new(),
            }
        } else {
            let mut envs = std::collections::HashSet::new();
            for var in &def.env_vars {
                if let Some(ref tier) = var.tier {
                    if *tier == super::types::EnvVarTier::Compat { continue; }
                }
                envs.insert(var.name.clone());
            }
            let mut paths = std::collections::HashSet::new();
            if let Some(ref dirs) = def.bin_dirs {
                for p in dirs {
                    paths.insert(p.clone());
                }
            }
            let mut optional_tools = std::collections::HashSet::new();
            for pm in &def.package_managers {
                if !pm.built_in {
                    optional_tools.insert(pm.id.clone());
                }
            }
            crate::commands::config::ProjectDelegation {
                env_vars: envs,
                path_vars: paths,
                version_control: true,
                create_symlink: true,
                manage_install_dir: true,
                manage_data_dir: true,
                manage_cache_dir: true,
                manage_optional_tools: optional_tools,
            }
        }
    } else {
        crate::commands::config::ProjectDelegation::default()
    }
}

/// 构建单个项目的运行时状态
fn build_project_status(def: &ProjectDef, config: &crate::commands::config::Config, skip_cache: bool, force_service_refresh: bool) -> Result<ProjectStatus, String> {
    let id = &def.id;
    let versions_dir = Path::new(&config.versions_dir).join(id);
    let links_dir = Path::new(&config.links_dir);

    // 扫描已安装版本（MSIX 型项目后面会整体覆盖：它不落在 versions_dir）
    let mut installed_versions = scan_installed_versions(&versions_dir);

    // 检测激活版本（通过 junction link 解析，或从配置中的 active_versions 中获取作为 fallback）
    let junction_path = links_dir.join(id);
    let active_version = resolve_active_version(&junction_path)
        .or_else(|| config.active_versions.get(id).cloned());

    let delegation = get_project_delegation(config, id, def);

    // 是否被 AnyVersion 托管
    let managed = config.managed_items.contains(id.as_str());
    let is_simple_managed = !delegation.version_control;

    // 安装来源检测（使用 sdk_resolver）—— 仅在未托管或简单托管时报告，完全托管后在"旧版数据"选项卡展示
    let (install_source, install_root) = if managed && !is_simple_managed {
        (None, None)
    } else {
        detect_install_source(def)
    };

    // 判断是否已安装（AnyVersion 版本目录 或 外部安装均可）
    let mut installed = !installed_versions.is_empty() || active_version.is_some() || install_root.is_some();
    let mut active_version = active_version;

    // 如果未托管或处于简单托管模式，且尚未解析出激活版本，尝试从本地安装路径中检测版本号作为激活版本
    if active_version.is_none() && (!managed || is_simple_managed) {
        if let Some(ref root) = install_root {
            if let Some(ver) = super::versions::detect_version_from_path(id, Path::new(root)) {
                active_version = Some(ver);
            }
        }
    }

    // MSIX 安装方式（如 winget）：包是**系统级注册**的，同名包只存在一份 ——
    // 它既不会出现在 versions_dir（`scan_installed_versions` 恒为空，装完也显示不出
    // 「已安装版本」），命令行入口又是「应用执行别名」
    // （`%LOCALAPPDATA%\Microsoft\WindowsApps\winget.exe`，用户可以在系统设置里关掉，
    // 关掉时 `winget` 就不在 PATH 里）。所以既不能靠版本目录、也不能靠 PATH 判断，
    // 直接问系统注册了没有：查到的版本就是唯一那个版本。
    if def.install_mode.as_deref() == Some("msix") {
        let pkg = def.msix_package_name.as_deref().unwrap_or(id.as_str());
        match msix_package_version(pkg) {
            Ok(Some(v)) => {
                installed = true;
                active_version = Some(v.clone());
                installed_versions = vec![v];
            }
            Ok(None) => {
                installed = false;
                active_version = None;
                installed_versions.clear();
            }
            Err(e) => {
                // 查询本身失败（PowerShell 被限制等）：保持 Kira 记下的版本，别把装好的显示成未装
                eprintln!("[scanner] {} MSIX 包查询失败: {}", id, e);
            }
        }
    } else if installed && !managed && install_source.as_deref() != Some("手动指定") {
        // 二次验证：未托管且不是手动指定路径的项目，通过 version_exe 在 PATH 中确认可执行文件真实存在
        // 防止残留的版本目录/junction 或无效 Graves 规则匹配导致误判为"已安装"
        if let Some(ref exe) = def.version_exe {
            let found = which_in_path(exe);
            if !found {
                installed = false;
                active_version = None;
            }
        }
    }

    // 环境变量状态
    let env_vars_status = build_env_vars_status(def, &config.links_dir, config, managed && !is_simple_managed);

    // Kira 写入用户 PATH 的条目 —— 只要 Kira 设置过（且还留在用户 PATH 里）就展示：
    // - 完全托管：展示全部候选条目（含"未在 PATH/目录缺失"的告警，帮助发现失效）；
    // - 未托管/简单托管/已取消托管：只展示仍留在用户 PATH 里的条目（例如取消托管
    //   清理失败留下的残留），从未托管过的 SDK 其 links_dir 路径不会出现在 PATH，
    //   过滤后为空数组，不会产生噪音。
    let fully_managed = managed && !is_simple_managed;
    let managed_path_entries = {
        let link_dir = format!("{}\\{}", config.links_dir, id);
        let entries = crate::commands::env::managed_path_entries_for_sdk(id, &link_dir, def);
        if fully_managed {
            entries
        } else {
            entries.into_iter().filter(|e| e.in_path).collect()
        }
    };

    // 缓存状态（主列表加载时跳过缓存大小计算以提升性能）
    let cache_status = if def.has_cache && !skip_cache {
        build_cache_status(def)
    } else {
        None
    };

    // 解析当前实际的安装根路径 (AnyVersion 托管链接，或已激活的下载版本目录，或 外部检测路径)
    let active_install_root = if junction_path.exists() || junction_path.is_symlink() {
        Some(junction_path.to_string_lossy().to_string())
    } else if let Some(ref ver) = active_version {
        let ver_path = Path::new(&config.versions_dir).join(id).join(ver);
        if ver_path.exists() {
            Some(ver_path.to_string_lossy().to_string())
        } else if let Some(ref root) = install_root {
            Some(root.clone())
        } else {
            None
        }
    } else if let Some(ref root) = install_root {
        Some(root.clone())
    } else {
        None
    };

    // 数据目录状态
    let data_dirs_status = build_data_dirs_status(def, active_install_root.as_deref());

    // 服务状态：单个 SDK 详情/状态请求先同步失效并检测一次，确保外部进程也能立即反映。
    let service_status = if def.is_service || def.category == super::types::ProjectCategory::Service {
        if force_service_refresh {
            crate::commands::service::refresh_service_status_for_id(&def.id)
        } else {
            build_service_status(def)
        }
    } else {
        None
    };

    // 右键菜单配置
    let menu_config = config.project_menu_configs.get(id);
    let show_version = menu_config.map_or(true, |c| c.show_version);
    let show_service = menu_config.map_or(true, |c| c.show_service);

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
        is_simple_managed,
        env_vars_status,
        managed_path_entries,
        cache_status,
        service_status,
        data_dirs_status,
        show_version,
        show_service,
        delegation,
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
    let config = load_config();
    if let Some(custom_path) = config.custom_install_paths.get(&def.id) {
        return (Some("手动指定".to_string()), Some(custom_path.clone()));
    }

    // 规则类型与 sdk_resolver 共用同一份（project::types::{FindRule, ResolvePattern}），
    // 直接借用，无需再做字段搬运。
    if let Some(location) = find_sdk_root(&def.id, &def.find_rules) {
        let source = location.source.clone();
        let root = location.root.to_string_lossy().to_string();
        (Some(source), Some(root))
    } else {
        (None, None)
    }
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
            let backup_val = config.original_envs.get(&def.id).and_then(|m| m.get(name));
            if let Some(backup_val) = backup_val {
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
    use crate::commands::utils::{expand_home, resolve_custom_cache_path, resolve_detected_path, run_simple_command_checked};

    // Find the first package manager under this project that has cache settings configured
    let pm = def.package_managers.iter().find(|pm| pm.cache_detect_cmd.is_some() || pm.cache_default_path.is_some() || pm.cache_config_source.is_some())?;
    
    // Resolve path: try custom config resolver first, then cmd, then default_path
    let mut resolved_path = resolve_custom_cache_path(pm).unwrap_or_default();
    
    if resolved_path.is_empty() {
        if let Some(ref cmd) = pm.cache_detect_cmd {
            let parts: Vec<&str> = cmd.split_whitespace().collect();
            if !parts.is_empty() {
                let output = if cmd.starts_with("pnpm config get") {
                    run_simple_command_checked(cmd)
                } else {
                    super::commands::run_cmd_capture(cmd.clone(), Some(def.id.clone()))
                };
                if let Ok(out) = output {
                    if let Some(path) = resolve_detected_path(&out, pm.cache_detect_json_path.as_deref()) {
                        resolved_path = path;
                    }
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

/// 构建服务状态。
/// 优先读取 service 模块的后台快照（3s TTL），避免每次前端轮询都同步执行
/// tasklist/wmic/netstat（这些命令长时运行后可能变慢阻塞事件循环）。
/// 快照缺失（首轮/缓存过期后台刷新中）时回退为同步检测，保证返回真实状态。
fn build_service_status(def: &ProjectDef) -> Option<ServiceStatus> {
    let mut snapshot = crate::commands::service::service_status_snapshot(&[def.id.clone()]);
    snapshot
        .remove(&def.id)
        .or_else(|| Some(crate::commands::service::service_status_for_def(def)))
}

/// 获取 SDK 的可执行目录列表（用于 PATH 管理）
/// 优先使用 projects/<id>/config.json 中由 Scoop 更新或手动定义的 bin_dirs 字段
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

/// MSIX 版本 + 安装目录查询结果缓存：起一次 PowerShell 进程约 1~2 秒，而 SDK 列表每 4 秒
/// 刷新一次全部项目状态，不能每次都查。安装 / 卸载成功后由 `invalidate_msix_version_cache`
/// 立即失效，避免装完还显示旧的「未安装」。
const MSIX_VERSION_TTL: std::time::Duration = std::time::Duration::from_secs(15);

/// 一次查回来的两项信息：`(版本, InstallLocation)`。
///
/// 为什么连安装目录一起查：Store / MSIX 版应用（ChatGPT、Claude 桌面端的新版）**没有**
/// 常规 exe 安装路径，`paths.json` 里也没法声明（`C:\Program Files\WindowsApps\<名>_<版本>_<架构>__<发布者>`
/// 里带版本号，装一次就变）。界面上的「检测到的路径」只能从这里取，否则 Store 应用永远显示不出
/// 装在哪 —— 用户看到的现象就是「路径是空的 / 说不清装哪了」。
type MsixInfo = (Option<String>, Option<String>);

fn msix_version_cache() -> &'static std::sync::Mutex<HashMap<String, (Instant, MsixInfo)>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<HashMap<String, (Instant, MsixInfo)>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

/// 让某个 MSIX 包的版本缓存立即失效（安装 / 卸载完成后调用）。
pub(crate) fn invalidate_msix_version_cache(package_name: &str) {
    if let Ok(mut cache) = msix_version_cache().lock() {
        cache.remove(package_name);
    }
    // 族名缓存的 key 是「候选列表用 | 拼接」，装 / 卸之后同样要失效，
    // 否则启动仍会用旧的 AUMID（刚装完的 Store 应用照样拉不起来）。
    if let Ok(mut cache) = msix_family_cache().lock() {
        cache.retain(|key, _| !key.split('|').any(|n| n == package_name));
    }
}

/// 查询系统里已注册的 MSIX 包信息：`Ok((None, _))` = 确实没装，`Err` = 查询失败（无法判定）。
///
/// AI 工具模块也用它判定 Store 应用（ChatGPT / Claude 桌面端）有没有装上 ——
/// 这类应用没有普通 exe 安装路径，只能问系统包注册。
pub(crate) fn msix_package_info(package_name: &str) -> Result<MsixInfo, String> {
    {
        let cache = msix_version_cache().lock().map_err(|e| format!("MSIX 缓存锁失败: {}", e))?;
        if let Some((at, value)) = cache.get(package_name) {
            if at.elapsed() < MSIX_VERSION_TTL {
                return Ok(value.clone());
            }
        }
    }

    let script = format!(
        "(Get-AppxPackage -Name '{}' | Sort-Object Version -Descending | Select-Object -First 1 \
         | ForEach-Object {{ \"$($_.Version)|$($_.InstallLocation)\" }})",
        package_name.replace('\'', "''")
    );
    let out = crate::commands::hidden_cmd::hidden_cmd("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .map_err(|e| format!("查询 MSIX 包 {} 失败: {}", package_name, e))?;
    let value = parse_msix_info(&String::from_utf8_lossy(&out.stdout));

    if let Ok(mut cache) = msix_version_cache().lock() {
        cache.insert(package_name.to_string(), (Instant::now(), value.clone()));
    }
    Ok(value)
}

/// 查询系统里已注册的 MSIX 包版本。`Ok(None)` = 确实没装，`Err` = 查询失败（无法判定）。
pub(crate) fn msix_package_version(package_name: &str) -> Result<Option<String>, String> {
    msix_package_info(package_name).map(|(version, _)| version)
}

/// 查询 MSIX 包的 `InstallLocation`（`C:\Program Files\WindowsApps\<…>`）。
///
/// 给 AI 工具模块填「检测到的路径」用：Store 应用只有包注册，没有可声明的 exe 路径。
pub(crate) fn msix_package_install_location(package_name: &str) -> Result<Option<String>, String> {
    msix_package_info(package_name).map(|(_, location)| location)
}

/// PackageFamilyName 查询结果缓存（与版本缓存同理：一次 PowerShell 约 1~2 秒）。
fn msix_family_cache() -> &'static std::sync::Mutex<HashMap<String, (Instant, Option<String>)>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<HashMap<String, (Instant, Option<String>)>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

/// 按候选顺序查出系统里已注册 MSIX 包的 **PackageFamilyName**（`<identity>_<publisherhash>`）。
///
/// 为什么需要它：`paths.json` 里硬编码的 `shell:AppsFolder\<PFN>!App` 把 publisher hash
/// 写死了 —— 装的是 beta 渠道（如 `OpenAI.CodexBeta`）或其它 publisher 的版本时，这个
/// AUMID 根本不存在，点启动要么没反应、要么弹出一个文件夹窗口。
/// `Get-AppxPackage` 是 Windows 自己的权威来源，与 publisher hash 无关。
///
/// 候选按优先级给出（stable 优先、beta 兜底），**一次** PowerShell 调用搞定全部候选。
/// 抄 EchoBird `codex_proxy/codex_binary.rs::find_codex_store_family_via_appx`。
pub(crate) fn msix_package_family_name(
    package_names: &[&str],
) -> Result<Option<String>, String> {
    let key = package_names.join("|");
    if key.is_empty() {
        return Ok(None);
    }
    {
        let cache = msix_family_cache()
            .lock()
            .map_err(|e| format!("MSIX 缓存锁失败: {}", e))?;
        if let Some((at, value)) = cache.get(&key) {
            if at.elapsed() < MSIX_VERSION_TTL {
                return Ok(value.clone());
            }
        }
    }

    let script = build_family_name_script(package_names);
    let out = crate::commands::hidden_cmd::hidden_cmd("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .map_err(|e| format!("查询 MSIX 包族名失败: {}", e))?;
    let value = parse_msix_family_name(&String::from_utf8_lossy(&out.stdout));

    if let Ok(mut cache) = msix_family_cache().lock() {
        cache.insert(key, (Instant::now(), value.clone()));
    }
    Ok(value)
}

/// 拼一段「按候选顺序取 PackageFamilyName」的 PowerShell。
///
/// 与 publisher hash 无关，所以候选里可以同时放 stable 与 beta；
/// 同名的多个安装按版本倒序取最新。
fn build_family_name_script(package_names: &[&str]) -> String {
    let mut script = String::new();
    for (i, name) in package_names.iter().enumerate() {
        let escaped = name.replace('\'', "''");
        let keyword = if i == 0 { "if" } else { "elseif" };
        script.push_str(&format!(
            "{keyword}($p=Get-AppxPackage -Name '{escaped}' -ErrorAction SilentlyContinue \
             | Sort-Object Version -Descending | Select-Object -First 1){{$p.PackageFamilyName}}"
        ));
    }
    script
}

/// 从 `Get-AppxPackage …PackageFamilyName` 的输出里取族名。
///
/// 必须校验 `<identity>_<hash>` 形态：PowerShell 报错或输出别的东西时，
/// 不能让它变成一个假的 AUMID（那会比「找不到」更难排查）。
fn parse_msix_family_name(stdout: &str) -> Option<String> {
    let pfn = stdout
        .lines()
        .map(|l| l.trim().trim_matches('\u{feff}'))
        .find(|l| !l.is_empty())?;
    if pfn.contains('_') {
        Some(pfn.to_string())
    } else {
        None
    }
}

/// 从 `Get-AppxPackage … | ForEach-Object { "$($_.Version)|$($_.InstallLocation)" }` 的输出里
/// 取 `(版本, 安装目录)`（可能带 BOM / 空行）。
///
/// 没装时输出为空 → `(None, None)`；版本与路径之间用 `|` 分隔，安装目录为空（异常情况）
/// 也不能把空串当成路径 —— 那会让界面显示一个空的「检测到的路径」。
fn parse_msix_info(stdout: &str) -> MsixInfo {
    let Some(line) = stdout
        .lines()
        .map(|l| l.trim().trim_matches('\u{feff}'))
        .find(|l| !l.is_empty())
    else {
        return (None, None);
    };
    let (version, location) = match line.split_once('|') {
        Some((v, l)) => (v.trim(), l.trim()),
        // 没有分隔符（脚本被截断 / PowerShell 报错）→ 只当版本，路径不认
        None => (line, ""),
    };
    let version = (!version.is_empty()).then(|| version.to_string());
    let location = (!location.is_empty()).then(|| location.to_string());
    (version, location)
}

/// 只取版本（历史调用点与测试都按这个语义）。
fn parse_msix_version(stdout: &str) -> Option<String> {
    parse_msix_info(stdout).0
}

#[cfg(test)]
mod tests {
    use super::{build_family_name_script, parse_msix_family_name, parse_msix_info, parse_msix_version};

    /// `<版本>|<InstallLocation>` 一次拿全：版本与安装目录都要，BOM / CRLF / 空行都要扛住。
    ///
    /// 安装目录是界面「检测到的路径」的唯一来源（Store 应用没法在 `paths.json` 里声明
    /// WindowsApps 路径），所以不能因为分隔符位置不理想就整条丢掉。
    #[test]
    fn msix_info_parses_version_and_install_location() {
        assert_eq!(
            parse_msix_info("2.19675.0.0|C:\\Program Files\\WindowsApps\\Claude_2.19675.0.0_x64__pzs8sxrjxfjjc\r\n"),
            (
                Some("2.19675.0.0".to_string()),
                Some("C:\\Program Files\\WindowsApps\\Claude_2.19675.0.0_x64__pzs8sxrjxfjjc".to_string())
            )
        );
        assert_eq!(
            parse_msix_info("\u{feff}1.29.380.0|C:\\Program Files\\WindowsApps\\Codex\r\n"),
            (
                Some("1.29.380.0".to_string()),
                Some("C:\\Program Files\\WindowsApps\\Codex".to_string())
            ),
            "BOM 要剥掉"
        );
        // 没装 → 两项都没有（绝不能把空串当成路径显示出去）
        assert_eq!(parse_msix_info("\r\n\r\n"), (None, None));
        assert_eq!(parse_msix_info(""), (None, None));
        // 路径为空（异常输出）→ 只认版本
        assert_eq!(parse_msix_info("1.2.3.4|\r\n"), (Some("1.2.3.4".to_string()), None));
        // 没有 `|`（脚本被截断）→ 版本仍可用，路径不猜
        assert_eq!(parse_msix_info("1.2.3.4"), (Some("1.2.3.4".to_string()), None));
        // WindowsApps 路径里本来就有 `|` 之外的字符，用 split_once 只切第一个分隔符
        assert_eq!(
            parse_msix_info("1.0.0.0|D:\\a\\b|c\r\n").1.as_deref(),
            Some("D:\\a\\b|c")
        );
    }

    /// PowerShell 的输出形态：CRLF、前导空行、可能有 BOM；没装时只有空行。
    #[test]
    fn msix_version_parses_first_non_empty_line() {
        assert_eq!(parse_msix_version("1.29.380.0\r\n").as_deref(), Some("1.29.380.0"));
        assert_eq!(parse_msix_version("\r\n\r\n").as_deref(), None, "没装时输出为空");
        assert_eq!(parse_msix_version("").as_deref(), None);
        assert_eq!(parse_msix_version("\u{feff}1.29.380.0\r\n").as_deref(), Some("1.29.380.0"), "BOM 要剥掉");
        // 多个包命中时（同名多版本 / 多用户）只取第一行
        assert_eq!(parse_msix_version("2.0.0.0\r\n1.29.380.0\r\n").as_deref(), Some("2.0.0.0"));
    }

    /// 族名必须是 `<identity>_<hash>` 形态：PowerShell 报错或输出别的东西时不能
    /// 让它变成一个假 AUMID（那比「找不到」更难排查）。
    #[test]
    fn msix_family_name_requires_identity_hash_shape() {
        assert_eq!(parse_msix_family_name("OpenAI.Codex_2p2nqsd0c76g0\r\n").as_deref(), Some("OpenAI.Codex_2p2nqsd0c76g0"));
        assert_eq!(parse_msix_family_name("\u{feff}OpenAI.Codex_abc\r\n").as_deref(), Some("OpenAI.Codex_abc"), "BOM 要剥掉");
        assert_eq!(parse_msix_family_name("\r\n\r\n").as_deref(), None, "没装时输出为空");
        // 报错信息 / 命令回显不是族名
        assert_eq!(parse_msix_family_name("Get-AppxPackage : 找不到").as_deref(), None);
    }

    /// 一次 PowerShell 调用覆盖全部候选：首个用 `if`，其余 `elseif`（stable 优先）。
    #[test]
    fn family_name_script_chains_candidates_in_one_call() {
        let script = build_family_name_script(&["OpenAI.Codex", "OpenAI.CodexBeta"]);
        assert!(script.starts_with("if($p=Get-AppxPackage -Name 'OpenAI.Codex'"), "首个候选必须是 if");
        assert!(script.contains("elseif($p=Get-AppxPackage -Name 'OpenAI.CodexBeta'"));
        assert_eq!(script.matches("Get-AppxPackage").count(), 2, "两个候选一次查完");
        // 单引号里的单引号要转义，否则 PowerShell 语法错、整条脚本静默失效
        assert!(build_family_name_script(&["O'Brien.App"]).contains("O''Brien.App"));
    }
}

/// 扫描数据目录状态
fn build_data_dirs_status(def: &ProjectDef, active_install_root: Option<&str>) -> Vec<crate::commands::project::types::DataDirStatus> {
    use crate::commands::cache::{format_bytes, get_dir_size};
    use crate::commands::project::types::DataDirStatus;

    let config = load_config();
    let install_root = active_install_root.map(PathBuf::from);
    let mut statuses = Vec::new();

    for dir_def in &def.data_dirs {
        let resolved = crate::commands::service::resolve_data_dir(
            def,
            dir_def,
            &config,
            install_root.as_deref(),
        );
        let path = Path::new(&resolved.path);
        let exists = path.exists();
        let mut is_link = false;
        let mut real_target = String::new();

        if exists {
            if let Ok(metadata) = fs::symlink_metadata(path) {
                if metadata.file_type().is_symlink() {
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
        }

        let size_path = if exists && is_link && !real_target.is_empty() {
            Path::new(&real_target)
        } else {
            path
        };
        let size = if exists { format_bytes(get_dir_size(size_path)) } else { "0 B".to_string() };

        statuses.push(DataDirStatus {
            id: resolved.id,
            display_name: resolved.display_name,
            path: resolved.path,
            size,
            is_link,
            real_target,
            exists,
            kind: resolved.kind,
            source: Some(resolved.source),
        });
    }

    statuses
}
