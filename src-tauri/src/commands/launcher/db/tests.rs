use super::*;
use rusqlite::Connection;

/// 与生产库同构的内存库（两张表），测试直接对它跑真实 SQL。
fn mem() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        r#"
            CREATE TABLE launcher_classification (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                parent_id INTEGER,
                name TEXT NOT NULL,
                classification_type INTEGER NOT NULL DEFAULT 0,
                data TEXT NOT NULL DEFAULT '{}',
                shortcut_key TEXT,
                global_shortcut_key INTEGER NOT NULL DEFAULT 0,
                sort_order INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE launcher_item (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                classification_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                item_type INTEGER NOT NULL DEFAULT 0,
                data TEXT NOT NULL DEFAULT '{}',
                shortcut_key TEXT,
                global_shortcut_key INTEGER NOT NULL DEFAULT 0,
                sort_order INTEGER NOT NULL DEFAULT 0
            );
            "#,
    )
    .unwrap();
    conn
}

fn add_cat(conn: &Connection, id: i64, parent: Option<i64>, name: &str) {
    conn.execute(
            "INSERT INTO launcher_classification (id, parent_id, name, sort_order) VALUES (?1, ?2, ?3, 0)",
            params![id, parent, name],
        )
        .unwrap();
}

fn add_item(conn: &Connection, cls: i64, name: &str) {
    conn.execute(
        "INSERT INTO launcher_item (classification_id, name) VALUES (?1, ?2)",
        params![cls, name],
    )
    .unwrap();
}

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

/// 孤儿检测：① 项目挂在已不存在的分类上；② 分类的 parent_id 指向已不存在的分类。
/// 这两种都是「界面上永远看不到、也删不掉」的死数据。
fn orphans(conn: &Connection) -> (i64, i64) {
    let items: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM launcher_item i
                 WHERE NOT EXISTS (SELECT 1 FROM launcher_classification c WHERE c.id = i.classification_id)",
                [],
                |r| r.get(0),
            )
            .unwrap();
    let cats: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM launcher_classification c
                 WHERE c.parent_id IS NOT NULL
                   AND NOT EXISTS (SELECT 1 FROM launcher_classification p WHERE p.id = c.parent_id)",
                [],
                |r| r.get(0),
            )
            .unwrap();
    (items, cats)
}

/// 回归：**孙级**分类不能留成孤儿 —— 旧实现只查一层直接子分类，
/// 孙级分类与其项目会永久失联（看不见也删不掉）。
#[test]
fn cascade_deletes_the_whole_subtree_without_orphans() {
    let conn = mem();
    add_cat(&conn, 1, None, "一级");
    add_cat(&conn, 2, Some(1), "二级");
    add_cat(&conn, 3, Some(2), "三级");
    add_item(&conn, 1, "i1");
    add_item(&conn, 2, "i2");
    add_item(&conn, 3, "i3");

    let r = delete_classification_on(&conn, 1, DeleteMode::Cascade).unwrap();
    assert_eq!(r.deleted_categories, 3, "整棵子树都要删: {r:?}");
    assert_eq!(r.deleted_items, 3);
    assert_eq!(count(&conn, "launcher_classification"), 0);
    assert_eq!(count(&conn, "launcher_item"), 0);
    assert_eq!(orphans(&conn), (0, 0), "不该留下任何孤儿");
}

/// 迁移模式：子分类与项目都搬到新上级，只删空壳分类，同样不留孤儿。
#[test]
fn reassign_moves_children_and_items_then_deletes_the_shell() {
    let conn = mem();
    add_cat(&conn, 1, None, "一级");
    add_cat(&conn, 2, None, "新家");
    add_cat(&conn, 3, Some(1), "子分类");
    add_item(&conn, 1, "直属项目");
    add_item(&conn, 3, "孙级项目");

    let r = delete_classification_on(&conn, 1, DeleteMode::Reassign { new_parent_id: 2 }).unwrap();
    assert_eq!(r.deleted_categories, 1, "只删分类本身: {r:?}");
    assert_eq!(r.moved_categories, 1);
    assert_eq!(r.moved_items, 1);
    assert_eq!(r.deleted_items, 0, "保留模式下不能删项目");

    // 内容都还在，只是换了归属
    assert_eq!(count(&conn, "launcher_classification"), 2);
    assert_eq!(count(&conn, "launcher_item"), 2);
    assert_eq!(orphans(&conn), (0, 0));
    let parent: Option<i64> = conn
        .query_row(
            "SELECT parent_id FROM launcher_classification WHERE id = 3",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(parent, Some(2));
    let cls: i64 = conn
        .query_row(
            "SELECT classification_id FROM launcher_item WHERE name = '直属项目'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cls, 2);
}

/// 非法迁移目标必须被拒：迁到自己 / 迁到自己的子树 / 目标不存在。
#[test]
fn reassign_rejects_illegal_targets() {
    let conn = mem();
    add_cat(&conn, 1, None, "一级");
    add_cat(&conn, 2, Some(1), "子级");

    assert!(delete_classification_on(&conn, 1, DeleteMode::Reassign { new_parent_id: 1 }).is_err());
    assert!(
        delete_classification_on(&conn, 1, DeleteMode::Reassign { new_parent_id: 2 }).is_err(),
        "不能迁到自己的子孙下"
    );
    assert!(
        delete_classification_on(&conn, 1, DeleteMode::Reassign { new_parent_id: 99 }).is_err(),
        "目标不存在要报错"
    );
    // 被拒后数据必须原封不动
    assert_eq!(count(&conn, "launcher_classification"), 2);
}

/// 分类层级成环（脏数据）时不能死循环。
#[test]
fn descendant_scan_survives_cycles() {
    let conn = mem();
    add_cat(&conn, 1, Some(2), "A");
    add_cat(&conn, 2, Some(1), "B");
    let ids = descendant_ids_on(&conn, 1).unwrap();
    assert!(ids.contains(&1) && ids.contains(&2));
}
