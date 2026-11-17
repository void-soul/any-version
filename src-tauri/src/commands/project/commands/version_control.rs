//! 「只开版本控制」这一个开关的窄命令。
//!
//! 为什么需要它：`delegation.version_control` 是**持久化**的。历史上以「简单托管」
//! 方式托管过的项目（或托管时该选项未勾选），这个值会一直是 false —— 界面于是只给
//! 「手动指定目录」，即使项目本身配好了在线版本来源也永远回不来（Redis 就是这类）。
//!
//! 这里只改这一个布尔值，不碰环境变量 / PATH / 备份，避免重复托管带来的副作用。

/// 开启「版本控制与下载」。项目必须真的配了在线版本来源，否则没有意义。
#[tauri::command]
pub fn project_enable_version_control(id: String) -> Result<(), String> {
    let def = crate::commands::project::registry::find_by_id(&id)
        .ok_or_else(|| format!("未找到项目: {}", id))?;
    if !crate::commands::project::online_install::supports_online_install(&def) {
        return Err(format!(
            "{} 未配置在线版本来源，无法开启下载（只能手动指定本地目录）",
            def.display_name
        ));
    }
    crate::commands::config::mutate_config(|config| {
        let mut delegation = config
            .project_delegations
            .get(&id)
            .cloned()
            .unwrap_or_default();
        delegation.version_control = true;
        delegation.create_symlink = true;
        config.project_delegations.insert(id.clone(), delegation);
        Ok(())
    })
}
