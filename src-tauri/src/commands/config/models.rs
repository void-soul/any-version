use super::*;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProjectMenuConfig {
    #[serde(default = "default_true")]
    pub show_version: bool,
    #[serde(default = "default_true")]
    pub show_service: bool,
}

pub(super) fn default_true() -> bool {
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

pub(super) fn default_false() -> bool {
    false
}

/// 2API 默认端口（避开 Free Router 的 8787，与原 Python 服务一致）
pub(super) fn default_twoapi_port() -> u16 {
    8788
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    /// In-process read baseline, never exposed through IPC or written to disk.
    #[serde(skip)]
    pub(super) baseline: Option<serde_json::Value>,
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
    pub custom_data_paths:
        std::collections::HashMap<String, std::collections::HashMap<String, String>>,
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
    /// 2API（内嵌的 WorkBuddy → OpenAI/Anthropic 兼容 API）监听端口。
    /// 端口被占用时用户可在 2API 面板改，改完会同步 AI 模块里指向它的供应商 URL。
    #[serde(default = "default_twoapi_port")]
    pub twoapi_port: u16,
    /// 内嵌证书接收端（原 cert-receiver 独立小工具，现已集成）。
    /// 为空表示还没配置过；首次启动若发现旧 `cert-receiver.toml` 会自动迁移并写入。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cert_receiver: Option<crate::commands::cert_receiver::CertReceiverConfig>,
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
    /// 动效偏好：`"system"`（跟随系统「减少动效」设置，默认） / `"always"`（无论系统怎么设都动）。
    /// 系统被切成「最佳性能」时 `prefers-reduced-motion` 会变成 reduce，
    /// 头像辉光闪烁等自家动效就全停了 —— 这里给一个不听系统的出口。
    #[serde(default)]
    pub motion_preference: String,
    /// 列表密度：`"compact"` = 紧凑（行内边距收窄）；其余（含空）= 舒适（默认）。
    /// 只影响 `--density-py` 这一个 CSS 变量，命中范围见 App.css 的 `data-density` 规则。
    #[serde(default)]
    pub density: String,
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
pub(super) struct ConfigPathsFile {
    #[serde(default)]
    pub data_dir: String,
    /// 同样属于派生路径：默认 `data_dir/node-projects`，**只有用户显式改到别处才落盘**
    /// （空 = 派生，跟 versions_dir / links_dir / sdk_dir 一个思路）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub node_projects_dir: String,
}

/// `settings.json` 的落盘形态：除数据路径以外的全部业务选项。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub(super) struct SettingsFile {
    pub managed_items: std::collections::HashSet<String>,
    pub simple_managed_items: std::collections::HashSet<String>,
    pub custom_install_paths: std::collections::HashMap<String, String>,
    pub custom_data_paths:
        std::collections::HashMap<String, std::collections::HashMap<String, String>>,
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
    #[serde(default)]
    pub motion_preference: String,
    #[serde(default)]
    pub density: String,
    /// 内嵌证书接收端配置（原 cert-receiver 的 toml 字段，迁移后落在这里）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cert_receiver: Option<crate::commands::cert_receiver::CertReceiverConfig>,
}
