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
    /// 协议不匹配整流器：剥离转换后残留的协议专有字段
    #[serde(default = "default_true")]
    pub protocol_mismatch: bool,
}

impl Default for RectifierConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            thinking_signature: true,
            thinking_budget: true,
            media_fallback: true,
            protocol_mismatch: true,
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

/// AI 供应商 / 中转站配置
///
/// 供应商为每个支持的协议分别配置端点 URL（空字符串 = 不支持该协议）。
/// 工具启动时代理必开，根据供应商「已配置的协议 URL」判断其支持的协议，
/// 并据此决定是否需要做协议转换（工具原生协议不被支持时转换）。
/// 模型别名/伪装由工具启动时的代理按 `tool.builtin_models` 与所选取模型动态生成。
#[derive(Serialize, Clone, Debug)]
pub struct AiProvider {
    pub id: String,
    pub name: String,
    pub category: String, // "provider" | "relay"
    pub api_key: String,
    pub website: String,
    /// OpenAI 协议端点 URL（空 = 不支持 OpenAI 协议）
    #[serde(default)]
    pub openai_url: String,
    /// Anthropic 协议端点 URL（空 = 不支持 Anthropic 协议）
    #[serde(default)]
    pub anthropic_url: String,
    /// Google 协议端点 URL（空 = 不支持 Google 协议）
    #[serde(default)]
    pub google_url: String,
    pub models: Vec<ModelEntry>,
    pub active_model_id: Option<String>,
}

impl AiProvider {
    /// 供应商支持的协议列表（非空 URL 对应的协议）。
    pub fn supported_protocols(&self) -> Vec<String> {
        let mut v = Vec::new();
        if !self.openai_url.is_empty() { v.push("openai".to_string()); }
        if !self.anthropic_url.is_empty() { v.push("anthropic".to_string()); }
        if !self.google_url.is_empty() { v.push("google".to_string()); }
        v
    }

    /// 取某协议的端点 URL（空字符串表示该协议未配置）。
    pub fn url_for(&self, protocol: &str) -> String {
        match protocol {
            "openai" => self.openai_url.clone(),
            "anthropic" => self.anthropic_url.clone(),
            "google" => self.google_url.clone(),
            _ => String::new(),
        }
    }

    /// 首个配置的协议（优先级 openai > anthropic > google），用于手动启动等无工具场景。
    pub fn primary_protocol(&self) -> String {
        if !self.openai_url.is_empty() { "openai".to_string() }
        else if !self.anthropic_url.is_empty() { "anthropic".to_string() }
        else if !self.google_url.is_empty() { "google".to_string() }
        else { "openai".to_string() }
    }
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

            // ─── 新格式：三个协议 URL ───
            #[serde(default)]
            openai_url: Option<String>,
            #[serde(default)]
            anthropic_url: Option<String>,
            #[serde(default)]
            google_url: Option<String>,

            // ─── 旧格式 v3：单一协议 base_url + protocol ───
            #[serde(default)]
            base_url: Option<String>,
            #[serde(default)]
            protocol: Option<String>,

            // ─── 旧格式 v2：protocols HashMap ───
            #[serde(default)]
            protocols: Option<std::collections::HashMap<String, ProtocolConfig>>,

            // ─── 旧格式 v1：扁平带后缀字段（openai_url 等与新格式同名，直接复用）───
            #[serde(default)]
            openai_enabled: bool,
            #[serde(default)]
            anthropic_enabled: bool,
            #[serde(default)]
            google_enabled: bool,
            // default_model 系列不再使用，仅用于兼容旧数据（已丢弃）
            #[serde(default)]
            default_model: Option<String>,
            #[serde(default)]
            openai_default_model: Option<String>,
            #[serde(default)]
            anthropic_default_model: Option<String>,
            #[serde(default)]
            google_default_model: Option<String>,
        }

        let h = Helper::deserialize(deserializer)?;

        let mut openai_url = h.openai_url.clone().unwrap_or_default();
        let mut anthropic_url = h.anthropic_url.clone().unwrap_or_default();
        let mut google_url = h.google_url.clone().unwrap_or_default();

        // 旧格式 v3：单一 base_url + protocol 折叠到对应协议 URL
        if let (Some(bu), Some(proto)) = (&h.base_url, &h.protocol) {
            match proto.as_str() {
                "anthropic" => anthropic_url = bu.clone(),
                "google" => google_url = bu.clone(),
                _ => openai_url = bu.clone(),
            }
        } else if let Some(protos) = &h.protocols {
            // 旧格式 v2：protocols HashMap，逐协议填入（仅在对应新字段为空时覆盖）
            if openai_url.is_empty() {
                if let Some(c) = protos.get("openai") { openai_url = c.url.clone(); }
            }
            if anthropic_url.is_empty() {
                if let Some(c) = protos.get("anthropic") { anthropic_url = c.url.clone(); }
            }
            if google_url.is_empty() {
                if let Some(c) = protos.get("google") { google_url = c.url.clone(); }
            }
        }
        // 旧格式 v1 扁平字段：openai_url / anthropic_url / google_url 与新格式同名字段，
        // 已在上方直接读取，无需额外处理。

        Ok(AiProvider {
            id: h.id,
            name: h.name,
            category: h.category,
            api_key: h.api_key,
            website: h.website,
            openai_url,
            anthropic_url,
            google_url,
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

/// 技能清单（持久化到 skills.json 的元数据）。
///
/// 注意：`installed_tools`（已部署到哪些工具）**不在此处持久化** —— 安装状态由
/// `get_skills` 扫描各工具 skills 目录实时推导。这样既能反映 AnyVersion 部署的链路，
/// 也能发现被工具私自安装、但不在全局仓库的技能。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub directory: String,
    pub installed_at: String,
    pub install_method: String,
}

/// 技能视图（返回给前端）：在 `Skill` 基础上补充实时推导的 `installed_tools`。
#[derive(Serialize, Clone, Debug)]
pub struct SkillView {
    pub id: String,
    pub name: String,
    pub description: String,
    pub directory: String,
    /// 由 AnyVersion 统一安装（junction 指向全局仓库）的工具
    pub installed_tools: Vec<String>,
    /// 工具私自安装 / junction 到其他目录的工具（非 AnyVersion 托管，可迁移）
    pub foreign_tools: Vec<String>,
    pub installed_at: String,
    pub install_method: String,
}

/// 技能总览（「AnyVersion 技能」列表 / 区块二）：仅包含源文件位于 AnyVersion 技能目录
/// （配置驱动，默认 `~/.any-version/skills`，含 `.system` 嵌套）的技能。
///
/// 托管的权威定义：**源文件是否物理位于 AnyVersion 技能目录**（`in_store`）。
/// 源文件不在该目录（工具私有目录 / 外部 junction）一律视为「无托管」，不会进入本列表
/// （归入 `get_discoverable_skills` 的可移动列表）。是否登记进 manifest（`registered`）
/// 只是 AnyVersion 的跟踪元数据，不改变「是否托管」的判定。
#[derive(Serialize, Clone, Debug)]
pub struct SkillOverview {
    /// canonical id（技能的文件夹名）
    pub id: String,
    pub name: String,
    pub description: String,
    /// 源文件是否位于 AnyVersion 技能目录（默认 ~/.any-version/skills，含 .system）。
    /// 这是「托管」的**唯一权威定义**：true = 被 AnyVersion 托管；false = 无托管。
    pub in_store: bool,
    /// 是否登记进 AnyVersion manifest（skills.json）。
    /// `in_store && !registered`：物理上已被 AnyVersion 目录托管（如 skills.sh 装入 .system），
    /// 但未被 AnyVersion 跟踪，前端显示「未纳入」并可「纳入管理」。
    pub registered: bool,
    /// 全局仓库路径（源文件实际位置）
    pub directory: String,
    pub installed_at: String,
    pub install_method: String,
    /// 每个工具上的安装现状
    pub tools: Vec<SkillToolStatus>,
}

/// 单个工具上的技能安装现状
#[derive(Serialize, Clone, Debug)]
pub struct SkillToolStatus {
    pub tool_id: String,
    /// "managed"（AnyVersion 托管）| "foreign"（工具私自安装 / 外部 junction）| "none"（未安装）
    pub status: String,
}

/// 工具私自安装、未由 AnyVersion 托管的技能（可迁移为托管方式）
#[derive(Serialize, Clone, Debug)]
pub struct ForeignSkill {
    /// 所属工具 id
    pub tool_id: String,
    /// 技能目录名（作为 id）
    pub skill_id: String,
    pub name: String,
    pub description: String,
    /// "real"（工具真实目录，情况A）| "external_junction"（junction 指向其他目录，情况B）
    pub kind: String,
    /// 真实数据源路径（目录本身或 junction 目标）。情况B 时为 junction 目标，即 link_target
    pub source_path: String,
    /// 该技能是否已在 AnyVersion 全局仓库（默认 ~/.any-version/skills，含 .system 嵌套）中存在。
    /// true = 整理时只需为工具重建 junction（relink）；false = 需先拷贝数据再建 junction。
    pub already_in_anyversion: bool,
}

/// 可移动技能（发现的可移动到 AnyVersion 目录的目标），按 skill_id 聚合多个工具位置。
/// 对应前端「发现的可移动技能」列表：展示叫什么、在哪里、情况A/B。
#[derive(Serialize, Clone, Debug)]
pub struct DiscoverableSkill {
    pub skill_id: String,
    pub name: String,
    pub description: String,
    /// 是否已在 AnyVersion 全局仓库中存在（决定整理时是「仅 relink」还是「需拷贝」）
    pub already_in_anyversion: bool,
    /// 各工具位置：情况A（直装）/ 情况B（外部 junction）及链接目标
    pub locations: Vec<SkillLocation>,
}

/// 可移动技能的一个工具位置
#[derive(Serialize, Clone, Debug)]
pub struct SkillLocation {
    pub tool_id: String,
    /// "A"=工具目录直装（真实目录）| "B"=junction 到非 AnyVersion 目录
    pub case: String,
    /// 情况B 的 junction 目标路径；情况A 为空
    pub link_target: String,
}

/// 技能问题（问题检测列表项）。
///
/// 问题类型：
/// - `"skills_sh"`: skills.sh 等管理工具目录中发现的可导入技能
/// - `"A"`: AI 工具目录中直接安装的技能（真实目录）
/// - `"B"`: AI 工具目录中 junction 指向非 AnyVersion 仓库的技能
/// - `"D"`: AI 工具目录中 junction 目标已失效（断链）
///
/// Case C（junction 指向 AnyVersion 仓库 = 已托管）不算问题，不在此列出。
#[derive(Serialize, Clone, Debug)]
pub struct SkillIssue {
    /// 问题类型："skills_sh" | "A" | "B" | "D"
    pub issue_type: String,
    /// 来源标识："skills.sh" 或工具 id
    pub tool_id: String,
    /// 技能 id（文件夹名）
    pub skill_id: String,
    pub name: String,
    pub description: String,
    /// 当前源路径（工具目录中的路径或 skills.sh 中的路径）
    pub source_path: String,
    /// junction 目标路径（Case B/D）；非 junction 时为空
    pub link_target: String,
    /// 是否已在 AnyVersion 仓库中存在（决定修复方式：仅需 relink 还是需要先拷贝）
    pub already_in_store: bool,
}

/// 问题引用（用于批量修复时传入 tool_id + skill_id）
#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct IssueRef {
    pub tool_id: String,
    pub skill_id: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct ScannedSkill {
    pub name: String,
    pub description: String,
    pub directory: String,
    pub full_path: String,
    pub found_in: Vec<String>,
    pub is_symlink: bool,
    /// 该技能是否已在 AnyVersion 全局仓库（默认 ~/.any-version/skills）中存在
    pub in_global: bool,
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
    pub by_provider: Vec<UsageByProvider>,
    pub recent: Vec<UsageDaily>,
}

#[derive(Serialize, Clone, Debug)]
pub struct UsageByTool {
    pub tool_id: String,
    pub request_count: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct UsageByModel {
    pub model: String,
    pub provider: String,
    pub request_count: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct UsageByProvider {
    pub provider: String,
    pub request_count: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct UsageDaily {
    pub date: String,
    pub request_count: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
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

#[derive(Deserialize, Clone, Debug, Default)]
pub struct LaunchAiToolRequest {
    pub tool_id: String,
    pub project_path: String,
    pub model_id: Option<String>,
    pub provider_id: Option<String>,
    /// fallback/light 模型（低级任务用）
    pub fallback_model_id: Option<String>,
    /// fallback/light 模型所属供应商 id（可为空；空 = 与大类模型同供应商）
    #[serde(default)]
    pub fallback_provider_id: Option<String>,
    /// fallback/light 模型的伪装声明名 C（可为空；空 = 不伪装，直接以实际供应商模型 B 名义请求）
    #[serde(default)]
    pub fallback_masquerade_model: Option<String>,
    pub session_id: Option<String>,
    pub session_mode: String,
    pub terminal_id: String,
    /// Claude Code relay-only: append [1m] to model id for 1M context window
    #[serde(default)]
    pub one_m_context: bool,
    /// fallback/light 模型是否同样追加 [1m]（可与主模型独立勾选）
    #[serde(default)]
    pub fallback_one_m_context: bool,
    /// 模型伪装：工具「以为自己调用的模型名」C（`tool.builtin_models` 中的一项）。
    /// 为空表示不作伪装，工具直接以所选取的供应商模型 B 名义请求。
    /// 代理将把请求中的 C 改写为实际模型 B（masquerade C → B）。
    #[serde(default)]
    pub masquerade_model: Option<String>,
    /// 当前启动是否启用优化器（None = 由工具能力 + 全局配置决定）
    #[serde(default)]
    pub optimizer_enabled: Option<bool>,
    /// 当前启动是否启用整流器（None = 由工具能力 + 全局配置决定）
    #[serde(default)]
    pub rectifier_enabled: Option<bool>,
    /// 整流器各策略开关（None = 沿用全局配置 AiConfig.rectifier.*）
    #[serde(default)]
    pub rectifier_thinking_signature: Option<bool>,
    #[serde(default)]
    pub rectifier_thinking_budget: Option<bool>,
    #[serde(default)]
    pub rectifier_media_fallback: Option<bool>,
    #[serde(default)]
    pub rectifier_protocol_mismatch: Option<bool>,
    /// 优化器各策略开关（None = 沿用全局配置 AiConfig.optimizer.*）
    #[serde(default)]
    pub optimizer_cache_injection: Option<bool>,
    #[serde(default)]
    pub optimizer_thinking: Option<bool>,
    #[serde(default)]
    pub optimizer_deepseek: Option<bool>,
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
    /// fallback/light 模型的伪装声明名 C（空表示不伪装）
    #[serde(default)]
    pub fallback_masquerade_model: Option<String>,
    pub use_official_model: bool,
    pub terminal_id: String,
    pub one_m_context: bool,
    /// fallback/light 模型是否同样追加 [1m]
    #[serde(default)]
    pub fallback_one_m_context: bool,
    pub project_path: String,
    /// 模型伪装：工具「以为自己调用的模型名」C（tool.builtinModels 中的一项），空表示不伪装
    #[serde(default)]
    pub masquerade_model: Option<String>,
    /// 本次启动是否启用优化器（None 表示沿用工具能力 + 全局配置）
    #[serde(default)]
    pub optimizer_enabled: Option<bool>,
    /// 本次启动是否启用整流器
    #[serde(default)]
    pub rectifier_enabled: Option<bool>,
    pub last_launched_at: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct LastLaunchConfigsFile {
    pub configs: std::collections::HashMap<String, LastLaunchConfig>,
}

// ─── 文件路径 ───

