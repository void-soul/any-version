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
    /// 协作模式中显示的头像（emoji 或文字，如 "🤖"）。为空时由前端按 id 回退。
    #[serde(default)]
    pub avatar: Option<String>,
    /// 协作模式中显示的昵称覆盖（为空时使用 display_name）。
    #[serde(default)]
    pub nickname: Option<String>,
    pub category: String,
    pub website: String,
    /// 工具「原生」协议（兼容旧逻辑：one_m 后缀、配置清理判定）。
    /// 新逻辑以 supports_openai/anthropic/google 三标志为准。
    pub api_protocol: String,
    pub support_model: bool,
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
    pub cache_dirs: Vec<String>,
    pub pkg_manager: Option<String>,
    pub pkg_name: Option<String>,
    pub config_file: Option<ConfigFileDef>,
    pub model_format: Option<ModelFormatDef>,
    pub sessions: Option<SessionScanDef>,
    pub skills_dir: Option<String>,
    /// 非交互派发命令模板（协作模式）：`{prompt_file}` 占位符会被替换为提示词文件路径（已加引号）。
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
    pub name: String,
    pub category: String,
    pub api_protocol: Vec<String>,
    pub command: String,
    pub start_command: String,
    pub detect_cmd: String,
    pub install_cmd: String,
    #[serde(default)]
    pub uninstall_cmd: Option<String>,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolConfig {
    /// 该工具的中心 MCP 配置文件路径（支持 ~ 与 %VAR%）
    pub config_path: String,
    /// 配置格式：`claude`(Claude Code) | `gemini`(Qwen/Gemini) | `opencode`(OpenCode 系)
    pub format: String,
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

/// 与前端交互的 Provider 预设 DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderPresetDto {
    pub id: String,
    pub name: String,
    pub category: String,
    pub website: String,
    pub openai_url: String,
    pub anthropic_url: String,
    pub google_url: String,
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
    fn find_registry_dir() -> PathBuf {
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

    /// 解析某工具的 MCP 中心配置文件路径与格式
    pub fn get_tool_mcp_config(&self, tool_id: &str) -> Option<(PathBuf, String)> {
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
        Some((resolved, entry.format.clone()))
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

        // AnyVersion 自身技能仓库（配置驱动，默认 ~/.any-version/skills）：作为托管仓库，
        // 在工具扫描循环中被跳过（仅用于判定 in_store / managed）。
        let av_store = skills_dir();
        dirs.push((av_store, "any-version".to_string()));

        // skills.sh 仓库（~/.agents/skills）：作为「可发现的外来技能来源」（非 AnyVersion 托管仓库）。
        // 用户可把其中的技能「整理」导入到 AnyVersion 目录。注意它与 AnyVersion 自身仓库无关。
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
