//! 聚合服务：本地 HTTP 聚合代理（按聚合链顺序请求，可启停 + 日志回传）。
//!
//! - 入口协议：OpenAI 兼容 `POST /v1/chat/completions`（`/v1/messages` 暂不支持，返回 501 并说明）
//! - 链路顺序：按 `route_chain` 依次尝试；当前候选失败（含重试）后切下一个
//! - 上下文限制：请求超过 `AggregateConfig.context_limit` 时裁剪最早的非 system 消息
//! - 压缩：Headroom 开启时先走 `/v1/compress`，失败按 `on_unavailable` 决定放行或报错

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
use super::usage::log_usage_db_timed;

/// 单个候选（启动快照）：解析好的上游端点与凭据。
#[derive(Clone, Debug)]
struct AggCandidate {
    provider_id: String,
    provider_name: String,
    base_url: String,
    api_key: String,
    model_id: String,
}

/// Headroom 压缩运行时参数（启动快照）。
#[derive(Clone, Debug)]
struct HeadroomRuntime {
    base_url: String,
    timeout_ms: u64,
    /// true = failClosed（压缩不可用直接报错）；false = failOpen（跳过压缩继续）
    fail_closed: bool,
}

#[derive(Clone)]
struct AggState {
    app: tauri::AppHandle,
    /// 本服务监听端口（用于运行期自引用兜底判定）
    port: u16,
    candidates: Arc<Vec<AggCandidate>>,
    /// 候选健康状态（失败计数 + 冷却截止时间），键 = `provider::model`
    health: Arc<Mutex<HashMap<String, CandidateHealth>>>,
    headroom: Option<HeadroomRuntime>,
    context_limit: u64,
}

struct AggregateEntry {
    port: u16,
    handle: tokio::task::JoinHandle<()>,
}

static AGGREGATE: OnceLock<Mutex<Option<AggregateEntry>>> = OnceLock::new();

/// 聚合服务对外只暴露一个模型（对内才按链分发）——这也是「聚合」的含义。
pub const AGGREGATE_MODEL_ID: &str = "auto";

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

/// 冷却时长（切换后多久内不再尝试该候选）。
pub fn cooldown_for(class: FailureClass, retry_after: Option<u64>, consecutive: u32) -> std::time::Duration {
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

/// 单个候选请求失败的结构化信息。
struct CandidateError {
    class: FailureClass,
    message: String,
    retry_after: Option<u64>,
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

// ─── 纯逻辑：上下文裁剪 ───

/// 是否为 CJK 字符（中日韩 + 全角标点），这类字符约 1 字 ≈ 1 token。
fn is_cjk(ch: char) -> bool {
    matches!(ch,
        '\u{4E00}'..='\u{9FFF}'   // 中日韩统一表意文字
        | '\u{3400}'..='\u{4DBF}' // 扩展 A
        | '\u{F900}'..='\u{FAFF}' // 兼容表意文字
        | '\u{3000}'..='\u{303F}' // 中文标点
        | '\u{FF00}'..='\u{FFEF}' // 全角形式
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
    let mut current = load_ai_config();
    if running_port().is_some() && current.aggregate.port != config.port {
        return Err("聚合服务运行中不能修改端口，请先停止服务".to_string());
    }
    current.aggregate = config.clone();
    save_ai_config_to_file(&current)?;
    Ok(config)
}

// ─── 启停 / 状态 ───

fn running_port() -> Option<u16> {
    AGGREGATE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|e| e.port))
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

/// 构建候选快照：把链路里的 (provider_id, model_id) 解析成可用的上游端点。
/// 取供应商的 `openai_url`（聚合服务当前只转发 OpenAI 协议端点）。
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
        let base = provider.openai_url.trim().trim_end_matches('/').to_string();
        if base.is_empty() || provider.api_key.trim().is_empty() {
            continue;
        }
        // 自引用（指向聚合服务自身）直接剔除，避免请求递归
        if is_self_referential(&base, aggregate_port) {
            continue;
        }
        out.push(AggCandidate {
            provider_id: provider.id.clone(),
            provider_name: if provider.name.trim().is_empty() { provider.id.clone() } else { provider.name.clone() },
            base_url: base,
            api_key: provider.api_key.clone(),
            model_id: candidate.model_id.clone(),
        });
    }
    out
}

/// 启动聚合服务。
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
    if candidates.is_empty() {
        // 区分两种空链：一种是没勾候选，一种是勾的全是「本地聚合」这类自引用
        let any_candidate = !chain.is_empty();
        return Err(if any_candidate {
            "聚合链里只有指向本服务的候选（如「本地聚合」），会造成请求递归，已全部剔除。请改选其他供应商".to_string()
        } else {
            "聚合链为空或候选缺少 OpenAI 端点 / API Key，请先勾选候选".to_string()
        });
    }
    if candidates.len() < chain.len() {
        emit_aggregate_log(
            &app,
            "start",
            "warn",
            format!(
                "已剔除 {} 个不可用/自引用的候选（如把「本地聚合」加进自身链路会递归）",
                chain.len() - candidates.len()
            ),
        );
    }
    if let Some(owner) = crate::commands::utils::port_conflict_description(port) {
        return Err(format!("端口 {} 已被占用：{}，请更换端口", port, owner));
    }

    let headroom = if config.headroom.enabled {
        Some(HeadroomRuntime {
            base_url: format!("http://127.0.0.1:{}", config.headroom.port),
            timeout_ms: config.headroom.timeout_ms.max(200),
            fail_closed: config.headroom.on_unavailable == "failClosed",
        })
    } else {
        None
    };
    let context_limit = config.aggregate.context_limit;
    let state = AggState {
        app: app.clone(),
        port,
        candidates: Arc::new(candidates.clone()),
        health: Arc::new(Mutex::new(HashMap::new())),
        headroom,
        context_limit,
    };

    let router = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/messages", post(messages_unsupported))
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

    emit_aggregate_log(
        &app,
        "start",
        "info",
        format!(
            "聚合服务已启动：http://127.0.0.1:{}（{} 个候选；上下文上限 {} tokens）",
            port,
            candidates.len(),
            context_limit
        ),
    );
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

/// 对外只暴露一个模型（聚合 = 多个候选对内的单一入口），内部按链分发到具体模型。
async fn list_models(State(state): State<AggState>) -> Json<Value> {
    let _ = &state;
    Json(serde_json::json!({
        "object": "list",
        "data": [{
            "id": AGGREGATE_MODEL_ID,
            "object": "model",
            "owned_by": "aggregate",
        }]
    }))
}

/// Anthropic 协议入口暂不支持（转换层在工具代理里，聚合服务先用 OpenAI 协议）。
async fn messages_unsupported() -> Response {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(serde_json::json!({
            "error": {
                "message": "聚合服务暂只支持 OpenAI 协议入口 /v1/chat/completions；Anthropic 入口将在后续版本接入",
                "type": "not_implemented"
            }
        })),
    )
        .into_response()
}

async fn chat_completions(State(state): State<AggState>, Json(mut body): Json<Value>) -> Response {
    // 上下文限制：裁剪最早的非 system 消息
    if let Some(messages) = body.get("messages").and_then(|m| m.as_array()).cloned() {
        let (kept, trimmed) = trim_messages_to_limit(messages, state.context_limit);
        if trimmed > 0 {
            body["messages"] = Value::Array(kept);
            emit_aggregate_log(
                &state.app,
                "context",
                "warn",
                format!("上下文超限，已裁剪 {} 条最早的消息（上限 {} tokens）", trimmed, state.context_limit),
            );
        }
    }

    // Headroom 压缩旁路
    if let Some(hr) = &state.headroom {
        match compress_body(hr, &body).await {
            Ok(Some(compressed)) => {
                emit_aggregate_log(&state.app, "compress", "info", "已压缩请求上下文".to_string());
                body = compressed;
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
    let mut last_error = String::new();
    let stream = body.get("stream").and_then(|s| s.as_bool()).unwrap_or(false);

    for (idx, candidate) in state.candidates.iter().enumerate() {
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
        // 冷却中的候选直接跳过（上一轮失败后的惩罚期）
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

        let mut body_for_candidate = body.clone();
        // 对外统一用聚合模型名；对内改写为候选自身模型
        body_for_candidate["model"] = Value::String(candidate.model_id.clone());

        let mut attempts = 0usize;
        loop {
            attempts += 1;
            match try_candidate(candidate, &body_for_candidate, stream).await {
                Ok(response) => {
                    clear_health(&state.health, &key);
                    if idx > 0 || attempts > 1 {
                        emit_aggregate_log(
                            &state.app,
                            "route",
                            "warn",
                            format!(
                                "已切换到候选 #{} {} / {}（第 {} 次尝试）",
                                idx + 1,
                                candidate.provider_name,
                                candidate.model_id,
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
                    record_aggregate_usage(&state, candidate, &body_for_candidate, &response.1, started);
                    return response.0;
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
                    if attempts >= retry_budget(err.class) {
                        let cooldown = record_failure(
                            &state.health,
                            &key,
                            err.class,
                            err.retry_after,
                        );
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
        format!("聚合链全部候选失败（{} 个）: {}", state.candidates.len(), last_error),
    );
    (
        StatusCode::BAD_GATEWAY,
        Json(serde_json::json!({
            "error": {
                "message": format!(
                    "聚合链 {} 个候选全部失败，最后错误：{}。请检查候选的 API Key / 额度，或到「服务」页查看聚合日志",
                    state.candidates.len(), last_error
                ),
                "type": "aggregate_chain_exhausted"
            }
        })),
    )
        .into_response()
}

/// 单候选请求：返回 (响应, 响应体文本用于统计)。
async fn try_candidate(
    candidate: &AggCandidate,
    body: &Value,
    stream: bool,
) -> Result<(Response, String), CandidateError> {
    let url = format!("{}/chat/completions", candidate.base_url);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| CandidateError {
            class: FailureClass::Transient,
            message: format!("HTTP 客户端构建失败: {}", e),
            retry_after: None,
        })?;
    let resp = client
        .post(&url)
        .bearer_auth(&candidate.api_key)
        .json(body)
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
    // 限流场景优先遵循上游 Retry-After
    let retry_after = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok());
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json")
        .to_string();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        let class = classify_failure(Some(status.as_u16()), &text);
        return Err(CandidateError {
            class,
            message: format!("HTTP {} {}", status.as_u16(), trim_error(&text)),
            retry_after,
        });
    }
    if stream {
        // 流式：透传上游 SSE 字节流（首字节后不再切换候选，避免重复计费）
        let stream = resp.bytes_stream();
        let body_stream = axum::body::Body::from_stream(stream);
        let response = Response::builder()
            .status(StatusCode::OK)
            .header("content-type", if content_type.contains("event-stream") { "text/event-stream" } else { &content_type })
            .body(body_stream)
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
        return Ok((response, String::new()));
    }
    let text = resp.text().await.map_err(|e| CandidateError {
        class: FailureClass::Transient,
        message: format!("读取响应失败: {}", e),
        retry_after: None,
    })?;
    let json: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    Ok((
        (StatusCode::OK, Json(json)).into_response(),
        text,
    ))
}

fn trim_error(text: &str) -> String {
    let cleaned = text.trim().replace('\n', " ");
    if cleaned.len() > 200 {
        format!("{}…", &cleaned[..200])
    } else {
        cleaned
    }
}

/// 该类别是否值得对同一候选再试一次（只有瞬时错误才重试，其余直接切下一个）。
fn is_retryable(class: FailureClass) -> bool {
    retry_budget(class) > 1
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

/// 用量落账：从上游响应里取 usage（缺失则记 0），tool_id 固定为 aggregate。
fn record_aggregate_usage(
    state: &AggState,
    candidate: &AggCandidate,
    request: &Value,
    response_text: &str,
    started: std::time::Instant,
) {
    if response_text.trim().is_empty() {
        return; // 流式透传不落账（拿不到 usage）
    }
    let value: Value = serde_json::from_str(response_text).unwrap_or(Value::Null);
    let usage = value.get("usage").cloned().unwrap_or(Value::Null);
    let input = usage.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    let output = usage.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    if input == 0 && output == 0 {
        return;
    }
    let _ = log_usage_db_timed(
        "aggregate",
        &candidate.model_id,
        Some(&candidate.provider_id),
        input,
        output,
        started.elapsed().as_millis() as u64,
        0,
    );
    let _ = request;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn msg(role: &str, text: &str) -> Value {
        json!({ "role": role, "content": text })
    }

    #[test]
    fn test_estimate_tokens_mixed() {
        assert!(estimate_tokens("") <= 1);
        // 英文约 4 字符 1 token
        assert!(estimate_tokens("a".repeat(100).as_str()) >= 20);
        // 中文约 1 字 1 token
        assert_eq!(estimate_tokens("你好世界"), 4);
        // 混合
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

    fn cand(base: &str) -> AggCandidate {
        AggCandidate {
            provider_id: "p".into(),
            provider_name: "P".into(),
            base_url: base.into(),
            api_key: "sk".into(),
            model_id: "m".into(),
        }
    }

    #[test]
    fn test_is_self_referential_detects_own_service() {
        // 高危：把「本地聚合」加进自己的链路 → 必须识别为自引用
        assert!(is_self_referential("http://127.0.0.1:15888/v1", 15888));
        assert!(is_self_referential("http://localhost:15888/v1", 15888));
        assert!(is_self_referential("http://127.0.0.1:15888", 15888));
    }

    #[test]
    fn test_is_self_referential_allows_others() {
        assert!(!is_self_referential("https://api.openai.com/v1", 15888));
        // 端口不同 = 别的本地服务（Ollama 等），不是自引用
        assert!(!is_self_referential("http://127.0.0.1:11434/v1", 15888));
        assert!(!is_self_referential("http://127.0.0.1:15889/v1", 15888));
        assert!(!is_self_referential("", 15888));
        // 没写端口时无法判定为本服务，按非自引用处理
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
    fn test_is_retryable_by_class() {
        assert!(is_retryable(FailureClass::Transient));
        assert!(!is_retryable(FailureClass::Authentication));
        assert!(!is_retryable(FailureClass::Billing));
        assert!(!is_retryable(FailureClass::RateLimit));
        assert!(!is_retryable(FailureClass::ModelUnavailable));
        assert!(!is_retryable(FailureClass::Fatal));
    }

    // ─── 失败分类与切换策略 ───

    #[test]
    fn test_classify_failure_by_status() {
        assert_eq!(classify_failure(Some(401), ""), FailureClass::Authentication);
        assert_eq!(classify_failure(Some(403), ""), FailureClass::Authentication);
        assert_eq!(classify_failure(Some(402), ""), FailureClass::Billing);
        assert_eq!(classify_failure(Some(404), ""), FailureClass::ModelUnavailable);
        assert_eq!(classify_failure(Some(429), ""), FailureClass::RateLimit);
        assert_eq!(classify_failure(Some(500), ""), FailureClass::Transient);
        assert_eq!(classify_failure(Some(503), ""), FailureClass::Transient);
        // 连接失败（无状态码）按瞬时处理，可重试
        assert_eq!(classify_failure(None, "connection refused"), FailureClass::Transient);
    }

    #[test]
    fn test_classify_failure_prefers_body_hints() {
        // 429 但文案是额度耗尽 → 判为计费问题（冷却更久、不重试）
        assert_eq!(
            classify_failure(Some(429), "error: insufficient_quota"),
            FailureClass::Billing
        );
        assert_eq!(
            classify_failure(Some(400), "insufficient quota for this key"),
            FailureClass::Billing
        );
        // 400 但文案是限流 → 判为限流
        assert_eq!(classify_failure(Some(400), "rate limit reached"), FailureClass::RateLimit);
        // 内容安全：不重试也不切换
        assert_eq!(
            classify_failure(Some(400), "blocked by safety policy"),
            FailureClass::Fatal
        );
        // 模型不存在
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
        // 瞬时：30 → 60 → 120 → 240 → 300（封顶）
        assert_eq!(cooldown_for(FailureClass::Transient, None, 1).as_secs(), 30);
        assert_eq!(cooldown_for(FailureClass::Transient, None, 2).as_secs(), 60);
        assert_eq!(cooldown_for(FailureClass::Transient, None, 3).as_secs(), 120);
        assert_eq!(cooldown_for(FailureClass::Transient, None, 4).as_secs(), 240);
        assert_eq!(cooldown_for(FailureClass::Transient, None, 5).as_secs(), 300);
        assert_eq!(cooldown_for(FailureClass::Transient, None, 99).as_secs(), 300);
        // 限流：默认 60s，遵循 Retry-After 但不超过 24h
        assert_eq!(cooldown_for(FailureClass::RateLimit, None, 1).as_secs(), 60);
        assert_eq!(cooldown_for(FailureClass::RateLimit, Some(120), 1).as_secs(), 120);
        assert_eq!(
            cooldown_for(FailureClass::RateLimit, Some(999_999), 1).as_secs(),
            24 * 3600
        );
        // 鉴权/额度 1h；模型不可用 6h
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
        // 成功后清除 → 立即可用
        clear_health(&health, key);
        assert!(cooldown_remaining_secs(&health, key).is_none());
    }

    #[test]
    fn test_aggregate_exposes_single_model() {
        // 聚合对外只暴露一个模型（否则就不叫聚合了）
        assert_eq!(AGGREGATE_MODEL_ID, "auto");
    }
}
