//! SDK 注册表 — 所有 SDK/工具/库的统一定义。
//!
//! 新增 SDK 只需在此文件添加一个 SdkDef 条目。以下功能自动生效：
//!   - 一键体检（环境变量 + PATH 扫描）
//!   - SDK 版本管理（安装/卸载/切换）
//!   - 环境变量自动配置/清理
//!
//! 重要：环境变量检查同时覆盖 用户级(HKCU) 和 系统级(HKLM) 注册表。

use serde::Serialize;
use super::sdk_resolver::{FindRule, ResolvePattern};

/// SDK 分类
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SdkCategory {
    Language,
    Service,
    BuildTool,
    Mobile,
    Tool,
}

impl SdkCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Language   => "language",
            Self::Service    => "service",
            Self::BuildTool  => "build_tool",
            Self::Mobile     => "mobile",
            Self::Tool       => "tool",
        }
    }
}

/// 环境变量检查类型
#[derive(Debug, Clone, Copy)]
pub enum EnvCheckType {
    Path,
    NonEmpty,
}

/// SDK 定义条目
pub struct SdkDef {
    pub id: &'static str,
    pub display_name: &'static str,
    pub category: SdkCategory,
    /// 该 SDK 关联的环境变量：(变量名, 用途, 检查类型)
    pub env_vars: &'static [(&'static str, &'static str, EnvCheckType)],
    /// 路径解析规则：如何在用户电脑上找到这个 SDK
    pub find_rules: &'static [FindRule],
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  辅助宏：简化规则定义
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

macro_rules! env_path {
    ($name:expr, $desc:expr) => { ($name, $desc, EnvCheckType::Path) };
}
macro_rules! env_str {
    ($name:expr, $desc:expr) => { ($name, $desc, EnvCheckType::NonEmpty) };
}

/// 在 PATH 中查找包含关键词的目录
macro_rules! path_match {
    ($key:expr, $exe:expr, $label:expr, $prio:expr, $offset:expr) => {
        FindRule {
            pattern: ResolvePattern::PathContains($key, $exe),
            source_label: $label,
            priority: $prio,
            root_offset: $offset,
        }
    };
    ($key:expr, $exe:expr, $label:expr) => {
        path_match!($key, $exe, $label, 50, 0)
    };
}

/// 从环境变量推导路径
macro_rules! env_match {
    ($env:expr, $bin:expr, $exe:expr, $label:expr, $prio:expr) => {
        FindRule {
            pattern: ResolvePattern::EnvBin($env, $bin, $exe),
            source_label: $label,
            priority: $prio,
            root_offset: 0,
        }
    };
    ($env:expr, $bin:expr, $exe:expr, $label:expr) => {
        env_match!($env, $bin, $exe, $label, 20)
    };
}

/// 固定路径检查
macro_rules! fixed_match {
    ($path:expr, $exe:expr, $label:expr, $prio:expr) => {
        FindRule {
            pattern: ResolvePattern::FixedPath($path, $exe),
            source_label: $label,
            priority: $prio,
            root_offset: 0,
        }
    };
}

/// 全局 SDK 注册表。新增 SDK 只需在此添加条目。
pub fn registry() -> &'static [SdkDef] {
    &[

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    //  编程语言
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    SdkDef {
        id: "nodejs",
        display_name: "Node.js",
        category: SdkCategory::Language,
        env_vars: &[
            env_path!("NODE_PATH",          "全局模块搜索路径"),
            env_path!("NPM_CONFIG_PREFIX",  "npm 全局安装前缀"),
            env_path!("NPM_CONFIG_CACHE",   "npm 缓存目录"),
            env_path!("NVM_DIR",            "nvm-windows 安装目录"),
            env_path!("NVM_HOME",           "nvm-windows 根目录"),
            env_path!("VOLTA_HOME",         "Volta 安装目录"),
        ],
        find_rules: &[
            // 优先级高：nvm-windows（通过 NVM_HOME 定位）
            env_match!("NVM_HOME", "", "node.exe", "nvm-windows", 10),
            // Volta
            env_match!("VOLTA_HOME", "bin", "node.exe", "Volta", 10),
            // Scoop
            path_match!("scoop\\apps\\nodejs", "node.exe", "Scoop", 40, 1),
            // Chocolatey
            path_match!("chocolatey\\lib\\nodejs", "node.exe", "Chocolatey", 40, 1),
            // MSYS2
            fixed_match!("C:\\msys64\\mingw64\\bin", "node.exe", "MSYS2", 60),
            // Program Files
            fixed_match!("C:\\Program Files\\nodejs", "node.exe", "Program Files", 70),
            // 通用
            path_match!("\\nodejs\\", "node.exe", "系统 PATH", 80, 1),
        ],
    },

    SdkDef {
        id: "go",
        display_name: "Go",
        category: SdkCategory::Language,
        env_vars: &[
            env_path!("GOROOT",      "Go 安装根目录"),
            env_path!("GOPATH",      "Go 工作区路径"),
            env_path!("GOBIN",       "Go 二进制安装目录"),
            env_path!("GOCACHE",     "Go 构建缓存目录"),
            env_str!("GOPROXY",     "Go 模块代理地址"),
            env_str!("GONOSUMDB",   "跳过 sum 校验的模块列表"),
            env_str!("GONOSUMCHECK","跳过校验的模块列表"),
        ],
        find_rules: &[
            env_match!("GOROOT", "bin", "go.exe", "环境变量 GOROOT", 5),
            path_match!("scoop\\apps\\go", "go.exe", "Scoop", 40, 1),
            path_match!("chocolatey\\lib\\golang", "go.exe", "Chocolatey", 40, 1),
            fixed_match!("C:\\Program Files\\Go\\bin", "go.exe", "Program Files", 70),
            path_match!("\\go\\bin", "go.exe", "系统 PATH", 80, 1),
            // Go 默认安装到用户目录
            fixed_match!("C:\\Users", "go.exe", "用户目录", 90),  // 特殊处理
        ],
    },

    SdkDef {
        id: "python",
        display_name: "Python",
        category: SdkCategory::Language,
        env_vars: &[
            env_path!("PYTHONHOME",     "Python 解释器根目录"),
            env_path!("PYTHONPATH",     "Python 模块搜索路径"),
            env_path!("PIP_CACHE_DIR",  "pip 缓存目录"),
            env_path!("CONDA_PREFIX",   "conda 环境目录"),
            env_path!("PYENV_ROOT",     "pyenv-win 根目录"),
        ],
        find_rules: &[
            // conda / Anaconda / Miniconda
            env_match!("CONDA_PREFIX", "", "python.exe", "conda", 10),
            // pyenv-win
            env_match!("PYENV_ROOT", "pyenv-win\\shims", "python.exe", "pyenv-win", 15),
            // Scoop
            path_match!("scoop\\apps\\python", "python.exe", "Scoop", 40, 1),
            // Chocolatey
            path_match!("chocolatey\\lib\\python", "python.exe", "Chocolatey", 40, 1),
            // MSYS2
            fixed_match!("C:\\msys64\\mingw64\\bin", "python.exe", "MSYS2", 60),
            // Program Files（多个版本）
            fixed_match!("C:\\Program Files\\Python313", "python.exe", "Program Files", 70),
            fixed_match!("C:\\Program Files\\Python312", "python.exe", "Program Files", 70),
            fixed_match!("C:\\Program Files\\Python311", "python.exe", "Program Files", 70),
            fixed_match!("C:\\Program Files\\Python310", "python.exe", "Program Files", 70),
            // 用户安装
            path_match!("\\python3", "python.exe", "系统 PATH", 80),
            path_match!("\\python2", "python.exe", "系统 PATH", 80),
        ],
    },

    SdkDef {
        id: "java",
        display_name: "Java",
        category: SdkCategory::Language,
        env_vars: &[
            env_path!("JAVA_HOME",  "JDK 安装根目录"),
            env_path!("JDK_HOME",   "JDK 根目录（替代变量）"),
            env_path!("JRE_HOME",   "JRE 根目录"),
            env_str!("CLASSPATH",  "Java 类库搜索路径"),
        ],
        find_rules: &[
            env_match!("JAVA_HOME", "bin", "java.exe", "环境变量 JAVA_HOME", 5),
            // Scoop（多个发行版）
            path_match!("scoop\\apps\\temurin",  "java.exe", "Scoop (Temurin)",  40),
            path_match!("scoop\\apps\\corretto", "java.exe", "Scoop (Corretto)", 40),
            path_match!("scoop\\apps\\openjdk",  "java.exe", "Scoop (OpenJDK)",  40),
            path_match!("scoop\\apps\\zulu",     "java.exe", "Scoop (Zulu)",     40),
            path_match!("scoop\\apps\\liberica", "java.exe", "Scoop (Liberica)", 40),
            // Chocolatey
            path_match!("chocolatey\\lib\\temurin",  "java.exe", "Chocolatey", 40),
            path_match!("chocolatey\\lib\\corretto", "java.exe", "Chocolatey", 40),
            path_match!("chocolatey\\lib\\jdk",      "java.exe", "Chocolatey", 40),
            // Adoptium / Eclipse Temurin
            fixed_match!("C:\\Program Files\\Eclipse Adoptium", "java.exe", "Adoptium", 60),
            fixed_match!("C:\\Program Files\\Eclipse Foundation", "java.exe", "Eclipse", 60),
            // Oracle JDK
            fixed_match!("C:\\Program Files\\Java", "java.exe", "Program Files\\Java", 65),
            // Amazon Corretto
            fixed_match!("C:\\Program Files\\Amazon Corretto", "java.exe", "Amazon Corretto", 60),
            // Azul Zulu
            fixed_match!("C:\\Program Files\\Zulu", "java.exe", "Azul Zulu", 60),
            // Microsoft JDK
            fixed_match!("C:\\Program Files\\Microsoft", "java.exe", "Microsoft JDK", 60),
            // MSYS2
            fixed_match!("C:\\msys64\\mingw64\\bin", "java.exe", "MSYS2", 70),
            // 通用
            path_match!("\\java\\jdk", "java.exe", "系统 PATH", 80),
            path_match!("\\java\\jre", "java.exe", "系统 PATH", 80),
            path_match!("\\adoptium",  "java.exe", "系统 PATH", 80),
        ],
    },

    SdkDef {
        id: "flutter",
        display_name: "Flutter",
        category: SdkCategory::Language,
        env_vars: &[
            env_path!("FLUTTER_ROOT",             "Flutter SDK 根目录"),
            env_str!("FLUTTER_STORAGE_BASE_URL", "Flutter 引擎下载 URL"),
            env_str!("PUB_HOSTED_URL",           "Dart pub 包仓库地址"),
        ],
        find_rules: &[
            env_match!("FLUTTER_ROOT", "bin", "flutter.bat", "环境变量 FLUTTER_ROOT", 10),
            path_match!("scoop\\apps\\flutter", "flutter.bat", "Scoop", 40, 1),
            path_match!("chocolatey\\lib\\flutter", "flutter.bat", "Chocolatey", 40, 1),
            fixed_match!("C:\\flutter\\bin", "flutter.bat", "C:\\flutter", 60),
            fixed_match!("C:\\src\\flutter\\bin", "flutter.bat", "C:\\src\\flutter", 60),
            path_match!("\\flutter\\bin", "flutter.bat", "系统 PATH", 80, 1),
        ],
    },

    SdkDef {
        id: "rust",
        display_name: "Rust",
        category: SdkCategory::Language,
        env_vars: &[
            env_path!("CARGO_HOME",         "Cargo 包管理器目录"),
            env_path!("RUSTUP_HOME",        "Rustup 工具链目录"),
            env_path!("CARGO_TARGET_DIR",   "Cargo 构建输出目录"),
        ],
        find_rules: &[
            // rustup（优先级最高）
            env_match!("RUSTUP_HOME", "", "rustc.exe", "rustup", 5),
            env_match!("CARGO_HOME", "bin", "rustc.exe", "Cargo", 8),
            // Scoop
            path_match!("scoop\\apps\\rustup", "rustc.exe", "Scoop", 40, 1),
            // Chocolatey
            path_match!("chocolatey\\lib\\rust", "rustc.exe", "Chocolatey", 40, 1),
            // MSYS2
            fixed_match!("C:\\msys64\\mingw64\\bin", "rustc.exe", "MSYS2", 60),
            // 通用
            path_match!("\\.cargo\\bin", "rustc.exe", ".cargo\\bin", 50),
            path_match!("\\rustup\\",    "rustc.exe", "rustup", 55),
        ],
    },

    SdkDef {
        id: "bun",
        display_name: "Bun",
        category: SdkCategory::Language,
        env_vars: &[
            env_path!("BUN_INSTALL", "Bun 安装根目录"),
        ],
        find_rules: &[
            env_match!("BUN_INSTALL", "bin", "bun.exe", "环境变量 BUN_INSTALL", 10),
            path_match!("scoop\\apps\\bun", "bun.exe", "Scoop", 40),
            path_match!("\\.bun\\bin", "bun.exe", ".bun\\bin", 50),
            fixed_match!("C:\\Users", "bun.exe", "用户目录", 90), // 特殊
        ],
    },

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    //  本地服务
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    SdkDef {
        id: "nginx",
        display_name: "Nginx",
        category: SdkCategory::Service,
        env_vars: &[env_path!("NGINX_HOME", "Nginx 安装根目录")],
        find_rules: &[
            path_match!("scoop\\apps\\nginx", "nginx.exe", "Scoop", 40),
            path_match!("chocolatey\\lib\\nginx", "nginx.exe", "Chocolatey", 40),
            fixed_match!("C:\\nginx", "nginx.exe", "C:\\nginx", 60),
     