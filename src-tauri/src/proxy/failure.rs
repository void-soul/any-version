//! Shared upstream failure classification, independent of application services.

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
