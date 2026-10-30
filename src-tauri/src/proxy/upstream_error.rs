//! 上游**业务码**分类：回答「上游到底为什么拒」，而不是只知道 HTTP 状态码。
//!
//! 为什么需要它：WorkBuddy 后端把真正的语义放在响应体的 `code` 字段里 ——
//! 渠道校验（`11-128` / `11128`）、模型不存在（`11102`）、参数非法（`11133`）、
//! 积分耗尽（`14018`）**都可能带 400**。只看状态码时它们全落进「瞬时错误」，
//! 于是：该换路径的不换（请求硬失败）、模型级问题被当网络抖动、日志里只剩
//! `transient` 这种看不出所以然的标签。
//!
//! 契约来源：Go 版 workbuddy2api 的补丁集（`workbuddy-manager/dev/upstream-patches/`
//! 0004/0006/0018/0025）。逐条语义见技能文档
//! `.codebuddy/skills/cockpit-buddy-sync/references/workbuddy-manager.md` §D.2。
//!
//! 本模块是**纯逻辑**（无 IO、无状态），便于单测；控制流（是否换路径、是否整流）
//! 由调用方按 [`UpstreamErrorKind`] 的谓词决定。

use std::sync::LazyLock;

use regex::Regex;

use super::failure::{classify_failure, FailureClass};

/// 上游失败的业务种类。
///
/// 命名对齐参考侧 `ErrKind`，但**只保留我们有处置动作的那些**：
/// 我们单账号、无冷却池，所以「账号级冷却 / 连败降权 / 轮转换号」在这里退化为
/// 「分类标签 + 是否换路径 + 是否值得整流」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpstreamErrorKind {
    /// 不是错误（成功，或防御分支）
    None,
    /// 渠道强校验：换 `/console` 路径重试，**永不当成封号**
    ChannelCheck,
    /// 模型不存在（主路径的 11102）：客户端应停止重试
    ModelMissing,
    /// 回退路径上的 11102：只是「这条路径没这个模型」，零处置
    FallbackModelMissing,
    /// 积分耗尽（14018 / 402 / 额度文案）
    CreditExhausted,
    /// 12153 session dead（会话被服务端判死）
    SessionDead,
    /// 401 但无 12153（网关拦截页 / 空体 / HTML）
    AuthRejected,
    /// 账号被封（11140 / `request illegal` / trial 未激活）
    AccountBanned,
    /// 403 且是网关拦截页（非业务信封）
    WafBlock,
    /// 内容安全拒绝
    ContentBlocked,
    /// 工具调用与结果不配对（11148）
    ToolCallMismatch,
    /// 参数非法（11133 / 11101）
    BadParams,
    /// prompt 过长（11115）
    PromptTooLong,
    /// 消息内容为空（11151）
    EmptyMessage,
    /// 限流（429）
    RateLimit,
    /// 404 路径不存在
    NotFound,
    /// 5xx
    Server,
    /// 其它 4xx
    Client,
}

impl UpstreamErrorKind {
    /// 落日志/落库用的稳定标识。**改这些字符串等于改历史统计口径**，别随手改。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::ChannelCheck => "channel_check",
            Self::ModelMissing => "model_missing",
            Self::FallbackModelMissing => "fallback_model_missing",
            Self::CreditExhausted => "credit_exhausted",
            Self::SessionDead => "session_dead",
            Self::AuthRejected => "auth_rejected",
            Self::AccountBanned => "account_banned",
            Self::WafBlock => "waf_block",
            Self::ContentBlocked => "content_blocked",
            Self::ToolCallMismatch => "tool_call_mismatch",
            Self::BadParams => "bad_params",
            Self::PromptTooLong => "prompt_too_long",
            Self::EmptyMessage => "empty_message",
            Self::RateLimit => "rate_limit",
            Self::NotFound => "not_found",
            Self::Server => "server",
            Self::Client => "client",
        }
    }

    /// 是否值得做**反应式整流**（改字段再重发一次）。
    ///
    /// 整流只解「请求体字段不被上游接受」这一类问题。鉴权 / 配额 / 模型缺失 /
    /// 渠道校验 / 工具配对 / 内容审核都不是字段问题，重发一遍只是多打一次上游，
    /// 还可能把「账号有麻烦」误记成「请求有问题」。
    pub fn worth_rectify(self) -> bool {
        !matches!(
            self,
            Self::ChannelCheck
                | Self::ModelMissing
                | Self::FallbackModelMissing
                | Self::CreditExhausted
                | Self::SessionDead
                | Self::AuthRejected
                | Self::AccountBanned
                | Self::ContentBlocked
                | Self::ToolCallMismatch
                | Self::BadParams
                | Self::PromptTooLong
                | Self::EmptyMessage
        )
    }

    /// 是否是「请求本身的问题」：换路径、换账号都没用，直接还给客户端。
    ///
    /// 当前控制流用 [`Self::worth_rectify`] 表达同一判断的超集；保留这个更窄的谓词，
    /// 供将来「按类别决定是否换账号/是否需要用户改请求」时复用。
    #[allow(dead_code)]
    pub fn is_request_level(self) -> bool {
        matches!(
            self,
            Self::ContentBlocked
                | Self::ToolCallMismatch
                | Self::BadParams
                | Self::PromptTooLong
                | Self::EmptyMessage
        )
    }
}

/// 只认 `"code"` 键：`"error_code"`（前一位不是引号）、`requestId`（键名不同）都不匹配。
///
/// 值允许三种写法：`11128` / `"11128"` / `11-128`（后者是**非法 JSON**，上游真这么发）。
static CODE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#""code"\s*:\s*"?([0-9]{2,8}(?:-[0-9]{1,4})?)"?"#).expect("上游业务码正则")
});

/// 取出响应体里的业务码（字符串形式，保留 `11-128` 这种带横线的写法）。
pub fn code_of(body: &str) -> Option<String> {
    CODE_RE
        .captures(body)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
}

/// 渠道强校验：**必须「业务码 + 文案」同时成立**。
///
/// 只认码会误伤：同一个 `11-128` 也会承载内容安全拦截
/// （`{"code":11-128,"msg":"blocked by security policy"}`），那种情况换路径只是
/// 把同一个拒绝再打一次。
///
/// 也**绝不能用 `msg` 当判据**：渠道校验的封套里会带 `request illegal` /
/// `conversation id missing` 这些历史 msg，而 `request illegal` 的老语义是
/// 「账号被封」—— 先判它会把握手期的好号直接写成封禁。
///
/// 文案指纹只认三个固定短语，**不认泛化的 "channel"**：`11128 Illegal API invocation
/// from an unapproved channel` 是「入站 role=developer 不被接受」（已在
/// `convert::normalize_openai_message_roles` 修掉），把它当渠道校验只会白换一次路径。
pub fn is_channel_check(body: &str) -> bool {
    let has_code = matches!(code_of(body).as_deref(), Some("11128") | Some("11-128"));
    if !has_code {
        return false;
    }
    let lower = body.to_lowercase();
    body.contains("渠道校验")
        || lower.contains("channel verification")
        || lower.contains("official client")
}

fn has_code(code: Option<&str>, want: &str) -> bool {
    code == Some(want)
}

fn is_credit_text(lower: &str) -> bool {
    const HINTS: &[&str] = &[
        "insufficient credit",
        "no credit",
        "credit exhausted",
        "credits exhausted",
        "out of credit",
    ];
    HINTS.iter().any(|h| lower.contains(h))
}

fn is_content_blocked(lower: &str) -> bool {
    const HINTS: &[&str] = &[
        "blocked by security policy",
        "content policy",
        "safety policy",
        "content moderation",
        "blocked by safety",
    ];
    HINTS.iter().any(|h| lower.contains(h))
}

/// 分类入口。**判定顺序有语义**，不是随便排的：
///
/// 1. `11102` 最先 —— 它可能夹在 400/404 里，先把「模型」这一维摘出来；
/// 2. 积分耗尽早于鉴权 —— 402 也可能被网关包成别的状态；
/// 3. 401 再细分（12153 = 会话判死，其余 = 网关拒）；
/// 4. **渠道校验必须早于 `request illegal`** —— 见 [`is_channel_check`] 的说明；
/// 5. 其余按业务码，最后才落到状态码兜底。
pub fn classify(status: u16, body: &str) -> UpstreamErrorKind {
    let code = code_of(body);
    let lower = body.to_lowercase();

    if has_code(code.as_deref(), "11102") {
        return UpstreamErrorKind::ModelMissing;
    }
    if status == 402
        || has_code(code.as_deref(), "14018")
        || is_credit_text(&lower)
        || body.contains("余额不足")
        || body.contains("额度不足")
        || body.contains("积分不足")
    {
        return UpstreamErrorKind::CreditExhausted;
    }
    if status == 401 {
        if has_code(code.as_deref(), "12153") || lower.contains("offline user session not found") {
            return UpstreamErrorKind::SessionDead;
        }
        return UpstreamErrorKind::AuthRejected;
    }
    // ← 这一行必须在下面 `request illegal` 的判定之前
    if is_channel_check(body) {
        return UpstreamErrorKind::ChannelCheck;
    }
    if has_code(code.as_deref(), "11148")
        || lower.contains("tool calls and tool results do not match")
    {
        return UpstreamErrorKind::ToolCallMismatch;
    }
    if has_code(code.as_deref(), "11133") || has_code(code.as_deref(), "11101") {
        return UpstreamErrorKind::BadParams;
    }
    if has_code(code.as_deref(), "11115") || lower.contains("prompt is too long") {
        return UpstreamErrorKind::PromptTooLong;
    }
    if has_code(code.as_deref(), "11151")
        || lower.contains("a message has empty content")
        || body.contains("存在空白消息")
    {
        return UpstreamErrorKind::EmptyMessage;
    }
    if is_content_blocked(&lower) {
        return UpstreamErrorKind::ContentBlocked;
    }
    // 历史语义：request illegal = 账号被封；trial 未激活也归这类（要用户处理账号）
    if has_code(code.as_deref(), "11140")
        || lower.contains("request illegal")
        || lower.contains("trial not activated")
    {
        return UpstreamErrorKind::AccountBanned;
    }
    // 403 且响应体不是业务信封（网关拦截页 / 空体 / HTML）
    if status == 403 && !body.trim_start().starts_with('{') {
        return UpstreamErrorKind::WafBlock;
    }

    match status {
        404 => UpstreamErrorKind::NotFound,
        429 => UpstreamErrorKind::RateLimit,
        500..=599 => UpstreamErrorKind::Server,
        400..=499 => UpstreamErrorKind::Client,
        _ => UpstreamErrorKind::None,
    }
}

/// 映射到用量统计的 `failure_class`。
///
/// 只有在业务码明确时才覆盖原判定；其余（`None` / `Client` / 状态码兜底）
/// 仍走 [`classify_failure`]，保证 AI 模块的既有统计口径不变。
pub fn to_failure_class(kind: UpstreamErrorKind, status: u16, body: &str) -> FailureClass {
    match kind {
        UpstreamErrorKind::CreditExhausted => FailureClass::Billing,
        UpstreamErrorKind::SessionDead
        | UpstreamErrorKind::AuthRejected
        | UpstreamErrorKind::AccountBanned => FailureClass::Authentication,
        UpstreamErrorKind::ModelMissing
        | UpstreamErrorKind::FallbackModelMissing
        | UpstreamErrorKind::NotFound => FailureClass::ModelUnavailable,
        UpstreamErrorKind::RateLimit | UpstreamErrorKind::WafBlock => FailureClass::RateLimit,
        // 渠道校验：换路径后可能成功，按「可重试的瞬时问题」记账
        UpstreamErrorKind::ChannelCheck | UpstreamErrorKind::Server => FailureClass::Transient,
        UpstreamErrorKind::ContentBlocked
        | UpstreamErrorKind::ToolCallMismatch
        | UpstreamErrorKind::BadParams
        | UpstreamErrorKind::PromptTooLong
        | UpstreamErrorKind::EmptyMessage => FailureClass::Fatal,
        UpstreamErrorKind::None | UpstreamErrorKind::Client => {
            classify_failure(Some(status), body)
        }
    }
}

/// 可搜索的错误标签：`channel_check (code 11-128, http 400)`。
///
/// **不许写死某个码**（历史事故里理由被写成固定 `11140`，现场其实是
/// `http 400 + code 11-128`，排查时被误导）。
pub fn label(kind: UpstreamErrorKind, status: u16, body: &str) -> String {
    match code_of(body) {
        Some(code) => format!("{} (code {}, http {})", kind.as_str(), code, status),
        None => format!("{} (http {})", kind.as_str(), status),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_of_accepts_number_string_and_dashed_forms() {
        assert_eq!(code_of(r#"{"code":40001}"#).as_deref(), Some("40001"));
        assert_eq!(code_of(r#"{"code":"11128"}"#).as_deref(), Some("11128"));
        // 上游真会发非法 JSON：值没引号
        assert_eq!(code_of(r#"{"code":11-128}"#).as_deref(), Some("11-128"));
        assert_eq!(code_of(r#"{ "code" : "11128" }"#).as_deref(), Some("11128"));
    }

    #[test]
    fn code_of_ignores_other_keys_and_empty_bodies() {
        assert_eq!(code_of(r#"{"requestId":"11128"}"#), None);
        assert_eq!(code_of(r#"{"error_code":11128}"#), None);
        assert_eq!(code_of(""), None);
        assert_eq!(code_of("<html><title>401</title></html>"), None);
    }

    #[test]
    fn channel_check_needs_code_and_phrase() {
        assert!(is_channel_check(
            r#"{"code":"11-128","msg":"渠道校验失败"}"#
        ));
        assert!(is_channel_check(
            r#"{"code":"11-128","msg":"channel verification required"}"#
        ));
        // 只有码、没有文案 → 不算（可能是别的语义）
        assert!(!is_channel_check(r#"{"code":11128,"msg":"whatever"}"#));
        // 只有文案、没有码 → 不算
        assert!(!is_channel_check(r#"{"msg":"渠道校验失败"}"#));
        // **关键反例**：同样带 11128、同样有 "channel" 字样，但它是「入站 role=developer
        // 不被接受」—— 已在 convert::normalize_openai_message_roles 修掉。把它当渠道校验
        // 会白换一次路径，还会掩盖真正的角色问题。所以文案指纹只认那三个固定短语。
        assert!(!is_channel_check(
            r#"{"code":11128,"msg":"Illegal API invocation from an unapproved channel"}"#
        ));
    }

    #[test]
    fn channel_check_does_not_swallow_content_block() {
        // 同一个码承载内容审核：换路径只会把同一个拒绝再打一次
        assert_eq!(
            classify(400, r#"{"code":11-128,"msg":"blocked by security policy"}"#),
            UpstreamErrorKind::ContentBlocked
        );
    }

    #[test]
    fn channel_check_is_classified_before_account_banned() {
        // 真实形态：渠道校验封套的顶层 msg 是历史文案 `request illegal`，
        // 渠道语义在 displayTips 里。若先判 msg 就会被写成「账号被封」。
        let body = r#"{"code":"11-128","msg":"request illegal","displayTips":{"message":"渠道校验未通过"}}"#;
        assert_eq!(classify(400, body), UpstreamErrorKind::ChannelCheck);
    }

    #[test]
    fn request_illegal_without_channel_phrase_is_account_banned() {
        assert_eq!(
            classify(403, r#"{"code":11140,"msg":"request illegal"}"#),
            UpstreamErrorKind::AccountBanned
        );
    }

    #[test]
    fn model_missing_and_credit_exhausted() {
        assert_eq!(
            classify(400, r#"{"code":11102,"msg":"model [x] service info not found"}"#),
            UpstreamErrorKind::ModelMissing
        );
        assert_eq!(
            classify(400, r#"{"code":14018,"msg":"credit not enough"}"#),
            UpstreamErrorKind::CreditExhausted
        );
        assert_eq!(classify(402, ""), UpstreamErrorKind::CreditExhausted);
    }

    #[test]
    fn auth_401_splits_on_12153() {
        assert_eq!(
            classify(401, r#"{"code":12153,"msg":"session dead"}"#),
            UpstreamErrorKind::SessionDead
        );
        assert_eq!(classify(401, ""), UpstreamErrorKind::AuthRejected);
        assert_eq!(
            classify(401, "<html><title>401 Authorization Required</title></html>"),
            UpstreamErrorKind::AuthRejected
        );
    }

    #[test]
    fn waf_block_only_for_non_json_403() {
        assert_eq!(
            classify(403, "<html>APISIX blocked</html>"),
            UpstreamErrorKind::WafBlock
        );
        assert_eq!(
            classify(403, r#"{"code":"11-128","msg":"渠道校验"}"#),
            UpstreamErrorKind::ChannelCheck
        );
    }

    #[test]
    fn status_only_fallbacks_keep_old_behaviour() {
        assert_eq!(classify(404, ""), UpstreamErrorKind::NotFound);
        assert_eq!(classify(429, ""), UpstreamErrorKind::RateLimit);
        assert_eq!(classify(503, ""), UpstreamErrorKind::Server);
        assert_eq!(classify(400, "totally unknown"), UpstreamErrorKind::Client);
    }

    #[test]
    fn rectify_only_for_field_level_errors() {
        assert!(!UpstreamErrorKind::ChannelCheck.worth_rectify());
        assert!(!UpstreamErrorKind::ModelMissing.worth_rectify());
        assert!(!UpstreamErrorKind::ToolCallMismatch.worth_rectify());
        assert!(UpstreamErrorKind::Client.worth_rectify());
        assert!(UpstreamErrorKind::Server.worth_rectify());
    }

    #[test]
    fn failure_class_mapping_is_stable() {
        assert_eq!(
            to_failure_class(UpstreamErrorKind::ModelMissing, 400, ""),
            FailureClass::ModelUnavailable
        );
        assert_eq!(
            to_failure_class(UpstreamErrorKind::CreditExhausted, 400, ""),
            FailureClass::Billing
        );
        assert_eq!(
            to_failure_class(UpstreamErrorKind::SessionDead, 401, ""),
            FailureClass::Authentication
        );
        assert_eq!(
            to_failure_class(UpstreamErrorKind::ToolCallMismatch, 400, ""),
            FailureClass::Fatal
        );
        // 业务码不明时退回原判定（400 → Transient，历史口径）
        assert_eq!(
            to_failure_class(UpstreamErrorKind::Client, 400, "unknown"),
            FailureClass::Transient
        );
    }

    #[test]
    fn label_quotes_real_code_not_a_fixed_one() {
        let l = label(
            UpstreamErrorKind::ChannelCheck,
            400,
            r#"{"code":"11-128","msg":"渠道校验"}"#,
        );
        assert!(l.contains("channel_check"));
        assert!(l.contains("code 11-128"));
        assert!(l.contains("http 400"));
        assert!(!l.contains("11140"));
    }
}
