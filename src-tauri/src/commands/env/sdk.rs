use super::*;

/// 自动配置 SDK 相关环境变量（注册表驱动）。
/// 新增 SDK 时只需在 projects/<id>/config.json 中定义 env_vars，此函数自动生效。
///
/// 设计原则：
///   - 所有 *_HOME 类变量指向 link_dir（版本切换只需重定向 junction）
///   - 需要独立缓存目录的变量（如 CARGO_HOME / RUSTUP_HOME）用 `sub_dir` 声明，
///     值锚定到**缓存根** `data_dir/caches/<sub_dir>`（见 config::get_cache_root）
/// 计算某 SDK 环境变量的目标值，并（对 sub_dir 类型）确保目录真实存在。
///
/// - 带 `sub_dir` 的变量（如 CARGO_HOME/RUSTUP_HOME）锚定到**缓存根**
///   `get_cache_root()/<sub_dir>`（= `data_dir/caches/<sub_dir>`），不指向工具链 junction 之下，
///   也不再拼 `<sdk_id>`。
///   原因一：link_dir 是指向某个具体版本工具链的 junction，版本目录内并不包含
///   cargo/rustup 的缓存数据；若把 *_HOME 指向 junction 下的子目录，该目录不会存在，
///   导致 cargo/rustup 启动时因找不到 HOME 而失败。
///   原因二：此前解析为 `data_dir/<sdk_id>/<sub_dir>`，与「缓存目录设置」写进注册表的
///   `data_dir/caches/<sub_dir>` 分叉成两份真实缓存（本机实测 cargo 各 848MB / 2.2GB）；
///   只要点一次「修复环境变量」，CARGO_HOME 就会被改到另一份，旧缓存变孤儿。
/// - 不带 sub_dir 的变量直接用 link_dir（版本切换只需重定向 junction）。
pub fn sdk_env_var_value(
    _sdk_id: &str,
    link_dir: &str,
    var_info: &crate::commands::project::types::EnvVarDef,
) -> String {
    if let Some(ref sub) = var_info.sub_dir {
        let home = crate::commands::config::get_cache_root().join(sub);
        let _ = std::fs::create_dir_all(&home);
        home.to_string_lossy().to_string()
    } else {
        link_dir.to_string()
    }
}

pub fn configure_sdk_env_vars(
    sdk_id: &str,
    link_dir: &str,
    _version_dir: &str,
) -> Result<(), String> {
    use crate::commands::project::registry;

    let config = load_config();
    if !config.managed_items.contains(sdk_id) {
        return Ok(());
    }

    let sdk_def = match registry::find_by_id(sdk_id) {
        Some(d) => d,
        None => return Ok(()),
    };

    for var_info in &sdk_def.env_vars {
        // Skip compat/discovery tier vars - only scan, not manage
        if let Some(ref tier) = var_info.tier {
            if *tier == crate::commands::project::types::EnvVarTier::Compat {
                continue;
            }
            if *tier == crate::commands::project::types::EnvVarTier::Clear {
                let _ = set_registry_env(&var_info.name, "");
                continue;
            }
        }

        // 仅托管「路径型」变量（check_type=path 或缺省）。URL/端口/标志等 nonempty/runtime
        // 变量（如 PUB_HOSTED_URL）不应被设为 link_dir（本地 SDK 路径），否则会破坏其语义
        // （例如 PUB_HOSTED_URL 需要 https:// 前缀，设成本地路径会导致 Invalid PUB_HOSTED_URL）。
        // 这类变量由各包管理器的镜像/运行时配置机制管理，不由本处托管。
        if var_info.check_type != "path" {
            continue;
        }

        // 值策略由 EnvVarDef.sub_dir 驱动（详见 sdk_env_var_value）。
        let value = sdk_env_var_value(sdk_id, link_dir, var_info);
        let _ = set_registry_env(&var_info.name, &value);
    }

    // 自动将可执行目录添加到用户 PATH 变量中
    let bin_paths = crate::commands::project::scanner::get_bin_paths(sdk_id, link_dir);
    let _ = add_to_user_path(&bin_paths);

    // 缓存型环境变量的 bin 目录**追加到 PATH 末尾**（rust 的 `caches\cargo\bin` 里有 rustup.exe）。
    // 放末尾的理由见 append_to_user_path 的注释：让 rustup 生态工具可达，同时不抢 Kira 的版本切换。
    if let Some(def) = registry::find_by_id(sdk_id) {
        let cache_bin_paths =
            sub_dir_bin_paths(&def.env_vars, &crate::commands::config::get_cache_root());
        let _ = append_to_user_path(&cache_bin_paths);
    }

    Ok(())
}

/// 移除 SDK 相关的环境变量（注册表驱动）。
/// 当卸载某 SDK 最后一个版本时调用。
pub fn remove_sdk_env_vars(sdk_id: &str) -> Result<(), String> {
    use crate::commands::project::registry;

    let sdk_def = match registry::find_by_id(sdk_id) {
        Some(d) => d,
        None => return Ok(()),
    };

    for var_info in &sdk_def.env_vars {
        // Only remove vars we would have set (core + package), skip compat
        if let Some(ref tier) = var_info.tier {
            if *tier == crate::commands::project::types::EnvVarTier::Compat {
                continue;
            }
        }
        // 与 configure 对称：仅清理我们实际托管过的路径型变量。
        // 非路径变量（URL/标志等）从未被本处托管，不应清空。
        if var_info.check_type != "path" {
            continue;
        }
        let _ = set_registry_env(&var_info.name, "");
    }

    // 从用户 PATH 中移除该 SDK 的可执行目录
    let config = load_config();
    let junction_path = Path::new(&config.links_dir).join(sdk_id);
    let link_str = junction_path.to_string_lossy().to_string();
    let bin_paths = crate::commands::project::scanner::get_bin_paths(sdk_id, &link_str);
    let _ = remove_from_user_path(&bin_paths);

    // 与 configure 完全对称：缓存型环境变量的 bin 目录（如 rust 的 `caches\cargo\bin`）
    // 只在托管期间存在；取消托管/卸载后必须一并撤下，不能留在用户 PATH 里当残留。
    let cache_bin_paths = sub_dir_bin_paths(
        &sdk_def.env_vars,
        &crate::commands::config::get_cache_root(),
    );
    let _ = remove_from_user_path(&cache_bin_paths);

    Ok(())
}

/// 获取指定项目可配置的运行时环境变量（user_configurable_vars）的当前值
#[tauri::command]
pub fn get_user_configurable_vars(project_id: String) -> Result<Vec<serde_json::Value>, String> {
    use crate::commands::project::registry;
    let def =
        registry::find_by_id(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;

    let mut results = Vec::new();
    for var in &def.user_configurable_vars {
        let (current_value, source) = get_registry_env_any(&var.name)
            .map(|(v, s)| (Some(v), s.to_string()))
            .unwrap_or((None, "未设置".to_string()));

        results.push(serde_json::json!({
            "name": var.name,
            "desc": var.desc,
            "placeholder": var.placeholder,
            "options": var.options,
            "var_type": var.var_type,
            "current_value": current_value,
            "source": source,
        }));
    }
    Ok(results)
}

/// 设置用户自定义环境变量（运行时参数）
/// 冲突检测：若该变量名已被某个「已托管」项目的核心 env 变量管理，则拒绝设置，
/// 避免用户自定义变量覆盖/破坏 vex 托管的 SDK 环境（如 JAVA_HOME、CARGO_HOME）。
#[tauri::command]
pub fn set_user_configurable_var(name: String, value: String) -> Result<(), String> {
    let name_trim = name.trim().to_string();
    if name_trim.is_empty() {
        return Err("环境变量名不能为空".to_string());
    }
    // 检测是否与已托管项目的核心 env 变量冲突
    if let Some((managed_by, is_clear)) = find_managed_env_conflict(&name_trim) {
        return Err(format!(
            "变量 {} 已被项目「{}」托管（{}），不能通过用户自定义设置覆盖。请先取消该项目的环境变量托管。",
            name_trim,
            managed_by,
            if is_clear { "该变量由工具接管/清空" } else { "该变量指向 Kira 管理的路径" }
        ));
    }
    set_registry_env(&name_trim, &value)
}

/// 在已托管项目的核心 env 变量中查找是否与 name 冲突。
/// 返回 (project_id, 是否为 Clear 级别变量)。None 表示无冲突。
pub(super) fn find_managed_env_conflict(name: &str) -> Option<(String, bool)> {
    use crate::commands::project::registry;
    use crate::commands::project::types::EnvVarTier;
    let config = crate::commands::config::load_config();
    let name_lower = name.to_lowercase();
    for managed_id in config.managed_items.iter() {
        if let Some(def) = registry::find_by_id(managed_id) {
            for var in &def.env_vars {
                // 兼容层变量由工具接管/忽略，不构成冲突；core/package/clear 才算托管
                if var
                    .tier
                    .as_ref()
                    .map_or(false, |t| *t == EnvVarTier::Compat)
                {
                    continue;
                }
                if var.name.eq_ignore_ascii_case(&name_lower) {
                    let is_clear = var.tier.as_ref().map_or(false, |t| *t == EnvVarTier::Clear);
                    return Some((managed_id.clone(), is_clear));
                }
            }
        }
    }
    None
}

/// 删除用户自定义环境变量
#[tauri::command]
pub fn delete_user_configurable_var(name: String) -> Result<(), String> {
    set_registry_env(&name, "")
}

#[tauri::command]
pub fn toggle_item_management(
    id: String,
    enable: bool,
    cache_dest: Option<String>,
) -> Result<(), String> {
    use crate::commands::project::registry;
    let sdk_def =
        registry::find_by_id(&id).ok_or_else(|| format!("未找到该标识符对应的配置: {}", id))?;

    let mut config = load_config();

    if enable {
        // 1. Add to managed_items
        config.managed_items.insert(id.clone());

        // 2. Backup conflicting environment variables（按项目隔离备份，避免多项目同名变量覆盖）
        for var_info in &sdk_def.env_vars {
            let var_name = &var_info.name;
            if let Some((val, _source)) = get_registry_env_any(var_name) {
                if !val
                    .to_lowercase()
                    .contains(&config.links_dir.to_lowercase())
                {
                    config
                        .original_envs
                        .entry(id.clone())
                        .or_default()
                        .entry(var_name.to_string())
                        .or_insert(val);
                }
            }
        }

        // 3. Backup conflicting PATH entries from HKCU
        let mut original_paths_to_save = Vec::new();
        if let Some(user_path) = get_registry_env("PATH") {
            let parts = std::env::split_paths(&user_path)
                .map(|p| p.to_string_lossy().to_string())
                .collect::<Vec<_>>();

            let mut matched_entries = Vec::new();
            let mut remaining_entries = Vec::new();

            for p_str in parts {
                if p_str.is_empty() {
                    continue;
                }

                let mut matches = false;
                let p_lower = p_str.to_lowercase();

                if !p_lower.contains(&config.links_dir.to_lowercase()) {
                    for rule in &sdk_def.find_rules {
                        match &rule.pattern {
                            crate::commands::project::types::ResolvePattern::PathContains {
                                path_key: pattern,
                                ..
                            } => {
                                if p_lower.contains(&pattern.to_lowercase()) {
                                    matches = true;
                                    break;
                                }
                            }
                            crate::commands::project::types::ResolvePattern::FixedPath {
                                path: fixed_path,
                                ..
                            } => {
                                if p_lower.contains(&fixed_path.to_lowercase()) {
                                    matches = true;
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                }

                if matches {
                    matched_entries.push(p_str.clone());
                    original_paths_to_save.push(p_str);
                } else {
                    remaining_entries.push(p_str);
                }
            }

            if !matched_entries.is_empty() {
                config
                    .original_paths
                    .entry(id.clone())
                    .or_insert_with(Vec::new)
                    .extend(matched_entries);

                let new_path = std::env::join_paths(remaining_entries.iter().map(Path::new))
                    .map_err(|e| e.to_string())?
                    .to_string_lossy()
                    .to_string();
                set_registry_env("PATH", &new_path)?;
            }
        }

        // Save config changes so far
        crate::commands::config::save_config(&config)?;

        // 4. Configure SDK env vars if a version is active
        let junction_path = Path::new(&config.links_dir).join(&id);
        if junction_path.exists() {
            let link_str = junction_path.to_string_lossy().to_string();
            let dest_str = fs::canonicalize(&junction_path)
                .map(|p| {
                    p.to_string_lossy()
                        .to_string()
                        .trim_start_matches(r"\\?\")
                        .to_string()
                })
                .unwrap_or_default();
            let _ = configure_sdk_env_vars(&id, &link_str, &dest_str);
        }

        // 5. Migrate cache if requested
        if let Some(dest) = cache_dest {
            if !dest.is_empty() {
                let _ = crate::commands::cache::migrate_cache_path(id.clone(), dest);
            }
        }
    } else {
        // Disable management
        config.managed_items.remove(&id);

        // 1. Remove AnyVersion environment variables for this SDK
        let _ = remove_sdk_env_vars(&id);

        // 2. Restore original environment variables from backup（按项目隔离恢复）
        if let Some(proj_envs) = config.original_envs.get_mut(&id) {
            for var_info in &sdk_def.env_vars {
                let var_name = &var_info.name;
                if let Some(orig_val) = proj_envs.remove(var_name) {
                    let _ = set_registry_env(var_name, &orig_val);
                }
            }
            // 该项目已无剩余备份时，清理空的子 Map
            if proj_envs.is_empty() {
                config.original_envs.remove(&id);
            }
        }

        // 3. Restore original PATH entries
        if let Some(orig_paths) = config.original_paths.remove(&id) {
            let _ = add_to_user_path(&orig_paths);
        }

        // Save updated config
        crate::commands::config::save_config(&config)?;
    }

    Ok(())
}
