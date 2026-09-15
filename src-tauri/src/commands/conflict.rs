use std::collections::HashMap;
use std::path::Path;
use serde::Serialize;
use crate::commands::project::types::{ConflictManagerDef, ConflictManagerStatus};
use crate::commands::project::registry;
use crate::commands::utils::{expand_home, is_exe_in_path};
use crate::commands::env::{get_registry_env_any, set_registry_env, set_system_registry_env, broadcast_setting_change, get_registry_env, get_system_registry_env};
use crate::commands::cache::{get_dir_size, format_bytes, clean_pkg_cache_impl, migrate_pkg_storage_impl};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  「一键停用」预案：预演与实际执行共用同一份计算
//
//  背景（为什么要有预案）：停用要清空第三方版本管理器的环境变量、并按关键字从
//  用户级/系统级 PATH 里整条删除匹配条目。这是**不可逆**操作（原值不归 Kira 备份
//  管），尤其系统级 PATH 条目删掉后很难找回。因此：
//    - 执行前先把"将要清空哪些变量、删掉哪些 PATH 条目"算出来给用户确认；
//    - 预演与实际执行**调用同一个 plan_disable / matched_path_entries**，
//      保证"提示删 A、实际删 B"这种偏差不可能发生；
//    - 执行前自动创建一次全量环境备份（失败即中止）。
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 某个变量在注册表中的存在情况（用户级 / 系统级）。
#[derive(Clone, Debug, Default)]
pub struct VarPresence {
    pub user: Option<String>,
    pub system: Option<String>,
}

/// 停用过程中单个环境变量的处置方案。
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct DisableVarAction {
    /// 变量名
    pub name: String,
    /// 当前值（优先取用户级，其次系统级；都没有则 null）
    pub current_value: Option<String>,
    /// 当前值所在层级："user" | "system" | "both" | "none"
    pub level: String,
    /// "clear" = 将被清空；"keep_managed" = 属于本项目托管变量，跳过不动
    pub action: String,
}

/// 「一键停用」的完整预案。
#[derive(Serialize, Clone, Debug, Default)]
pub struct DisablePlan {
    /// 管理器显示名（如 "Rustup"）
    pub manager_display_name: String,
    /// 逐个变量的处置明细
    pub variables: Vec<DisableVarAction>,
    /// 将被清空的变量名
    pub cleared_vars: Vec<String>,
    /// 因属于本项目托管变量（同时出现在 env_vars 与 conflict_managers[].env_vars）而跳过的变量名
    pub kept_managed_vars: Vec<String>,
    /// 将从**用户级** PATH 删除的条目
    pub user_path_entries: Vec<String>,
    /// 将从**系统级** PATH 删除的条目（清理需管理员权限，失败会静默忽略）
    pub system_path_entries: Vec<String>,
}

/// 判断 PATH 值中哪些条目「包含任一关键字」（大小写不敏感，保持原有顺序）。
///
/// 预演与实际删除**共用**此函数，避免提示与实际操作不一致。
/// 注意：这里刻意沿用「子串包含」语义——`\nvm`、`.cargo\bin` 这类关键字本来就
/// 需要子串匹配才能覆盖 nvm 的 symlink 目录与 rustup 的 shim 目录。
pub fn matched_path_entries(path_value: &str, keywords: &[String]) -> Vec<String> {
    let keys: Vec<String> = keywords
        .iter()
        .map(|k| k.trim().to_lowercase())
        .filter(|k| !k.is_empty())
        .collect();
    if keys.is_empty() {
        return Vec::new();
    }
    std::env::split_paths(path_value)
        .map(|p| p.to_string_lossy().to_string())
        .filter(|p| {
            let lower = p.to_lowercase();
            keys.iter().any(|k| lower.contains(k))
        })
        .collect()
}

/// 从匹配结果中剔除「Kira 自己托管」的 PATH 条目（忽略大小写与结尾反斜杠）。
///
/// 纯函数，便于单测；带注册表读取的包装见 `exclude_managed_entries`。
fn filter_out_managed_entries(matched: Vec<String>, managed: &[String]) -> Vec<String> {
    if managed.is_empty() {
        return matched;
    }
    let normalize = |p: &str| p.trim().trim_end_matches('\\').to_lowercase();
    let managed_set: std::collections::HashSet<String> =
        managed.iter().map(|p| normalize(p)).collect();
    matched
        .into_iter()
        .filter(|p| !managed_set.contains(&normalize(p)))
        .collect()
}

/// 见 `filter_out_managed_entries`：托管条目（SDK 自身的 bin，以及缓存型环境变量的 bin，
/// 如 rust 的 `caches\cargo\bin`）即使命中 `path_keywords` 也不算冲突，
/// 且「停用」时必须保留 —— 那是 Kira 自己写进去的（rustup 生态工具硬依赖它）。
fn exclude_managed_entries(matched: Vec<String>) -> Vec<String> {
    filter_out_managed_entries(matched, &super::env::managed_path_entries())
}

/// 计算「一键停用」预案（纯函数，便于单测）。
///
/// `managed_env_vars` 为该项目当前实际被 Kira 接管的变量集合
/// （未托管项目为空集）。落在该集合里的变量会被标记为 `keep_managed` 跳过：
/// 例如 rust 项目的 `RUSTUP_HOME` 同时属于项目 `env_vars`（Kira 写入
/// `data_dir\rust\rustup`）与 rustup 管理器的 `env_vars`，若一并清空就等于把
/// Kira 自己刚设好的工具链目录指向打回原形——与"停用外部管理器"的目的相反。
pub fn plan_disable(
    mgr: &ConflictManagerDef,
    managed_env_vars: &std::collections::HashSet<String>,
    presence: &HashMap<String, VarPresence>,
    user_path: Option<&str>,
    system_path: Option<&str>,
) -> DisablePlan {
    let mut plan = DisablePlan {
        manager_display_name: mgr.display_name.clone(),
        ..Default::default()
    };

    for name in &mgr.env_vars {
        let p = presence.get(name).cloned().unwrap_or_default();
        let level = match (p.user.as_ref(), p.system.as_ref()) {
            (Some(_), Some(_)) => "both",
            (Some(_), None) => "user",
            (None, Some(_)) => "system",
            (None, None) => "none",
        }
        .to_string();

        let keep_managed = managed_env_vars.contains(name);
        if keep_managed {
            plan.kept_managed_vars.push(name.clone());
        } else {
            plan.cleared_vars.push(name.clone());
        }

        plan.variables.push(DisableVarAction {
            name: name.clone(),
            current_value: p.user.or(p.system),
            level,
            action: if keep_managed { "keep_managed" } else { "clear" }.to_string(),
        });
    }

    if let Some(p) = user_path {
        // 排除 Kira 自托管条目：预演列出的条目必须与实际删除的完全一致
        plan.user_path_entries = exclude_managed_entries(matched_path_entries(p, &mgr.path_keywords));
    }
    if let Some(p) = system_path {
        plan.system_path_entries = exclude_managed_entries(matched_path_entries(p, &mgr.path_keywords));
    }

    plan
}

/// 读取一组变量在用户级/系统级的当前值。
fn collect_presence(vars: &[String]) -> HashMap<String, VarPresence> {
    let mut map = HashMap::new();
    for name in vars {
        map.insert(
            name.clone(),
            VarPresence {
                user: get_registry_env(name),
                system: get_system_registry_env(name),
            },
        );
    }
    map
}

/// 依据当前注册表状态计算某个管理器「一键停用」的预案。
fn build_disable_plan(
    sdk_id: &str,
    manager_id: &str,
) -> Result<(DisablePlan, ConflictManagerDef), String> {
    let project = registry::find_by_id(sdk_id)
        .ok_or_else(|| format!("未找到项目: {}", sdk_id))?;
    let mgr = project
        .conflict_managers
        .iter()
        .find(|m| m.id == manager_id)
        .ok_or_else(|| format!("在项目 {} 下未找到冲突管理器: {}", sdk_id, manager_id))?
        .clone();

    let config = crate::commands::config::load_config();
    let delegation =
        crate::commands::project::scanner::get_project_delegation(&config, sdk_id, &project);

    let presence = collect_presence(&mgr.env_vars);
    let user_path = get_registry_env("PATH");
    let system_path = get_system_registry_env("PATH");

    let plan = plan_disable(
        &mgr,
        &delegation.env_vars,
        &presence,
        user_path.as_deref(),
        system_path.as_deref(),
    );
    Ok((plan, mgr))
}

/// 预演「一键停用」：只读，不产生任何副作用，供前端在执行前列出影响面。
#[tauri::command]
pub fn preview_conflict_manager_disable(
    sdk_id: String,
    manager_id: String,
) -> Result<DisablePlan, String> {
    build_disable_plan(&sdk_id, &manager_id).map(|(plan, _)| plan)
}

#[tauri::command]
pub fn get_conflict_managers_status(sdk_id: String) -> Result<Vec<ConflictManagerStatus>, String> {
    let project = registry::find_by_id(&sdk_id)
        .ok_or_else(|| format!("未找到项目: {}", sdk_id))?;
        
    let mut status_list = Vec::new();
    
    for def in &project.conflict_managers {
        // 1. 判断是否安装：检查可执行文件是否存在于 PATH
        let installed = if let Some(ref exe) = def.exe_name {
            is_exe_in_path(exe)
        } else {
            false
        };
        
        // 2. 获取环境变量状态：遍历 env_vars
        let mut env_vars_status = HashMap::new();
        for var in &def.env_vars {
            let val = get_registry_env_any(var).map(|(v, _)| v);
            env_vars_status.insert(var.clone(), val);
        }
        
        // 3. 获取 PATH 匹配状态
        let mut path_status = Vec::new();
        let mut check_path = |path_str: String| {
            for part in std::env::split_paths(&path_str) {
                let part_str = part.to_string_lossy().to_string();
                let part_lower = part_str.to_lowercase();
                for keyword in &def.path_keywords {
                    if part_lower.contains(&keyword.to_lowercase()) {
                        if !path_status.contains(&part_str) {
                            path_status.push(part_str.clone());
                        }
                    }
                }
            }
        };
        if let Some(user_path) = get_registry_env("PATH") {
            check_path(user_path);
        }
        if let Some(sys_path) = get_system_registry_env("PATH") {
            check_path(sys_path);
        }
        // Kira 自己写入的条目（各 SDK 的 bin 与缓存型环境变量的 bin）不算冲突
        let path_status = exclude_managed_entries(path_status);
        
        // 4. 获取缓存目录与空间大小
        // 以「缓存位置对应的环境变量」为唯一真源（如 rustup 的 RUSTUP_HOME、
        // nvm 的 NVM_HOME），当前值优先取自注册表；未配置时才回退到默认路径
        // cache_default_path（如 {home}\.rustup）。避免环境变量区与缓存路径区脱节。
        let mut cache_path = String::new();
        if let Some(ref env_var) = def.cache_env_var {
            if let Some(val) = env_vars_status.get(env_var).cloned().flatten() {
                if !val.trim().is_empty() {
                    cache_path = val;
                }
            }
        }
        if cache_path.is_empty() {
            if let Some(ref raw_path) = def.cache_default_path {
                cache_path = expand_home(raw_path);
            }
        }
        
        let cache_size = if !cache_path.is_empty() && Path::new(&cache_path).exists() {
            let size = get_dir_size(Path::new(&cache_path));
            format_bytes(size)
        } else {
            "0 B".to_string()
        };
        
        // 5. 判断是否已被禁用：
        // 所有的环境变量要么为 None，要么为空，且在 PATH 中没有匹配到的路径项
        let all_env_empty = env_vars_status.values().all(|v| v.as_ref().map_or(true, |val| val.is_empty()));
        let path_empty = path_status.is_empty();
        let is_disabled = all_env_empty && path_empty;
        
        status_list.push(ConflictManagerStatus {
            id: def.id.clone(),
            display_name: def.display_name.clone(),
            installed,
            env_vars_status,
            path_status,
            cache_path,
            cache_size,
            is_disabled,
        });
    }
    
    Ok(status_list)
}

/// 解析冲突管理器的当前缓存路径：
/// 优先使用缓存位置对应环境变量（cache_env_var，如 RUSTUP_HOME）的注册表当前值，
/// 未配置时回退到 cache_default_path（如 {home}\.rustup）。
/// 保证缓存路径区与「修复环境变量」校准的环境变量值保持一致（唯一真源）。
fn resolve_conflict_cache_path(def: &crate::commands::project::types::ConflictManagerDef) -> String {
    if let Some(ref env_var) = def.cache_env_var {
        if let Some((val, _)) = get_registry_env_any(env_var) {
            if !val.trim().is_empty() {
                return val;
            }
        }
    }
    if let Some(ref raw_path) = def.cache_default_path {
        return expand_home(raw_path);
    }
    String::new()
}

#[tauri::command]
pub fn handle_conflict_manager_action(
    app_handle: tauri::AppHandle,
    sdk_id: String,
    manager_id: String,
    action: String,
    target_path: Option<String>,
) -> Result<(), String> {
    let project = registry::find_by_id(&sdk_id)
        .ok_or_else(|| format!("未找到项目: {}", sdk_id))?;
        
    let def = project.conflict_managers.iter().find(|m| m.id == manager_id)
        .ok_or_else(|| format!("在项目 {} 下未找到冲突管理器: {}", sdk_id, manager_id))?;
        
    match action.as_str() {
        "clean" => {
            let cache_path = resolve_conflict_cache_path(def);
            if !cache_path.is_empty() && Path::new(&cache_path).exists() {
                clean_pkg_cache_impl(&app_handle, &cache_path)?;
            }
        }
        "migrate" => {
            let new_path = target_path.ok_or_else(|| "迁移操作缺少 target_path 参数".to_string())?;
            let cache_path = resolve_conflict_cache_path(def);
            if cache_path.is_empty() {
                return Err("该冲突管理器未配置缓存路径，无法迁移".to_string());
            }
            migrate_pkg_storage_impl(
                &app_handle,
                &cache_path,
                &new_path,
                "cache",
                false,
            )?;
            // 迁移（Junction 模式）后，把缓存位置环境变量同步到新路径，
            // 使「修复环境变量」之外的路由（环境变量区）也能感知实际缓存位置。
            // 注意：migrate 会先在原路径建 junction 指向 new_path，故把环境变量指向
            // 实际目录 new_path（而非 junction 的原路径），避免环境变量区与实际存储脱节。
            if let Some(ref env_var) = def.cache_env_var {
                set_registry_env(env_var, &new_path)?;
                std::env::set_var(env_var, &new_path);
            }
            broadcast_setting_change();
        }
        "point" => {
            let new_path = target_path.ok_or_else(|| "指向操作缺少 target_path 参数".to_string())?;
            // 仅将缓存位置相关的环境变量重定向到 new_path，而不是全部 env_vars。
            // 例如 rustup 的 env_vars 含 RUSTUP_HOME 与 RUSTUP_TOOLCHAIN，
            // 后者是工具链名而非目录路径，若被写入目录路径会导致 rustup 异常。
            let loc_var = def.cache_env_var.clone()
                .or_else(|| def.env_vars.first().cloned());
            if let Some(var) = loc_var {
                set_registry_env(&var, &new_path)?;
                std::env::set_var(&var, &new_path);
            }
            broadcast_setting_change();
        }
        "disable" => {
            // 1. 先算预案 —— 与 preview_conflict_manager_disable 共用同一份计算，
            //    保证「确认框里列的」就是「实际会改的」。
            let config = crate::commands::config::load_config();
            let delegation = crate::commands::project::scanner::get_project_delegation(
                &config,
                &sdk_id,
                &project,
            );
            let presence = collect_presence(&def.env_vars);
            let user_path = get_registry_env("PATH");
            let system_path = get_system_registry_env("PATH");
            let plan = plan_disable(
                def,
                &delegation.env_vars,
                &presence,
                user_path.as_deref(),
                system_path.as_deref(),
            );

            // 2. 执行前自动创建全量环境备份（用户级 + 系统级）。
            //    失败即中止：这是不可逆操作，宁可不让做，也不能没有退路。
            crate::commands::env::create_env_backup(format!(
                "停用冲突版本管理器 {}（项目 {}）自动备份",
                def.display_name, sdk_id
            ))
            .map_err(|e| {
                format!(
                    "已中止：创建环境变量备份失败（{}）。可先在「设置 → 环境变量备份与还原」手动创建备份后重试。",
                    e
                )
            })?;

            // 3. 擦除环境变量（跳过属于本项目托管变量的项，保护 Kira 自己写入的值）
            for var in &plan.cleared_vars {
                let _ = set_registry_env(var, "");
                std::env::remove_var(var);
                // 尝试系统级清理（非管理员会失败，静默忽略）
                let _ = set_system_registry_env(var, "");
            }
            // 4. 清洗 PATH 变量
            disable_manager_in_path(&def.path_keywords)?;
            broadcast_setting_change();
        }
        _ => return Err(format!("未知的操作类型: {}", action)),
    }
    
    Ok(())
}

/// 从用户级与系统级 PATH 中删除所有命中关键字的条目。
///
/// 匹配规则与 `plan_disable` 完全一致（共用 `matched_path_entries`），
/// 保证预演列出的条目就是这里真正会删掉的条目。
fn disable_manager_in_path(path_keywords: &[String]) -> Result<(), String> {
    // 1. 用户级 PATH
    if let Some(user_path) = get_registry_env("PATH") {
        // 与预演（plan_disable）共用同一套过滤：Kira 托管的条目绝不删除
        let doomed = exclude_managed_entries(matched_path_entries(&user_path, path_keywords));
        if !doomed.is_empty() {
            let doomed_lower: std::collections::HashSet<String> =
                doomed.iter().map(|p| p.to_lowercase()).collect();
            let kept: Vec<String> = std::env::split_paths(&user_path)
                .map(|p| p.to_string_lossy().to_string())
                .filter(|p| !doomed_lower.contains(&p.to_lowercase()))
                .collect();
            let new_path_str = std::env::join_paths(kept.iter().map(Path::new))
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .to_string();
            set_registry_env("PATH", &new_path_str)?;
        }
    }

    // 2. 系统级 PATH（需要管理员权限；写失败静默忽略，不阻断用户级停用）
    if let Some(sys_path) = get_system_registry_env("PATH") {
        let doomed = exclude_managed_entries(matched_path_entries(&sys_path, path_keywords));
        if !doomed.is_empty() {
            let doomed_lower: std::collections::HashSet<String> =
                doomed.iter().map(|p| p.to_lowercase()).collect();
            let kept: Vec<String> = std::env::split_paths(&sys_path)
                .map(|p| p.to_string_lossy().to_string())
                .filter(|p| !doomed_lower.contains(&p.to_lowercase()))
                .collect();
            if let Ok(new_path_str) = std::env::join_paths(kept.iter().map(Path::new)) {
                let _ = set_system_registry_env("PATH", &new_path_str.to_string_lossy());
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn mgr(env_vars: &[&str], keywords: &[&str]) -> ConflictManagerDef {
        ConflictManagerDef {
            id: "rustup".to_string(),
            display_name: "Rustup".to_string(),
            env_vars: env_vars.iter().map(|s| s.to_string()).collect(),
            path_keywords: keywords.iter().map(|s| s.to_string()).collect(),
            exe_name: None,
            cache_default_path: None,
            cache_env_var: None,
        }
    }

    fn presence(items: &[(&str, Option<&str>, Option<&str>)]) -> HashMap<String, VarPresence> {
        items
            .iter()
            .map(|(n, u, s)| {
                (
                    (*n).to_string(),
                    VarPresence {
                        user: u.map(|v| v.to_string()),
                        system: s.map(|v| v.to_string()),
                    },
                )
            })
            .collect()
    }

    #[test]
    fn filter_out_managed_entries_keeps_only_foreign_entries() {
        let managed = vec![
            r"D:\any-versions\caches\cargo\bin".to_string(),
            r"D:\any-versions\sdk\rust\bin\".to_string(), // 结尾反斜杠也要能匹配
        ];
        let matched = vec![
            r"D:\any-versions\caches\CARGO\bin".to_string(), // 大小写不同 → 仍算托管，剔除
            r"D:\any-versions\sdk\rust\bin".to_string(),
            r"C:\Users\me\.cargo\bin".to_string(), // 真正的外部 rustup shim → 保留
        ];
        assert_eq!(
            filter_out_managed_entries(matched, &managed),
            vec![r"C:\Users\me\.cargo\bin".to_string()]
        );
    }

    #[test]
    fn filter_out_managed_entries_is_noop_without_managed() {
        let matched = vec![r"C:\Users\me\.cargo\bin".to_string()];
        assert_eq!(filter_out_managed_entries(matched.clone(), &[]), matched);
    }

    #[test]
    fn matched_path_entries_matches_substring_case_insensitively() {
        // 注意第二条：`bin-mirror` 这种用户自建目录也会被命中——
        // 这正是要在确认框里逐条列出来给用户看的原因。
        let path = r"C:\Windows;C:\Users\me\.cargo\bin;D:\tools\.CARGO\bin-mirror;D:\other";
        let got = matched_path_entries(path, &[".cargo\\bin".to_string()]);
        assert_eq!(
            got,
            vec![
                r"C:\Users\me\.cargo\bin".to_string(),
                r"D:\tools\.CARGO\bin-mirror".to_string()
            ]
        );
    }

    #[test]
    fn matched_path_entries_is_empty_without_usable_keywords() {
        assert!(matched_path_entries(r"C:\a;C:\b", &[]).is_empty());
        assert!(matched_path_entries(r"C:\a;C:\b", &["   ".to_string()]).is_empty());
        assert!(matched_path_entries("", &["nvm".to_string()]).is_empty());
    }

    #[test]
    fn plan_clears_everything_when_project_not_managed() {
        let m = mgr(&["RUSTUP_HOME", "RUSTUP_TOOLCHAIN"], &[".cargo\\bin"]);
        let p = presence(&[
            ("RUSTUP_HOME", Some(r"D:\x\rustup"), None),
            ("RUSTUP_TOOLCHAIN", None, Some("stable-x86_64-pc-windows-msvc")),
        ]);
        // 未托管 ⇒ delegation.env_vars 为空 ⇒ 全部按"清空"处理
        let plan = plan_disable(&m, &HashSet::new(), &p, Some(r"C:\Users\me\.cargo\bin;D:\keep"), None);

        assert_eq!(plan.cleared_vars, vec!["RUSTUP_HOME", "RUSTUP_TOOLCHAIN"]);
        assert!(plan.kept_managed_vars.is_empty());
        assert_eq!(plan.variables[0].action, "clear");
        assert_eq!(plan.variables[0].level, "user");
        assert_eq!(plan.variables[0].current_value.as_deref(), Some(r"D:\x\rustup"));
        assert_eq!(plan.variables[1].level, "system");
        assert_eq!(plan.user_path_entries, vec![r"C:\Users\me\.cargo\bin"]);
    }

    #[test]
    fn plan_keeps_vars_owned_by_the_managed_project() {
        // rust 的 RUSTUP_HOME 同时属于项目 env_vars（Kira 写入 data_dir\rust\rustup）
        // 与 rustup 管理器的 env_vars：此时必须跳过，否则等于抹掉 Kira 自己设的值。
        let m = mgr(&["RUSTUP_HOME", "RUSTUP_TOOLCHAIN"], &[".cargo\\bin"]);
        let managed: HashSet<String> = ["RUSTUP_HOME".to_string()].into_iter().collect();
        let p = presence(&[
            ("RUSTUP_HOME", Some(r"D:\any-versions\rust\rustup"), Some(r"D:\any-versions\rust\rustup")),
            ("RUSTUP_TOOLCHAIN", None, None),
        ]);
        let plan = plan_disable(&m, &managed, &p, None, None);

        assert_eq!(plan.kept_managed_vars, vec!["RUSTUP_HOME"]);
        assert_eq!(plan.cleared_vars, vec!["RUSTUP_TOOLCHAIN"]);
        assert_eq!(plan.variables[0].action, "keep_managed");
        assert_eq!(plan.variables[0].level, "both");
        assert_eq!(plan.variables[1].action, "clear");
        assert_eq!(plan.variables[1].level, "none");
    }

    #[test]
    fn plan_lists_path_entries_from_both_levels() {
        let m = mgr(&[], &["\\nvm"]);
        let plan = plan_disable(
            &m,
            &HashSet::new(),
            &presence(&[]),
            Some(r"C:\Users\me\AppData\Roaming\nvm;D:\keep"),
            Some(r"C:\ProgramData\nvm;C:\Windows"),
        );
        assert_eq!(plan.manager_display_name, "Rustup");
        assert_eq!(plan.user_path_entries, vec![r"C:\Users\me\AppData\Roaming\nvm"]);
        assert_eq!(plan.system_path_entries, vec![r"C:\ProgramData\nvm"]);
    }
}
