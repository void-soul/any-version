//! 2API 模型目录：转发后端权威列表，失败逐级回落。
//!
//! 客户端靠 `/v1/models` 枚举模型，而本机 product.json 与硬编码快照都会过期
//! —— 2026-10-03 因此漏掉上游已支持、本地清单还没有的 space-bunny
//! （实测：后端 31 个 vs 接口 18 个）。后端那份是客户端 UI 用的同一份
//! （客户端 asar 里的 remote.models），只有它跟得上新模型。

use std::time::{Duration, Instant};

pub const BACKEND_BASE: &str = "https://copilot.tencent.com";
pub const BACKEND_MODELS_PATH: &str = "/v2/enterprises/personal/models";

/// 客户端会反复拉模型列表，不缓存就是每次都打后端
pub const CATALOG_TTL: Duration = Duration::from_secs(300);

/// 兜底列表（后端与本机清单都不可用时，保证 /v1/models 不返回空）
pub const FALLBACK_MODELS: &[&str] = &["hy3", "hy4-preview", "kimi-k3", "glm-5.3"];

const NON_CHAT_TAGS: &[&str] = &["text-to-image", "image-to-image", "text-to-video"];
const INTERNAL_KEYWORDS: &[&str] = &["completion", "rewrite", "jump", "codewise"];

/// 解析后端目录：{"code":0,"data":{"models":[…]}}，滤掉非聊天模型并保持顺序。
/// 结构变了就返回空（调用方回落下一级），不抛异常。
pub fn parse_catalog(payload: &serde_json::Value) -> Vec<String> {
    let Some(models) = payload
        .get("data")
        .and_then(|d| d.get("models"))
        .and_then(|m| m.as_array())
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for model in models {
        let Some(id) = model
            .get("id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let is_non_chat = model
            .get("tags")
            .and_then(|t| t.as_array())
            .map(|tags| {
                tags.iter().any(|t| {
                    t.as_str()
                        .map(|s| NON_CHAT_TAGS.iter().any(|tag| s == *tag))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false);
        if is_non_chat {
            continue;
        }
        if model.get("vendor").and_then(|v| v.as_str()) == Some("tencent") {
            continue;
        }
        let lower = id.to_ascii_lowercase();
        if INTERNAL_KEYWORDS.iter().any(|k| lower.contains(k)) {
            continue;
        }
        out.push(id.to_string());
    }
    out
}

/// 目录缓存。**只缓存成功结果** —— 空结果视为失败不入缓存，
/// 否则一次网络抖动会把降级列表锁死整个 TTL。
#[derive(Default)]
pub struct CatalogCache {
    entry: Option<(Instant, Vec<String>)>,
}

impl CatalogCache {
    pub fn is_fresh(&self) -> bool {
        self.entry
            .as_ref()
            .map(|(at, list)| !list.is_empty() && at.elapsed() < CATALOG_TTL)
            .unwrap_or(false)
    }

    pub fn get(&self) -> Option<Vec<String>> {
        self.entry.as_ref().map(|(_, list)| list.clone())
    }

    /// 存入目录；空列表视为失败不缓存。返回是否真的写入。
    pub fn store(&mut self, list: Vec<String>, at: Instant) -> bool {
        if list.is_empty() {
            return false;
        }
        self.entry = Some((at, list));
        true
    }

    pub fn clear(&mut self) {
        self.entry = None;
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use std::time::{Duration, Instant};

    #[test]
    fn backend_catalog_is_filtered_and_keeps_new_models() {
        let payload = json!({"code": 0, "data": {"models": [
            {"id": "auto", "vendor": "f"},
            {"id": "hy3", "vendor": "f"},
            {"id": "space-bunny", "vendor": "f"},
            {"id": "hunyuan-image-alpha", "tags": ["text-to-image"], "vendor": "f"},
            {"id": "hunyuan-7b-dense", "vendor": "tencent"},
            {"id": "codewise-jump", "vendor": "f"},
            {"id": "hunyuan-video-gen", "tags": ["text-to-video"]}
        ]}});
        assert_eq!(
            super::parse_catalog(&payload),
            vec!["auto".to_string(), "hy3".to_string(), "space-bunny".to_string()],
            "只保留聊天模型，且顺序与后端一致"
        );
    }

    #[test]
    fn malformed_payload_yields_empty_not_panic() {
        assert!(super::parse_catalog(&json!({})).is_empty());
        assert!(super::parse_catalog(&json!({"code": 0, "data": {}})).is_empty());
        assert!(super::parse_catalog(&json!({"data": {"models": []}})).is_empty());
        assert!(super::parse_catalog(&json!({"data": {"models": [{"no_id": 1}]}})).is_empty());
    }

    #[test]
    fn cache_is_fresh_within_ttl() {
        let mut cache = super::CatalogCache::default();
        assert!(!cache.is_fresh(), "刚创建时没有内容");
        assert!(cache.store(vec!["hy3".to_string()], Instant::now()));
        assert!(cache.is_fresh(), "刚存的内容应新鲜");
        assert_eq!(cache.get(), Some(vec!["hy3".to_string()]));
    }

    #[test]
    fn cache_expires_after_ttl() {
        let mut cache = super::CatalogCache::default();
        cache.store(
            vec!["hy3".to_string()],
            Instant::now() - super::CATALOG_TTL - Duration::from_secs(1),
        );
        assert!(!cache.is_fresh(), "超过 TTL 应视为过期");
    }

    #[test]
    fn empty_result_is_never_cached() {
        let mut cache = super::CatalogCache::default();
        assert!(!cache.store(Vec::new(), Instant::now()), "空结果不写缓存");
        assert!(!cache.is_fresh());
        assert!(cache.get().is_none());
    }

    #[test]
    fn fallback_list_is_never_empty() {
        assert!(!super::FALLBACK_MODELS.is_empty(), "兜底不能是空列表");
    }
}
