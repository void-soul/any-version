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
    get_db()?;
    let timestamp = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    let mut guard = DB_CONN.lock().map_err(|e| format!("DB锁错误: {}", e))?;
    let conn = guard.as_mut().ok_or("数据库未初始化")?;
    conn.execute(
        "INSERT INTO ai_usage (tool_id, model, provider, input_tokens, output_tokens, duration_ms, first_token_ms, timestamp) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            tool_id,
            model,
            provider,
            input_tokens,
            output_tokens,
            duration_ms as i64,
            first_token_ms as i64,
            timestamp,
        ],
    )
    .map_err(|e| format!("插入用量记录失败: {}", e))?;
    Ok(())
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
    if in_t == 0 && out_t == 0 {
        return;
    }
    if let Err(e) = log_usage_db(tool_id, model, provider_id, in_t, out_t) {
        eprintln!("[ai-usage] 记录用量失败 (tool_id={}): {}", tool_id, e);
    }
}

/// 从数据库聚合查询用量摘要
pub fn get_usage_summary_db() -> Result<UsageSummary, String> {
    get_db()?;
    let mut guard = DB_CONN.lock().map_err(|e| format!("DB锁错误: {}", e))?;
    let conn = guard.as_mut().ok_or("数据库未初始化")?;

    // 总计
    let (total_records, total_input, total_output): (u64, u64, u64) = conn
        .query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(input_tokens), 0),
                    COALESCE(SUM(output_tokens), 0)
             FROM ai_usage",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)? as u64,
                    row.get::<_, i64>(1)? as u64,
                    row.get::<_, i64>(2)? as u64,
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
        .prepare("SELECT model, COALESCE(provider, ''), COUNT(*), COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0), COALESCE(SUM(input_tokens + output_tokens),0), COALESCE(SUM(CASE WHEN duration_ms > 0 THEN duration_ms - first_token_ms ELSE 0 END),0), COALESCE(SUM(CASE WHEN duration_ms > 0 THEN output_tokens ELSE 0 END),0) FROM ai_usage GROUP BY model, provider ORDER BY SUM(input_tokens + output_tokens) DESC")
        .map_err(|e| format!("预处理 by_model 失败: {}", e))?;
    let model_iter = stmt
        .query_map([], |row| {
            // 生成窗口 = Σ(总耗时 − 首字延迟)；非流式请求 first_token_ms 为 0，窗口即总耗时
            let generation_window_ms = row.get::<_, i64>(6)?.max(0) as u64;
            let measured_output = row.get::<_, i64>(7)?.max(0) as u64;
            Ok(UsageByModel {
                model: row.get(0)?,
                provider: row.get(1)?,
                request_count: row.get::<_, i64>(2)? as u64,
                input_tokens: row.get::<_, i64>(3)? as u64,
                output_tokens: row.get::<_, i64>(4)? as u64,
                total_tokens: row.get::<_, i64>(5)? as u64,
                output_tps: output_tps(measured_output, generation_window_ms),
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
    use super::output_tps;

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
}
