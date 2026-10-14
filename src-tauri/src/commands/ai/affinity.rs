//! 会话亲和（保prompt cache）。
//!
//! 抄 magpie `internal/gateway/affinity.go` 的**核心**：一个对话留在**上次回答它的那个
//! 候选**上，只要厂商的prompt cache 还值得保留。我们聚合层此前无状态，换家就丢 cache
//! 命中率（缓存前缀按「模型 + 前缀」命中，换供应商等于换前缀池）。
//!
//! 只抄核心的 1/4：`strict` / `sealsRefused`（Codex 换账号要剥加密推理）/ 加权轮询 nudge
//! / WSL 地址处理全都没抄。magpie 那部分每项各 60~100 行且互相耦合，性价比低。
//!
//! ## 会话键怎么来的（抄 `gateway.go:5548 conversationID` 的骨架，但改了两处）
//!
//! 1. **请求头优先** —— 客户端自己报的会话标识最可靠；
//! 2. 头都没有 → body 里**第一条 user 消息的纯文本**哈希；
//! 3. 都没有 → 整份body 哈希。
//!
//! **相对 magpie 改了两处（都是它的设计缺陷）**：
//! - 它哈希**原始 JSON 字节**，首条 user 消息里任何一个字节变化（`<system-reminder>`
//!   插入、环境上下文、时间戳、字段顺序、空白）就换一个键 → 亲和静默失效。
//!   我们**先抽纯文本、再剥逐轮变化的注入块**，按用户真实说的话哈希。
//! - 它的 `X-Magpie-Session`（自有头）不参与会话键，和 `sessionOf()` 不一致；
//!   我们允许调用方用 `x-anyversion-session` 显式覆盖。
//!
//! ## 状态存哪
//!
//! 纯内存 + TTL + 条数上限（默认 512 条 / 24h，与 magpie 同值）。
//! **不落盘**：我们只有服务模式一个进程，没有 CLI/服务双进程共享亲和的需求；
//! 落盘只会带来「上次退出前的粘性影响新会话」的怪异行为。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde_json::Value;

/// 当前 Unix 毫秒（记录/淘汰用；抽成函数便于测试注入固定时钟）。
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 上次读到的 cache 命中token 数低于这个值，就认为「这个候选的 cache 不值钱」，不粘。
///抄 magpie `affinity.go:42 cacheWorth = 1024`。
const CACHE_WORTH_TOKENS: u64 = 1024;

/// 距上次回答超过这么久，就认为对话已经结束，不粘。抄 magpie `affinity.go:45 cacheCold = 5min`。
const COLD_AFTER_MS: u64 = 5 * 60 * 1000;

/// 记录条数上限（防内存无界增长）。抄 magpie `stickKeep = 512`。
const MAX_STICKS: usize = 512;

/// 记录最长保留时间。抄 magpie的 24h TTL。
const STICK_TTL_MS: u64 = 24 * 60 * 60 * 1000;

/// 一条亲和记录。
#[derive(Debug, Clone)]
pub struct Stick {
    /// 上次回答用的候选，键 = `provider_id::model_id`（与冷却健康表同键）。
    pub who: String,
    /// 上次回答时读到的 cache 命中 token 数。`None` = **拿不到证据**（流式路径的
    /// usage 未必回传全）。**拿不到就不该阻断亲和** —— 不能因为「没证据」就判定
    /// 「cache 不值钱」而永不粘连，那会让整个功能在主力场景（Claude Code 流式）里失效。
    pub cache_read: Option<u64>,
    /// 记录时刻（Unix 毫秒）。
    pub at_ms: u64,
}

static STICKS: OnceLock<Mutex<HashMap<String, Stick>>> = OnceLock::new();

fn sticks() -> &'static Mutex<HashMap<String, Stick>> {
    STICKS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 客户端报会话标识的请求头，按可靠性排序。
///
/// magpie 的 6 个头里 `x-session-affinity` 是它自己写进 OpenCode/omp 配置的，
/// 对我们无意义，故不收。
const SESSION_HEADERS: &[&str] = &[
    // Claude Code 走 `/v1/messages` 时必发 —— 我们最可靠的路径
    "x-claude-code-session-id",
    // Codex 走 `/responses` 时发
    "session_id",
    "session-id",
    // 通用
    "x-session-id",
    "x-opencode-session",
    // 调用方显式覆盖（magpie 缺这一条：它的自有头不参与会话键）
    "x-anyversion-session",
];

/// 构造会话键。入参按顺序尝试，拿到第一个非空就返回 `Header(...)`。
///
/// 拿不到头就用 body 的纯文本兜底；再拿不到就哈希整份 body。
pub fn conversation_id(headers: &[(&str, &str)], body: &Value, inbound: &str) -> String {
    for name in SESSION_HEADERS {
        if let Some((_, v)) = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)) {
            let v = v.trim();
            if !v.is_empty() {
                return format!("header:{}:{v}", name.to_ascii_lowercase());
            }
        }
    }
    match first_user_text(body, inbound) {
        Some(text) if !text.trim().is_empty() => format!("text:{}", short_hash(text.trim())),
        _ => format!("body:{}", short_hash(&body.to_string())),
    }
}

/// 抽第一条 user 消息的**纯文本**，并剥掉逐轮变化的注入块。
///
/// 抽纯文本而不是哈希原始 JSON：Claude Code 每轮都会往首条消息里塞
/// `<system-reminder>`、环境上下文、当前时间 —— 原始字节哈希每轮都变。
fn first_user_text(body: &Value, inbound: &str) -> Option<String> {
    let items: Vec<Value> = match inbound {
        // Responses 形态：input 数组
        "responses" => body
            .get("input")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default(),
        // Gemini 形态：contents，且 role 可省略（默认 user）
        "google" => body
            .get("contents")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default(),
        // Chat / Messages：messages（Claude Code 入站是 anthropic，但字段名相同）
        _ => body
            .get("messages")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default(),
    };
    for item in items {
        let role = item.get("role").and_then(|v| v.as_str()).unwrap_or("user");
        if role != "user" {
            continue;
        }
        // Anthropic / OpenAI 用 `content`；Gemini 用 `parts`（且 role 常省略）
        let text = match item.get("content") {
            Some(c) => content_text(Some(c)),
            None => content_text(item.get("parts")),
        };
        if !text.trim().is_empty() {
            return Some(strip_volatile(&text));
        }
    }
    None
}

/// 把一条消息的 content 抽成纯文本（覆盖 string / 文本块数组 / Gemini parts）。
fn content_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => {
            let mut out = String::new();
            for b in blocks {
                match b {
                    Value::String(s) => out.push_str(s),
                    Value::Object(_) => {
                        let t = b.get("text").and_then(|v| v.as_str()).unwrap_or("");
                        out.push_str(t);
                    }
                    _ => {}
                }
            }
            out
        }
        Some(Value::Object(o)) => o
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        _ => String::new(),
    }
}

/// 剥掉逐轮变化、由客户端注入的块 —— 这些不该参与会话身份。
///
/// 抄 magpie `rules.go:117-134 ruleKey` 的思路（按用户真实文字），但magpie 是为了
/// 意图路由的规则匹配，我们是会话键。形态上参考它 `firstWords` 忽略的前缀。
fn strip_volatile(text: &str) -> String {
    static VOLATILE_TAGS: &[&str] = &[
        "system-reminder",
        "environment_context",
        "user-context",
        "command-name",
        "local-command-stdout",
    ];
    let mut out = text.to_string();
    for tag in VOLATILE_TAGS {
        out = strip_tag_block(&out, tag);
    }
    out
}

/// 去掉 `<tag>…</tag>` 与自闭合 `<tag/>` 的整块内容。
fn strip_tag_block(text: &str, tag: &str) -> String {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let Some(start) = rest.find(&open) else {
            out.push_str(rest);
            return out;
        };
        // `</` 开头说明已经过了同名标签，避免把闭合标签当成开头
        if rest[start..].starts_with(&close) {
            out.push_str(rest);
            return out;
        }
        out.push_str(&rest[..start]);
        let after = &rest[start + open.len()..];
        // 自闭合
        if after.trim_start().starts_with("/>") {
            let skip = after.find("/>").map(|i| i + 2).unwrap_or(after.len());
            rest = &after[skip..];
            continue;
        }
        match after.find(&close) {
            Some(i) => rest = &after[i + close.len()..],
            None => return out, // 没有闭合：整体丢弃更安全
        }
    }
}

/// 短哈希（16 个十六进制字符 = 64bit），够区分会话又不会把日志撑长。
fn short_hash(text: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in text.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// 记一条亲和（**只有成功回答后才记**，抄 magpie `answered:517` 的时机）。
pub fn record(key: &str, who: &str, cache_read: Option<u64>, now_ms: u64) {
    let Ok(mut guard) = sticks().lock() else { return };
    guard.insert(key.to_string(), Stick { who: who.to_string(), cache_read, at_ms: now_ms });
    // 顺手淘汰：过期的 + 超量的
    guard.retain(|_, s| now_ms.saturating_sub(s.at_ms) < STICK_TTL_MS);
    while guard.len() > MAX_STICKS {
        // HashMap 无序，删掉最旧的一条（O(n) 但只在超量时发生，且 n ≤ 513）
        if let Some(oldest) = guard
            .iter()
            .min_by_key(|(_, s)| s.at_ms)
            .map(|(k, _)| k.clone())
        {
            guard.remove(&oldest);
        } else {
            break;
        }
    }
}

/// 读一条亲和（顺带做两项过期检查：超过 TTL 直接视为没有）。
pub fn get(key: &str, now_ms: u64) -> Option<Stick> {
    let guard = sticks().lock().ok()?;
    let s = guard.get(key)?;
    if now_ms.saturating_sub(s.at_ms) >= STICK_TTL_MS {
        return None;
    }
    Some(s.clone())
}

/// 删一条亲和（回复已发出又中断 → 删掉，让 agent 重试走正常路由而不是回到刚失败的那个）。
/// 抄 magpie `unanswered:547`。
pub fn forget(key: &str) {
    if let Ok(mut guard) = sticks().lock() {
        guard.remove(key);
    }
}

/// 为什么粘 / 不粘 —— 与 magpie 的 `Affinity.Why` 同构，用于日志与排障。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AffinityDecision {
    /// 从没记录过（首轮）
    First,
    /// 上次答的那个候选已经不在链路里
    Gone,
    /// 它正在冷却/ 额度用尽
    Resting,
    /// cache 命中太少，粘着没意义
    NotWorth,
    /// 距上次太久，对话大概结束了
    Cold,
    /// 粘住
    Kept,
}

impl AffinityDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            AffinityDecision::First => "first",
            AffinityDecision::Gone => "gone",
            AffinityDecision::Resting => "resting",
            AffinityDecision::NotWorth => "no-cache",
            AffinityDecision::Cold => "cold",
            AffinityDecision::Kept => "kept",
        }
    }
}

/// 决定这一轮要不要把上次答的那个候选提到最前。
///
/// `candidate_keys` 是本轮候选的键列表（`provider_id::model_id`）。
/// 决策链抄 magpie `affinity.go:366-396` 的 `auto` 分支，砍掉 strict/turn/rotate 之后剩下的。
pub fn decide(
    stick: Option<&Stick>,
    candidate_keys: &[String],
    is_resting: &dyn Fn(&str) -> bool,
    now_ms: u64,
) -> AffinityDecision {
    let Some(s) = stick else {
        return AffinityDecision::First;
    };
    if !candidate_keys.iter().any(|k| k == &s.who) {
        return AffinityDecision::Gone;
    }
    if is_resting(&s.who) {
        return AffinityDecision::Resting;
    }
    if s.cache_read.is_some_and(|n| n < CACHE_WORTH_TOKENS) {
        return AffinityDecision::NotWorth;
    }
    if now_ms.saturating_sub(s.at_ms) > COLD_AFTER_MS {
        return AffinityDecision::Cold;
    }
    AffinityDecision::Kept
}

/// 把 `who` 对应的候选提到最前（其余保持原顺序）。
///
/// 只在 [`decide`] 返回 [`AffinityDecision::Kept`] 时调用。
pub fn promote<T, F: Fn(&T) -> String>(candidates: &mut Vec<T>, who: &str, key_of: F) -> bool {
    let Some(pos) = candidates.iter().position(|c| key_of(c) == who) else {
        return false;
    };
    if pos == 0 {
        return true;
    }
    let picked = candidates.remove(pos);
    candidates.insert(0, picked);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn headers<'a>(pairs: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
        pairs.iter().map(|(k, v)| (*k, *v)).collect()
    }

    #[test]
    fn session_header_wins() {
        let body = json!({"messages":[{"role":"user","content":"hi"}]});
        let id = conversation_id(
            &headers(&[("x-claude-code-session-id", "abc-123")]),
            &body,
            "anthropic",
        );
        assert_eq!(id, "header:x-claude-code-session-id:abc-123");
    }

    #[test]
    fn explicit_override_header_is_accepted() {
        let body = json!({"messages":[]});
        let id = conversation_id(
            &headers(&[("x-anyversion-session", "my-own")]),
            &body,
            "anthropic",
        );
        assert_eq!(id, "header:x-anyversion-session:my-own");
    }

    /// 没有头时用第一条 user 消息的纯文本 —— 且必须**跨轮稳定**
    /// （这是 magpie哈希原始 JSON 会失效的地方）。
    #[test]
    fn body_fallback_is_stable_across_turns() {
        let turn1 = json!({"messages":[
            {"role":"system","content":"you are helpful"},
            {"role":"user","content":"帮我看下这个 bug"},
        ]});
        let turn2 = json!({"messages":[
            {"role":"system","content":"you are helpful"},
            // 客户端往首条里塞了 system-reminder + 环境上下文 + 时间
            {"role":"user","content":"<system-reminder>claude says: 2026-10-14</system-reminder><environment_context>cwd: D:\\x</environment_context>帮我看下这个 bug"},
            {"role":"assistant","content":"好的"},
            {"role":"user","content":"那修它"},
        ]});
        let a = conversation_id(&[], &turn1, "anthropic");
        let b = conversation_id(&[], &turn2, "anthropic");
        assert_eq!(a, b, "逐轮注入的块必须被剥掉，否则亲和静默失效");
    }

    #[test]
    fn body_fallback_differs_for_different_conversations() {
        let a = conversation_id(&[], &json!({"messages":[{"role":"user","content":"问题 A"}]}), "anthropic");
        let b = conversation_id(&[], &json!({"messages":[{"role":"user","content":"问题 B"}]}), "anthropic");
        assert_ne!(a, b);
    }

    /// 没user 消息时退到整 body 哈希，仍要稳定且能区分
    #[test]
    fn body_fallback_uses_whole_body_when_no_user_message() {
        let b = json!({"messages":[{"role":"assistant","content":"only assistant"}],"seed":7});
        assert_eq!(
            conversation_id(&[], &b, "anthropic"),
            conversation_id(&[], &b, "anthropic")
        );
        let c = json!({"messages":[{"role":"assistant","content":"only assistant"}],"seed":8});
        assert_ne!(
            conversation_id(&[], &b, "anthropic"),
            conversation_id(&[], &c, "anthropic")
        );
    }

    #[test]
    fn responses_and_google_entrypoints() {
        let r = json!({"input":[{"role":"user","content":[{"type":"input_text","text":"问题A"}]}]});
        let g = json!({"contents":[{"parts":[{"text":"问题A"}]}]});
        assert!(conversation_id(&[], &r, "responses").starts_with("text:"));
        assert!(conversation_id(&[], &g, "google").starts_with("text:"));
        assert_eq!(
            conversation_id(&[], &r, "responses"),
            conversation_id(&[], &g, "google")
        );
    }

    #[test]
    fn strip_removes_self_closing_and_nested_ish_blocks() {
        assert_eq!(strip_tag_block("a<system-reminder>x</system-reminder>b", "system-reminder"), "ab");
        assert_eq!(strip_tag_block("a<command-name/>b", "command-name"), "ab");
        // 没有闭合 → 整体丢弃，不留半截
        assert_eq!(strip_tag_block("a<system-reminder>xyz", "system-reminder"), "a");
    }

    #[test]
    fn decide_walks_the_chain() {
        let now = 1_000_000u64;
        let keys = vec!["a::m1".to_string(), "b::m2".to_string()];
        let never = |_: &str| false;
        let always = |_: &str| true;

        // 首轮
        assert_eq!(decide(None, &keys, &never, now), AffinityDecision::First);
        // 候选没了
        let s = Stick { who: "zz::m".into(), cache_read: Some(9000), at_ms: now };
        assert_eq!(decide(Some(&s), &keys, &never, now), AffinityDecision::Gone);
        // 正在冷却
        let s = Stick { who: "a::m1".into(), cache_read: Some(9000), at_ms: now };
        assert_eq!(decide(Some(&s), &keys, &always, now), AffinityDecision::Resting);
        // cache 命中太少，粘着没意义
        let s = Stick { who: "a::m1".into(), cache_read: Some(10), at_ms: now };
        assert_eq!(decide(Some(&s), &keys, &never, now), AffinityDecision::NotWorth);
        // 太久之前 → 对话大概结束了
        let s = Stick { who: "a::m1".into(), cache_read: Some(9000), at_ms: now - 6 * 60 * 1000 };
        assert_eq!(decide(Some(&s), &keys, &never, now), AffinityDecision::Cold);
        // 粘住
        let s = Stick { who: "a::m1".into(), cache_read: Some(9000), at_ms: now };
        assert_eq!(decide(Some(&s), &keys, &never, now), AffinityDecision::Kept);
        // **拿不到 cache 证据 → 照样粘**（不能因「没证据」判定 cache 不值钱，
        //   否则流式主力场景下亲和永不生效）
        let s = Stick { who: "a::m1".into(), cache_read: None, at_ms: now };
        assert_eq!(decide(Some(&s), &keys, &never, now), AffinityDecision::Kept);
    }

    #[test]
    fn promote_moves_the_sticky_candidate_to_front() {
        let key = |c: &(String, String)| format!("{}::{}", c.0, c.1);
        let mut list = vec![
            ("a".to_string(), "m1".to_string()),
            ("b".to_string(), "m2".to_string()),
            ("c".to_string(), "m3".to_string()),
        ];
        assert!(promote(&mut list, "c::m3", &key));
        assert_eq!(list[0].0, "c");
        assert_eq!(list[1].0, "a", "其余保持原顺序");
        // 已在首位 → no-op 但返回 true
        assert!(promote(&mut list, "c::m3", &key));
        assert_eq!(list[0].0, "c");
        // 不在列表里 → false
        assert!(!promote(&mut list, "zz::m", &key));
    }

    #[test]
    fn record_evicts_and_expires() {
        let key = format!("test-evict-{}", short_hash(&now_ms().to_string()));
        forget(&key);
        let now = now_ms();
        // 条数上限：塞 MAX_STICKS+10 条，只剩 ≤ MAX_STICKS
        for i in 0..(MAX_STICKS + 10) {
            record(&format!("{key}-{i}"), "a::m", Some(9999), now + i as u64);
        }
        let guard = sticks().lock().unwrap();
        assert!(guard.len() <= MAX_STICKS, "超量后应淘汰，实际 {}", guard.len());
        drop(guard);
        // TTL 过期后读不到。
        // 时间戳要比上面那批都新，否则这条会是最旧的、被淘汰逻辑正常淘汰掉
        //（淘汰「最旧」是正确的行为，不是 bug）。
        let k = format!("{key}-ttl");
        let fresh = now + (MAX_STICKS + 100) as u64;
        record(&k, "a::m", Some(9999), fresh);
        assert!(get(&k, fresh + STICK_TTL_MS + 1).is_none());
        assert!(get(&k, fresh).is_some());
        forget(&k);
    }
}