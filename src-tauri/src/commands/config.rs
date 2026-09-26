use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use serde::{Serialize, Deserialize};
use tauri::Emitter;

/// 串行化 config.json 的写入，避免并发命令交错写坏文件。
static CONFIG_SAVE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProjectMenuConfig {
    #[serde(default = "default_true")]
    pub show_version: bool,
    #[serde(default = "default_true")]
    pub show_service: bool,
}

fn default_true() -> bool {
    true
}

/// 托盘右键菜单配置。
///
/// - `enabled` 是全局总开关（「设置 → 托盘右键」），控制托盘图标是否挂载右键菜单。
/// - `show_mihomo` 决定是否显示 Mihomo 项，其设置入口已迁至**代理模块**的设置弹窗。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct TrayMenuConfig {
    /// 是否启用托盘右键菜单（关闭后托盘图标不挂载菜单）
    pub enabled: bool,
    /// 是否在托盘菜单中显示 Mihomo 项
    pub show_mihomo: bool,
}

impl Default for TrayMenuConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            show_mihomo: true,
        }
    }
}

/// 托盘「启动上次配置」所需的记忆
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct LastServerConfig {
    /// HTTP 静态服务：最近一次的目录与端口
    #[serde(default)]
    pub http: Option<serde_json::Value>,
    /// RTSP 推流：最近一次的完整配置
    #[serde(default)]
    pub rtsp: Option<serde_json::Value>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ProjectDelegation {
    #[serde(default)]
    pub env_vars: std::collections::HashSet<String>,
    #[serde(default)]
    pub path_vars: std::collections::HashSet<String>,
    #[serde(default = "default_false")]
    pub version_control: bool,
    #[serde(default = "default_false")]
    pub create_symlink: bool,
    #[serde(default = "default_false")]
    pub manage_install_dir: bool,
    #[serde(default = "default_false")]
    pub manage_data_dir: bool,
    #[serde(default = "default_false")]
    pub manage_cache_dir: bool,
    #[serde(default)]
    pub manage_optional_tools: std::collections::HashSet<String>,
}

fn default_false() -> bool {
    false
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    /// 已废弃：SDK 版本库目录（由 `sdk_dir/_versions` 取代）。保留读取兼容。
    #[serde(default)]
    pub versions_dir: String,
    /// 已废弃：SDK 链接目录（由 `sdk_dir` 取代）。保留读取兼容。
    #[serde(default)]
    pub links_dir: String,
    /// 数据根目录（可改到非系统盘），承载所有可变数据：sdk、node-projects、backup、certs、version_cache、数据库等。
    #[serde(default)]
    pub data_dir: String,
    /// SDK 目录（合并 versions+links）。默认 data_dir/sdk，内部用 `_versions` 存版本库、sdk 根放 junction 锚点。
    #[serde(default)]
    pub sdk_dir: String,
    #[serde(default)]
    pub managed_items: std::collections::HashSet<String>,
    #[serde(default)]
    pub simple_managed_items: std::collections::HashSet<String>,
    #[serde(default)]
    pub custom_install_paths: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub custom_data_paths: std::collections::HashMap<String, std::collections::HashMap<String, String>>,
    #[serde(default)]
    pub project_menu_configs: std::collections::HashMap<String, ProjectMenuConfig>,
    #[serde(default)]
    pub project_delegations: std::collections::HashMap<String, ProjectDelegation>,
    #[serde(default)]
    pub active_versions: std::collections::HashMap<String, String>,
    /// GitHub API Token：SDK 远程版本列表走 api.github.com 时自动附加（限流配额 60→5000 次/小时）。
    /// 留空则回退读取 GITHUB_TOKEN / GH_TOKEN 环境变量。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_token: Option<String>,
    /// 托管前各项目环境变量的原始值备份。
    /// 结构：project_id -> (变量名 -> 原始值)。按项目隔离，避免多项目同名的变量互相覆盖原始值。
    #[serde(default, deserialize_with = "deserialize_original_envs")]
    pub original_envs: std::collections::HashMap<String, std::collections::HashMap<String, String>>,
    #[serde(default)]
    pub original_paths: std::collections::HashMap<String, Vec<String>>,
    #[serde(default)]
    pub rss_sources: Vec<String>,
    /// RSS 订阅源自定义名称：url -> name。为空时前端回退使用 feed 标题。
    #[serde(default)]
    pub rss_source_names: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub has_run_before: bool,
    /// 托盘右键菜单配置
    #[serde(default)]
    pub tray_menu: TrayMenuConfig,
    /// 托盘「启动上次配置」记忆
    #[serde(default)]
    pub last_servers: LastServerConfig,
    /// 服务类 Node 项目的存储目录（安装 clone / pnpm install 的目标根目录）。
    /// 默认 ~/.any-version/node-projects，可改到其他盘以节约 C 盘空间。
    #[serde(default)]
    pub node_projects_dir: String,
    /// 软件启动时自动拉起的服务 ID 列表，如 ["mihomo", "rtsp", "mysql", "redis", "mongodb"]
    #[serde(default)]
    pub auto_start_services: std::collections::HashSet<String>,
    /// 顶级模块主题色：模块 id -> 主题色 hex（如 launcher -> "#8b5cf6"）
    #[serde(default)]
    pub module_theme_colors: std::collections::HashMap<String, String>,
    /// 全局字体（CSS font-family 名称），空则使用默认字体
    #[serde(default)]
    pub global_font: String,
    /// 导入的自定义字体文件路径（用于 @font-face 注册），空表示未导入
    #[serde(default)]
    pub custom_font_path: String,
    /// 顶级模块自定义顺序（模块 id 列表）。为空时前端使用默认顺序。
    #[serde(default)]
    pub module_order: Vec<String>,
    /// 置顶显示在顶栏的模块 id 列表（其余模块收进「更多」）。
    /// 为空时前端按默认布局（全部顶级模块在顶栏，子工具在「更多」）。
    #[serde(default)]
    pub toolbar_modules: Vec<String>,
    /// 被用户禁用的模块 id 列表（导航隐藏且不渲染，后端命令仍注册）。
    #[serde(default)]
    pub disabled_modules: Vec<String>,
    /// 全局背景底图纹理（grid / dots / scanline / aurora / solid）。空 = 默认网格。
    #[serde(default)]
    pub background_texture: String,
    /// 界面语言（"zh" / "en"），空 = 跟随系统。
    #[serde(default)]
    pub language: String,
}

// ─── 落盘形态：config.json 只存数据路径，其余选项存数据目录下的 settings.json ───
//
// 以前所有东西都堆在 config.json 里（SDK 托管、RSS 订阅、外观、托盘…），
// 而 config.json 是「数据在哪」的入口，混在一起后：
//   1）改数据目录时整份配置被带着迁移，语义混乱；
//   2）备份/排障时没法只看路径。
// 现在拆分：内存里仍是完整的 `Config`（所有现有读取点不变），
// 落盘时拆成两个文件，读取时再合并回来。
// config.json 固定留在 base_dir（数据入口），settings.json 跟着 data_dir 走。

/// `config.json` 的落盘形态：**只有数据路径**。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct ConfigPathsFile {
    #[serde(default)]
    pub data_dir: String,
    #[serde(default)]
    pub node_projects_dir: String,
}

/// `settings.json` 的落盘形态：除数据路径以外的全部业务选项。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
struct SettingsFile {
    pub managed_items: std::collections::HashSet<String>,
    pub simple_managed_items: std::collections::HashSet<String>,
    pub custom_install_paths: std::collections::HashMap<String, String>,
    pub custom_data_paths: std::collections::HashMap<String, std::collections::HashMap<String, String>>,
    pub project_menu_configs: std::collections::HashMap<String, ProjectMenuConfig>,
    pub project_delegations: std::collections::HashMap<String, ProjectDelegation>,
    pub active_versions: std::collections::HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_token: Option<String>,
    #[serde(deserialize_with = "deserialize_original_envs")]
    pub original_envs: std::collections::HashMap<String, std::collections::HashMap<String, String>>,
    pub original_paths: std::collections::HashMap<String, Vec<String>>,
    pub rss_sources: Vec<String>,
    pub rss_source_names: std::collections::HashMap<String, String>,
    pub has_run_before: bool,
    pub tray_menu: TrayMenuConfig,
    pub last_servers: LastServerConfig,
    pub auto_start_services: std::collections::HashSet<String>,
    pub module_theme_colors: std::collections::HashMap<String, String>,
    pub global_font: String,
    pub custom_font_path: String,
    pub module_order: Vec<String>,
    pub toolbar_modules: Vec<String>,
    pub disabled_modules: Vec<String>,
    pub background_texture: String,
    pub language: String,
}

/// 业务选项文件：**跟随数据目录**（它是数据，不是入口配置；config.json 才是入口）。
///
/// 注意这里不能调 `get_data_dir()`——那条链路是 `get_data_dir → load_config →
/// load_settings_file → settings_file_path`，会无限递归。所以只从 config.json
/// 里取 `data_dir` 字段（读不到就回退 base_dir）。
pub(crate) fn settings_file_path() -> PathBuf {
    let base_dir = get_base_dir();
    let data_dir = fs::read_to_string(base_dir.join("config.json"))
        .ok()
        .and_then(|raw| serde_json::from_str::<ConfigPathsFile>(&raw).ok())
        .map(|p| p.data_dir.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or(base_dir);
    data_dir.join("settings.json")
}

/// 完整配置 → 两个落盘文件的拆分。
fn split_config(config: &Config) -> (ConfigPathsFile, SettingsFile) {
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
        },
    )
}

/// 两个落盘文件 → 完整配置（versions_dir/links_dir/sdk_dir 由 fill_legacy_dirs 派生）。
fn merge_config(paths: ConfigPathsFile, settings: SettingsFile) -> Config {
    Config {
        versions_dir: String::new(),
        links_dir: String::new(),
        data_dir: paths.data_dir,
        sdk_dir: String::new(),
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
    }
}

/// 读 settings.json；缺失时把旧版写在 config.json 里的选项一次性搬过来（幂等迁移）。
/// `raw_config_json` 是 config.json 的原文，仅用于这次迁移。
fn load_settings_file(raw_config_json: &str) -> SettingsFile {
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

pub fn get_base_dir() -> PathBuf {
    let user_profile = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| {
            let drive = std::env::var("HOMEDRIVE").unwrap_or_default();
            let path = std::env::var("HOMEPATH").unwrap_or_default();
            if drive.is_empty() && path.is_empty() {
                "C:\\any-version".to_string()
            } else {
                format!("{}{}", drive, path)
            }
        });
    let mut path = PathBuf::from(user_profile);
    if path.as_os_str().is_empty() || path == PathBuf::from("C:\\any-version") {
        PathBuf::from("C:\\any-version")
    } else {
        path.push(".any-version");
        path
    }
}

/// 数据根目录（可改，空则回退到默认 ~/.any-version）。
pub fn get_data_dir() -> PathBuf {
    let base_dir = get_base_dir();
    let configured = load_config().data_dir;
    let trimmed = configured.trim();
    if trimmed.is_empty() {
        base_dir
    } else {
        PathBuf::from(trimmed)
    }
}

/// SDK 目录（合并 versions+links）。始终为 data_dir/sdk（单一路径策略，不单独配置）。
pub fn get_sdk_dir() -> PathBuf {
    get_data_dir().join("sdk")
}

/// SDK 版本库目录：sdk_dir/_versions。
pub fn get_sdk_versions_dir() -> PathBuf {
    get_sdk_dir().join("_versions")
}

/// SDK 链接（junction 锚点）根目录：即 sdk_dir 本身。
pub fn get_sdk_link_dir() -> PathBuf {
    get_sdk_dir()
}

/// 缓存根目录：始终为 `data_dir/caches`。
///
/// 各 SDK 的「缓存型环境变量」（`env_vars.json` 里的 `sub_dir`，如 rust 的
/// CARGO_HOME / RUSTUP_HOME）统一锚定在这里，与各包管理器「缓存目录设置」
/// 写注册表时使用的路径同属一套布局（`caches/<名称>`），
/// 避免出现 `data_dir/<sdk_id>/<sub_dir>` 与 `data_dir/caches/<sub_dir>` 两份缓存。
pub fn get_cache_root() -> PathBuf {
    get_data_dir().join("caches")
}

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
pub fn get_node_projects_dir() -> PathBuf {
    let cfg = load_config();
    let dir = cfg.node_projects_dir.trim();
    if !dir.is_empty() {
        let p = PathBuf::from(dir);
        // 若是相对路径，则锚定到数据根目录，避免散落到工作目录
        if p.is_absolute() {
            return p;
        }
        return get_data_dir().join(p);
    }
    get_data_dir().join("node-projects")
}

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
    let default_config = default_config();
    let _ = fs::create_dir_all(&base_dir);
    let _ = save_config(&default_config);
    default_config
}

/// 构建默认配置（首次启动/配置损坏重建时使用）。
fn default_config() -> Config {
    let base_dir = get_base_dir();
    Config {
        versions_dir: base_dir.join("versions").to_string_lossy().to_string(),
        links_dir: base_dir.join("links").to_string_lossy().to_string(),
        data_dir: base_dir.to_string_lossy().to_string(),
        sdk_dir: base_dir.join("sdk").to_string_lossy().to_string(),
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
        node_projects_dir: base_dir.join("node-projects").to_string_lossy().to_string(),
        auto_start_services: std::collections::HashSet::new(),
        module_theme_colors: std::collections::HashMap::new(),
        global_font: String::new(),
        custom_font_path: String::new(),
        module_order: Vec::new(),
        toolbar_modules: Vec::new(),
        disabled_modules: Vec::new(),
        background_texture: String::new(),
        language: String::new(),
    }
}

/// 用 sdk_dir 派生填充已废弃的 versions_dir/links_dir 字段（最小侵入兼容方案）。
/// 供所有现有 `config.versions_dir`/`config.links_dir` 引用读取到正确路径。
fn fill_legacy_dirs(config: &mut Config, base_dir: &Path) {
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
}

/// 把损坏的 config.json 备份为 config.json.corrupt-<unix秒>.bak，避免用户数据直接丢失。
fn backup_corrupt_config(config_path: &Path) {
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
    write_config_unlocked(config)
}

/// 原子写入文件（tmp + rename），崩溃时不损坏原文件。
/// rename 失败（如被其他进程占用）时退回直接写，并清理临时文件。
pub(crate) fn atomic_write_file(path: &Path, data: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let file_name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let tmp_path = path.with_file_name(format!("{}.tmp", file_name));
    fs::write(&tmp_path, data).map_err(|e| format!("写入临时文件失败: {}", e))?;
    match fs::rename(&tmp_path, path) {
        Ok(_) => Ok(()),
        Err(_) => {
            fs::write(path, data).map_err(|e| format!("写入文件失败: {}", e))?;
            let _ = fs::remove_file(&tmp_path);
            Ok(())
        }
    }
}

/// 写入已由调用方持有锁时的落盘实现（tmp + rename 原子替换）。
/// 拆成两份写：config.json（路径）+ settings.json（其余选项）。
fn write_config_unlocked(config: &Config) -> Result<(), String> {
    let base_dir = get_base_dir();
    let (paths, settings) = split_config(config);
    let paths_data = serde_json::to_string_pretty(&paths).map_err(|e| e.to_string())?;
    let settings_data = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    atomic_write_file(&base_dir.join("config.json"), paths_data.as_bytes())?;
    atomic_write_file(&settings_file_path(), settings_data.as_bytes())?;
    Ok(())
}

/// 记录托盘「启动上次配置」所需的最近一次服务参数
pub fn remember_last_server(kind: &str, value: serde_json::Value) {
    let _ = mutate_config(|config| {
        match kind {
            "http" => config.last_servers.http = Some(value.clone()),
            "rtsp" => config.last_servers.rtsp = Some(value.clone()),
            _ => return Ok(()),
        }
        Ok(())
    });
}

#[tauri::command]
pub fn get_tray_menu_config() -> TrayMenuConfig {
    load_config().tray_menu
}

#[tauri::command]
pub fn set_tray_menu_config(app: tauri::AppHandle, value: TrayMenuConfig) -> Result<(), String> {
    mutate_config(|config| {
        config.tray_menu = value.clone();
        Ok(())
    })?;
    crate::tray::rebuild_tray_menu(&app).map_err(|e| e.to_string())
}

// ─── Backup storage (separate file) ───

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct BackupStore {
    /// project_id -> env var name -> original value
    pub env_vars: std::collections::HashMap<String, std::collections::HashMap<String, String>>,
    /// project_id -> list of PATH entries that were removed
    pub path_entries: std::collections::HashMap<String, Vec<String>>,
}

fn backup_path() -> PathBuf {
    get_data_dir().join("backups.json")
}

pub fn load_backups() -> BackupStore {
    let path = backup_path();
    if path.exists() {
        if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(store) = serde_json::from_str::<BackupStore>(&data) {
                return store;
            }
        }
    }
    BackupStore::default()
}

pub fn save_backups(store: &BackupStore) -> Result<(), String> {
    let path = backup_path();
    let data = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
    fs::write(path, data).map_err(|e| e.to_string())?;
    Ok(())
}

/// 兼容反序列化 original_envs：同时接受旧版扁平结构 `{变量名: 值}` 与新版按项目隔离的
/// `{project_id: {变量名: 值}}`。旧版扁平数据归入 "__legacy__" 项目，避免多项目同名的原始值丢失。
fn deserialize_original_envs<'de, D>(
    deserializer: D,
) -> Result<std::collections::HashMap<String, std::collections::HashMap<String, String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let raw = serde_json::Value::deserialize(deserializer)?;
    let mut out: std::collections::HashMap<String, std::collections::HashMap<String, String>> =
        std::collections::HashMap::new();
    if let serde_json::Value::Object(map) = raw {
        for (k, v) in map {
            match v {
                serde_json::Value::String(s) => {
                    // 旧版扁平格式：{变量名: 值}
                    out.entry("__legacy__".to_string())
                        .or_default()
                        .insert(k, s);
                }
                serde_json::Value::Object(nested) => {
                    // 新版：{project_id: {变量名: 值}}
                    let inner = nested
                        .into_iter()
                        .map(|(nk, nv)| {
                            (nk, nv.as_str().unwrap_or_default().to_string())
                        })
                        .collect();
                    out.insert(k, inner);
                }
                _ => {}
            }
        }
    }
    Ok(out)
}

/// Migrate old backups from config.json into backups.json (one-time)
pub fn migrate_backups_from_config(config: &mut Config) {
    if !config.original_envs.is_empty() || !config.original_paths.is_empty() {
        let mut store = load_backups();
        for (project_id, vars) in config.original_envs.drain() {
            for (k, v) in vars {
                store.env_vars.entry(project_id.clone()).or_default().insert(k, v);
            }
        }
        for (k, v) in config.original_paths.drain() {
            store.path_entries.entry(k).or_default().extend(v);
        }
        let _ = save_backups(&store);
        let _ = save_config(config);
    }
}

fn normalize(path: &str) -> String {
    let p = path.trim_end_matches('\\').trim_end_matches('/');
    p.to_lowercase()
}

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
    pub stage: String,       // 当前阶段描述
    pub current: usize,
    pub total: usize,
    pub file_name: String,   // 当前正在处理的文件/目录名
}

/// 执行存储路径迁移：移动文件、更新 junction、更新环境变量/PATH
/// app_handle 用于发射进度事件（可选，传 None 则跳过）
pub fn do_migrate_storage(
    old_versions_dir: &str,
    new_versions_dir: &str,
    old_links_dir: &str,
    new_links_dir: &str,
    config: &Config,
    app_handle: Option<&tauri::AppHandle>,
) -> MigrateResult {
    let emit_progress = |stage: &str, current: usize, total: usize, file_name: &str| {
        if let Some(handle) = app_handle {
            let _ = handle.emit("migrate-progress", MigrateProgress {
                stage: stage.to_string(),
                current,
                total,
                file_name: file_name.to_string(),
            });
        }
    };

    let mut result = MigrateResult {
        moved_versions: false,
        moved_links: false,
        recreated_junctions: Vec::new(),
        updated_env_vars: Vec::new(),
        updated_path_entries: Vec::new(),
        errors: Vec::new(),
        old_dirs_remain: Vec::new(),
    };

    let fully_managed_items: Vec<String> = config.managed_items.iter()
        .filter_map(|item_id| {
            if config.simple_managed_items.contains(item_id) {
                return None;
            }
            match crate::commands::project::registry::find_by_id(item_id) {
                Some(def) if !def.simple_mode => Some(item_id.clone()),
                _ => None,
            }
        })
        .collect();

    let versions_changed = normalize(old_versions_dir) != normalize(new_versions_dir);
    let links_changed = normalize(old_links_dir) != normalize(new_links_dir);

    if !versions_changed && !links_changed {
        return result;
    }

    // ── 1. 迁移 links_dir ──
    if links_changed {
        let old_links = Path::new(old_links_dir);
        let new_links = Path::new(new_links_dir);

        if old_links.exists() {
            if let Err(e) = fs::create_dir_all(new_links) {
                result.errors.push(format!("创建新 links 目录失败: {}", e));
            } else {
                // 收集条目列表
                let entries: Vec<_> = if let Ok(rd) = fs::read_dir(old_links) {
                    rd.filter_map(|e| e.ok())
                        .filter(|e| e.path().is_dir() || e.path().is_symlink())
                        .collect()
                } else {
                    Vec::new()
                };
                let total = entries.len();

                // 移动 junction 目录
                for (i, entry) in entries.iter().enumerate() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy().to_string();
                    emit_progress("移动链接目录", i + 1, total, &name_str);

                    let old_path = old_links.join(&name);
                    let new_path = new_links.join(&name);
                    if new_path.exists() {
                        let _ = fs::remove_dir_all(&new_path);
                    }
                    if fs::rename(&old_path, &new_path).is_err() {
                        if let Err(e) = crate::commands::cache::copy_dir_all_with_progress(
                            &old_path, &new_path, app_handle,
                        ) {
                            result.errors.push(format!("复制链接目录 {:?} 失败: {}", e, name_str));
                        } else {
                            let _ = fs::remove_dir_all(&old_path);
                        }
                    }
                }
                result.moved_links = true;

                // 重建所有 junction
                let managed_vec: Vec<_> = fully_managed_items.iter().collect();
                let total_j = managed_vec.len();
                for (i, item_id) in managed_vec.iter().enumerate() {
                    emit_progress("重建 junction 链接", i + 1, total_j, item_id);
                    let link_path = new_links.join(item_id);
                    if let Ok(canonical) = fs::canonicalize(&link_path) {
                        let target_str = canonical.to_string_lossy().to_string()
                            .trim_start_matches(r"\\?\").to_string();
                        let new_target = if versions_changed {
                            let old_ver_prefix = format!("{}\\{}", normalize(old_versions_dir), item_id);
                            let target_lower = target_str.to_lowercase();
                            if let Some(rel_pos) = target_lower.find(&old_ver_prefix) {
                                let rel = &target_str[rel_pos + old_ver_prefix.len()..];
                                let rel = rel.trim_start_matches('\\').trim_start_matches('/');
                                format!("{}\\{}\\{}", new_versions_dir, item_id, rel)
                            } else {
                                target_str.clone()
                            }
                        } else {
                            target_str
                        };
                        let _ = crate::commands::cache::create_junction(&link_path, Path::new(&new_target));
                        result.recreated_junctions.push(item_id.to_string());
                    }
                }

                // 兜底：对 active_versions 中记录的所有 SDK，重建锚点 junction
                result.recreated_junctions.extend(rebuild_sdk_junctions(&config));

                // ── 重写 PATH 环境变量：删除所有旧 links_dir 相关条目，重新写入所有托管 SDK 的路径 ──
                if let Some(user_path) = crate::commands::env::get_registry_env("PATH") {
                    let parts: Vec<String> = std::env::split_paths(&user_path)
                        .map(|p| p.to_string_lossy().to_string())
                        .collect();

                    let old_links_lower = normalize(old_links_dir);
                    let new_links_lower = normalize(new_links_dir);
                    let mut remaining_parts: Vec<String> = Vec::new();
                    let mut removed_count = 0usize;

                    // 删除所有与软件相关的 PATH 条目（旧 links_dir 和新 links_dir 的都删掉，确保干净重写）
                    for part in &parts {
                        let part_lower = part.to_lowercase();
                        if part_lower.contains(&old_links_lower) || part_lower.contains(&new_links_lower) {
                            result.updated_path_entries.push(format!("删除: {}", part));
                            removed_count += 1;
                        } else {
                            remaining_parts.push(part.clone());
                        }
                    }

                    // 重新写入：根据当前完全托管 SDK 列表生成新的 PATH 条目
                    use crate::commands::project::scanner;
                    for item_id in &fully_managed_items {
                        let link_dir = format!("{}\\{}", new_links_dir, item_id);
                        let bin_paths = scanner::get_bin_paths(item_id, &link_dir);
                        for bp in bin_paths.iter().rev() {
                            remaining_parts.insert(0, bp.clone());
                            result.updated_path_entries.push(format!("添加: {}", bp));
                        }
                    }

                    if removed_count > 0 || !fully_managed_items.is_empty() {
                        emit_progress("重写 PATH 环境变量", 1, 1, "");
                        if let Ok(new_path) = std::env::join_paths(remaining_parts.iter().map(Path::new)) {
                            let _ = crate::commands::env::set_registry_env(
                                "PATH",
                                &new_path.to_string_lossy().to_string(),
                            );
                        }
                    }
                }

                // ── 重写注册表环境变量：仅处理完全托管项目，并尊重 env tier ──
                use crate::commands::project::registry;
                use crate::commands::project::types::EnvVarTier;
                for item_id in &fully_managed_items {
                    if let Some(sdk_def) = registry::find_by_id(item_id) {
                        let link_dir = format!("{}\\{}", new_links_dir, item_id);
                        for var_info in &sdk_def.env_vars {
                            if var_info.tier.as_ref().map_or(false, |t| *t == EnvVarTier::Compat) {
                                continue;
                            }
                            if var_info.tier.as_ref().map_or(false, |t| *t == EnvVarTier::Clear) {
                                let _ = crate::commands::env::set_registry_env(&var_info.name, "");
                                result.updated_env_vars.push(format!("{} => <清空>", var_info.name));
                                continue;
                            }
                            let value = crate::commands::env::sdk_env_var_value(item_id, &link_dir, var_info);
                            let _ = crate::commands::env::set_registry_env(&var_info.name, &value);
                            result.updated_env_vars.push(format!("{} => {}", var_info.name, value));
                        }
                    }
                }
            }
        }

        // 记录残留的旧目录
        if old_links.exists() {
            result.old_dirs_remain.push(old_links_dir.to_string());
        }
    }

    // ── 2. 迁移 versions_dir ──
    if versions_changed {
        let old_versions = Path::new(old_versions_dir);
        let new_versions = Path::new(new_versions_dir);

        if old_versions.exists() {
            if let Err(e) = fs::create_dir_all(new_versions) {
                result.errors.push(format!("创建新 versions 目录失败: {}", e));
            } else {
                let entries: Vec<_> = if let Ok(rd) = fs::read_dir(old_versions) {
                    rd.filter_map(|e| e.ok())
                        .filter(|e| e.path().is_dir())
                        .collect()
                } else {
                    Vec::new()
                };
                let total = entries.len();

                for (i, entry) in entries.iter().enumerate() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy().to_string();
                    emit_progress("移动版本目录", i + 1, total, &name_str);

                    let old_path = old_versions.join(&name);
                    let new_path = new_versions.join(&name);
                    if new_path.exists() {
                        let _ = fs::remove_dir_all(&new_path);
                    }
                    if fs::rename(&old_path, &new_path).is_err() {
                        if let Err(e) = crate::commands::cache::copy_dir_all_with_progress(
                            &old_path, &new_path, app_handle,
                        ) {
                            result.errors.push(format!("复制版本目录 {:?} 失败: {}", e, name_str));
                        } else {
                            let _ = fs::remove_dir_all(&old_path);
                        }
                    }
                }
                result.moved_versions = true;

                // 如果 links_dir 没变，需要单独重建 junction
                if !links_changed {
                    let links_path = Path::new(new_links_dir);
                    let managed_vec: Vec<_> = fully_managed_items.iter().collect();
                    let total_j = managed_vec.len();
                    for (i, item_id) in managed_vec.iter().enumerate() {
                        emit_progress("重建 junction 链接", i + 1, total_j, item_id);
                        let link_path = links_path.join(item_id);
                        if let Ok(canonical) = fs::canonicalize(&link_path) {
                            let target_str = canonical.to_string_lossy().to_string()
                                .trim_start_matches(r"\\?\").to_string();
                            let old_ver_prefix = format!("{}\\{}", normalize(old_versions_dir), item_id);
                            let target_lower = target_str.to_lowercase();
                            if let Some(rel_pos) = target_lower.find(&old_ver_prefix) {
                                let rel = &target_str[rel_pos + old_ver_prefix.len()..];
                                let rel = rel.trim_start_matches('\\').trim_start_matches('/');
                                let new_target = format!("{}\\{}\\{}", new_versions_dir, item_id, rel);
                                let _ = crate::commands::cache::create_junction(&link_path, Path::new(&new_target));
                                result.recreated_junctions.push(item_id.to_string());
                            }
                        }
                    }
                }
            }
        }

        // 记录残留的旧目录
        if old_versions.exists() {
            result.old_dirs_remain.push(old_versions_dir.to_string());
        }
    }

    // 发送完成事件
    emit_progress("完成", 1, 1, "");
    result
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
pub fn update_config(app_handle: tauri::AppHandle, data_dir: String) -> Result<MigrateResult, String> {
    let old_config = load_config();
    // 旧路径（fill_legacy_dirs 已保证非空）
    let old_versions_dir = old_config.versions_dir.clone();
    let old_links_dir = old_config.links_dir.clone();

    let mut config = old_config;
    let base_dir = get_base_dir();
    // 选项文件跟着数据目录走：记下旧位置，写进新目录后把旧的那份删掉
    let old_settings_path = settings_file_path();

    let new_data_dir = if data_dir.trim().is_empty() {
        base_dir.clone()
    } else {
        PathBuf::from(data_dir.trim())
    };

    // 新 sdk 两层路径（始终 new_data_dir/sdk）
    let new_sdk = new_data_dir.join("sdk");
    let new_versions_dir = new_sdk.join("_versions");
    let new_links_dir = new_sdk.clone();

    // 1. 迁移 SDK（版本库 + junction + env/PATH）
    let result = do_migrate_storage(
        &old_versions_dir,
        &new_versions_dir.to_string_lossy(),
        &old_links_dir,
        &new_links_dir.to_string_lossy(),
        &config,
        Some(&app_handle),
    );
    if !result.errors.is_empty() {
        return Err(result.errors.join("\n"));
    }

    config.data_dir = new_data_dir.to_string_lossy().to_string();
    // 同步填充废弃字段，保证现有引用一致
    config.versions_dir = new_versions_dir.to_string_lossy().to_string();
    config.links_dir = new_links_dir.to_string_lossy().to_string();

    // 2. data_dir 变化时，迁移 base_dir 下其余可变数据（含 node-projects）
    if new_data_dir != base_dir {
        migrate_data_dir_items(&app_handle, &base_dir, &new_data_dir);
    }

    save_config(&config)?;

    // settings.json 已随新数据目录写出；旧目录里那份是过期副本，留着会导致
    // 将来切回旧目录时读到一套陈旧选项，所以删掉（删除失败不影响迁移结果）。
    let new_settings_path = settings_file_path();
    if old_settings_path != new_settings_path && old_settings_path.is_file() {
        if let Err(e) = fs::remove_file(&old_settings_path) {
            eprintln!("[config] 删除旧选项文件失败 {}: {}", old_settings_path.display(), e);
        }
    }

    Ok(result)
}

/// 把 base_dir（~/.any-version）下的可变数据文件/目录迁移到 data_dir。
/// 排除 config.json（数据入口，必须留在 base_dir）以及由 sdk 专用迁移逻辑
/// （do_migrate_storage）处理的 versions/links/sdk。
fn migrate_data_dir_items(_app: &tauri::AppHandle, base_dir: &Path, data_dir: &Path) {
    let dirs = [
        "node-projects",
        "backup",
        "certs",
        "version_cache",
        "bin",
        "collab_tmp",
        ".tmp",
        "_temp_skill_clone",
    ];
    let files = [
        "tasks.db",
        "ai_usage.db",
        "ai_config.json",
        "ai_sessions.json",
        "last_launch_configs.json",
        "collab.json",
        "skills.json",
        "mcp.json",
        "backups.json",
        "skill-debug.log",
    ];
    for d in dirs {
        move_item(base_dir, data_dir, d);
    }
    for f in files {
        move_item(base_dir, data_dir, f);
    }
}

/// 对 active_versions 中记录的所有 SDK，重建锚点 junction。
/// 若目标版本存在且锚点不是有效 junction（如迁移后变成的普通空目录），
/// 统一重建（create_junction 内部会清理普通目录后重建）。
/// 返回被重建的 id 列表。
pub fn rebuild_sdk_junctions(config: &Config) -> Vec<String> {
    let mut rebuilt = Vec::new();
    let links_dir = get_sdk_link_dir();
    let versions_dir = get_sdk_versions_dir();
    for (item_id, version) in &config.active_versions {
        let link_path = links_dir.join(item_id);
        let target_path = versions_dir.join(item_id).join(version);
        if !target_path.exists() {
            continue;
        }
        let needs_rebuild = if link_path.is_symlink() {
            // 已是 junction，校验指向是否正确。
            // 注意：fs::canonicalize 在 Windows 返回带 `\\?\` 前缀的路径，
            // 必须去掉前缀再比较，否则恒判断为「指向错误」，每次启动都会误删重建 junction。
            fs::canonicalize(&link_path)
                .map(|c| {
                    let resolved = c.to_string_lossy().to_string();
                    let resolved_clean = resolved.trim_start_matches("\\\\?\\").to_lowercase();
                    let target_clean = target_path.to_string_lossy().to_lowercase();
                    !resolved_clean.starts_with(&target_clean)
                })
                .unwrap_or(false)
        } else {
            // 非 junction（不存在或普通目录）都需要重建
            true
        };
        if needs_rebuild {
            let _ = crate::commands::cache::create_junction(&link_path, &target_path);
            rebuilt.push(item_id.clone());
        }
    }
    rebuilt
}

/// 把 base_dir 下的单个文件/目录移动到 data_dir（幂等：已存在/不存在则跳过）。
/// Windows 上 `fs::rename` 不能跨磁盘驱动器，失败时回退为「复制 + 删除」。
fn move_item(base_dir: &Path, data_dir: &Path, name: &str) {
    let src = base_dir.join(name);
    if !src.exists() {
        return;
    }
    let dst = data_dir.join(name);
    if dst.exists() {
        return;
    }
    if let Some(parent) = dst.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::rename(&src, &dst) {
        Ok(()) => {
            eprintln!("[config] 已迁移数据项 {} -> {}", src.display(), dst.display());
        }
        Err(e) => {
            // 跨盘（os error 17 ERROR_NOT_SAME_DEVICE）时回退为复制 + 删除
            eprintln!("[config] rename 迁移 {} 失败({})，尝试复制回退", name, e);
            if src.is_dir() {
                if copy_dir_all(&src, &dst).is_ok() {
                    let _ = std::fs::remove_dir_all(&src);
                    eprintln!("[config] 已复制迁移目录 {} -> {}", src.display(), dst.display());
                } else {
                    eprintln!("[config] 复制迁移目录 {} 失败", name);
                }
            } else if std::fs::copy(&src, &dst).is_ok() {
                let _ = std::fs::remove_file(&src);
                eprintln!("[config] 已复制迁移文件 {} -> {}", src.display(), dst.display());
            } else {
                eprintln!("[config] 复制迁移文件 {} 失败", name);
            }
        }
    }
}

/// 递归复制目录（用于跨盘迁移回退）。
fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn get_project_menu_config(id: String) -> Result<ProjectMenuConfig, String> {
    let config = load_config();
    Ok(config.project_menu_configs.get(&id).cloned().unwrap_or_else(|| ProjectMenuConfig {
        show_version: true,
        show_service: true,
    }))
}

#[tauri::command]
pub fn update_project_menu_config(app_handle: tauri::AppHandle, id: String, show_version: bool, show_service: bool) -> Result<(), String> {
    let mut config = load_config();
    config.project_menu_configs.insert(id, ProjectMenuConfig {
        show_version,
        show_service,
    });
    save_config(&config)?;
    let _ = crate::tray::rebuild_tray_menu(&app_handle);
    Ok(())
}

/// 单个 RSS 订阅源（含自定义名称）
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RssSourceDto {
    pub url: String,
    #[serde(default)]
    pub name: String,
    /// rss = XML feed, web = site-specific HTML adapter.
    #[serde(default = "default_rss_source_kind")]
    pub kind: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RssConfig {
    pub sources: Vec<RssSourceDto>,
    pub is_first_launch: bool,
}

/// 内置默认 RSS 源的 kind（兼容旧配置反序列化）
fn default_rss_source_kind() -> String {
    "rss".to_string()
}

/// 内置默认 RSS 源的名称映射
fn default_rss_source_names() -> std::collections::HashMap<String, String> {
    let mut m = std::collections::HashMap::new();
    m.insert("https://36kr.com/feed".to_string(), "36氪".to_string());
    m.insert(
        "https://www.ruanyifeng.com/blog/atom.xml".to_string(),
        "阮一峰的网络日志".to_string(),
    );
    m
}

/// 将 config 中的 url 列表与名称映射合并为带名称的源列表
fn build_rss_sources(config: &Config) -> Vec<RssSourceDto> {
    config
        .rss_sources
        .iter()
        .map(|url| RssSourceDto {
            url: url.clone(),
            name: config.rss_source_names.get(url).cloned().unwrap_or_default(),
            kind: "rss".to_string(),
        })
        .collect()
}


#[tauri::command]
pub fn get_rss_config() -> Result<RssConfig, String> {
    let mut config = load_config();
    let is_first_launch = !config.has_run_before;
    if is_first_launch {
        config.has_run_before = true;
        if config.rss_sources.is_empty() {
            config.rss_sources = vec![
                "https://36kr.com/feed".to_string(),
                "https://www.ruanyifeng.com/blog/atom.xml".to_string(),
            ];
        }
        if config.rss_source_names.is_empty() {
            config.rss_source_names = default_rss_source_names();
        }
        save_config(&config)?;
    }
    Ok(RssConfig {
        sources: build_rss_sources(&config),
        is_first_launch,
    })
}

#[tauri::command]
pub fn set_rss_sources(sources: Vec<RssSourceDto>) -> Result<(), String> {
    let mut config = load_config();
    config.rss_sources = sources.iter().map(|s| s.url.clone()).collect();
    config.rss_source_names = sources
        .iter()
        .filter(|s| !s.name.trim().is_empty())
        .map(|s| (s.url.clone(), s.name.trim().to_string()))
        .collect();
    save_config(&config)?;
    Ok(())
}

fn http_client() -> reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .connect_timeout(std::time::Duration::from_secs(5))
                .build()
                .expect("构建 HTTP 客户端失败")
        })
        .clone()
}

/// 仅允许 http/https 网址，拒绝 file:// 等本地协议。
fn validate_http_url(url: &str) -> Result<(), String> {
    if url.starts_with("http://") || url.starts_with("https://") {
        Ok(())
    } else {
        Err("仅支持 http/https 网址".to_string())
    }
}


#[tauri::command]
pub async fn fetch_rss_feed(url: String) -> Result<String, String> {
    validate_http_url(&url)?;
    let client = http_client();

    let response = client.get(&url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .header("Accept", "application/rss+xml, application/xml, text/xml, text/html;q=0.9, */*;q=0.8")
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("HTTP 请求失败: 状态码 {}", status));
    }

    let text = response.text()
        .await
        .map_err(|e| format!("读取内容失败: {}", e))?;

    Ok(text)
}

#[tauri::command]
pub fn get_auto_start_services() -> Vec<String> {
    let config = load_config();
    let mut list: Vec<String> = config.auto_start_services.into_iter().collect();
    list.sort();
    list
}

#[tauri::command]
pub fn set_auto_start_service(service_id: String, enabled: bool) -> Result<(), String> {
    let mut config = load_config();
    if enabled {
        config.auto_start_services.insert(service_id);
    } else {
        config.auto_start_services.remove(&service_id);
    }
    save_config(&config)
}

// ---- 外观：顶级模块主题色 + 全局字体 ----

/// 供前端读取外观配置（模块主题色 + 全局字体 + 自定义字体 + 模块顺序）
#[tauri::command]
pub fn get_appearance_config() -> AppearanceConfig {
    let config = load_config();
    AppearanceConfig {
        module_theme_colors: config.module_theme_colors,
        global_font: config.global_font,
        custom_font_path: config.custom_font_path,
        module_order: config.module_order,
        toolbar_modules: config.toolbar_modules,
        disabled_modules: config.disabled_modules,
        background_texture: config.background_texture,
        language: config.language,
    }
}

/// 保存顶级模块的自定义顺序（模块 id 列表，空 = 恢复默认顺序）
#[tauri::command]
pub fn set_module_order(order: Vec<String>) -> Result<(), String> {
    let mut config = load_config();
    config.module_order = order;
    save_config(&config)
}

/// 保存模块布局：顶栏模块列表 + 禁用模块列表（所有模块平级，用户自由归类）。
#[tauri::command]
pub fn set_module_layout(
    toolbar_modules: Vec<String>,
    disabled_modules: Vec<String>,
) -> Result<(), String> {
    let mut config = load_config();
    config.toolbar_modules = toolbar_modules;
    config.disabled_modules = disabled_modules;
    save_config(&config)
}

/// 设置单个顶级模块的主题色（hex，如 #8b5cf6）
#[tauri::command]
pub fn set_module_theme_color(module_id: String, color: String) -> Result<(), String> {
    let mut config = load_config();
    if color.trim().is_empty() {
        config.module_theme_colors.remove(&module_id);
    } else {
        config.module_theme_colors.insert(module_id, color);
    }
    save_config(&config)
}

/// 设置全局字体（CSS font-family 名称，空=恢复默认）
#[tauri::command]
pub fn set_global_font(font: String) -> Result<(), String> {
    let mut config = load_config();
    config.global_font = font;
    save_config(&config)
}

/// 设置全局背景底图纹理（grid / dots / scanline / aurora / solid；空=默认网格）。
#[tauri::command]
pub fn set_background_texture(texture: String) -> Result<(), String> {
    let mut config = load_config();
    config.background_texture = texture;
    save_config(&config)
}

/// 设置界面语言（"zh" / "en"；空=跟随系统）。
/// 语言变更后立即重建托盘菜单（菜单文案随语言切换）。
#[tauri::command]
pub fn set_language(language: String) -> Result<(), String> {
    let mut config = load_config();
    config.language = language;
    save_config(&config)?;
    let _ = crate::tray::rebuild_tray_menu_global();
    Ok(())
}

/// 导入自定义字体文件：把 src 拷贝到数据目录 fonts/，返回字体家族名与目标路径
#[tauri::command]
pub fn import_custom_font(src: String) -> Result<CustomFontInfo, String> {
    let src_path = Path::new(&src);
    if !src_path.exists() {
        return Err("字体文件不存在".to_string());
    }
    let ext = src_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .filter(|e| matches!(e.as_str(), "ttf" | "otf" | "woff" | "woff2"))
        .unwrap_or_default();
    if ext.is_empty() {
        return Err("仅支持 .ttf / .otf / .woff / .woff2 字体文件".to_string());
    }

    let fonts_dir = get_data_dir().join("fonts");
    let _ = fs::create_dir_all(&fonts_dir);

    let file_name = src_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("custom_font")
        .to_string();
    // 家族名用文件名去掉扩展名，并替换非法字符
    let family = file_name
        .trim_end_matches(&format!(".{ext}"))
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' { c } else { '_' })
        .collect::<String>();

    let dest = fonts_dir.join(&file_name);
    fs::copy(src_path, &dest).map_err(|e| format!("拷贝字体失败: {e}"))?;

    let mut config = load_config();
    config.custom_font_path = dest.to_string_lossy().to_string();
    config.global_font = family.clone();
    save_config(&config)?;

    Ok(CustomFontInfo {
        family,
        path: dest.to_string_lossy().to_string(),
        ext,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 落盘的 config.json 只能有数据路径；业务选项必须落进 settings.json。
    #[test]
    fn config_json_carries_only_paths() {
        let mut config = default_config();
        config.data_dir = "D:/any-versions".to_string();
        config.node_projects_dir = "D:/any-versions/node-projects".to_string();
        config.managed_items.insert("nodejs".to_string());
        config.rss_sources = vec!["https://example.com/feed".to_string()];
        config.tray_menu.enabled = false;

        let (paths, settings) = split_config(&config);
        let json = serde_json::to_value(&paths).unwrap();
        let keys: Vec<String> = json.as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys.len(), 2, "config.json 字段: {:?}", keys);
        assert!(keys.iter().any(|k| k == "data_dir"));
        assert!(keys.iter().any(|k| k == "node_projects_dir"));
        assert!(
            !keys.iter().any(|k| k.starts_with("rss_")
                || k.starts_with("managed")
                || k.starts_with("module_")
                || k == "tray_menu"),
            "业务选项不该出现在 config.json: {:?}",
            keys
        );

        // 合并回来后一个字段都不能丢
        let merged = merge_config(paths, settings);
        assert_eq!(merged.data_dir, "D:/any-versions");
        assert_eq!(merged.node_projects_dir, "D:/any-versions/node-projects");
        assert!(merged.managed_items.contains("nodejs"));
        assert_eq!(merged.rss_sources, vec!["https://example.com/feed".to_string()]);
        assert!(!merged.tray_menu.enabled);
    }

    /// 旧版 config.json（选项和路径混在一起）要能被拆开读：路径归路径，选项归选项。
    #[test]
    fn legacy_combined_config_splits_on_read() {
        let legacy = r#"{
            "data_dir": "D:/any-versions",
            "managed_items": ["go", "nodejs"],
            "rss_sources": ["https://a/feed"],
            "tray_menu": { "enabled": false, "show_mihomo": true }
        }"#;
        let paths: ConfigPathsFile = serde_json::from_str(legacy).unwrap();
        let settings: SettingsFile = serde_json::from_str(legacy).unwrap();
        assert_eq!(paths.data_dir, "D:/any-versions");
        assert!(settings.managed_items.contains("go"));
        assert_eq!(settings.rss_sources, vec!["https://a/feed".to_string()]);
        assert!(!settings.tray_menu.enabled);
    }

    /// 缺省字段的旧 settings.json 也要能读（全 default，不能因为缺字段整份解析失败）。
    #[test]
    fn settings_file_tolerates_missing_fields() {
        let settings: SettingsFile = serde_json::from_str("{}").unwrap();
        assert!(settings.managed_items.is_empty());
        assert!(settings.rss_sources.is_empty());
        assert!(settings.tray_menu.enabled);
        assert_eq!(settings.language, "");
    }
}

/// 移除自定义字体（恢复默认字体）
#[tauri::command]
pub fn clear_custom_font() -> Result<(), String> {
    let mut config = load_config();
    if !config.custom_font_path.is_empty() {
        let p = config.custom_font_path.clone();
        let _ = fs::remove_file(Path::new(&p));
    }
    config.custom_font_path.clear();
    config.global_font.clear();
    save_config(&config)
}

/// 枚举系统已安装的字体家族名（供前端「全局字体」下拉动态填充）。
///
/// 合并读取两个字体注册表位置，避免漏掉「仅当前用户安装」的字体：
/// - `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts`（为所有用户安装）
/// - `HKCU\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts`（为当前用户安装）
/// 值名形如 `Arial (TrueType)`，去掉括号后缀与扩展名得到家族名，去重排序返回。
#[tauri::command]
pub fn list_system_fonts() -> Result<Vec<String>, String> {
    #[cfg(windows)]
    {
        use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
        use winreg::RegKey;

        const FONTS_SUBKEY: &str = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Fonts";
        let mut families: Vec<String> = Vec::new();

        for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
            let key = RegKey::predef(root).open_subkey(FONTS_SUBKEY);
            let key = match key {
                Ok(k) => k,
                Err(_) => continue, // 该位置不存在/无权访问则跳过
            };
            for name in key.enum_values().filter_map(|v| v.ok()).map(|(n, _)| n) {
                let fam = strip_font_suffix(&name);
                if !fam.is_empty() && !families.contains(&fam) {
                    families.push(fam);
                }
            }
        }
        families.sort();
        Ok(families)
    }
    #[cfg(not(windows))]
    {
        Ok(Vec::new())
    }
}

/// 从字体注册表值名里提取家族名：去掉「(xxx)」后缀、去掉 Regular/Bold/Italic 等字重后缀。
fn strip_font_suffix(name: &str) -> String {
    let trimmed = name.trim();
    // 去掉末尾括号及其内容，如 "Arial (TrueType)" -> "Arial"
    let no_paren = match trimmed.rfind('(') {
        Some(idx) => trimmed[..idx].trim_end().to_string(),
        None => trimmed.to_string(),
    };
    // 去掉末尾的常见字重/样式标记（Regular / Bold / Italic / Light / Medium ...）
    const STYLE_SUFFIXES: &[&str] = &[
        " Regular", " Bold", " Italic", " Bold Italic", " Light", " Medium",
        " SemiBold", " Semibold", " Black", " Heavy", " Thin", " ExtraLight",
        " ExtraBold", " UltraLight", " SemiLight", " DemiBold", " Demi",
    ];
    let mut fam = no_paren;
    for s in STYLE_SUFFIXES {
        if let Some(stripped) = fam.strip_suffix(s) {
            fam = stripped.to_string();
            break;
        }
    }
    // 去掉扩展名（部分字体值名带 .ttf/.ttc/.otf）
    for ext in [".ttf", ".ttc", ".otf", ".TTF", ".TTC", ".OTF"] {
        if let Some(stripped) = fam.strip_suffix(ext) {
            fam = stripped.to_string();
            break;
        }
    }
    fam.trim().to_string()
}

/// 自定义字体信息
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CustomFontInfo {
    pub family: String,
    pub path: String,
    pub ext: String,
}

/// 外观配置（模块主题色 + 全局字体 + 模块顺序 + 模块布局）
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AppearanceConfig {
    pub module_theme_colors: std::collections::HashMap<String, String>,
    pub global_font: String,
    pub custom_font_path: String,
    pub module_order: Vec<String>,
    pub toolbar_modules: Vec<String>,
    pub disabled_modules: Vec<String>,
    /// 全局背景底图纹理。
    pub background_texture: String,
    /// 界面语言（"zh" / "en"）。
    pub language: String,
}

