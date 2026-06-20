//! SDK 注册表 — 所有 SDK/工具/库的动态定义。
//!
//! 支持从文件 `%USERPROFILE%\.any-version\sdks_registry.json` 加载。
//! 如果文件不存在，会自动创建默认的设置文件。

use serde::{Serialize, Deserialize};
use super::sdk_resolver::{FindRule, ResolvePattern};

/// SDK 分类
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SdkCategory {
    Language,   // 开发语言
    LibManager, // 库管理
    Tool,       // 开发工具
    Service,    // 本地服务
}

impl SdkCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Language   => "language",
            Self::LibManager => "lib_manager",
            Self::Tool       => "tool",
            Self::Service    => "service",
        }
    }
}

/// 环境变量检查类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvCheckType {
    Path,
    NonEmpty,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvVar {
    pub name: String,
    pub desc: String,
    pub check_type: EnvCheckType,
}

/// SDK 定义条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SdkDef {
    pub id: String,
    pub display_name: String,
    pub category: SdkCategory,
    pub official_website: String,
    pub has_cache: bool,
    pub has_mirror: bool,
    pub has_pkg: bool,
    /// 该 SDK 关联的环境变量
    pub env_vars: Vec<EnvVar>,
    /// 路径解析规则
    pub find_rules: Vec<FindRule>,
    
    // 服务与端口数据路径管理
    #[serde(default)]
    pub is_service: Option<bool>,
    #[serde(default)]
    pub default_port: Option<u16>,
    #[serde(default)]
    pub data_dir: Option<String>,
    #[serde(default)]
    pub log_dir: Option<String>,
}

/// 全局 SDK 注册表，从磁盘 JSON 文件中读取。
pub fn registry() -> Vec<SdkDef> {
    load_registry()
}

/// 从配置文件动态加载注册表
pub fn load_registry() -> Vec<SdkDef> {
    let base_dir = crate::commands::config::get_base_dir();
    let registry_path = base_dir.join("sdks_registry.json");
    if registry_path.exists() {
        if let Ok(data) = std::fs::read_to_string(&registry_path) {
            if let Ok(list) = serde_json::from_str::<Vec<SdkDef>>(&data) {
                // 如果已存的默认服务项没有 is_service 字段，强制重新生成配置以载入服务与端口数据
                let needs_regeneration = list.iter().any(|s| {
                    (s.id == "mysql" || s.id == "redis" || s.id == "nginx") && s.is_service.is_none()
                });
                if !needs_regeneration {
                    return list;
                }
            }
        }
    }
    // 不存在或解析错误则写入默认列表
    let defaults = get_default_registry();
    let _ = std::fs::create_dir_all(&base_dir);
    if let Ok(data) = serde_json::to_string_pretty(&defaults) {
        let _ = std::fs::write(&registry_path, data);
    }
    defaults
}

/// 根据 id 查找 SDK 定义
pub fn find_by_id(id: &str) -> Option<SdkDef> {
    registry().into_iter().find(|s| s.id == id)
}

/// 返回所有 SDK id 列表（用于遍历）
pub fn all_ids() -> Vec<String> {
    registry().into_iter().map(|s| s.id).collect()
}

pub fn get_default_registry() -> Vec<SdkDef> {
    vec![
        SdkDef {
            id: "nodejs".to_string(),
            display_name: "Node.js".to_string(),
            category: SdkCategory::Language,
            official_website: "https://nodejs.org".to_string(),
            has_cache: true,
            has_mirror: true,
            has_pkg: true,
            env_vars: vec![EnvVar { name: "NODE_PATH".to_string(), desc: "全局模块搜索路径".to_string(), check_type: EnvCheckType::Path }, EnvVar { name: "NPM_CONFIG_PREFIX".to_string(), desc: "npm 全局安装前缀".to_string(), check_type: EnvCheckType::Path }, EnvVar { name: "NPM_CONFIG_CACHE".to_string(), desc: "npm 缓存目录".to_string(), check_type: EnvCheckType::Path }, EnvVar { name: "NVM_DIR".to_string(), desc: "nvm-windows 安装目录".to_string(), check_type: EnvCheckType::Path }, EnvVar { name: "NVM_HOME".to_string(), desc: "nvm-windows 根目录".to_string(), check_type: EnvCheckType::Path }, EnvVar { name: "VOLTA_HOME".to_string(), desc: "Volta 安装目录".to_string(), check_type: EnvCheckType::Path }],
            find_rules: vec![
                FindRule { pattern: ResolvePattern::EnvBin { env: "NVM_HOME".to_string(), bin_sub: "".to_string(), exe: "node.exe".to_string() }, source_label: "nvm-windows".to_string(), priority: 10, root_offset: 0 },
                FindRule { pattern: ResolvePattern::EnvBin { env: "VOLTA_HOME".to_string(), bin_sub: "bin".to_string(), exe: "node.exe".to_string() }, source_label: "Volta".to_string(), priority: 10, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "scoop\\apps\\nodejs".to_string(), exe: "node.exe".to_string() }, source_label: "Scoop".to_string(), priority: 40, root_offset: 1 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "chocolatey\\lib\\nodejs".to_string(), exe: "node.exe".to_string() }, source_label: "Chocolatey".to_string(), priority: 40, root_offset: 1 },
                FindRule { pattern: ResolvePattern::FixedPath { path: "C:\\msys64\\mingw64\\bin".to_string(), exe: "node.exe".to_string() }, source_label: "MSYS2".to_string(), priority: 60, root_offset: 0 },
                FindRule { pattern: ResolvePattern::FixedPath { path: "C:\\Program Files\\nodejs".to_string(), exe: "node.exe".to_string() }, source_label: "Program Files".to_string(), priority: 70, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "\\nodejs\\".to_string(), exe: "node.exe".to_string() }, source_label: "系统 PATH".to_string(), priority: 80, root_offset: 1 }
            ],
            is_service: None,
            default_port: None,
            data_dir: None,
            log_dir: None,
        },
        SdkDef {
            id: "go".to_string(),
            display_name: "Go".to_string(),
            category: SdkCategory::Language,
            official_website: "https://go.dev".to_string(),
            has_cache: false,
            has_mirror: true,
            has_pkg: false,
            env_vars: vec![EnvVar { name: "GOROOT".to_string(), desc: "Go 安装根目录".to_string(), check_type: EnvCheckType::Path }, EnvVar { name: "GOPATH".to_string(), desc: "Go 工作区路径".to_string(), check_type: EnvCheckType::Path }, EnvVar { name: "GOBIN".to_string(), desc: "Go 二进制安装目录".to_string(), check_type: EnvCheckType::Path }, EnvVar { name: "GOCACHE".to_string(), desc: "Go 构建缓存目录".to_string(), check_type: EnvCheckType::Path }, EnvVar { name: "GOPROXY".to_string(), desc: "Go 模块代理地址".to_string(), check_type: EnvCheckType::NonEmpty }, EnvVar { name: "GONOSUMDB".to_string(), desc: "跳过 sum 校验 of 模块列表".to_string(), check_type: EnvCheckType::NonEmpty }, EnvVar { name: "GONOSUMCHECK".to_string(), desc: "跳过校验 of 模块列表".to_string(), check_type: EnvCheckType::NonEmpty }],
            find_rules: vec![
                FindRule { pattern: ResolvePattern::EnvBin { env: "GOROOT".to_string(), bin_sub: "bin".to_string(), exe: "go.exe".to_string() }, source_label: "环境变量 GOROOT".to_string(), priority: 5, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "scoop\\apps\\go".to_string(), exe: "go.exe".to_string() }, source_label: "Scoop".to_string(), priority: 40, root_offset: 1 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "chocolatey\\lib\\golang".to_string(), exe: "go.exe".to_string() }, source_label: "Chocolatey".to_string(), priority: 40, root_offset: 1 },
                FindRule { pattern: ResolvePattern::FixedPath { path: "C:\\Program Files\\Go\\bin".to_string(), exe: "go.exe".to_string() }, source_label: "Program Files".to_string(), priority: 70, root_offset: 1 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "\\go\\bin".to_string(), exe: "go.exe".to_string() }, source_label: "系统 PATH".to_string(), priority: 80, root_offset: 1 },
                FindRule { pattern: ResolvePattern::FixedPath { path: "C:\\Users".to_string(), exe: "go.exe".to_string() }, source_label: "用户目录".to_string(), priority: 90, root_offset: 0 }
            ],
            is_service: None,
            default_port: None,
            data_dir: None,
            log_dir: None,
        },
        SdkDef {
            id: "python".to_string(),
            display_name: "Python".to_string(),
            category: SdkCategory::Language,
            official_website: "https://www.python.org".to_string(),
            has_cache: true,
            has_mirror: true,
            has_pkg: true,
            env_vars: vec![EnvVar { name: "PYTHONHOME".to_string(), desc: "Python 解释器根目录".to_string(), check_type: EnvCheckType::Path }, EnvVar { name: "PYTHONPATH".to_string(), desc: "Python 模块搜索路径".to_string(), check_type: EnvCheckType::Path }, EnvVar { name: "PIP_CACHE_DIR".to_string(), desc: "pip 缓存目录".to_string(), check_type: EnvCheckType::Path }, EnvVar { name: "CONDA_PREFIX".to_string(), desc: "conda 环境目录".to_string(), check_type: EnvCheckType::Path }, EnvVar { name: "PYENV_ROOT".to_string(), desc: "pyenv-win 根目录".to_string(), check_type: EnvCheckType::Path }],
            find_rules: vec![
                FindRule { pattern: ResolvePattern::EnvBin { env: "CONDA_PREFIX".to_string(), bin_sub: "".to_string(), exe: "python.exe".to_string() }, source_label: "conda".to_string(), priority: 10, root_offset: 0 },
                FindRule { pattern: ResolvePattern::EnvBin { env: "PYENV_ROOT".to_string(), bin_sub: "pyenv-win\\shims".to_string(), exe: "python.exe".to_string() }, source_label: "pyenv-win".to_string(), priority: 15, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "scoop\\apps\\python".to_string(), exe: "python.exe".to_string() }, source_label: "Scoop".to_string(), priority: 40, root_offset: 1 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "chocolatey\\lib\\python".to_string(), exe: "python.exe".to_string() }, source_label: "Chocolatey".to_string(), priority: 40, root_offset: 1 },
                FindRule { pattern: ResolvePattern::FixedPath { path: "C:\\msys64\\mingw64\\bin".to_string(), exe: "python.exe".to_string() }, source_label: "MSYS2".to_string(), priority: 60, root_offset: 0 },
                FindRule { pattern: ResolvePattern::FixedPath { path: "C:\\Program Files\\Python313".to_string(), exe: "python.exe".to_string() }, source_label: "Program Files".to_string(), priority: 70, root_offset: 0 },
                FindRule { pattern: ResolvePattern::FixedPath { path: "C:\\Program Files\\Python312".to_string(), exe: "python.exe".to_string() }, source_label: "Program Files".to_string(), priority: 70, root_offset: 0 },
                FindRule { pattern: ResolvePattern::FixedPath { path: "C:\\Program Files\\Python311".to_string(), exe: "python.exe".to_string() }, source_label: "Program Files".to_string(), priority: 70, root_offset: 0 },
                FindRule { pattern: ResolvePattern::FixedPath { path: "C:\\Program Files\\Python310".to_string(), exe: "python.exe".to_string() }, source_label: "Program Files".to_string(), priority: 70, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "\\python3".to_string(), exe: "python.exe".to_string() }, source_label: "系统 PATH".to_string(), priority: 80, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "\\python2".to_string(), exe: "python.exe".to_string() }, source_label: "系统 PATH".to_string(), priority: 80, root_offset: 0 }
            ],
            is_service: None,
            default_port: None,
            data_dir: None,
            log_dir: None,
        },
        SdkDef {
            id: "java".to_string(),
            display_name: "Java".to_string(),
            category: SdkCategory::Language,
            official_website: "https://adoptium.net".to_string(),
            has_cache: false,
            has_mirror: false,
            has_pkg: false,
            env_vars: vec![EnvVar { name: "JAVA_HOME".to_string(), desc: "JDK 安装根目录".to_string(), check_type: EnvCheckType::Path }, EnvVar { name: "JDK_HOME".to_string(), desc: "JDK 根目录（替代变量）".to_string(), check_type: EnvCheckType::Path }, EnvVar { name: "JRE_HOME".to_string(), desc: "JRE 根目录".to_string(), check_type: EnvCheckType::Path }, EnvVar { name: "CLASSPATH".to_string(), desc: "Java 类库搜索路径".to_string(), check_type: EnvCheckType::NonEmpty }],
            find_rules: vec![
                FindRule { pattern: ResolvePattern::EnvBin { env: "JAVA_HOME".to_string(), bin_sub: "bin".to_string(), exe: "java.exe".to_string() }, source_label: "环境变量 JAVA_HOME".to_string(), priority: 5, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "scoop\\apps\\temurin".to_string(), exe: "java.exe".to_string() }, source_label: "Scoop (Temurin)".to_string(), priority: 40, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "scoop\\apps\\corretto".to_string(), exe: "java.exe".to_string() }, source_label: "Scoop (Corretto)".to_string(), priority: 40, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "scoop\\apps\\openjdk".to_string(), exe: "java.exe".to_string() }, source_label: "Scoop (OpenJDK)".to_string(), priority: 40, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "scoop\\apps\\zulu".to_string(), exe: "java.exe".to_string() }, source_label: "Scoop (Zulu)".to_string(), priority: 40, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "scoop\\apps\\liberica".to_string(), exe: "java.exe".to_string() }, source_label: "Scoop (Liberica)".to_string(), priority: 40, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "chocolatey\\lib\\temurin".to_string(), exe: "java.exe".to_string() }, source_label: "Chocolatey".to_string(), priority: 40, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "chocolatey\\lib\\corretto".to_string(), exe: "java.exe".to_string() }, source_label: "Chocolatey".to_string(), priority: 40, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "chocolatey\\lib\\jdk".to_string(), exe: "java.exe".to_string() }, source_label: "Chocolatey".to_string(), priority: 40, root_offset: 0 },
                FindRule { pattern: ResolvePattern::FixedPath { path: "C:\\Program Files\\Eclipse Adoptium".to_string(), exe: "java.exe".to_string() }, source_label: "Adoptium".to_string(), priority: 60, root_offset: 0 },
                FindRule { pattern: ResolvePattern::FixedPath { path: "C:\\Program Files\\Eclipse Foundation".to_string(), exe: "java.exe".to_string() }, source_label: "Eclipse".to_string(), priority: 60, root_offset: 0 },
                FindRule { pattern: ResolvePattern::FixedPath { path: "C:\\Program Files\\Java".to_string(), exe: "java.exe".to_string() }, source_label: "Program Files\\Java".to_string(), priority: 65, root_offset: 0 },
                FindRule { pattern: ResolvePattern::FixedPath { path: "C:\\Program Files\\Amazon Corretto".to_string(), exe: "java.exe".to_string() }, source_label: "Amazon Corretto".to_string(), priority: 60, root_offset: 0 },
                FindRule { pattern: ResolvePattern::FixedPath { path: "C:\\Program Files\\Zulu".to_string(), exe: "java.exe".to_string() }, source_label: "Azul Zulu".to_string(), priority: 60, root_offset: 0 },
                FindRule { pattern: ResolvePattern::FixedPath { path: "C:\\Program Files\\Microsoft".to_string(), exe: "java.exe".to_string() }, source_label: "Microsoft JDK".to_string(), priority: 60, root_offset: 0 },
                FindRule { pattern: ResolvePattern::FixedPath { path: "C:\\msys64\\mingw64\\bin".to_string(), exe: "java.exe".to_string() }, source_label: "MSYS2".to_string(), priority: 70, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "\\java\\jdk".to_string(), exe: "java.exe".to_string() }, source_label: "系统 PATH".to_string(), priority: 80, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "\\java\\jre".to_string(), exe: "java.exe".to_string() }, source_label: "系统 PATH".to_string(), priority: 80, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "\\adoptium".to_string(), exe: "java.exe".to_string() }, source_label: "系统 PATH".to_string(), priority: 80, root_offset: 0 }
            ],
            is_service: None,
            default_port: None,
            data_dir: None,
            log_dir: None,
        },
        SdkDef {
            id: "flutter".to_string(),
            display_name: "Flutter".to_string(),
            category: SdkCategory::Language,
            official_website: "https://flutter.dev".to_string(),
            has_cache: false,
            has_mirror: true,
            has_pkg: false,
            env_vars: vec![EnvVar { name: "FLUTTER_ROOT".to_string(), desc: "Flutter SDK 根目录".to_string(), check_type: EnvCheckType::Path }, EnvVar { name: "FLUTTER_STORAGE_BASE_URL".to_string(), desc: "Flutter 引擎下载 URL".to_string(), check_type: EnvCheckType::NonEmpty }, EnvVar { name: "PUB_HOSTED_URL".to_string(), desc: "Dart pub 包仓库地址".to_string(), check_type: EnvCheckType::NonEmpty }],
            find_rules: vec![
                FindRule { pattern: ResolvePattern::EnvBin { env: "FLUTTER_ROOT".to_string(), bin_sub: "bin".to_string(), exe: "flutter.bat".to_string() }, source_label: "环境变量 FLUTTER_ROOT".to_string(), priority: 10, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "scoop\\apps\\flutter".to_string(), exe: "flutter.bat".to_string() }, source_label: "Scoop".to_string(), priority: 40, root_offset: 1 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "chocolatey\\lib\\flutter".to_string(), exe: "flutter.bat".to_string() }, source_label: "Chocolatey".to_string(), priority: 40, root_offset: 1 },
                FindRule { pattern: ResolvePattern::FixedPath { path: "C:\\flutter\\bin".to_string(), exe: "flutter.bat".to_string() }, source_label: "C:\\flutter".to_string(), priority: 60, root_offset: 1 },
                FindRule { pattern: ResolvePattern::FixedPath { path: "C:\\src\\flutter\\bin".to_string(), exe: "flutter.bat".to_string() }, source_label: "C:\\src\\flutter".to_string(), priority: 60, root_offset: 1 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "\\flutter\\bin".to_string(), exe: "flutter.bat".to_string() }, source_label: "系统 PATH".to_string(), priority: 80, root_offset: 1 }
            ],
            is_service: None,
            default_port: None,
            data_dir: None,
            log_dir: None,
        },
        SdkDef {
            id: "rust".to_string(),
            display_name: "Rust".to_string(),
            category: SdkCategory::Language,
            official_website: "https://www.rust-lang.org".to_string(),
            has_cache: false,
            has_mirror: true,
            has_pkg: false,
            env_vars: vec![EnvVar { name: "CARGO_HOME".to_string(), desc: "Cargo 包管理器目录".to_string(), check_type: EnvCheckType::Path }, EnvVar { name: "RUSTUP_HOME".to_string(), desc: "Rustup 工具链目录".to_string(), check_type: EnvCheckType::Path }, EnvVar { name: "CARGO_TARGET_DIR".to_string(), desc: "Cargo 构建输出目录".to_string(), check_type: EnvCheckType::Path }],
            find_rules: vec![
                FindRule { pattern: ResolvePattern::EnvBin { env: "RUSTUP_HOME".to_string(), bin_sub: "".to_string(), exe: "rustc.exe".to_string() }, source_label: "rustup".to_string(), priority: 5, root_offset: 0 },
                FindRule { pattern: ResolvePattern::EnvBin { env: "CARGO_HOME".to_string(), bin_sub: "bin".to_string(), exe: "rustc.exe".to_string() }, source_label: "Cargo".to_string(), priority: 8, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "scoop\\apps\\rustup".to_string(), exe: "rustc.exe".to_string() }, source_label: "Scoop".to_string(), priority: 40, root_offset: 1 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "chocolatey\\lib\\rust".to_string(), exe: "rustc.exe".to_string() }, source_label: "Chocolatey".to_string(), priority: 40, root_offset: 1 },
                FindRule { pattern: ResolvePattern::FixedPath { path: "C:\\msys64\\mingw64\\bin".to_string(), exe: "rustc.exe".to_string() }, source_label: "MSYS2".to_string(), priority: 60, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "\\.cargo\\bin".to_string(), exe: "rustc.exe".to_string() }, source_label: ".cargo\\bin".to_string(), priority: 50, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "\\rustup\\".to_string(), exe: "rustc.exe".to_string() }, source_label: "rustup".to_string(), priority: 55, root_offset: 0 }
            ],
            is_service: None,
            default_port: None,
            data_dir: None,
            log_dir: None,
        },
        SdkDef {
            id: "bun".to_string(),
            display_name: "Bun".to_string(),
            category: SdkCategory::Language,
            official_website: "https://bun.sh".to_string(),
            has_cache: false,
            has_mirror: false,
            has_pkg: false,
            env_vars: vec![EnvVar { name: "BUN_INSTALL".to_string(), desc: "Bun 安装根目录".to_string(), check_type: EnvCheckType::Path }],
            find_rules: vec![
                FindRule { pattern: ResolvePattern::EnvBin { env: "BUN_INSTALL".to_string(), bin_sub: "bin".to_string(), exe: "bun.exe".to_string() }, source_label: "环境变量 BUN_INSTALL".to_string(), priority: 10, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "scoop\\apps\\bun".to_string(), exe: "bun.exe".to_string() }, source_label: "Scoop".to_string(), priority: 40, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "\\.bun\\bin".to_string(), exe: "bun.exe".to_string() }, source_label: ".bun\\bin".to_string(), priority: 50, root_offset: 0 },
                FindRule { pattern: ResolvePattern::FixedPath { path: "C:\\Users".to_string(), exe: "bun.exe".to_string() }, source_label: "用户目录".to_string(), priority: 90, root_offset: 0 }
            ],
            is_service: None,
            default_port: None,
            data_dir: None,
            log_dir: None,
        },
        SdkDef {
            id: "maven".to_string(),
            display_name: "Maven".to_string(),
            category: SdkCategory::LibManager,
            official_website: "https://maven.apache.org".to_string(),
            has_cache: true,
            has_mirror: true,
            has_pkg: false,
            env_vars: vec![EnvVar { name: "MAVEN_HOME".to_string(), desc: "Maven 安装根目录".to_string(), check_type: EnvCheckType::Path }, EnvVar { name: "M2_HOME".to_string(), desc: "Maven 根目录（旧版本）".to_string(), check_type: EnvCheckType::Path }],
            find_rules: vec![
                FindRule { pattern: ResolvePattern::EnvBin { env: "MAVEN_HOME".to_string(), bin_sub: "bin".to_string(), exe: "mvn.cmd".to_string() }, source_label: "环境变量 MAVEN_HOME".to_string(), priority: 10, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "scoop\\apps\\maven".to_string(), exe: "mvn.cmd".to_string() }, source_label: "Scoop".to_string(), priority: 40, root_offset: 1 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "\\apache-maven\\bin".to_string(), exe: "mvn.cmd".to_string() }, source_label: "系统 PATH".to_string(), priority: 80, root_offset: 1 }
            ],
            is_service: None,
            default_port: None,
            data_dir: None,
            log_dir: None,
        },
        SdkDef {
            id: "gradle".to_string(),
            display_name: "Gradle".to_string(),
            category: SdkCategory::LibManager,
            official_website: "https://gradle.org".to_string(),
            has_cache: false,
            has_mirror: false,
            has_pkg: false,
            env_vars: vec![EnvVar { name: "GRADLE_HOME".to_string(), desc: "Gradle 安装目录".to_string(), check_type: EnvCheckType::Path }, EnvVar { name: "GRADLE_USER_HOME".to_string(), desc: "Gradle 用户数据目录".to_string(), check_type: EnvCheckType::Path }],
            find_rules: vec![
                FindRule { pattern: ResolvePattern::EnvBin { env: "GRADLE_HOME".to_string(), bin_sub: "bin".to_string(), exe: "gradle.bat".to_string() }, source_label: "环境变量 GRADLE_HOME".to_string(), priority: 10, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "scoop\\apps\\gradle".to_string(), exe: "gradle.bat".to_string() }, source_label: "Scoop".to_string(), priority: 40, root_offset: 1 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "\\gradle\\bin".to_string(), exe: "gradle.bat".to_string() }, source_label: "系统 PATH".to_string(), priority: 80, root_offset: 1 }
            ],
            is_service: None,
            default_port: None,
            data_dir: None,
            log_dir: None,
        },
        SdkDef {
            id: "yarn".to_string(),
            display_name: "Yarn".to_string(),
            category: SdkCategory::LibManager,
            official_website: "https://yarnpkg.com".to_string(),
            has_cache: true,
            has_mirror: false,
            has_pkg: false,
            env_vars: vec![],
            find_rules: vec![
                FindRule { pattern: ResolvePattern::PathContains { keyword: "scoop\\apps\\yarn".to_string(), exe: "yarn.cmd".to_string() }, source_label: "Scoop".to_string(), priority: 40, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "\\.yarn\\bin".to_string(), exe: "yarn.cmd".to_string() }, source_label: ".yarn\\bin".to_string(), priority: 50, root_offset: 0 }
            ],
            is_service: None,
            default_port: None,
            data_dir: None,
            log_dir: None,
        },
        SdkDef {
            id: "pnpm".to_string(),
            display_name: "pnpm".to_string(),
            category: SdkCategory::LibManager,
            official_website: "https://pnpm.io".to_string(),
            has_cache: true,
            has_mirror: false,
            has_pkg: false,
            env_vars: vec![],
            find_rules: vec![
                FindRule { pattern: ResolvePattern::PathContains { keyword: "scoop\\apps\\pnpm".to_string(), exe: "pnpm.exe".to_string() }, source_label: "Scoop".to_string(), priority: 40, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "\\.pnpm\\".to_string(), exe: "pnpm.exe".to_string() }, source_label: ".pnpm".to_string(), priority: 50, root_offset: 0 }
            ],
            is_service: None,
            default_port: None,
            data_dir: None,
            log_dir: None,
        },
        SdkDef {
            id: "nuget".to_string(),
            display_name: "NuGet".to_string(),
            category: SdkCategory::LibManager,
            official_website: "https://www.nuget.org".to_string(),
            has_cache: true,
            has_mirror: false,
            has_pkg: false,
            env_vars: vec![EnvVar { name: "NUGET_PACKAGES".to_string(), desc: "NuGet 全局包缓存目录".to_string(), check_type: EnvCheckType::Path }],
            find_rules: vec![
                FindRule { pattern: ResolvePattern::PathContains { keyword: "\\.nuget\\packages".to_string(), exe: "nuget.exe".to_string() }, source_label: "NuGet cache".to_string(), priority: 80, root_offset: 0 }
            ],
            is_service: None,
            default_port: None,
            data_dir: None,
            log_dir: None,
        },
        SdkDef {
            id: "nginx".to_string(),
            display_name: "Nginx".to_string(),
            category: SdkCategory::Service,
            official_website: "https://nginx.org".to_string(),
            has_cache: false,
            has_mirror: false,
            has_pkg: false,
            env_vars: vec![EnvVar { name: "NGINX_HOME".to_string(), desc: "Nginx 安装根目录".to_string(), check_type: EnvCheckType::Path }],
            find_rules: vec![
                FindRule { pattern: ResolvePattern::PathContains { keyword: "scoop\\apps\\nginx".to_string(), exe: "nginx.exe".to_string() }, source_label: "Scoop".to_string(), priority: 40, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "chocolatey\\lib\\nginx".to_string(), exe: "nginx.exe".to_string() }, source_label: "Chocolatey".to_string(), priority: 40, root_offset: 0 },
                FindRule { pattern: ResolvePattern::FixedPath { path: "C:\\nginx".to_string(), exe: "nginx.exe".to_string() }, source_label: "C:\\nginx".to_string(), priority: 60, root_offset: 0 }
            ],
            is_service: Some(true),
            default_port: Some(80),
            data_dir: Some("html".to_string()),
            log_dir: Some("logs".to_string()),
        },
        SdkDef {
            id: "redis".to_string(),
            display_name: "Redis".to_string(),
            category: SdkCategory::Service,
            official_website: "https://redis.io".to_string(),
            has_cache: false,
            has_mirror: false,
            has_pkg: false,
            env_vars: vec![EnvVar { name: "REDIS_HOME".to_string(), desc: "Redis 安装根目录".to_string(), check_type: EnvCheckType::Path }],
            find_rules: vec![
                FindRule { pattern: ResolvePattern::EnvBin { env: "REDIS_HOME".to_string(), bin_sub: "".to_string(), exe: "redis-server.exe".to_string() }, source_label: "环境变量 REDIS_HOME".to_string(), priority: 10, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "scoop\\apps\\redis".to_string(), exe: "redis-server.exe".to_string() }, source_label: "Scoop".to_string(), priority: 40, root_offset: 0 },
                FindRule { pattern: ResolvePattern::FixedPath { path: "C:\\redis".to_string(), exe: "redis-server.exe".to_string() }, source_label: "C:\\redis".to_string(), priority: 60, root_offset: 0 }
            ],
            is_service: Some(true),
            default_port: Some(6379),
            data_dir: Some(".".to_string()),
            log_dir: Some(".".to_string()),
        },
        SdkDef {
            id: "mysql".to_string(),
            display_name: "MySQL".to_string(),
            category: SdkCategory::Service,
            official_website: "https://www.mysql.com".to_string(),
            has_cache: false,
            has_mirror: false,
            has_pkg: false,
            env_vars: vec![EnvVar { name: "MYSQL_HOME".to_string(), desc: "MySQL 安装根目录".to_string(), check_type: EnvCheckType::Path }],
            find_rules: vec![
                FindRule { pattern: ResolvePattern::EnvBin { env: "MYSQL_HOME".to_string(), bin_sub: "bin".to_string(), exe: "mysql.exe".to_string() }, source_label: "环境变量 MYSQL_HOME".to_string(), priority: 10, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "scoop\\apps\\mysql".to_string(), exe: "mysql.exe".to_string() }, source_label: "Scoop".to_string(), priority: 40, root_offset: 1 },
                FindRule { pattern: ResolvePattern::FixedPath { path: "C:\\Program Files\\MySQL\\MySQL Server".to_string(), exe: "mysql.exe".to_string() }, source_label: "Program Files".to_string(), priority: 70, root_offset: 1 }
            ],
            is_service: Some(true),
            default_port: Some(3306),
            data_dir: Some("data".to_string()),
            log_dir: Some("data".to_string()),
        },
        SdkDef {
            id: "mongodb".to_string(),
            display_name: "MongoDB".to_string(),
            category: SdkCategory::Service,
            official_website: "https://www.mongodb.com".to_string(),
            has_cache: false,
            has_mirror: false,
            has_pkg: false,
            env_vars: vec![EnvVar { name: "MONGO_HOME".to_string(), desc: "MongoDB 安装根目录".to_string(), check_type: EnvCheckType::Path }],
            find_rules: vec![
                FindRule { pattern: ResolvePattern::EnvBin { env: "MONGO_HOME".to_string(), bin_sub: "bin".to_string(), exe: "mongod.exe".to_string() }, source_label: "环境变量 MONGO_HOME".to_string(), priority: 10, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "scoop\\apps\\mongodb".to_string(), exe: "mongod.exe".to_string() }, source_label: "Scoop".to_string(), priority: 40, root_offset: 1 },
                FindRule { pattern: ResolvePattern::FixedPath { path: "C:\\Program Files\\MongoDB\\Server".to_string(), exe: "mongod.exe".to_string() }, source_label: "Program Files".to_string(), priority: 70, root_offset: 1 }
            ],
            is_service: Some(true),
            default_port: Some(27017),
            data_dir: Some("data".to_string()),
            log_dir: Some("mongod.log".to_string()),
        },
        SdkDef {
            id: "postgresql".to_string(),
            display_name: "PostgreSQL".to_string(),
            category: SdkCategory::Service,
            official_website: "https://www.postgresql.org".to_string(),
            has_cache: false,
            has_mirror: false,
            has_pkg: false,
            env_vars: vec![EnvVar { name: "PGDATA".to_string(), desc: "PostgreSQL 数据目录".to_string(), check_type: EnvCheckType::Path }, EnvVar { name: "PGHOME".to_string(), desc: "PostgreSQL 安装根目录".to_string(), check_type: EnvCheckType::Path }],
            find_rules: vec![
                FindRule { pattern: ResolvePattern::EnvBin { env: "PGHOME".to_string(), bin_sub: "bin".to_string(), exe: "psql.exe".to_string() }, source_label: "环境变量 PGHOME".to_string(), priority: 10, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "scoop\\apps\\postgresql".to_string(), exe: "psql.exe".to_string() }, source_label: "Scoop".to_string(), priority: 40, root_offset: 1 },
                FindRule { pattern: ResolvePattern::FixedPath { path: "C:\\Program Files\\PostgreSQL".to_string(), exe: "psql.exe".to_string() }, source_label: "Program Files".to_string(), priority: 70, root_offset: 1 }
            ],
            is_service: Some(true),
            default_port: Some(5432),
            data_dir: Some("data".to_string()),
            log_dir: Some("logfile".to_string()),
        },
        SdkDef {
            id: "android".to_string(),
            display_name: "Android SDK".to_string(),
            category: SdkCategory::Tool,
            official_website: "https://developer.android.com".to_string(),
            has_cache: false,
            has_mirror: false,
            has_pkg: false,
            env_vars: vec![EnvVar { name: "ANDROID_HOME".to_string(), desc: "Android SDK 根目录".to_string(), check_type: EnvCheckType::Path }, EnvVar { name: "ANDROID_SDK_ROOT".to_string(), desc: "Android SDK 根目录（旧版本）".to_string(), check_type: EnvCheckType::Path }, EnvVar { name: "ANDROID_SDK_HOME".to_string(), desc: "Android 用户数据目录".to_string(), check_type: EnvCheckType::Path }, EnvVar { name: "ANDROID_NDK_HOME".to_string(), desc: "Android NDK 目录".to_string(), check_type: EnvCheckType::Path }, EnvVar { name: "ANDROID_PREFS_ROOT".to_string(), desc: "Android 偏好设置目录".to_string(), check_type: EnvCheckType::Path }, EnvVar { name: "NDK_HOME".to_string(), desc: "NDK 根目录".to_string(), check_type: EnvCheckType::Path }],
            find_rules: vec![
                FindRule { pattern: ResolvePattern::EnvBin { env: "ANDROID_HOME".to_string(), bin_sub: "cmdline-tools\\latest\\bin".to_string(), exe: "sdkmanager.bat".to_string() }, source_label: "环境变量 ANDROID_HOME".to_string(), priority: 10, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "\\android-sdk".to_string(), exe: "sdkmanager.bat".to_string() }, source_label: "系统 PATH".to_string(), priority: 80, root_offset: 0 }
            ],
            is_service: None,
            default_port: None,
            data_dir: None,
            log_dir: None,
        },
        SdkDef {
            id: "harmony".to_string(),
            display_name: "鸿蒙 HarmonyOS".to_string(),
            category: SdkCategory::Tool,
            official_website: "https://developer.huawei.com/consumer/cn/harmony/".to_string(),
            has_cache: false,
            has_mirror: false,
            has_pkg: false,
            env_vars: vec![EnvVar { name: "OHOS_SDK_HOME".to_string(), desc: "鸿蒙 SDK 根目录".to_string(), check_type: EnvCheckType::Path }],
            find_rules: vec![
                FindRule { pattern: ResolvePattern::EnvBin { env: "OHOS_SDK_HOME".to_string(), bin_sub: "bin".to_string(), exe: "ohpm.bat".to_string() }, source_label: "环境变量 OHOS_SDK_HOME".to_string(), priority: 10, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "\\ohpm\\bin".to_string(), exe: "ohpm.bat".to_string() }, source_label: "系统 PATH".to_string(), priority: 80, root_offset: 0 }
            ],
            is_service: None,
            default_port: None,
            data_dir: None,
            log_dir: None,
        },
        SdkDef {
            id: "cuda".to_string(),
            display_name: "CUDA Toolkit".to_string(),
            category: SdkCategory::Tool,
            official_website: "https://developer.nvidia.com/cuda-toolkit".to_string(),
            has_cache: false,
            has_mirror: false,
            has_pkg: false,
            env_vars: vec![EnvVar { name: "CUDA_PATH".to_string(), desc: "CUDA Toolkit 安装目录".to_string(), check_type: EnvCheckType::Path }, EnvVar { name: "CUDA_HOME".to_string(), desc: "CUDA Toolkit 根目录".to_string(), check_type: EnvCheckType::Path }],
            find_rules: vec![
                FindRule { pattern: ResolvePattern::EnvBin { env: "CUDA_PATH".to_string(), bin_sub: "bin".to_string(), exe: "nvcc.exe".to_string() }, source_label: "环境变量 CUDA_PATH".to_string(), priority: 10, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "scoop\\apps\\cuda".to_string(), exe: "nvcc.exe".to_string() }, source_label: "Scoop".to_string(), priority: 40, root_offset: 1 },
                FindRule { pattern: ResolvePattern::FixedPath { path: "C:\\Program Files\\NVIDIA GPU Computing Toolkit\\CUDA".to_string(), exe: "nvcc.exe".to_string() }, source_label: "NVIDIA GPU".to_string(), priority: 60, root_offset: 1 }
            ],
            is_service: None,
            default_port: None,
            data_dir: None,
            log_dir: None,
        },
        SdkDef {
            id: "ffmpeg".to_string(),
            display_name: "FFmpeg".to_string(),
            category: SdkCategory::Tool,
            official_website: "https://ffmpeg.org".to_string(),
            has_cache: false,
            has_mirror: false,
            has_pkg: false,
            env_vars: vec![EnvVar { name: "FFMPEG_HOME".to_string(), desc: "FFmpeg 安装目录".to_string(), check_type: EnvCheckType::Path }],
            find_rules: vec![
                FindRule { pattern: ResolvePattern::EnvBin { env: "FFMPEG_HOME".to_string(), bin_sub: "bin".to_string(), exe: "ffmpeg.exe".to_string() }, source_label: "环境变量 FFMPEG_HOME".to_string(), priority: 10, root_offset: 0 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "scoop\\apps\\ffmpeg".to_string(), exe: "ffmpeg.exe".to_string() }, source_label: "Scoop".to_string(), priority: 40, root_offset: 1 },
                FindRule { pattern: ResolvePattern::PathContains { keyword: "\\ffmpeg\\".to_string(), exe: "ffmpeg.exe".to_string() }, source_label: "系统 PATH".to_string(), priority: 80, root_offset: 1 }
            ],
            is_service: None,
            default_port: None,
            data_dir: None,
            log_dir: None,
        }
    ]
}
