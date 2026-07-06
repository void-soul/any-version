use serde::{Deserialize, Serialize};
use std::collections::HashMap;


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

    // ─── 模型别名映射（按协议分组）───
    /// Anthropic 协议：角色关键词 → 实际模型 ID
    /// 例如: {"sonnet": "nvidia/llama-4-maverick"}
    /// Claude Code 发送 claude-sonnet-4 时，代理/环境变量将其映射到指定模型
    /// `alias = "model_aliases"` 保证旧配置文件的字段兼容
    #[serde(alias = "model_aliases", default)]
    pub anthropic_model_aliases: std::collections::HashMap<String, String>,
    /// Anthropic 协议的默认模型（当角色无匹配时使用）
    #[serde(alias = "default_model", default)]
    pub anthropic_default_model: Option<String>,

    /// OpenAI 协议的模型别名映射（未来扩展）
    #[serde(default)]
    pub openai_model_aliases: std::collections::HashMap<String, String>,
    /// OpenAI 协议的默认模型（未来扩展）
    #[serde(default)]
    pub openai_default_model: Option<String>,

    /// Google 协议的模型别名映射（未来扩展）
    #[serde(default)]
    pub google_model_aliases: std::collections::HashMap<String, String>,
    /// Google 协议的默认模型（未来扩展）
    #[serde(default)]
    pub google_default_model: Option<String>,

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

/// 记录每个工具的"上次启动方式"，工具切换时可恢复配置
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LastLaunchConfig {
    pub provider_id: Option<String>,
    pub provider_name: Option<String>,
    pub model_id: Option<String>,
    pub fallback_model_id: Option<String>,
    pub fallback_provider_id: Option<String>,
    pub use_official_model: bool,
    pub terminal_id: String,
    pub one_m_context: bool,
    pub project_path: String,
    pub last_launched_at: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct LastLaunchConfigsFile {
    pub configs: std::collections::HashMap<String, LastLaunchConfig>,
}

// ─── 文件路径 ───

