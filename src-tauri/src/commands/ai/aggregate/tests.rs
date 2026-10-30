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

fn provider_for_test(id: &str, openai_url: &str) -> AiProvider {
    AiProvider {
        id: id.to_string(),
        name: id.to_string(),
        category: "provider".to_string(),
        api_key: "sk-test".to_string(),
        website: String::new(),
        openai_url: openai_url.to_string(),
        anthropic_url: String::new(),
        google_url: String::new(),
        models: vec![],
        active_model_id: None,
        custom_headers: vec![],
        openai_include_v1: None,
        anthropic_include_v1: None,
        promotions: vec![],
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
        ordered
            .iter()
            .map(|c| c.model_id.as_str())
            .collect::<Vec<_>>(),
        vec!["m2", "m1", "m1"]
    );
    // 入口名 / 空名 = 无偏好，保持链序
    assert_eq!(
        prioritize_by_model(chain.clone(), AGGREGATE_MODEL_ID, AGGREGATE_MODEL_ID)[0].provider_id,
        "a"
    );
    assert_eq!(
        prioritize_by_model(chain.clone(), "", AGGREGATE_MODEL_ID)[0].provider_id,
        "a"
    );
    // 无匹配 → 原顺序
    assert_eq!(
        prioritize_by_model(chain, "m9", AGGREGATE_MODEL_ID)[0].provider_id,
        "a"
    );
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
    assert_eq!(
        classify_failure(Some(401), ""),
        FailureClass::Authentication
    );
    assert_eq!(
        classify_failure(Some(403), ""),
        FailureClass::Authentication
    );
    assert_eq!(classify_failure(Some(402), ""), FailureClass::Billing);
    assert_eq!(
        classify_failure(Some(404), ""),
        FailureClass::ModelUnavailable
    );
    assert_eq!(classify_failure(Some(429), ""), FailureClass::RateLimit);
    assert_eq!(classify_failure(Some(500), ""), FailureClass::Transient);
    assert_eq!(classify_failure(Some(503), ""), FailureClass::Transient);
    assert_eq!(
        classify_failure(None, "connection refused"),
        FailureClass::Transient
    );
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
    assert_eq!(
        classify_failure(Some(400), "rate limit reached"),
        FailureClass::RateLimit
    );
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
    assert_eq!(
        cooldown_for(FailureClass::Transient, None, 3).as_secs(),
        120
    );
    assert_eq!(
        cooldown_for(FailureClass::Transient, None, 4).as_secs(),
        240
    );
    assert_eq!(
        cooldown_for(FailureClass::Transient, None, 5).as_secs(),
        300
    );
    assert_eq!(
        cooldown_for(FailureClass::Transient, None, 99).as_secs(),
        300
    );
    assert_eq!(cooldown_for(FailureClass::RateLimit, None, 1).as_secs(), 60);
    assert_eq!(
        cooldown_for(FailureClass::RateLimit, Some(120), 1).as_secs(),
        120
    );
    // 抄 magpie longestWait：Retry-After 只信到 1 小时。脏数据（把毫秒当秒填、
    // 或厂商返回 86400）以前会把候选冻一整天，期间这条链少一个可选项。
    assert_eq!(
        cooldown_for(FailureClass::RateLimit, Some(999_999), 1).as_secs(),
        RETRY_AFTER_TRUST_CAP_SECS
    );
    assert_eq!(
        cooldown_for(FailureClass::RateLimit, Some(86_400), 1).as_secs(),
        RETRY_AFTER_TRUST_CAP_SECS
    );
    assert_eq!(
        cooldown_for(FailureClass::Authentication, None, 1).as_secs(),
        3600
    );
    assert_eq!(cooldown_for(FailureClass::Billing, None, 1).as_secs(), 3600);
    assert_eq!(
        cooldown_for(FailureClass::ModelUnavailable, None, 1).as_secs(),
        6 * 3600
    );
}

/// 限额类优先「等到厂商说的那个点」（抄 magpie `failQuota` 的取值优先级）。
#[test]
fn quota_cooldown_prefers_written_reset_time() {
    let now = 1_760_000_000_000u64;
    // 19 分钟后恢复 → 就等 19 分钟，而不是默认的 1 小时
    let d = quota_cooldown_for(3600, None, Some(now + 19 * 60 * 1000), now);
    assert_eq!(d.as_secs(), 19 * 60);
    // 说了恢复时刻 + 也有 Retry-After → 恢复时刻优先
    let d = quota_cooldown_for(3600, Some(60), Some(now + 19 * 60 * 1000), now);
    assert_eq!(d.as_secs(), 19 * 60);
    // 只有 Retry-After → 用它（封顶 1h）
    assert_eq!(
        quota_cooldown_for(3600, Some(300), None, now).as_secs(),
        300
    );
    assert_eq!(
        quota_cooldown_for(3600, Some(999_999), None, now).as_secs(),
        RETRY_AFTER_TRUST_CAP_SECS
    );
    // 都没有 → 回落默认
    assert_eq!(quota_cooldown_for(3600, None, None, now).as_secs(), 3600);
    // 恢复时刻在 7 天之外（脏数据）→ 不采信，回落 Retry-After / 默认
    let far = now + 30 * 24 * 3600 * 1000;
    assert_eq!(
        quota_cooldown_for(3600, None, Some(far), now).as_secs(),
        3600
    );
    // 恢复时刻已过期 → 不采信
    assert_eq!(
        quota_cooldown_for(3600, None, Some(now - 60_000), now).as_secs(),
        3600
    );
}

/// 解析厂商写的「什么时候恢复」——四种形态（抄 magpie 的 resets* 系列）
#[test]
fn parses_written_reset_times() {
    let now = 1_760_000_000_000u64;

    // 1) resets_in_seconds（ChatGPT 系）
    let body = format!(r#"{{"error":{{"resets_in_seconds":1140}}}}"#);
    let at = parse_resets_at(&body, now).expect("应读出 1140s");
    assert_eq!(at, now + 1_140_000);

    // 2) resets_at（unix 秒）
    let unix_secs = (now / 1000) + 600;
    let body = format!(r#"{{"error":{{"resets_at":{unix_secs}}}}}"#);
    assert_eq!(parse_resets_at(&body, now), Some(unix_secs * 1000));

    // 3) usage limit reached|<unix>（Claude Code 系）
    let body = format!("usage limit reached|{unix_secs}");
    assert_eq!(parse_resets_at(&body, now), Some(unix_secs * 1000));

    // 4) resets in 0h 19min
    assert_eq!(
        parse_resets_at("Rate limit reached. resets in 0h 19min", now),
        Some(now + 19 * 60 * 1000)
    );

    // 5) resets in 2 hours 5 minutes
    assert_eq!(
        parse_resets_at("resets in 2 hours 5 minutes", now),
        Some(now + (2 * 3600 + 5 * 60) * 1000)
    );
}

/// 读不出 / 明显不合法 → 返回 None（走固定时长兜底），绝不猜
#[test]
fn refuses_unreadable_or_invalid_reset_times() {
    let now = 1_760_000_000_000u64;
    // 没有任何恢复信息
    assert_eq!(parse_resets_at("rate limit exceeded", now), None);
    // 月份 13 → 明显是路过的日期，挡掉（magpie 的严格校验）
    assert_eq!(parse_resets_at("将在 2026-13-45 12:00:00 重置", now), None);
    // 过去的时间（已过期）→ 当没读懂
    let past = (now / 1000) - 600;
    assert_eq!(
        parse_resets_at(&format!("usage limit reached|{past}"), now),
        None
    );
    // 超过 7 天上限 → 当没读懂
    let far = (now / 1000) + 30 * 24 * 3600;
    assert_eq!(
        parse_resets_at(&format!("usage limit reached|{far}"), now),
        None
    );
    // 2 月 31 日这类溢出必须被 round-trip 校验挡掉
    assert_eq!(parse_resets_at("将在 2027-02-31 10:00:00 重置", now), None);
}

#[test]
fn record_failure_uses_reset_time_for_billing() {
    let health: Arc<Mutex<HashMap<String, CandidateHealth>>> = Arc::new(Mutex::new(HashMap::new()));
    let key = "p::m";
    let now = now_ms();
    // 约 19 分钟后恢复 → 冷却应接近 19 分钟，而不是固定 1 小时
    let d = record_failure(
        &health,
        key,
        FailureClass::Billing,
        None,
        Some(now + 19 * 60 * 1000),
    );
    assert!(
        d.as_secs() > 18 * 60 && d.as_secs() <= 19 * 60 + 5,
        "应等到厂商说的恢复点，实际 {}s",
        d.as_secs()
    );
}

#[test]
fn test_health_record_and_clear() {
    let health: Arc<Mutex<HashMap<String, CandidateHealth>>> = Arc::new(Mutex::new(HashMap::new()));
    let key = "p::m";
    assert!(cooldown_remaining_secs(&health, key).is_none());
    let d = record_failure(&health, key, FailureClass::RateLimit, Some(90), None);
    assert_eq!(d.as_secs(), 90);
    let remaining = cooldown_remaining_secs(&health, key).unwrap_or(0);
    assert!(
        remaining > 0 && remaining <= 90,
        "剩余冷却异常: {}",
        remaining
    );
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

/// 漏判回归：userinfo / query / fragment 会让端口号解析失败，
/// 于是「真递归」被判成「不是自引用」→ 转发时打到自己。
#[test]
fn self_referential_sees_through_userinfo_and_query() {
    // 以前这些全部返回 false（真递归漏判）
    for url in [
        "http://user:pass@127.0.0.1:15888/v1",
        "http://token@localhost:15888/v1",
        "http://127.0.0.1:15888?x=1",
        "http://127.0.0.1:15888/v1#frag",
        "http://127.0.0.1:15888/?a=b",
    ] {
        assert!(is_self_referential(url, 15888), "应判为自引用：{url}");
    }
}

/// 漏判回归：整个 `127.0.0.0/8` 都指向本机，不只是 `127.0.0.1`。
#[test]
fn self_referential_covers_whole_loopback_block() {
    for url in [
        "http://127.0.0.2:15888/v1",
        "http://127.1:15888/v1",
        "http://127.0.0.1:15888/v1",
        "http://[::1]:15888/v1",
        "http://[::]:15888/v1",
        "http://0.0.0.0:15888/v1",
        "http://app.localhost:15888/v1",
    ] {
        assert!(is_self_referential(url, 15888), "应判为自引用：{url}");
    }
}

/// 反向：userinfo 里的 host 不是本机、或端口不同，都不该误判
#[test]
fn self_referential_does_not_over_trigger() {
    for url in [
        "http://user:pass@api.openai.com:15888/v1",
        "http://127.0.0.1:15889/v1",
        // 端口号在 host 里但不是端口（IPv6 无括号写法）仍按 rfind 处理，不误判为非本机
        "http://192.168.1.5:15888/v1",
    ] {
        assert!(!is_self_referential(url, 15888), "不应判为自引用：{url}");
    }
}

/// 成员可用性：三处（候选视图 / 保存清洗 / 运行期构建）必须看到同一个答案。
/// 以前前端只看 `openai_url`，运行期却按 openai → anthropic → google 取。
#[test]
fn member_availability_matches_running_pick_outbound() {
    let mut p = provider_for_test("agg", "http://127.0.0.1:15888");
    // 只填 anthropic / google 端点
    p.openai_url = String::new();
    p.anthropic_url = "http://127.0.0.1:15888".to_string();
    p.google_url = String::new();

    let avail = member_availability(&p, 15888);
    assert_eq!(avail.outbound, Some(Outbound::Anthropic));
    assert_eq!(avail.unusable_reason, Some(MemberIssue::SelfReferential));
}

#[test]
fn member_availability_reports_missing_key_and_endpoint() {
    let mut no_endpoint = provider_for_test("x", "");
    no_endpoint.openai_url = String::new();
    let avail = member_availability(&no_endpoint, 15888);
    assert_eq!(avail.outbound, None);
    assert_eq!(avail.unusable_reason, Some(MemberIssue::NoEndpoint));

    let mut no_key = provider_for_test("y", "https://api.example.com/v1");
    no_key.api_key = String::new();
    let avail = member_availability(&no_key, 15888);
    assert_eq!(avail.unusable_reason, Some(MemberIssue::NoApiKey));

    let ok = provider_for_test("z", "https://api.example.com/v1");
    assert_eq!(member_availability(&ok, 15888).unusable_reason, None);
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
    assert!(kept
        .iter()
        .all(|c| !is_self_referential(&c.base_url, 15888)));
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
    let (url, auth) =
        build_candidate_url(Anthropic, "https://x.com/anthropic/v1", "m", false, None);
    assert_eq!(url, "https://x.com/anthropic/v1/messages");
    let (url, auth) = build_candidate_url(Google, "https://g.com", "m", true, None);
    assert_eq!(
        url,
        "https://g.com/v1beta/models/m:streamGenerateContent?alt=sse"
    );
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
        build_candidate_url(
            Anthropic,
            "https://x.com/anthropic",
            "m",
            false,
            Some(false)
        )
        .0,
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
    let cache = PrefixSnapshot {
        original: original.clone(),
        forwarded: forwarded.clone(),
    };
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
