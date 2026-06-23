//! 项目管理模块 — 核心类型定义。
//!
//! 包含项目定义、运行时状态、托管管理等全部数据结构。

use serde::{Serialize, Deserialize};
use std::collections::HashMap;

/// 项目分类
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectCategory {
    Language,
    Tool,
    Service,
}

/// 环境变量定义
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EnvVarTier {
    Core,
    Package,
    Compat,
    Clear,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EnvVarDef {
    /// 环境变量名
    pub name: String,
    /// 描述
    pub desc: String,
    /// 检查类型: "path" | "nonempty" | "runtime"
    pub check_type: String,
    #[serde(default)]
    pub tier: Option<EnvVarTier>,
    /// 托管时的值子目录（相对于 link_dir），不填则值 = link_dir
    #[serde(default)]
    pub sub_dir: Option<String>,
}

/// 用户可配置的环境变量（运行时参数，非路径类）
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UserConfigurableVar {
    pub name: String,
    pub desc: String,
    #[serde(default)]
    pub placeholder: Option<String>,
    #[serde(default)]
    pub options: Option<Vec<String>>,
    #[serde(default)]
    pub var_type: Option<String>, // "boolean" | undefined (free text)
}

/// 路径解析模式
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResolvePattern {
    /// 在 PATH 中查找包含指定路径关键字的条目
    PathContains { path_key: String, exe_name: String },
    /// 从环境变量获取根目录，拼接 bin 子路径，检查可执行文件
    EnvBin { env_var: String, bin_sub: String, exe_name: String },
    /// 检查固定路径是否存在可执行文件
    FixedPath { path: String, exe_name: String },
}

/// 路径解析规则
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FindRule {
    /// 解析模式
    pub pattern: ResolvePattern,
    /// 来源标签（如 "Scoop", "Chocolatey", "系统 PATH" 等）
    pub source_label: String,
    /// 优先级（越小越优先，0 = 最高）
    pub priority: u8,
    /// 发现后，实际根目录相对于匹配路径的向上回溯层数
    pub root_offset: u8,
}

/// 镜像选项
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MirrorOption {
    /// 镜像类型
    pub mirror_type: String,
    /// 镜像名称
    pub name: String,
    /// 镜像 URL
    pub url: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CacheConfigSource {
    pub parser_type: String,
    pub paths: Vec<String>,
    #[serde(default)]
    pub keys: Vec<String>,
    #[serde(default)]
    pub env_vars: Vec<String>,
    #[serde(default)]
    pub replacements: HashMap<String, String>,
    #[serde(default)]
    pub suffix: Option<String>,
}

/// 包管理器定义（嵌套在项目内，如 Node.js 下 of npm/yarn/pnpm）
///
/// 缓存、镜像、数据路径等属性属于包管理器，而非语言本身。
/// 例如 `npm config get cache` / `npm config set registry` 是 npm 的功能，
/// 不应放在 nodejs 语言定义上。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PackageManagerDef {
    /// 唯一标识
    pub id: String,
    /// 显示名称
    pub display_name: String,
    /// 是否为内置包管理器（随语言一起安装，如 npm 随 Node.js 安装）
    #[serde(default)]
    pub built_in: bool,
    /// 安装命令（如 "npm install -g yarn"）；built_in 的包管理器填 null
    pub install_cmd: Option<String>,
    /// 版本检测命令（如 "npm --version"）
    #[serde(default)]
    pub version_cmd: Option<String>,
    /// 版本检测可执行文件名（如 "npm", "yarn"）
    #[serde(default)]
    pub version_exe: Option<String>,
    /// 缓存路径检测命令（如 "yarn cache dir"）
    pub cache_detect_cmd: Option<String>,
    /// 全局包列表命令
    pub pkg_list_cmd: Option<String>,
    /// 镜像设置命令模板
    pub mirror_cmd_template: Option<String>,
    /// 可用镜像选项
    pub mirror_options: Option<Vec<MirrorOption>>,

    /// Remote version detection config.
    #[serde(default)]
    pub remote_versions_config: Option<serde_json::Value>,
    /// Default cache path.
    #[serde(default)]
    pub cache_default_path: Option<String>,
    /// Cache environment variable.
    #[serde(default)]
    pub cache_env_var: Option<String>,
    /// Cache set command template.
    #[serde(default)]
    pub cache_set_cmd_template: Option<String>,
    /// Default data path.
    #[serde(default)]
    pub data_default_path: Option<String>,
    /// Data environment variable.
    #[serde(default)]
    pub data_env_var: Option<String>,
    /// Data set command template.
    #[serde(default)]
    pub data_set_cmd_template: Option<String>,
    /// Data path detect command.
    #[serde(default)]
    pub data_detect_cmd: Option<String>,
    /// Proxy detect command (e.g., "npm config get proxy").
    #[serde(default)]
    pub proxy_detect_cmd: Option<String>,
    /// Proxy set command template (e.g., "npm config set proxy {url}").
    #[serde(default)]
    pub proxy_set_cmd_template: Option<String>,
    
    /// Global package list output format ("npm_json" | "pip_json" | "yarn_json" | "pnpm_json" | "text_lines").
    #[serde(default)]
    pub pkg_list_format: Option<String>,
    /// Global package upgrade command template (e.g., "npm install -g {pkg}@latest").
    #[serde(default)]
    pub pkg_upgrade_cmd_template: Option<String>,
    /// Package homepage template (e.g., "https://www.npmjs.com/package/{pkg}").
    #[serde(default)]
    pub pkg_homepage_template: Option<String>,
    /// Outdated packages list command.
    #[serde(default)]
    pub pkg_outdated_cmd: Option<String>,
    /// Outdated packages list output format ("npm_outdated_json" | "pip_outdated_json" | etc.).
    #[serde(default)]
    pub pkg_outdated_format: Option<String>,
    /// Custom configuration-file based cache resolver config.
    #[serde(default)]
    pub cache_config_source: Option<CacheConfigSource>,
}

/// Scoop 引用：指向 ScoopInstaller 仓库中的 manifest
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ScoopRef {
    /// Bucket 名称（默认 "Main"）
    #[serde(default = "default_bucket")]
    pub bucket: String,
    /// manifest 文件名（不含 .json 后缀）
    pub name: String,
}

fn default_bucket() -> String { "Main".to_string() }

/// 项目定义（存储在 projects.json）
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProjectDef {
    /// 唯一标识
    pub id: String,
    /// 显示名称
    pub display_name: String,
    /// 分类
    pub category: ProjectCategory,
    /// 官方网站
    pub official_website: String,

    /// 该项目可用的包管理器（如 Node.js 下的 yarn/pnpm/npm）
    #[serde(default)]
    pub package_managers: Vec<PackageManagerDef>,

    /// 用户可配置的运行时环境变量（非路径类，高级用户可自定义设置）
    #[serde(default)]
    pub user_configurable_vars: Vec<UserConfigurableVar>,

    /// 关联的环境变量
    pub env_vars: Vec<EnvVarDef>,
    /// 路径解析规则
    pub find_rules: Vec<FindRule>,

    /// 是否有缓存管理
    pub has_cache: bool,
    /// 缓存检测命令
    pub cache_detect_cmd: Option<String>,
    /// 默认缓存路径
    pub cache_default_path: Option<String>,

    /// 是否支持镜像
    pub has_mirror: bool,
    /// 镜像选项列表
    pub mirror_options: Option<Vec<MirrorOption>>,

    /// 是否有包管理
    pub has_pkg: bool,
    /// 包管理器名称
    pub pkg_manager: Option<String>,
    /// 包主页 URL 模板
    pub pkg_homepage_template: Option<String>,

    /// 是否为本地服务
    pub is_service: bool,
    /// 默认端口
    pub default_port: Option<u16>,
    /// 数据目录
    pub data_dir: Option<String>,
    /// 日志目录
    pub log_dir: Option<String>,
    /// 配置文件路径
    pub config_file: Option<String>,
    /// 启动命令
    pub start_cmd: Option<String>,
    /// 停止命令
    pub stop_cmd: Option<String>,

    /// Version detection command (e.g. "node --version", "go version")
    #[serde(default)]
    pub version_cmd: Option<String>,
    /// Version detection executable name (e.g. "node", "go")
    #[serde(default)]
    pub version_exe: Option<String>,

    /// Scoop manifest 引用（用于从 Scoop 更新安装参数）
    #[serde(default)]
    pub scoop_ref: Option<ScoopRef>,

    /// 下载 URL 模板（{version} 为占位符）
    #[serde(default)]
    pub download_url_template: Option<String>,
    /// 下载文件扩展名 (zip / tar.gz / nupkg / exe / msi / 7z)
    #[serde(default)]
    pub download_file_ext: Option<String>,
    /// 解压后的子目录（如 python 的 "tools"）
    #[serde(default)]
    pub extract_subdir: Option<String>,
    /// 需要添加到 PATH 的目录列表（相对于安装根目录）
    /// 由 Scoop 更新工具自动填充，也支持手动定义
    #[serde(default)]
    pub bin_dirs: Option<Vec<String>>,
    /// Scoop 数据最后同步时间（ISO 8601）
    #[serde(default)]
    pub scoop_updated_at: Option<String>,
    /// 版本前缀 → URL 模板映射（如 java 的 adoptium-/microsoft- 前缀）
    #[serde(default)]
    pub version_prefix_map: Option<std::collections::HashMap<String, String>>,
    /// 版本前缀 → URL 模板映射（按版本号前缀匹配，如 mysql 的 5.7/8.0/8.4）
    #[serde(default)]
    pub version_url_prefix_map: Option<std::collections::HashMap<String, String>>,

    /// 远程版本列表 URL
    #[serde(default)]
    pub remote_versions_url: Option<String>,
    /// 远程版本获取配置
    #[serde(default)]
    pub remote_versions_config: Option<serde_json::Value>,

    /// 安装后置配置
    #[serde(default)]
    pub post_install: Option<serde_json::Value>,
    /// Local version detection output regex pattern.
    #[serde(default)]
    pub version_parse_regex: Option<String>,
}

/// 环境变量运行时状态
#[derive(Serialize, Clone, Debug)]
pub struct EnvVarStatus {
    /// 变量名
    pub name: String,
    /// 描述
    pub desc: String,
    /// 当前值
    pub value: Option<String>,
    /// 来源: "HKCU" | "HKLM" | "未设置"
    pub source: String,
    /// 路径是否存在（path 类型时有效）
    pub exists: bool,
    /// 是否指向 AnyVersion 管理的目录
    pub in_anyversion: bool,
    #[serde(default)]
    pub tier: Option<EnvVarTier>,
}

/// 缓存状态
#[derive(Serialize, Clone, Debug)]
pub struct CacheStatus {
    /// 缓存路径
    pub path: String,
    /// 缓存大小（格式化后）
    pub size: String,
    /// 是否为链接/junction
    pub is_link: bool,
    /// 实际指向的目标路径
    pub real_target: String,
    /// 检测来源说明
    pub detect_source: String,
}

/// 服务状态
#[derive(Serialize, Clone, Debug)]
pub struct ServiceStatus {
    /// 是否正在运行
    pub running: bool,
    /// 端口号
    pub port: Option<u16>,
    /// 进程 PID
    pub pid: Option<u32>,
    /// 数据目录
    pub data_dir: String,
    /// 日志目录
    pub log_dir: String,
}

/// 项目运行时状态（实时扫描结果）
#[derive(Serialize, Clone, Debug)]
pub struct ProjectStatus {
    /// 项目 ID
    pub id: String,
    /// 显示名称
    pub display_name: String,
    /// 分类
    pub category: ProjectCategory,
    /// 是否已安装（至少存在一个版本）
    pub installed: bool,
    /// 当前激活的版本
    pub active_version: Option<String>,
    /// 已安装的版本列表
    pub installed_versions: Vec<String>,
    /// 安装来源（如 "Scoop", "AnyVersion" 等）
    pub install_source: Option<String>,
    /// 安装根目录
    pub install_root: Option<String>,
    /// 是否被 AnyVersion 托管管理
    pub managed: bool,
    /// 环境变量状态列表
    pub env_vars_status: Vec<EnvVarStatus>,
    /// 缓存状态（如果项目有缓存）
    pub cache_status: Option<CacheStatus>,
    /// 服务状态（如果项目是服务）
    pub service_status: Option<ServiceStatus>,
}

/// 项目详情（比 Status 多出定义信息）
#[derive(Serialize, Clone, Debug)]
pub struct ProjectDetail {
    /// 项目定义
    pub def: ProjectDef,
    /// 运行时状态
    pub status: ProjectStatus,
}

/// 托管预览操作
#[derive(Serialize, Clone, Debug)]
pub struct ManagePreview {
    /// 操作步骤列表
    pub steps: Vec<ManageStep>,
    /// 是否检测到本地已安装版本
    pub has_local_install: bool,
    /// 本地安装路径
    pub local_install_root: Option<String>,
    /// 本地安装来源标签
    pub local_install_source: Option<String>,
}

/// 托管操作步骤
#[derive(Serialize, Clone, Debug)]
pub struct ManageStep {
    pub action: String,
    pub description: String,
    pub target: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProjectBackup {
    pub env_vars: HashMap<String, String>,
    pub path_entries: Vec<String>,
    pub service_path: Option<String>,
}

/// 旧版安装信息备份（托管时自动保存）
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LegacyInstallInfo {
    /// 旧版安装来源（如 Scoop、Chocolatey 等）
    pub install_source: String,
    /// 旧版安装根目录
    pub install_root: String,
    /// 旧版检测到的版本号
    pub version: String,
    /// 托管时备份的环境变量（变量名 → 原始值）
    pub backed_env_vars: HashMap<String, String>,
    /// 托管时从 PATH 移除的条目
    pub removed_path_entries: Vec<String>,
    /// 备份时间戳
    pub timestamp: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ManagedProject {
    pub project_id: String,
    pub managed_at: String,
    pub backup: ProjectBackup,
    pub state: String,
}
