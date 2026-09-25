//! AI 工具注册表 —— 从 JSON 配置文件加载工具定义、Provider 预设、终端配置等。
//! 参考 EchoBird 的 tools/ 目录结构：每个工具一个 config.json + paths.json。
//! 新增工具 = 在 ai-tools/ 目录下添加 JSON 文件，零代码改动。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use super::ai::skills::skills_dir;

// ─── JSON 类型定义 ───

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolConfig {
    pub id: String,
    pub display_name: String,
    /// 协同模式中显示的头像（emoji 或文字，如 "🤖"）。为空时由前端按 id 回退。
    #[serde(default)]
    pub avatar: Option<String>,
    /// 协同模式中显示的昵称覆盖（为空时使用 display_name）。
    #[serde(default)]
    pub nickname: Option<String>,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub website: String,
    /// 官网地址（优先于 website；website 可能指向 GitHub 仓库）
    #[serde(default)]
    pub homepage: Option<String>,
    /// GitHub 仓库地址（homepage / website 均缺省时兜底）
    #[serde(default)]
    pub github: Option<String>,
    /// 工具「原生」协议（兼容旧逻辑：one_m 后缀、配置清理判定）。
    /// 新逻辑以 supports_openai/anthropic/google 三标志为准。
    #[serde(default)]
    pub api_protocol: String,
    #[serde(default)]
    pub support_model: bool,
    #[serde(default)]
    pub support_fallback_model: bool,
    #[serde(default)]
    pub support_one_m_context: bool,
    /// 工具支持的入站协议（代理为这些协议都注册路由）。
    #[serde(default)]
    pub supports_openai: bool,
    #[serde(default)]
    pub supports_anthropic: bool,
    #[serde(default)]
    pub supports_google: bool,
    /// 内置模型名称列表（伪装模型名的预设值 C）。
    /// 非空时用户可把所选取的供应商模型 B「伪装」成其中某项 C。
    #[serde(default)]
    pub builtin_models: Vec<String>,
    /// 该工具是否支持请求优化（启动页可开关）
    #[serde(default)]
    pub supports_optimizer: bool,
    /// 该工具是否支持抹平协议差异（整流器，启动页可开关）
    #[serde(default)]
    pub supports_rectifier: bool,
    pub resume_cmd: Option<String>,
    pub continue_cmd: Option<String>,
    // 以下都是「可有可无」的声明。必须给 `#[serde(default)]`：漏一个字段整份 config.json
    // 就解析失败，而这个工具会被**整个从注册表里丢掉**（表现为「列表里莫名少了一个工具」，
    // 只有 stderr 上一行 parse 失败，界面完全看不出来）。现状：omp 因为少写 cacheDirs
    // 一直没进注册表。
    #[serde(default)]
    pub cache_dirs: Vec<String>,
    #[serde(default)]
    pub pkg_manager: Option<String>,
    #[serde(default)]
    pub pkg_name: Option<String>,
    #[serde(default)]
    pub config_file: Option<ConfigFileDef>,
    #[serde(default)]
    pub model_format: Option<ModelFormatDef>,
    #[serde(default)]
    pub sessions: Option<SessionScanDef>,
    #[serde(default)]
    pub skills_dir: Option<String>,
    /// 非交互派发命令模板（协同模式）：`{prompt_file}` 占位符会被替换为提示词文件路径（已加引号）。
    #[serde(default)]
    pub dispatch_cmd: Option<String>,
    /// 续聊派发命令模板：同一房间内对同一工具的后续派发走此模板（如 claude --continue），实现上下文连续。
    #[serde(default)]
    pub dispatch_continue_cmd: Option<String>,
    /// 续聊派发命令模板（带 {session_id} 占位，按工具原生会话 id 精确恢复上下文）。
    #[serde(default)]
    pub dispatch_resume_cmd: Option<String>,
    /// 派发运行模式："stream-json"(claude) / "codex-json"(codex) / "opencode-json"(opencode)，省略则一次性读取输出（兜底）。
    #[serde(default)]
    pub runner: Option<String>,
    /// 提示词传入方式："file"(--input-file 占位) / "stdin"(子进程 stdin 喂临时文件) / "arg"(`{prompt}` 内联)，省略默认 file。
    #[serde(default)]
    pub prompt_mode: Option<String>,
    #[serde(default)]
    pub skills_dir_xdg: Option<String>,
}

impl ToolConfig {
    /// 工具支持的入站协议列表（用于代理为每种协议注册路由）。
    /// 若三个标志都为空（旧配置），按 `api_protocol` 回退推导。
    pub fn inbound_protocols(&self) -> Vec<String> {
        let mut v = Vec::new();
        if self.supports_openai { v.push("openai".to_string()); }
        if self.supports_anthropic { v.push("anthropic".to_string()); }
        if self.supports_google { v.push("google".to_string()); }
        if v.is_empty() {
            // 旧配置兜底：both → anthropic
            match self.api_protocol.as_str() {
                "anthropic" | "both" => v.push("anthropic".to_string()),
                "google" => v.push("google".to_string()),
                _ => v.push("openai".to_string()),
            }
        }
        v
    }

    /// 工具的「原生」协议：用于协议转换消息展示与 one_m 后缀判定。
    pub fn native_protocol(&self) -> String {
        if self.supports_anthropic { "anthropic".to_string() }
        else if self.supports_google { "google".to_string() }
        else { "openai".to_string() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigFileDef {
    pub path: String,
    pub format: String,
    #[serde(default)]
    pub schema: Option<String>,
    pub write: Option<HashMap<String, String>>,
    /// 自定义写入器：schema 复杂到「路径 → 值」表达不了的（WorkBuddy 的 models.json 就是
    /// `{models:[{id,name,vendor,url,apiKey,…}], availableModels:[…]}`），交给 Rust 侧整份生成。
    ///
    /// 抄 EchoBird：它的 `tools/*/config.json` 用 `"custom": true` 标记，实际写入在
    /// `src-tauri/src/services/tool_config_manager/<tool>.rs`（我们对应
    /// `commands/ai/tool_config_custom.rs`）。`true` = 用工具 id 当写入器名；也可以写字符串
    /// 显式指定（如 workbuddyai 复用 `"workbuddy"` 写入器，只是路径不同）。
    #[serde(default, alias = "customWriter")]
    pub custom: Option<CustomWriter>,
    /// 目录级环境变量覆盖（按顺序取第一个非空值作为配置目录）：如 OpenCode v2 的
    /// `OPENCODE_CONFIG_DIR`。目录确定后文件名沿用 `path` 的文件名。
    /// 抄自 EchoBird c6f4bc25。
    #[serde(default, alias = "pathEnvDirs")]
    pub path_env_dirs: Vec<String>,
    /// 用 `XDG_CONFIG_HOME` 拼接的子目录名（如 `opencode`），在 `path_env_dirs` 之后生效。
    #[serde(default, alias = "xdgSubdir")]
    pub xdg_subdir: Option<String>,
    /// 同 stem 的其它扩展名文件已存在时优先沿用它（如 OpenCode v2 的 `opencode.jsonc`）。
    #[serde(default, alias = "preferExistingExtensions")]
    pub prefer_existing_extensions: Vec<String>,
}

/// `configFile.custom` 的两种写法（untagged：`true` 或 `"workbuddy"`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CustomWriter {
    Flag(bool),
    Key(String),
}

impl ConfigFileDef {
    /// 该工具要用的自定义写入器名（None = 走通用的「路径 → 值」写入）。
    pub fn custom_writer(&self, tool_id: &str) -> Option<String> {
        match self.custom.as_ref()? {
            // `"custom": true` → 工具 id 就是写入器名（EchoBird 的写法）
            CustomWriter::Flag(true) => Some(tool_id.to_string()),
            CustomWriter::Flag(false) => None,
            CustomWriter::Key(key) => {
                let key = key.trim();
                if key.is_empty() {
                    None
                } else {
                    Some(key.to_string())
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelFormatDef {
    pub prefix: Option<String>,
    #[serde(default)]
    pub extract_last: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionScanDef {
    #[serde(rename = "type")]
    pub scan_type: String,
    #[serde(default)]
    pub dirs: Vec<String>,
}

/// 非标准安装路径的检测提示。各字段仅在对应平台上生效。
/// 用于 GUI/桌面应用在 paths.json 硬编码路径之外的检测：
/// Windows 扫描注册表 Uninstall 键、macOS 查找 /Applications、Linux 扫描 .desktop。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InstallHints {
    /// Windows: 匹配注册表 `DisplayName`（精确、大小写不敏感）。
    #[serde(default)]
    pub windows_display_names: Vec<String>,
    /// Windows: 前缀匹配（用于 DisplayName 内嵌版本号的应用，如 "WorkBuddy 4.24.2"）。
    #[serde(default)]
    pub windows_display_name_prefixes: Vec<String>,
    /// Windows: 可选的 `Publisher` 过滤（消歧义）。
    #[serde(default)]
    pub windows_publisher: Option<String>,
    /// macOS: 要搜索的 `.app` 名称（在 /Applications 与 ~/Applications 下查找）。
    #[serde(default)]
    pub macos_app_name: Option<String>,
    /// Linux: 匹配 .desktop 文件 `Name=` 的名称列表。
    #[serde(default)]
    pub linux_desktop_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathConfig {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub api_protocol: Vec<String>,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub start_command: String,
    #[serde(default)]
    pub detect_cmd: String,
    #[serde(default)]
    pub install_cmd: String,
    #[serde(default)]
    pub uninstall_cmd: Option<String>,
    #[serde(default)]
    pub paths: HashMap<String, Vec<String>>,
    /// MSIX/Store 应用启动 URI（如 "shell:AppsFolder\\Claude_...!Claude"），
    /// 用于没有普通 .exe 路径的应用启动与检测。
    #[serde(default)]
    pub launch_uri: Option<String>,
    /// 非标准安装路径的检测提示（注册表 / Applications / .desktop）。
    #[serde(default)]
    pub install_hints: Option<InstallHints>,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderPreset {
    pub id: String,
    pub name: String,
    pub category: String,
    pub website: String,
    pub openai_url: String,
    pub anthropic_url: String,
    #[serde(default)]
    pub google_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsScanConfig {
    pub description: String,
    pub base_skills_dir: String,
    pub tool_skills_dirs: HashMap<String, Vec<String>>,
    /// 会主动读取 canonical 目录（~/.agents/skills）的工具列表。
    /// 这些工具不应再对其自身 skills 目录建 junction，否则同一批技能
    /// 会被「原生目录(junction→canonical)」和「canonical 直读」各读一遍 → 重复告警。
    #[serde(default)]
    pub reads_agents_skills: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolConfig {
    /// 该工具的中心 MCP 配置文件路径（支持 ~ 与 %VAR%）
    pub config_path: String,
    /// 配置格式：`claude`(Claude Code) | `gemini`(Qwen/Gemini) | `opencode`(OpenCode 系)
    pub format: String,
    /// 外部指定的 MCP 根 Key（如 "mcp" 或 "mcpServers"）
    #[serde(default)]
    pub mcp_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct McpConfig {
    pub description: String,
    pub tools: HashMap<String, McpToolConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalsConfig {
    pub description: String,
    pub terminals: HashMap<String, TerminalDef>,
    pub proxy_settings: ProxySettingsDef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalDef {
    pub name: String,
    pub exe_names: Vec<String>,
    #[serde(default)]
    pub exe_path: Option<String>,
    #[serde(default)]
    pub always: bool,
    #[serde(default)]
    pub launch_args: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxySettingsDef {
    pub default_port: u16,
    pub listen_address: String,
    pub timeout_seconds: u32,
}

// ─── 编译后嵌入的运行时结构 ───

/// 与前端交互的 Provider 预设 DTO。
///
/// 注意：**不要**加 `rename_all = "camelCase"`。前端 `Preset` 类型与 AI 配置面
/// （`AiProvider`）一致按 snake_case 读 `openai_url` 等字段；一旦序列化成
/// `openaiUrl`，前端取不到 URL → 「添加预设供应商」只剩名称和官网。
/// （camelCase rename 只属于上面反序列化 `providers.json` 的 `ProviderPreset`。）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderPresetDto {
    pub id: String,
    pub name: String,
    pub category: String,
    pub website: String,
    pub openai_url: String,
    pub anthropic_url: String,
    pub google_url: String,
}

/// 工具配置文件的简要信息（给前端看「模型会写到哪里」）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolConfigFileDto {
    pub path: String,
    pub format: String,
}

/// 由 paths.json 的形态分类推导粗粒度归类（CLI / 桌面端 / 其它）。
///
/// EchoBird 用 `category` 直接分组，这里多做一层归一：Desktop/IDE 都算
/// 「桌面端」，CLI Code 算「命令行」，将来新增分类也不会让前端分组漏掉。
pub fn tool_kind_of(category: Option<&str>) -> String {
    match category.unwrap_or_default().trim().to_lowercase().as_str() {
        "cli code" | "cli" => "cli".to_string(),
        "desktop" | "ide" => "desktop".to_string(),
        other if other.is_empty() => "other".to_string(),
        _ => "other".to_string(),
    }
}

/// 与前端交互的工具定义（从 JSON 构建）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiToolDefDto {
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub nickname: Option<String>,
    pub installed: bool,
    /// 该工具是否**确实由它声明的包管理器安装**（npm/pip 全局注册表里查得到）。
    ///
    /// 仅供前端提示「这不是包管理器装的」，**不再**用来关闭升级/卸载入口：
    /// 查不到往往只是 Kira 用的 npm 前缀与用户安装时不同，工具本身仍装在用户的
    /// node/pip 下。后端此时会依次尝试「声明的包管理器 → 官方命令 → 按文件清理」。
    pub pm_managed: bool,
    pub version: Option<String>,
    pub latest_version_cmd: Option<String>,
    pub install_cmd: String,
    pub upgrade_cmd: String,
    pub uninstall_cmd: String,
    pub website: String,
    pub api_protocol: String,
    pub supports_model: bool,
    pub supports_fallback_model: bool,
    pub resume_cmd: Option<String>,
    pub continue_cmd: Option<String>,
    pub cache_dirs: Vec<String>,
    pub category: String,
    pub support_one_m_context: bool,
    /// 工具支持的入站协议
    pub supports_openai: bool,
    pub supports_anthropic: bool,
    pub supports_google: bool,
    /// 内置模型名（伪装预设）
    pub builtin_models: Vec<String>,
    /// 是否支持请求优化 / 整流器（启动页可开关）
    pub supports_optimizer: bool,
    pub supports_rectifier: bool,
    /// MSIX/Store 启动 URI（无普通 exe 时使用）
    pub launch_uri: Option<String>,
    /// 检测到的可执行文件路径（GUI/桌面应用启动用）
    pub detected_path: Option<String>,
    /// 用户在界面上手动指定的路径（`~/.any-version/tool-paths.json` 里非默认的那一条）。
    /// 为空表示未指定，走注册表默认路径。
    #[serde(default)]
    pub custom_path: Option<String>,

    /// 工具自身的配置文件（**只有声明了这个字段才支持「设置模型」**）。
    /// 前端据此决定是否显示「只保存模型」入口 —— 没声明的工具摆个按钮只会骗人。
    #[serde(default)]
    pub config_file: Option<ToolConfigFileDto>,

    /// 工具的**形态分类**（来自 paths.json 的 `category`：`CLI Code` / `Desktop` / `IDE` …），
    /// 与 `category`（厂商维度）区分开：前者决定「怎么用」，后者决定「谁家的」。
    /// 前端按它把列表分成 CLI / 桌面端（抄 EchoBird 的分组维度）。
    #[serde(default)]
    pub tool_category: Option<String>,

    /// 由形态分类推导的粗粒度归类：`cli` / `desktop` / `other`。
    #[serde(default)]
    pub tool_kind: Option<String>,

    /// 进行中操作（"upgrading" | "installing" | "uninstalling"），由后端 TOOL_OPS 跟踪；
    /// 前端据此持续显示“升级中/安装中/卸载中”，即使切换 Agent / 页面后也能从 detect 结果恢复。
    #[serde(default)]
    pub busy: Option<String>,
}

// ─── 注册表 ───

/// AI 工具注册表 —— 全局单例，从 ai-tools/ 目录加载
pub struct AiToolRegistry {
    tools: HashMap<String, (ToolConfig, PathConfig)>,
    providers: Vec<ProviderPreset>,
    skills_scan: SkillsScanConfig,
    mcp: McpConfig,
    terminals: TerminalsConfig,
}

impl AiToolRegistry {
    /// 从 ai-tools/ 目录加载所有工具定义
    pub fn load() -> Self {
        let registry_dir = Self::find_registry_dir();

        let mut tools = HashMap::new();

        // 扫描每个子目录
        if registry_dir.exists() {
            if let Ok(entries) = fs::read_dir(&registry_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        let tool_id = path.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("")
                            .to_string();
                        if tool_id.is_empty() || tool_id.starts_with('.') {
                            continue;
                        }
                        let config_path = path.join("config.json");
                        let paths_path = path.join("paths.json");
                        if !config_path.exists() || !paths_path.exists() {
                            eprintln!("[ai_registry] 跳过 {}: config.json 或 paths.json 不存在", tool_id);
                            continue;
                        }
                        let config_str = match fs::read_to_string(&config_path) {
                            Ok(s) => s,
                            Err(e) => {
                                eprintln!("[ai_registry] 读取 config.json 失败 [{}]: {}", tool_id, e);
                                continue;
                            }
                        };
                        let paths_str = match fs::read_to_string(&paths_path) {
                            Ok(s) => s,
                            Err(e) => {
                                eprintln!("[ai_registry] 读取 paths.json 失败 [{}]: {}", tool_id, e);
                                continue;
                            }
                        };
                        let config = match serde_json::from_str::<ToolConfig>(&config_str) {
                            Ok(c) => c,
                            Err(e) => {
                                eprintln!("[ai_registry] 解析 config.json 失败 [{}]: {}", tool_id, e);
                                continue;
                            }
                        };
                        let paths = match serde_json::from_str::<PathConfig>(&paths_str) {
                            Ok(p) => p,
                            Err(e) => {
                                eprintln!("[ai_registry] 解析 paths.json 失败 [{}]: {}", tool_id, e);
                                continue;
                            }
                        };
                        tools.insert(tool_id.clone(), (config, paths));
                        eprintln!("[ai_registry] 加载工具: {}", tool_id);
                    }
                }
            }
        }

        // 加载 providers.json
        let providers = Self::load_json::<Vec<ProviderPreset>>(
            &registry_dir.join("providers.json"),
        ).unwrap_or_else(|_| {
            eprintln!("[ai_registry] 无法加载 providers.json，使用空列表");
            Vec::new()
        });

        // 加载 skills-scan.json
        let skills_scan = Self::load_json::<SkillsScanConfig>(
            &registry_dir.join("skills-scan.json"),
        ).unwrap_or_else(|_| {
            eprintln!("[ai_registry] 无法加载 skills-scan.json，使用默认配置");
            SkillsScanConfig {
                description: String::new(),
                base_skills_dir: "~/.agents/skills".to_string(),
                tool_skills_dirs: HashMap::new(),
                reads_agents_skills: Vec::new(),
            }
        });

        // 加载 mcp-config.json
        let mcp = Self::load_json::<McpConfig>(
            &registry_dir.join("mcp-config.json"),
        ).unwrap_or_else(|_| {
            eprintln!("[ai_registry] 无法加载 mcp-config.json，使用默认配置");
            McpConfig {
                description: String::new(),
                tools: HashMap::new(),
            }
        });

        // 加载 terminals.json
        let terminals = Self::load_json::<TerminalsConfig>(
            &registry_dir.join("terminals.json"),
        ).unwrap_or_else(|_| {
            eprintln!("[ai_registry] 无法加载 terminals.json，使用默认配置");
            TerminalsConfig {
                description: String::new(),
                terminals: HashMap::new(),
                proxy_settings: ProxySettingsDef {
                    default_port: 15721,
                    listen_address: "127.0.0.1".to_string(),
                    timeout_seconds: 300,
                },
            }
        });

        eprintln!(
            "[ai_registry] 加载完成: {} 个工具, {} 个 provider 预设",
            tools.len(),
            providers.len()
        );

        Self { tools, providers, skills_scan, mcp, terminals }
    }

    /// 查找 ai-tools 注册表目录。
    /// 搜索策略与 projects 注册表（project/registry.rs::load_registry）保持一致：
    /// 依次在「资源目录 / exe 同目录及向上 5 层 / 当前工作目录 / 用户配置目录」中查找
    /// `ai-tools` 或 `_up_/ai-tools`（Tauri 打包时 `../ai-tools` 的 `..` 会被映射为 `_up_` 前缀）。
    pub(crate) fn find_registry_dir() -> PathBuf {
        let mut search_dirs: Vec<PathBuf> = Vec::new();

        // 优先在 Tauri 2 打包后的官方资源目录下查找
        if let Some(res_dir) = crate::commands::utils::get_resource_dir() {
            search_dirs.push(res_dir);
        }

        // exe 同目录及向上 5 层
        if let Ok(exe) = std::env::current_exe() {
            if let Some(exe_dir) = exe.parent() {
                search_dirs.push(exe_dir.to_path_buf());
                let mut dir = exe_dir.to_path_buf();
                for _ in 0..5 {
                    if let Some(parent) = dir.parent() {
                        dir = parent.to_path_buf();
                        search_dirs.push(dir.clone());
                    }
                }
            }
        }

        // 当前工作目录
        if let Ok(cwd) = std::env::current_dir() {
            search_dirs.push(cwd);
        }

        // 用户配置目录（~/.any-version）
        search_dirs.push(crate::commands::config::get_base_dir());

        // 每个候选目录下查找 ai-tools 目录（含 Tauri 打包时的 `_up_` 前缀布局）
        for dir in &search_dirs {
            for candidate in [dir.join("_up_").join("ai-tools"), dir.join("ai-tools")] {
                if candidate.exists() && candidate.is_dir() {
                    eprintln!("[ai_registry] 找到 ai-tools 目录: {}", candidate.display());
                    return candidate;
                }
            }
        }

        // Fallback: 使用默认路径
        eprintln!("[ai_registry] 未找到 ai-tools 目录，使用默认路径");
        search_dirs.first().cloned().unwrap_or_else(|| PathBuf::from("ai-tools"))
    }

    fn load_json<T: serde::de::DeserializeOwned>(path: &PathBuf) -> Result<T, String> {
        let content = fs::read_to_string(path)
            .map_err(|e| {
                let msg = format!("[ai_registry] 读取文件失败 {}: {}", path.display(), e);
                eprintln!("{}", msg);
                msg
            })?;
        serde_json::from_str(&content)
            .map_err(|e| {
                let msg = format!("[ai_registry] JSON 解析失败 {}: {}", path.display(), e);
                eprintln!("{}", msg);
                msg
            })
    }

    // ─── 查询方法 ───

    pub fn tool_ids(&self) -> Vec<&String> {
        self.tools.keys().collect()
    }

    pub fn tool_iter(&self) -> impl Iterator<Item = (&String, &(ToolConfig, PathConfig))> {
        self.tools.iter()
    }

    pub fn get_tool(&self, id: &str) -> Option<&(ToolConfig, PathConfig)> {
        self.tools.get(id)
    }

    pub fn get_tool_config(&self, id: &str) -> Option<&ToolConfig> {
        self.tools.get(id).map(|(c, _)| c)
    }

    pub fn get_path_config(&self, id: &str) -> Option<&PathConfig> {
        self.tools.get(id).map(|(_, p)| p)
    }

    pub fn providers(&self) -> &[ProviderPreset] {
        &self.providers
    }

    pub fn skills_scan(&self) -> &SkillsScanConfig {
        &self.skills_scan
    }

    pub fn mcp(&self) -> &McpConfig {
        &self.mcp
    }

    /// 返回可部署 MCP 的工具 id 列表（来自 mcp-config.json）
    pub fn mcp_tool_ids(&self) -> Vec<String> {
        self.mcp.tools.keys().cloned().collect()
    }

    /// 解析某工具的 MCP 中心配置文件路径、格式与根 Key (mcp / mcpServers)
    pub fn get_tool_mcp_config(&self, tool_id: &str) -> Option<(PathBuf, String, String)> {
        let home = Self::get_home();
        let config_home = std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home.join(".config"));
        let entry = self.mcp.tools.get(tool_id)?;
        let resolved = if entry.config_path.starts_with("~/.config/") {
            let relative = entry.config_path.strip_prefix("~/.config/").unwrap_or("").trim_start_matches('/');
            config_home.join(relative)
        } else {
            Self::resolve_path(&entry.config_path, &home)
        };
        let mcp_key = entry.mcp_key.clone().unwrap_or_else(|| {
            if entry.format == "opencode" { "mcp".to_string() } else { "mcpServers".to_string() }
        });
        Some((resolved, entry.format.clone(), mcp_key))
    }

    pub fn terminals(&self) -> &TerminalsConfig {
        &self.terminals
    }

    pub fn terminal_defs(&self) -> Vec<(&String, &TerminalDef)> {
        self.terminals.terminals.iter().collect()
    }

    // ─── 构建 DTO ───

    /// 将 ToolConfig 转换为前端使用的 AiToolDefDto
    pub fn to_dto(config: &ToolConfig, paths: &PathConfig, installed: bool, version: Option<String>) -> AiToolDefDto {
        let pkg_name = config.pkg_name.as_deref().unwrap_or(&config.id);
        let upgrade_cmd = match config.pkg_manager.as_deref() {
            Some("npm") => format!("npm install -g {}@latest", pkg_name),
            Some("pip") => format!("pip install --upgrade {}", pkg_name),
            _ => paths.install_cmd.clone(),
        };

        let uninstall_cmd = match config.pkg_manager.as_deref() {
            Some("npm") => format!("npm uninstall -g {}", pkg_name),
            Some("pip") => format!("pip uninstall -y {}", pkg_name),
            _ => paths.uninstall_cmd.clone().unwrap_or_default(),
        };

        AiToolDefDto {
            id: config.id.clone(),
            display_name: config.display_name.clone(),
            avatar: config.avatar.clone(),
            nickname: config.nickname.clone(),
            installed,
            // 这里是「未探测」的投影路径，不了解安装来源 → 保守填 false，
            // 宁可不给卸载入口，也不要让用户点了才发现卸不掉。
            pm_managed: false,
            version,
            latest_version_cmd: None,
            install_cmd: paths.install_cmd.clone(),
            upgrade_cmd,
            uninstall_cmd,
            website: config.website.clone(),
            api_protocol: config.api_protocol.clone(),
            supports_model: config.support_model,
            supports_fallback_model: config.support_fallback_model,
            resume_cmd: config.resume_cmd.clone(),
            continue_cmd: config.continue_cmd.clone(),
            cache_dirs: config.cache_dirs.clone(),
            category: config.category.clone(),
            support_one_m_context: config.support_one_m_context,
            supports_openai: config.supports_openai,
            supports_anthropic: config.supports_anthropic,
            supports_google: config.supports_google,
            builtin_models: config.builtin_models.clone(),
            supports_optimizer: config.supports_optimizer,
            supports_rectifier: config.supports_rectifier,
            launch_uri: paths.launch_uri.clone(),
            detected_path: None,
            custom_path: None,
            config_file: config.config_file.as_ref().map(|cf| ToolConfigFileDto {
                path: cf.path.clone(),
                format: cf.format.clone(),
            }),
            tool_category: Some(paths.category.clone()),
            tool_kind: Some(tool_kind_of(Some(&paths.category))),
            busy: None,
        }
    }

    // ─── 技能目录解析 ───

    /// 解析所有需要扫描的技能目录（展开 ~ 等占位符）
    pub fn get_skill_scan_dirs(&self) -> Vec<(PathBuf, String)> {
        let home = Self::get_home();
        let config_home = std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home.join(".config"));

        let mut dirs: Vec<(PathBuf, String)> = Vec::new();

        // 技能托管仓库（即 skills.sh 公共仓库，默认 ~/.agents/skills）：
        // 作为 skills.sh 的 GUI，托管仓库与公共仓库为同一目录，
        // 在工具扫描循环中被跳过（仅用于判定 in_store / managed）。
        let av_store = skills_dir();
        dirs.push((av_store, "any-version".to_string()));

        // skills.sh 仓库（~/.agents/skills）：作为「可发现的外来技能来源」（非 vex 托管仓库）。
        // 用户可把其中的技能「整理」导入到 vex 目录。注意它与 vex 自身仓库无关。
        let sh_store = Self::resolve_path(&self.skills_scan.base_skills_dir, &home);
        dirs.push((sh_store, "skills.sh".to_string()));

        // 各工具独有的技能目录
        for (tool_id, tool_dirs) in &self.skills_scan.tool_skills_dirs {
            for d in tool_dirs {
                let resolved = Self::resolve_path(d, &home);
                // 替换 ~/.config 为实际 config 目录
                let resolved = if d.starts_with("~/.config/") {
                    let relative = d.strip_prefix("~/.config/").unwrap_or("");
                    config_home.join(relative)
                } else {
                    resolved
                };
                dirs.push((resolved, tool_id.clone()));
            }
        }

        dirs
    }

    /// 解析工具的技能目录 JUNCTION 目标路径
    pub fn get_tool_skill_dir(&self, tool_id: &str, skill_id: &str) -> PathBuf {
        let home = Self::get_home();
        let config_home = std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home.join(".config"));

        // 先从 skills-scan 中查找
        if let Some(dirs) = self.skills_scan.tool_skills_dirs.get(tool_id) {
            if let Some(first) = dirs.first() {
                let resolved = if first.starts_with("~/.config/") {
                    let relative = first.strip_prefix("~/.config/").unwrap_or("");
                    config_home.join(relative)
                } else {
                    Self::resolve_path(first, &home)
                };
                return resolved.join(skill_id);
            }
        }

        // 从工具配置中查找
        if let Some((config, _)) = self.tools.get(tool_id) {
            if let Some(dir) = &config.skills_dir {
                return Self::resolve_path(dir, &home).join(skill_id);
            }
            if let Some(dir) = &config.skills_dir_xdg {
                return config_home.join(dir.trim_start_matches("~/.config/")).join(skill_id);
            }
        }

        // Fallback: ~/.{tool_id}/skills/{skill_id}
        home.join(format!(".{}", tool_id)).join("skills").join(skill_id)
    }

    // ─── 工具函数 ───

    fn get_home() -> PathBuf {
        crate::commands::utils::get_home_dir()
    }

    fn resolve_path(path: &str, home: &PathBuf) -> PathBuf {
        let resolved = if path.starts_with("~/") {
            home.join(&path[2..])
        } else if path.starts_with('~') {
            home.join(&path[1..])
        } else {
            PathBuf::from(path)
        };
        // 解析 %VAR% 格式（Windows）
        let resolved_str = resolved.to_string_lossy().to_string();
        let final_str = resolved_str
            .replace("%APPDATA%", &std::env::var("APPDATA").unwrap_or_default())
            .replace("%LOCALAPPDATA%", &std::env::var("LOCALAPPDATA").unwrap_or_default())
            .replace("%USERPROFILE%", &home.to_string_lossy())
            .replace("%PROGRAMFILES%", &std::env::var("ProgramFiles").unwrap_or_default());
        PathBuf::from(final_str)
    }

    /// 解析工具 skills JUNCTION 目标（泛化 fallback）
    pub fn resolve_skill_junction_target(&self, tool_id: &str, skill_id: &str) -> PathBuf {
        self.get_tool_skill_dir(tool_id, skill_id)
    }
}

// ─── 全局单例 ───

use std::sync::RwLock;

static REGISTRY: RwLock<Option<&'static AiToolRegistry>> = RwLock::new(None);

/// 获取全局注册表单例（首次调用时从磁盘加载）
pub fn registry() -> &'static AiToolRegistry {
    {
        let r = REGISTRY.read().unwrap();
        if let Some(reg) = *r {
            return reg;
        }
    }
    let mut w = REGISTRY.write().unwrap();
    if w.is_none() {
        let reg = Box::leak(Box::new(AiToolRegistry::load()));
        *w = Some(reg);
    }
    w.unwrap()
}

/// 强制重新加载（用于开发时热重载）
/// 写入最新的注册表，获取最新的实例
pub fn reload_registry() -> &'static AiToolRegistry {
    let mut w = REGISTRY.write().unwrap();
    let reg = Box::leak(Box::new(AiToolRegistry::load()));
    *w = Some(reg);
    reg
}

/// Tauri 命令：强制重新加载 AI 工具注册表（热重载）
/// 在前端修改 ai-tools/ 配置后调用此命令可使更改立即生效
#[tauri::command]
pub fn reload_ai_registry() -> Result<usize, String> {
    let reg = reload_registry();
    let count = reg.tool_ids().len();
    eprintln!("[ai_registry] 热重载完成: {} 个工具", count);
    Ok(count)
}

/// Tauri 命令：修改某工具的归类（写回 ai-tools/<id>/config.json 的 category 字段并热重载）
#[tauri::command]
pub fn update_tool_category(tool_id: String, category: String) -> Result<(), String> {
    if tool_id.is_empty() {
        return Err("工具 id 不能为空".to_string());
    }
    let dir = AiToolRegistry::find_registry_dir();
    let config_path = dir.join(&tool_id).join("config.json");
    if !config_path.exists() {
        return Err(format!("工具 {} 的 config.json 不存在", tool_id));
    }

    let raw = fs::read_to_string(&config_path).map_err(|e| e.to_string())?;
    let mut config: serde_json::Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    if let Some(obj) = config.as_object_mut() {
        obj.insert("category".to_string(), serde_json::Value::String(category));
    }
    let data = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    fs::write(&config_path, data).map_err(|e| e.to_string())?;

    let _ = reload_ai_registry();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{registry, tool_kind_of, ProviderPreset, ProviderPresetDto};

    /// 回归：DTO 一旦被标上 `rename_all = "camelCase"`，URL 会序列化成
    /// `openaiUrl`，前端按 `openai_url` 读取全部落空 → 添加预设只剩名称/官网。
    #[test]
    fn desktop_tools_are_registered_and_classified_as_desktop() {
        // 从 EchoBird 移植进来的桌面端工具（category=Desktop）：
        // 注册表读得到、且被归到 desktop —— 否则工具列表的「桌面端」分组永远是空的。
        let reg = registry();
        for id in [
            "claudedesktop",
            "chatgptdesktop",
            "mimodesktop",
            "kimidesktop",
            "opencodedesktop",
            "openscience",
            "zcode",
            "workbuddy",
            "workbuddyai",
            "dsh",
        ] {
            let Some((config, paths)) = reg.get_tool(id) else {
                panic!("注册表里没有桌面端工具 {}（ai-tools/{}/ 是否完整？）", id, id);
            };
            assert_eq!(
                tool_kind_of(Some(&paths.category)),
                "desktop",
                "{} 的形态是 {}，没被归到桌面端",
                id,
                paths.category
            );
            // 声明了 configFile 才允许显示「设置模型」入口，两者必须一致
            assert_eq!(
                config.config_file.is_some(),
                config.support_model,
                "{} 的 configFile 与 supportModel 不一致（会成为点了没效果的假入口）",
                id
            );
        }
    }

    /// 每个工具目录都必须能加载进注册表。
    ///
    /// 回归的是「少写一个字段 → 整份 config.json 解析失败 → 工具被静默丢掉」：
    /// omp 就因为没有 `cacheDirs` 一直没出现在列表里，只在 stderr 留了一行 parse 失败。
    #[test]
    fn every_tool_directory_loads_into_the_registry() {
        let dir = super::AiToolRegistry::find_registry_dir();
        let mut dirs: Vec<String> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("读不到 ai-tools 目录 {}: {e}", dir.display()))
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        dirs.sort();
        assert!(!dirs.is_empty(), "{} 下没有工具目录", dir.display());

        let reg = registry();
        let missing: Vec<String> = dirs
            .iter()
            .filter(|id| reg.get_tool_config(id).is_none())
            .cloned()
            .collect();
        assert!(
            missing.is_empty(),
            "这些工具目录没能加载（config.json 多半缺字段，解析失败会被整个丢掉）: {missing:?}"
        );
    }

    #[test]
    fn tool_kind_normalizes_cli_and_desktop_categories() {
        // 列表要按「CLI / 桌面端」分组（抄 EchoBird 的维度）：
        // EchoBird 的 category 取值更细，这里归一层，前端只认 cli/desktop/other
        assert_eq!(tool_kind_of(Some("CLI Code")), "cli");
        assert_eq!(tool_kind_of(Some("cli code")), "cli");
        assert_eq!(tool_kind_of(Some("Desktop")), "desktop");
        assert_eq!(tool_kind_of(Some("IDE")), "desktop");
        // 认不出的分类不会让分组崩掉，只是落到 other
        assert_eq!(tool_kind_of(None), "other");
        assert_eq!(tool_kind_of(Some("")), "other");
        assert_eq!(tool_kind_of(Some("Science")), "other");
    }

    #[test]
    fn provider_preset_dto_serializes_urls_as_snake_case() {
        let dto = ProviderPresetDto {
            id: "workbuddy2api".into(),
            name: "WorkBuddy2API 本地转换".into(),
            category: "relay".into(),
            website: "https://example.com".into(),
            openai_url: "http://127.0.0.1:8788/v1".into(),
            anthropic_url: "http://127.0.0.1:8788".into(),
            google_url: String::new(),
        };
        let value = serde_json::to_value(&dto).expect("序列化失败");
        assert!(value.get("openai_url").is_some(), "缺少 openai_url 键: {value}");
        assert!(value.get("anthropic_url").is_some(), "缺少 anthropic_url 键: {value}");
        assert!(value.get("google_url").is_some(), "缺少 google_url 键: {value}");
        assert!(value.get("openaiUrl").is_none(), "不应输出驼峰键: {value}");
    }

    /// providers.json 健全性：id 唯一、name 非空、category 仅取已知值、
    /// 至少一个协议端点非空。新增预设时防止手误破坏前端过滤与展示。
    #[test]
    fn providers_json_entries_are_valid_and_unique() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../ai-tools/providers.json");
        let raw = std::fs::read_to_string(&path).expect("providers.json 读取失败");
        let presets: Vec<ProviderPreset> = serde_json::from_str(&raw).expect("providers.json 解析失败");
        assert!(presets.len() >= 40, "预设数量异常: {}", presets.len());

        let mut seen = std::collections::HashSet::new();
        for p in &presets {
            assert!(!seen.contains(&p.id), "重复预设 id: {}", p.id);
            seen.insert(p.id.clone());
            assert!(!p.name.trim().is_empty(), "预设 {} 名称为空", p.id);
            assert!(
                matches!(p.category.as_str(), "provider" | "relay" | "local"),
                "预设 {} 分类未知: {}（前端过滤只认 provider/relay/local）",
                p.id,
                p.category
            );
            assert!(
                !p.openai_url.is_empty() || !p.anthropic_url.is_empty() || !p.google_url.is_empty(),
                "预设 {} 没有任何协议端点",
                p.id
            );
            for url in [&p.openai_url, &p.anthropic_url, &p.google_url] {
                if !url.is_empty() {
                    assert!(
                        url.starts_with("http://") || url.starts_with("https://"),
                        "预设 {} 端点不是 http(s) URL: {}",
                        p.id,
                        url
                    );
                }
            }
        }
    }
}
