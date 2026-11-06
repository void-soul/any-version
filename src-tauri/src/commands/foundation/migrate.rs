//! 版本化 SQLite 迁移框架（AUDIT-024 / 050 单一真源）。
//!
//! 项目此前每个模块各自写 `CREATE TABLE IF NOT EXISTS` + `PRAGMA table_info`
//! + `ALTER TABLE`，没有版本号、没有事务、没有执行记录；`execute_batch` 中途
//! 失败会留下半套 schema。本框架统一为：
//! - `PRAGMA user_version` 记录已应用的版本；
//! - 每个版本一步幂等闭包，全部包在**同一个事务**里；
//! - 任一步失败整体回滚，并把错误向上传递（绝不 `return Ok(())` 吞掉）；
//! - 只前进不回退，跨版本跳级升级按序执行所有中间版本。

use rusqlite::{Connection, Transaction};

/// 一步迁移。`version` 从 1 开始连续递增，不可跳号。
pub struct Step {
    pub version: u32,
    pub name: &'static str,
    pub run: fn(&Transaction<'_>) -> Result<(), String>,
}

/// 读取当前 schema 版本。
pub fn current_version(conn: &Connection) -> Result<u32, String> {
    conn.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
        .map_err(|e| format!("读取 schema 版本失败: {e}"))
}

/// 按序应用所有尚未执行的迁移步骤。
///
/// `steps` 必须按 version 升序且连续；返回本次实际应用的步数。
pub fn ensure_schema(conn: &mut Connection, steps: &[Step]) -> Result<usize, String> {
    let mut expected = 1u32;
    for step in steps {
        if step.version != expected {
            return Err(format!(
                "迁移步骤版本不连续：期望 {expected}，实际 {} ({})",
                step.version, step.name
            ));
        }
        expected += 1;
    }

    let from = current_version(conn)?;
    let pending: Vec<&Step> = steps.iter().filter(|s| s.version > from).collect();
    if pending.is_empty() {
        return Ok(0);
    }

    let tx = conn
        .transaction()
        .map_err(|e| format!("开启迁移事务失败: {e}"))?;
    let mut applied = 0usize;
    for step in pending {
        (step.run)(&tx).map_err(|e| format!("迁移 v{} ({}) 失败: {e}", step.version, step.name))?;
        // PRAGMA user_version 不支持参数绑定，但值来自内部常量，无注入面。
        tx.execute_batch(&format!("PRAGMA user_version = {};", step.version))
            .map_err(|e| format!("写入 schema 版本 v{} 失败: {e}", step.version))?;
        applied += 1;
    }
    tx.commit().map_err(|e| format!("提交迁移失败: {e}"))?;
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_items(tx: &Transaction<'_>) -> Result<(), String> {
        tx.execute_batch("CREATE TABLE IF NOT EXISTS items (id INTEGER PRIMARY KEY, name TEXT);")
            .map_err(|e| e.to_string())
    }

    fn add_email(tx: &Transaction<'_>) -> Result<(), String> {
        tx.execute_batch("ALTER TABLE items ADD COLUMN email TEXT;")
            .map_err(|e| e.to_string())
    }

    #[test]
    fn applies_steps_in_order_and_is_idempotent() {
        let mut conn = Connection::open_in_memory().unwrap();
        let steps = [
            Step {
                version: 1,
                name: "items",
                run: create_items,
            },
            Step {
                version: 2,
                name: "email",
                run: add_email,
            },
        ];
        assert_eq!(ensure_schema(&mut conn, &steps).unwrap(), 2);
        assert_eq!(current_version(&conn).unwrap(), 2);
        // 再次执行不重复应用
        assert_eq!(ensure_schema(&mut conn, &steps).unwrap(), 0);
        assert_eq!(current_version(&conn).unwrap(), 2);
        let cols: i64 = conn
            .query_row("SELECT COUNT(*) FROM pragma_table_info('items')", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(cols, 3, "id / name / email 三列都应存在");
    }

    #[test]
    fn failed_step_rolls_back_everything() {
        let mut conn = Connection::open_in_memory().unwrap();
        let steps = [
            Step {
                version: 1,
                name: "items",
                run: create_items,
            },
            Step {
                version: 2,
                name: "broken",
                run: |tx| {
                    tx.execute_batch("THIS IS NOT SQL")
                        .map_err(|e| e.to_string())
                },
            },
        ];
        assert!(ensure_schema(&mut conn, &steps).is_err());
        // 事务回滚后 v1 的表也不应存在，且版本仍是 0。
        assert_eq!(current_version(&conn).unwrap(), 0);
        let tables: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='items'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tables, 0);
    }

    #[test]
    fn non_contiguous_versions_are_rejected() {
        let mut conn = Connection::open_in_memory().unwrap();
        let steps = [Step {
            version: 2,
            name: "skip",
            run: create_items,
        }];
        assert!(ensure_schema(&mut conn, &steps).is_err());
    }
}
