use super::db::{increment_item_open_count_on, migrate_item_usage_columns};
use rusqlite::{params, Connection};

fn legacy(conn: &Connection) {
    conn.execute_batch("CREATE TABLE launcher_item (id INTEGER PRIMARY KEY, classification_id INTEGER NOT NULL, name TEXT NOT NULL, item_type INTEGER NOT NULL DEFAULT 0, data TEXT NOT NULL DEFAULT '{}', shortcut_key TEXT, global_shortcut_key INTEGER NOT NULL DEFAULT 0, sort_order INTEGER NOT NULL DEFAULT 0);").unwrap();
}

fn current(conn: &Connection) {
    conn.execute_batch("CREATE TABLE launcher_item (id INTEGER PRIMARY KEY, classification_id INTEGER NOT NULL, name TEXT NOT NULL, item_type INTEGER NOT NULL DEFAULT 0, data TEXT NOT NULL DEFAULT '{}', shortcut_key TEXT, global_shortcut_key INTEGER NOT NULL DEFAULT 0, sort_order INTEGER NOT NULL DEFAULT 0, open_number INTEGER NOT NULL DEFAULT 0, last_open INTEGER NOT NULL DEFAULT 0);").unwrap();
}

#[test]
fn launch_count_persists_and_updates_last_open() {
    let conn = Connection::open_in_memory().unwrap();
    current(&conn);
    conn.execute("INSERT INTO launcher_item (id, classification_id, name) VALUES (1, 1, 'Editor')", []).unwrap();
    increment_item_open_count_on(&conn, 1).unwrap();
    increment_item_open_count_on(&conn, 1).unwrap();
    let values: (i64, i64) = conn.query_row("SELECT open_number, last_open FROM launcher_item WHERE id=1", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!(values.0, 2);
    assert!(values.1 > 0);
}

#[test]
fn legacy_schema_migration_adds_zeroed_usage_columns() {
    let conn = Connection::open_in_memory().unwrap();
    legacy(&conn);
    conn.execute("INSERT INTO launcher_item (id, classification_id, name) VALUES (7, 1, 'Legacy')", []).unwrap();
    migrate_item_usage_columns(&conn).unwrap();
    let columns: Vec<String> = conn.prepare("PRAGMA table_info(launcher_item)").unwrap().query_map([], |r| r.get(1)).unwrap().filter_map(Result::ok).collect();
    assert!(columns.iter().any(|c| c == "open_number"));
    assert!(columns.iter().any(|c| c == "last_open"));
    let values: (i64, i64) = conn.query_row("SELECT open_number, last_open FROM launcher_item WHERE id=7", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!(values, (0, 0));
    increment_item_open_count_on(&conn, 7).unwrap();
    assert_eq!(conn.query_row("SELECT open_number FROM launcher_item WHERE id=7", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
}

#[test]
fn usage_statistics_sort_by_count_then_name() {
    let conn = Connection::open_in_memory().unwrap();
    current(&conn);
    for (id, name, count) in [(1, "Beta", 2_i64), (2, "Alpha", 5), (3, "Gamma", 2), (4, "Never", 0)] {
        conn.execute("INSERT INTO launcher_item (id, classification_id, name, open_number) VALUES (?1, 1, ?2, ?3)", params![id, name, count]).unwrap();
    }
    let names: Vec<String> = conn.prepare("SELECT name FROM launcher_item WHERE open_number > 0 ORDER BY open_number DESC, name ASC").unwrap().query_map([], |r| r.get(0)).unwrap().filter_map(Result::ok).collect();
    assert_eq!(names, ["Alpha", "Beta", "Gamma"]);
}
