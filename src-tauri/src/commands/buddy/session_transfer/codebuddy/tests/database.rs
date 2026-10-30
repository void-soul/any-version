use super::*;

#[test]
fn rejects_unsafe_uids() {
    for uid in ["", "../a", "a/b", "a\\b", "a..b", " a"] {
        assert!(validate_uid(uid).is_err(), "uid should be rejected: {uid}");
    }
    assert!(validate_uid("384c6dd0-c1bc-4ae2-a0d0-f70350c62f7b").is_ok());
}

/// 批量裁决落盘用的 action 名必须与 parse 成反函数（前端按它做 i18n）。
#[test]
fn conflict_action_roundtrips_through_as_str() {
    for action in [
        ConflictAction::Merge,
        ConflictAction::Overwrite,
        ConflictAction::Keep,
    ] {
        assert_eq!(ConflictAction::parse(action.as_str()).unwrap(), action);
    }
    assert!(ConflictAction::parse("nope").is_err());
}

#[test]
fn remaps_vscdb_user_id() {
    let dest = make_temp();
    let db_path = dest.join("codebuddy-sessions.vscdb");
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
                "CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                INSERT INTO ItemTable VALUES ('session:1', '{\"userId\":\"source\",\"messages\":[]}');
                INSERT INTO ItemTable VALUES ('session:2', '{\"userId\":\"source\",\"messages\":[]}');
                INSERT INTO ItemTable VALUES ('session:3', '{\"userId\":\"target\",\"messages\":[]}');
                INSERT INTO ItemTable VALUES ('session:4', '{\"userId\":\"other\",\"messages\":[]}');",
            )
            .unwrap();
    }

    let backup_root = dest.join("backup");
    let changed = remap_session_vscdb_user_id(&db_path, "source", "target", &backup_root).unwrap();
    assert_eq!(changed, 2);

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let remaining: String = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = 'session:1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&remaining).unwrap();
    assert_eq!(value["userId"].as_str(), Some("target"));
    assert!(backup_root.join("codebuddy-sessions.vscdb").is_file());
}

#[test]
fn missing_database_is_a_noop() {
    let dest = make_temp();
    assert_eq!(
        remap_session_vscdb_user_id(
            &dest.join("missing.vscdb"),
            "source",
            "target",
            &dest.join("backup")
        )
        .unwrap(),
        0
    );
}
