//! AI 工具注册表 —— 从 JSON 配置文件加载工具定义、Provider 预设、终端配置等。
//! 参考 EchoBird 的 tools/ 目录结构：每个工具一个 config.json + paths.json。
//! 新增工具 = 在 ai-tools/ 目录下添加 JSON 文件，零代码改动。

use super::ai::skills::skills_dir;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

// ─── JSON 类型定义 ───

// ─── 编译后嵌入的运行时结构 ───

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
    pub fork_cmd: Option<String>,
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
    /// 是否使用官方插件市场（前端据此显示「插件市场」区块）
    #[serde(default)]
    pub supports_plugin_marketplace: bool,
    /// 插件市场后端：`"codex"` / `"claude"`（前端据此调不同的命令）
    #[serde(default)]
    pub plugin_marketplace_kind: Option<String>,
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
                        let tool_id = path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("")
                            .to_string();
                        if tool_id.is_empty() || tool_id.starts_with('.') {
                            continue;
                        }
                        let config_path = path.join("config.json");
                        let paths_path = path.join("paths.json");
                        if !config_path.exists() || !paths_path.exists() {
                            eprintln!(
                                "[ai_registry] 跳过 {}: config.json 或 paths.json 不存在",
                                tool_id
                            );
                            continue;
                        }
                        let config_str = match fs::read_to_string(&config_path) {
                            Ok(s) => s,
                            Err(e) => {
                                eprintln!(
                                    "[ai_registry] 读取 config.json 失败 [{}]: {}",
                                    tool_id, e
                                );
                                continue;
                            }
                        };
                        let paths_str = match fs::read_to_string(&paths_path) {
                            Ok(s) => s,
                            Err(e) => {
                                eprintln!(
                                    "[ai_registry] 读取 paths.json 失败 [{}]: {}",
                                    tool_id, e
                                );
                                continue;
                            }
                        };
                        let config = match serde_json::from_str::<ToolConfig>(&config_str) {
                            Ok(c) => c,
                            Err(e) => {
                                eprintln!(
                                    "[ai_registry] 解析 config.json 失败 [{}]: {}",
                                    tool_id, e
                                );
                                continue;
                            }
                        };
                        let paths = match serde_json::from_str::<PathConfig>(&paths_str) {
                            Ok(p) => p,
                            Err(e) => {
                                eprintln!(
                                    "[ai_registry] 解析 paths.json 失败 [{}]: {}",
                                    tool_id, e
                                );
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
        let providers =
            Self::load_json::<Vec<ProviderPreset>>(&registry_dir.join("providers.json"))
                .unwrap_or_else(|_| {
                    eprintln!("[ai_registry] 无法加载 providers.json，使用空列表");
                    Vec::new()
                });

        // 加载 skills-scan.json
        let skills_scan =
            Self::load_json::<SkillsScanConfig>(&registry_dir.join("skills-scan.json"))
                .unwrap_or_else(|_| {
                    eprintln!("[ai_registry] 无法加载 skills-scan.json，使用默认配置");
                    SkillsScanConfig {
                        description: String::new(),
                        base_skills_dir: "~/.agents/skills".to_string(),
                        tool_skills_dirs: HashMap::new(),
                        reads_agents_skills: Vec::new(),
                    }
                });

        // 加载 mcp-config.json
        let mcp = Self::load_json::<McpConfig>(&registry_dir.join("mcp-config.json"))
            .unwrap_or_else(|_| {
                eprintln!("[ai_registry] 无法加载 mcp-config.json，使用默认配置");
                McpConfig {
                    description: String::new(),
                    tools: HashMap::new(),
                }
            });

        // 加载 terminals.json
        let terminals = Self::load_json::<TerminalsConfig>(&registry_dir.join("terminals.json"))
            .unwrap_or_else(|_| {
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

        Self {
            tools,
            providers,
            skills_scan,
            mcp,
            terminals,
        }
    }

    /// 查找 ai-tools 注册表目录。
    /// 搜索策略与 projects 注册表（project/registry.rs::load_registry）保持一致：
    /// 依次在「资源目录 / exe 同目录及向上 5 层 / 当前工作目录 / 用户配置目录」中查找
    /// `ai-tools` 或 `_up_/ai-tools`（Tauri 打包时 `../ai-tools` 的 `..` 会被映射为 `_up_` 前缀）。
    pub(crate) fn find_registry_dir() -> PathBuf {
        let mut search_dirs: Vec<PathBuf> = Vec::new();

        // 开发构建优先用仓库源目录（原因见 `utils::dev_repo_root`）
        if let Some(repo) = crate::commands::utils::dev_repo_root() {
            search_dirs.push(repo);
        }

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
        search_dirs
            .first()
            .cloned()
            .unwrap_or_else(|| PathBuf::from("ai-tools"))
    }

    fn load_json<T: serde::de::DeserializeOwned>(path: &PathBuf) -> Result<T, String> {
        let content = fs::read_to_string(path).map_err(|e| {
            let msg = format!("[ai_registry] 读取文件失败 {}: {}", path.display(), e);
            eprintln!("{}", msg);
            msg
        })?;
        serde_json::from_str(&content).map_err(|e| {
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
            let relative = entry
                .config_path
                .strip_prefix("~/.config/")
                .unwrap_or("")
                .trim_start_matches('/');
            config_home.join(relative)
        } else {
            Self::resolve_path(&entry.config_path, &home)
        };
        let mcp_key = entry.mcp_key.clone().unwrap_or_else(|| {
            if entry.format == "opencode" {
                "mcp".to_string()
            } else {
                "mcpServers".to_string()
            }
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
    pub fn to_dto(
        config: &ToolConfig,
        paths: &PathConfig,
        installed: bool,
        version: Option<String>,
    ) -> AiToolDefDto {
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
            fork_cmd: config.fork_cmd.clone(),
            cache_dirs: config.cache_dirs.clone(),
            category: config.category.clone(),
            support_one_m_context: config.support_one_m_context,
            supports_openai: config.supports_openai,
            supports_anthropic: config.supports_anthropic,
            supports_google: config.supports_google,
            builtin_models: config.builtin_models.clone(),
            supports_optimizer: config.supports_optimizer,
            supports_rectifier: config.supports_rectifier,
            supports_plugin_marketplace: config.supports_plugin_marketplace,
            plugin_marketplace_kind: config.plugin_marketplace_kind.clone(),
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
                return config_home
                    .join(dir.trim_start_matches("~/.config/"))
                    .join(skill_id);
            }
        }

        // Fallback: ~/.{tool_id}/skills/{skill_id}
        home.join(format!(".{}", tool_id))
            .join("skills")
            .join(skill_id)
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
            .replace(
                "%LOCALAPPDATA%",
                &std::env::var("LOCALAPPDATA").unwrap_or_default(),
            )
            .replace("%USERPROFILE%", &home.to_string_lossy())
            .replace(
                "%PROGRAMFILES%",
                &std::env::var("ProgramFiles").unwrap_or_default(),
            );
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

mod models;
pub use models::*;
#[cfg(test)]
mod tests;
