use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Stdio;
use tauri::AppHandle;
use tauri::Emitter;

use super::config::get_base_dir;
use crate::commands::tool_version::is_newer;
use super::hidden_cmd;
use super::cache::{get_dir_size, format_bytes, create_junction, migrate_pkg_storage_impl};

// ─── 数据结构 ───

fn default_true() -> bool { true }

/// 整流器配置（被动修复：上游报错后自动重试）
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RectifierConfig {
    /// 总开关
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Thinking 签名整流器
    #[serde(default = "default_true")]
    pub thinking_signature: bool,
    /// Thinking budget 整流器
    #[serde(default = "default_true")]
    pub thinking_budget: bool,
    /// 图片降级整流器
    #[serde(default = "default_true")]
    pub media_fallback: bool,
}

impl Default for RectifierConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            thinking_signature: true,
            thinking_budget: true,
            media_fallback: true,
        }
    }
}

/// 优化器配置（主动优化：请求发出前自动调整）
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OptimizerConfig {
    /// 总开关
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Cache 注入（降低 API 费用）
    #[serde(default = "default_true")]
    pub cache_injection: bool,
    /// Thinking 参数优化
    #[serde(default = "default_true")]
    pub thinking_optimizer: bool,
    /// DeepSeek 兼容规范化
    #[serde(default = "default_true")]
    pub deepseek_normalize: bool,
}

impl Default for OptimizerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cache_injection: true,
            thinking_optimizer: true,
            deepseek_normalize: true,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AiConfig {
    pub providers: Vec<AiProvider>,
    pub active_provider: Option<String>,
    pub active_model: Option<String>,
    /// 全局代理端口
    #[serde(default = "default_proxy_port")]
    pub proxy_port: u16,
    /// 默认项目目录
    #[serde(default)]
    pub default_project_path: String,
    /// 整流器配置
    #[serde(default)]
    pub rectifier: RectifierConfig,
    /// 优化器配置
    #[serde(default)]
    pub optimizer: OptimizerConfig,
    /// 技能存储目录（空字符串 = 默认 ~/.any-version/skills）
    #[serde(default)]
    pub skills_dir: String,
}

fn default_proxy_port() -> u16 {
    15721
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AiProvider {
    pub id: String,
    pub name: String,
    #[serde(default = "default_provider_category")]
    pub category: String, // "provider" | "relay"
    pub api_key: String,
    /// 官方网站 URL
    #[serde(default)]
    pub website: String,

    // ─── OpenAI 协议 ───
    #[serde(default)]
    pub openai_enabled: bool,
    #[serde(default)]
    pub openai_url: String,
    /// 启用转换代理：将 Anthropic 请求转换为 OpenAI 请求
    #[serde(default)]
    pub openai_use_proxy: bool,

    // ─── Anthropic 协议 ───
    #[serde(default)]
    pub anthropic_enabled: bool,
    #[serde(default)]
    pub anthropic_url: String,
    /// 启用转换代理：将 OpenAI 请求转换为 Anthropic 请求
    #[serde(default)]
    pub anthropic_use_proxy: bool,

    // ─── Google 协议（Gemini CLI）───
    #[serde(default)]
    pub google_enabled: bool,
    #[serde(default)]
    pub google_url: String,

    // ─── 模型别名映射（代理模式下使用）───
    /// 角色关键词 → 实际模型 ID 映射
    /// 例如: {"sonnet": "deepseek-v4-pro", "opus": "claude-opus-4-8"}
    /// Claude Code 发送 claude-sonnet-4 时，代理将其映射为 deepseek-v4-pro
    #[serde(default)]
    pub model_aliases: std::collections::HashMap<String, String>,
    /// 默认模型（当别名无匹配时使用）
    #[serde(default)]
    pub default_model: Option<String>,

    pub models: Vec<ModelEntry>,
    pub active_model_id: Option<String>,
}

fn default_provider_category() -> String {
    "provider".to_string()
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ModelEntry {
    pub id: String,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AiSession {
    pub tool_id: String,
    pub project_path: String,
    pub session_id: Option<String>,
    pub last_used: String,
    pub model_id: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct AiSessionsFile {
    pub sessions: Vec<AiSession>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ToolSession {
    pub session_id: String,
    pub project_path: String,
    pub last_used: String,
    pub summary: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub directory: String,
    pub enabled_tools: Vec<String>,
    pub installed_at: String,
    pub install_method: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct ScannedSkill {
    pub name: String,
    pub description: String,
    pub directory: String,
    pub full_path: String,
    pub found_in: Vec<String>,
    pub is_symlink: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct SkillFile {
    pub path: String,
    pub contents: String,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct UsageSummary {
    pub total_records: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_tokens: u64,
    pub by_tool: Vec<UsageByTool>,
    pub by_model: Vec<UsageByModel>,
    pub daily: Vec<UsageDaily>,
}

#[derive(Serialize, Clone, Debug)]
pub struct UsageByTool {
    pub tool_id: String,
    pub request_count: u64,
    pub total_tokens: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct UsageByModel {
    pub model: String,
    pub provider: String,
    pub request_count: u64,
    pub total_tokens: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct UsageDaily {
    pub date: String,
    pub request_count: u64,
    pub total_tokens: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UsageRecord {
    pub tool_id: String,
    pub model: String,
    pub provider: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub timestamp: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct TerminalInfo {
    pub id: String,
    pub name: String,
    pub exe_path: String,
}

#[derive(Deserialize, Clone, Debug)]
pub struct LaunchAiToolRequest {
    pub tool_id: String,
    pub project_path: String,
    pub model_id: Option<String>,
    pub provider_id: Option<String>,
    /// fallback/light 模型（低级任务用）
    pub fallback_model_id: Option<String>,
    pub session_id: Option<String>,
    pub session_mode: String,
    pub terminal_id: String,
    /// Claude Code relay-only: append [1m] to model id for 1M context window
    #[serde(default)]
    pub one_m_context: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SkillsFile {
    pub skills: Vec<Skill>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct UsageFile {
    pub records: Vec<UsageRecord>,
}

// ─── 文件路径 ───

fn ai_config_path() -> PathBuf {
    get_base_dir().join("ai_config.json")
}

fn ai_sessions_path() -> PathBuf {
    get_base_dir().join("ai_sessions.json")
}

// ─── 读写 ───

fn load_ai_config() -> AiConfig {
    let path = ai_config_path();
    if path.exists() {
        if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(config) = serde_json::from_str::<AiConfig>(&data) {
                return config;
            }
        }
    }
    AiConfig {
        providers: Vec::new(),
        active_provider: None,
        active_model: None,
        proxy_port: 15721,
        default_project_path: String::new(),
        skills_dir: String::new(),
        rectifier: RectifierConfig::default(),
        optimizer: OptimizerConfig::default(),
    }
}

fn save_ai_config_to_file(config: &AiConfig) -> Result<(), String> {
    let path = ai_config_path();
    let _ = fs::create_dir_all(path.parent().unwrap());
    let data = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    fs::write(path, data).map_err(|e| e.to_string())
}

fn load_sessions() -> AiSessionsFile {
    let path = ai_sessions_path();
    if path.exists() {
        if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(sessions) = serde_json::from_str::<AiSessionsFile>(&data) {
                return sessions;
            }
        }
    }
    AiSessionsFile::default()
}

fn save_sessions_to_file(sessions: &AiSessionsFile) -> Result<(), String> {
    let path = ai_sessions_path();
    let _ = fs::create_dir_all(path.parent().unwrap());
    let data = serde_json::to_string_pretty(sessions).map_err(|e| e.to_string())?;
    fs::write(path, data).map_err(|e| e.to_string())
}

// ─── AI 工具检测 ───

/// AI 工具定义（参考 EchoBird 的工具检测模式）
#[derive(Clone)]
struct AiToolDef {
    id: &'static str,
    display_name: &'static str,
    detect_cmd: &'static str,
    install_cmd: &'static str,
    website: &'static str,
    api_protocol: &'static str,
    supports_model: bool,
    /// 是否支持双模型（高级模型 + 低级/fallback 模型）
    supports_fallback_model: bool,
    /// fallback 模型的 CLI 参数名（如 "--fallback-model" / "--light-model"）
    fallback_model_arg: Option<&'static str>,
    resume_cmd: Option<&'static str>,
    continue_cmd: Option<&'static str>,
    /// 缓存/数据目录（相对于 HOME，支持 ~ 占位符）
    cache_dirs: &'static [&'static str],
    /// 工具分类：cli-code / desktop / ide / utility
    category: &'static str,
    /// 包管理器类型：Some("npm") / Some("pip") / None（仅 PATH 检测）
    pkg_manager: Option<&'static str>,
    /// npm/pip 包名（用于 PM 查询版本），None 时自动从 install_cmd 解析
    pkg_name: Option<&'static str>,
}

/// const fn 辅助：在函数调用位置触发 unsized coercion，避免 const/static 上下文中的类型推断问题
const fn slice<T>(s: &[T]) -> &[T] {
    s
}

/// 所有支持的 AI 工具定义（参考 EchoBird）
const AI_TOOLS: &[AiToolDef] = slice(&[
    // ─── CLI 编程 Agent ───
    AiToolDef {
        id: "claude-code",
        display_name: "Claude Code",
        detect_cmd: "claude --version",
        install_cmd: "npm install -g @anthropic-ai/claude-code",
        website: "https://claude.ai/code",
        api_protocol: "anthropic",
        supports_model: true,
        supports_fallback_model: true,
        fallback_model_arg: Some("--fallback-model"),
        resume_cmd: Some("claude --resume {session_id}"),
        continue_cmd: Some("claude --continue"),
        cache_dirs: slice(&[".claude"]),
        category: "cli-code",
        pkg_manager: Some("npm"),
        pkg_name: Some("@anthropic-ai/claude-code"),
    },
    AiToolDef {
        id: "codex-cli",
        display_name: "Codex CLI",
        detect_cmd: "codex --version",
        install_cmd: "npm install -g @openai/codex@latest",
        website: "https://github.com/openai/codex",
        api_protocol: "openai",
        supports_model: true,
        supports_fallback_model: true,
        fallback_model_arg: Some("--light-model"),
        resume_cmd: None,
        continue_cmd: Some("codex resume --last"),
        cache_dirs: slice(&[".codex"]),
        category: "cli-code",
        pkg_manager: Some("npm"),
        pkg_name: Some("@openai/codex"),
    },
    AiToolDef {
        id: "aider",
        display_name: "Aider",
        detect_cmd: "aider --version",
        install_cmd: "pip install aider-chat",
        website: "https://aider.chat",
        api_protocol: "both",
        supports_model: true,
        supports_fallback_model: true,
        fallback_model_arg: Some("--weak-model"),
        resume_cmd: None,
        continue_cmd: None,
        cache_dirs: slice(&[".aider", ".aider.cache"]),
        category: "cli-code",
        pkg_manager: Some("pip"),
        pkg_name: Some("aider-chat"),
    },
    AiToolDef {
        id: "opencode",
        display_name: "OpenCode",
        detect_cmd: "opencode --version",
        install_cmd: "npm install -g opencode-ai",
        website: "https://opencode.ai",
        api_protocol: "openai",
        supports_model: true,
        supports_fallback_model: false,
        fallback_model_arg: None,
        resume_cmd: Some("opencode -s {session_id}"),
        continue_cmd: None,
        cache_dirs: slice(&[".opencode"]),
        category: "cli-code",
        pkg_manager: Some("npm"),
        pkg_name: Some("opencode-ai"),
    },
    AiToolDef {
        id: "pi",
        display_name: "Pi Coding Agent",
        detect_cmd: "pi --version",
        install_cmd: "npm install -g @earendil-works/pi-coding-agent",
        website: "https://pi.dev",
        api_protocol: "openai",
        supports_model: true,
        supports_fallback_model: false,
        fallback_model_arg: None,
        resume_cmd: None,
        continue_cmd: None,
        cache_dirs: slice(&[".pi-agent"]),
        category: "cli-code",
        pkg_manager: Some("npm"),
        pkg_name: Some("@earendil-works/pi-coding-agent"),
    },
    AiToolDef {
        id: "mimocode",
        display_name: "MiMo Code",
        detect_cmd: "mimo --version",
        install_cmd: "npm install -g @mimo-ai/cli",
        website: "https://mimo.xiaomi.com/zh/mimocode/config-files",
        api_protocol: "openai",
        supports_model: true,
        supports_fallback_model: true,
        fallback_model_arg: Some("--small-model"),
        resume_cmd: None,
        continue_cmd: None,
        cache_dirs: slice(&[".local/share/mimocode"]),
        category: "cli-code",
        pkg_manager: Some("npm"),
        pkg_name: Some("@mimo-ai/cli"),
    },
    AiToolDef {
        id: "qwencode",
        display_name: "Qwen Code",
        detect_cmd: "qwen --version",
        install_cmd: "npm install -g @qwen-code/qwen-code",
        website: "https://github.com/QwenLM/qwen-code",
        api_protocol: "openai",
        supports_model: true,
        supports_fallback_model: false,
        fallback_model_arg: None,
        resume_cmd: None,
        continue_cmd: None,
        cache_dirs: slice(&[".qwen-code", ".qwen"]),
        category: "cli-code",
        pkg_manager: Some("npm"),
        pkg_name: Some("@qwen-code/qwen-code"),
    },
    AiToolDef {
        id: "grok",
        display_name: "Grok Build",
        detect_cmd: "grok --version",
        install_cmd: "curl -fsSL https://x.ai/cli | bash",
        website: "https://x.ai",
        api_protocol: "openai",
        supports_model: true,
        supports_fallback_model: false,
        fallback_model_arg: None,
        resume_cmd: None,
        continue_cmd: None,
        cache_dirs: slice(&[".grok"]),
        category: "cli-code",
        pkg_manager: None,
        pkg_name: None,
    },
    AiToolDef {
        id: "deveco",
        display_name: "Deveco Code",
        detect_cmd: "deveco --version",
        install_cmd: "npm install -g @deveco/deveco-code",
        website: "https://deveco.dev",
        api_protocol: "none",
        supports_model: false,
        supports_fallback_model: false,
        fallback_model_arg: None,
        resume_cmd: None,
        continue_cmd: None,
        cache_dirs: slice(&[".deveco"]),
        category: "cli-code",
        pkg_manager: Some("npm"),
        pkg_name: Some("@deveco/deveco-code"),
    },
    AiToolDef {
        id: "gemini-cli",
        display_name: "Gemini CLI",
        detect_cmd: "gemini --version",
        install_cmd: "npm install -g @google/gemini-cli",
        website: "https://gemini.google.com",
        api_protocol: "google",
        supports_model: true,
        supports_fallback_model: false,
        fallback_model_arg: None,
        resume_cmd: Some("gemini --resume {session_id}"),
        continue_cmd: Some("gemini --resume latest"),
        cache_dirs: slice(&[".gemini"]),
        category: "cli-code",
        pkg_manager: Some("npm"),
        pkg_name: Some("@google/gemini-cli"),
    },
    AiToolDef {
        id: "kilocode",
        display_name: "Kilo Code",
        detect_cmd: "kilo --version",
        install_cmd: "npm install -g @kilocode/cli",
        website: "https://kilocode.ai",
        api_protocol: "both",
        supports_model: true,
        supports_fallback_model: false,
        fallback_model_arg: None,
        resume_cmd: Some("kilo --session {session_id}"),
        continue_cmd: Some("kilo --continue"),
        cache_dirs: slice(&[".kilocode"]),
        category: "cli-code",
        pkg_manager: Some("npm"),
        pkg_name: Some("@kilocode/cli"),
    },
]);

#[derive(Serialize, Clone, Debug)]
pub struct DetectedAiTool {
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
    pub fallback_model_arg: Option<String>,
    pub resume_cmd: Option<String>,
    pub continue_cmd: Option<String>,
    pub cache_dirs: Vec<String>,
    pub category: String,
}

/// 检测单个 AI 工具：按 PM 类型查询版本，或回退到 detect_cmd + PATH
fn detect_single_tool(def: &AiToolDef) -> DetectedAiTool {
    eprintln!("[detect] ========== {} ({}) ==========", def.display_name, def.id);

    let upgrade_cmd = def.install_cmd.to_string();

    let not_found = DetectedAiTool {
        id: def.id.to_string(),
        display_name: def.display_name.to_string(),
        installed: false,
        version: None,
        latest_version_cmd: None,
        install_cmd: def.install_cmd.to_string(),
        upgrade_cmd,
        website: def.website.to_string(),
        api_protocol: def.api_protocol.to_string(),
        supports_model: def.supports_model,
        supports_fallback_model: def.supports_fallback_model,
        fallback_model_arg: def.fallback_model_arg.map(|s| s.to_string()),
        resume_cmd: def.resume_cmd.map(|s| s.to_string()),
        continue_cmd: def.continue_cmd.map(|s| s.to_string()),
        cache_dirs: def.cache_dirs.iter().map(|s| s.to_string()).collect(),
        category: def.category.to_string(),
    };

    // 策略 1：按 PM 类型精准查询版本
    if let Some(pm) = def.pkg_manager {
        if let Some(pkg) = def.pkg_name {
            eprintln!("[detect]   [策略 1] PM={}, pkg={}", pm, pkg);
            if let Some(ver) = detect_via_pm(pm, pkg) {
                eprintln!("[detect]   [策略 1] ✓ 成功 → version={}", ver);
                return DetectedAiTool {
                    installed: true,
                    version: Some(ver),
                    ..not_found
                };
            } else {
                eprintln!("[detect]   [策略 1] ✗ PM 查询失败");
            }
        }
    }

    // 策略 2：回退到 detect_cmd（调用工具自身的 --version）
    eprintln!("[detect]   [策略 2] detect_cmd=\"{}\"", def.detect_cmd);
    if let Some(ver) = detect_via_cmd(def.detect_cmd) {
        eprintln!("[detect]   [策略 2] ✓ 成功 → version={}", ver);
        return DetectedAiTool {
            installed: true,
            version: Some(ver),
            ..not_found
        };
    }
    eprintln!("[detect]   [策略 2] ✗ 失败");

    eprintln!("[detect] ✗ 未检测到安装");
    not_found
}

/// 通过包管理器查询已安装版本（npm / pip）
fn detect_via_pm(pm: &str, pkg_name: &str) -> Option<String> {
    match pm {
        "npm" => {
            // npm ls {pkg} -g --json
            let npm_args = &["ls", pkg_name, "-g", "--depth=0", "--json"];
            eprintln!("[detect]     npm cmd: npm {}", npm_args.join(" "));
            let (stdout, stderr) = run_cmd_output_full("cmd", &["/c", "npm", "ls", pkg_name, "-g", "--depth=0", "--json"]);
            let stdout = stdout?;
            eprintln!("[detect]     npm stdout: {}", &stdout[..stdout.len().min(500)]);
            if !stderr.is_empty() {
                eprintln!("[detect]     npm stderr: {}", &stderr[..stderr.len().min(500)]);
            }
            // 解析 JSON：{"dependencies": {"@scope/pkg": {"version": "1.2.3"}}}
            match serde_json::from_str::<JsonValue>(&stdout) {
                Ok(val) => {
                    eprintln!("[detect]     npm JSON parsed OK");
                    // 兼容两种格式：dependencies 为对象 或 直接为空
                    if let Some(deps) = val.get("dependencies") {
                        // 先精确匹配
                        if let Some(pkg_info) = deps.get(pkg_name) {
                            if let Some(ver) = pkg_info.get("version").and_then(|v| v.as_str()) {
                                eprintln!("[detect]     npm found {}@{} (exact)", pkg_name, ver);
                                return Some(ver.to_string());
                            }
                        }
                        // 再尝试 scoped package 的可能命名变体
                        eprintln!("[detect]     npm deps keys: {:?}", deps.as_object().map(|o| o.keys().collect::<Vec<_>>()));
                        for (k, v) in deps.as_object()? {
                            let vinfo = v.get("version").and_then(|v| v.as_str());
                            eprintln!("[detect]     npm dep: {}={}", k, vinfo.unwrap_or("?"));
                        }
                    } else {
                        eprintln!("[detect]     npm JSON has no 'dependencies' key, keys: {:?}", val.as_object().map(|o| o.keys().collect::<Vec<_>>()));
                    }
                }
                Err(e) => {
                    eprintln!("[detect]     npm JSON parse error: {}", e);
                }
            }
            None
        }
        "pip" => {
            let pip_args = &["show", pkg_name];
            eprintln!("[detect]     pip cmd: pip {}", pip_args.join(" "));
            let (stdout, stderr) = run_cmd_output_full("cmd", &["/c", "pip", "show", pkg_name]);
            let stdout = stdout?;
            eprintln!("[detect]     pip stdout: {}", &stdout[..stdout.len().min(300)]);
            if !stderr.is_empty() {
                eprintln!("[detect]     pip stderr: {}", &stderr[..stderr.len().min(300)]);
            }
            // 输出包含 "Version: 1.2.3" 行
            for line in stdout.lines() {
                if let Some(ver) = line.strip_prefix("Version:").or_else(|| line.strip_prefix("version:")) {
                    let v = ver.trim().to_string();
                    eprintln!("[detect]     pip found version: {}", v);
                    return Some(v);
                }
            }
            eprintln!("[detect]     pip 'Version:' line not found");
            None
        }
        _ => None,
    }
}

/// 通过 detect_cmd 回退检测（执行工具自身的 --version 命令）
fn detect_via_cmd(detect_cmd: &str) -> Option<String> {
    let parts: Vec<&str> = detect_cmd.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }

    // 先在 PATH 中找可执行文件
    let exe = find_in_path(parts[0])?;
    eprintln!("[detect]     find_in_path({}) → {:?}", parts[0], exe);

    let output = {
        let mut cmd = hidden_cmd::hidden_cmd(&exe);
        if parts.len() > 1 {
            cmd.args(&parts[1..]);
        }
        match cmd.output() {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                eprintln!("[detect]     cmd exit_code={}", out.status.code().map_or(-1, |c| c));
                if !stdout.is_empty() {
                    eprintln!("[detect]     cmd stdout: {}", trimmed_first_line(&stdout, 500));
                }
                if !stderr.is_empty() {
                    eprintln!("[detect]     cmd stderr: {}", trimmed_first_line(&stderr, 500));
                }
                if out.status.success() {
                    Some(format!("{}{}", stdout, stderr))
                } else {
                    None
                }
            }
            Err(e) => {
                eprintln!("[detect]     cmd spawn error: {}", e);
                None
            }
        }
    }?;

    let stdout = output.trim().to_string();
    if stdout.is_empty() {
        eprintln!("[detect]     empty output → (installed)");
        return Some("(installed)".to_string());
    }

    // 用正则提取纯净的 semver 版本号
    let ver = extract_semver(&stdout);
    eprintln!("[detect]     extract_semver({}) → {:?}", trimmed_first_line(&stdout, 100), ver);
    ver
}

/// 在 PATH 中查找可执行文件的绝对路径
fn find_in_path(exe_name: &str) -> Option<PathBuf> {
    // Windows 下补齐 .exe/.cmd/.bat 后缀
    let names: Vec<String> = {
        let lower = exe_name.to_lowercase();
        if lower.ends_with(".exe") || lower.ends_with(".cmd") || lower.ends_with(".bat") {
            vec![exe_name.to_string()]
        } else {
            vec![
                exe_name.to_string(),
                format!("{}.exe", exe_name),
                format!("{}.cmd", exe_name),
            ]
        }
    };

    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            for name in &names {
                let full = dir.join(name);
                if full.is_file() {
                    return Some(full);
                }
            }
        }
    }
    eprintln!("[detect]     find_in_path FAILED for {:?} searching variants {:?}", exe_name, names);
    None
}

/// 从字符串中提取 semver 版本号（如 1.2.3, 0.45.0-alpha）
fn extract_semver(text: &str) -> Option<String> {
    let re = regex::Regex::new(r"(\d+\.\d+\.\d+(?:-[a-zA-Z0-9.]+)?)").ok()?;
    re.captures(text)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// 执行命令并返回 (stdout, stderr)，返回 None 时表示失败
fn run_cmd_output_full(exe: &str, args: &[&str]) -> (Option<String>, String) {
    match hidden_cmd::hidden_cmd(exe).args(args).output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            eprintln!("[detect]     run_cmd: {} {}, exit={}",
                exe, args.join(" "),
                output.status.code().map_or(-1, |c| c));
            if output.status.success() {
                (if stdout.is_empty() { None } else { Some(stdout) }, stderr)
            } else {
                (None, stderr)
            }
        }
        Err(e) => {
            eprintln!("[detect]     run_cmd FAILED: {} {} → {}", exe, args.join(" "), e);
            (None, String::new())
        }
    }
}

fn trimmed_first_line(s: &str, max_len: usize) -> &str {
    let line = s.lines().next().unwrap_or(s);
    if line.len() > max_len {
        &line[..max_len]
    } else {
        line
    }
}

#[tauri::command]
pub async fn detect_ai_tools() -> Result<Vec<DetectedAiTool>, String> {
    let mut handles = Vec::with_capacity(AI_TOOLS.len());
    for def in AI_TOOLS {
        let handle = tokio::task::spawn_blocking(move || detect_single_tool(def));
        handles.push(handle);
    }

    let mut results = Vec::with_capacity(handles.len());
    for handle in handles {
        match handle.await {
            Ok(result) => results.push(result),
            Err(e) => eprintln!("[detect_ai_tools] task join error: {}", e),
        }
    }
    Ok(results)
}

/// AI 工具版本状态
#[derive(Serialize, Clone, Debug)]
pub struct AiToolVersionStatus {
    pub tool_id: String,
    pub display_name: String,
    pub current_version: Option<String>,
    pub latest_version: Option<String>,
    pub status: String,
}

/// 检查所有 AI 工具的最新版本（npm/pip 在线查询）
#[tauri::command]
pub async fn check_ai_tool_versions() -> Result<Vec<AiToolVersionStatus>, String> {
    // 先并行获取所有 AI 工具的当前版本
    let mut handles = Vec::with_capacity(AI_TOOLS.len());
    for def in AI_TOOLS {
        let handle = tokio::task::spawn_blocking(move || (def.id, def.display_name, detect_single_tool(def)));
        handles.push(handle);
    }

    let mut tools: Vec<(&str, &str, DetectedAiTool)> = Vec::new();
    for handle in handles {
        match handle.await {
            Ok((id, name, result)) => tools.push((id, name, result)),
            Err(e) => eprintln!("[ai_ver] task join error: {}", e),
        }
    }

    // 只对已安装且有 pkg_manager 的工具查最新版本
    let mut results = Vec::new();
    for (id, name, tool) in &tools {
        let def = AI_TOOLS.iter().find(|d| d.id == *id);
        let pkg_manager = def.and_then(|d| d.pkg_manager);
        let pkg_name = def.and_then(|d| d.pkg_name);

        let latest = if tool.installed && pkg_manager.is_some() {
            match pkg_manager.unwrap() {
                "npm" => {
                    if let Some(n) = pkg_name {
                        fetch_npm_latest_version(n).await
                    } else { None }
                }
                "pip" => {
                    if let Some(n) = pkg_name {
                        fetch_pypi_latest_version(n).await
                    } else { None }
                }
                _ => None,
            }
        } else {
            None
        };

        let status = match (&tool.version, &latest) {
            (None, _) => "not_installed".to_string(),
            (Some(_), None) => "unknown".to_string(),
            (Some(cur), Some(ver)) => {
                if is_newer(ver, cur) {
                    "outdated".to_string()
                } else {
                    "latest".to_string()
                }
            }
        };

        eprintln!("[ai_ver] {}: installed={}, current={:?}, latest={:?}, status={}",
            id, tool.installed, tool.version, latest, status);

        results.push(AiToolVersionStatus {
            tool_id: id.to_string(),
            display_name: name.to_string(),
            current_version: tool.version.clone(),
            latest_version: latest,
            status,
        });
    }

    Ok(results)
}

/// 查询 npm registry 获取最新版本号
async fn fetch_npm_latest_version(package: &str) -> Option<String> {
    let client = match reqwest::Client::builder()
        .user_agent("Any-Version-Manager")
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[ai_ver] npm client build error: {}", e);
            return None;
        }
    };

    let url = format!("https://registry.npmjs.org/{}/latest", package);
    eprintln!("[ai_ver] npm fetch: {}", url);

    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[ai_ver] npm request failed: {}", e);
            return None;
        }
    };

    let body = match resp.text().await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[ai_ver] npm read body failed: {}", e);
            return None;
        }
    };

    if let Ok(info) = serde_json::from_str::<JsonValue>(&body) {
        if let Some(ver) = info.get("version").and_then(|v| v.as_str()) {
            eprintln!("[ai_ver] npm latest for {}: {}", package, ver);
            return Some(ver.to_string());
        }
    }
    eprintln!("[ai_ver] npm response for {} had no 'version' field", package);
    None
}

/// 查询 PyPI 获取最新版本号
async fn fetch_pypi_latest_version(package: &str) -> Option<String> {
    let client = match reqwest::Client::builder()
        .user_agent("Any-Version-Manager")
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[ai_ver] pypi client build error: {}", e);
            return None;
        }
    };

    let url = format!("https://pypi.org/pypi/{}/json", package);
    eprintln!("[ai_ver] pypi fetch: {}", url);

    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[ai_ver] pypi request failed: {}", e);
            return None;
        }
    };

    let body = match resp.text().await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[ai_ver] pypi read body failed: {}", e);
            return None;
        }
    };

    if let Ok(info) = serde_json::from_str::<JsonValue>(&body) {
        if let Some(ver) = info
            .get("info")
            .and_then(|i| i.get("version"))
            .and_then(|v| v.as_str())
        {
            eprintln!("[ai_ver] pypi latest for {}: {}", package, ver);
            return Some(ver.to_string());
        }
    }
    eprintln!("[ai_ver] pypi response for {} had no 'info.version' field", package);
    None
}

// ─── skills / usage 文件路径 ───

fn skills_path() -> PathBuf {
    get_base_dir().join("skills.json")
}

fn usage_path() -> PathBuf {
    get_base_dir().join("ai_usage.json")
}

fn skills_dir() -> PathBuf {
    // 使用 ~/.agents/skills 作为 canonical 目录（与 skills.sh 规范一致）
    let home = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default());
    let home = if home.as_os_str().is_empty() {
        PathBuf::from(std::env::var("HOME").unwrap_or_default())
    } else {
        home
    };
    home.join(".agents").join("skills")
}

fn load_skills() -> SkillsFile {
    let path = skills_path();
    if path.exists() {
        if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(skills) = serde_json::from_str::<SkillsFile>(&data) {
                return skills;
            }
        }
    }
    SkillsFile::default()
}

fn save_skills(skills: &SkillsFile) -> Result<(), String> {
    let path = skills_path();
    let _ = fs::create_dir_all(path.parent().unwrap());
    let data = serde_json::to_string_pretty(skills).map_err(|e| e.to_string())?;
    fs::write(path, data).map_err(|e| e.to_string())
}

fn load_usage() -> UsageFile {
    let path = usage_path();
    if path.exists() {
        if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(usage) = serde_json::from_str::<UsageFile>(&data) {
                return usage;
            }
        }
    }
    UsageFile::default()
}

fn save_usage(data: &UsageFile) -> Result<(), String> {
    let path = usage_path();
    let _ = fs::create_dir_all(path.parent().unwrap());
    let json = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}

// ─── AI 配置 ───

#[tauri::command]
pub fn get_ai_config() -> Result<AiConfig, String> {
    Ok(load_ai_config())
}

#[tauri::command]
pub async fn save_ai_config(app: AppHandle, config: AiConfig) -> Result<serde_json::Value, String> {
    let old_config = load_ai_config();
    let old_dir = old_config.skills_dir.clone();
    let new_dir = config.skills_dir.clone();

    // 先保存新配置
    save_ai_config_to_file(&config)?;

    // 检测 skill 目录是否变更，执行迁移
    let mut skill_migrated = false;
    if !new_dir.is_empty() && normalize_path(&old_dir) != normalize_path(&new_dir) {
        let skills_file = load_skills();
        if !skills_file.skills.is_empty() && !old_dir.is_empty() {
            // Clone 需要移入闭包的值
            let old_dir_mv = old_dir.clone();
            let new_dir_mv = new_dir.clone();
            let skills_list = skills_file.skills.clone();
            let app_handle = app.clone();
            let result = tokio::task::spawn_blocking(move || {
                do_migrate_skills(&old_dir_mv, &new_dir_mv, &skills_list, Some(&app_handle))
            }).await.map_err(|e| e.to_string())?;
            skill_migrated = result.moved_count > 0 || result.rebuilt_junctions > 0;

            // 更新 skills.json 中的 directory 路径
            let mut updated_skills = skills_file;
            let old_skills_dir = PathBuf::from(&old_dir);
            let new_skills_dir = PathBuf::from(&new_dir);
            for skill in &mut updated_skills.skills {
                let old_path = PathBuf::from(&skill.directory);
                if let Ok(rel) = old_path.strip_prefix(&old_skills_dir) {
                    skill.directory = new_skills_dir.join(rel).to_string_lossy().to_string();
                }
            }
            save_skills(&updated_skills)?;
        }
    }

    let _ = app.emit("ai-config-changed", serde_json::json!({
        "default_project_path": &config.default_project_path,
        "skills_dir": &config.skills_dir,
        "providers_changed": true,
    }));
    Ok(serde_json::json!({
        "ok": true,
        "skill_migrated": skill_migrated,
    }))
}

// ─── Provider 模型获取 ───

#[tauri::command]
pub async fn fetch_provider_models(base_url: String, api_key: String) -> Result<Vec<String>, String> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    let status = resp.status();
    let body: serde_json::Value = resp.json().await.map_err(|e| format!("解析响应失败: {}", e))?;

    if !status.is_success() {
        let msg = body.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str()).unwrap_or("未知错误");
        return Err(format!("API 返回错误 ({}): {}", status.as_u16(), msg));
    }

    let models: Vec<String> = body.get("data")
        .and_then(|v| v.as_array())
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|m| m.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();

    if models.is_empty() {
        return Err("未获取到模型列表".to_string());
    }
    Ok(models)
}

// ─── 用量统计 ───

#[tauri::command]
pub fn record_usage(tool_id: String, model: String, provider: Option<String>, input_tokens: u64, output_tokens: u64) -> Result<(), String> {
    let mut usage = load_usage();
    usage.records.push(UsageRecord {
        tool_id,
        model,
        provider,
        input_tokens,
        output_tokens,
        timestamp: chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
    });
    save_usage(&usage)
}

#[tauri::command]
pub fn get_usage_summary() -> Result<UsageSummary, String> {
    let usage = load_usage();
    let total_records = usage.records.len() as u64;
    let total_input_tokens: u64 = usage.records.iter().map(|r| r.input_tokens).sum();
    let total_output_tokens: u64 = usage.records.iter().map(|r| r.output_tokens).sum();

    // by_tool
    let mut tool_map: HashMap<String, (u64, u64)> = HashMap::new();
    for r in &usage.records {
        let entry = tool_map.entry(r.tool_id.clone()).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += r.input_tokens + r.output_tokens;
    }
    let mut by_tool: Vec<UsageByTool> = tool_map
        .into_iter()
        .map(|(tool_id, (count, tokens))| UsageByTool { tool_id, request_count: count, total_tokens: tokens })
        .collect();
    by_tool.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));

    // by_model
    let mut model_map: HashMap<(String, String), (u64, u64)> = HashMap::new();
    for r in &usage.records {
        let key = (r.model.clone(), r.provider.clone().unwrap_or_default());
        let entry = model_map.entry(key).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += r.input_tokens + r.output_tokens;
    }
    let mut by_model: Vec<UsageByModel> = model_map
        .into_iter()
        .map(|((model, provider), (count, tokens))| UsageByModel { model, provider, request_count: count, total_tokens: tokens })
        .collect();
    by_model.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));

    // daily
    let mut daily_map: HashMap<String, (u64, u64)> = HashMap::new();
    for r in &usage.records {
        let date = &r.timestamp[..10];
        let entry = daily_map.entry(date.to_string()).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += r.input_tokens + r.output_tokens;
    }
    let mut daily: Vec<UsageDaily> = daily_map
        .into_iter()
        .map(|(date, (count, tokens))| UsageDaily { date, request_count: count, total_tokens: tokens })
        .collect();
    daily.sort_by(|a, b| a.date.cmp(&b.date));

    Ok(UsageSummary {
        total_records,
        total_input_tokens,
        total_output_tokens,
        total_tokens: total_input_tokens + total_output_tokens,
        by_tool,
        by_model,
        daily,
    })
}

#[tauri::command]
pub fn clear_usage() -> Result<(), String> {
    save_usage(&UsageFile::default())
}

// ─── 技能管理 ───

#[tauri::command]
pub fn get_skills() -> Result<Vec<Skill>, String> {
    Ok(load_skills().skills)
}

#[tauri::command]
pub fn install_skill(skill_dir: String) -> Result<(), String> {
    let src = PathBuf::from(&skill_dir);
    if !src.exists() || !src.is_dir() {
        return Err("技能目录不存在".to_string());
    }

    // 从 SKILL.md 读取名称
    let skill_md = src.join("SKILL.md");
    let (name, description) = if skill_md.exists() {
        let content = fs::read_to_string(&skill_md).unwrap_or_default();
        let desc = content.lines().next().unwrap_or("").trim_start_matches('#').trim().to_string();
        let folder_name = src.file_name().unwrap_or_default().to_string_lossy().to_string();
        (folder_name, desc)
    } else {
        let n = src.file_name().unwrap_or_default().to_string_lossy().to_string();
        (n.clone(), n)
    };

    let id = name.to_lowercase().replace(' ', "-");
    let dest_dir = skills_dir().join(&id);

    // 如果已存在则删除
    if dest_dir.exists() {
        let _ = fs::remove_dir_all(&dest_dir);
    }
    copy_dir_recursive(&src, &dest_dir)?;

    let mut skills = load_skills();
    skills.skills.retain(|s| s.id != id);
    skills.skills.push(Skill {
        id: id.clone(),
        name: name.clone(),
        description,
        directory: dest_dir.to_string_lossy().to_string(),
        enabled_tools: vec![],
        installed_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        install_method: "local".to_string(),
    });
    save_skills(&skills)
}

#[tauri::command]
pub fn uninstall_skill(skill_id: String) -> Result<(), String> {
    let mut skills = load_skills();
    if let Some(pos) = skills.skills.iter().position(|s| s.id == skill_id) {
        let dir = skills.skills[pos].directory.clone();
        if !dir.is_empty() {
            let _ = fs::remove_dir_all(&dir);
        }
        skills.skills.remove(pos);
    }
    save_skills(&skills)
}

#[tauri::command]
pub fn toggle_skill_tool(skill_id: String, tool_id: String, enabled: bool) -> Result<(), String> {
    let mut skills = load_skills();
    if let Some(skill) = skills.skills.iter_mut().find(|s| s.id == skill_id) {
        if enabled {
            if !skill.enabled_tools.contains(&tool_id) {
                skill.enabled_tools.push(tool_id);
            }
        } else {
            skill.enabled_tools.retain(|t| t != &tool_id);
        }
    } else {
        return Err("技能不存在".to_string());
    }
    save_skills(&skills)
}

#[tauri::command]
pub fn get_skill_files(skill_id: String) -> Result<(String, Vec<SkillFile>), String> {
    let skills = load_skills();
    let skill = skills.skills.iter().find(|s| s.id == skill_id).ok_or("技能不存在")?;
    let dir = PathBuf::from(&skill.directory);
    if !dir.exists() {
        return Err("技能目录不存在".to_string());
    }
    let mut files = Vec::new();
    collect_skill_files(&dir, &dir, &mut files)?;
    Ok((skill.name.clone(), files))
}

fn collect_skill_files(base: &PathBuf, current: &PathBuf, files: &mut Vec<SkillFile>) -> Result<(), String> {
    for entry in fs::read_dir(current).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let rel = path.strip_prefix(base).unwrap_or(&path).to_string_lossy().to_string();
        if path.is_dir() {
            collect_skill_files(base, &path, files)?;
        } else if path.is_file() {
            let contents = fs::read_to_string(&path).unwrap_or_default();
            files.push(SkillFile { path: rel, contents });
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn install_skill_from_source(source: String) -> Result<(), String> {
    let src_trimmed = source.trim();
    if src_trimmed.is_empty() {
        return Err("来源不能为空".to_string());
    }

    // 本地路径
    let local_path = PathBuf::from(src_trimmed);
    if local_path.exists() && local_path.is_dir() {
        return install_skill(local_path.to_string_lossy().to_string());
    }

    // Git URL 或 owner/repo
    let repo_url = if src_trimmed.starts_with("http://") || src_trimmed.starts_with("https://") {
        src_trimmed.to_string()
    } else if src_trimmed.contains('/') && !src_trimmed.contains('\\') {
        format!("https://github.com/{}", src_trimmed)
    } else {
        return Err("无效的来源格式".to_string());
    };

    let temp_dir = get_base_dir().join("_temp_skill_clone");
    let _ = fs::remove_dir_all(&temp_dir);

    let output = tokio::process::Command::new("git")
        .args(["clone", "--depth", "1", &repo_url])
        .arg(&temp_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .await
        .map_err(|e| format!("git clone 失败: {}", e))?;

    if !output.status.success() {
        let _ = fs::remove_dir_all(&temp_dir);
        return Err(format!("git clone 失败: {}", String::from_utf8_lossy(&output.stderr)));
    }

    let result = install_skill(temp_dir.to_string_lossy().to_string());
    let _ = fs::remove_dir_all(&temp_dir);
    result
}

#[tauri::command]
pub fn scan_existing_skills() -> Result<Vec<ScannedSkill>, String> {
    let mut results: Vec<ScannedSkill> = Vec::new();
    let home = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default());
    let home = if home.as_os_str().is_empty() {
        PathBuf::from(std::env::var("HOME").unwrap_or_default())
    } else {
        home
    };
    let config_home = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".config"));

    // 扫描所有已知的技能目录（参考 skills.sh agents.ts 的完整 globalSkillsDir）
    // 注意：使用 universal .agents/skills 的工具不会重复列（列了也不会多扫）
    let scan_dirs: Vec<(PathBuf, &str)> = vec![
        // ── 通用 skill 目录（skills.sh 官方规范目录）──
        (home.join(".agents").join("skills"), ".agents"),
        // ── 有独立 skills 目录的工具（globalSkillsDir ≠ .agents/skills）──
        (home.join(".claude").join("skills"), ".claude"),
        (home.join(".codex").join("skills"), ".codex"),
        (home.join(".gemini").join("skills"), ".gemini"),
        (home.join(".kilocode").join("skills"), ".kilocode"),
        (home.join(".aider").join("skills"), ".aider"),
        // OpenCode（支持 ~/.opencode/skills 和 XDG opencode/skills）
        (home.join(".opencode").join("skills"), ".opencode"),
        (config_home.join("opencode").join("skills"), "opencode"),
        // Cursor
        (home.join(".cursor").join("skills"), ".cursor"),
        // GitHub Copilot
        (home.join(".copilot").join("skills"), ".copilot"),
        // Firebender
        (home.join(".firebender").join("skills"), ".firebender"),
        // Cline
        (home.join(".cline").join("skills"), ".cline"),
        // Augment
        (home.join(".augment").join("skills"), ".augment"),
        // Windsurf（特殊路径 .codeium/windsurf/skills）
        (home.join(".codeium").join("windsurf").join("skills"), ".codeium/windsurf"),
        // Qwen Code
        (home.join(".qwen").join("skills"), ".qwen"),
        // Goose（XDG 规范）
        (config_home.join("goose").join("skills"), "goose"),
        // Continue
        (home.join(".continue").join("skills"), ".continue"),
        // CodeBuddy
        (home.join(".codebuddy").join("skills"), ".codebuddy"),
        // Roo Code
        (home.join(".roo").join("skills"), ".roo"),
        // Pi（特殊子目录 agent/skills）
        (home.join(".pi").join("agent").join("skills"), ".pi/agent"),
        // Tabnine CLI（特殊子目录 agent/skills）
        (home.join(".tabnine").join("agent").join("skills"), ".tabnine/agent"),
        // Cortex Code（嵌套路径 .snowflake/cortex/skills）
        (home.join(".snowflake").join("cortex").join("skills"), ".snowflake/cortex"),
        // Devin（XDG 规范）
        (config_home.join("devin").join("skills"), "devin"),
        // Warp（虽然 universal，也扫一次 .warp 以防用户手动放）
        (home.join(".warp").join("skills"), ".warp"),
        // MiMo Code
        (home.join(".mimocode").join("skills"), ".mimocode"),
        // Neovate
        (home.join(".neovate").join("skills"), ".neovate"),
        // Bob
        (home.join(".bob").join("skills"), ".bob"),
        // Junie
        (home.join(".junie").join("skills"), ".junie"),
        // Trae
        (home.join(".trae").join("skills"), ".trae"),
        // Trae CN
        (home.join(".trae-cn").join("skills"), ".trae-cn"),
        // Kode
        (home.join(".kode").join("skills"), ".kode"),
        // Lingma
        (home.join(".lingma").join("skills"), ".lingma"),
        // CodeArts Agent
        (home.join(".codeartsdoer").join("skills"), ".codeartsdoer"),
        // Reasonix
        (home.join(".reasonix").join("skills"), ".reasonix"),
        // Rovo Dev
        (home.join(".rovodev").join("skills"), ".rovodev"),
        // ForgeCode
        (home.join(".forge").join("skills"), ".forge"),
        // Autohand Code
        (home.join(".autohand").join("skills"), ".autohand"),
        // Codemaker
        (home.join(".codemaker").join("skills"), ".codemaker"),
        // Code Studio
        (home.join(".codestudio").join("skills"), ".codestudio"),
        // Terramind
        (home.join(".terramind").join("skills"), ".terramind"),
        // Zencoder / Zenflow
        (home.join(".zencoder").join("skills"), ".zencoder"),
        // Grok
        (home.join(".grok").join("skills"), ".grok"),
        // Deveco
        (home.join(".deveco").join("skills"), ".deveco"),
    ];

    let mut seen = std::collections::HashSet::new();
    for (base_dir, location_label) in &scan_dirs {
        if !base_dir.exists() {
            continue;
        }
        if let Ok(entries) = fs::read_dir(base_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                let skill_md = path.join("SKILL.md");
                let description = if skill_md.exists() {
                    fs::read_to_string(&skill_md).unwrap_or_default()
                        .lines().next().unwrap_or("")
                        .trim_start_matches('#').trim().to_string()
                } else {
                    String::new()
                };

                let full_path = path.to_string_lossy().to_string();
                if seen.contains(&full_path) {
                    // 已扫描过（通过前面的 .agents/skills），追加位置标签
                    if let Some(existing) = results.iter_mut().find(|s| s.full_path == full_path) {
                        let loc = location_label.to_string();
                        if !existing.found_in.contains(&loc) {
                            existing.found_in.push(loc);
                        }
                    }
                    continue;
                }
                seen.insert(full_path.clone());

                let is_symlink = path.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false);

                results.push(ScannedSkill {
                    name: name.clone(),
                    description,
                    directory: name,
                    full_path,
                    found_in: vec![location_label.to_string()],
                    is_symlink,
                });
            }
        }
    }
    Ok(results)
}

#[tauri::command]
pub fn import_existing_skill(skill_path: String) -> Result<(), String> {
    install_skill(skill_path)
}

// ─── 模型连接测试 ───

#[tauri::command]
pub async fn test_model_connection(
    openai_url: Option<String>,
    anthropic_url: Option<String>,
    api_key: String,
) -> Result<serde_json::Value, String> {
    let url = openai_url
        .filter(|u| !u.is_empty())
        .or_else(|| anthropic_url.filter(|u| !u.is_empty()))
        .unwrap_or_default();

    if url.is_empty() {
        return Err("未提供 API URL".to_string());
    }

    let test_url = format!("{}/models", url.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let start = std::time::Instant::now();

    let resp = client
        .get(&test_url)
        .header("Authorization", format!("Bearer {}", api_key))
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("连接失败: {}", e))?;

    let latency_ms = start.elapsed().as_millis() as u64;
    let status = resp.status();

    Ok(serde_json::json!({
        "success": status.is_success(),
        "message": if status.is_success() { "连接成功".to_string() } else { format!("HTTP {}", status.as_u16()) },
        "latency_ms": latency_ms,
    }))
}

// ─── 启动 AI 工具 ───

#[tauri::command]
pub async fn launch_ai_tool(req: LaunchAiToolRequest) -> Result<serde_json::Value, String> {
    let config = load_ai_config();
    let tool_def = AI_TOOLS.iter().find(|t| t.id == req.tool_id).ok_or("未知工具")?;
    let provider = req.provider_id.as_ref().and_then(|pid| config.providers.iter().find(|p| &p.id == pid));

    // 确定是否需要启动代理
    let needs_proxy = provider.map_or(false, |p| p.openai_use_proxy || p.anthropic_use_proxy);
    if needs_proxy {
        if let Some(p) = provider {
            let proxy_config = crate::proxy::types::ProxyConfig {
                listen_address: "127.0.0.1".to_string(),
                listen_port: config.proxy_port,
                upstream_base_url: p.openai_url.clone(),
                upstream_api_key: p.api_key.clone(),
                upstream_anthropic_url: p.anthropic_url.clone(),
                upstream_protocol: if p.openai_use_proxy { "openai" } else { "anthropic" }.to_string(),
                target_model: req.model_id.clone().unwrap_or_default(),
                timeout_secs: 300,
                model_aliases: p.model_aliases.clone(),
                default_model: p.default_model.clone(),
                rectifier_enabled: config.rectifier.enabled,
                rectifier_thinking_signature: config.rectifier.thinking_signature,
                rectifier_thinking_budget: config.rectifier.thinking_budget,
                rectifier_media_fallback: config.rectifier.media_fallback,
                optimizer_enabled: config.optimizer.enabled,
                optimizer_cache_injection: config.optimizer.cache_injection,
                optimizer_thinking: config.optimizer.thinking_optimizer,
                optimizer_deepseek: config.optimizer.deepseek_normalize,
            };
            // spawn 到独立 task 运行，避免 axum::serve() 阻塞主流程
            tokio::spawn(async move {
                if let Err(e) = crate::proxy::server::start_proxy_server(proxy_config).await {
                    eprintln!("[proxy] 代理服务器错误: {}", e);
                }
            });
            // 给代理一点时间绑定端口
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }

    // 构建环境变量
    let mut env_vars: Vec<(String, String)> = Vec::new();
    if let Some(p) = provider {
        if needs_proxy {
            let proxy_url = format!("http://127.0.0.1:{}", config.proxy_port);
            if p.openai_use_proxy {
                env_vars.push(("OPENAI_BASE_URL".to_string(), proxy_url.clone()));
                env_vars.push(("OPENAI_API_KEY".to_string(), p.api_key.clone()));
            }
            if p.anthropic_use_proxy {
                env_vars.push(("ANTHROPIC_BASE_URL".to_string(), proxy_url));
                env_vars.push(("ANTHROPIC_AUTH_TOKEN".to_string(), p.api_key.clone()));
            }
        } else {
            if !p.openai_url.is_empty() { env_vars.push(("OPENAI_BASE_URL".to_string(), p.openai_url.clone())); }
            if !p.openai_url.is_empty() { env_vars.push(("OPENAI_API_KEY".to_string(), p.api_key.clone())); }
            if !p.anthropic_url.is_empty() { env_vars.push(("ANTHROPIC_BASE_URL".to_string(), p.anthropic_url.clone())); }
            if !p.anthropic_url.is_empty() { env_vars.push(("ANTHROPIC_AUTH_TOKEN".to_string(), p.api_key.clone())); }
        }
        // Google 协议（Gemini CLI）：GEMINI_API_KEY + GOOGLE_GEMINI_BASE_URL
        if tool_def.api_protocol == "google" {
            env_vars.push(("GEMINI_API_KEY".to_string(), p.api_key.clone()));
            if !p.google_url.is_empty() {
                env_vars.push(("GOOGLE_GEMINI_BASE_URL".to_string(), p.google_url.clone()));
            }
        }

        if let Some(ref model) = req.model_id {
            let effective_model = if req.one_m_context {
                format!("{}[1m]", model)
            } else {
                model.clone()
            };
            env_vars.push(("ANTHROPIC_MODEL".to_string(), effective_model));
        }
        // 非代理模式下也设置角色模型映射（参考 cc-switch model_mapper）
        if !needs_proxy && !p.model_aliases.is_empty() {
            for (role, mapped) in &p.model_aliases {
                let env_key = match role.to_lowercase().as_str() {
                    "sonnet" => "ANTHROPIC_DEFAULT_SONNET_MODEL",
                    "opus" => "ANTHROPIC_DEFAULT_OPUS_MODEL",
                    "haiku" => "ANTHROPIC_DEFAULT_HAIKU_MODEL",
                    "fable" => "ANTHROPIC_DEFAULT_FABLE_MODEL",
                    _ => continue,
                };
                // 1M context: append [1m] to sonnet/opus but NOT haiku
                let effective_mapped = if req.one_m_context && role.to_lowercase() != "haiku" {
                    format!("{}[1m]", mapped)
                } else {
                    mapped.clone()
                };
                env_vars.push((env_key.to_string(), effective_mapped));
            }
            if let Some(ref default_model) = p.default_model {
                if !env_vars.iter().any(|(k, _)| k == "ANTHROPIC_MODEL") {
                    let effective_default = if req.one_m_context {
                        format!("{}[1m]", default_model)
                    } else {
                        default_model.clone()
                    };
                    env_vars.push(("ANTHROPIC_MODEL".to_string(), effective_default));
                }
            }
        }
    }

    // 获取终端 exe
    let terminal_exe = get_terminal_exe(&req.terminal_id);

    // 从 detect_cmd 提取真实可执行文件名（例如 "claude --version" → "claude"）
    let tool_exe = tool_def.detect_cmd
        .split_whitespace()
        .next()
        .unwrap_or(tool_def.id)
        .to_string();

    // resume / continue 参数：resume_cmd / continue_cmd 包含可执行文件名，
    // 需剥离前缀以免与 tool_exe 重复（例如 "claude --resume {sid}" → "--resume {sid}"）
    let exe_prefix = format!("{} ", &tool_exe);
    let extra_args = if req.session_mode == "resume" {
        req.session_id.as_ref().and_then(|sid| {
            tool_def.resume_cmd.map(|s| {
                s.replace("{session_id}", sid)
                    .strip_prefix(&exe_prefix)
                    .unwrap_or(&s.replace("{session_id}", sid))
                    .to_string()
            })
        }).unwrap_or_default()
    } else if req.session_mode == "continue" {
        tool_def.continue_cmd.map(|s| {
            s.strip_prefix(&exe_prefix).unwrap_or(s).to_string()
        }).unwrap_or_default()
    } else {
        String::new()
    };

    // fallback 模型参数
    let fallback_arg = if tool_def.supports_fallback_model {
        req.fallback_model_id.as_ref().and_then(|fm| {
            tool_def.fallback_model_arg.map(|arg| format!("{} {}", arg, fm))
        }).unwrap_or_default()
    } else {
        String::new()
    };

    // 工具命令行参数（去掉前导空格）
    let tool_args = [extra_args.as_str(), fallback_arg.as_str()]
        .iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(" ");

    let mut cmd = hidden_cmd::hidden_cmd(&terminal_exe);
    cmd.current_dir(&req.project_path);

    // 设置环境变量
    for (k, v) in &env_vars {
        cmd.env(k, v);
    }
    // 若设置了 ANTHROPIC_AUTH_TOKEN（自定义 Provider），清除 ANTHROPIC_API_KEY
    // 避免 Claude Code 同时看到两个 auth 变量报 "Both ANTHROPIC_AUTH_TOKEN and ANTHROPIC_API_KEY set" 警告
    if env_vars.iter().any(|(k, _)| k == "ANTHROPIC_AUTH_TOKEN") {
        cmd.env_remove("ANTHROPIC_API_KEY");
    }

    // 将工具参数拆分空格，作为独立 arg 传到底层进程（避免 && 等被外层 shell 解析）
    let tool_arg_parts: Vec<&str> = extra_args
        .split_whitespace()
        .chain(fallback_arg.split_whitespace())
        .filter(|s| !s.is_empty())
        .collect();

    if terminal_exe.to_lowercase().contains("cmd") {
        // CMD：start /d 设目录避免 && 被外层 cmd 解析
        cmd.arg("/c").arg("start").arg("/d").arg(&req.project_path)
           .arg("cmd").arg("/k").arg(&tool_exe);
        for a in &tool_arg_parts { cmd.arg(a); }
    } else if terminal_exe.to_lowercase().contains("wt") {
        // Windows Terminal：-d 设目录
        cmd.arg("-d").arg(&req.project_path).arg("cmd").arg("/k").arg(&tool_exe);
        for a in &tool_arg_parts { cmd.arg(a); }
    } else if is_ext_terminal(&terminal_exe) {
        // WezTerm / Alacritty / Tabby
        let launcher: &[&str] = if terminal_exe.to_lowercase().contains("wezterm") {
            &["start", "--", "cmd", "/k"]  // wezterm 必须用 --
        } else {
            &["-e", "cmd", "/k"]           // alacritty/tabby 不用 --
        };
        for s in launcher { cmd.arg(*s); }
        cmd.arg(&tool_exe);
        for a in &tool_arg_parts { cmd.arg(a); }
    } else {
        // PowerShell / pwsh：使用 Set-Location 替代 cd
        let escaped_path = req.project_path.replace('\'', "''");
        let run_cmd = if tool_args.is_empty() {
            format!("Set-Location -LiteralPath '{}'; {}", escaped_path, &tool_exe)
        } else {
            format!("Set-Location -LiteralPath '{}'; {} {}", escaped_path, &tool_exe, &tool_args)
        };
        cmd.args(["-NoExit", "-Command", &run_cmd]);
    }

    cmd.spawn().map_err(|e| format!("启动失败: {}", e))?;

    // 保存会话信息
    let mut sessions = load_sessions();
    let session_id = req.session_id.unwrap_or_else(|| {
        chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
    });
    sessions.sessions.retain(|s| !(s.tool_id == req.tool_id && s.project_path == req.project_path && s.session_id.as_deref() == Some(&session_id)));
    sessions.sessions.push(AiSession {
        tool_id: req.tool_id,
        project_path: req.project_path,
        session_id: Some(session_id),
        last_used: chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
        model_id: req.model_id,
    });
    save_sessions_to_file(&sessions)?;

    Ok(serde_json::json!({
        "success": true,
        "message": "启动成功".to_string(),
    }))
}

// ─── AI 会话管理 ───

#[tauri::command]
pub fn get_ai_sessions() -> Result<AiSessionsFile, String> {
    Ok(load_sessions())
}

#[tauri::command]
pub fn remove_ai_session(tool_id: String, project_path: String, session_id: Option<String>) -> Result<(), String> {
    let mut sessions = load_sessions();
    sessions.sessions.retain(|s| {
        !(s.tool_id == tool_id && s.project_path == project_path && s.session_id == session_id)
    });
    save_sessions_to_file(&sessions)
}

// ─── 代理服务器 ───

#[tauri::command]
pub async fn start_proxy(port: u16) -> Result<(), String> {
    let config = load_ai_config();
    let provider = config.providers.iter().find(|p| p.openai_use_proxy || p.anthropic_use_proxy)
        .ok_or("没有配置了代理的 Provider")?;

    let proxy_config = crate::proxy::types::ProxyConfig {
        listen_address: "127.0.0.1".to_string(),
        listen_port: port,
        upstream_base_url: provider.openai_url.clone(),
        upstream_api_key: provider.api_key.clone(),
        upstream_anthropic_url: provider.anthropic_url.clone(),
        upstream_protocol: if provider.openai_use_proxy { "openai" } else { "anthropic" }.to_string(),
        target_model: String::new(),
        timeout_secs: 300,
        model_aliases: provider.model_aliases.clone(),
        default_model: provider.default_model.clone(),
        rectifier_enabled: config.rectifier.enabled,
        rectifier_thinking_signature: config.rectifier.thinking_signature,
        rectifier_thinking_budget: config.rectifier.thinking_budget,
        rectifier_media_fallback: config.rectifier.media_fallback,
        optimizer_enabled: config.optimizer.enabled,
        optimizer_cache_injection: config.optimizer.cache_injection,
        optimizer_thinking: config.optimizer.thinking_optimizer,
        optimizer_deepseek: config.optimizer.deepseek_normalize,
    };
    crate::proxy::server::start_proxy_server(proxy_config).await
}

// ─── 终端检测 ───

fn find_terminal(id: &str, name: &str, exe_names: &[&str]) -> Option<TerminalInfo> {
    for exe in exe_names {
        let output = hidden_cmd::hidden_cmd("cmd")
            .args(["/c", "where", exe])
            .output()
            .ok()?;
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let first_line = path.lines().next().unwrap_or(&path).to_string();
            if !first_line.is_empty() {
                return Some(TerminalInfo {
                    id: id.to_string(),
                    name: name.to_string(),
                    exe_path: first_line,
                });
            }
        }
    }
    None
}

fn is_ext_terminal(terminal_exe: &str) -> bool {
    let lower = terminal_exe.to_lowercase();
    lower.contains("wezterm") || lower.contains("alacritty") || lower.contains("tabby")
}

fn get_terminal_exe(terminal_id: &str) -> String {
    match terminal_id {
        "powershell" => "powershell.exe".to_string(),
        "wt" => "wt.exe".to_string(),
        "pwsh" => "pwsh.exe".to_string(),
        "wezterm" => "wezterm.exe".to_string(),
        "alacritty" => "alacritty.exe".to_string(),
        "tabby" => "tabby.exe".to_string(),
        _ => "cmd.exe".to_string(),
    }
}

#[tauri::command]
pub fn detect_terminals() -> Result<Vec<TerminalInfo>, String> {
    let mut terminals = Vec::new();

    // 检测各终端
    let checks: Vec<(&str, &str, &[&str])> = vec![
        ("cmd", "命令提示符", &["cmd.exe"] as &[&str]),
        ("powershell", "PowerShell", &["powershell.exe"]),
        ("wt", "Windows Terminal", &["wt.exe"]),
        ("pwsh", "PowerShell Core", &["pwsh.exe"]),
        ("wezterm", "WezTerm", &["wezterm.exe", "wezterm-gui.exe"]),
        ("alacritty", "Alacritty", &["alacritty.exe"]),
        ("tabby", "Tabby", &["tabby.exe"]),
    ];

    for (id, name, exe_names) in &checks {
        if let Some(term) = find_terminal(id, name, exe_names) {
            terminals.push(term);
        }
    }

    // CMD 作为 fallback 总是可用
    if !terminals.iter().any(|t| t.id == "cmd") {
        terminals.push(TerminalInfo {
            id: "cmd".to_string(),
            name: "命令提示符".to_string(),
            exe_path: "cmd.exe".to_string(),
        });
    }

    Ok(terminals)
}

// ─── 工具会话扫描 ───

/// 扫描工具会话（使用增强版解析，参考 EchoBird + cc-switch）
#[tauri::command]
pub fn scan_tool_sessions(tool_id: String) -> Result<Vec<ToolSession>, String> {
    let mut sessions = Vec::new();
    let home = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default());
    // 跨平台兼容：Linux/macOS 用 HOME
    let home = if home.as_os_str().is_empty() {
        PathBuf::from(std::env::var("HOME").unwrap_or_default())
    } else {
        home
    };

    match tool_id.as_str() {
        "claude-code" => {
            let projects_dir = home.join(".claude").join("projects");
            if projects_dir.exists() {
                scan_claude_sessions_enhanced(&projects_dir, &mut sessions);
            }
        }
        "codex-cli" => {
            let sessions_file = home.join(".codex").join("sessions.jsonl");
            if sessions_file.exists() {
                scan_codex_sessions(&sessions_file, &mut sessions);
            }
        }
        "gemini-cli" => {
            let projects_dir = home.join(".gemini").join("projects");
            if projects_dir.exists() {
                scan_claude_sessions_enhanced(&projects_dir, &mut sessions);
            }
        }
        "kilocode" => {
            let projects_dir = home.join(".kilocode").join("projects");
            if projects_dir.exists() {
                scan_claude_sessions_enhanced(&projects_dir, &mut sessions);
            }
        }
        "opencode" => {
            // OpenCode sessions: 参考 EchoBird，configDir 在 ~/.config/opencode 或 ~/.opencode
            let config_home = std::env::var("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| home.join(".config"));
            let candidate_dirs = vec![
                config_home.join("opencode"),
                home.join(".opencode"),
            ];
            for dir in &candidate_dirs {
                if dir.exists() {
                    scan_opencode_sessions(dir, &mut sessions);
                }
            }
        }
        "mimocode" => {
            // MiMo Code sessions: ~/.config/mimocode（参考 EchoBird tools/mimocode/paths.json）
            let config_home = std::env::var("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| home.join(".config"));
            let candidate_dirs = vec![
                config_home.join("mimocode"),
                home.join(".mimo-code"),
            ];
            for dir in &candidate_dirs {
                if dir.exists() {
                    scan_mimocode_sessions(dir, &mut sessions);
                }
            }
        }
        _ => {}
    }

    sessions.sort_by(|a, b| b.last_used.cmp(&a.last_used));
    Ok(sessions)
}

/// 扫描 Codex sessions（JSONL 格式，参考 cc-switch）
fn scan_codex_sessions(file_path: &PathBuf, sessions: &mut Vec<ToolSession>) {
    if let Ok(content) = fs::read_to_string(file_path) {
        for line in content.lines() {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
                let session_id = val["session_id"].as_str().unwrap_or("").to_string();
                let project_path = val["project_path"].as_str().unwrap_or("").to_string();
                let last_used = val["last_used"].as_str().unwrap_or("").to_string();
                let summary = val["summary"].as_str().map(|s| s.to_string());
                if !session_id.is_empty() {
                    sessions.push(ToolSession {
                        session_id,
                        project_path,
                        last_used,
                        summary,
                    });
                }
            }
        }
    }
}

/// 扫描 OpenCode sessions（文件系统遍历，参考 EchoBird）
fn scan_opencode_sessions(opencode_dir: &PathBuf, sessions: &mut Vec<ToolSession>) {
    // OpenCode 可能在 sessions/ 子目录或直接在根目录存储
    let session_dir = opencode_dir.join("sessions");
    let scan_dir = if session_dir.exists() { session_dir } else { opencode_dir.clone() };

    // 尝试 SQLite 数据库
    let db_path = scan_dir.join("sessions.db");
    if db_path.exists() {
        // 简单读取 SQLite（启发式，非严格 SQL）
        if let Ok(data) = fs::read(&db_path) {
            // 读取包含 "CREATE TABLE" 确认是 SQLite
            if data.starts_with(b"SQLite format 3\0") {
                eprintln!("[scan_opencode] 发现 SQLite 数据，跳过（需要 rusqlite）");
            }
        }
    }

    // 遍历文件系统查找 session 文件
    if let Ok(entries) = walk_dir_for_sessions(&scan_dir, 3) {
        for (path, modified, _size) in entries {
            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // 尝试 JSONL 格式
            if path.extension().map_or(false, |e| e == "jsonl") {
                // 用 Claude 增强版解析器处理
                parse_jsonl_session(&content, sessions);
            } else if path.extension().map_or(false, |e| e == "json") {
                // JSON 格式
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                    extract_session_from_value(&val, &path, sessions, &modified);
                }
            } else {
                // 纯文本格式（可能是聊天记录）
                let sid = path.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                if !sid.is_empty() {
                    let summary = content.lines().next().map(|s| s.chars().take(200).collect());
                    sessions.push(ToolSession {
                        session_id: sid,
                        project_path: path.parent().unwrap_or(&path).to_string_lossy().to_string(),
                        last_used: modified.clone(),
                        summary,
                    });
                }
            }
        }
    }
}

/// 扫描 MiMo Code sessions（参考 EchoBird）
fn scan_mimocode_sessions(mimocode_dir: &PathBuf, sessions: &mut Vec<ToolSession>) {
    // MiMo Code 通常采用 Claude Code 兼容的 session 存储格式
    let projects_dir = mimocode_dir.join("projects");
    if projects_dir.exists() {
        scan_claude_sessions_enhanced(&projects_dir, sessions);
        return;
    }

    // 或者 sessions/ 子目录
    let session_dir = mimocode_dir.join("sessions");
    if session_dir.exists() {
        scan_opencode_sessions(&session_dir, sessions);
    }
}

/// 递归遍历目录查找 session 文件，限制最大深度
fn walk_dir_for_sessions(dir: &PathBuf, max_depth: u32) -> Result<Vec<(PathBuf, String, u64)>, std::io::Error> {
    let mut results = Vec::new();
    walk_dir_recursive(dir, max_depth, &mut results)?;
    Ok(results)
}

fn walk_dir_recursive(dir: &PathBuf, depth: u32, results: &mut Vec<(PathBuf, String, u64)>) -> Result<(), std::io::Error> {
    if depth == 0 { return Ok(()); }
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let modified = entry.metadata()
            .map(|m| {
                if let Ok(t) = m.modified() {
                    chrono::DateTime::<chrono::Local>::from(t)
                        .format("%Y-%m-%dT%H:%M:%S").to_string()
                } else {
                    String::new()
                }
            })
            .unwrap_or_default();

        if path.is_dir() {
            walk_dir_recursive(&path, depth - 1, results)?;
        } else if path.is_file() {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            // 跳过非 session 文件
            if name.starts_with("agent-") || name == "meta.json" || name == "config.json" {
                continue;
            }
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            if size > 0 && size < 10 * 1024 * 1024 { // 跳过超过 10MB 的文件
                results.push((path, modified, size));
            }
        }
    }
    Ok(())
}

/// 解析 JSONL 格式的 session 文件（兼容 Claude Code 格式）
fn parse_jsonl_session(content: &str, sessions: &mut Vec<ToolSession>) {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() { return; }

    let mut session_id = String::new();
    let mut project_path = String::new();
    let mut last_used = String::new();
    let mut summary: Option<String> = None;
    let mut title: Option<String> = None;

    // 头行提取 session_id / cwd / 标题
    for line in lines.iter().take(15) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
            if session_id.is_empty() {
                session_id = val["session_id"].as_str().unwrap_or("").to_string();
            }
            if project_path.is_empty() {
                project_path = val["cwd"].as_str()
                    .or(val["project_path"].as_str())
                    .unwrap_or("").to_string();
            }
            // 从 message 中提取第一条用户输入作为标题
            if let Some(msg) = val.get("message") {
                if msg.get("role").and_then(|r| r.as_str()) == Some("user") {
                    let text = extract_message_text(msg);
                    if !text.is_empty() && title.is_none() {
                        title = Some(text.chars().take(80).collect());
                    }
                }
            }
        }
    }

    // 尾行提取最后时间戳和摘要
    let tail_start = if lines.len() > 30 { lines.len() - 30 } else { 0 };
    for line in &lines[tail_start..] {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(ts) = val["timestamp"].as_str() {
                if ts > last_used.as_str() { last_used = ts.to_string(); }
            }
            if let Some(msg) = val.get("message") {
                if msg.get("role").and_then(|r| r.as_str()) == Some("assistant") {
                    let text = extract_message_text(msg);
                    if !text.is_empty() {
                        summary = Some(text.chars().take(200).collect());
                    }
                }
            }
        }
    }

    if !session_id.is_empty() {
        sessions.push(ToolSession {
            session_id,
            project_path,
            last_used,
            summary: summary.or(title),
        });
    }
}

/// 从 JSON Value 直接提取 session 信息
fn extract_session_from_value(val: &serde_json::Value, _path: &PathBuf, sessions: &mut Vec<ToolSession>, modified: &str) {
    let session_id = val.get("session_id").and_then(|v| v.as_str())
        .or(val.get("id").and_then(|v| v.as_str()))
        .or(val.get("uuid").and_then(|v| v.as_str()))
        .unwrap_or("").to_string();
    if session_id.is_empty() { return; }

    let project_path = val.get("cwd").and_then(|v| v.as_str())
        .or(val.get("project_path").and_then(|v| v.as_str()))
        .or(val.get("projectDir").and_then(|v| v.as_str()))
        .unwrap_or("").to_string();
    let last_used = val.get("last_used").and_then(|v| v.as_str())
        .or(val.get("timestamp").and_then(|v| v.as_str()))
        .or(val.get("updatedAt").and_then(|v| v.as_str()))
        .unwrap_or(modified)
        .to_string();
    let summary = val.get("summary").and_then(|v| v.as_str())
        .or(val.get("title").and_then(|v| v.as_str()))
        .or(val.get("name").and_then(|v| v.as_str()))
        .map(|s| s.chars().take(200).collect());

    sessions.push(ToolSession {
        session_id,
        project_path,
        last_used,
        summary,
    });
}

/// 从 message JSON 中提取文本内容
fn extract_message_text(msg: &serde_json::Value) -> String {
    if let Some(content) = msg.get("content") {
        if let Some(arr) = content.as_array() {
            for item in arr {
                if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                    return text.to_string();
                }
            }
        } else if let Some(s) = content.as_str() {
            return s.to_string();
        }
    }
    String::new()
}

// ─── 工具升级 ───

#[tauri::command]
pub async fn upgrade_ai_tool(tool_id: String) -> Result<String, String> {
    let tool_def = AI_TOOLS.iter().find(|t| t.id == tool_id).ok_or("未知工具")?;
    let output = tokio::process::Command::new("cmd")
        .args(["/c", tool_def.install_cmd])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("升级失败: {}", e))?;

    if output.status.success() {
        Ok("升级成功".to_string())
    } else {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if err.is_empty() { "升级失败".to_string() } else { err })
    }
}

// ─── AI 工具缓存管理 ───

/// 获取 AI 工具的缓存/数据信息（复用 SDK 统一缓存管理，仅支持 Junction）
#[derive(Serialize, Clone, Debug)]
pub struct AiToolCacheInfo {
    pub tool_id: String,
    pub dir_name: String,
    pub full_path: String,
    pub size: String,
    pub size_bytes: u64,
    pub is_junction: bool,
    pub junction_target: String,
    pub exists: bool,
}

#[tauri::command]
pub fn get_ai_tool_cache_info() -> Result<Vec<AiToolCacheInfo>, String> {
    let home = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default());
    let mut results = Vec::new();

    for def in AI_TOOLS {
        for cache_dir in def.cache_dirs {
            let full_path = home.join(cache_dir);
            if !full_path.exists() {
                results.push(AiToolCacheInfo {
                    tool_id: def.id.to_string(),
                    dir_name: cache_dir.to_string(),
                    full_path: full_path.to_string_lossy().to_string(),
                    size: "0 B".to_string(),
                    size_bytes: 0,
                    is_junction: false,
                    junction_target: String::new(),
                    exists: false,
                });
                continue;
            }

            let is_junction = fs::symlink_metadata(&full_path)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false);
            let junction_target = if is_junction {
                fs::read_link(&full_path)
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default()
            } else {
                String::new()
            };

            let size_bytes = get_dir_size(&full_path);
            let size_str = format_bytes(size_bytes);

            results.push(AiToolCacheInfo {
                tool_id: def.id.to_string(),
                dir_name: cache_dir.to_string(),
                full_path: full_path.to_string_lossy().to_string(),
                size: size_str,
                size_bytes,
                is_junction,
                junction_target,
                exists: true,
            });
        }
    }

    results.sort_by(|a, b| {
        b.size_bytes.cmp(&a.size_bytes)
    });
    Ok(results)
}

/// 迁移 AI 工具缓存目录（复用 SDK 统一缓存管理，仅支持 Junction，禁止直接指向目录）
#[tauri::command]
pub fn migrate_ai_tool_cache(
    app: AppHandle,
    _tool_id: String,
    dir_name: String,
    new_path: String,
) -> Result<(), String> {
    let home = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default());
    let orig_path = home.join(&dir_name);
    let target_path = PathBuf::from(&new_path);

    if orig_path.to_string_lossy() == target_path.to_string_lossy() {
        return Err("原路径与目标路径相同".to_string());
    }

    // 禁止直接指向 C 盘：必须迁移到非 C 盘以释放 C 盘空间
    if new_path.to_lowercase().starts_with("c:") {
        return Err("AI 工具缓存只能迁移到非 C 盘（例如 D:\\...），禁止直接指向 C 盘目录".to_string());
    }

    let orig_path_str = orig_path.to_string_lossy().to_string();
    let target_path_str = target_path.to_string_lossy().to_string();

    // 复用 SDK 统一缓存迁移：安全拷贝模式（不可先删），且始终创建 Junction
    migrate_pkg_storage_impl(&app, &orig_path_str, &target_path_str, "cache", false)
}

/// 清理 AI 工具缓存目录
#[tauri::command]
pub fn clean_ai_tool_cache(
    app: AppHandle,
    _tool_id: String,
    dir_name: String,
) -> Result<(), String> {
    let home = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default());
    let cache_path = home.join(&dir_name);
    let cache_path_str = cache_path.to_string_lossy().to_string();
    super::cache::clean_pkg_cache_impl(&app, &cache_path_str)
}

/// 在资源管理器中打开工具缓存目录（按 dir_name）
#[tauri::command]
pub fn open_ai_tool_cache_dir(dir_name: String) -> Result<(), String> {
    let home = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default());
    let cache_path = home.join(&dir_name);
    if cache_path.exists() {
        std::process::Command::new("explorer")
            .arg(&cache_path)
            .spawn()
            .map_err(|e| format!("打开目录失败: {}", e))?;
    }
    Ok(())
}

/// 在资源管理器中打开工具缓存目录（按 full_path）
#[tauri::command]
pub fn open_ai_tool_cache_dir_path(full_path: String) -> Result<(), String> {
    let cache_path = PathBuf::from(&full_path);
    if cache_path.exists() {
        std::process::Command::new("explorer")
            .arg(&cache_path)
            .spawn()
            .map_err(|e| format!("打开目录失败: {}", e))?;
    }
    Ok(())
}

// ─── 技能目录迁移 ───

/// 技能迁移进度
#[derive(Serialize, Clone, Debug)]
pub struct SkillMigrateProgress {
    pub stage: String,
    pub current: usize,
    pub total: usize,
    pub skill_name: String,
}

/// 技能迁移结果
#[derive(Serialize, Clone, Debug)]
pub struct SkillMigrateResult {
    pub moved_count: usize,
    pub rebuilt_junctions: usize,
    pub errors: Vec<String>,
}

/// 执行技能目录迁移：移动文件 + 重建 JUNCTION
fn do_migrate_skills(
    old_dir: &str,
    new_dir: &str,
    skills: &[Skill],
    app_handle: Option<&tauri::AppHandle>,
) -> SkillMigrateResult {
    let home = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default());
    let home = if home.as_os_str().is_empty() {
        PathBuf::from(std::env::var("HOME").unwrap_or_default())
    } else {
        home
    };
    let config_home = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".config"));

    let old_path = PathBuf::from(old_dir);
    let new_path = PathBuf::from(new_dir);

    let mut result = SkillMigrateResult {
        moved_count: 0,
        rebuilt_junctions: 0,
        errors: Vec::new(),
    };

    let emit_progress = |stage: &str, current: usize, total: usize, skill_name: &str| {
        if let Some(handle) = app_handle {
            let _ = handle.emit("skill-migrate-progress", SkillMigrateProgress {
                stage: stage.to_string(),
                current,
                total,
                skill_name: skill_name.to_string(),
            });
        }
    };

    // 确保新目录存在
    if let Err(e) = fs::create_dir_all(&new_path) {
        result.errors.push(format!("创建新目录失败: {}", e));
        return result;
    }

    let total = skills.len();

    for (i, skill) in skills.iter().enumerate() {
        let skill_id = &skill.id;
        emit_progress("移动技能", i + 1, total, &skill.name);

        // 移动技能目录：old_skills_dir/skill_id -> new_skills_dir/skill_id
        let old_skill_dir = old_path.join(skill_id);
        let new_skill_dir = new_path.join(skill_id);

        if old_skill_dir.exists() && old_skill_dir != new_skill_dir {
            if new_skill_dir.exists() {
                let _ = fs::remove_dir_all(&new_skill_dir);
            }
            match fs::rename(&old_skill_dir, &new_skill_dir) {
                Ok(()) => {
                    result.moved_count += 1;
                }
                Err(e) => {
                    // rename 失败时尝试拷贝
                    if let Err(e2) = copy_dir_recursive(&old_skill_dir, &new_skill_dir) {
                        result.errors.push(format!("迁移 {} 失败: {} -> {}", skill.name, e, e2));
                        continue;
                    } else {
                        let _ = fs::remove_dir_all(&old_skill_dir);
                        result.moved_count += 1;
                    }
                }
            }
        } else if !old_skill_dir.exists() && new_skill_dir.exists() {
            // 已在新位置，跳过
            continue;
        } else if !old_skill_dir.exists() && !new_skill_dir.exists() {
            continue;
        }

        // 重建 JUNCTION 链接
        if !skill.enabled_tools.is_empty() {
            emit_progress("重建链接", i + 1, total, &skill.name);

            let tool_skill_dirs: Vec<(&str, PathBuf)> = skill.enabled_tools.iter().map(|t| {
                let dir = match t.as_str() {
                    "claude-code" => home.join(".claude").join("skills").join(skill_id),
                    "codex-cli" => home.join(".codex").join("skills").join(skill_id),
                    "gemini-cli" => home.join(".gemini").join("skills").join(skill_id),
                    "kilocode" => home.join(".kilocode").join("skills").join(skill_id),
                    "aider" => home.join(".aider").join("skills").join(skill_id),
                    "opencode" => config_home.join("opencode").join("skills").join(skill_id),
                    "pi" => home.join(".pi").join("agent").join("skills").join(skill_id),
                    "qwencode" => home.join(".qwen").join("skills").join(skill_id),
                    "mimocode" => home.join(".mimocode").join("skills").join(skill_id),
                    "grok" => home.join(".grok").join("skills").join(skill_id),
                    "deveco" => home.join(".deveco").join("skills").join(skill_id),
                    _ => home.join(format!(".{}", t)).join("skills").join(skill_id),
                };
                (t.as_str(), dir)
            }).collect();

            for (_tool_id, tool_dir) in &tool_skill_dirs {
                if let Some(parent) = tool_dir.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                if tool_dir.exists() {
                    let is_junction = fs::symlink_metadata(tool_dir)
                        .map(|m| m.file_type().is_symlink())
                        .unwrap_or(false);
                    if is_junction {
                        let _ = fs::remove_dir(tool_dir);
                    } else {
                        let _ = fs::remove_dir_all(tool_dir);
                    }
                }
                if let Err(e) = create_ai_junction(tool_dir, &new_skill_dir) {
                    result.errors.push(format!("JUNCTION 失败 for {}: {}", skill.name, e));
                } else {
                    result.rebuilt_junctions += 1;
                }
            }
        }
    }

    emit_progress("完成", total, total, "");
    result
}

/// 标准化路径用于比较
fn normalize_path(path: &str) -> String {
    path.trim_end_matches('\\').trim_end_matches('/').to_lowercase()
}

fn create_ai_junction(link_path: &PathBuf, target_path: &PathBuf) -> Result<(), String> {
    create_junction(link_path, target_path)
}

// ─── Skills.sh 本地安装集成 ───

/// 从在线路径安装 skill：clone 到 anyversion 核心 skill 仓库，再通过 JUNCTION 链接给各工具
#[tauri::command]
pub async fn install_skill_from_online(
    source: String,
    target_tools: Vec<String>,
) -> Result<(), String> {
    let src_trimmed = source.trim();
    if src_trimmed.is_empty() {
        return Err("来源不能为空".to_string());
    }
    if target_tools.is_empty() {
        return Err("请至少选择一个目标工具".to_string());
    }

    // 解析为 Git URL
    let repo_url = if src_trimmed.starts_with("http://") || src_trimmed.starts_with("https://") {
        src_trimmed.to_string()
    } else if src_trimmed.contains('/') && !src_trimmed.contains('\\') {
        format!("https://github.com/{}", src_trimmed)
    } else {
        // 本地路径
        let local_path = PathBuf::from(src_trimmed);
        if local_path.exists() && local_path.is_dir() {
            return install_skill_with_junctions(local_path.to_string_lossy().to_string(), &target_tools);
        }
        return Err("无效的来源格式（需要 Git URL 或 owner/repo）".to_string());
    };

    // 1. Git clone 到临时目录
    let temp_dir = get_base_dir().join("_temp_skill_clone");
    let _ = fs::remove_dir_all(&temp_dir);

    let output = tokio::process::Command::new("git")
        .args(["clone", "--depth", "1", &repo_url])
        .arg(&temp_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .await
        .map_err(|e| format!("git clone 失败: {}", e))?;

    if !output.status.success() {
        let _ = fs::remove_dir_all(&temp_dir);
        return Err(format!("git clone 失败: {}", String::from_utf8_lossy(&output.stderr)));
    }

    // 2. 安装到核心 skill 仓库 + 创建 JUNCTION
    let result = install_skill_with_junctions(temp_dir.to_string_lossy().to_string(), &target_tools);
    let _ = fs::remove_dir_all(&temp_dir);
    result
}

/// 安装 skill：先复制到核心仓库，再为每个工具创建 JUNCTION
fn install_skill_with_junctions(src_dir: String, target_tools: &[String]) -> Result<(), String> {
    let src = PathBuf::from(&src_dir);
    if !src.exists() || !src.is_dir() {
        return Err("技能目录不存在".to_string());
    }

    // 从 SKILL.md 读取名称
    let skill_md = src.join("SKILL.md");
    let (name, description) = if skill_md.exists() {
        let content = fs::read_to_string(&skill_md).unwrap_or_default();
        let desc = content.lines().next().unwrap_or("").trim_start_matches('#').trim().to_string();
        let folder_name = src.file_name().unwrap_or_default().to_string_lossy().to_string();
        (folder_name, desc)
    } else {
        let n = src.file_name().unwrap_or_default().to_string_lossy().to_string();
        (n.clone(), n)
    };

    let id = name.to_lowercase().replace(' ', "-");
    let home = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default());
    let home = if home.as_os_str().is_empty() {
        PathBuf::from(std::env::var("HOME").unwrap_or_default())
    } else {
        home
    };
    let config_home = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".config"));

    // 1. 复制到核心 skill 仓库 ~/.agents/skills/<id>/
    let canonical_dir = skills_dir().join(&id);
    if canonical_dir.exists() {
        let _ = fs::remove_dir_all(&canonical_dir);
    }
    copy_dir_recursive(&src, &canonical_dir)?;

    // 2. 为每个目标工具创建 JUNCTION
    // 路径映射参考 skills.sh agents.ts 的 globalSkillsDir
    let tool_skill_dirs: Vec<(&str, PathBuf)> = target_tools.iter().map(|t| {
        let dir = match t.as_str() {
            "claude-code" => home.join(".claude").join("skills").join(&id),
            "codex-cli" => home.join(".codex").join("skills").join(&id),
            "gemini-cli" => home.join(".gemini").join("skills").join(&id),
            "kilocode" => home.join(".kilocode").join("skills").join(&id),
            "aider" => home.join(".aider").join("skills").join(&id),
            // OpenCode 遵循 XDG 规范
            "opencode" => config_home.join("opencode").join("skills").join(&id),
            // Pi 特殊子目录 agent/skills
            "pi" => home.join(".pi").join("agent").join("skills").join(&id),
            // Qwen Code 使用 .qwen/skills
            "qwencode" => home.join(".qwen").join("skills").join(&id),
            // MiMo Code 使用 .mimocode/skills
            "mimocode" => home.join(".mimocode").join("skills").join(&id),
            // Grok
            "grok" => home.join(".grok").join("skills").join(&id),
            // Deveco
            "deveco" => home.join(".deveco").join("skills").join(&id),
            // 泛化 fallback：~/.{tool}/skills/{id}
            _ => return (t.as_str(), home.join(format!(".{}", t)).join("skills").join(&id)),
        };
        (t.as_str(), dir)
    }).collect();

    let mut enabled_tools: Vec<String> = Vec::new();
    for (tool_id, tool_dir) in &tool_skill_dirs {
        // 确保父目录存在
        if let Some(parent) = tool_dir.parent() {
            let _ = fs::create_dir_all(parent);
        }
        // 如果目标已存在（非 junction），先删除
        if tool_dir.exists() {
            let is_junction = fs::symlink_metadata(tool_dir).map(|m| m.file_type().is_symlink()).unwrap_or(false);
            if is_junction {
                let _ = fs::remove_dir(tool_dir);
            } else {
                let _ = fs::remove_dir_all(tool_dir);
            }
        }
        // 创建 JUNCTION
        if let Err(e) = create_ai_junction(tool_dir, &canonical_dir) {
            eprintln!("[install_skill] JUNCTION 失败 for {}: {}", tool_id, e);
        } else {
            enabled_tools.push(tool_id.to_string());
        }
    }

    // 3. 保存到 skills.json
    let mut skills = load_skills();
    skills.skills.retain(|s| s.id != id);
    skills.skills.push(Skill {
        id: id.clone(),
        name: name.clone(),
        description,
        directory: canonical_dir.to_string_lossy().to_string(),
        enabled_tools,
        installed_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        install_method: "managed".to_string(),
    });
    save_skills(&skills)
}

// ─── 改进的会话扫描（并行，参考 cc-switch Provider Adapter 模式）───

#[tauri::command]
pub async fn scan_tool_sessions_parallel(tool_ids: Vec<String>) -> Result<std::collections::HashMap<String, Vec<ToolSession>>, String> {
    let mut results = std::collections::HashMap::new();

    let mut handles = Vec::new();
    for tool_id in tool_ids {
        let handle = tokio::task::spawn_blocking(move || {
            match scan_tool_sessions(tool_id.clone()) {
                Ok(sessions) => (tool_id, sessions),
                Err(e) => {
                    eprintln!("[scan_tool_sessions_parallel] {} error: {}", tool_id, e);
                    (tool_id, Vec::new())
                }
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        match handle.await {
            Ok((id, sessions)) => { results.insert(id, sessions); }
            Err(e) => eprintln!("[scan_tool_sessions_parallel] task join error: {}", e),
        }
    }

    Ok(results)
}

/// 增强版 Claude Session 扫描：头 10 行提取 title，尾 30 行提取 summary
/// 增强版 Claude 风格 session 扫描（兼容 Claude Code / Gemini CLI / KiloCode 等）
///
/// 目录结构（Claude Code v2.x 实测）：
///   .claude/projects/E--pro-my-any-version/
///     ├── {uuid1}/           ← UUID 子目录（可能包含派生数据）
///     ├── {uuid2}/           
///     ├── {uuid1}.jsonl      ← 实际 session 数据（与子目录同级的文件！）
///     ├── {uuid2}.jsonl
///     └── memory/            ← 特殊目录，跳过
///
/// JSONL 每行格式（顶层字段，注意驼峰命名）：
///   - "sessionId": "uuid"        ← 注意是驼峰 sessionId，不是 session_id
///   - "cwd": "E:\\pro\\..."       ← 工作目录
///   - "timestamp": "2026-..."     ← 每行都有时间戳
///   - "type": "ai-title", "aiTitle": "标题"   ← 会话标题事件
///   - "message": {"role": "user"|"assistant", "content": [...]}
fn scan_claude_sessions_enhanced(dir: &PathBuf, sessions: &mut Vec<ToolSession>) {
    if !dir.is_dir() { return; }

    let read_dir = match fs::read_dir(dir) {
        Ok(d) => d,
        Err(_) => return,
    };

    // 收集所有 .jsonl 文件（在项目根目录下直接扫描，不只是子目录里）
    let mut jsonl_files: Vec<PathBuf> = Vec::new();
    for entry in read_dir.flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();

        // 跳过 memory 目录、agent- 子代理文件
        if name == "memory" || name.starts_with("agent-") {
            continue;
        }

        // jsonl 文件直接在项目目录下（同级）
        if path.is_file() && path.extension().map_or(false, |e| e == "jsonl") {
            jsonl_files.push(path);
            continue;
        }

        // 同时兼容旧格式：UUID 子目录里的 jsonl 文件
        if path.is_dir() && name.len() > 8 && name.contains('-') {
            if let Ok(sub_entries) = fs::read_dir(&path) {
                for sub in sub_entries.flatten() {
                    let sp = sub.path();
                    let sn = sub.file_name().to_string_lossy().to_string();
                    if sn.starts_with("agent-") { continue; }
                    if sp.is_file() && sp.extension().map_or(false, |e| e == "jsonl") {
                        jsonl_files.push(sp);
                    }
                }
            }
        }
    }

    // 去重（同名文件可能被两种扫描路径都收集到）
    jsonl_files.sort();
    jsonl_files.dedup();

    for file_path in &jsonl_files {
        let content = match fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let lines: Vec<&str> = content.lines().collect();
        if lines.is_empty() { continue; }

        let mut session_id = String::new();
        let mut project_path = String::new();
        let mut last_used = String::new();
        let mut title: Option<String> = None;
        let mut summary: Option<String> = None;

        for line in &lines {
            let val = match serde_json::from_str::<serde_json::Value>(line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // 提取 sessionId（驼峰命名）
            if session_id.is_empty() {
                session_id = val["sessionId"].as_str().unwrap_or("").to_string();
            }

            // 提取 cwd（顶层字段）
            if project_path.is_empty() {
                if let Some(cwd) = val["cwd"].as_str() {
                    project_path = cwd.to_string();
                }
            }

            // 提取 timestamp（取最大的）
            if let Some(ts) = val["timestamp"].as_str() {
                if ts > last_used.as_str() {
                    last_used = ts.to_string();
                }
            }

            // 提取 ai-title 作为标题（优先于 message 内容）
            if val["type"].as_str() == Some("ai-title") {
                if let Some(at) = val["aiTitle"].as_str() {
                    if !at.is_empty() {
                        title = Some(at.to_string());
                    }
                }
            }

            // 从 message 中提取内容（作为 summary 后备）
            if let Some(msg) = val.get("message") {
                let role = msg["role"].as_str().unwrap_or("");
                let text = extract_msg_content(msg);

                if role == "user" && !text.is_empty() && title.is_none() {
                    title = Some(text.chars().take(80).collect());
                }
                if role == "assistant" && !text.is_empty() {
                    summary = Some(text.chars().take(200).collect());
                }
            }
        }

        if !session_id.is_empty() {
            if project_path.is_empty() {
                // 从父目录名推断项目路径（如 E--pro-my-any-version → E:/pro/my/any-version）
                if let Some(parent) = file_path.parent() {
                    if let Some(name) = parent.file_name().and_then(|n| n.to_str()) {
                        project_path = decode_project_path(name);
                    }
                }
            }
            sessions.push(ToolSession {
                session_id,
                project_path,
                last_used,
                summary: summary.or(title),
            });
        }
    }
}

/// 从 Claude Code 编码的项目目录名还原实际路径
/// "E--pro-my-any-version" → "E:/pro/my/any-version"
fn decode_project_path(encoded: &str) -> String {
    encoded.replace("--", ":").replace('-', "/")
}

/// 从 message JSON 提取文本内容（支持 string 和 array 两种格式）
fn extract_msg_content(msg: &serde_json::Value) -> String {
    let content = msg.get("content");
    if let Some(s) = content.and_then(|v| v.as_str()) {
        return s.to_string();
    }
    if let Some(arr) = content.and_then(|v| v.as_array()) {
        for item in arr {
            if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                return text.to_string();
            }
        }
    }
    String::new()
}

// ─── 工具函数 ───

fn copy_dir_recursive(src: &PathBuf, dest: &PathBuf) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let dest_path = dest.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_dir_recursive(&path, &dest_path)?;
        } else {
            fs::copy(&path, &dest_path).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
