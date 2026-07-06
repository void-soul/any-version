//! AI 工具注册表 —— 从 JSON 配置文件加载工具定义、Provider 预设、终端配置等。
//! 参考 EchoBird 的 tools/ 目录结构：每个工具一个 config.json + paths.json。
//! 新增工具 = 在 ai-tools/ 目录下添加 JSON 文件，零代码改动。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

// ─── JSON 类型定义 ───

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolConfig {
    pub id: String,
    pub display_name: String,
    pub category: String,
    pub website: String,
    pub api_protocol: String,
    pub support_model: bool,
    pub support_fallback_model: bool,
    #[serde(default)]
    pub support_one_m_context: bool,
    pub resume_cmd: Option<String>,
    pub continue_cmd: Option<String>,
    pub cache_dirs: Vec<String>,
    pub pkg_manager: Option<String>,
    pub pkg_name: Option<String>,
    pub config_file: Option<ConfigFileDef>,
    pub model_format: Option<ModelFormatDef>,
    pub sessions: Option<SessionScanDef>,
    pub skills_dir: Option<String>,
    pub skills_dir_xdg: Option<String>,
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
    pub paths: HashMap<String, Vec<String>>,
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
    pub installed: bool,
    pub version: Option<String>,
    pub latest_version_cmd: Option<String>,
    pub install_cmd: String,
    pub upgrade_cmd: String,
    pub website: String,
    pub api_protocol: String,
    pub supports_model: bool,
    pub supports_fallback_model: bool,
    pub resume_cmd: Option<String>,
    pub continue_cmd: Option<String>,
    pub cache_dirs: Vec<String>,
    pub category: String,
    pub support_one_m_context: bool,
}

// ─── 注册表 ───

/// AI 工具注册表 —— 全局单例，从 ai-tools/ 目录加载
pub struct AiToolRegistry {
    tools: HashMap<String, (ToolConfig, PathConfig)>,
    providers: Vec<ProviderPreset>,
    skills_scan: SkillsScanConfig,
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

        Self { tools, providers, skills_scan, terminals }
    }

    fn find_registry_dir() -> PathBuf {
        // 开发模式：相对于可执行文件查找
        let mut candidates = Vec::new();

        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                let p = parent.join("ai-tools");
                candidates.push(p);
                // 向上查找 (Tauri dev: src-tauri)
                for up in 1..=5 {
                    let up_path = parent
                        .parent()
                        .and_then(|_| (0..up).fold(Some(parent), |acc, _| acc?.parent()))
                        .map(|p| p.join("ai-tools"));
                    if let Some(p) = up_path {
                        candidates.push(p);
                    }
                }
            }
        }

        // 当前工作目录
        if let Ok(cwd) = std::env::current_dir() {
            candidates.push(cwd.join("ai-tools"));
            candidates.push(cwd.join("..").join("ai-tools"));
        }

        // 用户目录
        if let Ok(home) = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
        {
            candidates.push(PathBuf::from(&home).join(".any-version").join("ai-tools"));
        }

        for c in &candidates {
            if c.exists() && c.is_dir() {
                eprintln!("[ai_registry] 找到 ai-tools 目录: {}", c.display());
                return c.clone();
            }
        }

        // Fallback: 使用当前目录
        eprintln!("[ai_registry] 未找到 ai-tools 目录，使用默认路径");
        candidates.first().cloned().unwrap_or_else(|| PathBuf::from("ai-tools"))
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

    pub fn terminals(&self) -> &TerminalsConfig {
        &self.terminals
    }

    pub fn terminal_defs(&self) -> Vec<(&String, &TerminalDef)> {
        self.terminals.terminals.iter().collect()
    }

    // ─── 构建 DTO ───

    /// 将 ToolConfig 转换为前端使用的 AiToolDefDto
    pub fn to_dto(config: &ToolConfig, paths: &PathConfig, installed: bool, version: Option<String>) -> AiToolDefDto {
        let _cmd = &paths.command;
        let pkg_name = config.pkg_name.as_deref().unwrap_or(&config.id);
        let upgrade_cmd = match config.pkg_manager.as_deref() {
            Some("npm") => format!("npm install -g {}@latest", pkg_name),
            Some("pip") => format!("pip install --upgrade {}", pkg_name),
            _ => paths.install_cmd.clone(),
        };

        AiToolDefDto {
            id: config.id.clone(),
            display_name: config.display_name.clone(),
            installed,
            version,
            latest_version_cmd: None,
            install_cmd: paths.install_cmd.clone(),
            upgrade_cmd,
            website: config.website.clone(),
            api_protocol: config.api_protocol.clone(),
            supports_model: config.support_model,
            supports_fallback_model: config.support_fallback_model,
            resume_cmd: config.resume_cmd.clone(),
            continue_cmd: config.continue_cmd.clone(),
            cache_dirs: config.cache_dirs.clone(),
            category: config.category.clone(),
            support_one_m_context: config.support_one_m_context,
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

        // 通用目录
        let base_dir = Self::resolve_path(&self.skills_scan.base_skills_dir, &home);
        dirs.push((base_dir, ".agents".to_string()));

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
        let home = std::env::var("USERPROFILE").unwrap_or_default();
        let home = if home.is_empty() {
            std::env::var("HOME").unwrap_or_default()
        } else {
            home
        };
        if home.is_empty() {
            eprintln!("[ai_registry] 警告：无法获取 HOME 目录");
            PathBuf::from(".")
        } else {
            PathBuf::from(home)
        }
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

use std::sync::OnceLock;

static REGISTRY: OnceLock<AiToolRegistry> = OnceLock::new();

/// 获取全局注册表单例（首次调用时从磁盘加载）
pub fn registry() -> &'static AiToolRegistry {
    REGISTRY.get_or_init(|| AiToolRegistry::load())
}

/// 强制重新加载（用于开发时热重载，生产环境不需要）
pub fn reload_registry() -> &'static AiToolRegistry {
    let reg = AiToolRegistry::load();
    // OnceLock 只能 set 一次，所以这里用另一个方法
    // 生产环境一般不需要热重载
    match REGISTRY.set(reg) {
        Ok(_) => {},
        Err(_) => {
            // 已经被设置过，返回现有的
        }
    }
    REGISTRY.get().unwrap()
}
