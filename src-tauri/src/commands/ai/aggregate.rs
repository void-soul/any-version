//! 聚合服务：本地 HTTP 聚合代理（按聚合链顺序请求，可启停 + 日志回传）。
//!
//! - 对外单一模型入口：`kiro-proxy`（聚合 = 多个候选对内的单一入口）
//! - 入口协议：OpenAI `/v1/chat/completions` + Anthropic `/v1/messages`
//! - 候选出口协议：按供应商配置的 `openai_url` / `anthropic_url` / `google_url` 决定
//!   （openai → anthropic → google 优先级），协议转换复用 `proxy::transform` / `proxy::google`
//! - 链路顺序：按 `route_chain` 依次尝试；失败按类别决定重试/切换，并进入冷却
//! - 上下文限制：请求超过 `AggregateConfig.context_limit` 时裁剪最早的非 system 消息
//! - 压缩：Headroom 开启时先走 `/v1/compress`，并用 `frozen_message_count` 保持多轮前缀缓存
//! - 链路热更新：每次请求实时读取配置，改完链路无需重启服务

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;
use serde_json::Value;
use tauri::Emitter;

use super::config::{load_ai_config, save_ai_config_to_file};
use super::models::{AggregateConfig, AiConfig, AiProvider, RouteCandidate};
use super::route::normalize_route_chain;
use super::usage::{log_usage_db_timed, log_usage_entry, log_usage_failure, UsageEntry};
use crate::proxy::optimizers;
use crate::proxy::types::{ModelRoute, ProxyConfig};

/// 聚合服务对外只暴露一个模型（对内才按链分发）——这也是「聚合」的含义。
pub const AGGREGATE_MODEL_ID: &str = "kiro-proxy";

// ─── 失败分类与切换策略（纯逻辑，便于单测） ───

/// 上游失败类别，决定「是否重试 / 是否切换 / 冷却多久」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureClass {
    /// 鉴权失败（401/403）：key 有问题，长时间冷却
    Authentication,
    /// 额度耗尽 / 计费问题（402 或 quota 文案）：长时间冷却
    Billing,
    /// 模型不存在或不支持（404 / model_not_found）：长时间冷却
    ModelUnavailable,
    /// 限流（429 / rate limit 文案）：按 Retry-After 或 60s 冷却
    RateLimit,
    /// 瞬时错误（408/409/5xx/超时/连不上）：可重试，指数退避冷却
    Transient,
    /// 不重试也不切换：内容安全等策略拒绝，直接把上游错误返回给客户端
    Fatal,
}

impl FailureClass {
    /// 落库用的稳定标识（**不要改这些字符串**：已写入 ai_usage.failure_class 的是它们）。
    pub fn as_str(self) -> &'static str {
        match self {
            FailureClass::Authentication => "authentication",
            FailureClass::Billing => "billing",
            FailureClass::ModelUnavailable => "model_unavailable",
            FailureClass::RateLimit => "rate_limit",
            FailureClass::Transient => "transient",
            FailureClass::Fatal => "fatal",
        }
    }
}

/// 判定失败类别：先按响应体文案（更准），再按状态码。
pub fn classify_failure(status: Option<u16>, body: &str) -> FailureClass {
    let lower = body.to_ascii_lowercase();
    // 内容安全 / 策略拒绝：换供应商也一样，直接返回给客户端
    const FATAL_HINTS: &[&str] = &[
        "content policy",
        "safety policy",
        "content moderation",
        "blocked by safety",
    ];
    if FATAL_HINTS.iter().any(|h| lower.contains(h)) {
        return FailureClass::Fatal;
    }
    const BILLING_HINTS: &[&str] = &[
        "insufficient_quota",
        "insufficient quota",
        "quota exceeded",
        "credit balance",
        "credits exhausted",
        "余额不足",
        "额度不足",
    ];
    if BILLING_HINTS.iter().any(|h| lower.contains(h)) {
        return FailureClass::Billing;
    }
    const MODEL_HINTS: &[&str] = &[
        "model_not_found",
        "model not found",
        "unknown model",
        "does not support tool",
        "function calling is not supported",
    ];
    if MODEL_HINTS.iter().any(|h| lower.contains(h)) {
        return FailureClass::ModelUnavailable;
    }
    const RATE_HINTS: &[&str] = &["rate limit", "rate_limit", "too many requests"];
    if RATE_HINTS.iter().any(|h| lower.contains(h)) {
        return FailureClass::RateLimit;
    }
    match status {
        Some(401) | Some(403) => FailureClass::Authentication,
        Some(402) => FailureClass::Billing,
        Some(404) => FailureClass::ModelUnavailable,
        Some(429) => FailureClass::RateLimit,
        Some(408) | Some(409) => FailureClass::Transient,
        Some(s) if (500..=599).contains(&s) => FailureClass::Transient,
        // 连接失败 / 超时（无状态码）也归瞬时
        None => FailureClass::Transient,
        _ => FailureClass::Transient,
    }
}

/// 同一候选的尝试次数（含首次）。鉴权/额度/模型/限流不重试，直接切下一个。
pub fn retry_budget(class: FailureClass) -> usize {
    match class {
        FailureClass::Transient => 2,
        _ => 1,
    }
}

/// 该失败是否值得换到下一个候选。
///
/// 抄 cc-switch `forwarder.rs` 的换家判定：只有「换一家有可能成功」的错误才换。
/// 落到我们的分类上：
/// - `Fatal`（内容策略拒绝）不换 —— 换哪家都一样；
/// - **请求体层面的拒绝（400 / 422）不换** —— 这是我们这条请求本身的问题，
///   换供应商也修不好，白跑一遍整条链还会把每个候选都拖进冷却；
/// - 其余（5xx / 超时 / 连不上 / 401 / 402 / 404 / 429）都是**候选自身**的问题
///   （凭据坏了、额度没了、模型没了、被限流）→ 换一家很可能就成了。
///
/// 与 cc-switch 的偏差是刻意的：它把整个 4xx 都归为「不换」，但我们的候选链里
/// 401/402/404/429 是**按候选**变化的（每家 key、额度、上架模型都不同），
/// 全部不换会让故障转移在这些最常见的场景下完全失效。
pub fn should_switch_candidate(class: FailureClass, status: Option<u16>) -> bool {
    if class == FailureClass::Fatal {
        return false;
    }
    !matches!(status, Some(400) | Some(422))
}

/// 该类别是否值得对同一候选再试一次（只有瞬时错误才重试，其余直接切下一个）。
fn is_retryable(class: FailureClass) -> bool {
    retry_budget(class) > 1
}

/// 冷却时长（切换后多久内不再尝试该候选）。
pub fn cooldown_for(
    class: FailureClass,
    retry_after: Option<u64>,
    consecutive: u32,
) -> std::time::Duration {
    const TRANSIENT_BASE: u64 = 30;
    const TRANSIENT_MAX: u64 = 5 * 60;
    const RATE_LIMIT_DEFAULT: u64 = 60;
    const RATE_LIMIT_MAX: u64 = 24 * 60 * 60;
    const AUTH_OR_BILLING: u64 = 60 * 60;
    const MODEL_UNAVAILABLE: u64 = 6 * 60 * 60;
    match class {
        FailureClass::Authentication | FailureClass::Billing => {
            std::time::Duration::from_secs(AUTH_OR_BILLING)
        }
        FailureClass::ModelUnavailable => std::time::Duration::from_secs(MODEL_UNAVAILABLE),
        FailureClass::RateLimit => std::time::Duration::from_secs(
            retry_after.unwrap_or(RATE_LIMIT_DEFAULT).min(RATE_LIMIT_MAX),
        ),
        FailureClass::Transient => {
            // 30s → 60s → 120s → 240s → 300s（封顶）
            let shift = consecutive.saturating_sub(1).min(4);
            let secs = (TRANSIENT_BASE << shift).min(TRANSIENT_MAX);
            std::time::Duration::from_secs(secs)
        }
        FailureClass::Fatal => std::time::Duration::from_secs(0),
    }
}

/// 候选健康状态（服务进程内维护；重启即清空）。
#[derive(Debug, Clone, Default)]
struct CandidateHealth {
    consecutive_failures: u32,
    cooldown_until_ms: Option<u64>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 候选剩余冷却秒数（未冷却返回 None）。
fn cooldown_remaining_secs(
    health: &Arc<Mutex<HashMap<String, CandidateHealth>>>,
    key: &str,
) -> Option<u64> {
    let guard = health.lock().ok()?;
    let until = guard.get(key)?.cooldown_until_ms?;
    let now = now_ms();
    if until > now {
        Some((until - now) / 1000)
    } else {
        None
    }
}

/// 记录一次失败：累加连续失败次数并按类别设置冷却，返回冷却时长。
fn record_failure(
    health: &Arc<Mutex<HashMap<String, CandidateHealth>>>,
    key: &str,
    class: FailureClass,
    retry_after: Option<u64>,
) -> std::time::Duration {
    let mut guard = match health.lock() {
        Ok(g) => g,
        Err(_) => return std::time::Duration::from_secs(0),
    };
    let entry = guard.entry(key.to_string()).or_default();
    entry.consecutive_failures = entry.consecutive_failures.saturating_add(1);
    let cooldown = cooldown_for(class, retry_after, entry.consecutive_failures);
    entry.cooldown_until_ms = Some(now_ms().saturating_add(cooldown.as_millis() as u64));
    cooldown
}

/// 成功后清除该候选的健康记录（失败计数与冷却一起清零）。
fn clear_health(health: &Arc<Mutex<HashMap<String, CandidateHealth>>>, key: &str) {
    if let Ok(mut guard) = health.lock() {
        guard.remove(key);
    }
}

// ─── 自引用防护 ───

/// 单个候选请求失败的结构化信息。
struct CandidateError {
    class: FailureClass,
    message: String,
    retry_after: Option<u64>,
    /// 上游 HTTP 状态码（连接失败/超时为 None）
    status: Option<u16>,
    /// 上游原始错误体：反应式整流靠它判断「能不能修」
    error_body: String,
    /// 本次实际发出去的 P_out 请求体（整流器操作的就是这个形态）
    outbound_body: Value,
}

/// 过滤掉自引用候选（会造成递归），返回 (可用候选, 被剔除数量)。
pub fn filter_self_referential(
    candidates: Vec<AggCandidate>,
    aggregate_port: u16,
) -> (Vec<AggCandidate>, usize) {
    let total = candidates.len();
    let kept: Vec<AggCandidate> = candidates
        .into_iter()
        .filter(|c| !is_self_referential(&c.base_url, aggregate_port))
        .collect();
    let removed = total - kept.len();
    (kept, removed)
}

/// 判断一个上游端点是否是「聚合服务自身」。
///
/// 高危场景：把「本地聚合」这类指向本服务的供应商也加进聚合链，
/// 转发时就会打到自己 → 请求无限递归。启动与转发两侧都要拦。
pub fn is_self_referential(base_url: &str, aggregate_port: u16) -> bool {
    let raw = base_url.trim();
    if raw.is_empty() {
        return false;
    }
    let without_scheme = match raw.split_once("://") {
        Some((_scheme, rest)) => rest,
        None => raw,
    };
    let authority = without_scheme.split('/').next().unwrap_or("").trim();
    if authority.is_empty() {
        return false;
    }
    let (host, port_part) = match authority.rfind(':') {
        Some(idx) => (&authority[..idx], Some(&authority[idx + 1..])),
        None => (authority, None),
    };
    let host = host.trim().trim_matches('[').trim_matches(']').to_ascii_lowercase();
    let is_loopback = matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1" | "0.0.0.0");
    let port_matches = port_part
        .and_then(|p| p.parse::<u16>().ok())
        .map(|p| p == aggregate_port)
        .unwrap_or(false);
    is_loopback && port_matches
}

// ─── 候选与出站协议 ───

/// 候选出口协议（按供应商配置的 URL 决定）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outbound {
    OpenAi,
    Anthropic,
    Google,
}

impl Outbound {
    /// 转成 `proxy::upstream::resolve_url` 用的协议字符串。
    fn as_str(&self) -> &'static str {
        match self {
            Outbound::OpenAi => "openai",
            Outbound::Anthropic => "anthropic",
            Outbound::Google => "google",
        }
    }
}

/// 单个候选（每次请求实时解析）：上游端点与凭据。
#[derive(Clone, Debug)]
struct AggCandidate {
    provider_id: String,
    provider_name: String,
    outbound: Outbound,
    base_url: String,
    api_key: String,
    model_id: String,
    /// 该候选拼接时是否补 `/v1`：None = 自动（URL 结尾已是 /v1 就不补）。
    include_v1: Option<bool>,
    /// 该供应商的自定义上游请求头（未配置则为空）。以前聚合转发完全忽略这个字段，
    /// 导致需要 X-Request-Id / 厂商标识的网关在聚合路径被拒。
    headers: Vec<crate::proxy::types::UpstreamHeader>,
    /// 用户在聚合链上显式声明的「该模型是否支持图片」（None = 交给注册表启发式）。
    supports_image: Option<bool>,
}

/// 为候选构造一份 **协议整流用的 ProxyConfig**。
///
/// 为什么必须有：整流器（`apply_optimizers` / `apply_preventive_rectifiers` /
/// `try_reactive_rectify`）此前只在 `proxy::server`（单工具的协议转换代理）里被调用，
/// 聚合服务一条都没调 —— 于是「走代理的启动」和「走聚合的启动」协议行为不一致，
/// 同一份 AI 配置在两条路径上表现不同。这里让聚合复用**同一份**整流实现，
/// 只是把端点/凭据/出站协议换成当前候选的。
///
/// 注意不要在单测里构造它（见 `proxy::optimizers` 的注释）：`#[serde(skip)]` 的
/// `app_handle` 会把 tauri 运行时链进测试二进制。
fn candidate_proxy_config(candidate: &AggCandidate, cfg: &AiConfig) -> ProxyConfig {
    let r = &cfg.rectifier;
    let o = &cfg.optimizer;
    let mut config = ProxyConfig {
        outbound_protocol: candidate.outbound.as_str().to_string(),
        upstream_api_key: candidate.api_key.clone(),
        upstream_base_url: candidate.base_url.clone(),
        upstream_headers: candidate.headers.clone(),
        upstream_include_v1: candidate.include_v1,
        target_model: candidate.model_id.clone(),
        tool_id: "aggregate".to_string(),
        provider_id: candidate.provider_id.clone(),
        rectifier_enabled: r.enabled,
        rectifier_thinking_signature: r.thinking_signature,
        rectifier_thinking_budget: r.thinking_budget,
        rectifier_media_fallback: r.media_fallback,
        rectifier_media_heuristic: r.media_heuristic,
        rectifier_protocol_mismatch: r.protocol_mismatch,
        rectifier_toolcall_dialect: r.toolcall_dialect,
        optimizer_enabled: o.enabled,
        optimizer_cache_injection: o.cache_injection,
        optimizer_thinking: o.thinking_optimizer,
        optimizer_deepseek: o.deepseek_normalize,
        ..ProxyConfig::default()
    };
    // 显式能力声明按模型名挂进路由表：整流器从 `model_routes` 取「声明」，
    // 与单工具代理走完全相同的解析路径（声明 > 注册表）。
    if candidate.supports_image.is_some() {
        config.model_routes.insert(
            candidate.model_id.clone(),
            ModelRoute {
                supports_image: candidate.supports_image,
                ..ModelRoute::default()
            },
        );
    }
    config
}

/// 按出站协议拼上游 URL，返回 (url, 鉴权头名)。
/// 拼接统一走 `proxy::upstream`（与协议转换代理共用，避免 /v1/v1 重复或行为漂移）。
pub fn build_candidate_url(
    outbound: Outbound,
    base: &str,
    model: &str,
    is_stream: bool,
    include_v1: Option<bool>,
) -> (String, &'static str) {
    crate::proxy::upstream::resolve_url(outbound.as_str(), base, model, is_stream, include_v1)
}

/// 从供应商配置选出出站协议与端点（openai → anthropic → google 优先级）。
///
/// 第二个返回值是该协议「要不要补 `/v1`」的开关（None = 自动）。
fn pick_outbound(provider: &AiProvider) -> Option<(Outbound, String, Option<bool>)> {
    let openai = provider.openai_url.trim().to_string();
    let anthropic = provider.anthropic_url.trim().to_string();
    let google = provider.google_url.trim().to_string();
    if !openai.is_empty() {
        return Some((Outbound::OpenAi, openai, provider.openai_include_v1));
    }
    if !anthropic.is_empty() {
        return Some((Outbound::Anthropic, anthropic, provider.anthropic_include_v1));
    }
    if !google.is_empty() {
        return Some((Outbound::Google, google, None));
    }
    None
}

/// 构建候选快照：把链路里的 (provider_id, model_id) 解析成可用的上游端点。
/// 自引用（指向聚合服务自身）的候选直接剔除，避免请求递归。
fn build_candidates(
    providers: &[AiProvider],
    chain: &[RouteCandidate],
    aggregate_port: u16,
) -> Vec<AggCandidate> {
    let mut out = Vec::new();
    for candidate in chain {
        let Some(provider) = providers.iter().find(|p| p.id == candidate.provider_id) else {
            continue;
        };
        let Some((outbound, base, include_v1)) = pick_outbound(provider) else {
            continue;
        };
        let base = base.trim().trim_end_matches('/').to_string();
        if base.is_empty() || provider.api_key.trim().is_empty() {
            continue;
        }
        if is_self_referential(&base, aggregate_port) {
            continue;
        }
        out.push(AggCandidate {
            provider_id: provider.id.clone(),
            provider_name: if provider.name.trim().is_empty() { provider.id.clone() } else { provider.name.clone() },
            outbound,
            base_url: base,
            api_key: provider.api_key.clone(),
            model_id: candidate.model_id.clone(),
            include_v1,
            headers: crate::proxy::headers::normalize(&provider.custom_headers),
            supports_image: candidate.supports_image,
        });
    }
    out
}

/// 把「与请求模型名匹配」的候选提到最前（其余保持链序）。
///
/// 「聚合挂代理上游」的关键：工具经协议转换代理把真实模型名透传给聚合，聚合据此优先落到
/// 配置了该模型的供应商，而不是机械地按链序第一个候选走。请求名是聚合的入口名（`kiro-proxy`
/// 或用户自定义的 entry_model）时视为「无偏好」，保持链序。
///
/// 找不到任何匹配则原样返回（纯链序，向后兼容）。
fn prioritize_by_model(
    candidates: Vec<AggCandidate>,
    requested_model: &str,
    entry_model_name: &str,
) -> Vec<AggCandidate> {
    let requested = requested_model.trim();
    if requested.is_empty() || requested == entry_model_name || requested == AGGREGATE_MODEL_ID {
        return candidates;
    }
    let mut matched = Vec::with_capacity(candidates.len());
    let mut rest = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if candidate.model_id == requested {
            matched.push(candidate);
        } else {
            rest.push(candidate);
        }
    }
    matched.extend(rest);
    matched
}

// ─── Headroom 压缩旁路 + 多轮前缀缓存 ───

#[derive(Clone, Debug)]
struct HeadroomRuntime {
    base_url: String,
    timeout_ms: u64,
    /// true = failClosed（压缩不可用直接报错）；false = failOpen（跳过压缩继续）
    fail_closed: bool,
}

/// 多轮前缀缓存：记录上一轮「客户端发来的原始 messages」与「实际转发出去的压缩 messages」。
/// 下一轮若客户端消息以上一轮原始前缀开头（同一会话增长），则复用压缩前缀并携带
/// `frozen_message_count`，避免每轮重新压缩打爆 provider 的 KV 缓存。
#[derive(Debug, Clone)]
pub struct PrefixSnapshot {
    pub original: Vec<Value>,
    pub forwarded: Vec<Value>,
}

/// 纯函数：按前缀缓存计算本轮实际发送的 messages 与 frozen_message_count。
/// 不匹配（新会话/消息被改写）时原样返回、frozen = 0。
pub fn apply_frozen_prefix(messages: &[Value], cache: Option<&PrefixSnapshot>) -> (Vec<Value>, usize) {
    let Some(cache) = cache else {
        return (messages.to_vec(), 0);
    };
    let n = cache.original.len();
    if n == 0 || messages.len() < n || messages[..n] != cache.original[..] {
        return (messages.to_vec(), 0);
    }
    let mut out = cache.forwarded.clone();
    out.extend(messages[n..].iter().cloned());
    (out, cache.forwarded.len())
}

// ─── 服务状态 ───

struct AggregateEntry {
    port: u16,
    handle: tokio::task::JoinHandle<()>,
}

static AGGREGATE: OnceLock<Mutex<Option<AggregateEntry>>> = OnceLock::new();

#[derive(Clone)]
struct AggState {
    app: tauri::AppHandle,
    /// 本服务监听端口（用于运行期自引用兜底判定）
    port: u16,
    /// 候选健康状态（失败计数 + 冷却截止时间），键 = `provider::model`
    health: Arc<Mutex<HashMap<String, CandidateHealth>>>,
    /// 多轮前缀缓存（单会话语义，v1 只保留最近一个会话）
    prefix_cache: Arc<Mutex<Option<PrefixSnapshot>>>,
    /// 上次使用的候选签名（用于热更新链路时的变更日志）
    chain_signature: Arc<Mutex<Option<String>>>,
}

fn running_port() -> Option<u16> {
    AGGREGATE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|e| e.port))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AggregateStatus {
    pub running: bool,
    pub port: u16,
    pub candidate_count: usize,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AggregateLog {
    phase: String,
    line: String,
    level: String,
}

fn emit_aggregate_log(app: &tauri::AppHandle, phase: &str, level: &str, line: String) {
    let _ = app.emit(
        "aggregate-log",
        AggregateLog {
            phase: phase.to_string(),
            line,
            level: level.to_string(),
        },
    );
}

// ─── 上下文裁剪 ───

/// 是否为 CJK 字符（中日韩 + 全角标点），这类字符约 1 字 ≈ 1 token。
fn is_cjk(ch: char) -> bool {
    matches!(ch,
        '\u{4E00}'..='\u{9FFF}'
        | '\u{3400}'..='\u{4DBF}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{3000}'..='\u{303F}'
        | '\u{FF00}'..='\u{FFEF}'
    )
}

/// token 估算（启发式）：CJK 每字 1 token，其余约 4 字符 1 token。
pub fn estimate_tokens(text: &str) -> u64 {
    let mut cjk = 0u64;
    let mut other = 0u64;
    for ch in text.chars() {
        if is_cjk(ch) {
            cjk += 1;
        } else {
            other += 1;
        }
    }
    cjk + other / 4 + if other % 4 > 0 { 1 } else { 0 }
}

/// 消息文本估算（content 可能是字符串，也可能是分块数组）。
fn message_tokens(message: &Value) -> u64 {
    let content = message.get("content").cloned().unwrap_or(Value::Null);
    let text = match content {
        Value::String(s) => s,
        other => other.to_string(),
    };
    estimate_tokens(&text)
}

/// 纯函数：把 messages 裁剪到上下文预算内。
///
/// 规则：保留全部 `system` 消息与最后一条消息，从最早的非 system 消息开始丢弃，
/// 直到估算 token 数不超过 `limit`；返回 (裁剪后的 messages, 被丢弃的条数)。
pub fn trim_messages_to_limit(messages: Vec<Value>, limit: u64) -> (Vec<Value>, usize) {
    let total: u64 = messages.iter().map(message_tokens).sum();
    if total <= limit {
        return (messages, 0);
    }
    let mut kept: Vec<Value> = messages;
    let mut budget: u64 = kept.iter().map(message_tokens).sum();
    let mut trimmed = 0usize;
    while budget > limit {
        // 可丢弃位置：非 system 且**不是最后一条**（最后一条是当前提问，必须保留）
        let last = kept.len().saturating_sub(1);
        let Some(idx) = kept
            .iter()
            .position(|m| m.get("role").and_then(|r| r.as_str()) != Some("system"))
        else {
            break;
        };
        if idx == last || kept.len() <= 1 {
            break;
        }
        budget = budget.saturating_sub(message_tokens(&kept[idx]));
        kept.remove(idx);
        trimmed += 1;
    }
    (kept, trimmed)
}

// ─── 配置读写 ───

/// 读取聚合服务配置。
#[tauri::command]
pub fn get_aggregate_config() -> AggregateConfig {
    load_ai_config().aggregate
}

/// 保存聚合服务配置（运行中不允许改端口，需先停止）。
#[tauri::command]
pub fn save_aggregate_config(config: AggregateConfig) -> Result<AggregateConfig, String> {
    let mut sanitized = config.clone();
    sanitized.retry_count = sanitized.retry_count.clamp(1, 5);
    // 入口模型名：trim，留空回落默认 kiro-proxy
    sanitized.entry_model = entry_model(&sanitized);
    let mut current = load_ai_config();
    if running_port().is_some() && current.aggregate.port != sanitized.port {
        return Err("聚合服务运行中不能修改端口，请先停止服务".to_string());
    }
    current.aggregate = sanitized.clone();
    save_ai_config_to_file(&current)?;
    Ok(sanitized)
}

// ─── 启停 / 状态 ───

/// 启动聚合服务。链路支持热更新：启动后修改聚合链无需重启。
#[tauri::command]
pub async fn start_aggregate_service(app: tauri::AppHandle) -> Result<AggregateStatus, String> {
    let config = load_ai_config();
    let port = config.aggregate.port;
    if let Some(p) = running_port() {
        emit_aggregate_log(&app, "start", "warn", format!("聚合服务已在端口 {} 运行", p));
        return Ok(AggregateStatus { running: true, port: p, candidate_count: 0, detail: "已在运行".to_string() });
    }
    let chain = normalize_route_chain(&config.route_chain, &config.providers);
    let candidates = build_candidates(&config.providers, &chain, port);
    if let Some(owner) = crate::commands::utils::port_conflict_description(port) {
        return Err(format!("端口 {} 已被占用：{}，请更换端口", port, owner));
    }

    let state = AggState {
        app: app.clone(),
        port,
        health: Arc::new(Mutex::new(HashMap::new())),
        prefix_cache: Arc::new(Mutex::new(None)),
        chain_signature: Arc::new(Mutex::new(None)),
    };

    let router = Router::new()
        .route("/health", get(|| async { "ok" }))
        // 与协议转换代理的路由保持一致：同时挂「带 /v1」与「不带 /v1」两种前缀，
        // 否则把 baseUrl 配成不含 /v1 的工具（opencode 系自己补 /chat/completions）会 404。
        .route("/v1/chat/completions", post(chat_completions))
        .route("/chat/completions", post(chat_completions))
        .route("/v1/messages", post(messages))
        .route("/messages", post(messages))
        .route("/v1/messages/count_tokens", post(count_tokens))
        .route("/messages/count_tokens", post(count_tokens))
        .route("/v1/models", get(list_models))
        .route("/models", get(list_models))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|e| format!("监听 127.0.0.1:{} 失败: {}", port, e))?;

    let handle = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, router).await {
            eprintln!("[aggregate] 服务异常退出: {}", e);
        }
    });
    *AGGREGATE.get_or_init(|| Mutex::new(None)).lock().unwrap() =
        Some(AggregateEntry { port, handle });

    if candidates.is_empty() {
        emit_aggregate_log(
            &app,
            "start",
            "warn",
            "聚合链为空，服务已启动但请求会失败；请在聚合页勾选候选（支持热更新，无需重启）".to_string(),
        );
    } else {
        emit_aggregate_log(
            &app,
            "start",
            "info",
            format!(
                "聚合服务已启动：http://127.0.0.1:{}（对外模型 {}；{} 个候选；上下文上限 {} tokens）",
                port,
                entry_model(&config.aggregate),
                candidates.len(),
                config.aggregate.context_limit
            ),
        );
    }
    Ok(AggregateStatus {
        running: true,
        port,
        candidate_count: candidates.len(),
        detail: "已启动".to_string(),
    })
}

/// 停止聚合服务。
#[tauri::command]
pub fn stop_aggregate_service(app: tauri::AppHandle) -> Result<AggregateStatus, String> {
    let entry = AGGREGATE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| "聚合服务状态锁损坏".to_string())?
        .take();
    let Some(entry) = entry else {
        emit_aggregate_log(&app, "stop", "warn", "聚合服务未在运行".to_string());
        return Ok(AggregateStatus { running: false, port: 0, candidate_count: 0, detail: "未运行".to_string() });
    };
    entry.handle.abort();
    emit_aggregate_log(&app, "stop", "info", format!("聚合服务已停止（端口 {}）", entry.port));
    Ok(AggregateStatus { running: false, port: entry.port, candidate_count: 0, detail: "已停止".to_string() })
}

/// 聚合服务状态（含端口健康检查）。
#[tauri::command]
pub async fn get_aggregate_status() -> AggregateStatus {
    let Some(port) = running_port() else {
        return AggregateStatus { running: false, port: 0, candidate_count: 0, detail: "未运行".to_string() };
    };
    let alive = reqwest::get(format!("http://127.0.0.1:{}/health", port))
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false);
    AggregateStatus {
        running: alive,
        port,
        candidate_count: 0,
        detail: if alive { "运行中".to_string() } else { "服务无响应".to_string() },
    }
}

// ─── 处理函数 ───

/// 对外暴露的入口模型名：取配置（可配置，默认 kiro-proxy），空值回落默认。
pub(crate) fn entry_model(config: &AggregateConfig) -> String {
    let name = config.entry_model.trim();
    if name.is_empty() {
        AGGREGATE_MODEL_ID.to_string()
    } else {
        name.to_string()
    }
}

/// 对外只暴露一个模型（聚合 = 多个候选对内的单一入口），内部按链分发到具体模型。
async fn list_models() -> Json<Value> {
    let name = entry_model(&load_ai_config().aggregate);
    Json(serde_json::json!({
        "object": "list",
        "data": [{
            "id": name,
            "object": "model",
            "owned_by": "aggregate",
        }]
    }))
}

async fn chat_completions(State(state): State<AggState>, Json(body): Json<Value>) -> Response {
    forward(state, "openai", body).await
}

async fn messages(State(state): State<AggState>, Json(body): Json<Value>) -> Response {
    forward(state, "anthropic", body).await
}

/// `/v1/messages/count_tokens`：Anthropic 入站时的 token 估算（Claude Code 会先探测这个端点）。
/// 与协议转换代理同款估算（按 UTF-16 长度 / 4），够用即可——真正计费以真实上游 usage 为准。
async fn count_tokens(Json(body): Json<Value>) -> Json<Value> {
    let text_len = body
        .get("messages")
        .and_then(|m| m.as_array())
        .map(|arr| arr.iter().map(|m| m.to_string()).collect::<String>().len())
        .unwrap_or(0);
    Json(serde_json::json!({ "input_tokens": (text_len / 4).max(1) }))
}

/// 请求体中实际携带的 messages 数组（两种入口协议字段相同）。
fn messages_of(body: &Value) -> Vec<Value> {
    body.get("messages")
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_default()
}

/// 共享转发流程（OpenAI / Anthropic 入口通用）。
async fn forward(state: AggState, inbound: &'static str, mut body: Value) -> Response {
    // 每次请求实时读取配置：链路热更新、重试次数与压缩设置即改即生效
    let config = load_ai_config();
    let retry_count = config.aggregate.retry_count.clamp(1, 5) as usize;
    let context_limit = config.aggregate.context_limit;
    // 首字节 / 空闲超时：每次请求实时读取，改完配置即生效（与 retry_count 一致）
    let first_byte_timeout = config.aggregate.first_byte_timeout_secs;
    let idle_timeout = config.aggregate.idle_timeout_secs;
    let headroom = if config.headroom.enabled {
        Some(HeadroomRuntime {
            base_url: format!("http://127.0.0.1:{}", config.headroom.port),
            timeout_ms: config.headroom.timeout_ms.max(200),
            fail_closed: config.headroom.on_unavailable == "failClosed",
        })
    } else {
        None
    };

    // 链路热更新：实时解析候选，变更时打日志
    let chain = normalize_route_chain(&config.route_chain, &config.providers);
    let candidates = build_candidates(&config.providers, &chain, state.port);
    let signature = candidates
        .iter()
        .map(|c| format!("{}::{}::{}", c.provider_id, c.model_id, c.outbound.as_str()))
        .collect::<Vec<_>>()
        .join("|");
    {
        let mut seen = state.chain_signature.lock().unwrap_or_else(|_| panic!("poisoned"));
        if seen.as_deref() != Some(signature.as_str()) {
            if let Some(prev) = seen.as_ref() {
                emit_aggregate_log(
                    &state.app,
                    "route",
                    "info",
                    format!("聚合链已热更新：{} → {} 个候选", prev.split('|').count(), candidates.len()),
                );
            } else {
                emit_aggregate_log(
                    &state.app,
                    "route",
                    "info",
                    format!("聚合链加载完成：{} 个候选", candidates.len()),
                );
            }
            *seen = Some(signature);
        }
    }
    if candidates.is_empty() {
        emit_aggregate_log(&state.app, "route", "error", "聚合链为空（或候选缺少端点/API Key），请求失败".to_string());
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "error": { "message": "聚合链为空：请到 AI「聚合」页勾选候选（支持热更新，改完直接重试）", "type": "aggregate_chain_empty" }
            })),
        )
            .into_response();
    }

    // 多轮前缀缓存：同一会话复用上轮压缩结果，避免打爆 provider KV 缓存
    let messages = messages_of(&body);
    if !messages.is_empty() {
        let cache = state.prefix_cache.lock().ok().and_then(|g| g.clone());
        let (effective, frozen) = apply_frozen_prefix(&messages, cache.as_ref());
        if frozen > 0 {
            body["messages"] = Value::Array(effective.clone());
            body["config"] = serde_json::json!({ "frozen_message_count": frozen });
            emit_aggregate_log(
                &state.app,
                "context",
                "info",
                format!("命中前缀缓存：{} 条已冻结（省去重新压缩与缓存未命中）", frozen),
            );
        }
    }

    // 上下文限制：裁剪最早的非 system 消息
    if let Some(messages) = body.get("messages").and_then(|m| m.as_array()).cloned() {
        let (kept, trimmed) = trim_messages_to_limit(messages, context_limit);
        if trimmed > 0 {
            body["messages"] = Value::Array(kept);
            emit_aggregate_log(
                &state.app,
                "context",
                "warn",
                format!("上下文超限，已裁剪 {} 条最早的消息（上限 {} tokens）", trimmed, context_limit),
            );
        }
    }

    // Headroom 压缩旁路（成功后更新前缀缓存）
    let original_messages = messages_of(&body);
    if let Some(hr) = &headroom {
        match compress_body(hr, &body).await {
            Ok(Some(compressed)) => {
                emit_aggregate_log(&state.app, "compress", "info", "已压缩请求上下文".to_string());
                body = compressed;
                if let Ok(mut guard) = state.prefix_cache.lock() {
                    *guard = Some(PrefixSnapshot {
                        original: original_messages.clone(),
                        forwarded: messages_of(&body),
                    });
                }
            }
            Ok(None) => {}
            Err(err) => {
                if hr.fail_closed {
                    emit_aggregate_log(&state.app, "compress", "error", format!("压缩失败（策略：直接报错）: {}", err));
                    return (
                        StatusCode::BAD_GATEWAY,
                        Json(serde_json::json!({
                            "error": { "message": format!("Headroom 压缩服务不可用：{}", err), "type": "compression_unavailable" }
                        })),
                    )
                        .into_response();
                }
                emit_aggregate_log(
                    &state.app,
                    "compress",
                    "warn",
                    format!("压缩失败，跳过压缩继续发送: {}", err),
                );
            }
        }
    }

    let started = std::time::Instant::now();
    let stream_requested = body.get("stream").and_then(|s| s.as_bool()).unwrap_or(false);
    let mut last_error = String::new();

    // 「聚合挂代理上游」：按请求模型名优先路由（匹配不到按链序，入口名 = 无偏好）
    let entry_name = entry_model(&config.aggregate);
    let requested_model = body.get("model").and_then(|m| m.as_str()).unwrap_or_default().to_string();
    let candidates = prioritize_by_model(candidates, &requested_model, &entry_name);

    for (idx, candidate) in candidates.iter().enumerate() {
        // 运行期兜底：万一端口改动后候选变成自引用，直接跳过而不是递归打自己
        if is_self_referential(&candidate.base_url, state.port) {
            emit_aggregate_log(
                &state.app,
                "route",
                "error",
                format!(
                    "跳过自引用候选 {} / {}（指向聚合服务自身，会递归）",
                    candidate.provider_name, candidate.model_id
                ),
            );
            continue;
        }
        let key = format!("{}::{}", candidate.provider_id, candidate.model_id);
        if let Some(remaining) = cooldown_remaining_secs(&state.health, &key) {
            emit_aggregate_log(
                &state.app,
                "route",
                "warn",
                format!(
                    "跳过冷却中的候选 #{} {} / {}（剩余 {}s）",
                    idx + 1,
                    candidate.provider_name,
                    candidate.model_id,
                    remaining
                ),
            );
            continue;
        }

        // 整流配置按候选构造（端点/凭据/出站协议取当前候选，开关取全局）——
        // 与单工具协议代理共用同一套整流实现，避免两条路径行为漂移。
        let proxy_cfg = candidate_proxy_config(candidate, &config);
        let mut attempts = 0usize;
        // 同一候选最多做一次反应式整流重试（cc-switch 同款：每个 provider 的每种
        // 整流各只认领一次，避免上游持续报错时无限重试）
        let mut rectified = false;
        let mut rectified_body: Option<Value> = None;
        loop {
            attempts += 1;
            let result = try_candidate(
                &state,
                candidate,
                inbound,
                &body,
                stream_requested,
                rectified_body.as_ref(),
                &proxy_cfg,
                std::time::Duration::from_secs(first_byte_timeout.max(1)),
                std::time::Duration::from_secs(idle_timeout.max(1)),
            )
            .await;
            match result {
                Ok(response) => {
                    clear_health(&state.health, &key);
                    if idx > 0 || attempts > 1 {
                        emit_aggregate_log(
                            &state.app,
                            "route",
                            "warn",
                            format!(
                                "已切换到候选 #{} {} / {}（{} 协议，第 {} 次尝试）",
                                idx + 1,
                                candidate.provider_name,
                                candidate.model_id,
                                candidate.outbound.as_str(),
                                attempts
                            ),
                        );
                    }
                    emit_aggregate_log(
                        &state.app,
                        "route",
                        "info",
                        format!(
                            "候选 #{} {} / {} 响应成功（{} ms）",
                            idx + 1,
                            candidate.provider_name,
                            candidate.model_id,
                            started.elapsed().as_millis()
                        ),
                    );
                    return response;
                }
                Err(err) => {
                    last_error = format!("{:?}: {}", err.class, err.message);
                    // 策略类拒绝（内容安全等）：不重试、不切换，直接把错误回给客户端
                    if err.class == FailureClass::Fatal {
                        emit_aggregate_log(
                            &state.app,
                            "route",
                            "error",
                            format!(
                                "候选 #{} {} / {} 被策略拒绝，不再切换: {}",
                                idx + 1,
                                candidate.provider_name,
                                candidate.model_id,
                                err.message
                            ),
                        );
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(serde_json::json!({
                                "error": { "message": err.message, "type": "content_policy" }
                            })),
                        )
                            .into_response();
                    }
                    emit_aggregate_log(
                        &state.app,
                        "route",
                        "warn",
                        format!(
                            "候选 #{} {} / {} 第 {} 次失败 [{:?}]: {}",
                            idx + 1,
                            candidate.provider_name,
                            candidate.model_id,
                            attempts,
                            err.class,
                            err.message
                        ),
                    );
                    // 反应式整流（与单工具协议代理同一套实现）：上游报的是「能修的错」
                    // （thinking 签名 / budget、图片不支持、协议残留字段）就修正后重试一次；
                    // 修不了才往下走。这是抄 cc-switch forwarder 的「报错后整流重试」。
                    if !rectified {
                        if let Some(status) = err.status {
                            if let Some(fixed) = optimizers::try_reactive_rectify(
                                status,
                                &err.error_body,
                                &err.outbound_body,
                                &proxy_cfg,
                                candidate.outbound.as_str(),
                            ) {
                                rectified = true;
                                rectified_body = Some(fixed);
                                emit_aggregate_log(
                                    &state.app,
                                    "rectify",
                                    "info",
                                    format!(
                                        "候选 #{} {} / {} 上报 HTTP {}，已整流修正并重发一次",
                                        idx + 1,
                                        candidate.provider_name,
                                        candidate.model_id,
                                        status
                                    ),
                                );
                                continue;
                            }
                        }
                    }
                    // 换家判定（抄 cc-switch）：请求体层面被拒（400/422）换到哪一家都一样，
                    // 直接把上游错误回给客户端 —— 既不白跑整条链，也不误伤其它候选的冷却。
                    if !should_switch_candidate(err.class, err.status) {
                        emit_aggregate_log(
                            &state.app,
                            "route",
                            "error",
                            format!(
                                "候选 #{} {} / {} 的请求被上游拒绝且不属候选问题，不再切换: {}",
                                idx + 1,
                                candidate.provider_name,
                                candidate.model_id,
                                err.message
                            ),
                        );
                        let status_code = err
                            .status
                            .and_then(|s| StatusCode::from_u16(s).ok())
                            .unwrap_or(StatusCode::BAD_GATEWAY);
                        return (
                            status_code,
                            Json(serde_json::json!({
                                "error": {
                                    "message": format!(
                                        "候选 {} / {} 拒绝了该请求（整流后仍失败）: {}",
                                        candidate.provider_name, candidate.model_id, err.message
                                    ),
                                    "type": "upstream_rejected_request"
                                }
                            })),
                        )
                            .into_response();
                    }
                    // 达到该类别的重试预算 → 记冷却并切换到下一个候选
                    if attempts >= retry_budget(err.class).min(retry_count.max(1)) {
                        let cooldown = record_failure(&state.health, &key, err.class, err.retry_after);
                        emit_aggregate_log(
                            &state.app,
                            "route",
                            "warn",
                            format!(
                                "候选 #{} {} / {} 进入冷却 {}s，切换到下一个候选",
                                idx + 1,
                                candidate.provider_name,
                                candidate.model_id,
                                cooldown.as_secs()
                            ),
                        );
                        break;
                    }
                }
            }
        }
    }

    emit_aggregate_log(
        &state.app,
        "route",
        "error",
        format!("聚合链全部候选失败（{} 个）: {}", candidates.len(), last_error),
    );
    (
        StatusCode::BAD_GATEWAY,
        Json(serde_json::json!({
            "error": {
                "message": format!(
                    "聚合链 {} 个候选全部失败，最后错误：{}。请检查候选的 API Key / 额度，或到「服务」页查看聚合日志",
                    candidates.len(), last_error
                ),
                "type": "aggregate_chain_exhausted"
            }
        })),
    )
        .into_response()
}

/// 从上游响应 JSON 提取 usage（按入口协议的形态），落账到 ai_usage。
fn record_usage(inbound: &str, candidate: &AggCandidate, resp: &Value, elapsed_ms: u128) {
    let usage = resp.get("usage").cloned().unwrap_or(Value::Null);
    let (input, output) = match inbound {
        "anthropic" => (
            usage.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
            usage.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
        ),
        _ => (
            usage.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
            usage.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
        ),
    };
    let (cache_read, cache_write) = super::usage::cache_tokens_from_json(&usage);
    if input == 0 && output == 0 && cache_read == 0 {
        return;
    }
    let _ = log_usage_entry(
        &UsageEntry::success("aggregate", &candidate.model_id, Some(&candidate.provider_id))
            .tokens(input, output)
            .cache(cache_read, cache_write)
            .timing(elapsed_ms as u64, 0),
    );
}

/// 按入口/出站协议对响应做转换（跨协议时），并提取 usage 落账。
fn finish_non_stream(
    inbound: &'static str,
    candidate: &AggCandidate,
    upstream: Value,
    request_model: &str,
    elapsed_ms: u128,
) -> Response {
    let converted = if inbound == candidate.outbound.as_str() {
        upstream
    } else {
        crate::proxy::convert::convert_response(
            candidate.outbound.as_str(),
            inbound,
            &upstream,
            request_model,
        )
    };
    record_usage(inbound, candidate, &converted, elapsed_ms);
    (StatusCode::OK, Json(converted)).into_response()
}

/// 同协议转发时的模型名改写（纯函数，便于单测）。
///
/// 聚合入口对外只暴露 `kiro-proxy`（AGGREGATE_MODEL_ID）；直接 `body.clone()`
/// 会把这个入口模型名原样透传给上游 → 上游报「模型不存在」。
/// 必须改写为候选配置的 `model_id` 再转发。跨协议路径由 transform 函数完成同样改写。
fn rewrite_upstream_model(mut body: Value, model: &str) -> Value {
    body["model"] = Value::String(model.to_string());
    body
}

/// 单候选请求：成功返回响应；失败返回分类错误。
/// 空闲超时包装：流式过程中两个 chunk 之间超过 `idle` 没有数据就终止流。
///
/// 抄 cc-switch `AppProxyConfig.streamingIdleTimeout`。注意它**只能终止当前流**，
/// 不能换候选 —— 首字节之后上游已经开始产出，换一家会把这段输出重复计费。
/// 流被终止时以 `io::Error` 收尾，axum 会把它当成连接错误传给客户端，
/// 客户端至少能立刻报错而不是无限挂住。
fn idle_timeout_stream<S>(
    inner: S,
    idle: std::time::Duration,
) -> impl futures_util::Stream<Item = Result<bytes::Bytes, std::io::Error>>
where
    S: futures_util::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send + 'static,
{
    async_stream::stream! {
        // Pin 到堆上：reqwest 的 `bytes_stream()` 只承诺 `impl Stream`，不承诺 Unpin，
        // 而 `StreamExt::next` 需要 Unpin。`Pin<Box<S>>` 本身是 Unpin。
        let mut inner = Box::pin(inner);
        loop {
            match tokio::time::timeout(idle, futures_util::StreamExt::next(&mut inner)).await {
                Ok(Some(Ok(chunk))) => yield Ok(chunk),
                Ok(Some(Err(e))) => {
                    yield Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()));
                    break;
                }
                Ok(None) => break,
                Err(_) => {
                    yield Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        format!("上游流式响应空闲超时（{}s 无数据）", idle.as_secs()),
                    ));
                    break;
                }
            }
        }
    }
}

async fn try_candidate(
    state: &AggState,
    candidate: &AggCandidate,
    inbound: &'static str,
    body: &Value,
    stream_requested: bool,
    // 反应式整流修正后的 P_out 请求体（Some = 本次直接发它，不再跑优化器）
    outbound_override: Option<&Value>,
    // 整流用的统一代理配置（由 [`candidate_proxy_config`] 构造）
    proxy_cfg: &ProxyConfig,
    // 首字节超时：连上上游后迟迟不吐字节就判失败，让故障转移真正生效
    first_byte_timeout: std::time::Duration,
    idle_timeout: std::time::Duration,
) -> Result<Response, CandidateError> {
    let model = &candidate.model_id;
    let outbound = candidate.outbound.as_str();
    // 同协议只改模型名（入口是 kiro-proxy，上游只认自己配置的模型名）；跨协议先按非流式
    // 整体转换（流式转换仅覆盖 a↔o 主流组合）。转换分发表统一走 proxy::convert，与代理一致。
    let cross = inbound != outbound;
    let upstream_body = match outbound_override {
        Some(fixed) => fixed.clone(),
        None => {
            let mut built = if !cross {
                rewrite_upstream_model(body.clone(), model)
            } else {
                let mut b = body.clone();
                if stream_requested {
                    b["stream"] = Value::Bool(false);
                }
                crate::proxy::convert::convert_request(inbound, outbound, &b, model)
            };
            // 与单工具代理同一套整流：出站前跑角色归一化 + 优化器 + 预防式整流。
            // 此前聚合完全不走这几步，导致同一份配置在「代理」与「聚合」两条路径上
            // 协议行为不一致（thinking 参数、图片降级、cache 断点全都只在一边生效）。
            if outbound == "openai" {
                crate::proxy::convert::normalize_openai_message_roles(&mut built);
                crate::proxy::convert::collapse_system_messages_to_head(&mut built);
                crate::proxy::convert::normalize_image_data_urls_for_model(&mut built, model);
            }
            optimizers::apply_optimizers(&mut built, outbound, proxy_cfg);
            optimizers::apply_preventive_rectifiers(&mut built, outbound, proxy_cfg);
            built
        }
    };
    let upstream_stream = !cross && stream_requested;

    let (url, auth_name) = build_candidate_url(
        candidate.outbound,
        &candidate.base_url,
        model,
        upstream_stream,
        candidate.include_v1,
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| CandidateError {
            class: FailureClass::Transient,
            message: format!("HTTP 客户端构建失败: {}", e),
            retry_after: None,
            status: None,
            error_body: String::new(),
            outbound_body: upstream_body.clone(),
        })?;
    // 鉴权 + 自定义头统一走 proxy::upstream（与协议转换代理一致：anthropic 双头、显式头优先）
    let req = crate::proxy::upstream::inject_auth(
        client.post(&url),
        auth_name,
        &candidate.api_key,
        &candidate.headers,
    );
    // 首字节超时（抄 cc-switch `streamingFirstByteTimeout`）：`send()` 返回即响应头到达，
    // 也就是"第一个字节"。此前只有 300s 的整体超时，上游「连得上但挂住不响应」会把整条
    // 聚合链堵死 300s —— 故障转移在此期间完全不起作用。超时按瞬时失败处理，
    // 由调用方决定重试还是切下一个候选。
    let send_result = tokio::time::timeout(first_byte_timeout, req.json(&upstream_body).send()).await;
    let resp = match send_result {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            let message = if e.is_connect() {
                format!("连接被拒绝（{}）", url)
            } else if e.is_timeout() {
                format!("请求超时（{}s）", std::time::Duration::from_secs(300).as_secs())
            } else {
                format!("请求失败: {}", e)
            };
            return Err(CandidateError {
                class: FailureClass::Transient,
                message,
                retry_after: None,
                status: None,
                error_body: String::new(),
                outbound_body: upstream_body.clone(),
            });
        }
        Err(_) => {
            return Err(CandidateError {
                class: FailureClass::Transient,
                message: format!("首字节超时（{}s 内上游未响应，{}）", first_byte_timeout.as_secs(), url),
                retry_after: None,
                status: None,
                error_body: String::new(),
                outbound_body: upstream_body.clone(),
            });
        }
    };
    let status = resp.status();
    let retry_after = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok());
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        let class = classify_failure(Some(status.as_u16()), &text);
        // 失败落账：按模型成功率的分母靠它（参考 ai-toolbox per-model success rate）。
        // 这里记的是「候选级失败」——同一请求换到下一个候选成功时，成功那条也会落账，
        // 因此按模型看是"该模型失败过几次"，这正是排查坏供应商需要的口径。
        let _ = log_usage_failure(
            "aggregate",
            &candidate.model_id,
            Some(&candidate.provider_id),
            class.as_str(),
        );
        return Err(CandidateError {
            class,
            message: format!("HTTP {} {}", status.as_u16(), trim_error(&text)),
            retry_after,
            status: Some(status.as_u16()),
            error_body: text,
            outbound_body: upstream_body.clone(),
        });
    }

    let started = std::time::Instant::now();

    // 同协议 + 流式：字节流透传（首字节后不再切换候选，避免重复计费）
    if upstream_stream && inbound == candidate.outbound.as_str() {
        let stream = idle_timeout_stream(resp.bytes_stream(), idle_timeout);
        let body_stream = axum::body::Body::from_stream(stream);
        let response = Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "text/event-stream")
            .body(body_stream)
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
        return Ok(response);
    }

    // 跨协议 + 流式：a↔o 用流式转换器；google 组合降级为非流式整体转换（记日志说明）
    if upstream_stream && inbound != candidate.outbound.as_str() {
        let pair_supported = matches!(
            (inbound, candidate.outbound),
            ("anthropic", Outbound::OpenAi) | ("openai", Outbound::Anthropic)
        );
        if pair_supported {
            return stream_cross_protocol(state, candidate, inbound, resp, started, idle_timeout);
        }
        emit_aggregate_log(
            &state.app,
            "route",
            "warn",
            format!(
                "候选 {} / {} 为 google 出站，跨协议流式暂不支持，已降级为非流式整体转换",
                candidate.provider_name, candidate.model_id
            ),
        );
    }

    // 非流式：整体转换 + 落账
    let text = resp.text().await.map_err(|e| CandidateError {
        class: FailureClass::Transient,
        message: format!("读取响应失败: {}", e),
        retry_after: None,
        status: Some(status.as_u16()),
        error_body: String::new(),
        outbound_body: upstream_body.clone(),
    })?;
    let upstream_json: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    Ok(finish_non_stream(inbound, candidate, upstream_json, model, started.elapsed().as_millis()))
}

/// a↔o 跨协议流式转换器的统一包装（两个转换器类型不同，但方法签名一致）。
enum CrossStreamConverter {
    AnthropicToOpenai(crate::proxy::transform::StreamConverter),
    OpenaiToAnthropic(crate::proxy::transform::AnthropicToOpenaiStreamConverter),
}

impl CrossStreamConverter {
    fn convert_chunk(&mut self, chunk: &Value) -> Vec<String> {
        match self {
            Self::AnthropicToOpenai(c) => c.convert_chunk(chunk),
            Self::OpenaiToAnthropic(c) => c.convert_chunk(chunk),
        }
    }
    fn usage(&self) -> (u64, u64) {
        match self {
            Self::AnthropicToOpenai(c) => c.usage(),
            Self::OpenaiToAnthropic(c) => c.usage(),
        }
    }
}

/// 跨协议流式转发（仅 a↔o）：用流式转换器把上游 chunk 转成入口协议的 SSE 事件。
fn stream_cross_protocol(
    state: &AggState,
    candidate: &AggCandidate,
    inbound: &'static str,
    resp: reqwest::Response,
    started: std::time::Instant,
    idle_timeout: std::time::Duration,
) -> Result<Response, CandidateError> {
    use axum::response::sse::{Event, Sse};
    use futures_util::StreamExt;

    let model = candidate.model_id.clone();
    let provider_id = candidate.provider_id.clone();
    let outbound = candidate.outbound;
    let stream = resp.bytes_stream();
    // 生成出的流必须 'static，不能借用 `state`，先把 AppHandle 克隆进去
    let app = state.app.clone();
    let sse = async_stream::stream! {
        let mut conv = match (inbound, outbound) {
            ("anthropic", Outbound::OpenAi) => Some(CrossStreamConverter::AnthropicToOpenai(
                crate::proxy::transform::StreamConverter::new(model.clone()),
            )),
            ("openai", Outbound::Anthropic) => Some(CrossStreamConverter::OpenaiToAnthropic(
                crate::proxy::transform::AnthropicToOpenaiStreamConverter::new(model.clone()),
            )),
            _ => None,
        };
        let Some(conv) = conv.as_mut() else {
            yield Ok::<_, std::convert::Infallible>(Event::default().data("{\"error\":\"unsupported stream pair\"}"));
            return;
        };
        let mut buffer = String::new();
        tokio::pin!(stream);
        // 空闲超时：两个 chunk 之间超过 `idle_timeout` 无数据就终止流（首字节之后不能换
        // 候选 —— 上游已在产出，换一家会重复计费，只能把错误传给客户端让它立刻失败）
        loop {
            let next = tokio::time::timeout(idle_timeout, stream.next()).await;
            let r = match next {
                Ok(Some(r)) => r,
                Ok(None) => break,
                Err(_) => {
                    let msg = format!("上游流式响应空闲超时（{}s 无数据）", idle_timeout.as_secs());
                    emit_aggregate_log(&app, "route", "warn", format!(
                        "候选 {} / {} 流式空闲超时，已终止该流: {msg}", provider_id, model
                    ));
                    yield Ok::<_, std::convert::Infallible>(Event::default().data(format!("{{\"error\":\"{msg}\"}}")));
                    break;
                }
            };
            let chunk = match r {
                Ok(c) => c,
                Err(e) => {
                    yield Ok::<_, std::convert::Infallible>(Event::default().data(format!("{{\"error\":\"{}\"}}", e)));
                    break;
                }
            };
            buffer = crate::proxy::sse::append_utf8_safe(&buffer, &chunk);
            while let Some((block, remainder)) = crate::proxy::sse::take_sse_block(&buffer) {
                buffer = remainder.to_string();
                let data_str = match crate::proxy::sse::extract_sse_data(&block) {
                    Some(d) => d,
                    None => continue,
                };
                if data_str == "[DONE]" {
                    yield Ok::<_, std::convert::Infallible>(Event::default().data("[DONE]"));
                    continue;
                }
                if let Ok(cj) = serde_json::from_str::<Value>(&data_str) {
                    for ev in conv.convert_chunk(&cj) {
                        let mut event = Event::default();
                        for line in ev.lines() {
                            if let Some(rest) = line.strip_prefix("event:") {
                                event = event.event(rest.trim());
                            } else if let Some(rest) = line.strip_prefix("data:") {
                                event = event.data(rest.trim());
                            }
                        }
                        yield Ok::<_, std::convert::Infallible>(event);
                    }
                }
            }
        }
        // 跨协议流式的 usage 从转换器聚合而来，按入口协议形态落账
        let (in_t, out_t) = conv.usage();
        if in_t > 0 || out_t > 0 {
            let _ = log_usage_db_timed(
                "aggregate",
                &model,
                Some(&provider_id),
                in_t,
                out_t,
                started.elapsed().as_millis() as u64,
                0,
            );
        }
    };
    Ok(Sse::new(sse).into_response())
}

/// 上游错误体压成一行（用于日志与 `CandidateError.message`）。
/// 截断走 [`crate::commands::utils::truncate_utf8`]：上游错误文案常带中文，
/// `&cleaned[..200]` 会正好切在多字节字符中间而 panic。
fn trim_error(text: &str) -> String {
    let cleaned = text.trim().replace('\n', " ");
    let truncated = crate::commands::utils::truncate_utf8(&cleaned, 200);
    if truncated.len() == cleaned.len() {
        cleaned
    } else {
        format!("{}…", truncated)
    }
}

/// 压缩旁路：成功返回 Some(压缩后的 body)；未启用/不可达返回 None 或 Err。
async fn compress_body(hr: &HeadroomRuntime, body: &Value) -> Result<Option<Value>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(hr.timeout_ms))
        .build()
        .map_err(|e| format!("HTTP 客户端构建失败: {}", e))?;
    let url = format!("{}/v1/compress", hr.base_url);
    let resp = client
        .post(&url)
        .json(body)
        .send()
        .await
        .map_err(|e| if e.is_connect() {
            format!("连接被拒绝——Headroom 服务未启动（{}）", hr.base_url)
        } else if e.is_timeout() {
            "压缩超时".to_string()
        } else {
            format!("请求失败: {}", e)
        })?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status().as_u16()));
    }
    let value: Value = resp.json().await.map_err(|e| format!("解析压缩响应失败: {}", e))?;
    // 超时等场景 headroom 会 fail-open：原样返回并带 compression_skipped
    if value.get("compression_skipped").and_then(|v| v.as_bool()).unwrap_or(false) {
        return Ok(None);
    }
    let messages = value.get("messages").cloned();
    match messages {
        Some(msgs) => {
            let mut next = body.clone();
            next["messages"] = msgs;
            Ok(Some(next))
        }
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn msg(role: &str, text: &str) -> Value {
        json!({ "role": role, "content": text })
    }

    fn cand(base: &str) -> AggCandidate {
        AggCandidate {
            provider_id: "p".into(),
            provider_name: "P".into(),
            outbound: Outbound::OpenAi,
            base_url: base.into(),
            api_key: "sk".into(),
            model_id: "m".into(),
            include_v1: None,
            headers: vec![],
            supports_image: None,
        }
    }

    /// 换家判定：候选自身的问题换家，请求体层面的拒绝不换家。
    #[test]
    fn switch_only_when_another_candidate_could_succeed() {
        use FailureClass as F;
        // 候选自身的问题 → 换家
        for status in [500, 502, 503, 401, 403, 402, 404, 429, 408] {
            assert!(
                should_switch_candidate(classify_failure(Some(status), ""), Some(status)),
                "HTTP {status} 应换家"
            );
        }
        // 连不上 / 超时（无状态码）→ 换家
        assert!(should_switch_candidate(F::Transient, None));
        // 请求体层面的拒绝 → 不换家（换到哪一家都一样）
        for status in [400, 422] {
            assert!(
                !should_switch_candidate(classify_failure(Some(status), ""), Some(status)),
                "HTTP {status} 不该换家"
            );
        }
        // 内容策略拒绝 → 不换家
        assert!(!should_switch_candidate(F::Fatal, Some(400)));
    }

    #[test]
    fn prioritize_by_model_moves_match_to_front_and_keeps_chain_order() {
        use super::{Outbound, AGGREGATE_MODEL_ID};
        let mk = |provider: &str, model: &str| AggCandidate {
            provider_id: provider.into(),
            provider_name: provider.to_uppercase(),
            outbound: Outbound::OpenAi,
            base_url: "https://x".into(),
            api_key: "k".into(),
            model_id: model.into(),
            include_v1: None,
            headers: vec![],
            supports_image: None,
        };
        let chain = vec![mk("a", "m1"), mk("b", "m2"), mk("c", "m1")];
        // 命中 m2 → 提到最前，其余保持原相对顺序
        let ordered = prioritize_by_model(chain.clone(), "m2", AGGREGATE_MODEL_ID);
        assert_eq!(
            ordered.iter().map(|c| c.model_id.as_str()).collect::<Vec<_>>(),
            vec!["m2", "m1", "m1"]
        );
        // 入口名 / 空名 = 无偏好，保持链序
        assert_eq!(prioritize_by_model(chain.clone(), AGGREGATE_MODEL_ID, AGGREGATE_MODEL_ID)[0].provider_id, "a");
        assert_eq!(prioritize_by_model(chain.clone(), "", AGGREGATE_MODEL_ID)[0].provider_id, "a");
        // 无匹配 → 原顺序
        assert_eq!(prioritize_by_model(chain, "m9", AGGREGATE_MODEL_ID)[0].provider_id, "a");
    }

    #[test]
    fn test_estimate_tokens_mixed() {
        assert!(estimate_tokens("") <= 1);
        // 英文约 4 字符 1 token
        assert!(estimate_tokens("a".repeat(100).as_str()) >= 20);
        // 中文约 1 字 1 token
        assert_eq!(estimate_tokens("你好世界"), 4);
        // 2 个汉字 + 3 个英文字符 → 2 + 1 = 3
        assert_eq!(estimate_tokens("你好abc"), 3);
    }

    #[test]
    fn test_trim_keeps_system_and_last() {
        let messages = vec![
            msg("system", "你是助手"),
            msg("user", "很早的问题"),
            msg("assistant", "很早的回答"),
            msg("user", "最新问题"),
        ];
        let (kept, trimmed) = trim_messages_to_limit(messages, 4);
        // 丢掉两条中间的历史，system 与当前这条 user 提问必须保留
        assert_eq!(trimmed, 2);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept.first().unwrap().get("role").unwrap(), "system");
        assert_eq!(kept.last().unwrap().get("content").unwrap(), "最新问题");
    }

    #[test]
    fn test_trim_noop_when_under_limit() {
        let messages = vec![msg("system", "s"), msg("user", "u")];
        let (kept, trimmed) = trim_messages_to_limit(messages.clone(), 100_000);
        assert_eq!(trimmed, 0);
        assert_eq!(kept.len(), messages.len());
    }

    #[test]
    fn test_trim_stop_when_only_system_left() {
        // 只剩 system 时即使超限也不再丢（丢光了没法回答）
        let messages = vec![msg("system", &"很长的系统提示词".repeat(50))];
        let (kept, trimmed) = trim_messages_to_limit(messages, 10);
        assert_eq!(trimmed, 0);
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn test_is_retryable_by_class() {
        assert!(is_retryable(FailureClass::Transient));
        assert!(!is_retryable(FailureClass::Authentication));
        assert!(!is_retryable(FailureClass::Billing));
        assert!(!is_retryable(FailureClass::RateLimit));
        assert!(!is_retryable(FailureClass::ModelUnavailable));
        assert!(!is_retryable(FailureClass::Fatal));
    }

    #[test]
    fn test_classify_failure_by_status() {
        assert_eq!(classify_failure(Some(401), ""), FailureClass::Authentication);
        assert_eq!(classify_failure(Some(403), ""), FailureClass::Authentication);
        assert_eq!(classify_failure(Some(402), ""), FailureClass::Billing);
        assert_eq!(classify_failure(Some(404), ""), FailureClass::ModelUnavailable);
        assert_eq!(classify_failure(Some(429), ""), FailureClass::RateLimit);
        assert_eq!(classify_failure(Some(500), ""), FailureClass::Transient);
        assert_eq!(classify_failure(Some(503), ""), FailureClass::Transient);
        assert_eq!(classify_failure(None, "connection refused"), FailureClass::Transient);
    }

    #[test]
    fn test_classify_failure_prefers_body_hints() {
        assert_eq!(
            classify_failure(Some(429), "error: insufficient_quota"),
            FailureClass::Billing
        );
        assert_eq!(
            classify_failure(Some(400), "insufficient quota for this key"),
            FailureClass::Billing
        );
        assert_eq!(classify_failure(Some(400), "rate limit reached"), FailureClass::RateLimit);
        assert_eq!(
            classify_failure(Some(400), "blocked by safety policy"),
            FailureClass::Fatal
        );
        assert_eq!(
            classify_failure(Some(400), "model_not_found: gpt-9"),
            FailureClass::ModelUnavailable
        );
    }

    #[test]
    fn test_retry_budget_only_for_transient() {
        assert_eq!(retry_budget(FailureClass::Transient), 2);
        assert_eq!(retry_budget(FailureClass::Billing), 1);
        assert_eq!(retry_budget(FailureClass::RateLimit), 1);
        assert_eq!(retry_budget(FailureClass::Authentication), 1);
        assert_eq!(retry_budget(FailureClass::ModelUnavailable), 1);
    }

    #[test]
    fn test_cooldown_table() {
        assert_eq!(cooldown_for(FailureClass::Transient, None, 1).as_secs(), 30);
        assert_eq!(cooldown_for(FailureClass::Transient, None, 2).as_secs(), 60);
        assert_eq!(cooldown_for(FailureClass::Transient, None, 3).as_secs(), 120);
        assert_eq!(cooldown_for(FailureClass::Transient, None, 4).as_secs(), 240);
        assert_eq!(cooldown_for(FailureClass::Transient, None, 5).as_secs(), 300);
        assert_eq!(cooldown_for(FailureClass::Transient, None, 99).as_secs(), 300);
        assert_eq!(cooldown_for(FailureClass::RateLimit, None, 1).as_secs(), 60);
        assert_eq!(cooldown_for(FailureClass::RateLimit, Some(120), 1).as_secs(), 120);
        assert_eq!(
            cooldown_for(FailureClass::RateLimit, Some(999_999), 1).as_secs(),
            24 * 3600
        );
        assert_eq!(cooldown_for(FailureClass::Authentication, None, 1).as_secs(), 3600);
        assert_eq!(cooldown_for(FailureClass::Billing, None, 1).as_secs(), 3600);
        assert_eq!(
            cooldown_for(FailureClass::ModelUnavailable, None, 1).as_secs(),
            6 * 3600
        );
    }

    #[test]
    fn test_health_record_and_clear() {
        let health: Arc<Mutex<HashMap<String, CandidateHealth>>> = Arc::new(Mutex::new(HashMap::new()));
        let key = "p::m";
        assert!(cooldown_remaining_secs(&health, key).is_none());
        let d = record_failure(&health, key, FailureClass::RateLimit, Some(90));
        assert_eq!(d.as_secs(), 90);
        let remaining = cooldown_remaining_secs(&health, key).unwrap_or(0);
        assert!(remaining > 0 && remaining <= 90, "剩余冷却异常: {}", remaining);
        clear_health(&health, key);
        assert!(cooldown_remaining_secs(&health, key).is_none());
    }

    #[test]
    fn test_is_self_referential_detects_own_service() {
        assert!(is_self_referential("http://127.0.0.1:15888/v1", 15888));
        assert!(is_self_referential("http://localhost:15888/v1", 15888));
        assert!(is_self_referential("http://127.0.0.1:15888", 15888));
    }

    #[test]
    fn test_is_self_referential_allows_others() {
        assert!(!is_self_referential("https://api.openai.com/v1", 15888));
        assert!(!is_self_referential("http://127.0.0.1:11434/v1", 15888));
        assert!(!is_self_referential("http://127.0.0.1:15889/v1", 15888));
        assert!(!is_self_referential("", 15888));
        assert!(!is_self_referential("http://127.0.0.1/v1", 15888));
    }

    #[test]
    fn test_filter_self_referential_removes_only_self() {
        let list = vec![
            cand("http://127.0.0.1:15888/v1"),
            cand("https://api.openai.com/v1"),
            cand("http://127.0.0.1:11434/v1"),
        ];
        let (kept, removed) = filter_self_referential(list, 15888);
        assert_eq!(removed, 1);
        assert_eq!(kept.len(), 2);
        assert!(kept.iter().all(|c| !is_self_referential(&c.base_url, 15888)));
    }

    #[test]
    fn test_aggregate_exposes_single_model() {
        assert_eq!(AGGREGATE_MODEL_ID, "kiro-proxy");
    }

    #[test]
    fn test_entry_model_trim_and_fallback() {
        let mut cfg = AggregateConfig::default();
        cfg.entry_model = "  my-model  ".to_string();
        assert_eq!(entry_model(&cfg), "my-model");
        cfg.entry_model = "   ".to_string();
        assert_eq!(entry_model(&cfg), AGGREGATE_MODEL_ID, "留空回落默认");
        assert_eq!(cfg.entry_model.trim().is_empty(), true);
    }

    #[test]
    fn test_same_protocol_rewrite_replaces_entry_model() {
        // 回归：同协议转发必须把入口模型名（kiro-proxy）改写为候选的 model_id
        let body = serde_json::json!({
            "model": "kiro-proxy",
            "messages": [{"role": "user", "content": "hi"}]
        });
        let out = rewrite_upstream_model(body.clone(), "claude-sonnet-4");
        assert_eq!(out["model"], "claude-sonnet-4");
        assert_eq!(out["messages"], body["messages"], "其它字段不动");
        // 非 kiro-proxy 的入口模型同样被改写（规则统一，不特判名字）
        let out2 = rewrite_upstream_model(body, "gpt-4o");
        assert_eq!(out2["model"], "gpt-4o");
    }

    #[test]
    fn test_build_candidate_url_avoids_double_v1() {
        use Outbound::*;
        let (url, auth) = build_candidate_url(OpenAi, "https://api.x.com/v1", "m", false, None);
        assert_eq!(url, "https://api.x.com/v1/chat/completions");
        assert_eq!(auth, "Authorization");
        let (url, auth) = build_candidate_url(Anthropic, "https://api.anthropic.com", "m", false, None);
        assert_eq!(url, "https://api.anthropic.com/v1/messages");
        assert_eq!(auth, "x-api-key");
        let (url, auth) = build_candidate_url(Anthropic, "https://x.com/anthropic/v1", "m", false, None);
        assert_eq!(url, "https://x.com/anthropic/v1/messages");
        let (url, auth) = build_candidate_url(Google, "https://g.com", "m", true, None);
        assert_eq!(url, "https://g.com/v1beta/models/m:streamGenerateContent?alt=sse");
        assert_eq!(auth, "x-goog-api-key");
    }

    /// 「是否包含 /v1」开关：自动之外的两种显式取值都必须被尊重。
    /// 场景：某些兼容层把 `/v1` 写进网关路径（要关掉），某些则完全没有版本号（要打开）。
    #[test]
    fn test_build_candidate_url_honors_include_v1_switch() {
        use Outbound::*;
        // 自动（None）：URL 结尾没有 /v1 → 补；有 → 不补
        assert_eq!(
            build_candidate_url(OpenAi, "https://x.com/api", "m", false, None).0,
            "https://x.com/api/v1/chat/completions"
        );
        assert_eq!(
            build_candidate_url(OpenAi, "https://x.com/api/v1", "m", false, None).0,
            "https://x.com/api/v1/chat/completions"
        );
        // 显式「包含」：即使 URL 里已经写了 /v1 也不能拼出 /v1/v1
        assert_eq!(
            build_candidate_url(OpenAi, "https://x.com/api", "m", false, Some(true)).0,
            "https://x.com/api/v1/chat/completions"
        );
        assert_eq!(
            build_candidate_url(OpenAi, "https://x.com/api/v1", "m", false, Some(true)).0,
            "https://x.com/api/v1/chat/completions"
        );
        // 显式「不包含」：直接用 base（网关自带版本段 / 或根本不带版本号）
        assert_eq!(
            build_candidate_url(OpenAi, "https://x.com/api/v9", "m", false, Some(false)).0,
            "https://x.com/api/v9/chat/completions"
        );
        // Anthropic 同样受控
        assert_eq!(
            build_candidate_url(Anthropic, "https://x.com/anthropic", "m", false, Some(false)).0,
            "https://x.com/anthropic/messages"
        );
        assert_eq!(
            build_candidate_url(Anthropic, "https://x.com/anthropic", "m", false, Some(true)).0,
            "https://x.com/anthropic/v1/messages"
        );
    }

    #[test]
    fn test_pick_outbound_priority_openai_first() {
        let mut provider = AiProvider {
            id: "p".into(),
            name: "P".into(),
            category: "provider".into(),
            api_key: "sk".into(),
            website: String::new(),
            openai_url: "https://x/v1".into(),
            anthropic_url: "https://x/anthropic".into(),
            google_url: "https://g".into(),
            models: vec![],
            active_model_id: None,
            custom_headers: Vec::new(),
            openai_include_v1: None,
            anthropic_include_v1: None,
            promotions: Vec::new(),
        };
        let (outbound, base, _) = pick_outbound(&provider).unwrap();
        assert_eq!(outbound, Outbound::OpenAi);
        assert_eq!(base, "https://x/v1");
        // 只有 anthropic → 用 anthropic
        provider.openai_url = String::new();
        let (outbound, base, _) = pick_outbound(&provider).unwrap();
        assert_eq!(outbound, Outbound::Anthropic);
        assert_eq!(base, "https://x/anthropic");
        // 全空 → None
        provider.anthropic_url = String::new();
        provider.google_url = String::new();
        assert!(pick_outbound(&provider).is_none());
    }

    #[test]
    fn test_apply_frozen_prefix_reuses_same_session() {
        let original = vec![msg("system", "sys"), msg("user", "q1")];
        let forwarded = vec![msg("system", "sys"), msg("user", "q1(压缩)")];
        let cache = PrefixSnapshot { original: original.clone(), forwarded: forwarded.clone() };
        // 下一轮：原始前缀 + 新消息 → 复用压缩前缀
        let next: Vec<Value> = vec![
            msg("system", "sys"),
            msg("user", "q1"),
            msg("assistant", "a1"),
            msg("user", "q2"),
        ];
        let (out, frozen) = apply_frozen_prefix(&next, Some(&cache));
        assert_eq!(frozen, forwarded.len());
        assert_eq!(out.len(), forwarded.len() + 2);
        assert_eq!(out[..forwarded.len()], forwarded[..]);
    }

    #[test]
    fn test_apply_frozen_prefix_miss_on_new_session() {
        let cache = PrefixSnapshot {
            original: vec![msg("system", "sys"), msg("user", "q1")],
            forwarded: vec![msg("system", "sys"), msg("user", "q1c")],
        };
        // 前缀不同（新会话）→ 原样返回
        let next = vec![msg("system", "另一个会话"), msg("user", "q")];
        let (out, frozen) = apply_frozen_prefix(&next, Some(&cache));
        assert_eq!(frozen, 0);
        assert_eq!(out, next);
        // 无缓存 → 原样返回
        let (out, frozen) = apply_frozen_prefix(&next, None);
        assert_eq!(frozen, 0);
        assert_eq!(out, next);
    }
}
