//! Protocol usage parsing without persistence or application state.

/// 从 usage JSON 提取缓存 token：读 = 命中缓存的部分，写 = 新建缓存的部分。
///
/// 兼容 Anthropic（`cache_read_input_tokens` / `cache_creation_input_tokens`）与
/// OpenAI 系（`prompt_tokens_details.cached_tokens` / `prompt_cache_hit_tokens`）。
pub fn cache_tokens_from_json(usage: &serde_json::Value) -> (u64, u64) {
    let read = usage
        .get("cache_read_input_tokens")
        .or_else(|| usage.get("prompt_cache_hit_tokens"))
        .or_else(|| {
            usage
                .get("prompt_tokens_details")
                .and_then(|d| d.get("cached_tokens"))
        })
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let write = usage
        .get("cache_creation_input_tokens")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    (read, write)
}
