// 验证任务模块的 SQL 语句在 bundled SQLite 上可以正确执行。
// 重点验证：建表 DDL、NULLS LAST 语法、聚合统计 SQL。

fn setup() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE tasks (
            id TEXT PRIMARY KEY, title TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '', scheduled_date TEXT,
            priority TEXT NOT NULL DEFAULT 'medium', progress INTEGER NOT NULL DEFAULT 0,
            sort_order INTEGER NOT NULL DEFAULT 0, estimate_minutes INTEGER NOT NULL DEFAULT 0,
            tags TEXT NOT NULL DEFAULT '', archived INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL, updated_at TEXT NOT NULL, completed_at TEXT
        );
        CREATE TABLE task_logs (
            id TEXT PRIMARY KEY, task_id TEXT NOT NULL, log_date TEXT NOT NULL,
            content TEXT NOT NULL DEFAULT '', progress_before INTEGER NOT NULL DEFAULT 0,
            progress_after INTEGER NOT NULL DEFAULT 0, minutes_spent INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL
        );
        INSERT INTO tasks VALUES
          ('t1','A','',  '2026-07-31','high',   0, 10, 60,'', 0,'x','x',NULL),
          ('t2','B','',  '2026-07-31','medium',100,20, 30,'', 0,'x','x','x'),
          ('t3','C','',  '2026-07-31','low',    50,30,  0,'', 0,'x','x',NULL),
          ('t4','D','',  NULL,        'low',     0,40,  0,'', 0,'x','x',NULL);
        INSERT INTO task_logs VALUES ('l1','t3','2026-07-31','进展',0,50,45,'x');
        "#,
    )
    .unwrap();
    conn
}

/// 搜索 SQL 使用了 `NULLS LAST`，需确认 bundled SQLite 版本支持。
#[test]
fn search_sql_with_nulls_last_is_supported() {
    let conn = setup();
    let mut stmt = conn
        .prepare(
            "SELECT id FROM tasks WHERE (title LIKE ?1 OR description LIKE ?1 OR tags LIKE ?1) \
             AND archived = 0 ORDER BY scheduled_date DESC NULLS LAST, sort_order ASC LIMIT 300",
        )
        .expect("NULLS LAST 语法应被支持");
    let ids: Vec<String> = stmt
        .query_map(rusqlite::params!["%%"], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    // 未排期(t4)应排在最后
    assert_eq!(ids.last().unwrap(), "t4");
    assert_eq!(ids.len(), 4);
}

/// 汇总统计：SUM(CASE...) 在无匹配行时返回 NULL，代码里用 Option 兜底。
#[test]
fn summary_counts_by_status() {
    let conn = setup();
    let (total, completed, in_progress, estimate): (i64, i64, i64, i64) = conn
        .query_row(
            "SELECT COUNT(*), \
             SUM(CASE WHEN progress >= 100 THEN 1 ELSE 0 END), \
             SUM(CASE WHEN progress > 0 AND progress < 100 THEN 1 ELSE 0 END), \
             COALESCE(SUM(estimate_minutes), 0) \
             FROM tasks WHERE archived = 0 AND scheduled_date = ?1",
            rusqlite::params!["2026-07-31"],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                    r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                    r.get(3)?,
                ))
            },
        )
        .unwrap();
    assert_eq!((total, completed, in_progress, estimate), (3, 1, 1, 90));
}

/// 空结果集时 SUM 返回 NULL，必须能安全降级为 0。
#[test]
fn summary_on_empty_day_returns_zeros() {
    let conn = setup();
    let (total, completed): (i64, i64) = conn
        .query_row(
            "SELECT COUNT(*), SUM(CASE WHEN progress >= 100 THEN 1 ELSE 0 END) \
             FROM tasks WHERE archived = 0 AND scheduled_date = ?1",
            rusqlite::params!["2099-01-01"],
            |r| Ok((r.get(0)?, r.get::<_, Option<i64>>(1)?.unwrap_or(0))),
        )
        .unwrap();
    assert_eq!((total, completed), (0, 0));
}

/// 今日列表排序：已完成的沉底，其余按 sort_order。
#[test]
fn list_by_date_puts_done_last() {
    let conn = setup();
    let mut stmt = conn
        .prepare(
            "SELECT id FROM tasks WHERE scheduled_date = ?1 AND archived = 0 \
             ORDER BY progress >= 100, sort_order ASC, created_at ASC",
        )
        .unwrap();
    let ids: Vec<String> = stmt
        .query_map(rusqlite::params!["2026-07-31"], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(ids, vec!["t1", "t3", "t2"]);
}

/// 按天聚合 + 工时合并。
#[test]
fn day_stats_aggregates_minutes() {
    let conn = setup();
    let spent: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(minutes_spent), 0) FROM task_logs \
             WHERE log_date >= ?1 AND log_date <= ?2",
            rusqlite::params!["2026-07-01", "2026-07-31"],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(spent, 45);
}

/// 逾期查询：进度未满且计划日期早于给定日期。
#[test]
fn overdue_excludes_completed_and_unscheduled() {
    let conn = setup();
    let mut stmt = conn
        .prepare(
            "SELECT id FROM tasks WHERE archived = 0 AND progress < 100 \
             AND scheduled_date IS NOT NULL AND scheduled_date < ?1",
        )
        .unwrap();
    let ids: Vec<String> = stmt
        .query_map(rusqlite::params!["2026-08-01"], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    // t2 已完成、t4 未排期，均应排除
    assert_eq!(ids, vec!["t1", "t3"]);
}
