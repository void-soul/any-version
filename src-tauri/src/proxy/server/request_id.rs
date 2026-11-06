//! 请求级 ID 与轻量运行时指标（AUDIT-020）。
//!
//! 只写进程内聚合，不落盘、不引入监控依赖；日志查看器与前端可按需读取。
//! 代理侧已有 `call_id`，这里补上证书接收端与统一错误率/耗时统计。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

static NEXT: AtomicU64 = AtomicU64::new(1);

/// 生成本进程内唯一的请求 ID（单调递增，带进程号避免多实例混淆）。
pub fn next_request_id() -> String {
    format!(
        "r{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

#[derive(Default, Clone, Debug)]
pub struct EndpointStat {
    pub total: u64,
    pub failed: u64,
    pub total_ms: u64,
}

fn stats() -> &'static Mutex<HashMap<String, EndpointStat>> {
    // HashMap::new 非 const，不能直接放进 static；用 OnceLock 懒初始化。
    static STATS: std::sync::OnceLock<Mutex<HashMap<String, EndpointStat>>> =
        std::sync::OnceLock::new();
    STATS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 记录一次请求结果。`endpoint` 用「方法 路径」或简单标签，避免高基数。
pub fn record(endpoint: &str, success: bool, elapsed_ms: u64) {
    if let Ok(mut map) = stats().lock() {
        let entry = map.entry(endpoint.to_string()).or_default();
        entry.total += 1;
        if !success {
            entry.failed += 1;
        }
        entry.total_ms += elapsed_ms;
    }
}

/// 当前快照，供诊断命令/日志查看器读取。
pub fn snapshot() -> HashMap<String, EndpointStat> {
    stats().lock().map(|m| m.clone()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_ids_are_unique() {
        let a = next_request_id();
        let b = next_request_id();
        assert_ne!(a, b);
        assert!(a.starts_with("r"));
    }

    #[test]
    fn stats_aggregate_success_and_failure() {
        let key = format!("GET /test-{}", next_request_id());
        record(&key, true, 10);
        record(&key, false, 30);
        let snap = snapshot();
        let s = snap.get(&key).unwrap();
        assert_eq!(s.total, 2);
        assert_eq!(s.failed, 1);
        assert_eq!(s.total_ms, 40);
    }
}
