use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 代理服务器配置
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProxyConfig {
    /// 监听地址
    pub listen_address: String,
    /// 监听端口
    pub listen_port: u16,
    /// 上游 Base URL（OpenAI 兼容）
    pub upstream_base_url: String,
    /// 上游 API Key
    pub upstream_api_key: String,
    /// 上游 Anthropic URL（可选，双协议支持）
    pub upstream_anthropic_url: String,
    /// 上游协议类型："openai" | "anthropic"
    pub upstream_protocol: String,
    /// 目标模型 ID（发送给上游的模型名）
    pub target_model: String,
    /// 请求超时（秒）
    pub timeout_secs: u64,
    /// 模型别名映射：角色关键词 → 实际模型 ID
    /// 例如 {"sonnet": "deepseek-v4-pro", "opus": "claude-opus-4-8"}
    #[serde(default)]
    pub model_aliases: HashMap<String, String>,
    /// 默认模型（当别名无匹配时使用）
    #[serde(default)]
    pub default_model: Option<String>,
    // ─── 整流器开关 ───
    #[serde(default)]
    pub rectifier_enabled: bool,
    #[serde(default)]
    pub rectifier_thinking_signature: bool,
    #[serde(default)]
    pub rectifier_thinking_budget: bool,
    #[serde(default)]
    pub rectifier_media_fallback: bool,
    // ─── 优化器开关 ───
    #[serde(default)]
    pub optimizer_enabled: bool,
    #[serde(default)]
    pub optimizer_cache_injection: bool,
    #[serde(default)]
    pub optimizer_thinking: bool,
    #[serde(default)]
    pub optimizer_deepseek: bool,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            listen_address: "127.0.0.1".to_string(),
            listen_port: 15721,
            upstream_base_url: "https://api.openai.com/v1".to_string(),
            upstream_api_key: String::new(),
            upstream_anthropic_url: String::new(),
            upstream_protocol: "openai".to_string(),
            target_model: "gpt-4o".to_string(),
            timeout_secs: 300,
            model_aliases: HashMap::new(),
            default_model: None,
            rectifier_enabled: true,
            rectifier_thinking_signature: true,
            rectifier_thinking_budget: true,
            rectifier_media_fallback: true,
            optimizer_enabled: true,
            optimizer_cache_injection: true,
            optimizer_thinking: true,
            optimizer_deepseek: true,
        }
    }
}

/// 代理服务器运行状态
#[derive(Clone, Debug, Serialize, Default)]
pub struct ProxyStatus {
    pub running: bool,
    pub address: String,
    pub port: u16,
    pub active_connections: u64,
    pub total_requests: u64,
    pub success_requests: u64,
    pub failed_requests: u64,
}
