use serde::{Deserialize, Serialize};


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

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ProtocolConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub use_proxy: bool,
    #[serde(default)]
    pub model_aliases: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub default_model: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct AiProvider {
    pub id: String,
    pub name: String,
    pub category: String, // "provider" | "relay"
    pub api_key: String,
    pub website: String,
    pub protocols: std::collections::HashMap<String, ProtocolConfig>,
    pub models: Vec<ModelEntry>,
    pub active_model_id: Option<String>,
}

impl<'de> Deserialize<'de> for AiProvider {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Helper {
            id: String,
            name: String,
            #[serde(default = "default_provider_category")]
            category: String,
            api_key: String,
            #[serde(default)]
            website: String,
            #[serde(default)]
            protocols: Option<std::collections::HashMap<String, ProtocolConfig>>,
            
            // Old fields for migration:
            #[serde(default)]
            openai_enabled: bool,
            #[serde(default)]
            openai_url: String,
            #[serde(default)]
            openai_use_proxy: bool,
            #[serde(default)]
            openai_model_aliases: std::collections::HashMap<String, String>,
            #[serde(default)]
            openai_default_model: Option<String>,

            #[serde(default)]
            anthropic_enabled: bool,
            #[serde(default)]
            anthropic_url: String,
            #[serde(default)]
            anthropic_use_proxy: bool,
            #[serde(default)]
            anthropic_model_aliases: std::collections::HashMap<String, String>,
            #[serde(default)]
            anthropic_default_model: Option<String>,

            #[serde(default)]
            google_enabled: bool,
            #[serde(default)]
            google_url: String,
            #[serde(default)]
            google_model_aliases: std::collections::HashMap<String, String>,
            #[serde(default)]
            google_default_model: Option<String>,

            models: Vec<ModelEntry>,
            active_model_id: Option<String>,
        }

        let helper = Helper::deserialize(deserializer)?;
        
        let protocols = if let Some(mut protos) = helper.protocols {
            if !protos.contains_key("openai") {
                protos.insert("openai".to_string(), ProtocolConfig::default());
            }
            if !protos.contains_key("anthropic") {
                protos.insert("anthropic".to_string(), ProtocolConfig::default());
            }
            if !protos.contains_key("google") {
                protos.insert("google".to_string(), ProtocolConfig::default());
            }
            protos
        } else {
            let mut protos = std::collections::HashMap::new();
            
            protos.insert("openai".to_string(), ProtocolConfig {
                enabled: helper.openai_enabled,
                url: helper.openai_url,
                use_proxy: helper.openai_use_proxy,
                model_aliases: helper.openai_model_aliases,
                default_model: helper.openai_default_model,
            });

            protos.insert("anthropic".to_string(), ProtocolConfig {
                enabled: helper.anthropic_enabled,
                url: helper.anthropic_url,
                use_proxy: helper.anthropic_use_proxy,
                model_aliases: helper.anthropic_model_aliases,
                default_model: helper.anthropic_default_model,
            });

            protos.insert("google".to_string(), ProtocolConfig {
                enabled: helper.google_enabled,
                url: helper.google_url,
                use_proxy: false,
                model_aliases: helper.google_model_aliases,
                default_model: helper.google_default_model,
            });

            protos
        };

        Ok(AiProvider {
            id: helper.id,
            name: helper.name,
            category: helper.category,
            api_key: helper.api_key,
            website: helper.website,
            protocols,
            models: helper.models,
            active_model_id: helper.active_model_id,
        })
    }
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
    /// 恢复命令（已填入 session_id 的实际可执行命令，如 "opencode -s abc123"）
    pub resume_cmd: Option<String>,
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

