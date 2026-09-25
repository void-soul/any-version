//! 订阅自动更新的 cron 表达式（抄 clash-party `core/profileUpdater.ts` 的 5 段式）。
//!
//! clash-party 用的是 `croner` 库，这里为避免新增依赖自己解析，只支持最常用的语法：
//! `*`、`*/n`、`a-b`、`a-b/n`、逗号列表（`a,b,c`）。字段顺序与标准 cron 一致：
//!
//! ```text
//! ┌─ 分 (0-59)
//! │ ┌─ 时 (0-23)
//! │ │ ┌─ 日 (1-31)
//! │ │ │ ┌─ 月 (1-12)
//! │ │ │ │ ┌─ 周 (0-7，0 与 7 都是周日)
//! * * * * *
//! ```
//!
//! 不认识的语法一律解析失败（报错交由上层提示），绝不「猜一个意思」静默按间隔跑。

/// 单个字段的取值范围
#[derive(Debug, Clone, PartialEq, Eq)]
struct CronField {
    min: u32,
    max: u32,
    /// 命中的取值集合（已展开）
    values: Vec<u32>,
}

impl CronField {
    fn parse(raw: &str, min: u32, max: u32) -> Result<Self, String> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err("字段为空".to_string());
        }
        let mut values: Vec<u32> = Vec::new();
        for part in raw.split(',') {
            let part = part.trim();
            let (range, step) = match part.split_once('/') {
                Some((r, s)) => (r.trim(), Some(s.trim())),
                None => (part, None),
            };
            let step_given = step.is_some();
            let step: u32 = match step {
                Some(s) => {
                    let n: u32 = s.parse().map_err(|_| format!("步长非法: {s}"))?;
                    if n == 0 {
                        return Err("步长不能为 0".to_string());
                    }
                    n
                }
                None => 1,
            };
            let (start, end) = if range == "*" || range.is_empty() {
                (min, max)
            } else if let Some((a, b)) = range.split_once('-') {
                let a: u32 = a.trim().parse().map_err(|_| format!("范围起点非法: {a}"))?;
                let b: u32 = b.trim().parse().map_err(|_| format!("范围终点非法: {b}"))?;
                (a, b)
            } else {
                let a: u32 = range.parse().map_err(|_| format!("取值非法: {range}"))?;
                // 单值没写步长就是「仅该值」；写了步长（如 `3/5`）才从该值展开到上界
                if step_given {
                    (a, max)
                } else {
                    (a, a)
                }
            };
            if start > end || end > max || start < min {
                return Err(format!("范围越界: {start}-{end}（允许 {min}-{max}）"));
            }
            let mut v = start;
            loop {
                values.push(v);
                if v >= end {
                    break;
                }
                v += step;
                if v > end {
                    break;
                }
            }
        }
        values.sort_unstable();
        values.dedup();
        Ok(CronField { min, max, values })
    }

    fn matches(&self, value: u32) -> bool {
        self.values.contains(&value)
    }
}

/// 已解析的 cron 表达式
#[derive(Debug, Clone)]
pub struct CronExpr {
    minute: CronField,
    hour: CronField,
    dom: CronField,
    month: CronField,
    dow: CronField,
}

impl CronExpr {
    pub fn parse(expr: &str) -> Result<Self, String> {
        let fields: Vec<&str> = expr.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(format!(
                "cron 需要 5 段（分 时 日 月 周），实际 {} 段: {expr}",
                fields.len()
            ));
        }
        Ok(CronExpr {
            minute: CronField::parse(fields[0], 0, 59)?,
            hour: CronField::parse(fields[1], 0, 23)?,
            dom: CronField::parse(fields[2], 1, 31)?,
            month: CronField::parse(fields[3], 1, 12)?,
            dow: CronField::parse(fields[4], 0, 7)?,
        })
    }

    /// 当前时间是否命中。`dow` 用 0=周日（与 chrono/Utc 的 `weekday().num_days_from_sunday()` 一致）。
    pub fn matches(&self, minute: u32, hour: u32, dom: u32, month: u32, dow: u32) -> bool {
        // 0 与 7 都表示周日：两边都要认（`* * * * 0` 也要能命中传进来的 7）
        let dow_hit = self.dow.matches(dow)
            || (dow == 0 && self.dow.matches(7))
            || (dow == 7 && self.dow.matches(0));
        self.minute.matches(minute)
            && self.hour.matches(hour)
            && self.dom.matches(dom)
            && self.month.matches(month)
            && dow_hit
    }

    /// 该 cron 的最小有效分辨率（分钟）：用于推导调度器去重窗口，避免同一分钟重复触发。
    pub fn minute_key_of(&self, _minute: u32) -> u32 {
        1
    }
}

/// 便捷入口：给定「分/时/日/月/周」判断 cron 字符串是否命中。
///
/// 空串视为「未配置 cron」→ 返回 false（调用方继续走秒级间隔逻辑）。
pub fn cron_matches(
    expr: Option<&str>,
    minute: u32,
    hour: u32,
    dom: u32,
    month: u32,
    dow: u32,
) -> bool {
    match expr.map(str::trim).filter(|s| !s.is_empty()) {
        None => false,
        Some(e) => match CronExpr::parse(e) {
            Ok(c) => c.matches(minute, hour, dom, month, dow),
            Err(err) => {
                eprintln!("[mihomo] cron 表达式非法（{}），本次跳过自动更新: {}", e, err);
                false
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{cron_matches, CronExpr};

    #[test]
    fn parses_standard_forms() {
        assert!(CronExpr::parse("*/5 * * * *").is_ok());
        assert!(CronExpr::parse("0 3 * * *").is_ok());
        assert!(CronExpr::parse("30 8-18/2 * * 1-5").is_ok());
        assert!(CronExpr::parse("0,15,30 12 * * *").is_ok());
        // 段数不对 / 越界 / 步长为 0 都要报错，不能静默当成别的语义
        assert!(CronExpr::parse("* * * *").is_err());
        assert!(CronExpr::parse("*/0 * * * *").is_err());
        assert!(CronExpr::parse("99 * * * *").is_err());
        assert!(CronExpr::parse("abc * * * *").is_err());
    }

    #[test]
    fn matches_expected_minutes() {
        let every5 = CronExpr::parse("*/5 * * * *").unwrap();
        assert!(every5.matches(0, 9, 1, 1, 3));
        assert!(every5.matches(5, 9, 1, 1, 3));
        assert!(!every5.matches(3, 9, 1, 1, 3));

        // 每天 03:00（周一）
        let daily = CronExpr::parse("0 3 * * *").unwrap();
        assert!(daily.matches(0, 3, 20, 6, 1));
        assert!(!daily.matches(1, 3, 20, 6, 1));
        assert!(!daily.matches(0, 4, 20, 6, 1));

        // 工作日 8-18 点每 2 小时的第 30 分
        let work = CronExpr::parse("30 8-18/2 * * 1-5").unwrap();
        assert!(work.matches(30, 8, 2, 3, 1));
        assert!(work.matches(30, 10, 2, 3, 5));
        assert!(!work.matches(30, 11, 2, 3, 1), "步长外的小时不命中");
        assert!(!work.matches(30, 10, 2, 3, 6), "周六不命中");

        // 周日：0 与 7 等价
        let sunday = CronExpr::parse("0 0 * * 0").unwrap();
        assert!(sunday.matches(0, 0, 4, 1, 0));
        assert!(sunday.matches(0, 0, 4, 1, 7));
    }

    #[test]
    fn empty_expression_never_matches() {
        // 未配置 cron → 交给秒级间隔逻辑，不能因为「空串」被当成每分钟执行
        assert!(!cron_matches(None, 0, 0, 1, 1, 0));
        assert!(!cron_matches(Some("  "), 0, 0, 1, 1, 0));
        assert!(!cron_matches(Some("* * *"), 0, 0, 1, 1, 0), "非法表达式不命中");
    }
}
