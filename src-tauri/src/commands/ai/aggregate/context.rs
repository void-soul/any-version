//! context responsibility.
use super::*;

// ─── Headroom 压缩旁路 + 多轮前缀缓存 ───

#[derive(Clone, Debug)]
pub(super) struct HeadroomRuntime {
    pub(super) base_url: String,
    pub(super) timeout_ms: u64,
    /// true = failClosed（压缩不可用直接报错）；false = failOpen（跳过压缩继续）
    pub(super) fail_closed: bool,
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
pub fn apply_frozen_prefix(
    messages: &[Value],
    cache: Option<&PrefixSnapshot>,
) -> (Vec<Value>, usize) {
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

pub(super) struct AggregateEntry {
    pub(super) port: u16,
    pub(super) handle: tokio::task::AbortHandle,
}

pub(super) static AGGREGATE: OnceLock<Mutex<Option<AggregateEntry>>> = OnceLock::new();

/// 每次请求热更新链路时刷新（供 `get_aggregate_status` 返回真实候选数）。
/// 以前 `candidate_count` 在状态查询里恒为 0、前端也从未渲染 → 有效候选数没有任何来源。
pub(super) static AGG_CANDIDATE_COUNT: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone)]
pub(super) struct AggState {
    pub(super) app: tauri::AppHandle,
    /// 本服务监听端口（用于运行期自引用兜底判定）
    pub(super) port: u16,
    /// 候选健康状态（失败计数 + 冷却截止时间），键 = `provider::model`
    pub(super) health: Arc<Mutex<HashMap<String, CandidateHealth>>>,
    /// 多轮前缀缓存（单会话语义，v1 只保留最近一个会话）
    pub(super) prefix_cache: Arc<Mutex<Option<PrefixSnapshot>>>,
    /// 上次使用的候选签名（用于热更新链路时的变更日志）
    pub(super) chain_signature: Arc<Mutex<Option<String>>>,
}

pub(super) fn running_port() -> Option<u16> {
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
pub(super) struct AggregateLog {
    pub(super) phase: String,
    pub(super) line: String,
    pub(super) level: String,
}

pub(super) fn emit_aggregate_log(app: &tauri::AppHandle, phase: &str, level: &str, line: String) {
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
pub(super) fn is_cjk(ch: char) -> bool {
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
pub(super) fn message_tokens(message: &Value) -> u64 {
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
