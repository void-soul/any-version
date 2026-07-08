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

// ─── 旧版 ProtocolConfig（仅用于反序列化迁移，不再在新代码中使用）───
#[derive(Deserialize, Clone, Debug, Default)]
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

/// AI 供应商 / 中转站配置（简化版）
///
/// 只关注供应商本身的特征：协议端点 URL、API Key、模型列表、模型别名映射。
/// 代理是否启用由工具启动逻辑决定（工具启动时代理必开），不再在此配置。
#[derive(Serialize, Clone, Debug)]
pub struct AiProvider {
    pub id: String,
    pub name: String,
    pub category: String, // "provider" | "relay"
    pub api_key: String,
    pub website: String,
    /// OpenAI 协议端点 URL（空 = 不支持）
    #[serde(default)]
    pub openai_url: String,
    /// Anthropic 协议端点 URL（空 = 不支持）
    #[serde(default)]
    pub anthropic_url: String,
    /// Google 协议端点 URL（空 = 不支持）
    #[serde(default)]
    pub google_url: String,
    /// 模型别名映射：角色关键词 → 实际模型 ID
    /// 例如 {"sonnet": "deepseek-v4-pro", "opus": "claude-opus-4-8"}
    #[serde(default)]
    pub model_aliases: std::collections::HashMap<String, String>,
    /// 默认模型（当别名无匹配时使用）
    #[serde(default)]
    pub default_model: Option<String>,
    pub models: Vec<ModelEntry>,
    pub active_model_id: Option<String>,
}

impl<'de> Deserialize<'de> for AiProvider {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct Helper {
            id: String,
            name: String,
            #[serde(default = "default_provider_category")]
            category: String,
            api_key: String,
            #[serde(default)]
            website: String,
            models: Vec<ModelEntry>,
            #[serde(default)]
            active_model_id: Option<String>,

            // ─── 新格式：扁平字段 ───
            #[serde(default)]
            openai_url: Option<String>,
            #[serde(default)]
            anthropic_url: Option<String>,
            #[serde(default)]
            google_url: Option<String>,
            #[serde(default)]
            model_aliases: Option<std::collections::HashMap<String, String>>,
            #[serde(default)]
            default_model: Option<String>,

            // ─── 旧格式 v2：protocols HashMap ───
            #[serde(default)]
            protocols: Option<std::collections::HashMap<String, ProtocolConfig>>,

            // ─── 旧格式 v1：扁平带后缀字段 ───
            #[serde(default)]
            openai_enabled: bool,
            // openai_url 已在上面 Option 声明（复用）
            #[serde(default)]
            openai_use_proxy: bool,
            #[serde(default)]
            openai_model_aliases: std::collections::HashMap<String, String>,
            #[serde(default)]
            openai_default_model: Option<String>,

            #[serde(default)]
            anthropic_enabled: bool,
            // anthropic_url 已在上面 Option 声明（复用）
            #[serde(default)]
            anthropic_use_proxy: bool,
            #[serde(default)]
            anthropic_model_aliases: std::collections::HashMap<String, String>,
            #[serde(default)]
            anthropic_default_model: Option<String>,

            #[serde(default)]
            google_enabled: bool,
            // google_url 已在上面 Option 声明（复用）
            #[serde(default)]
            google_model_aliases: std::collections::HashMap<String, String>,
            #[serde(default)]
            google_default_model: Option<String>,
        }

        let h = Helper::deserialize(deserializer)?;

        // 优先使用新格式扁平字段，缺失时从 protocols/旧字段迁移
        let (openai_url, anthropic_url, google_url, model_aliases, default_model) =
            if let Some(protos) = &h.protocols {
                // 从 protocols HashMap 迁移
                let oai = protos.get("openai");
                let ant = protos.get("anthropic");
                let goo = protos.get("google");
                let url = |p: Option<&ProtocolConfig>| p.map(|c| c.url.clone()).unwrap_or_default();
                let aliases = |p: Option<&ProtocolConfig>| p.map(|c| c.model_aliases.clone()).unwrap_or_default();
                let dm = |p: Option<&ProtocolConfig>| p.and_then(|c| c.default_model.clone());

                // model_aliases 优先用 anthropic 的（主要场景），否则用 openai 的
                let ma = {
                    let a = aliases(ant);
                    if !a.is_empty() { a } else { aliases(oai) }
                };
                // default_model 优先 anthropic，否则 openai
                let dm_val = dm(ant).or_else(|| dm(oai));

                (
                    h.openai_url.clone().unwrap_or_else(|| url(oai)),
                    h.anthropic_url.clone().unwrap_or_else(|| url(ant)),
                    h.google_url.clone().unwrap_or_else(|| url(goo)),
                    h.model_aliases.clone().unwrap_or(ma),
                    h.default_model.clone().or(dm_val),
                )
            } else {
                // 从旧格式 v1 扁平字段迁移（或直接使用新格式）
                (
                    h.openai_url.unwrap_or_default(),
                    h.anthropic_url.unwrap_or_default(),
                    h.google_url.unwrap_or_default(),
                    h.model_aliases.unwrap_or_else(|| {
                        // 优先 anthropic aliases，否则 openai
                        let a = h.anthropic_model_aliases.clone();
                        if !a.is_empty() { a } else { h.openai_model_aliases.clone() }
                    }),
                    h.default_model.or(h.anthropic_default_model.clone())
                        .or(h.openai_default_model.clone()),
                )
            };

        Ok(AiProvider {
            id: h.id,
            name: h.name,
            category: h.category,
            api_key: h.api_key,
            website: h.website,
            openai_url,
            anthropic_url,
            google_url,
            model_aliases,
            default_model,
            models: h.models,
            active_model_id: h.active_model_id,
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

