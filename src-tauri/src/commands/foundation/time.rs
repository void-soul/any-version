//! 时间的唯一真源（AUDIT-025 / 050）。
//!
//! 项目此前散落 19 个 `now_ms` / `now_ts` / `now_iso` / `now_secs` 实现，
//! 有的用本地时间 TEXT、有的用 UTC、有的用秒，跨时区/DST 会错位。
//! 存储一律用 **UTC RFC3339** 或 **毫秒 epoch INTEGER**；展示层再按用户时区格式化。

use chrono::{DateTime, SecondsFormat, TimeZone, Utc};

/// 当前 UTC 时间。
pub fn utc_now() -> DateTime<Utc> {
    Utc::now()
}

/// UTC RFC3339 字符串（`2026-11-04T12:34:56Z`），用于新建记录。
pub fn now_rfc3339_utc() -> String {
    utc_now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// 毫秒 epoch，用于时间戳窗口/签名防重放。
pub fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// 秒 epoch，用于 cron/过期判断。
pub fn now_secs() -> i64 {
    utc_now().timestamp()
}

/// 把 RFC3339 / 本地无时区字符串解析为 UTC RFC3339。
/// 无法解析时返回 `None`，由调用方显式决定回退策略（不静默造一个错误时间）。
pub fn parse_to_utc_rfc3339(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
        return Some(
            dt.with_timezone(&Utc)
                .to_rfc3339_opts(SecondsFormat::Secs, true),
        );
    }
    // 旧版本写入的本地无时区字符串：按本地时区解释后转 UTC。
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M:%S") {
        let local = chrono::Local.from_local_datetime(&naive).single()?;
        return Some(
            local
                .with_timezone(&Utc)
                .to_rfc3339_opts(SecondsFormat::Secs, true),
        );
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_string_ends_with_z() {
        let s = now_rfc3339_utc();
        assert!(s.ends_with('Z'), "{s}");
        assert!(s.contains('T'));
    }

    #[test]
    fn millis_and_secs_are_consistent() {
        let ms = now_millis();
        let s = now_secs();
        assert!((ms / 1000) as i64 - s <= 1);
        assert!(ms > 1_000_000_000_000);
    }

    #[test]
    fn rfc3339_roundtrip_is_utc() {
        let parsed = parse_to_utc_rfc3339("2026-11-04T20:00:00+08:00").unwrap();
        assert_eq!(parsed, "2026-11-04T12:00:00Z");
        assert_eq!(parse_to_utc_rfc3339(""), None);
        assert_eq!(parse_to_utc_rfc3339("not a date"), None);
    }
}
