use std::sync::Mutex;
use crate::commands::config::get_data_dir;

use super::models::*;

// ─── SQLite 持久化 ───

/// 全局数据库连接池（Mutex 保护，保证多线程安全写入）。
/// 使用 WAL 模式以支持并发读写不阻塞。
static DB_CONN: Mutex<Option<rusqlite::Connection>> = Mutex::new(None);

/// 获取数据库文件路径
fn db_path() -> std::path::PathBuf {
    get_data_dir().join("ai_usage.db")
}

/// 打开并初始化连接（不持有全局锁；由调用方在锁内调用，避免并发重复初始化）。
fn init_connection() -> Result<rusqlite::Connection, String> {
    let path = db_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let conn = rusqlite::Connection::open(&path)
        .map_err(|e| format!("打开数据库失败: {}", e))?;

    // 启用 WAL 模式，提升并发写入性能
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| format!("设置 WAL 模式失败: {}", e))?;

    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS ai_usage (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            tool_id     TEXT    NOT NULL,
            model       TEXT    NOT NULL,
            provider    TEXT,
            input_tokens  INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            duration_ms     INTEGER NOT NULL DEFAULT 0,
            first_token_ms  INTEGER NOT NULL DEFAULT 0,
            ok              INTEGER NOT NULL DEFAULT 1,
            failure_class   TEXT,
            cache_read_tokens  INTEGER NOT NULL DEFAULT 0,
            cache_write_tokens INTEGER NOT NULL DEFAULT 0,
            timestamp   TEXT    NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_ai_usage_tool  ON ai_usage(tool_id);
        CREATE INDEX IF NOT EXISTS idx_ai_usage_model ON ai_usage(model);
        CREATE INDEX IF NOT EXISTS idx_ai_usage_ts    ON ai_usage(timestamp);
        "#,
    )
    .map_err(|e| format!("初始化表失败: {}", e))?;

    ensure_usage_columns(&conn)?;

    Ok(conn)
}

/// 老库补列：`CREATE TABLE IF NOT EXISTS` 不会给已存在的表加列，
/// 因此耗时字段（t/s 统计的数据来源）需要显式迁移。抄自 cc-switch dc0febe5 的用量表扩展。
fn ensure_usage_columns(conn: &rusqlite::Connection) -> Result<(), String> {
    let existing: Vec<String> = {
        let mut stmt = conn
            .prepare("PRAGMA table_info(ai_usage)")
            .map_err(|e| format!("读取表结构失败: {}", e))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| format!("读取表结构失败: {}", e))?;
        rows.filter_map(|r| r.ok()).collect()
    };

    for (column, ddl) in [
        (
            "duration_ms",
            "ALTER TABLE ai_usage ADD COLUMN duration_ms INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "first_token_ms",
            "ALTER TABLE ai_usage ADD COLUMN first_token_ms INTEGER NOT NULL DEFAULT 0",
        ),
        // 成功率维度：失败请求也会落一行（token 为 0），否则 succeeded/total 无法统计。
        (
            "ok",
            "ALTER TABLE ai_usage ADD COLUMN ok INTEGER NOT NULL DEFAULT 1",
        ),
        (
            "failure_class",
            "ALTER TABLE ai_usage ADD COLUMN failure_class TEXT",
        ),
        // 缓存维度：缓存命中率 = cache_read / (input + cache_read)，只统计上报过缓存的请求。
        (
            "cache_read_tokens",
            "ALTER TABLE ai_usage ADD COLUMN cache_read_tokens INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "cache_write_tokens",
            "ALTER TABLE ai_usage ADD COLUMN cache_write_tokens INTEGER NOT NULL DEFAULT 0",
        ),
    ] {
        if !existing.iter().any(|c| c == column) {
            conn.execute(ddl, [])
                .map_err(|e| format!("迁移用量表列 {} 失败: {}", column, e))?;
        }
    }
    Ok(())
}

/// 输出速度（tokens/s）：只在「有输出 token」且「测得生成窗口」时给出数值。
///
/// 口径抄自 cc-switch dc0febe5：生成窗口 = 总耗时 − 首字延迟（流式）；
/// 非流式没有首字延迟，窗口即总耗时。窗口 <= 0 时返回 None，由前端留空。
fn output_tps(output_tokens: u64, generation_window_ms: u64) -> Option<f64> {
    if output_tokens == 0 || generation_window_ms == 0 {
        return None;
    }
    Some(output_tokens as f64 * 1000.0 / generation_window_ms as f64)
}

/// 初始化数据库（幂等，可在应用启动和首次写入时调用）
pub fn init_db() -> Result<(), String> {
    let conn = init_connection()?;
    // 将连接存入全局池
    *DB_CONN.lock().map_err(|e| format!("DB锁错误: {}", e))? = Some(conn);
    Ok(())
}

/// 获取数据库连接（首次调用时自动初始化；检查 + 初始化在同一锁临界区，避免并发重复初始化）
fn get_db() -> Result<(), String> {
    let mut guard = DB_CONN.lock().map_err(|e| format!("DB锁错误: {}", e))?;
    if guard.is_none() {
        let conn = init_connection()?;
        *guard = Some(conn);
    }
    Ok(())
}

/// 一次用量落库的完整维度。
///
/// 参考 ai-toolbox「per-model success and cache-hit rates with aligned stats tables」：
/// 成功率与缓存命中率都要求「失败请求」和「缓存 token」这两个维度存在，
/// 否则只能算 token 总量。参数收成结构体，避免位置参数继续膨胀。
#[derive(Debug, Clone)]
pub struct UsageEntry<'a> {
    pub tool_id: &'a str,
    pub model: &'a str,
    pub provider: Option<&'a str>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub duration_ms: u64,
    pub first_token_ms: u64,
    pub ok: bool,
    pub failure_class: Option<&'a str>,
}

impl<'a> UsageEntry<'a> {
    /// 成功请求的起点（token / 耗时 / 缓存默认 0，由链式方法补齐）。
    pub fn success(tool_id: &'a str, model: &'a str, provider: Option<&'a str>) -> Self {
        Self {
            tool_id,
            model,
            provider,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            duration_ms: 0,
            first_token_ms: 0,
            ok: true,
            failure_class: None,
        }
    }

    /// 失败请求：token 记 0，只贡献成功率的分母。
    pub fn failure(
        tool_id: &'a str,
        model: &'a str,
        provider: Option<&'a str>,
        failure_class: &'a str,
    ) -> Self {
        Self {
            ok: false,
            failure_class: Some(failure_class),
            ..Self::success(tool_id, model, provider)
        }
    }

    pub fn tokens(mut self, input_tokens: u64, output_tokens: u64) -> Self {
        self.input_tokens = input_tokens;
        self.output_tokens = output_tokens;
        self
    }

    pub fn cache(mut self, cache_read_tokens: u64, cache_write_tokens: u64) -> Self {
        self.cache_read_tokens = cache_read_tokens;
        self.cache_write_tokens = cache_write_tokens;
        self
    }

    pub fn timing(mut self, duration_ms: u64, first_token_ms: u64) -> Self {
        self.duration_ms = duration_ms;
        self.first_token_ms = first_token_ms;
        self
    }
}

/// 成功率 = 成功请求 / 总请求。没有请求时返回 None（前端留空，而不是显示 0%）。
pub fn success_rate(ok_count: u64, total_count: u64) -> Option<f64> {
    if total_count == 0 {
        return None;
    }
    Some(ok_count as f64 / total_count as f64)
}

/// 缓存命中率 = cache_read / (input + cache_read)。
///
/// **只统计上报过缓存的请求**（`reported_input_tokens` 是这些请求的输入 token 之和）：
/// 把没上报缓存的供应商一起算进去会把命中率系统性拉低，等于给用户看一个错的数。
pub fn cache_hit_rate(cache_read_tokens: u64, reported_input_tokens: u64) -> Option<f64> {
    let denominator = reported_input_tokens + cache_read_tokens;
    if denominator == 0 {
        return None;
    }
    Some(cache_read_tokens as f64 / denominator as f64)
}

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

/// 写入一条用量记录（唯一的 INSERT 出口）。
pub fn log_usage_entry(entry: &UsageEntry) -> Result<(), String> {
    get_db()?;
    let timestamp = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    let mut guard = DB_CONN.lock().map_err(|e| format!("DB锁错误: {}", e))?;
    let conn = guard.as_mut().ok_or("数据库未初始化")?;
    conn.execute(
        "INSERT INTO ai_usage (tool_id, model, provider, input_tokens, output_tokens, duration_ms, first_token_ms, ok, failure_class, cache_read_tokens, cache_write_tokens, timestamp) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        rusqlite::params![
            entry.tool_id,
            entry.model,
            entry.provider,
            entry.input_tokens,
            entry.output_tokens,
            entry.duration_ms as i64,
            entry.first_token_ms as i64,
            entry.ok as i64,
            entry.failure_class,
            entry.cache_read_tokens as i64,
            entry.cache_write_tokens as i64,
            timestamp,
        ],
    )
    .map_err(|e| format!("插入用量记录失败: {}", e))?;
    Ok(())
}

/// 记录一次**失败**尝试（上游报错 / 路由放弃 / 网络异常）。token 记 0，只进成功率分母。
pub fn log_usage_failure(
    tool_id: &str,
    model: &str,
    provider: Option<&str>,
    failure_class: &str,
) -> Result<(), String> {
    log_usage_entry(&UsageEntry::failure(tool_id, model, provider, failure_class))
}

/// 向数据库插入一条用量记录（线程安全），并带上耗时以支持输出速度统计。
///
/// `duration_ms` / `first_token_ms` 传 0 表示未测量（不经代理的直连调用方），
/// 这类记录不参与 t/s 聚合。口径见 [`output_tps`]。
pub fn log_usage_db_timed(
    tool_id: &str,
    model: &str,
    provider: Option<&str>,
    input_tokens: u64,
    output_tokens: u64,
    duration_ms: u64,
    first_token_ms: u64,
) -> Result<(), String> {
    log_usage_entry(
        &UsageEntry::success(tool_id, model, provider)
            .tokens(input_tokens, output_tokens)
            .timing(duration_ms, first_token_ms),
    )
}

/// 向数据库插入一条用量记录（无耗时信息，耗时字段落 0）。
pub fn log_usage_db(
    tool_id: &str,
    model: &str,
    provider: Option<&str>,
    input_tokens: u64,
    output_tokens: u64,
) -> Result<(), String> {
    log_usage_db_timed(tool_id, model, provider, input_tokens, output_tokens, 0, 0)
}

/// 从 OpenAI 风格 usage JSON 提取 token 数并落库（兼容 prompt/completion 与 input/output 两种命名）。
/// 供不经代理、直连共享 AI 通道（ai/channel.rs）的调用方使用：翻译、思维导图、API 智能导入等。
/// usage 缺失或 token 全为 0 时不记录（与代理侧「有用量才落库」的口径一致）。
pub fn log_usage_from_json(tool_id: &str, model: &str, provider_id: Option<&str>, usage: &serde_json::Value) {
    let in_t = usage
        .get("prompt_tokens")
        .or_else(|| usage.get("input_tokens"))
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let out_t = usage
        .get("completion_tokens")
        .or_else(|| usage.get("output_tokens"))
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let (cache_read, cache_write) = cache_tokens_from_json(usage);
    if in_t == 0 && out_t == 0 && cache_read == 0 {
        return;
    }
    let entry = UsageEntry::success(tool_id, model, provider_id)
        .tokens(in_t, out_t)
        .cache(cache_read, cache_write);
    if let Err(e) = log_usage_entry(&entry) {
        eprintln!("[ai-usage] 记录用量失败 (tool_id={}): {}", tool_id, e);
    }
}

/// 从数据库聚合查询用量摘要
pub fn get_usage_summary_db() -> Result<UsageSummary, String> {
    get_db()?;
    let mut guard = DB_CONN.lock().map_err(|e| format!("DB锁错误: {}", e))?;
    let conn = guard.as_mut().ok_or("数据库未初始化")?;

    // 总计（含成功数与缓存读取量，供顶部概览展示成功率）
    let (total_records, total_input, total_output, total_success, total_cache_read): (
        u64,
        u64,
        u64,
        u64,
        u64,
    ) = conn
        .query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(input_tokens), 0),
                    COALESCE(SUM(output_tokens), 0),
                    COALESCE(SUM(ok), 0),
                    COALESCE(SUM(cache_read_tokens), 0)
             FROM ai_usage",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)? as u64,
                    row.get::<_, i64>(1)? as u64,
                    row.get::<_, i64>(2)? as u64,
                    row.get::<_, i64>(3)? as u64,
                    row.get::<_, i64>(4)? as u64,
                ))
            },
        )
        .map_err(|e| format!("查询总计失败: {}", e))?;

    // by_tool
    let mut by_tool: Vec<UsageByTool> = Vec::new();
    let mut stmt = conn
        .prepare("SELECT tool_id, COUNT(*), COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0), COALESCE(SUM(input_tokens + output_tokens),0) FROM ai_usage GROUP BY tool_id ORDER BY SUM(input_tokens + output_tokens) DESC")
        .map_err(|e| format!("预处理 by_tool 失败: {}", e))?;
    let tool_iter = stmt
        .query_map([], |row| {
            Ok(UsageByTool {
                tool_id: row.get(0)?,
                request_count: row.get::<_, i64>(1)? as u64,
                input_tokens: row.get::<_, i64>(2)? as u64,
                output_tokens: row.get::<_, i64>(3)? as u64,
                total_tokens: row.get::<_, i64>(4)? as u64,
            })
        })
        .map_err(|e| format!("查询 by_tool 失败: {}", e))?;
    for tool in tool_iter {
        if let Ok(t) = tool {
            by_tool.push(t);
        }
    }

    // by_model（附带输出速度：仅聚合测得耗时的记录，未测耗时的记录不参与）
    let mut by_model: Vec<UsageByModel> = Vec::new();
    let mut stmt = conn
        .prepare("SELECT model, COALESCE(provider, ''), COUNT(*), COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0), COALESCE(SUM(input_tokens + output_tokens),0), COALESCE(SUM(CASE WHEN duration_ms > 0 THEN duration_ms - first_token_ms ELSE 0 END),0), COALESCE(SUM(CASE WHEN duration_ms > 0 THEN output_tokens ELSE 0 END),0), COALESCE(SUM(ok),0), max(0, COALESCE(SUM(ok),0) - COUNT(*)), COALESCE(SUM(cache_read_tokens),0), COALESCE(SUM(CASE WHEN cache_read_tokens > 0 OR cache_write_tokens > 0 THEN input_tokens ELSE 0 END),0) FROM ai_usage GROUP BY model, provider ORDER BY SUM(input_tokens + output_tokens) DESC, COUNT(*) DESC")
        .map_err(|e| format!("预处理 by_model 失败: {}", e))?;
    let model_iter = stmt
        .query_map([], |row| {
            // 生成窗口 = Σ(总耗时 − 首字延迟)；非流式请求 first_token_ms 为 0，窗口即总耗时
            let generation_window_ms = row.get::<_, i64>(6)?.max(0) as u64;
            let measured_output = row.get::<_, i64>(7)?.max(0) as u64;
            let request_count = row.get::<_, i64>(2)? as u64;
            // 成功数 = Σ ok；失败数 = 总请求 − 成功（老库补列后默认 1，不会少算成功）
            let ok_count = row.get::<_, i64>(8)?.max(0) as u64;
            let failure_count = row.get::<_, i64>(9)?.max(0) as u64;
            let cache_read = row.get::<_, i64>(10)?.max(0) as u64;
            let reported_input = row.get::<_, i64>(11)?.max(0) as u64;
            Ok(UsageByModel {
                model: row.get(0)?,
                provider: row.get(1)?,
                request_count,
                input_tokens: row.get::<_, i64>(3)? as u64,
                output_tokens: row.get::<_, i64>(4)? as u64,
                total_tokens: row.get::<_, i64>(5)? as u64,
                output_tps: output_tps(measured_output, generation_window_ms),
                success_count: ok_count,
                failure_count,
                success_rate: success_rate(ok_count, request_count),
                cache_read_tokens: cache_read,
                cache_hit_rate: cache_hit_rate(cache_read, reported_input),
            })
        })
        .map_err(|e| format!("查询 by_model 失败: {}", e))?;
    for model in model_iter {
        if let Ok(m) = model {
            by_model.push(m);
        }
    }

    // by_provider
    let mut by_provider: Vec<UsageByProvider> = Vec::new();
    let mut stmt = conn
        .prepare("SELECT COALESCE(provider, ''), COUNT(*), COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0), COALESCE(SUM(input_tokens + output_tokens),0) FROM ai_usage GROUP BY provider ORDER BY SUM(input_tokens + output_tokens) DESC")
        .map_err(|e| format!("预处理 by_provider 失败: {}", e))?;
    let provider_iter = stmt
        .query_map([], |row| {
            Ok(UsageByProvider {
                provider: row.get(0)?,
                request_count: row.get::<_, i64>(1)? as u64,
                input_tokens: row.get::<_, i64>(2)? as u64,
                output_tokens: row.get::<_, i64>(3)? as u64,
                total_tokens: row.get::<_, i64>(4)? as u64,
            })
        })
        .map_err(|e| format!("查询 by_provider 失败: {}", e))?;
    for p in provider_iter {
        if let Ok(p) = p {
            by_provider.push(p);
        }
    }

    // daily（最近）
    let mut daily: Vec<UsageDaily> = Vec::new();
    let mut stmt = conn
        .prepare("SELECT substr(timestamp, 1, 10) as date, COUNT(*), COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0), COALESCE(SUM(input_tokens + output_tokens),0) FROM ai_usage GROUP BY date ORDER BY date ASC")
        .map_err(|e| format!("预处理 daily 失败: {}", e))?;
    let daily_iter = stmt
        .query_map([], |row| {
            Ok(UsageDaily {
                date: row.get(0)?,
                request_count: row.get::<_, i64>(1)? as u64,
                input_tokens: row.get::<_, i64>(2)? as u64,
                output_tokens: row.get::<_, i64>(3)? as u64,
                total_tokens: row.get::<_, i64>(4)? as u64,
            })
        })
        .map_err(|e| format!("查询 daily 失败: {}", e))?;
    for d in daily_iter {
        if let Ok(day) = d {
            daily.push(day);
        }
    }

    Ok(UsageSummary {
        total_records,
        total_input_tokens: total_input,
        total_output_tokens: total_output,
        total_tokens: total_input + total_output,
        total_success,
        total_failure: total_records.saturating_sub(total_success),
        total_cache_read_tokens: total_cache_read,
        by_tool,
        by_model,
        by_provider,
        recent: daily,
    })
}

/// 清空所有用量记录
pub fn clear_usage_db() -> Result<(), String> {
    get_db()?;
    let mut guard = DB_CONN.lock().map_err(|e| format!("DB锁错误: {}", e))?;
    let conn = guard.as_mut().ok_or("数据库未初始化")?;
    conn.execute("DELETE FROM ai_usage", [])
        .map_err(|e| format!("清空用量失败: {}", e))?;
    Ok(())
}

// ─── Tauri 命令 ───

#[tauri::command]
pub fn record_usage(tool_id: String, model: String, provider: Option<String>, input_tokens: u64, output_tokens: u64) -> Result<(), String> {
    log_usage_db(&tool_id, &model, provider.as_deref(), input_tokens, output_tokens)
}

#[tauri::command]
pub fn get_usage_summary() -> Result<UsageSummary, String> {
    get_usage_summary_db()
}

#[tauri::command]
pub fn clear_usage() -> Result<(), String> {
    clear_usage_db()
}

#[cfg(test)]
mod tests {
    use super::{cache_hit_rate, cache_tokens_from_json, output_tps, success_rate};

    #[test]
    fn output_tps_uses_generation_window() {
        // 1000 token / 10s → 100 t/s
        let tps = output_tps(1000, 10_000).expect("measured");
        assert!((tps - 100.0).abs() < 1e-6, "got {}", tps);
    }

    #[test]
    fn output_tps_is_none_without_measurable_window() {
        // 没有输出 token / 没有耗时（未测量）→ 不给出数值
        assert_eq!(output_tps(0, 10_000), None);
        assert_eq!(output_tps(1000, 0), None);
    }

    #[test]
    fn success_rate_counts_failures_in_denominator() {
        assert_eq!(success_rate(0, 0), None, "没有请求时不显示 0%");
        assert_eq!(success_rate(3, 4), Some(0.75));
        assert_eq!(success_rate(0, 2), Some(0.0), "全失败要显示 0% 而不是留空");
    }

    #[test]
    fn cache_hit_rate_ignores_providers_without_cache_reporting() {
        // 只统计上报过缓存的请求：命中 800，这些请求输入 200 → 80%
        let rate = cache_hit_rate(800, 200).expect("reported");
        assert!((rate - 0.8).abs() < 1e-9, "got {}", rate);
        // 完全没上报缓存 → 留空，而不是显示 0%
        assert_eq!(cache_hit_rate(0, 0), None);
        // 命中率 0%（上报了缓存但一次未命中）要显示 0%
        assert_eq!(cache_hit_rate(0, 500), Some(0.0));
    }

    #[test]
    fn cache_tokens_reads_both_protocol_shapes() {
        let anthropic = serde_json::json!({
            "input_tokens": 120,
            "cache_read_input_tokens": 800,
            "cache_creation_input_tokens": 64
        });
        assert_eq!(cache_tokens_from_json(&anthropic), (800, 64));

        let openai = serde_json::json!({
            "prompt_tokens": 900,
            "prompt_tokens_details": { "cached_tokens": 512 }
        });
        assert_eq!(cache_tokens_from_json(&openai), (512, 0));

        // 未上报缓存的供应商 → 全 0，不参与命中率
        let plain = serde_json::json!({ "prompt_tokens": 900, "completion_tokens": 10 });
        assert_eq!(cache_tokens_from_json(&plain), (0, 0));
    }
}
