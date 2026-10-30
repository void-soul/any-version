use super::*;

fn make_temp() -> std::path::PathBuf {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "kira-workbuddy-transfer-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        id
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn rejects_unsafe_uids() {
    for uid in ["", "../a", "a/b", "a\\b", "a..b", " a"] {
        assert!(
            codebuddy::validate_uid(uid).is_err(),
            "uid should be rejected: {uid}"
        );
    }
    assert!(codebuddy::validate_uid("384c6dd0-c1bc-4ae2-a0d0-f70350c62f7b").is_ok());
}

#[test]
fn remaps_all_active_sessions_and_preserves_deleted_sessions() {
    let dest = make_temp();
    let db_path = dest.join("workbuddy.db");
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                    id TEXT PRIMARY KEY,
                    cwd TEXT NOT NULL,
                    user_id TEXT NOT NULL,
                    status TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,
                    deleted_at INTEGER
                );
                INSERT INTO sessions VALUES ('a', '/', 'source', 'Done', 1, 2, NULL);
                INSERT INTO sessions VALUES ('b', '/', 'other', 'Done', 1, 2, NULL);
                INSERT INTO sessions VALUES ('c', '/', 'source', 'Done', 1, 2, 3);
                INSERT INTO sessions VALUES ('d', '/', 'target', 'Done', 1, 2, NULL);",
        )
        .unwrap();
        // 制造一个 wal 文件，验证备份包含 wal
        std::fs::write(db_path.with_extension("db-wal"), b"wal-bytes").unwrap();
    }

    let backup_root = dest.join("backup");
    let mut tracker = SyncTracker::new(std::collections::BTreeMap::new());
    let changed =
        remap_workbuddy_database_user_id(&db_path, "target", &backup_root, &dest, &mut tracker)
            .unwrap();
    assert_eq!(changed, 2);
    let (summary, next_baseline) = tracker.finish();
    assert_eq!(summary.total, 2);
    assert_eq!(
        summary.copied, 0,
        "会话正文目录不存在，应记为 partial 而不是 copied"
    );
    assert_eq!(summary.partial, 2);
    assert!(summary
        .details
        .iter()
        .all(|detail| detail.reason == "contentMissing"));
    assert_eq!(next_baseline.len(), 2);

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let active_non_target: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE deleted_at IS NULL AND user_id != 'target'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let deleted_user: String = conn
        .query_row("SELECT user_id FROM sessions WHERE id = 'c'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(active_non_target, 0);
    assert_eq!(deleted_user, "source");
    assert!(backup_root.join("workbuddy.db").is_file());
    assert!(backup_root.join("workbuddy.db-wal").is_file());
}

#[test]
fn missing_database_is_a_noop() {
    let dest = make_temp();
    let mut tracker = SyncTracker::new(std::collections::BTreeMap::new());
    assert_eq!(
        remap_workbuddy_database_user_id(
            &dest.join("missing.db"),
            "target",
            &dest.join("backup"),
            &dest,
            &mut tracker
        )
        .unwrap(),
        0
    );
    let (summary, _) = tracker.finish();
    assert_eq!(summary.total, 0);
}

/// 参考 WorkDaddy §17：基线未变时第二次切换必须**一个会话都不处理**。
#[test]
fn second_transfer_skips_unchanged_sessions() {
    let dest = make_temp();
    let db_path = dest.join("workbuddy.db");
    let project = dest.join("projects").join("hash");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("s1.jsonl"), b"{}\n").unwrap();
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
                "CREATE TABLE sessions (
                    id TEXT PRIMARY KEY,
                    cwd TEXT NOT NULL,
                    title TEXT,
                    custom_title TEXT,
                    user_id TEXT NOT NULL,
                    status TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,
                    deleted_at INTEGER
                );
                INSERT INTO sessions VALUES ('s1', '/', '标题一', NULL, 'source', 'Done', 1, 2, NULL);",
            )
            .unwrap();
    }

    let mut first = SyncTracker::new(std::collections::BTreeMap::new());
    let changed = remap_workbuddy_database_user_id(
        &db_path,
        "target",
        &dest.join("backup"),
        &dest,
        &mut first,
    )
    .unwrap();
    assert_eq!(changed, 1, "首次同步应重映射该会话");
    let (summary, baseline) = first.finish();
    assert_eq!(summary.copied, 1);
    assert_eq!(summary.details[0].label, "标题一");
    // 把会话回写成来源账号，模拟"下次切换时它又出现在来源侧"，但内容与基线一致
    rusqlite::Connection::open(&db_path)
        .unwrap()
        .execute("UPDATE sessions SET user_id = 'source' WHERE id = 's1'", [])
        .unwrap();

    let mut second = SyncTracker::new(baseline);
    let changed = remap_workbuddy_database_user_id(
        &db_path,
        "target",
        &dest.join("backup"),
        &dest,
        &mut second,
    )
    .unwrap();
    assert_eq!(changed, 0, "基线未变时不应再处理该会话");
    let (summary, _) = second.finish();
    assert_eq!(summary.total, 1);
    assert_eq!(summary.skipped, 1);
    assert!(summary.unchanged);
}

#[test]
fn resolve_runtime_dirs_maps_app_leaf_to_parent_config() {
    let dest = make_temp();
    let app_dir = dest.join("app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let (config, electron) = resolve_runtime_dirs(&app_dir).unwrap();
    assert_eq!(config, dest);
    assert_eq!(electron, app_dir);

    let (config, electron) = resolve_runtime_dirs(&dest).unwrap();
    assert_eq!(config, dest);
    assert_eq!(electron, dest.join("app"));
}

/// 两个平台共用同一批 uid：CN 的会话目录已换成共享 junction 后，WorkBuddy 的
/// 「旧版扩展目录」探测仍会命中它（`account_has_history` 跟随链接看 `is_dir()`）。
/// 短路条件必须同时覆盖来源与目标账号，否则会拿同一份物理数据自我合并。
#[test]
fn detects_shared_extension_dirs_for_both_source_and_target() {
    let root = make_temp();
    let shared_history = root.join("shared").join("history");
    std::fs::create_dir_all(shared_history.join("ws1").join("conv-1")).unwrap();

    let extension = root.join("extension");
    let source_uid = "uid-workbuddy-source";
    let target_uid = "uid-workbuddy-target";
    let mut linked = 0;
    for uid in [source_uid, target_uid] {
        let account_root = extension.join(uid).join("CodeBuddyIDE").join(uid);
        std::fs::create_dir_all(&account_root).unwrap();
        if crate::commands::cache::create_junction(&account_root.join("history"), &shared_history)
            .is_ok()
        {
            linked += 1;
        }
    }
    if linked < 2 {
        // 非 Windows / 无 mklink：跳过
        let _ = std::fs::remove_dir_all(&root);
        return;
    }

    assert!(codebuddy::target_sessions_shared(&extension, source_uid));
    assert!(codebuddy::target_sessions_shared(&extension, target_uid));
    assert!(!codebuddy::target_sessions_shared(
        &extension,
        "uid-not-managed"
    ));

    // 只删链接不应带走共享数据
    for uid in [source_uid, target_uid] {
        let history = extension
            .join(uid)
            .join("CodeBuddyIDE")
            .join(uid)
            .join("history");
        std::fs::remove_dir(&history).unwrap();
    }
    assert!(shared_history.join("ws1").join("conv-1").is_dir());
    let _ = std::fs::remove_dir_all(&root);
}
