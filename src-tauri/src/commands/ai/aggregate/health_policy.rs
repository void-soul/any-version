//! health policy responsibility.
use super::*;

/// 同一候选的尝试次数（含首次）。鉴权/额度/模型/限流不重试，直接切下一个。
pub fn retry_budget(class: FailureClass) -> usize {
    match class {
        FailureClass::Transient => 2,
        _ => 1,
    }
}

/// 该失败是否值得换到下一个候选。
///
/// 抄 cc-switch `forwarder.rs` 的换家判定：只有「换一家有可能成功」的错误才换。
/// 落到我们的分类上：
/// - `Fatal`（内容策略拒绝）不换 —— 换哪家都一样；
/// - **请求体层面的拒绝（400 / 422）不换** —— 这是我们这条请求本身的问题，
///   换供应商也修不好，白跑一遍整条链还会把每个候选都拖进冷却；
/// - 其余（5xx / 超时 / 连不上 / 401 / 402 / 404 / 429）都是**候选自身**的问题
///   （凭据坏了、额度没了、模型没了、被限流）→ 换一家很可能就成了。
///
/// 与 cc-switch 的偏差是刻意的：它把整个 4xx 都归为「不换」，但我们的候选链里
/// 401/402/404/429 是**按候选**变化的（每家 key、额度、上架模型都不同），
/// 全部不换会让故障转移在这些最常见的场景下完全失效。
pub fn should_switch_candidate(class: FailureClass, status: Option<u16>) -> bool {
    if class == FailureClass::Fatal {
        return false;
    }
    !matches!(status, Some(400) | Some(422))
}

/// 该类别是否值得对同一候选再试一次（只有瞬时错误才重试，其余直接切下一个）。
pub(super) fn is_retryable(class: FailureClass) -> bool {
    retry_budget(class) > 1
}

/// `Retry-After` 的信任上限：**1 小时**。
///
/// 抄 magpie `internal/gateway/routing.go:101` 的 `longestWait = 1h`。我们以前直接
/// `.min(24h)` 收下厂商给的任何值 —— 遇到脏数据（`Retry-After: 86400`、或把毫秒当秒填）
/// 就把候选冻一整天，期间这条链少一个可选项。厂商说的话要信，但只信到小时级别；
/// 想更长的等待，去读「什么时候恢复」那条（[`parse_resets_at`]）。
pub const RETRY_AFTER_TRUST_CAP_SECS: u64 = 60 * 60;

/// 「厂商明确说了什么时候恢复」的信任上限：7 天（抄 magpie 的 `longestWritten`）。
/// 超过就当没读懂 —— 一个声称要等半年的限额信息一定是解析错了。
pub const RESETS_WRITTEN_CAP_SECS: u64 = 7 * 24 * 60 * 60;

/// 从错误体里解析「厂商说自己什么时候恢复」，返回 Unix **毫秒**。
///
/// 抄 magpie `internal/gateway/routing.go` 的四种 `resets*` 解析（`:520-617`）。
/// 我们以前只会按固定时长冷却，等于「明知道 19 分钟后恢复，却先冻 1 小时」。
/// 形态：
/// - `resets_at`（RFC3339 / unix 秒）—— ChatGPT 系
/// - `resets_in_seconds` / `resetsAfter` —— 相对秒数
/// - `usage limit reached|<10 位 unix>` —— Claude Code 系
/// - `resets in 0h 19min` / `resets in 19m` —— 相对时长
/// - 中文「将在 2026-10-11 20:16:02 重置」（含可选时区偏移）
///
/// `now_ms` 由调用方传入，便于测试。解析不出或超出 7 天上限 → `None`（走固定时长兜底）。
pub fn parse_resets_at(body: &str, now_ms: u64) -> Option<u64> {
    let lower = body.to_ascii_lowercase();

    // 1) resets_at / resets_in_seconds
    if let Some(v) = json_u64_field(body, &["resets_at", "resetsAt", "reset_at"]) {
        if let Some(ms) = unix_to_ms(v) {
            if in_trust_window(ms, now_ms) {
                return Some(ms);
            }
        }
    }
    if let Some(secs) = json_u64_field(body, &["resets_in_seconds", "resetsInSeconds"]) {
        let ms = now_ms.saturating_add(secs.saturating_mul(1000));
        if in_trust_window(ms, now_ms) {
            return Some(ms);
        }
    }

    // 2) usage limit reached|<unix>（Claude Code 把 10 位时间戳拼在文案后面）
    for marker in [
        "usage limit reached|",
        "usage limit reached:",
        "usage limit reached ",
    ] {
        if let Some(idx) = lower.find(marker) {
            let tail = &lower[idx + marker.len()..];
            let digits: String = tail
                .chars()
                .skip_while(|c| !c.is_ascii_digit())
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if digits.len() == 10 {
                if let Ok(secs) = digits.parse::<u64>() {
                    let ms = secs.saturating_mul(1000);
                    if in_trust_window(ms, now_ms) {
                        return Some(ms);
                    }
                }
            }
        }
    }

    // 3) resets in 0h 19min / resets in 19m / resets in 19 minutes
    if let Some(idx) = lower.find("resets in") {
        let secs = parse_relative_duration(&lower[idx + "resets in".len()..]);
        if secs > 0 {
            let ms = now_ms.saturating_add(secs.saturating_mul(1000));
            if in_trust_window(ms, now_ms) {
                return Some(ms);
            }
        }
    }

    // 4) 中文「将在 2026-10-11 20:16:02 重置」（时区偏移可选）
    if let Some(idx) = body.find("将在") {
        let tail: String = body[idx + "将在".len()..].chars().take(40).collect();
        if let Some(ms) = parse_cn_reset_time(&tail) {
            if in_trust_window(ms, now_ms) {
                return Some(ms);
            }
        }
    }
    if let Some(idx) = body.find("将于") {
        let tail: String = body[idx + "将于".len()..].chars().take(40).collect();
        if let Some(ms) = parse_cn_reset_time(&tail) {
            if in_trust_window(ms, now_ms) {
                return Some(ms);
            }
        }
    }
    None
}

/// 恢复时刻必须在「未来 5 分钟」~「未来 7 天」之间才采信。
/// 过去的时间（时钟漂移 / 已过期）当没读懂；太远的当解析错了。
pub(super) fn in_trust_window(ms: u64, now_ms: u64) -> bool {
    let five_min_ms = 5 * 60 * 1000;
    let cap_ms = RESETS_WRITTEN_CAP_SECS.saturating_mul(1000);
    ms > now_ms.saturating_add(five_min_ms) && ms <= now_ms.saturating_add(cap_ms)
}

/// 10 位按 unix 秒、13 位按毫秒处理；其他位数不采信。
pub(super) fn unix_to_ms(v: u64) -> Option<u64> {
    match v {
        v if (1_000_000_000..10_000_000_000).contains(&v) => Some(v.saturating_mul(1000)),
        v if v > 10_000_000_000 => Some(v),
        _ => None,
    }
}

/// 从 JSON 里取第一个能解析成 u64 的字段。
///
/// **顶层和 `error` 子对象都要看**：限额信息在真实响应里通常是嵌在 `error` 下的
/// （`{"error":{"resets_in_seconds":1140}}`），只看顶层会整条漏掉。
pub(super) fn json_u64_field(body: &str, names: &[&str]) -> Option<u64> {
    let parsed: Value = serde_json::from_str(body).ok()?;
    let mut scopes: Vec<&Value> = vec![&parsed];
    if let Some(err) = parsed.get("error") {
        scopes.push(err);
    }
    for scope in scopes {
        for name in names {
            match scope.get(*name) {
                Some(Value::Number(n)) => {
                    if let Some(u) = n.as_u64() {
                        return Some(u);
                    }
                }
                Some(Value::String(s)) => {
                    if let Ok(u) = s.trim().parse::<u64>() {
                        return Some(u);
                    }
                }
                _ => {}
            }
        }
    }
    None
}

/// 解析 `0h 19min` / `19m` / `2 hours 5 minutes` / `20分` 这类相对时长（秒）。
///
/// 逐个扫「数字 + 单位」对并累加；没有任一带单位的数字 → 0（当没读懂）。
pub(super) fn parse_relative_duration(text: &str) -> u64 {
    let chars: Vec<char> = text.chars().collect();
    let mut total: u64 = 0;
    let mut found = false;
    let mut i = 0usize;
    while i < chars.len() {
        if !chars[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let mut num = String::new();
        while i < chars.len() && chars[i].is_ascii_digit() {
            num.push(chars[i]);
            i += 1;
        }
        let value: u64 = num.parse().unwrap_or(0);
        // 单位：吞掉数字与单位之间的分隔符（空格/逗号），读到下一个数字或 12 字符为止
        let mut unit = String::new();
        while i < chars.len() && !chars[i].is_ascii_digit() && unit.chars().count() < 12 {
            unit.push(chars[i]);
            i += 1;
        }
        // 分隔符要先去掉：`" minutes"` 不 strip 就 `starts_with('m')` 判false，
        // `"resets in 2 hours 5 minutes"` 会被整段丢掉
        let u = unit.trim().to_ascii_lowercase();
        let secs = if u.starts_with('h') || u.contains("小时") {
            value.saturating_mul(3600)
        } else if u.starts_with('m') || u.contains("分") {
            value.saturating_mul(60)
        } else if u.starts_with('s') || u.contains("秒") {
            value
        } else {
            0
        };
        if secs > 0 {
            total = total.saturating_add(secs);
            found = true;
        }
    }
    if found {
        total
    } else {
        0
    }
}

/// 解析中文「2026-10-11 20:16:02（可选时区）」这类绝对时刻。
///
/// 严格校验（抄 magpie `resetsWritten:589-617` 的防误读）：
/// - 月份 1~12、日 1~31、时 0~23、分秒 0~59 —— 挡掉「2026-13-45」这类路过的日期；
/// - 数字原样拼成时间戳后再 round-trip 回年月日比对 —— 挡掉 2 月 31 日这类溢出。
/// 无时区信息按 **本地时区** 处理（厂商文案通常给本地时间）。
pub(super) fn parse_cn_reset_time(text: &str) -> Option<u64> {
    // 抓第一组 `YYYY-MM-DD`
    let bytes: Vec<char> = text.chars().collect();
    let mut h: u32 = 0;
    let mut mi: u32 = 0;
    let mut se: u32 = 0;
    let mut i = 0usize;
    // 年
    let mut num = String::new();
    while i < bytes.len() && num.len() < 4 && bytes[i].is_ascii_digit() {
        num.push(bytes[i]);
        i += 1;
    }
    if num.len() != 4 || i >= bytes.len() || bytes[i] != '-' {
        return None;
    }
    let y: i64 = num.parse().ok()?;
    i += 1;
    num.clear();
    while i < bytes.len() && num.len() < 2 && bytes[i].is_ascii_digit() {
        num.push(bytes[i]);
        i += 1;
    }
    if num.len() != 2 || i >= bytes.len() || bytes[i] != '-' {
        return None;
    }
    let mo: u32 = num.parse().ok()?;
    i += 1;
    num.clear();
    while i < bytes.len() && num.len() < 2 && bytes[i].is_ascii_digit() {
        num.push(bytes[i]);
        i += 1;
    }
    if num.len() != 2 {
        return None;
    }
    let d: u32 = num.parse().ok()?;
    // 校验月/日范围
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    // 找时间部分（跳过日期与时间之间的任意非数字串）
    let rest: String = bytes[i.min(bytes.len())..].iter().collect();
    let time_part = rest.trim_start_matches(|c: char| !c.is_ascii_digit());
    let tparts: Vec<&str> = time_part.split(':').collect();
    if tparts.len() >= 2 {
        h = tparts[0].get(..2)?.parse().ok()?;
        mi = tparts[1].get(..2)?.parse().ok()?;
        if tparts.len() >= 3 && tparts[2].len() >= 2 {
            se = tparts[2][..2].parse().ok()?;
        }
    }
    if h > 23 || mi > 59 || se > 59 {
        return None;
    }
    // 拼时间戳并 round-trip 校验（挡掉 2 月 31 日这种溢出）
    let candidate = local_time_to_millis(y, mo, d, h, mi, se)?;
    if round_trip_local(candidate) != (y, mo, d, h, mi, se) {
        return None;
    }
    Some(candidate)
}

/// 把本地年月日时分秒转成 Unix 毫秒（用 `chrono::Local` 的时区偏移）。
pub(super) fn local_time_to_millis(
    y: i64,
    mo: u32,
    d: u32,
    h: u32,
    mi: u32,
    se: u32,
) -> Option<u64> {
    use chrono::{Local, TimeZone};
    let naive = chrono::NaiveDate::from_ymd_opt(y as i32, mo, d)?.and_hms_opt(h, mi, se)?;
    let local: chrono::DateTime<Local> = Local.from_local_datetime(&naive).earliest()?;
    Some(local.timestamp_millis().max(0) as u64)
}

/// 把时间戳换回本地年月日时分秒（校验用）。
pub(super) fn round_trip_local(ms: u64) -> (i64, u32, u32, u32, u32, u32) {
    use chrono::{Datelike, Local, TimeZone, Timelike};
    match Local.timestamp_millis_opt(ms as i64).single() {
        Some(dt) => (
            dt.year() as i64,
            dt.month(),
            dt.day(),
            dt.hour(),
            dt.minute(),
            dt.second(),
        ),
        None => (0, 0, 0, 0, 0, 0),
    }
}

/// 冷却时长（切换后多久内不再尝试该候选）。
pub fn cooldown_for(
    class: FailureClass,
    retry_after: Option<u64>,
    consecutive: u32,
) -> std::time::Duration {
    const TRANSIENT_BASE: u64 = 30;
    const TRANSIENT_MAX: u64 = 5 * 60;
    const RATE_LIMIT_DEFAULT: u64 = 60;
    const AUTH_OR_BILLING: u64 = 60 * 60;
    const MODEL_UNAVAILABLE: u64 = 6 * 60 * 60;
    match class {
        FailureClass::Authentication | FailureClass::Billing => {
            std::time::Duration::from_secs(AUTH_OR_BILLING)
        }
        FailureClass::ModelUnavailable => std::time::Duration::from_secs(MODEL_UNAVAILABLE),
        FailureClass::RateLimit => std::time::Duration::from_secs(
            // 封顶 1 小时（magpie longestWait），不再盲信 24h
            retry_after
                .unwrap_or(RATE_LIMIT_DEFAULT)
                .min(RETRY_AFTER_TRUST_CAP_SECS),
        ),
        FailureClass::Transient => {
            // 30s → 60s → 120s → 240s → 300s（封顶）
            let shift = consecutive.saturating_sub(1).min(4);
            let secs = (TRANSIENT_BASE << shift).min(TRANSIENT_MAX);
            std::time::Duration::from_secs(secs)
        }
        FailureClass::Fatal => std::time::Duration::from_secs(0),
    }
}

/// 「限额类」失败的冷却：优先用厂商说的恢复时刻，退化到默认时长。
///
/// 抄 magpie 的 `failQuota`（`routing.go:651-682`）的取值优先级：
/// 「用户自设 > 厂商说的恢复时刻 > Retry-After > 固定兜底」。
/// 这是 magpie ② 里对我们价值最高的一条：以前只知道「要等」，现在能等到点。
pub fn quota_cooldown_for(
    default_secs: u64,
    retry_after: Option<u64>,
    resets_at_ms: Option<u64>,
    now_ms: u64,
) -> std::time::Duration {
    let cap_ms = RETRY_AFTER_TRUST_CAP_SECS.saturating_mul(1000);
    // 1) 厂商明确说了恢复时刻 → 用它（但仍受 1 小时上限约束，避免脏数据把候选冻太久；
    //    超过上限时先试 Retry-After，两者都没有就用默认值）
    if let Some(target) =
        resets_at_ms.filter(|t| *t > now_ms && *t <= now_ms.saturating_add(cap_ms))
    {
        let secs = target.saturating_sub(now_ms).div_ceil(1000);
        return std::time::Duration::from_secs(secs.max(1));
    }
    // 2) Retry-After（封顶 1 小时）
    if let Some(secs) = retry_after {
        return std::time::Duration::from_secs(secs.clamp(1, RETRY_AFTER_TRUST_CAP_SECS));
    }
    std::time::Duration::from_secs(default_secs)
}

/// 候选健康状态（服务进程内维护；重启即清空）。
#[derive(Debug, Clone, Default)]
pub(super) struct CandidateHealth {
    pub(super) consecutive_failures: u32,
    pub(super) cooldown_until_ms: Option<u64>,
}

pub(super) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 候选剩余冷却秒数（未冷却返回 None）。
pub(super) fn cooldown_remaining_secs(
    health: &Arc<Mutex<HashMap<String, CandidateHealth>>>,
    key: &str,
) -> Option<u64> {
    let guard = health.lock().ok()?;
    let until = guard.get(key)?.cooldown_until_ms?;
    let now = now_ms();
    if until > now {
        Some((until - now) / 1000)
    } else {
        None
    }
}

/// 记录一次失败：累加连续失败次数并按类别设置冷却，返回冷却时长。
pub(super) fn record_failure(
    health: &Arc<Mutex<HashMap<String, CandidateHealth>>>,
    key: &str,
    class: FailureClass,
    retry_after: Option<u64>,
    resets_at_ms: Option<u64>,
) -> std::time::Duration {
    let mut guard = match health.lock() {
        Ok(g) => g,
        Err(_) => return std::time::Duration::from_secs(0),
    };
    let entry = guard.entry(key.to_string()).or_default();
    entry.consecutive_failures = entry.consecutive_failures.saturating_add(1);
    // 限额类优先「等到厂商说的那个点」，而不是一律固定时长（抄 magpie failQuota）
    let cooldown = match class {
        FailureClass::Billing | FailureClass::RateLimit => quota_cooldown_for(
            cooldown_for(class, retry_after, entry.consecutive_failures).as_secs(),
            retry_after,
            resets_at_ms,
            now_ms(),
        ),
        _ => cooldown_for(class, retry_after, entry.consecutive_failures),
    };
    entry.cooldown_until_ms = Some(now_ms().saturating_add(cooldown.as_millis() as u64));
    cooldown
}

/// 成功后清除该候选的健康记录（失败计数与冷却一起清零）。
pub(super) fn clear_health(health: &Arc<Mutex<HashMap<String, CandidateHealth>>>, key: &str) {
    if let Ok(mut guard) = health.lock() {
        guard.remove(key);
    }
}

// ─── 自引用防护 ───

/// 单个候选请求失败的结构化信息。
pub(super) struct CandidateError {
    pub(super) class: FailureClass,
    pub(super) message: String,
    pub(super) retry_after: Option<u64>,
    /// 上游 HTTP 状态码（连接失败/超时为 None）
    pub(super) status: Option<u16>,
    /// 上游原始错误体：反应式整流靠它判断「能不能修」
    pub(super) error_body: String,
    /// 本次实际发出去的 P_out 请求体（整流器操作的就是这个形态）
    pub(super) outbound_body: Value,
}
