//! SDK 注册表 — 所有 SDK/工具/库的统一定义。
//!
//! 新增 SDK 时只需在此文件添加一个条目，扫描、安装、卸载、环境变量配置
//! 等功能将自动生效，无需修改其他文件。
//!
//! 重要：环境变量检查同时覆盖 用户级(HKCU) 和 系统级(HKLM) 注册表。

use serde::Serialize;

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
    /// 路径类变量：值应是一个存在的目录，不存在则报严重
    Path,
    /// 非空字符串类变量（URL、列表等）：仅记录，不主动报错
    NonEmpty,
}

/// SDK 定义条目
pub struct SdkDef {
    pub id: &'static str,
    pub display_name: &'static str,
    pub category: SdkCategory,
    /// 安装目录内，可执行文件所在子路径（用于 PATH 检测模式）
    pub exe_path: &'static str,
    /// 安装目录内，bin 子路径（用于 PATH 检测模式）
    pub bin_path: &'static str,
    /// 该 SDK 关联的环境变量：(变量名, 用途, 检查类型)
    pub env_vars: &'static [(&'static str, &'static str, EnvCheckType)],
    /// PATH 中的外部安装路径模式（用于检测未管理的安装）
    /// (路径包含的关键词, 可执行文件名)
    pub path_patterns: &'static [(&'static str, &'static str)],
}

macro_rules! env_path {
    ($name:expr, $desc:expr) => { ($name, $desc, EnvCheckType::Path) };
}
macro_rules! env_str {
    ($name:expr, $desc:expr) => { ($name, $desc, EnvCheckType::NonEmpty) };
}

/// 全局 SDK 注册表。新增 SDK 只需在此添加条目。
pub fn registry() -> &'static [SdkDef] {
    &[
        // ━━━ 编程语言 ━━━
        SdkDef {
            id: "nodejs",
            display_name: "Node.js",
            category: SdkCategory::Language,
            exe_path: "node.exe",
            bin_path: "bin",
            env_vars: &[
                env_path!("NODE_PATH",          "全局模块搜索路径"),
                env_path!("NPM_CONFIG_PREFIX",  "npm 全局安装前缀"),
                env_path!("NPM_CONFIG_CACHE",   "npm 缓存目录"),
                env_path!("NVM_DIR",            "nvm 安装目录"),
            ],
            path_patterns: &[
                ("scoop\\apps\\nodejs", "node.exe"),
                ("\\nodejs\\",         "node.exe"),
                ("\\nvm\\",            "node.exe"),
            ],
        },
        SdkDef {
            id: "go",
            display_name: "Go",
            category: SdkCategory::Language,
            exe_path: "go.exe",
            bin_path: "bin",
            env_vars: &[
                env_path!("GOROOT",     "Go 安装根目录"),
                env_path!("GOPATH",     "Go 工作区路径"),
                env_path!("GOBIN",      "Go 二进制安装目录"),
                env_path!("GOCACHE",    "Go 构建缓存目录"),
                env_str!("GOPROXY",    "Go 模块代理地址"),
                env_str!("GONOSUMDB",  "跳过 sum 校验的模块列表"),
                env_str!("GONOSUMCHECK","跳过校验的模块列表"),
            ],
            path_patterns: &[
                ("scoop\\apps\\go",    "go.exe"),
                ("\\go\\bin",         "go.exe"),
            ],
        },
        SdkDef {
            id: "python",
            display_name: "Python",
            category: SdkCategory::Language,
            exe_path: "python.exe",
            bin_path: "",
            env_vars: &[
                env_path!("PYTHONHOME",     "Python 解释器根目录"),
                env_path!("PYTHONPATH",     "Python 模块搜索路径"),
                env_path!("PIP_CACHE_DIR",  "pip 缓存目录"),
                env_path!("VIRTUALENV_HOME","virtualenv 默认目录"),
            ],
            path_patterns: &[
                ("scoop\\apps\\python", "python.exe"),
                ("\\python3",          "python.exe"),
                ("\\python2",          "python.exe"),
            ],
        },
        SdkDef {
            id: "java",
            display_name: "Java",
            category: SdkCategory::Language,
            exe_path: "java.exe",
            bin_path: "bin",
            env_vars: &[
                env_path!("JAVA_HOME",  "JDK 安装根目录"),
                env_path!("JDK_HOME",   "JDK 根目录（替代变量）"),
                env_path!("JRE_HOME",   "JRE 根目录"),
                env_str!("CLASSPATH",  "Java 类库搜索路径"),
            ],
            path_patterns: &[
                ("scoop\\apps\\temurin",  "java.exe"),
                ("scoop\\apps\\corretto", "java.exe"),
                ("scoop\\apps\\openjdk",  "java.exe"),
                ("scoop\\apps\\zulu",     "java.exe"),
                ("\\adoptium",            "java.exe"),
                ("\\eclipse-temurin",     "java.exe"),
                ("\\java\\jdk",           "java.exe"),
                ("\\java\\jre",           "java.exe"),
            ],
        },
        SdkDef {
            id: "flutter",
            display_name: "Flutter",
            category: SdkCategory::Language,
            exe_path: "flutter.bat",
            bin_path: "bin",
            env_vars: &[
                env_path!("FLUTTER_ROOT",             "Flutter SDK 根目录"),
                env_str!("FLUTTER_STORAGE_BASE_URL", "Flutter 引擎下载 URL"),
                env_str!("PUB_HOSTED_URL",           "Dart pub 包仓库地址"),
            ],
            path_patterns: &[
                ("scoop\\apps\\flutter", "flutter.bat"),
            ],
        },
        SdkDef {
            id: "rust",
            display_name: "Rust",
            category: SdkCategory::Language,
            exe_path: "rustc.exe",
            bin_path: "bin",
            env_vars: &[
                env_path!("CARGO_HOME",         "Cargo 包管理器目录"),
                env_path!("RUSTUP_HOME",        "Rustup 工具链目录"),
                env_path!("CARGO_TARGET_DIR",   "Cargo 构建输出目录"),
            ],
            path_patterns: &[
                ("scoop\\apps\\rustup", "rustc.exe"),
                ("\\.cargo\\bin",      "rustc.exe"),
            ],
        },
        SdkDef {
            id: "bun",
            display_name: "Bun",
            category: SdkCategory::Language,
            exe_path: "bun.exe",
            bin_path: "",
            env_vars: &[
                env_path!("BUN_INSTALL", "Bun 安装根目录"),
            ],
            path_patterns: &[
                ("scoop\\apps\\bun", "bun.exe"),
            ],
        },

        // ━━━ 本地服务 ━━━
        SdkDef {
            id: "nginx",
            display_name: "Nginx",
            category: SdkCategory::Service,
            exe_path: "nginx.exe",
            bin_path: "",
            env_vars: &[
                env_path!("NGINX_HOME", "Nginx 安装根目录"),
            ],
            path_patterns: &[
                ("scoop\\apps\\nginx", "nginx.exe"),
                ("\\nginx\\",         "nginx.exe"),
            ],
        },
        SdkDef {
            id: "redis",
            display_name: "Redis",
            category: SdkCategory::Service,
            exe_path: "redis-server.exe",
            bin_path: "",
            env_vars: &[
                env_path!("REDIS_HOME", "Redis 安装根目录"),
            ],
            path_patterns: &[
                ("scoop\\apps\\redis", "redis-server.exe"),
            ],
        },
        SdkDef {
            id: "mysql",
            display_name: "MySQL",
            category: SdkCategory::Service,
            exe_path: "mysql.exe",
            bin_path: "bin",
            env_vars: &[
                env_path!("MYSQL_HOME", "MySQL 安装根目录"),
            ],
            path_patterns: &[
                ("scoop\\apps\\mysql", "mysql.exe"),
            ],
        },
        SdkDef {
            id: "mongodb",
            display_name: "MongoDB",
            category: SdkCategory::Service,
            exe_path: "mongod.exe",
            bin_path: "bin",
            env_vars: &[
                env_path!("MONGO_HOME", "MongoDB 安装根目录"),
            ],
            path_patterns: &[
                ("scoop\\apps\\mongodb", "mongod.exe"),
            ],
        },
        SdkDef {
            id: "postgresql",
            display_name: "PostgreSQL",
            category: SdkCategory::Service,
            exe_path: "psql.exe",
            bin_path: "bin",
            env_vars: &[
                env_path!("PGDATA", "PostgreSQL 数据目录"),
                env_path!("PGHOME", "PostgreSQL 安装根目录"),
            ],
            path_patterns: &[
                ("scoop\\apps\\postgresql", "psql.exe"),
            ],
        },

        // ━━━ 构建工具 ━━━
        SdkDef {
            id: "maven",
            display_name: "Maven",
            category: SdkCategory::BuildTool,
            exe_path: "mvn.cmd",
            bin_path: "bin",
            env_vars: &[
                env_path!("MAVEN_HOME", "Maven 安装根目录"),
                env_path!("M2_HOME",    "Maven 根目录（旧版）"),
            ],
            path_patterns: &[
                ("scoop\\apps\\maven", "mvn.cmd"),
            ],
        },
        SdkDef {
            id: "gradle",
            display_name: "Gradle",
            category: SdkCategory::BuildTool,
            exe_path: "gradle.bat",
            bin_path: "bin",
            env_vars: &[
                env_path!("GRADLE_HOME",      "Gradle 安装目录"),
                env_path!("GRADLE_USER_HOME", "Gradle 用户数据目录"),
            ],
            path_patterns: &[
                ("scoop\\apps\\gradle", "gradle.bat"),
            ],
        },
        SdkDef {
            id: "yarn",
            display_name: "Yarn",
            category: SdkCategory::BuildTool,
            exe_path: "yarn.cmd",
            bin_path: "",
            env_vars: &[],
            path_patterns: &[],
        },
        SdkDef {
            id: "pnpm",
            display_name: "pnpm",
            category: SdkCategory::BuildTool,
            exe_path: "pnpm.exe",
            bin_path: "",
            env_vars: &[],
            path_patterns: &[],
        },

        // ━━━ 移动端 SDK ━━━
        SdkDef {
            id: "android",
            display_name: "Android SDK",
            category: SdkCategory::Mobile,
            exe_path: "sdkmanager.bat",
            bin_path: "cmdline-tools\\latest\\bin",
            env_vars: &[
                env_path!("ANDROID_HOME",       "Android SDK 根目录"),
                env_path!("ANDROID_SDK_ROOT",   "Android SDK 根目录（旧版）"),
                env_path!("ANDROID_SDK_HOME",   "Android 用户数据目录"),
                env_path!("ANDROID_NDK_HOME",   "Android NDK 目录"),
                env_path!("ANDROID_PREFS_ROOT", "Android 偏好设置目录"),
                env_path!("NDK_HOME",           "NDK 根目录"),
            ],
            path_patterns: &[],
        },
        SdkDef {
            id: "harmony",
            display_name: "鸿蒙 HarmonyOS",
            category: SdkCategory::Mobile,
            exe_path: "ohpm.bat",
            bin_path: "bin",
            env_vars: &[
                env_path!("OHOS_SDK_HOME", "鸿蒙 SDK 根目录"),
            ],
            path_patterns: &[],
        },

        // ━━━ 开发工具 ━━━
        SdkDef {
            id: "cuda",
            display_name: "CUDA Toolkit",
            category: SdkCategory::Tool,
            exe_path: "nvcc.exe",
            bin_path: "bin",
            env_vars: &[
                env_path!("CUDA_PATH", "CUDA Toolkit 安装目录"),
                env_path!("CUDA_HOME", "CUDA Toolkit 根目录"),
            ],
            path_patterns: &[
                ("scoop\\apps\\cuda",           "nvcc.exe"),
                ("\\nvidia gpu computing toolkit", "nvcc.exe"),
            ],
        },
        SdkDef {
            id: "ffmpeg",
            display_name: "FFmpeg",
            category: SdkCategory::Tool,
            exe_path: "ffmpeg.exe",
            bin_path: "bin",
            env_vars: &[
                env_path!("FFMPEG_HOME", "FFmpeg 安装目录"),
            ],
            path_patterns: &[
                ("scoop\\apps\\ffmpeg", "ffmpeg.exe"),
                ("\\ffmpeg\\",         "ffmpeg.exe"),
            ],
        },
    ]
}

/// 根据 id 查找 SDK 定义
pub fn find_by_id(id: &str) -> Option<&'static SdkDef> {
    registry().iter().find(|s| s.id == id)
}

/// 返回所有 SDK id 列表（用于遍历）
pub fn all_ids() -> Vec<&'static str> {
    registry().iter().map(|s| s.id).collect()
}
