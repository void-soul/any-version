use super::*;

/// 业务选项文件：**跟随数据目录**（它是数据，不是入口配置；config.json 才是入口）。
///
/// 路径解析由无业务依赖的 foundation 提供，不再递归加载业务 Config。
pub(crate) fn settings_file_path() -> PathBuf {
    get_data_dir().join("settings.json")
}

/// 完整配置 → 两个落盘文件的拆分。
pub(super) fn split_config(config: &Config) -> (ConfigPathsFile, SettingsFile) {
    (
        ConfigPathsFile {
            data_dir: config.data_dir.clone(),
            node_projects_dir: config.node_projects_dir.clone(),
        },
        SettingsFile {
            managed_items: config.managed_items.clone(),
            simple_managed_items: config.simple_managed_items.clone(),
            custom_install_paths: config.custom_install_paths.clone(),
            custom_data_paths: config.custom_data_paths.clone(),
            project_menu_configs: config.project_menu_configs.clone(),
            project_delegations: config.project_delegations.clone(),
            active_versions: config.active_versions.clone(),
            github_token: config.github_token.clone(),
            original_envs: config.original_envs.clone(),
            original_paths: config.original_paths.clone(),
            rss_sources: config.rss_sources.clone(),
            rss_source_names: config.rss_source_names.clone(),
            has_run_before: config.has_run_before,
            tray_menu: config.tray_menu.clone(),
            last_servers: config.last_servers.clone(),
            auto_start_services: config.auto_start_services.clone(),
            module_theme_colors: config.module_theme_colors.clone(),
            global_font: config.global_font.clone(),
            custom_font_path: config.custom_font_path.clone(),
            module_order: config.module_order.clone(),
            toolbar_modules: config.toolbar_modules.clone(),
            disabled_modules: config.disabled_modules.clone(),
            background_texture: config.background_texture.clone(),
            language: config.language.clone(),
            motion_preference: config.motion_preference.clone(),
            density: config.density.clone(),
            cert_receiver: config.cert_receiver.clone(),
        },
    )
}

/// 两个落盘文件 → 完整配置（versions_dir/links_dir/sdk_dir 由 fill_legacy_dirs 派生）。
pub(super) fn merge_config(paths: ConfigPathsFile, settings: SettingsFile) -> Config {
    Config {
        baseline: None,
        versions_dir: String::new(),
        links_dir: String::new(),
        data_dir: paths.data_dir,
        sdk_dir: String::new(),
        twoapi_port: default_twoapi_port(),
        cert_receiver: settings.cert_receiver,
        managed_items: settings.managed_items,
        simple_managed_items: settings.simple_managed_items,
        custom_install_paths: settings.custom_install_paths,
        custom_data_paths: settings.custom_data_paths,
        project_menu_configs: settings.project_menu_configs,
        project_delegations: settings.project_delegations,
        active_versions: settings.active_versions,
        github_token: settings.github_token,
        original_envs: settings.original_envs,
        original_paths: settings.original_paths,
        rss_sources: settings.rss_sources,
        rss_source_names: settings.rss_source_names,
        has_run_before: settings.has_run_before,
        tray_menu: settings.tray_menu,
        last_servers: settings.last_servers,
        node_projects_dir: paths.node_projects_dir,
        auto_start_services: settings.auto_start_services,
        module_theme_colors: settings.module_theme_colors,
        global_font: settings.global_font,
        custom_font_path: settings.custom_font_path,
        module_order: settings.module_order,
        toolbar_modules: settings.toolbar_modules,
        disabled_modules: settings.disabled_modules,
        background_texture: settings.background_texture,
        language: settings.language,
        motion_preference: settings.motion_preference,
        density: settings.density,
    }
}

/// 读 settings.json；缺失时把旧版写在 config.json 里的选项一次性搬过来（幂等迁移）。
/// `raw_config_json` 是 config.json 的原文，仅用于这次迁移。
pub(super) fn load_settings_file(raw_config_json: &str) -> SettingsFile {
    let path = settings_file_path();
    if path.is_file() {
        match fs::read_to_string(&path) {
            Ok(data) => match serde_json::from_str::<SettingsFile>(&data) {
                Ok(settings) => return settings,
                Err(e) => {
                    eprintln!("[config] settings.json 解析失败: {}，回退为默认值", e);
                    backup_corrupt_config(&path);
                }
            },
            Err(e) => eprintln!("[config] settings.json 读取失败: {}", e),
        }
        return SettingsFile::default();
    }
    // 迁移：旧版 config.json 里带着全部业务选项，这里照字段名搬进 settings.json。
    // serde 会忽略它不认识的 data_dir 等路径字段。
    let migrated: SettingsFile = serde_json::from_str(raw_config_json).unwrap_or_default();
    if let Ok(data) = serde_json::to_string_pretty(&migrated) {
        if let Err(e) = atomic_write_file(&path, data.as_bytes()) {
            eprintln!("[config] 迁移选项到 settings.json 失败: {}", e);
        } else {
            eprintln!("[config] 已把业务选项从 config.json 迁移到 settings.json");
        }
    }
    migrated
}

/// 数据根目录（可改，空则回退到默认 ~/.any-version）。

/// SDK 目录（合并 versions+links）。始终为 data_dir/sdk（单一路径策略，不单独配置）。

/// SDK 版本库目录：sdk_dir/_versions。

/// SDK 链接（junction 锚点）根目录：即 sdk_dir 本身。

/// 缓存根目录：始终为 `data_dir/caches`。
///
/// 各 SDK 的「缓存型环境变量」（`env_vars.json` 里的 `sub_dir`，如 rust 的
/// CARGO_HOME / RUSTUP_HOME）统一锚定在这里，与各包管理器「缓存目录设置」
/// 写注册表时使用的路径同属一套布局（`caches/<名称>`），
/// 避免出现 `data_dir/<sdk_id>/<sub_dir>` 与 `data_dir/caches/<sub_dir>` 两份缓存。

/// 供前端读取当前数据目录（命令）。
#[tauri::command]
pub fn get_data_dir_cmd() -> String {
    get_data_dir().to_string_lossy().to_string()
}

/// 供前端读取当前 SDK 目录（命令）。
#[tauri::command]
pub fn get_sdk_dir_cmd() -> String {
    get_sdk_dir().to_string_lossy().to_string()
}

/// 服务类 Node 项目存储目录。
/// 优先使用用户在全局设置中配置的 node_projects_dir（可指向非系统盘节约 C 盘空间）；
/// 未配置/为空时回退到 data_dir/node-projects。

/// 读取 config.json（不存在/损坏时返回 None，损坏文件会被备份）。
pub fn read_config_file() -> Option<Config> {
    let base_dir = get_base_dir();
    let config_path = base_dir.join("config.json");
    if !config_path.exists() {
        return None;
    }
    match fs::read_to_string(&config_path) {
        Ok(data) => match serde_json::from_str::<ConfigPathsFile>(&data) {
            Ok(paths) => {
                let settings = load_settings_file(&data);
                let mut config = merge_config(paths, settings);
                fill_legacy_dirs(&mut config, &base_dir);
                config.baseline = serde_json::to_value(&config).ok();
                Some(config)
            }
            Err(e) => {
                eprintln!("[config] config.json 解析失败: {}，已备份损坏文件后重建", e);
                backup_corrupt_config(&config_path);
                None
            }
        },
        Err(e) => {
            eprintln!("[config] config.json 读取失败: {}，已备份损坏文件后重建", e);
            backup_corrupt_config(&config_path);
            None
        }
    }
}

pub fn load_config() -> Config {
    let base_dir = get_base_dir();
    if let Some(config) = read_config_file() {
        return config;
    }
    let mut default_config = default_config();
    let _ = fs::create_dir_all(&base_dir);
    let _ = save_config(&default_config);
    default_config.baseline = serde_json::to_value(&default_config).ok();
    default_config
}

/// 构建默认配置（首次启动/配置损坏重建时使用）。
pub(super) fn default_config() -> Config {
    let base_dir = get_base_dir();
    Config {
        baseline: None,
        versions_dir: base_dir.join("versions").to_string_lossy().to_string(),
        links_dir: base_dir.join("links").to_string_lossy().to_string(),
        data_dir: base_dir.to_string_lossy().to_string(),
        sdk_dir: base_dir.join("sdk").to_string_lossy().to_string(),
        twoapi_port: default_twoapi_port(),
        cert_receiver: None,
        managed_items: std::collections::HashSet::new(),
        simple_managed_items: std::collections::HashSet::new(),
        github_token: None,
        custom_install_paths: std::collections::HashMap::new(),
        custom_data_paths: std::collections::HashMap::new(),
        project_menu_configs: std::collections::HashMap::new(),
        project_delegations: std::collections::HashMap::new(),
        active_versions: std::collections::HashMap::new(),
        original_envs: std::collections::HashMap::new(),
        original_paths: std::collections::HashMap::new(),
        rss_sources: vec![
            "https://36kr.com/feed".to_string(),
            "https://www.ruanyifeng.com/blog/atom.xml".to_string(),
        ],
        rss_source_names: default_rss_source_names(),
        has_run_before: false,
        tray_menu: TrayMenuConfig::default(),
        last_servers: LastServerConfig::default(),
        // 留空 = 派生为 data_dir/node-projects（改数据目录时自动跟着走，不会变成过期值）
        node_projects_dir: String::new(),
        auto_start_services: std::collections::HashSet::new(),
        module_theme_colors: std::collections::HashMap::new(),
        global_font: String::new(),
        custom_font_path: String::new(),
        module_order: Vec::new(),
        toolbar_modules: Vec::new(),
        disabled_modules: Vec::new(),
        background_texture: String::new(),
        language: String::new(),
        // 空 / 非 "always" 都按「跟随系统」处理
        motion_preference: String::new(),
        // 空 / 非 "compact" 都按「舒适」处理
        density: String::new(),
    }
}

/// 用 sdk_dir 派生填充已废弃的 versions_dir/links_dir 字段（最小侵入兼容方案）。
/// 供所有现有 `config.versions_dir`/`config.links_dir` 引用读取到正确路径。
pub(super) fn fill_legacy_dirs(config: &mut Config, base_dir: &Path) {
    // 单一路径策略：versions_dir / links_dir 已废弃，始终由 data_dir/sdk 派生，
    // 忽略 config 中任何旧值，确保链接锚点与版本库路径一致。
    let data = if config.data_dir.trim().is_empty() {
        base_dir.to_path_buf()
    } else {
        PathBuf::from(config.data_dir.trim())
    };
    let sdk = data.join("sdk");
    config.versions_dir = sdk.join("_versions").to_string_lossy().to_string();
    config.links_dir = sdk.to_string_lossy().to_string();
    config.sdk_dir = sdk.to_string_lossy().to_string();

    // node_projects_dir 默认就是 data_dir/node-projects：存一份等于默认值的路径没有意义，
    // 而且数据目录一改它就变成过期值。这里统一清掉，让「空」代表派生。
    let derived_node = data.join("node-projects");
    let configured = config.node_projects_dir.trim();
    if !configured.is_empty() && PathBuf::from(configured) == derived_node {
        config.node_projects_dir.clear();
    }
}

/// 把损坏的 config.json 备份为 config.json.corrupt-<unix秒>.bak，避免用户数据直接丢失。
pub(super) fn backup_corrupt_config(config_path: &Path) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let bak = config_path.with_file_name(format!("config.json.corrupt-{}.bak", ts));
    match fs::copy(config_path, &bak) {
        Ok(_) => eprintln!("[config] 已备份损坏配置到 {}", bak.display()),
        Err(e) => eprintln!("[config] 备份损坏配置失败: {}", e),
    }
}

/// 在单锁内执行「读-改-写」，避免多个配置命令并发时丢失更新：
/// 读取最新配置、调用闭包就地修改、原子写回均在同一把锁内完成。
pub fn mutate_config<F>(mutate: F) -> Result<(), String>
where
    F: FnOnce(&mut Config) -> Result<(), String>,
{
    let _guard = CONFIG_SAVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut config = read_config_file().unwrap_or_else(default_config);
    mutate(&mut config)?;
    write_config_unlocked(&config)
}

/// 原子写入 config.json：先写临时文件再 rename 替换，避免写入中途崩溃/断电截断文件。
/// 进程内用互斥锁串行化，防止多个命令并发写交错。
pub fn save_config(config: &Config) -> Result<(), String> {
    let base_dir = get_base_dir();
    if let Err(e) = fs::create_dir_all(&base_dir) {
        return Err(e.to_string());
    }
    let _guard = CONFIG_SAVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Legacy background callers still use load/modify/save. Apply only their changed
    // fields to the latest state while holding the same lock as mutate_config.
    if let Some(baseline) = &config.baseline {
        if let Some(latest) = read_config_file() {
            let mut merged = serde_json::to_value(latest).map_err(|e| e.to_string())?;
            let desired = serde_json::to_value(config).map_err(|e| e.to_string())?;
            merge_config_patch(&mut merged, baseline, &desired, "");
            let merged: Config = serde_json::from_value(merged).map_err(|e| e.to_string())?;
            return write_config_unlocked(&merged);
        }
    }
    write_config_unlocked(config)
}

/// A three-way patch preserves unrelated fields and independently changed map keys.
pub(super) fn merge_config_patch(
    latest: &mut serde_json::Value,
    baseline: &serde_json::Value,
    desired: &serde_json::Value,
    field: &str,
) {
    if baseline == desired {
        return;
    }
    if let (Some(old), Some(new)) = (baseline.as_object(), desired.as_object()) {
        if !latest.is_object() {
            *latest = serde_json::json!({});
        }
        let current = latest.as_object_mut().unwrap();
        for key in old
            .keys()
            .chain(new.keys())
            .collect::<std::collections::BTreeSet<_>>()
        {
            match (old.get(key), new.get(key)) {
                (Some(a), Some(b)) if a == b => {}
                (Some(_), None) => {
                    current.remove(key);
                }
                (None, Some(b)) => {
                    current.insert(key.clone(), b.clone());
                }
                (Some(a), Some(b)) => merge_config_patch(
                    current
                        .entry(key.clone())
                        .or_insert(serde_json::Value::Null),
                    a,
                    b,
                    key,
                ),
                _ => {}
            }
        }
    } else if matches!(
        field,
        "managed_items" | "simple_managed_items" | "auto_start_services"
    ) {
        if let (Some(old), Some(new), Some(current)) = (
            baseline.as_array(),
            desired.as_array(),
            latest.as_array_mut(),
        ) {
            current.retain(|v| !old.contains(v) || new.contains(v));
            for v in new {
                if !old.contains(v) && !current.contains(v) {
                    current.push(v.clone());
                }
            }
        } else {
            *latest = desired.clone();
        }
    } else {
        *latest = desired.clone();
    }
}

/// 原子写入文件（tmp + rename），崩溃时不损坏原文件。
/// rename 失败（如被其他进程占用）时退回直接写，并清理临时文件。

/// 写入已由调用方持有锁时的落盘实现（tmp + rename 原子替换）。
/// 拆成两份写：config.json（路径）+ settings.json（其余选项）。
pub(super) fn write_config_unlocked(config: &Config) -> Result<(), String> {
    let base_dir = get_base_dir();
    let (paths, settings) = split_config(config);
    let paths_data = serde_json::to_string_pretty(&paths).map_err(|e| e.to_string())?;
    let settings_data = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    atomic_write_file(&base_dir.join("config.json"), paths_data.as_bytes())?;
    atomic_write_file(&settings_file_path(), settings_data.as_bytes())?;
    Ok(())
}
