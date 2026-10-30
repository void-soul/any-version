//! candidates responsibility.
use super::*;

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
///
/// 解析 authority 时必须剥掉 **userinfo**（`http://user:pass@host:port`）与
/// **query / fragment**（`http://host:port?x=1`）—— 两者都会让端口号解析失败，
/// 于是「真递归」被判成「不是自引用」。反过来，路径前缀（`/v1`）不需要比：
/// 只要 host + 端口对上就是同一个监听器。
pub fn is_self_referential(base_url: &str, aggregate_port: u16) -> bool {
    let raw = base_url.trim();
    if raw.is_empty() {
        return false;
    }
    let without_scheme = match raw.split_once("://") {
        Some((_scheme, rest)) => rest,
        None => raw,
    };
    // query / fragment 之后的部分不属于 authority
    let authority = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .trim();
    if authority.is_empty() {
        return false;
    }
    // userinfo：最后一个 `@` 之后才是 host[:port]（密码里可能有 `@`，故用 rfind）
    let host_port = match authority.rfind('@') {
        Some(idx) => &authority[idx + 1..],
        None => authority,
    };
    let (host, port_part) = match host_port.rfind(':') {
        Some(idx) => (&host_port[..idx], Some(&host_port[idx + 1..])),
        None => (host_port, None),
    };
    let host = host
        .trim()
        .trim_matches('[')
        .trim_matches(']')
        .to_ascii_lowercase();
    let port_matches = port_part
        .and_then(|p| p.parse::<u16>().ok())
        .map(|p| p == aggregate_port)
        .unwrap_or(false);
    if !port_matches {
        return false;
    }
    is_local_host(&host)
}

/// host 是否指向本机。**端口相等是前提**（调用方已判），这里只判主机。
///
/// 覆盖：`localhost` 及其子域、`::1` / `[::]` / IPv6 回环、`0.0.0.0`、
/// 以及整个 `127.0.0.0/8`（`127.0.0.2`、`127.1` 这类写法都指向本机）。
/// 以前只认 4 个字面量，`127.0.0.0/8` 的其余地址会漏判成「不是自引用」→ 真递归。
pub(super) fn is_local_host(host: &str) -> bool {
    if host.is_empty() {
        return false;
    }
    if host == "localhost" || host.ends_with(".localhost") {
        return true;
    }
    // IPv6 回环：::1 / :: / 0:0:0:0:0:0:0:1
    let ipv6 = host.trim_start_matches("::ffff:");
    if ipv6 == "::1" || ipv6 == "::" {
        return true;
    }
    if host == "0.0.0.0" {
        return true;
    }
    // IPv4：解析 dotted-quad，判 A 段是不是 127
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() == 4 && parts.iter().all(|p| p.parse::<u8>().is_ok()) {
        return parts[0] == "127";
    }
    // 短写法：`a.b` = `a.0.0.b`（所以 `127.1` 就是 `127.0.0.1`）、`a.b.c` = `a.0.b.c`。
    // A 段是 127 就够，不必看其余段。
    if (2..=3).contains(&parts.len()) && parts.iter().all(|p| p.parse::<u8>().is_ok()) {
        return parts[0] == "127";
    }
    false
}

/// 一个供应商作为「聚合链成员」时的可用性判定结果。
///
/// **这是成员可用性的唯一真相源**：`route.rs` 的候选视图、保存时的清洗，
/// 以及 `build_candidates` 的构建期过滤都读它。以前三处各判各的：
/// 前两处只看 `openai_url`，运行期却按 openai → anthropic → google 取，
/// 于是「只填了 `anthropic_url`」的供应商前端不标红、保存不剔除、入运行时被静默丢弃 ——
/// 用户以为它在链里。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberAvailability {
    /// 运行期实际会用到的出站协议（`None` = 三种端点都没填）。
    pub outbound: Option<Outbound>,
    /// 运行期实际会用到的 base_url（已 trim 掉尾部 `/`）。
    pub base_url: String,
    /// 不可用原因（可直接展示给用户）；`None` = 可用。
    pub unusable_reason: Option<MemberIssue>,
}

/// 成员不可用的原因。`Copy` + `Display` 便于前端直接拿到文案。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberIssue {
    /// 三种协议端点一个都没填
    NoEndpoint,
    /// 端点有，但没填 API Key
    NoApiKey,
    /// 端点指向聚合服务自身（会递归）
    SelfReferential,
}

impl MemberIssue {
    pub fn message(self) -> &'static str {
        match self {
            MemberIssue::NoEndpoint => {
                "未配置任何端点（openai_url / anthropic_url / google_url 全为空）"
            }
            MemberIssue::NoApiKey => "缺少 API Key",
            MemberIssue::SelfReferential => "指向聚合服务自身，会造成请求递归",
        }
    }
}

/// 判定一个供应商能否作为聚合链成员。
///
/// 与 [`pick_outbound`] 的取端点优先级严格一致（openai → anthropic → google），
/// 这是本函数存在的全部意义：**三处调用点必须看到同一个答案**。
pub fn member_availability(provider: &AiProvider, aggregate_port: u16) -> MemberAvailability {
    let picked = pick_outbound(provider);
    let base = picked
        .as_ref()
        .map(|(_, b, _)| b.trim().trim_end_matches('/').to_string())
        .unwrap_or_default();
    let issue = if picked.is_none() {
        Some(MemberIssue::NoEndpoint)
    } else if base.is_empty() || provider.api_key.trim().is_empty() {
        Some(MemberIssue::NoApiKey)
    } else if is_self_referential(&base, aggregate_port) {
        Some(MemberIssue::SelfReferential)
    } else {
        None
    };
    MemberAvailability {
        outbound: picked.map(|(o, _, _)| o),
        base_url: base,
        unusable_reason: issue,
    }
}

// ─── 候选与出站协议 ───

/// 候选出口协议（按供应商配置的 URL 决定）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Outbound {
    OpenAi,
    Anthropic,
    Google,
}

impl Outbound {
    /// 转成 `proxy::upstream::resolve_url` 用的协议字符串。
    pub(super) fn as_str(&self) -> &'static str {
        match self {
            Outbound::OpenAi => "openai",
            Outbound::Anthropic => "anthropic",
            Outbound::Google => "google",
        }
    }
}

/// 单个候选（每次请求实时解析）：上游端点与凭据。
#[derive(Clone, Debug)]
pub(super) struct AggCandidate {
    pub(super) provider_id: String,
    pub(super) provider_name: String,
    pub(super) outbound: Outbound,
    pub(super) base_url: String,
    pub(super) api_key: String,
    pub(super) model_id: String,
    /// 该候选拼接时是否补 `/v1`：None = 自动（URL 结尾已是 /v1 就不补）。
    pub(super) include_v1: Option<bool>,
    /// 该供应商的自定义上游请求头（未配置则为空）。以前聚合转发完全忽略这个字段，
    /// 导致需要 X-Request-Id / 厂商标识的网关在聚合路径被拒。
    pub(super) headers: Vec<crate::proxy::types::UpstreamHeader>,
    /// 用户在聚合链上显式声明的「该模型是否支持图片」（None = 交给注册表启发式）。
    pub(super) supports_image: Option<bool>,
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
pub(super) fn candidate_proxy_config(candidate: &AggCandidate, cfg: &AiConfig) -> ProxyConfig {
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
pub(super) fn pick_outbound(provider: &AiProvider) -> Option<(Outbound, String, Option<bool>)> {
    let openai = provider.openai_url.trim().to_string();
    let anthropic = provider.anthropic_url.trim().to_string();
    let google = provider.google_url.trim().to_string();
    if !openai.is_empty() {
        return Some((Outbound::OpenAi, openai, provider.openai_include_v1));
    }
    if !anthropic.is_empty() {
        return Some((
            Outbound::Anthropic,
            anthropic,
            provider.anthropic_include_v1,
        ));
    }
    if !google.is_empty() {
        return Some((Outbound::Google, google, None));
    }
    None
}

/// 构建候选快照：把链路里的 (provider_id, model_id) 解析成可用的上游端点。
/// 自引用（指向聚合服务自身）的候选直接剔除，避免请求递归。
pub(super) fn build_candidates(
    providers: &[AiProvider],
    chain: &[RouteCandidate],
    aggregate_port: u16,
) -> Vec<AggCandidate> {
    let mut out = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for candidate in chain {
        let Some(provider) = providers.iter().find(|p| p.id == candidate.provider_id) else {
            skipped.push(format!("{}: 供应商已不存在", candidate.provider_id));
            continue;
        };
        // 可用性判定只有一个真相源（与前端候选视图、保存清洗完全同一套口径）
        let avail = member_availability(provider, aggregate_port);
        let Some(outbound) = avail.outbound else {
            skipped.push(format!(
                "{}: {}",
                provider.name_or_id(),
                avail
                    .unusable_reason
                    .map(|r| r.message())
                    .unwrap_or("不可用")
            ));
            continue;
        };
        let base = avail.base_url.clone();
        let include_v1 = pick_outbound(provider).map(|(_, _, v)| v).unwrap_or(None);
        out.push(AggCandidate {
            provider_id: provider.id.clone(),
            provider_name: provider.name_or_id().to_string(),
            outbound,
            base_url: base,
            api_key: provider.api_key.clone(),
            model_id: candidate.model_id.clone(),
            include_v1,
            headers: crate::proxy::headers::normalize(&provider.custom_headers),
            supports_image: candidate.supports_image,
        });
    }
    // 静默丢弃是这条链路上最难排查的问题：用户看到「模型在链里」，实际根本没进候选。
    // 把被丢掉的成员连同原因打出来，至少能从日志定位到是哪几个、为什么。
    if !skipped.is_empty() {
        eprintln!(
            "[aggregate] 构建期剔除 {} 个候选: {}",
            skipped.len(),
            skipped.join("；")
        );
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
pub(super) fn prioritize_by_model(
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
