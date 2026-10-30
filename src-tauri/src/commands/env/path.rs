use super::*;

pub fn add_to_user_path(paths: &[String]) -> Result<(), String> {
    let known_tools = crate::commands::project::registry::all_ids();

    if let Some(user_path) = get_registry_env("PATH") {
        let mut parts = std::env::split_paths(&user_path)
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        let mut modified = false;
        // 倒序遍历插入，以保持传入的 paths 之间的相对顺序在最前列
        for path in paths.iter().rev() {
            let path_lower = path.to_lowercase();

            // 防御检查：是否包含重复的工具名（如 ...nodejs\nodejs）
            for tool in &known_tools {
                let double = format!("{}{}", tool, tool);
                let double_sep = format!("{}\\{}", tool, tool);
                if path_lower.ends_with(&double) || path_lower.ends_with(&double_sep) {
                    return Err(format!("PATH 条目疑似损坏（重复的工具名）: {}", path));
                }
            }

            // 无论当前 PATH 中是否已存在，我们都先将其移除并插入到最前面，以保证最高优先级
            parts.retain(|p| p.to_lowercase() != path_lower);
            parts.insert(0, path.clone());
            modified = true;
        }

        if modified {
            let new_path = std::env::join_paths(parts.iter().map(Path::new))
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .to_string();
            set_registry_env("PATH", &new_path)?;
        }
    } else {
        let new_path = std::env::join_paths(paths.iter().map(Path::new))
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .to_string();
        set_registry_env("PATH", &new_path)?;
    }
    Ok(())
}

/// 把条目**追加到用户 PATH 末尾**（已存在时先移除再追加，确保落在最后）。
///
/// 与 `add_to_user_path`（插到最前，用于 SDK 自身的 bin，保证版本切换优先）相对：
/// 缓存型环境变量的 bin —— rustup 把 `rustup.exe` 等 shim 放在 `$CARGO_HOME\bin` —— 必须排在
/// `sdk\<id>\bin` **之后**。否则 shim 会抢走 `cargo`/`rustc`，Kira 通过 junction 做的
/// 版本切换就失效了（实测 rustup 的 stable 是 1.96.0，而 Kira 管的是 1.88.0）。
pub fn append_to_user_path(paths: &[String]) -> Result<(), String> {
    let wanted: Vec<String> = paths
        .iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    if wanted.is_empty() {
        return Ok(());
    }

    let current = get_registry_env("PATH").unwrap_or_default();
    let mut parts: Vec<String> = std::env::split_paths(&current)
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    for path in &wanted {
        let lower = path.to_lowercase();
        parts.retain(|p| p.to_lowercase() != lower);
        parts.push(path.clone());
    }

    let new_path = std::env::join_paths(parts.iter().map(Path::new))
        .map_err(|e| e.to_string())?
        .to_string_lossy()
        .to_string();
    // 结果与现状一致就不写注册表（避免无意义的 WM_SETTINGCHANGE 广播与 PATH 抖动）
    if new_path.eq_ignore_ascii_case(&current) {
        return Ok(());
    }
    set_registry_env("PATH", &new_path)
}

/// 缓存型环境变量（`sub_dir`）对应的可执行目录：`<缓存根>/<sub_dir>/bin`。
///
/// 只返回**真实存在**的目录：rustup 把 shim 放在 `$CARGO_HOME\bin`，而 `$RUSTUP_HOME` 下没有
/// `bin`，所以 rust 只会得到 `caches\cargo\bin` 一条。
pub(crate) fn sub_dir_bin_paths(
    env_vars: &[crate::commands::project::types::EnvVarDef],
    cache_root: &Path,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for var in env_vars {
        let Some(sub) = var.sub_dir.as_ref() else {
            continue;
        };
        let dir = cache_root.join(sub).join("bin");
        if dir.is_dir() {
            let value = dir.to_string_lossy().to_string();
            if !out.iter().any(|p| p.eq_ignore_ascii_case(&value)) {
                out.push(value);
            }
        }
    }
    out
}

/// Kira 当前托管的全部 PATH 条目：各完全托管 SDK 的 bin 目录 + 其缓存型环境变量的 bin 目录。
///
/// 供「冲突检测 / 一键停用」排除**自托管**条目使用：例如 rust 的 `caches\cargo\bin` 里有
/// `rustup.exe`，会命中 rustup 的 `path_keywords`，但它是 Kira 自己写进去的
/// （cargokit 这类 rustup 生态工具硬依赖 PATH 上的 rustup.exe），
/// 既不该报成冲突，更不该被"停用 rustup"删掉。
pub fn managed_path_entries() -> Vec<String> {
    let config = load_config();
    let cache_root = crate::commands::config::get_cache_root();
    let mut out: Vec<String> = Vec::new();
    for item_id in &config.managed_items {
        let link_dir = format!("{}\\{}", config.links_dir, item_id);
        for path in crate::commands::project::scanner::get_bin_paths(item_id, &link_dir) {
            push_unique_path(&mut out, path);
        }
        if let Some(def) = crate::commands::project::registry::find_by_id(item_id) {
            for path in sub_dir_bin_paths(&def.env_vars, &cache_root) {
                push_unique_path(&mut out, path);
            }
        }
    }
    out
}

/// 忽略大小写去重追加（PATH 条目在不同来源里大小写可能不一致）。
pub(super) fn push_unique_path(out: &mut Vec<String>, value: String) {
    if !out.iter().any(|p| p.eq_ignore_ascii_case(&value)) {
        out.push(value);
    }
}

/// PATH 条目列表里是否已包含某目录（忽略大小写与结尾反斜杠）。
pub(super) fn path_list_contains(entries: &[String], target: &str) -> bool {
    let normalize = |p: &str| p.trim().trim_end_matches('\\').to_lowercase();
    let target = normalize(target);
    entries.iter().any(|p| normalize(p) == target)
}

/// 列出某 SDK 被 Kira 托管写入用户 PATH 的条目（供前端「环境变量」页展示）。
///
/// 与 `configure_sdk_env_vars` / `remove_sdk_env_vars` 完全对应的两份来源：
/// 1. `sdk_bin` —— SDK 自身的可执行目录（`bin_dirs`，如 `sdk\rust\bin`）；
/// 2. `cache_bin` —— 缓存型环境变量（`sub_dir`）派生的 bin，如 `data_dir\caches\cargo\bin`
///    （rustup 把 `rustup.exe` 等 shim 放在 `$CARGO_HOME\bin`，cargokit 这类工具硬依赖它）。
///
/// 只在**完全托管**时调用（调用方判断），因此这里不做托管判定。
pub fn managed_path_entries_for_sdk(
    sdk_id: &str,
    link_dir: &str,
    def: &crate::commands::project::types::ProjectDef,
) -> Vec<crate::commands::project::types::ManagedPathEntry> {
    // MSIX 型（如 WinGet）不写 PATH（见 `project_manage`），因此没有「托管写入的条目」。
    // 这里必须短路：`bin_dirs` 的空串会生成 `links_dir/<id>` 这个**永远不存在**的候选目录
    // （msix 不做 junction），而完全托管的项目是全量展示候选（含「目录缺失」告警）——
    // 结果就是环境变量页挂出一条我们从不写入、也永远不存在的假条目。
    if def.is_msix() {
        return Vec::new();
    }

    let current: Vec<String> = std::env::split_paths(&get_registry_env("PATH").unwrap_or_default())
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    let mut result: Vec<crate::commands::project::types::ManagedPathEntry> = Vec::new();
    let push_entry = |path: String, kind: &str, result: &mut Vec<_>| {
        let exists = Path::new(&path).exists();
        let in_path = path_list_contains(&current, &path);
        result.push(crate::commands::project::types::ManagedPathEntry {
            path,
            kind: kind.to_string(),
            in_path,
            exists,
        });
    };

    for path in crate::commands::project::scanner::get_bin_paths(sdk_id, link_dir) {
        push_entry(path, "sdk_bin", &mut result);
    }
    for path in sub_dir_bin_paths(&def.env_vars, &crate::commands::config::get_cache_root()) {
        push_entry(path, "cache_bin", &mut result);
    }
    result
}

pub fn remove_from_user_path(paths: &[String]) -> Result<(), String> {
    if let Some(user_path) = get_registry_env("PATH") {
        let parts = std::env::split_paths(&user_path)
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        let initial_len = parts.len();
        let new_parts = parts
            .into_iter()
            .filter(|p| {
                let p_lower = p.to_lowercase();
                !paths.iter().any(|target| target.to_lowercase() == p_lower)
            })
            .collect::<Vec<_>>();

        if new_parts.len() != initial_len {
            let new_path = std::env::join_paths(new_parts.iter().map(Path::new))
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .to_string();
            set_registry_env("PATH", &new_path)?;
        }
    }
    Ok(())
}

/// 将路径添加到【系统级 PATH】(HKLM)，对所有用户和构建子进程可见。
/// 需要管理员权限；失败时返回错误，调用方应提示用户以管理员身份运行。
pub fn add_to_system_path(paths: &[String]) -> Result<(), String> {
    let known_tools = crate::commands::project::registry::all_ids();

    if let Some(system_path) = get_system_registry_env("PATH") {
        let mut parts = std::env::split_paths(&system_path)
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        let mut modified = false;
        for path in paths.iter().rev() {
            let path_lower = path.to_lowercase();

            for tool in &known_tools {
                let double = format!("{}{}", tool, tool);
                let double_sep = format!("{}\\{}", tool, tool);
                if path_lower.ends_with(&double) || path_lower.ends_with(&double_sep) {
                    return Err(format!("PATH 条目疑似损坏（重复的工具名）: {}", path));
                }
            }

            parts.retain(|p| p.to_lowercase() != path_lower);
            parts.insert(0, path.clone());
            modified = true;
        }

        if modified {
            let new_path = std::env::join_paths(parts.iter().map(Path::new))
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .to_string();
            set_system_registry_env("PATH", &new_path)?;
        }
    } else {
        let new_path = std::env::join_paths(paths.iter().map(Path::new))
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .to_string();
        set_system_registry_env("PATH", &new_path)?;
    }
    Ok(())
}

/// 从【系统级 PATH】(HKLM) 中移除指定路径。
pub fn remove_from_system_path(paths: &[String]) -> Result<(), String> {
    if let Some(system_path) = get_system_registry_env("PATH") {
        let parts = std::env::split_paths(&system_path)
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        let initial_len = parts.len();
        let new_parts = parts
            .into_iter()
            .filter(|p| {
                let p_lower = p.to_lowercase();
                !paths.iter().any(|target| target.to_lowercase() == p_lower)
            })
            .collect::<Vec<_>>();

        if new_parts.len() != initial_len {
            let new_path = std::env::join_paths(new_parts.iter().map(Path::new))
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .to_string();
            set_system_registry_env("PATH", &new_path)?;
        }
    }
    Ok(())
}
