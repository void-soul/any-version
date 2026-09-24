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
use super::models::{AggregateConfig, AiProvider, RouteCandidate};
use super::route::normalize_route_chain;
use super::usage::{log_usage_db_timed, log_usage_entry, log_usage_failure, UsageEntry};

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
}

/// 按出站协议拼上游 URL，返回 (url, 鉴权头名)。
/// 拼接规则与 proxy::server::build_upstream_url 一致（避免 /v1/v1 之类的重复）。
pub fn build_candidate_url(
    outbound: Outbound,
    base: &str,
    model: &str,
    is_stream: bool,
) -> (String, &'static str) {
    let base = base.trim().trim_end_matches('/');
    match outbound {
        Outbound::OpenAi => {
            let url = if base.ends_with("/chat/completions") {
                base.to_string()
            } else if base.ends_with("/v1") {
                format!("{base}/chat/completions")
            } else {
                format!("{base}/v1/chat/completions")
            };
            (url, "Authorization")
        }
        Outbound::Anthropic => {
            let url = if base.ends_with("/messages") {
                base.to_string()
            } else if base.ends_with("/v1") {
                format!("{base}/messages")
            } else {
                format!("{base}/v1/messages")
            };
            (url, "x-api-key")
        }
        Outbound::Google => {
            let gbase = if let Some(stripped) = base.strip_suffix("/v1beta") {
                stripped
            } else {
                base
            };
            let gbase = gbase.trim_end_matches('/');
            let url = if is_stream {
                format!("{gbase}/v1beta/models/{model}:streamGenerateContent?alt=sse")
            } else {
                format!("{gbase}/v1beta/models/{model}:generateContent")
            };
            (url, "x-goog-api-key")
        }
    }
}

/// 从供应商配置选出出站协议与端点（openai → anthropic → google 优先级）。
fn pick_outbound(provider: &AiProvider) -> Option<(Outbound, String)> {
    let openai = provider.openai_url.trim().to_string();
    let anthropic = provider.anthropic_url.trim().to_string();
    let google = provider.google_url.trim().to_string();
    if !openai.is_empty() {
        return Some((Outbound::OpenAi, openai));
    }
    if !anthropic.is_empty() {
        return Some((Outbound::Anthropic, anthropic));
    }
    if !google.is_empty() {
        return Some((Outbound::Google, google));
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
        let Some((outbound, base)) = pick_outbound(provider) else {
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
        });
    }
    out
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
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/messages", post(messages))
        .route("/v1/models", get(list_models))
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

        let mut attempts = 0usize;
        loop {
            attempts += 1;
            match try_candidate(&state, candidate, inbound, &body, stream_requested).await {
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
    let converted = match (inbound, candidate.outbound) {
        ("openai", Outbound::OpenAi) | ("anthropic", Outbound::Anthropic) => upstream,
        ("openai", Outbound::Anthropic) => {
            crate::proxy::transform::anthropic_response_to_openai(&upstream, request_model)
        }
        ("openai", Outbound::Google) => {
            crate::proxy::google::google_response_to_openai(&upstream, request_model)
        }
        ("anthropic", Outbound::OpenAi) => {
            crate::proxy::transform::openai_response_to_anthropic(&upstream, request_model)
        }
        ("anthropic", Outbound::Google) => {
            crate::proxy::google::google_response_to_anthropic(&upstream, request_model)
        }
        _ => upstream,
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
async fn try_candidate(
    state: &AggState,
    candidate: &AggCandidate,
    inbound: &'static str,
    body: &Value,
    stream_requested: bool,
) -> Result<Response, CandidateError> {
    let model = &candidate.model_id;
    // 请求体转换：入口协议 → 候选出站协议（model 统一改为候选模型）
    let (upstream_body, upstream_stream) = match (inbound, candidate.outbound) {
        ("openai", Outbound::OpenAi) | ("anthropic", Outbound::Anthropic) => {
            // 同协议也要改写：入口模型是 kiro-proxy，上游只认自己配置的模型名
            (rewrite_upstream_model(body.clone(), model), stream_requested)
        }
        // 跨协议：v1 先按非流式请求并整体转换（流式转换仅覆盖 a↔o 主流组合）
        (_, _) => {
            let mut b = body.clone();
            if stream_requested {
                b["stream"] = Value::Bool(false);
            }
            (b, false)
        }
    };
    let upstream_body = match (inbound, candidate.outbound) {
        ("openai", Outbound::OpenAi) | ("anthropic", Outbound::Anthropic) => {
            upstream_body
        }
        ("openai", Outbound::Anthropic) => {
            crate::proxy::transform::openai_to_anthropic(&upstream_body, model, None)
        }
        ("anthropic", Outbound::OpenAi) => {
            crate::proxy::transform::anthropic_to_openai(&upstream_body, model, None)
        }
        ("openai", Outbound::Google) => {
            crate::proxy::google::openai_to_google(&upstream_body, model)
        }
        ("anthropic", Outbound::Google) => {
            crate::proxy::google::anthropic_to_google(&upstream_body, model)
        }
        // 入口协议只有 openai/anthropic（axum 路由保证），兜底防御
        _ => upstream_body,
    };

    let (url, auth_name) = build_candidate_url(candidate.outbound, &candidate.base_url, model, upstream_stream);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| CandidateError {
            class: FailureClass::Transient,
            message: format!("HTTP 客户端构建失败: {}", e),
            retry_after: None,
        })?;
    let mut req = client.post(&url);
    req = match auth_name {
        "x-api-key" => req.header("x-api-key", &candidate.api_key).header("anthropic-version", "2023-06-01"),
        "x-goog-api-key" => req.header("x-goog-api-key", &candidate.api_key),
        _ => req.bearer_auth(&candidate.api_key),
    };
    let resp = req
        .json(&upstream_body)
        .send()
        .await
        .map_err(|e| {
            let message = if e.is_connect() {
                format!("连接被拒绝（{}）", url)
            } else if e.is_timeout() {
                "请求超时".to_string()
            } else {
                format!("请求失败: {}", e)
            };
            CandidateError { class: FailureClass::Transient, message, retry_after: None }
        })?;
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
        });
    }

    let started = std::time::Instant::now();

    // 同协议 + 流式：字节流透传（首字节后不再切换候选，避免重复计费）
    if upstream_stream && inbound == candidate.outbound.as_str() {
        let stream = resp.bytes_stream();
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
            return stream_cross_protocol(state, candidate, inbound, resp, started);
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
) -> Result<Response, CandidateError> {
    use axum::response::sse::{Event, Sse};
    use futures_util::StreamExt;

    let model = candidate.model_id.clone();
    let provider_id = candidate.provider_id.clone();
    let outbound = candidate.outbound;
    let stream = resp.bytes_stream();
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
        while let Some(r) = stream.next().await {
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
        }
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
        let (url, auth) = build_candidate_url(OpenAi, "https://api.x.com/v1", "m", false);
        assert_eq!(url, "https://api.x.com/v1/chat/completions");
        assert_eq!(auth, "Authorization");
        let (url, auth) = build_candidate_url(Anthropic, "https://api.anthropic.com", "m", false);
        assert_eq!(url, "https://api.anthropic.com/v1/messages");
        assert_eq!(auth, "x-api-key");
        let (url, auth) = build_candidate_url(Anthropic, "https://x.com/anthropic/v1", "m", false);
        assert_eq!(url, "https://x.com/anthropic/v1/messages");
        let (url, auth) = build_candidate_url(Google, "https://g.com", "m", true);
        assert_eq!(url, "https://g.com/v1beta/models/m:streamGenerateContent?alt=sse");
        assert_eq!(auth, "x-goog-api-key");
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
        };
        let (outbound, base) = pick_outbound(&provider).unwrap();
        assert_eq!(outbound, Outbound::OpenAi);
        assert_eq!(base, "https://x/v1");
        // 只有 anthropic → 用 anthropic
        provider.openai_url = String::new();
        let (outbound, base) = pick_outbound(&provider).unwrap();
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
