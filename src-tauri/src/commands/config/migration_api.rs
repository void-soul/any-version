use super::*;

/// 路径迁移结果（供前端展示）
#[derive(Serialize, Clone, Debug)]
pub struct MigrateResult {
    pub moved_versions: bool,
    pub moved_links: bool,
    pub recreated_junctions: Vec<String>,
    pub updated_env_vars: Vec<String>,
    pub updated_path_entries: Vec<String>,
    pub errors: Vec<String>,
    /// 迁移后仍在原位置的旧目录（供前端询问是否删除）
    pub old_dirs_remain: Vec<String>,
}

/// 迁移进度事件（通过 Tauri emit 发送）
#[derive(Serialize, Clone, Debug)]
pub struct MigrateProgress {
    pub stage: String, // 当前阶段描述
    pub current: usize,
    pub total: usize,
    pub file_name: String, // 当前正在处理的文件/目录名
}

/// 安排存储路径迁移：真正复制、链接和环境变量切换在下次安全启动时执行。
/// 返回值只表示计划是否成功创建，不表示数据已经迁移完成。
/// Compatibility entry: data migrations are applied only at the next clean startup.
pub fn do_migrate_storage(
    _old_versions_dir: &str,
    _new_versions_dir: &str,
    _old_links_dir: &str,
    new_links_dir: &str,
    config: &Config,
    _app_handle: Option<&tauri::AppHandle>,
) -> MigrateResult {
    let destination = Path::new(new_links_dir)
        .parent()
        .unwrap_or_else(|| Path::new(new_links_dir));
    let errors = crate::commands::storage_migration::schedule_migration(
        Path::new(&config.data_dir),
        destination,
    )
    .err()
    .into_iter()
    .collect();
    MigrateResult {
        moved_versions: false,
        moved_links: false,
        recreated_junctions: Vec::new(),
        updated_env_vars: Vec::new(),
        updated_path_entries: Vec::new(),
        errors,
        old_dirs_remain: Vec::new(),
    }
}

/// 删除旧的存储目录（迁移后调用）
#[tauri::command]
pub fn delete_old_storage_dirs(dirs: Vec<String>) -> Result<Vec<String>, String> {
    let mut deleted = Vec::new();
    let mut errors = Vec::new();
    for dir in &dirs {
        let path = Path::new(dir);
        if path.exists() {
            match fs::remove_dir_all(path) {
                Ok(()) => deleted.push(dir.clone()),
                Err(e) => errors.push(format!("删除 {} 失败: {}", dir, e)),
            }
        } else {
            deleted.push(format!("{} (已不存在)", dir));
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    Ok(deleted)
}

#[tauri::command]
pub fn get_config() -> Result<Config, String> {
    Ok(load_config())
}

/// 获取应用版本号（从 Cargo.toml）
#[tauri::command]
pub fn get_app_version() -> Result<String, String> {
    Ok(env!("CARGO_PKG_VERSION").to_string())
}

/// 更新配置并自动迁移存储路径。
/// 只接收 `data_dir`（单一路径策略）：SDK、Node 服务项目、证书、数据库等
/// 全部作为 data_dir 的子目录自动派生。
/// 修改 data_dir 时自动执行：
///   1. 将 base_dir 下所有可变数据（含 sdk 版本库/链接、node-projects、certs、数据库等）迁移到新 data_dir
///   2. 重建 junction 指向、更新 PATH 环境变量与 *_HOME 变量
#[tauri::command]
pub fn update_config(
    app_handle: tauri::AppHandle,
    data_dir: String,
) -> Result<MigrateResult, String> {
    let source = get_data_dir();
    let destination = if data_dir.trim().is_empty() {
        get_base_dir()
    } else {
        PathBuf::from(data_dir.trim())
    };
    crate::commands::storage_migration::schedule_migration(&source, &destination)?;
    let _ = app_handle.emit(
        "migrate-progress",
        MigrateProgress {
            stage: "Queued for restart".into(),
            current: 1,
            total: 1,
            file_name: destination.to_string_lossy().into_owned(),
        },
    );
    Ok(MigrateResult {
        moved_versions: false,
        moved_links: false,
        recreated_junctions: Vec::new(),
        updated_env_vars: Vec::new(),
        updated_path_entries: Vec::new(),
        errors: Vec::new(),
        old_dirs_remain: Vec::new(),
    })
}
