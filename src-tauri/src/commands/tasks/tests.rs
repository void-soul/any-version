use super::{commands::*, db};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::sync::Mutex;

/// tasks 的测试共用一个全局 SQLite 连接，且 `failed_move_*` 会在该连接上
/// 创建 TEMP TRIGGER 注入故障。不加串行锁时并发跑会让另一个测试撞上
/// 「injected move failure」（AUDIT-041 测试隔离）。
static DB_TEST_LOCK: Mutex<()> = Mutex::new(());

fn db_guard() -> std::sync::MutexGuard<'static, ()> {
    DB_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn input<T: DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).unwrap()
}

#[test]
fn burst_ids_are_unique_for_tasks_logs_and_references() {
    for prefix in ["task", "log", "ref", "mv", "sticker"] {
        let ids: std::collections::HashSet<_> = (0..4096).map(|_| db::new_id(prefix)).collect();
        assert_eq!(ids.len(), 4096, "duplicate {prefix} IDs");
    }
}

#[test]
fn commands_persist_reopen_and_preserve_parent_log_relationships() {
    let _guard = db_guard();
    let parent = tasks_create(input(json!({"title":"persistence parent"}))).unwrap();
    let child = tasks_create(input(json!({
        "title":"persistence child", "parentId":parent.id,
        "scheduledDate":"2026-11-03", "detail":"中文 **Markdown**"
    }))).unwrap();
    assert!(tasks_update(parent.id.clone(), input(json!({"parentId":child.id}))).is_err());
    let finished = tasks_set_progress(child.id.clone(), input(json!({
        "progress":100,"logContent":"done","minutesSpent":12,
        "references":[{"kind":"file","target":"fixture-a.md"},{"kind":"file","target":"fixture-b.md"}]
    }))).unwrap();
    assert!(finished.completed_at.is_some());
    db::init_db().unwrap(); // close the old connection and open the on-disk DB again
    let loaded = tasks_search("persistence child".into(), None).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].detail, "中文 **Markdown**");
    assert_eq!(loaded[0].progress, 100);
    assert_eq!(tasks_list_logs(child.id.clone()).unwrap()[0].references.len(), 2);
    assert_eq!(tasks_list_moves(child.id.clone()).unwrap().len(), 1);
    let reopened = tasks_update(child.id.clone(), input(json!({"progress":25,"scheduledDate":null}))).unwrap();
    assert!(reopened.completed_at.is_none());
    assert!(reopened.scheduled_date.is_none());
    tasks_set_archived(child.id.clone(), true).unwrap();
    assert!(tasks_search("persistence child".into(), None).unwrap().is_empty());
    assert_eq!(tasks_search("persistence child".into(), Some(true)).unwrap().len(), 1);
    tasks_delete(parent.id).unwrap();
    assert!(tasks_search("persistence child".into(), Some(true)).unwrap()[0].parent_id.is_none());
    tasks_delete(child.id.clone()).unwrap();
    assert!(tasks_list_logs(child.id.clone()).unwrap().is_empty());
    assert!(tasks_list_moves(child.id).unwrap().is_empty());
    db::with_conn(|conn| {
        let orphan_refs: i64 = conn.query_row(
            "SELECT COUNT(*) FROM task_log_references r LEFT JOIN task_logs l ON l.id=r.log_id WHERE l.id IS NULL",
            [], |row| row.get(0),
        ).map_err(|e| e.to_string())?;
        assert_eq!(orphan_refs, 0);
        Ok(())
    }).unwrap();
}

#[test]
fn failed_move_and_reference_writes_roll_back_the_whole_command() {
    let _guard = db_guard();
    let task = tasks_create(input(json!({"title":"rollback fixture","scheduledDate":"2026-11-01"}))).unwrap();
    db::with_conn(|conn| conn.execute_batch(
        "CREATE TEMP TRIGGER fail_move BEFORE INSERT ON task_moves BEGIN SELECT RAISE(FAIL, 'injected move failure'); END;"
    ).map_err(|e| e.to_string())).unwrap();
    assert!(tasks_set_progress(task.id.clone(), input(json!({"progress":100,"logContent":"must roll back"}))).is_err());
    let persisted = tasks_search("rollback fixture".into(), None).unwrap();
    assert_eq!(persisted[0].progress, 0);
    assert!(persisted[0].completed_at.is_none());
    assert!(tasks_list_logs(task.id.clone()).unwrap().is_empty());
    assert!(tasks_update(task.id.clone(), input(json!({"progress":50}))).is_err());
    assert!(tasks_move(task.id.clone(), input(json!({"toDate":null}))).is_err());
    assert!(tasks_carry_over(Some("2026-11-03".into())).is_err());
    let persisted = tasks_search("rollback fixture".into(), None).unwrap();
    assert_eq!(persisted[0].progress, 0);
    assert_eq!(persisted[0].scheduled_date.as_deref(), Some("2026-11-01"));
    db::with_conn(|conn| conn.execute_batch(
        "DROP TRIGGER fail_move; CREATE TEMP TRIGGER fail_ref BEFORE INSERT ON task_log_references BEGIN SELECT RAISE(FAIL, 'injected reference failure'); END;"
    ).map_err(|e| e.to_string())).unwrap();
    assert!(tasks_add_log(input(json!({"taskId":task.id,"logDate":"2026-11-03","content":"must roll back", "references":[{"kind":"file","target":"fixture.md"}]}))).is_err());
    assert!(tasks_list_logs(task.id.clone()).unwrap().is_empty());
    db::with_conn(|conn| conn.execute_batch("DROP TRIGGER fail_ref;").map_err(|e| e.to_string())).unwrap();
    let child = tasks_create(input(json!({"title":"rollback child","parentId":task.id}))).unwrap();
    tasks_add_log(input(json!({"taskId":task.id,"logDate":"2026-11-03","content":"keep me"}))).unwrap();
    db::with_conn(|conn| conn.execute_batch(
        "CREATE TEMP TRIGGER fail_delete BEFORE DELETE ON tasks BEGIN SELECT RAISE(FAIL, 'injected delete failure'); END;"
    ).map_err(|e| e.to_string())).unwrap();
    assert!(tasks_delete(task.id.clone()).is_err());
    assert_eq!(tasks_list_logs(task.id.clone()).unwrap().len(), 1);
    assert_eq!(tasks_search("rollback child".into(), None).unwrap()[0].parent_id.as_deref(), Some(task.id.as_str()));
    db::with_conn(|conn| conn.execute_batch("DROP TRIGGER fail_delete;").map_err(|e| e.to_string())).unwrap();
    tasks_delete(task.id).unwrap();
    tasks_delete(child.id).unwrap();
}

#[test]
fn legacy_migration_rolls_back_partial_import_and_retries_without_duplicates() {
    let _guard = db_guard();
    let source = crate::commands::config::get_data_dir().join("mindmap.db");
    let conn = rusqlite::Connection::open(&source).unwrap();
    conn.execute_batch(
        "CREATE TABLE mindmap_documents(id TEXT, name TEXT, project_dir TEXT);
         CREATE TABLE mindmap_nodes(id TEXT, name TEXT, plan_at TEXT, document_id TEXT);
         INSERT INTO mindmap_documents VALUES('fixture', 'legacy fixture', '');
         INSERT INTO mindmap_nodes VALUES('one','legacy fixture first','2026-11-03T10:00:00','fixture');
         INSERT INTO mindmap_nodes VALUES('two','legacy fixture second','2026-11-03T10:00:00','fixture');"
    ).unwrap();
    drop(conn);
    db::with_conn(|conn| {
        conn.execute_batch("CREATE TEMP TRIGGER fail_import BEFORE INSERT ON tasks WHEN NEW.title='legacy fixture second' BEGIN SELECT RAISE(FAIL, 'injected import failure'); END;").map_err(|e| e.to_string())?;
        assert_eq!(super::migrate::migrate_mindmap_plans(conn)?, 0);
        assert!(db::get_meta(conn, "mindmap_plan_migration_v1")?.is_none());
        conn.execute_batch("DROP TRIGGER fail_import;").map_err(|e| e.to_string())
    }).unwrap();
    assert!(tasks_search("legacy fixture".into(), None).unwrap().is_empty());
    db::with_conn(|conn| {
        assert_eq!(super::migrate::migrate_mindmap_plans(conn)?, 2);
        assert_eq!(super::migrate::migrate_mindmap_plans(conn)?, 0);
        Ok(())
    }).unwrap();
    assert_eq!(tasks_search("legacy fixture".into(), None).unwrap().len(), 2);
    std::fs::remove_file(source).unwrap();
}
