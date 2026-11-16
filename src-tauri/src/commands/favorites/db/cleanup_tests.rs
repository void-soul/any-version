use super::*;

/// 新库不创建 `favorite_tag`（AUDIT-023），初始化清理不能因此失败。
#[test]
fn cleanup_orphans_works_without_legacy_tag_table() {
    let conn = Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();
    cleanup_orphans(&conn).unwrap();
}

/// 老库仍有 `favorite_tag` 时，孤儿行要被清掉（表本身由迁移流程负责 DROP）。
#[test]
fn cleanup_orphans_cleans_legacy_tag_table_when_present() {
    let conn = Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();
    conn.execute_batch(
        "CREATE TABLE favorite_tag (favorite_id INTEGER NOT NULL, tag TEXT NOT NULL, PRIMARY KEY (favorite_id, tag));
         INSERT INTO favorite_tag VALUES (999, 'orphan');",
    )
    .unwrap();

    cleanup_orphans(&conn).unwrap();

    let left: i64 = conn
        .query_row("SELECT COUNT(*) FROM favorite_tag", [], |row| row.get(0))
        .unwrap();
    assert_eq!(left, 0, "孤儿标签行应被清理");
}
