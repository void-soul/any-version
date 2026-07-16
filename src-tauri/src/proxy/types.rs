use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 代理服务器配置
///
/// 纯供应商模型下：供应商只讲一种协议（ProxyConfig.outbound_protocol），
/// 端点 URL 单一（upstream_base_url）。每次启动一个独立实例，由工具的
/// 支持协议列表（inbound_protocols）注册入站路由，出站协议 = 供应商协议。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProxyConfig {
    /// 监听地址
    pub listen_address: String,
    /// 监听端口
    pub listen_port: u16,

    // ─── 协议 ───
    /// 入站协议列表：工具支持的协议（"anthropic" | "openai" | "google"）
    /// 代理为其中每种协议注册对应路由，全部转发到同一出站协议。
    #[serde(default)]
    pub inbound_protocols: Vec<String>,
    /// 出站协议：发往上游的协议（同上，= 供应商 protocol）
    pub outbound_protocol: String,
    /// 转换模式（冗余观测字段）："none" | "a2o" | "o2a" | "a2g" | "g2a" | "o2g" | "g2o"
    #[serde(default)]
    pub conversion_mode: String,

    /// 上游 API Key
    pub upstream_api_key: String,
    /// 上游协议端点 URL（供应商 base_url，单一）
    #[serde(default, alias = "upstream_openai_url")]
    pub upstream_base_url: String,

    /// 跨供应商路由：实际模型名 B → 该模型所属供应商的上游端点 + key。
    /// 命中时用其端点/key，否则回退到全局 upstream_base_url / upstream_api_key。
    /// 用于支持「大模型」与「辅助模型」分属不同供应商的场景。
    #[serde(default)]
    pub model_routes: HashMap<String, ModelRoute>,

    /// 目标模型 ID（请求体写入的"实际模型 B"）
    pub target_model: String,
    /// 请求超时（秒）
    pub timeout_secs: u64,
    /// 模型别名映射（伪装）：声明名 C → 实际模型 B
    #[serde(default)]
    pub model_aliases: HashMap<String, String>,
    /// 默认模型（当别名无匹配时使用）
    #[serde(default)]
    pub default_model: Option<String>,

    // ─── 统计归属（强制，修正原硬编码 "proxy"）───
    /// 归属工具 ID（用于 SQLite 落库，如 "claude-code"）
    pub tool_id: String,
    /// 归属供应商 ID
    #[serde(default)]
    pub provider_id: String,

    // ─── 整流器开关 ───
    #[serde(default)]
    pub rectifier_enabled: bool,
    #[serde(default)]
    pub rectifier_thinking_signature: bool,
    #[serde(default)]
    pub rectifier_thinking_budget: bool,
    #[serde(default)]
    pub rectifier_media_fallback: bool,
    /// 协议不匹配整流：剥离转换后仍残留的协议专有字段（如 Anthropic thinking 落到 OpenAI 上游）
    #[serde(default)]
    pub rectifier_protocol_mismatch: bool,

    // ─── 优化器开关 ───
    #[serde(default)]
    pub optimizer_enabled: bool,
    #[serde(default)]
    pub optimizer_cache_injection: bool,
    #[serde(default)]
    pub optimizer_thinking: bool,
    #[serde(default)]
    pub optimizer_deepseek: bool,

    // ─── 协作上下文（可选，协作派发时设置，用于代理 → 前端事件推送）───
    #[serde(skip)]
    pub app_handle: Option<tauri::AppHandle>,
    #[serde(skip)]
    pub collab_room_id: Option<String>,
}

/// 单个模型的供应商路由（跨供应商支持）：该模型实际所属供应商的上游端点与 key。
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ModelRoute {
    pub base_url: String,
    pub api_key: String,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            listen_address: "127.0.0.1".to_string(),
            listen_port: 15721,
            inbound_protocols: vec!["openai".to_string()],
            outbound_protocol: "openai".to_string(),
            conversion_mode: "none".to_string(),
            upstream_api_key: String::new(),
            upstream_base_url: "https://api.openai.com/v1".to_string(),
            model_routes: HashMap::new(),
            target_model: "gpt-4o".to_string(),
            timeout_secs: 300,
            model_aliases: HashMap::new(),
            default_model: None,
            tool_id: String::new(),
            provider_id: String::new(),
            rectifier_enabled: true,
            rectifier_thinking_signature: true,
            rectifier_thinking_budget: true,
            rectifier_media_fallback: true,
            rectifier_protocol_mismatch: true,
            optimizer_enabled: true,
            optimizer_cache_injection: true,
            optimizer_thinking: true,
            optimizer_deepseek: true,
            app_handle: None,
            collab_room_id: None,
        }
    }
}

/// 根据入站/出站协议推导转换模式字符串
pub fn derive_conversion_mode(inbound: &str, outbound: &str) -> String {
    if inbound == outbound {
        "none".to_string()
    } else {
        match (inbound, outbound) {
            ("anthropic", "openai") => "a2o".to_string(),
            ("openai", "anthropic") => "o2a".to_string(),
            ("anthropic", "google") => "a2g".to_string(),
            ("google", "anthropic") => "g2a".to_string(),
            ("openai", "google") => "o2g".to_string(),
            ("google", "openai") => "g2o".to_string(),
            _ => format!("{}_{}", inbound, outbound),
        }
    }
}
