use super::*;

fn temp() -> PathBuf {
    let mut random = [0u8; 8];
    getrandom::getrandom(&mut random).unwrap();
    let path = std::env::temp_dir().join(format!("kira-storage-tests-{:016x}", u64::from_ne_bytes(random)));
    std::fs::create_dir(&path).unwrap();
    path
}

#[test]
fn rooted_drive_relative_parent_and_device_paths_are_rejected() {
    for path in [r"\outside\a", r"C:a", r"C:\a", r"\\host\share\a", "../a", "a/../b", "a//b", "NUL.txt", "a/COM1", "a/ads:stream", "a./b", ""] {
        assert!(normalize_relative_path(path).is_err(), "{path}");
    }
    assert_eq!(normalize_relative_path(r"clipboard\images\a.png").unwrap(), "clipboard/images/a.png");
}

#[test]
fn all_entries_are_validated_before_staging_any_restore() {
    let base = temp(); let data = base.join("data");
    std::fs::create_dir(&data).unwrap();
    std::fs::write(data.join("settings.json"), "original").unwrap();
    let state = serde_json::json!({"type":"any-version-state", "files":{"settings.json":STANDARD.encode(r#"{"value":"new"}"#), "backups.json":"not-base64"}});
    assert!(stage_restore(&base, &data, &state, None).is_err());
    assert_eq!(std::fs::read_to_string(data.join("settings.json")).unwrap(), "original");
    assert!(!operation_file(&base).exists());
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn malformed_declared_json_configuration_is_rejected_before_restore() {
    let base = temp(); let data = base.join("data"); std::fs::create_dir(&data).unwrap();
    let state = serde_json::json!({"type":"any-version-state", "files":{"settings.json":STANDARD.encode("invalid-json")}});
    assert!(stage_restore(&base, &data, &state, None).is_err());
    assert!(!operation_file(&base).exists());
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn sqlite_export_includes_committed_wal_content_without_copying_sidecars() {
    let root = temp(); let db = root.join("live.db");
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE sample(value TEXT); INSERT INTO sample VALUES ('latest');").unwrap();
    assert!(PathBuf::from(format!("{}-wal", db.display())).is_file());
    let bytes = snapshot_file_bytes(&db).unwrap();
    let copy = root.join("copy.db"); std::fs::write(&copy, bytes).unwrap();
    let snapshot = rusqlite::Connection::open(copy).unwrap();
    let value: String = snapshot.query_row("SELECT value FROM sample", [], |r| r.get(0)).unwrap();
    assert_eq!(value, "latest");
    drop(snapshot); drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn restore_is_queued_and_preserves_machine_path_configuration() {
    let base = temp(); let data = base.join("data"); std::fs::create_dir(&data).unwrap();
    std::fs::write(data.join("settings.json"), "old").unwrap();
    let state = serde_json::json!({"type":"any-version-state", "files":{"config.json":STANDARD.encode(r#"{"data_dir":"Z:\\other-machine"}"#), "settings.json":STANDARD.encode(r#"{"value":"new"}"#)}});
    assert_eq!(stage_restore(&base, &data, &state, None).unwrap(), 1);
    assert_eq!(std::fs::read_to_string(data.join("settings.json")).unwrap(), "old");
    let operation: PendingOperation = serde_json::from_slice(&std::fs::read(operation_file(&base)).unwrap()).unwrap();
    let PendingOperation::Restore(plan) = operation else { panic!("restore plan expected") };
    assert_eq!(plan.target_root, data);
    apply_restore(&plan).unwrap();
    assert_eq!(std::fs::read_to_string(data.join("settings.json")).unwrap(), r#"{"value":"new"}"#);
    assert!(!base.join("config.json").exists());
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn failed_restore_rolls_back_all_files_and_can_be_retried() {
    let base = temp(); let data = base.join("data"); std::fs::create_dir(&data).unwrap();
    std::fs::write(data.join("settings.json"), "old-settings").unwrap();
    std::fs::write(data.join("backups.json"), "old-backups").unwrap();
    let state = serde_json::json!({"type":"any-version-state", "files":{"settings.json":STANDARD.encode(r#"{"value":"new-settings"}"#),"backups.json":STANDARD.encode(r#"{"value":"new-backups"}"#)}});
    stage_restore(&base, &data, &state, None).unwrap();
    let PendingOperation::Restore(plan) = serde_json::from_slice(&std::fs::read(operation_file(&base)).unwrap()).unwrap() else { panic!("restore expected") };
    assert!(apply_restore_inner(&plan, Some(1)).is_err());
    assert_eq!(std::fs::read_to_string(data.join("settings.json")).unwrap(), "old-settings");
    assert_eq!(std::fs::read_to_string(data.join("backups.json")).unwrap(), "old-backups");
    apply_restore(&plan).unwrap();
    assert_eq!(std::fs::read_to_string(data.join("settings.json")).unwrap(), r#"{"value":"new-settings"}"#);
    assert_eq!(std::fs::read_to_string(data.join("backups.json")).unwrap(), r#"{"value":"new-backups"}"#);
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn restore_removes_old_wal_and_shm_before_replacing_database() {
    let base = temp(); let data = base.join("data"); std::fs::create_dir(&data).unwrap();
    let source = base.join("source.db");
    let conn = rusqlite::Connection::open(&source).unwrap();
    conn.execute_batch("CREATE TABLE sample(value TEXT); INSERT INTO sample VALUES ('snapshot');").unwrap(); drop(conn);
    std::fs::write(data.join("tasks.db"), b"old-database").unwrap();
    std::fs::write(data.join("tasks.db-wal"), b"old-wal").unwrap();
    std::fs::write(data.join("tasks.db-shm"), b"old-shm").unwrap();
    let state = serde_json::json!({"type":"any-version-state", "files":{"tasks.db":STANDARD.encode(std::fs::read(&source).unwrap())}});
    stage_restore(&base, &data, &state, None).unwrap();
    let PendingOperation::Restore(plan) = serde_json::from_slice(&std::fs::read(operation_file(&base)).unwrap()).unwrap() else { panic!("restore expected") };
    apply_restore(&plan).unwrap();
    assert!(!data.join("tasks.db-wal").exists() && !data.join("tasks.db-shm").exists());
    let conn = rusqlite::Connection::open(data.join("tasks.db")).unwrap();
    let value: String = conn.query_row("SELECT value FROM sample", [], |r| r.get(0)).unwrap(); assert_eq!(value, "snapshot"); drop(conn);
    std::fs::remove_dir_all(base).unwrap();
}
