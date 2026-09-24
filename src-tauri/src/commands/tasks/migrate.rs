//! 从思维导图迁移遗留的「计划」（一次性）。
//!
//! 背景：任务的排期原先存在**思维导图节点**上（`mindmap_nodes.plan_at` / `repeat`），
//! 日历也挂在思维导图模块里。现在任务计划独立成模块（本库 `tasks.db`），
//! 思维导图侧的计划功能整块移除 —— 所以这里把遗留的节点计划一次性搬过来，
//! 免得用户排好的安排凭空消失。
//!
//! 两条设计约束：
//! ① **幂等**：用 `task_meta` 里的一次性标记守住，只成功执行一次；
//! ② **不拖后腿**：迁移失败只记日志、不落标记（下次打开模块再试），
//!    绝不让「历史数据搬迁」把「任务计划模块能不能用」绑死。
//!
//! 迁移是**跨库读取**（只读打开 mindmap.db），因此直接用 SQL 读，不依赖思维导图模块的
//! Rust API —— 依赖方向保持单向（tasks → 读 mindmap 文件），不会形成模块环。

use rusqlite::Connection;

use super::db;
use crate::commands::config::get_data_dir;

/// 一次性迁移标记：存在即表示已跑过（值为 `日期/迁移条数`，便于排查）。
const MIGRATION_KEY: &str = "mindmap_plan_migration_v1";

/// 一条待迁移的旧计划（从 mindmap.db 读出的最小信息）。
struct LegacyPlan {
    node_id: String,
    name: String,
    date: String,
    repeat: String,
    doc_name: String,
    project_dir: String,
}

impl LegacyPlan {
    /// 任务标题：节点名优先；节点没名字（很常见，视图生成时常常留空）就用文档名兜底，
    /// 否则迁移出来的任务全叫「未命名」，等于没迁。
    fn title(&self) -> String {
        let name = self.name.trim();
        if !name.is_empty() {
            return name.to_string();
        }
        let doc = self.doc_name.trim();
        if doc.is_empty() {
            "思维导图计划".to_string()
        } else {
            format!("{} 的计划", doc)
        }
    }

    /// 简短来源说明（列表/日历上直接看得见这条从哪来）
    fn description(&self) -> String {
        let doc = self.doc_name.trim();
        if doc.is_empty() {
            "迁移自思维导图".to_string()
        } else {
            format!("迁移自思维导图：{}", doc)
        }
    }

    /// 长说明：节点 id / 项目目录 / 原重复规则，便于回溯原处
    fn detail(&self) -> String {
        let mut detail = format!("迁移自思维导图的节点计划（原节点 id：{}）", self.node_id);
        if !self.project_dir.trim().is_empty() {
            detail.push_str(&format!("\n原项目目录：{}", self.project_dir.trim()));
        }
        if self.repeat != "none" {
            detail.push_str(&format!(
                "\n原重复规则：{} —— 任务计划暂不支持重复，请按需要手动补排期",
                repeat_label(&self.repeat)
            ));
        }
        detail
    }
}

/// 把思维导图里带计划的节点迁移成任务；返回新建的任务数。已迁移过则返回 0。
pub fn migrate_mindmap_plans(conn: &Connection) -> Result<usize, String> {
    if db::get_meta(conn, MIGRATION_KEY)?.is_some() {
        return Ok(0);
    }
    let created = match run(conn) {
        Ok(count) => count,
        Err(e) => {
            eprintln!("[任务计划] 迁移思维导图旧计划失败（下次打开模块会重试）: {}", e);
            return Ok(0);
        }
    };
    db::set_meta(conn, MIGRATION_KEY, &format!("{}/{}", db::today(), created))?;
    if created > 0 {
        eprintln!("[任务计划] 已从思维导图迁移 {} 条旧计划", created);
    }
    Ok(created)
}

/// 真正的搬迁逻辑（可失败；失败时调用方不落标记）。
fn run(conn: &Connection) -> Result<usize, String> {
    let path = get_data_dir().join("mindmap.db");
    if !path.is_file() {
        return Ok(0); // 没装过思维导图：无事可做
    }
    let src = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("打开思维导图库失败: {}", e))?;
    // 老库可能既没有该表、也没有该列
    if !has_table(&src, "mindmap_nodes")? || !has_column(&src, "mindmap_nodes", "plan_at")? {
        return Ok(0);
    }
    let repeat_expr = if has_column(&src, "mindmap_nodes", "repeat")? {
        "n.repeat"
    } else {
        "'none'"
    };
    let sql = format!(
        "SELECT n.id, COALESCE(n.name, ''), n.plan_at, COALESCE({repeat_expr}, 'none'), \
                COALESCE(d.name, ''), COALESCE(d.project_dir, '') \
         FROM mindmap_nodes n \
         LEFT JOIN mindmap_documents d ON d.id = n.document_id \
         WHERE n.plan_at IS NOT NULL AND TRIM(n.plan_at) <> ''"
    );
    let mut stmt = src
        .prepare(&sql)
        .map_err(|e| format!("读取思维导图旧计划失败: {}", e))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(|e| format!("读取思维导图旧计划失败: {}", e))?;

    let mut legacy: Vec<LegacyPlan> = Vec::new();
    for row in rows {
        let (node_id, name, plan_at, repeat, doc_name, project_dir) =
            row.map_err(|e| format!("读取思维导图旧计划失败: {}", e))?;
        // 时间串取不出日期就跳过：宁可少迁一条，也不要往日历里塞一条日期非法的任务
        let Some(date) = date_part(&plan_at) else {
            continue;
        };
        legacy.push(LegacyPlan { node_id, name, date, repeat, doc_name, project_dir });
    }
    drop(stmt);
    drop(src);

    let ts = db::now_ts();
    let mut created = 0usize;
    for plan in legacy {
        let id = db::new_id("task");
        // 同一天内排在同日既有任务之后（与 tasks_create 的 next_sort_order 同口径）
        let sort_order: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(sort_order), 0) + 1 FROM tasks WHERE scheduled_date = ?1",
                [&plan.date],
                |row| row.get(0),
            )
            .unwrap_or(0);
        conn.execute(
            "INSERT INTO tasks (id, title, description, detail, scheduled_date, parent_id, color, \
             position_x, position_y, priority, progress, sort_order, estimate_minutes, tags, archived, \
             created_at, updated_at, completed_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, '#f59e0b', 0, 0, 'medium', 0, ?6, 0, ?7, 0, ?8, ?8, NULL)",
            rusqlite::params![
                id,
                plan.title(),
                plan.description(),
                plan.detail(),
                plan.date,
                sort_order,
                "迁移自思维导图",
                ts,
            ],
        )
        .map_err(|e| format!("写入迁移任务失败: {}", e))?;
        created += 1;
    }
    Ok(created)
}

/// 旧计划的重复规则 → 说明文本（只用于迁移说明，不影响任务数据）
fn repeat_label(repeat: &str) -> &str {
    match repeat {
        "daily" => "每天",
        "weekly" => "每周",
        other => other,
    }
}

/// 从时间串里取日期部分（`2026-09-24T09:00:00` → `2026-09-24`）；格式不对返回 None。
fn date_part(value: &str) -> Option<String> {
    let date = value.trim().get(0..10)?;
    let bytes = date.as_bytes();
    let digits = |range: std::ops::Range<usize>| {
        bytes[range].iter().all(|b| b.is_ascii_digit())
    };
    let ok = bytes[4] == b'-' && bytes[7] == b'-' && digits(0..4) && digits(5..7) && digits(8..10);
    ok.then(|| date.to_string())
}

/// 表是否存在。
fn has_table(conn: &Connection, table: &str) -> Result<bool, String> {
    let mut stmt = conn
        .prepare("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1")
        .map_err(|e| format!("读取思维导图表结构失败: {}", e))?;
    let mut rows = stmt
        .query([table])
        .map_err(|e| format!("读取思维导图表结构失败: {}", e))?;
    rows.next()
        .map(|row| row.is_some())
        .map_err(|e| format!("读取思维导图表结构失败: {}", e))
}

/// 列是否存在（老库没有新列时靠它兜底）。
fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool, String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({})", table))
        .map_err(|e| format!("读取思维导图表结构失败: {}", e))?;
    let mut rows = stmt
        .query([])
        .map_err(|e| format!("读取思维导图表结构失败: {}", e))?;
    while let Some(row) = rows
        .next()
        .map_err(|e| format!("读取思维导图表结构失败: {}", e))?
    {
        let name: String = row.get(1).map_err(|e| format!("读取思维导图表结构失败: {}", e))?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_part_accepts_iso_datetimes_only() {
        assert_eq!(date_part("2026-09-24T09:00:00").as_deref(), Some("2026-09-24"));
        assert_eq!(date_part("  2026-09-24  ").as_deref(), Some("2026-09-24"));
        // 格式不对一律拒绝：宁可少迁一条，也不要塞一条日期非法的任务
        assert_eq!(date_part("2026/09/24"), None);
        assert_eq!(date_part(""), None);
        assert_eq!(date_part("2026-9-4"), None);
        assert_eq!(date_part("计划 2026-09-24"), None);
    }

    #[test]
    fn legacy_plan_falls_back_to_document_name_and_keeps_repeat_note() {
        let plan = LegacyPlan {
            node_id: "n1".into(),
            name: "   ".into(),
            date: "2026-09-24".into(),
            repeat: "weekly".into(),
            doc_name: "重构计划".into(),
            project_dir: "E:/pro/my/any-version".into(),
        };
        // 节点无名 → 用文档名兜底，避免迁移出一堆「未命名」
        assert_eq!(plan.title(), "重构计划 的计划");
        assert!(plan.description().contains("重构计划"));
        let detail = plan.detail();
        assert!(detail.contains("n1"));
        assert!(detail.contains("E:/pro/my/any-version"));
        // 重复规则在任务里没有对应字段，必须写进说明，否则语义被静默丢掉
        assert!(detail.contains("每周"));
    }

    #[test]
    fn legacy_plan_without_metadata_still_produces_sane_text() {
        let plan = LegacyPlan {
            node_id: "n2".into(),
            name: "写周报".into(),
            date: "2026-09-25".into(),
            repeat: "none".into(),
            doc_name: String::new(),
            project_dir: String::new(),
        };
        assert_eq!(plan.title(), "写周报");
        assert_eq!(plan.description(), "迁移自思维导图");
        assert!(!plan.detail().contains("原重复规则"));
    }
}
