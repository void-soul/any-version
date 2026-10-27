//! 发往 WorkBuddy 上游的**会话头族**。
//!
//! 背景：后端会按「会话」聚合与灰度（prompt cache、风控、用量归属），而它认的是
//! 请求头而不是 body 里的字段：
//! - 客户端自己带 `conversationId` 却只放 body 里 → 上游读不到；
//! - 干脆不发 → 每个请求在它眼里都是一次性匿名会话。参考实现的原话是
//!   「旧行为会把被灰度的账号判死刑」，也就是说**缺头本身可能就是 `11-128` 的成因**。
//!
//! 只补头、**不往 body 塞 `conversationId`**：参考侧做过剥离实测，校验对象是头族。
//!
//! 契约来源：Go 版 workbuddy2api 的 `conversation-id-header-family` 补丁。
//! 逐条语义见 `.codebuddy/skills/cockpit-buddy-sync/references/workbuddy-manager.md` §D.3。
//!
//! 值的两个硬约束（照参考侧）：
//! 1. **派生值**必须是严格 32 位小写 hex（上游按形态校验）；
//! 2. **客户端透传值**不做形态校验（`client-conv-1` 这种原样发出去）。
//!
//! 不发 `X-Root-Request-ID`：Go 侧补丁里没有它的依据，旁证仓（Python hub）把它取成
//! 「等于轮级值」—— 少发一个比发一个猜出来的值强。

use axum::http::HeaderMap;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::types::UpstreamHeader;

pub const HEADER_CONVERSATION_ID: &str = "X-Conversation-ID";
pub const HEADER_REQUEST_ID: &str = "X-Conversation-Request-ID";
pub const HEADER_MESSAGE_ID: &str = "X-Conversation-Message-ID";
pub const HEADER_TRACE_ID: &str = "X-Trace-ID";

/// 一次客户端请求的会话标识。
///
/// 会话级 + 轮级在请求进来时算一次，之后**所有出站尝试共用**（首发 / 整流重试 /
/// 协议回退 / 渠道校验换路径）；消息级与 trace 每次尝试新生成。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionIds {
    conversation_id: String,
    request_id: String,
}

impl SessionIds {
    /// 从**入站原始 body** 与入站头推导。
    ///
    /// 必须在 body 被改写（模型伪装、协议转换、折叠 system）**之前**调用：
    /// 轮级锚点是「最后一条 user 消息的序号 + 文本」，改写后就变了。
    pub fn from_request(body: &Value, inbound: &HeaderMap) -> Self {
        Self {
            conversation_id: resolve_conversation_id(body),
            request_id: resolve_request_id(body, inbound),
        }
    }

    pub fn conversation_id(&self) -> &str {
        &self.conversation_id
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// 本次出站尝试要带的头。**调用一次就新生成一次 message / trace**。
    pub fn attempt_headers(&self) -> Vec<UpstreamHeader> {
        vec![
            header(HEADER_CONVERSATION_ID, self.conversation_id.clone()),
            header(HEADER_REQUEST_ID, self.request_id.clone()),
            header(HEADER_MESSAGE_ID, random_hex32()),
            header(HEADER_TRACE_ID, random_hex32()),
        ]
    }
}

fn header(key: &str, value: String) -> UpstreamHeader {
    UpstreamHeader {
        key: key.to_string(),
        value,
    }
}

/// 会话级 ID：客户端给什么就用什么 → prompt cache 键 → 首条 user 消息 → 随机。
fn resolve_conversation_id(body: &Value) -> String {
    if let Some(v) = client_conversation(body) {
        return v;
    }
    // 部分客户端（pi-ai 系）把会话标识塞在 OpenAI 的 prompt cache 字段里
    if let Some(key) = body
        .get("prompt_cache_key")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return hex32(&format!("conv:{key}"));
    }
    // 首条 user 文本：会话内追加历史不影响它，开新会话自然换值
    if let Some(first) = user_texts(body).into_iter().next() {
        let first = first.trim();
        if !first.is_empty() {
            return hex32(&format!("conv:{first}"));
        }
    }
    // 什么都没有：随机值只为满足「头必须非空且形态合法」
    random_hex32()
}

/// 客户端显式给的会话 ID：四个位置，**原值透传、不改形态**。
fn client_conversation(body: &Value) -> Option<String> {
    const KEYS: [&str; 2] = ["conversationId", "conversation_id"];
    let pick = |obj: &Value| -> Option<String> {
        for k in KEYS {
            if let Some(v) = obj.get(k).and_then(|v| v.as_str()) {
                let v = v.trim();
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
        None
    };
    if let Some(v) = pick(body) {
        return Some(v);
    }
    body.get("metadata").and_then(pick)
}

/// 轮级 ID：入站头透传 → 最后一条 user 消息派生 → 随机。
fn resolve_request_id(body: &Value, inbound: &HeaderMap) -> String {
    if let Some(v) = inbound
        .get("x-conversation-request-id")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return v.to_string();
    }
    if let Some(turn) = turn_key(body) {
        return hex32(&format!("turn:{turn}"));
    }
    random_hex32()
}

/// 轮级锚点 = 最后一条 user 消息的「序号 + 文本」。
fn turn_key(body: &Value) -> Option<String> {
    let (idx, text) = user_texts_indexed(body).pop()?;
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(format!("{idx}:{text}"))
}

fn user_texts(body: &Value) -> Vec<String> {
    user_texts_indexed(body)
        .into_iter()
        .map(|(_, text)| text)
        .collect()
}

/// 按顺序取所有 user 消息的文本（带原始下标，供轮级锚点用）。
///
/// 兼容三种入站形态：OpenAI / Anthropic 的 `messages`，Google 的 `contents`。
/// Responses 入站在 `responses_handler` 里已经转成 Chat 形态，到这里总是 `messages`。
fn user_texts_indexed(body: &Value) -> Vec<(usize, String)> {
    if let Some(msgs) = body.get("messages").and_then(|m| m.as_array()) {
        return msgs
            .iter()
            .enumerate()
            .filter(|(_, m)| m.get("role").and_then(|r| r.as_str()) == Some("user"))
            .filter_map(|(i, m)| content_text(m.get("content")).map(|t| (i, t)))
            .collect();
    }
    if let Some(contents) = body.get("contents").and_then(|c| c.as_array()) {
        return contents
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                c.get("role")
                    .and_then(|r| r.as_str())
                    .is_some_and(|r| r.eq_ignore_ascii_case("user"))
            })
            .filter_map(|(i, c)| {
                let parts = c.get("parts")?.as_array()?;
                let text = parts
                    .iter()
                    .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n");
                if text.trim().is_empty() {
                    None
                } else {
                    Some((i, text))
                }
            })
            .collect();
    }
    Vec::new()
}

fn content_text(content: Option<&Value>) -> Option<String> {
    match content? {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => Some(
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        _ => None,
    }
}

/// `sha256(key)` 取前 16 字节 → 32 位小写 hex。
fn hex32(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    let digest = hasher.finalize();
    to_hex(&digest[..16])
}

/// 请求级随机值（32 位小写 hex）。
///
/// 不追求密码学强度：只需要「同一进程内不重复」。熵来自高精度时间 + 进程内自增计数，
/// 二者组合足以避开同一毫秒内的并发碰撞。
fn random_hex32() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    hex32(&format!("rand:{nanos}:{seq}"))
}

fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};
    use serde_json::json;

    fn is_hex32(s: &str) -> bool {
        s.len() == 32 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }

    fn conv_of(body: Value) -> String {
        SessionIds::from_request(&body, &HeaderMap::new()).conversation_id().to_string()
    }

    #[test]
    fn derived_ids_are_strict_lower_hex_32() {
        let id = conv_of(json!({"messages": [{"role": "user", "content": "hello"}]}));
        assert!(is_hex32(&id), "{id}");
        assert_eq!(random_hex32().len(), 32);
        assert!(is_hex32(&random_hex32()));
    }

    #[test]
    fn client_conversation_passes_through_untouched() {
        // 透传值**不做形态校验**：含连字符、非 hex 也原样发
        for body in [
            json!({"conversationId": "client-conv-1"}),
            json!({"conversation_id": "client-conv-1"}),
            json!({"metadata": {"conversationId": "client-conv-1"}}),
            json!({"metadata": {"conversation_id": "client-conv-1"}}),
        ] {
            assert_eq!(conv_of(body), "client-conv-1");
        }
    }

    #[test]
    fn conversation_is_stable_across_turns() {
        // 首条 user 相同、末尾追加新一轮 → 会话 ID 必须相等
        let first = json!({"messages": [
            {"role": "user", "content": "第一节"},
            {"role": "assistant", "content": "好的"},
            {"role": "user", "content": "第二节"},
        ]});
        let second = json!({"messages": [
            {"role": "user", "content": "第一节"},
            {"role": "assistant", "content": "好的"},
            {"role": "user", "content": "第二节"},
            {"role": "assistant", "content": "好的"},
            {"role": "user", "content": "第三节"},
        ]});
        assert_eq!(conv_of(first), conv_of(second));
    }

    #[test]
    fn different_conversations_get_different_ids() {
        let a = conv_of(json!({"messages": [{"role": "user", "content": "会话 A"}]}));
        let b = conv_of(json!({"messages": [{"role": "user", "content": "会话 B"}]}));
        assert_ne!(a, b);
    }

    #[test]
    fn prompt_cache_key_is_used_before_first_message() {
        let with_cache =
            conv_of(json!({"prompt_cache_key": "sess-42", "messages": [{"role": "user", "content": "hi"}]}));
        let direct = hex32("conv:sess-42");
        assert_eq!(with_cache, direct);
        // 不同 cache key → 不同会话
        let other = conv_of(json!({"prompt_cache_key": "sess-43", "messages": []}));
        assert_ne!(with_cache, other);
    }

    #[test]
    fn empty_body_still_yields_a_valid_conversation_id() {
        let ids = SessionIds::from_request(&json!({}), &HeaderMap::new());
        assert!(is_hex32(ids.conversation_id()));
        assert!(is_hex32(ids.request_id()));
    }

    #[test]
    fn inbound_request_id_is_passed_through() {
        let mut headers = HeaderMap::new();
        headers.insert("x-conversation-request-id", HeaderValue::from_static("turn-abc"));
        let ids = SessionIds::from_request(
            &json!({"messages": [{"role": "user", "content": "hi"}]}),
            &headers,
        );
        assert_eq!(ids.request_id(), "turn-abc");
    }

    #[test]
    fn request_id_tracks_the_latest_user_turn() {
        let body1 = json!({"messages": [{"role": "user", "content": "第一节"}]});
        let body2 = json!({"messages": [
            {"role": "user", "content": "第一节"},
            {"role": "assistant", "content": "好"},
            {"role": "user", "content": "第二节"},
        ]});
        let r1 = SessionIds::from_request(&body1, &HeaderMap::new()).request_id().to_string();
        let r2 = SessionIds::from_request(&body2, &HeaderMap::new()).request_id().to_string();
        assert_ne!(r1, r2, "轮级 ID 应随新一轮变化");
        // 会话级则不变
        assert_eq!(conv_of(body1), conv_of(body2));
    }

    #[test]
    fn google_contents_shape_is_supported() {
        let body = json!({"contents": [
            {"role": "user", "parts": [{"text": "谷歌形态"}]},
            {"role": "model", "parts": [{"text": "ok"}]},
            {"role": "user", "parts": [{"text": "再来"}]},
        ]});
        let ids = SessionIds::from_request(&body, &HeaderMap::new());
        assert!(is_hex32(ids.conversation_id()));
        // 会话锚点取首条 user
        assert_eq!(ids.conversation_id(), hex32("conv:谷歌形态"));
    }

    #[test]
    fn attempt_headers_rotate_message_and_trace_each_time() {
        let ids = SessionIds::from_request(&json!({}), &HeaderMap::new());
        let a = ids.attempt_headers();
        let b = ids.attempt_headers();
        assert_eq!(a.len(), 4, "只发 4 个头（不发 X-Root-Request-ID）");
        let get = |hs: &[UpstreamHeader], k: &str| {
            hs.iter().find(|h| h.key == k).map(|h| h.value.clone()).unwrap()
        };
        // 会话级 / 轮级两次相同
        assert_eq!(
            get(&a, HEADER_CONVERSATION_ID),
            get(&b, HEADER_CONVERSATION_ID)
        );
        assert_eq!(get(&a, HEADER_REQUEST_ID), get(&b, HEADER_REQUEST_ID));
        // 消息级 / trace 每次不同
        assert_ne!(get(&a, HEADER_MESSAGE_ID), get(&b, HEADER_MESSAGE_ID));
        assert_ne!(get(&a, HEADER_TRACE_ID), get(&b, HEADER_TRACE_ID));
        // 四个值互不相同
        let conv = get(&a, HEADER_CONVERSATION_ID);
        for k in [HEADER_REQUEST_ID, HEADER_MESSAGE_ID, HEADER_TRACE_ID] {
            assert_ne!(conv, get(&a, k), "会话级 ID 不能与其它头复用同一个值");
        }
        // 形态
        for k in [
            HEADER_CONVERSATION_ID,
            HEADER_REQUEST_ID,
            HEADER_MESSAGE_ID,
            HEADER_TRACE_ID,
        ] {
            assert!(is_hex32(&get(&a, k)), "{k} 形态不对");
        }
    }
}
