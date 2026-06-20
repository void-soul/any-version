//! 项目扫描器 — 扫描本机项目状态的核心逻辑。
//!
//! 负责遍历注册表定义，实时检测每个项目在本机的安装状态、
//! 环境变量状态、缓存状态、服务状态等信息。

use std::fs;
use std::path::{Path, PathBuf};

use super::types::{
    ProjectDef, ProjectStatus, ProjectDetail,
    EnvVarStatus, CacheStatus, ServiceStatus,
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
        let link_str = links_dir.join(&id).to_string_lossy().to_string();
        steps.push(ManageStep {
            action: "set_env".to_string(),
            description: format!("设置环境变量 {} = {}", var.name, link_str),
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

    Ok(ManagePreview { steps })
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

    // 安装来源检测（使用 sdk_resolver）
    let (install_source, install_root) = detect_install_source(def);

    // 判断是否已安装（AnyVersion 版本目录 或 外部安装均可）
    let installed = !installed_versions.is_empty() || active_version.is_some() || install_root.is_some();

    // 是否被 AnyVersion 托管
    let managed = config.managed_items.contains(id.as_str());

    // 环境变量状态
    let env_vars_status = build_env_vars_status(def, &config.links_dir);

    // 缓存状态
    let cache_status = if def.has_cache {
        build_cache_status(def)
    } else {
        None
    };

    // 服务状态
    let service_status = if def.is_service {
        build_service_status(def, &junction_path)
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
    })
}

/// 扫描已安装版本列表
fn scan_installed_versions(versions_dir: &Path) -> Vec<String> {
    let mut versions = Vec::new();
    if versions_dir.exists() {
        if let Ok(entries) = fs::read_dir(versions_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    let name = entry.file_name().to_string_lossy().to_string();
                    // 跳过隐藏目录
                    if !name.starts_with('.') {
                        versions.push(name);
                    }
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
fn detect_install_source(def: &ProjectDef) -> (Option<String>, Option<String>) {
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
fn build_env_vars_status(def: &ProjectDef, links_dir: &str) -> Vec<EnvVarStatus> {
    let links_lower = links_dir.to_lowercase();
    let mut statuses = Vec::new();

    for var_def in &def.env_vars {
        let name = &var_def.name;
        let (value, source, exists, in_anyversion) = if let Some((val, src)) = get_registry_env_any(name) {
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
        });
    }

    statuses
}

/// 构建缓存状态
fn build_cache_status(def: &ProjectDef) -> Option<CacheStatus> {
    use crate::commands::cache::{get_npm_cache_path, get_yarn_cache_path, get_pnpm_cache_path,
                                  get_pip_cache_path, get_maven_cache_path, get_nuget_cache_path,
                                  get_dir_size, format_bytes, cache_detect_evidence};

    // 根据项目 ID 映射缓存路径
    let (cache_path, detect_source) = match def.id.as_str() {
        "nodejs" => {
            let p = get_npm_cache_path();
            let (src, _) = cache_detect_evidence("npm", &p.to_string_lossy());
            (p, src)
        }
        "yarn" => {
            let p = get_yarn_cache_path();
            let (src, _) = cache_detect_evidence("yarn", &p.to_string_lossy());
            (p, src)
        }
        "pnpm" => {
            let p = get_pnpm_cache_path();
            let (src, _) = cache_detect_evidence("pnpm", &p.to_string_lossy());
            (p, src)
        }
        "python" => {
            let p = get_pip_cache_path();
            let (src, _) = cache_detect_evidence("pip", &p.to_string_lossy());
            (p, src)
        }
        "maven" => {
            let p = get_maven_cache_path();
            let (src, _) = cache_detect_evidence("mvn", &p.to_string_lossy());
            (p, src)
        }
        "nuget" => {
            let p = get_nuget_cache_path();
            let (src, _) = cache_detect_evidence("nuget", &p.to_string_lossy());
            (p, src)
        }
        _ => return None,
    };

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

    Some(CacheStatus {
        path: cache_path.to_string_lossy().to_string(),
        size: size_str,
        is_link,
        real_target,
        detect_source,
    })
}

/// 构建服务状态
fn build_service_status(def: &ProjectDef, junction_path: &Path) -> Option<ServiceStatus> {
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
    } else {
        PathBuf::new()
    };

    let data_dir = def.data_dir.as_deref()
        .map(|d| base_path.join(d).to_string_lossy().to_string())
        .unwrap_or_default();

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
fn get_bin_paths(sdk_id: &str, link_dir: &str) -> Vec<String> {
    match sdk_id {
        "go" | "java" | "flutter" | "maven" | "gradle" | "harmony" | "cuda" | "ffmpeg" => {
            vec![format!("{}\\bin", link_dir)]
        }
        "python" => {
            vec![link_dir.to_string(), format!("{}\\Scripts", link_dir)]
        }
        "rust" => {
            vec![format!("{}\\.cargo\\bin", link_dir)]
        }
        "android" => {
            vec![
                format!("{}\\cmdline-tools\\latest\\bin", link_dir),
                format!("{}\\platform-tools", link_dir),
            ]
        }
        "nodejs" | "bun" | "yarn" | "pnpm" | "nginx" | "redis" => {
            vec![link_dir.to_string()]
        }
        "mysql" | "mongodb" | "postgresql" => {
                  vec![format!("{}\\bin", link_dir)]
        }
        _ => vec![link_dir.to_string()],
    }
}
