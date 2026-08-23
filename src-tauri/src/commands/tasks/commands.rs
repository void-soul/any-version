use std::collections::HashSet;

use rusqlite::params;

use super::db::{new_id, now_ts, today, with_conn};
use super::models::*;

// ─── 内部工具 ───

/// 从查询行构造 TaskItem（列顺序必须与 TASK_COLUMNS 一致）。
const TASK_COLUMNS: &str = "id, title, description, scheduled_date, parent_id, color, position_x, position_y, priority, progress, \
     sort_order, estimate_minutes, tags, archived, created_at, updated_at, completed_at";

fn row_to_task(row: &rusqlite::Row) -> rusqlite::Result<TaskItem> {
    Ok(TaskItem {
        id: row.get(0)?,
        title: row.get(1)?,
        description: row.get(2)?,
        scheduled_date: row.get(3)?,
        parent_id: row.get(4)?,
        color: row.get(5)?,
        position_x: row.get(6)?,
        position_y: row.get(7)?,
        priority: row.get(8)?,
        progress: row.get(9)?,
        sort_order: row.get(10)?,
        estimate_minutes: row.get(11)?,
        tags: row.get(12)?,
        archived: row.get::<_, i64>(13)? != 0,
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
        completed_at: row.get(16)?,
    })
}

fn status_str(progress: i64) -> &'static str {
    match derive_status(progress) {
        TaskStatus::Done => "done",
        TaskStatus::InProgress => "inProgress",
        TaskStatus::Todo => "todo",
    }
}

fn clamp_progress(p: i64) -> i64 {
    p.clamp(0, 100)
}

/// 校验父任务存在且不会形成自引用或多级循环。
fn validate_parent(
    conn: &rusqlite::Connection,
    task_id: Option<&str>,
    parent_id: &Option<String>,
) -> Result<(), String> {
    let Some(parent) = parent_id.as_deref() else {
        return Ok(());
    };
    if task_id == Some(parent) {
        return Err("任务不能设为自己的父任务".into());
    }

    let parent_exists: i64 = conn
        .query_row("SELECT COUNT(*) FROM tasks WHERE id = ?1", params![parent], |row| row.get(0))
        .map_err(|e| format!("校验父任务失败: {}", e))?;
    if parent_exists == 0 {
        return Err("父任务不存在".into());
    }

    let mut seen = HashSet::new();
    let mut current = Some(parent.to_string());
    while let Some(id) = current {
        if !seen.insert(id.clone()) {
            return Err("父任务关系存在循环".into());
        }
        let next: Option<String> = conn
            .query_row("SELECT parent_id FROM tasks WHERE id = ?1", params![id], |row| row.get(0))
            .map_err(|e| format!("校验父任务失败: {}", e))?;
        if task_id == Some(id.as_str()) {
            return Err("父任务关系存在循环".into());
        }
        current = next;
    }
    Ok(())
}

/// 计算某天末尾的排序值（追加到末尾）。
fn next_sort_order(conn: &rusqlite::Connection, date: &Option<String>) -> i64 {
    let res: rusqlite::Result<i64> = match date {
        Some(d) => conn.query_row(
            "SELECT COALESCE(MAX(sort_order), 0) + 10 FROM tasks WHERE scheduled_date = ?1",
            params![d],
            |r| r.get(0),
        ),
        None => conn.query_row(
            "SELECT COALESCE(MAX(sort_order), 0) + 10 FROM tasks WHERE scheduled_date IS NULL",
            [],
            |r| r.get(0),
        ),
    };
    res.unwrap_or(10)
}

/// 写一条转移记录。
#[allow(clippy::too_many_arguments)]
fn insert_move(
    conn: &rusqlite::Connection,
    task_id: &str,
    from_progress: i64,
    from_date: &Option<String>,
    to_progress: i64,
    to_date: &Option<String>,
    reason: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO task_moves (id, task_id, from_status, from_progress, from_date, \
         to_status, to_progress, to_date, reason, moved_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            new_id("mv"),
            task_id,
            status_str(from_progress),
            from_progress,
            from_date,
            status_str(to_progress),
            to_progress,
            to_date,
            reason,
            now_ts(),
        ],
    )
    .map_err(|e| format!("写入转移记录失败: {}", e))?;
    Ok(())
}

fn log_references(conn: &rusqlite::Connection, log_id: &str) -> Result<Vec<TaskLogReference>, String> {
    let mut stmt = conn
        .prepare("SELECT id, log_id, kind, target, label FROM task_log_references WHERE log_id = ?1 ORDER BY rowid ASC")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![log_id], |row| Ok(TaskLogReference {
            id: row.get(0)?,
            log_id: row.get(1)?,
            kind: row.get(2)?,
            target: row.get(3)?,
            label: row.get(4)?,
        }))
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| format!("读取日志引用失败: {}", e))
}

fn insert_log_references(conn: &rusqlite::Connection, log_id: &str, references: &[TaskLogReferenceInput]) -> Result<(), String> {
    for reference in references {
        let kind = reference.kind.trim();
        let target = reference.target.trim();
        if target.is_empty() || !matches!(kind, "file" | "image" | "picky") {
            continue;
        }
        conn.execute(
            "INSERT INTO task_log_references (id, log_id, kind, target, label) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![new_id("ref"), log_id, kind, target, reference.label.trim()],
        ).map_err(|e| format!("写入日志引用失败: {}", e))?;
    }
    Ok(())
}

fn fetch_task(conn: &rusqlite::Connection, id: &str) -> Result<TaskItem, String> {
    conn.query_row(
        &format!("SELECT {} FROM tasks WHERE id = ?1", TASK_COLUMNS),
        params![id],
        row_to_task,
    )
    .map_err(|e| format!("任务不存在: {}", e))
}

// ─── 查询 ───

/// 列出指定日期的任务（date 为空则返回未排期的 Inbox 任务）。
#[tauri::command]
pub fn tasks_list_by_date(date: Option<String>, include_archived: Option<bool>) -> Result<Vec<TaskItem>, String> {
    let include_archived = include_archived.unwrap_or(false);
    with_conn(|conn| {
        let arch_cond = if include_archived { "" } else { " AND archived = 0" };
        let (sql, has_date) = match &date {
            Some(_) => (
                format!(
                    "SELECT {} FROM tasks WHERE scheduled_date = ?1{} \
                     ORDER BY progress >= 100, sort_order ASC, created_at ASC",
                    TASK_COLUMNS, arch_cond
                ),
                true,
            ),
            None => (
                format!(
                    "SELECT {} FROM tasks WHERE scheduled_date IS NULL{} \
                     ORDER BY sort_order ASC, created_at ASC",
                    TASK_COLUMNS, arch_cond
                ),
                false,
            ),
        };
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = if has_date {
            stmt.query_map(params![date.as_ref().unwrap()], row_to_task)
        } else {
            stmt.query_map([], row_to_task)
        }
        .map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| format!("读取任务失败: {}", e))
    })
}

/// 列出日期区间内的任务（用于日历/复盘）。
#[tauri::command]
pub fn tasks_list_range(start: String, end: String) -> Result<Vec<TaskItem>, String> {
    with_conn(|conn| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {} FROM tasks WHERE archived = 0 AND scheduled_date IS NOT NULL \
                 AND scheduled_date >= ?1 AND scheduled_date <= ?2 \
                 ORDER BY scheduled_date ASC, sort_order ASC",
                TASK_COLUMNS
            ))
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![start, end], row_to_task)
            .map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| format!("读取任务失败: {}", e))
    })
}

/// 搜索任务（标题/描述/标签）。
#[tauri::command]
pub fn tasks_search(keyword: String, include_archived: Option<bool>) -> Result<Vec<TaskItem>, String> {
    let include_archived = include_archived.unwrap_or(false);
    with_conn(|conn| {
        let like = format!("%{}%", keyword.trim());
        let arch_cond = if include_archived { "" } else { " AND archived = 0" };
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {} FROM tasks WHERE (title LIKE ?1 OR description LIKE ?1 OR tags LIKE ?1){} \
                 ORDER BY scheduled_date DESC NULLS LAST, sort_order ASC LIMIT 300",
                TASK_COLUMNS, arch_cond
            ))
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map(params![like], row_to_task).map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| format!("搜索任务失败: {}", e))
    })
}

/// 查找所有"逾期未完成"的任务（计划日期早于今天且进度 < 100）。
#[tauri::command]
pub fn tasks_list_overdue(before: Option<String>) -> Result<Vec<TaskItem>, String> {
    let before = before.unwrap_or_else(today);
    with_conn(|conn| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {} FROM tasks WHERE archived = 0 AND progress < 100 \
                 AND scheduled_date IS NOT NULL AND scheduled_date < ?1 \
                 ORDER BY scheduled_date ASC, sort_order ASC",
                TASK_COLUMNS
            ))
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map(params![before], row_to_task).map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| format!("读取逾期任务失败: {}", e))
    })
}

// ─── 写入 ───

/// 创建任务。
#[tauri::command]
pub fn tasks_create(input: CreateTaskInput) -> Result<TaskItem, String> {
    let title = input.title.trim().to_string();
    if title.is_empty() {
        return Err("任务标题不能为空".into());
    }
    with_conn(move |conn| {
        let id = new_id("task");
        let ts = now_ts();
        let progress = clamp_progress(input.progress);        let sort_order = next_sort_order(conn, &input.scheduled_date);
        let parent_id = input.parent_id.filter(|parent| !parent.trim().is_empty());
        validate_parent(conn, Some(&id), &parent_id)?;
        let completed_at = if progress >= 100 { Some(ts.clone()) } else { None };

        conn.execute(
            "INSERT INTO tasks (id, title, description, scheduled_date, parent_id, color, position_x, position_y, priority, progress, \
             sort_order, estimate_minutes, tags, archived, created_at, updated_at, completed_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 0, ?14, ?15, ?16)",
            params![
                id,
                title,
                input.description,
                input.scheduled_date,
                parent_id,
                input.color,
                input.position_x,
                input.position_y,
                input.priority,
                progress,
                sort_order,
                input.estimate_minutes,
                input.tags,
                ts,
                ts,
                completed_at,
            ],
        )
        .map_err(|e| format!("创建任务失败: {}", e))?;
        fetch_task(conn, &id)
    })
}

/// 更新任务字段（部分更新）。若包含 progress 变化，会写入转移记录。
#[tauri::command]
pub fn tasks_update(id: String, input: UpdateTaskInput) -> Result<TaskItem, String> {
    with_conn(move |conn| {
        let old = fetch_task(conn, &id)?;

        let title = input.title.unwrap_or(old.title.clone());
        if title.trim().is_empty() {
            return Err("任务标题不能为空".into());
        }
        let description = input.description.unwrap_or(old.description.clone());
        let scheduled_date = match input.scheduled_date {
            Some(v) => v,
            None => old.scheduled_date.clone(),
        };
        let parent_id = match input.parent_id {
            Some(v) => v.filter(|parent| !parent.trim().is_empty()),
            None => old.parent_id.clone(),
        };
        validate_parent(conn, Some(&id), &parent_id)?;
        let color = input.color.unwrap_or(old.color.clone());
        let position_x = input.position_x.unwrap_or(old.position_x);
        let position_y = input.position_y.unwrap_or(old.position_y);
        let priority = input.priority.unwrap_or(old.priority.clone());
        let estimate = input.estimate_minutes.unwrap_or(old.estimate_minutes);
        let tags = input.tags.unwrap_or(old.tags.clone());
        let archived = input.archived.unwrap_or(old.archived);
        let progress = clamp_progress(input.progress.unwrap_or(old.progress));
        let sort_order = match input.sort_order {
            Some(v) => v,
            None => {
                // 换日期时重排到目标日期末尾
                if scheduled_date != old.scheduled_date {
                    next_sort_order(conn, &scheduled_date)
                } else {
                    old.sort_order
                }
            }
        };

        // completed_at 由 progress 派生，避免出现"未完成却有完成时间"。
        let completed_at = if progress >= 100 {
            old.completed_at.clone().or_else(|| Some(now_ts()))
        } else {
            None
        };

        conn.execute(
            "UPDATE tasks SET title = ?1, description = ?2, scheduled_date = ?3, parent_id = ?4, color = ?5, position_x = ?6, position_y = ?7, priority = ?8, \
             progress = ?9, sort_order = ?10, estimate_minutes = ?11, tags = ?12, archived = ?13, \
             updated_at = ?14, completed_at = ?15 WHERE id = ?16",
            params![
                title.trim(),
                description,
                scheduled_date,
                parent_id,
                color,
                position_x,
                position_y,
                priority,
                progress,
                sort_order,
                estimate,
                tags,
                if archived { 1 } else { 0 },
                now_ts(),
                completed_at,
                id,
            ],
        )
        .map_err(|e| format!("更新任务失败: {}", e))?;

        if progress != old.progress || scheduled_date != old.scheduled_date {
            insert_move(
                conn,
                &id,
                old.progress,
                &old.scheduled_date,
                progress,
                &scheduled_date,
                "编辑任务",
            )?;
        }
        fetch_task(conn, &id)
    })
}

/// 设置进度：progress 是完成度唯一真相来源，status 与 completedAt 全部由它派生。
/// 可选同时写复盘日志；进度未满且指定 carryToDate 时，自动结转到该日期。
#[tauri::command]
pub fn tasks_set_progress(id: String, input: SetProgressInput) -> Result<TaskItem, String> {
    with_conn(move |conn| {
        let old = fetch_task(conn, &id)?;
        let new_progress = clamp_progress(input.progress);

        // 未完成才允许结转；已完成的任务保持原日期。
        let new_date = match (&input.carry_to_date, new_progress >= 100) {
            (Some(d), false) => Some(d.clone()),
            _ => old.scheduled_date.clone(),
        };

        let completed_at = if new_progress >= 100 {
            old.completed_at.clone().or_else(|| Some(now_ts()))
        } else {
            None
        };
        let sort_order = if new_date != old.scheduled_date {
            next_sort_order(conn, &new_date)
        } else {
            old.sort_order
        };

        conn.execute(
            "UPDATE tasks SET progress = ?1, scheduled_date = ?2, sort_order = ?3, \
             updated_at = ?4, completed_at = ?5 WHERE id = ?6",
            params![new_progress, new_date, sort_order, now_ts(), completed_at, id],
        )
        .map_err(|e| format!("更新进度失败: {}", e))?;

        // 有日志内容、投入工时或引用就记一条复盘日志
        let content = input.log_content.unwrap_or_default();
        if !content.trim().is_empty() || input.minutes_spent > 0 || !input.references.is_empty() {
            let log_id = new_id("log");
            conn.execute(
                "INSERT INTO task_logs (id, task_id, log_date, content, progress_before, \
                 progress_after, minutes_spent, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    log_id,
                    id,
                    today(),
                    content.trim(),
                    old.progress,
                    new_progress,
                    input.minutes_spent,
                    now_ts(),
                ],
            )
            .map_err(|e| format!("写入日志失败: {}", e))?;
            insert_log_references(conn, &log_id, &input.references)?;
        }

        if new_progress != old.progress || new_date != old.scheduled_date {
            let reason = input
                .move_reason
                .clone()
                .filter(|r| !r.trim().is_empty())
                .unwrap_or_else(|| {
                    if new_date != old.scheduled_date {
                        "结转顺延".to_string()
                    } else {
                        "更新进度".to_string()
                    }
                });
            insert_move(conn, &id, old.progress, &old.scheduled_date, new_progress, &new_date, &reason)?;
        }
        fetch_task(conn, &id)
    })
}

/// 转移任务到其它日期（结转/顺延）。
#[tauri::command]
pub fn tasks_move(id: String, input: MoveTaskInput) -> Result<TaskItem, String> {
    with_conn(move |conn| {
        let old = fetch_task(conn, &id)?;
        // Some(x) 表示前端显式指定了目标（x 为 None 即移入收集箱）；None 表示不改日期。
        let to_date = match input.to_date {
            Some(d) => d,
            None => old.scheduled_date.clone(),
        };
        let to_progress = clamp_progress(input.to_progress.unwrap_or(old.progress));
        let completed_at = if to_progress >= 100 {
            old.completed_at.clone().or_else(|| Some(now_ts()))
        } else {
            None
        };
        let sort_order = if to_date != old.scheduled_date {
            next_sort_order(conn, &to_date)
        } else {
            old.sort_order
        };

        conn.execute(
            "UPDATE tasks SET scheduled_date = ?1, progress = ?2, sort_order = ?3, \
             updated_at = ?4, completed_at = ?5 WHERE id = ?6",
            params![to_date, to_progress, sort_order, now_ts(), completed_at, id],
        )
        .map_err(|e| format!("转移任务失败: {}", e))?;

        let reason = if input.reason.trim().is_empty() {
            "手动转移".to_string()
        } else {
            input.reason.trim().to_string()
        };
        insert_move(conn, &id, old.progress, &old.scheduled_date, to_progress, &to_date, &reason)?;
        fetch_task(conn, &id)
    })
}

/// 批量把逾期未完成的任务结转到指定日期（默认今天）。返回结转条数。
#[tauri::command]
pub fn tasks_carry_over(to_date: Option<String>) -> Result<i64, String> {
    let to_date = to_date.unwrap_or_else(today);
    with_conn(move |conn| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {} FROM tasks WHERE archived = 0 AND progress < 100 \
                 AND scheduled_date IS NOT NULL AND scheduled_date < ?1",
                TASK_COLUMNS
            ))
            .map_err(|e| e.to_string())?;
        let list: Vec<TaskItem> = stmt
            .query_map(params![to_date], row_to_task)
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        drop(stmt);

        let mut count = 0i64;
        for t in list {
            let sort_order = next_sort_order(conn, &Some(to_date.clone()));
            conn.execute(
                "UPDATE tasks SET scheduled_date = ?1, sort_order = ?2, updated_at = ?3 WHERE id = ?4",
                params![to_date, sort_order, now_ts(), t.id],
            )
            .map_err(|e| format!("结转失败: {}", e))?;
            insert_move(
                conn,
                &t.id,
                t.progress,
                &t.scheduled_date,
                t.progress,
                &Some(to_date.clone()),
                "逾期自动结转",
            )?;
            count += 1;
        }
        Ok(count)
    })
}

/// 批量重排某一天内的任务顺序。
#[tauri::command]
pub fn tasks_reorder(input: ReorderInput) -> Result<(), String> {
    with_conn(move |conn| {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        for (idx, id) in input.ids.iter().enumerate() {
            tx.execute(
                "UPDATE tasks SET sort_order = ?1, updated_at = ?2 WHERE id = ?3",
                params![(idx as i64 + 1) * 10, now_ts(), id],
            )
            .map_err(|e| format!("重排失败: {}", e))?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
}

/// 启动后的今日待办提醒数据：今日未完成（按 sort_order）+ 逾期未完成。
#[tauri::command]
pub fn tasks_today_reminder() -> Result<ReminderData, String> {
    let today = today();
    with_conn(move |conn| {
        let today_pending: Vec<TaskBrief> = conn
            .prepare(&format!(
                "SELECT {} FROM tasks \
                 WHERE archived = 0 AND scheduled_date = ?1 AND progress < 100 \
                 ORDER BY sort_order ASC, priority DESC",
                TASK_COLUMNS
            ))
            .map_err(|e| e.to_string())?
            .query_map(params![today], |row| Ok(row_to_task(row)?))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(TaskBrief::from_item)
            .collect();

        let overdue: Vec<TaskBrief> = conn
            .prepare(&format!(
                "SELECT {} FROM tasks \
                 WHERE archived = 0 AND scheduled_date < ?1 AND progress < 100 \
                 ORDER BY scheduled_date ASC, sort_order ASC",
                TASK_COLUMNS
            ))
            .map_err(|e| e.to_string())?
            .query_map(params![today], |row| Ok(row_to_task(row)?))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(TaskBrief::from_item)
            .collect();

        Ok(ReminderData {
            today,
            today_pending,
            overdue,
        })
    })
}

/// 归档（软删除）或恢复任务。
#[tauri::command]
pub fn tasks_set_archived(id: String, archived: bool) -> Result<(), String> {
    with_conn(move |conn| {
        conn.execute(
            "UPDATE tasks SET archived = ?1, updated_at = ?2 WHERE id = ?3",
            params![if archived { 1 } else { 0 }, now_ts(), id],
        )
        .map_err(|e| format!("归档任务失败: {}", e))?;
        Ok(())
    })
}

/// 彻底删除任务（连带日志与转移记录）。
#[tauri::command]
pub fn tasks_delete(id: String) -> Result<(), String> {
    with_conn(move |conn| {
        // 删除父任务不删除子任务，子任务自动提升为顶层任务。
        conn.execute("UPDATE tasks SET parent_id = NULL, updated_at = ?1 WHERE parent_id = ?2", params![now_ts(), id])
            .map_err(|e| format!("整理子任务关系失败: {}", e))?;
        conn.execute("DELETE FROM task_logs WHERE task_id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM task_moves WHERE task_id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM tasks WHERE id = ?1", params![id])
            .map_err(|e| format!("删除任务失败: {}", e))?;
        Ok(())
    })
}

// ─── 日志 / 转移记录 ───

/// 追加一条复盘日志（不改变进度）。
#[tauri::command]
pub fn tasks_add_log(input: AddLogInput) -> Result<TaskLog, String> {
    with_conn(move |conn| {
        let task = fetch_task(conn, &input.task_id)?;
        let id = new_id("log");
        conn.execute(
            "INSERT INTO task_logs (id, task_id, log_date, content, progress_before, progress_after, minutes_spent, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                id,
                input.task_id,
                input.log_date,
                input.content.trim(),
                task.progress,
                task.progress,
                input.minutes_spent,
                now_ts(),
            ],
        )
        .map_err(|e| format!("写入日志失败: {}", e))?;
        insert_log_references(conn, &id, &input.references)?;

        let base = conn.query_row(
            "SELECT id, task_id, log_date, content, progress_before, progress_after, minutes_spent, created_at \
             FROM task_logs WHERE id = ?1",
            params![id],
            |row| Ok((
                row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?,
                row.get::<_, String>(3)?, row.get::<_, i64>(4)?, row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?, row.get::<_, String>(7)?,
            )),
        ).map_err(|e| e.to_string())?;
        Ok(TaskLog {
            references: log_references(conn, &base.0)?,
            id: base.0,
            task_id: base.1,
            log_date: base.2,
            content: base.3,
            progress_before: base.4,
            progress_after: base.5,
            minutes_spent: base.6,
            created_at: base.7,
        })
    })
}

/// 查询某个任务的全部日志。
#[tauri::command]
pub fn tasks_list_logs(task_id: String) -> Result<Vec<TaskLog>, String> {
    with_conn(move |conn| {
        let base: Vec<(String, String, String, String, i64, i64, i64, String)> = {
            let mut stmt = conn
                .prepare(
                    "SELECT id, task_id, log_date, content, progress_before, progress_after, minutes_spent, created_at \
                     FROM task_logs WHERE task_id = ?1 ORDER BY created_at DESC",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt.query_map(params![task_id], |row| Ok((
                row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?,
                row.get::<_, String>(3)?, row.get::<_, i64>(4)?, row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?, row.get::<_, String>(7)?,
            ))).map_err(|e| e.to_string())?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| format!("读取日志失败: {}", e))?
        };
        base.into_iter().map(|item| Ok(TaskLog {
            references: log_references(conn, &item.0)?,
            id: item.0,
            task_id: item.1,
            log_date: item.2,
            content: item.3,
            progress_before: item.4,
            progress_after: item.5,
            minutes_spent: item.6,
            created_at: item.7,
        })).collect()
    })
}

/// 查询某个任务的转移记录（复盘"为什么一再顺延"）。
#[tauri::command]
pub fn tasks_list_moves(task_id: String) -> Result<Vec<TaskMoveRecord>, String> {
    with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, task_id, from_status, from_progress, from_date, to_status, \
                 to_progress, to_date, reason, moved_at FROM task_moves \
                 WHERE task_id = ?1 ORDER BY moved_at DESC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![task_id], |row| {
                Ok(TaskMoveRecord {
                    id: row.get(0)?,
                    task_id: row.get(1)?,
                    from_status: row.get(2)?,
                    from_progress: row.get(3)?,
                    from_date: row.get(4)?,
                    to_status: row.get(5)?,
                    to_progress: row.get(6)?,
                    to_date: row.get(7)?,
                    reason: row.get(8)?,
                    moved_at: row.get(9)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| format!("读取转移记录失败: {}", e))
    })
}

// ─── 统计 ───

/// 某天的汇总指标。
#[tauri::command]
pub fn tasks_summary(date: String) -> Result<TaskSummary, String> {
    with_conn(move |conn| {
        let (total, completed, in_progress, estimate): (i64, i64, i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), \
                 SUM(CASE WHEN progress >= 100 THEN 1 ELSE 0 END), \
                 SUM(CASE WHEN progress > 0 AND progress < 100 THEN 1 ELSE 0 END), \
                 COALESCE(SUM(estimate_minutes), 0) \
                 FROM tasks WHERE archived = 0 AND scheduled_date = ?1",
                params![date],
                |r| Ok((r.get(0)?, r.get::<_, Option<i64>>(1)?.unwrap_or(0), r.get::<_, Option<i64>>(2)?.unwrap_or(0), r.get(3)?)),
            )
            .map_err(|e| format!("统计失败: {}", e))?;

        let spent: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(minutes_spent), 0) FROM task_logs WHERE log_date = ?1",
                params![date],
                |r| r.get(0),
            )
            .unwrap_or(0);

        Ok(TaskSummary {
            date,
            total,
            completed,
            in_progress,
            pending: total - completed - in_progress,
            total_estimate: estimate,
            total_spent: spent,
        })
    })
}

/// 区间内按天聚合（复盘页折线/热力图）。
#[tauri::command]
pub fn tasks_day_stats(start: String, end: String) -> Result<Vec<DayStat>, String> {
    with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT scheduled_date, COUNT(*), \
                 SUM(CASE WHEN progress >= 100 THEN 1 ELSE 0 END), \
                 SUM(CASE WHEN progress > 0 AND progress < 100 THEN 1 ELSE 0 END) \
                 FROM tasks WHERE archived = 0 AND scheduled_date IS NOT NULL \
                 AND scheduled_date >= ?1 AND scheduled_date <= ?2 \
                 GROUP BY scheduled_date ORDER BY scheduled_date ASC",
            )
            .map_err(|e| e.to_string())?;
        let mut stats: Vec<DayStat> = stmt
            .query_map(params![start, end], |row| {
                let total: i64 = row.get(1)?;
                let completed: i64 = row.get::<_, Option<i64>>(2)?.unwrap_or(0);
                let in_progress: i64 = row.get::<_, Option<i64>>(3)?.unwrap_or(0);
                Ok(DayStat {
                    date: row.get(0)?,
                    total,
                    completed,
                    in_progress,
                    pending: total - completed - in_progress,
                    minutes_spent: 0,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        drop(stmt);

        // 合并每天实际投入工时
        let mut stmt2 = conn
            .prepare(
                "SELECT log_date, COALESCE(SUM(minutes_spent), 0) FROM task_logs \
                 WHERE log_date >= ?1 AND log_date <= ?2 GROUP BY log_date",
            )
            .map_err(|e| e.to_string())?;
        let spent: Vec<(String, i64)> = stmt2
            .query_map(params![start, end], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        for (d, m) in spent {
            if let Some(s) = stats.iter_mut().find(|s| s.date == d) {
                s.minutes_spent = m;
            }
        }
        Ok(stats)
    })
}

/// 初始化任务数据库（供启动时调用）。
#[tauri::command]
pub fn tasks_init() -> Result<(), String> {
    super::db::init_db()
}
