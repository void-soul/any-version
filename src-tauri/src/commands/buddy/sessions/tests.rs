use super::*;

/// 造一个最小 workbuddy.db（列比真机少，验证「按列名动态取」不是写死 SQL）。
fn make_workbuddy_db(dir: &Path, id: &str, title: &str) {
    let conn = Connection::open(dir.join("workbuddy.db")).unwrap();
    conn.execute_batch(
        "CREATE TABLE sessions (
                id TEXT PRIMARY KEY, cwd TEXT NOT NULL, user_id TEXT NOT NULL,
                title TEXT, custom_title TEXT, status TEXT NOT NULL DEFAULT 'Pending',
                created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
                deleted_at INTEGER, is_playground INTEGER NOT NULL DEFAULT 0,
                last_activity_at INTEGER
            );",
    )
    .unwrap();
    conn.execute(
            "INSERT INTO sessions (id,cwd,user_id,title,custom_title,status,created_at,updated_at,deleted_at,is_playground,last_activity_at)
             VALUES (?1,'/work/app','u1',?2,NULL,'Completed',1000,2000,NULL,0,2500)",
            rusqlite::params![id, title],
        )
        .unwrap();
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("buddy-fork-{}-{}", tag, now_ms()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 端到端：库记录复制 + 正文复制且正文里的旧 id 全部换成新 id，原会话纹丝不动。
#[test]
fn fork_workbuddy_copies_row_and_transcript() {
    let root = temp_dir("wb");
    let src_id = "11111111-2222-4333-8444-555555555555";
    let new_id = new_uuid_v4().unwrap();
    make_workbuddy_db(&root, src_id, "重构 API");

    let project = root.join("projects").join("c-work-app");
    std::fs::create_dir_all(&project).unwrap();
    let transcript = format!(
            "{{\"id\":\"m1\",\"sessionId\":\"{}\",\"role\":\"user\"}}\n{{\"id\":\"m2\",\"sessionId\":\"{}\",\"role\":\"assistant\"}}\n",
            src_id, src_id
        );
    std::fs::write(project.join(format!("{}.jsonl", src_id)), &transcript).unwrap();

    let report = fork_workbuddy_session_at(&root, src_id, &new_id, None).unwrap();
    assert_eq!(report.conversation_id, new_id);
    assert_eq!(report.title, "重构 API（分叉）");
    assert_eq!(report.files_copied, 1);
    assert!(!report.content_missing);
    assert!(report.errors.is_empty());

    // 新行：id/标题/时间都换了，cwd/user_id/status 沿用
    let conn = Connection::open(root.join("workbuddy.db")).unwrap();
    let (cwd, user_id, title, status, created, deleted): (String, String, String, String, i64, Option<i64>) = conn
            .query_row(
                "SELECT cwd, user_id, title, status, created_at, deleted_at FROM sessions WHERE id = ?1",
                rusqlite::params![new_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .unwrap();
    assert_eq!(cwd, "/work/app");
    assert_eq!(user_id, "u1");
    assert_eq!(title, "重构 API（分叉）");
    assert_eq!(status, "Completed");
    assert!(created > 2000);
    assert!(deleted.is_none());

    // 正文：新文件里的 sessionId 全换成新 id，旧文件一个字节都没动
    let copied = std::fs::read_to_string(project.join(format!("{}.jsonl", new_id))).unwrap();
    assert!(!copied.contains(src_id));
    assert_eq!(
        copied
            .matches(&format!("\"sessionId\":\"{}\"", new_id))
            .count(),
        2
    );
    assert_eq!(
        std::fs::read_to_string(project.join(format!("{}.jsonl", src_id))).unwrap(),
        transcript
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// 库里有记录但正文已丢：仍要建出新会话，并如实回报 contentMissing。
#[test]
fn fork_workbuddy_reports_missing_content() {
    let root = temp_dir("wb-missing");
    let src_id = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
    make_workbuddy_db(&root, src_id, "孤儿会话");
    let report = fork_workbuddy_session_at(&root, src_id, &new_uuid_v4().unwrap(), None).unwrap();
    assert_eq!(report.files_copied, 0);
    assert!(report.content_missing);
    let _ = std::fs::remove_dir_all(&root);
}

/// 源会话不存在 → 报错，且不留下半条记录。
#[test]
fn fork_workbuddy_rejects_unknown_session() {
    let root = temp_dir("wb-unknown");
    make_workbuddy_db(&root, "known-id", "x");
    let err =
        fork_workbuddy_session_at(&root, "missing-id", &new_uuid_v4().unwrap(), None).unwrap_err();
    assert!(err.contains("会话不存在"), "实际错误: {}", err);
    let conn = Connection::open(root.join("workbuddy.db")).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn fork_title_falls_back_to_source_title() {
    assert_eq!(fork_title("重构 API", None), "重构 API（分叉）");
    assert_eq!(fork_title("重构 API", Some("  ")), "重构 API（分叉）");
    assert_eq!(fork_title("重构 API", Some("新方案 A")), "新方案 A");
    assert_eq!(fork_title("", None), "（分叉）");
}

#[test]
fn new_uuid_v4_is_unique_and_versioned() {
    let a = new_uuid_v4().unwrap();
    let b = new_uuid_v4().unwrap();
    assert_ne!(a, b);
    assert_eq!(a.len(), 36);
    assert_eq!(&a[14..15], "4", "版本号位应为 v4: {}", a);
}

#[test]
fn parse_session_json() {
    let json = r#"{"conversationId":"conv1","title":"重构 API","cwd":"/work/app","userId":"u1","status":"Completed","createdAt":1000,"updatedAt":2000,"isPlayground":false}"#;
    let raw = parse_raw_session(json).unwrap();
    assert_eq!(raw.conversation_id, "conv1");
    assert_eq!(raw.title, "重构 API");
    assert_eq!(raw.status, "Completed");
    assert!(!raw.is_deleted);
}

#[test]
fn parse_session_with_deleted() {
    let json = r#"{"conversationId":"conv2","title":"t","cwd":"/x","userId":"u1","status":"InProgress","deletedAt":999}"#;
    let raw = parse_raw_session(json).unwrap();
    assert!(raw.is_deleted);
}

#[test]
fn aggregate_filters_and_dedupes() {
    let sessions = vec![
        RawSession {
            conversation_id: "a".into(),
            title: "Alpha".into(),
            cwd: "/p1".into(),
            user_id: "u1".into(),
            status: "Completed".into(),
            created_at: Some(1),
            updated_at: Some(10),
            is_deleted: false,
            is_playground: false,
            last_activity_at: None,
        },
        RawSession {
            conversation_id: "b".into(),
            title: "Beta".into(),
            cwd: "/p2".into(),
            user_id: "u1".into(),
            status: "InProgress".into(),
            created_at: Some(2),
            updated_at: Some(20),
            is_deleted: false,
            is_playground: false,
            last_activity_at: None,
        },
        RawSession {
            conversation_id: "a".into(),
            title: "Alpha".into(),
            cwd: "/p1".into(),
            user_id: "u1".into(),
            status: "Completed".into(),
            created_at: Some(1),
            updated_at: Some(30), // 更新
            is_deleted: false,
            is_playground: false,
            last_activity_at: None,
        },
        RawSession {
            conversation_id: "gone".into(),
            title: "Deleted".into(),
            cwd: "/p3".into(),
            user_id: "u1".into(),
            status: "Completed".into(),
            created_at: Some(3),
            updated_at: Some(40),
            is_deleted: true,
            is_playground: false,
            last_activity_at: None,
        },
    ];

    // 关键字过滤 + 去重 + 排除已删除
    let filter = BuddySessionFilter {
        keyword: Some("alpha".to_string()),
        status: None,
    };
    let records = aggregate_sessions(sessions.clone(), &filter, "default", "CB");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].conversation_id, "a");
    assert_eq!(records[0].updated_at, Some(30));
    assert_eq!(records[0].locations.len(), 1);

    // 状态过滤
    let filter = BuddySessionFilter {
        keyword: None,
        status: Some("InProgress".to_string()),
    };
    let records = aggregate_sessions(sessions, &filter, "default", "CB");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].conversation_id, "b");
}

#[test]
fn delete_sessions_at_removes_db_row_and_files() {
    let dest = std::env::temp_dir().join(format!(
        "kira-session-delete-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dest).unwrap();

    let uid = "uid-a";
    // vscdb：两条会话记录
    let db_path = dest.join("codebuddy-sessions.vscdb");
    {
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                INSERT INTO ItemTable VALUES ('session:conv1', '{\"userId\":\"uid-a\"}');
                INSERT INTO ItemTable VALUES ('session:conv2', '{\"userId\":\"uid-a\"}');",
        )
        .unwrap();
    }
    // 会话文件：conv1（含 messages 与辅助目录）、conv2
    let extension = dest.join("CodeBuddyExtension").join("Data");
    let account_root = extension.join(uid).join("VSCode").join(uid);
    let history = account_root.join("history").join("ws1");
    std::fs::create_dir_all(history.join("conv1").join("messages")).unwrap();
    std::fs::write(history.join("conv1").join("messages").join("0.json"), "[]").unwrap();
    std::fs::create_dir_all(history.join("conv2")).unwrap();
    std::fs::write(
        history.join("index.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "conversations": [
                {"id": "conv1", "lastMessageAt": 1},
                {"id": "conv2", "lastMessageAt": 2}
            ],
            "current": "conv1"
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::create_dir_all(account_root.join("check-point").join("ws1").join("conv1")).unwrap();

    let mut report = BuddySessionDeleteReport::default();
    delete_codebuddy_sessions_at(&dest, Some(&extension), &["conv1".to_string()], &mut report);

    // 数据库记录删除
    assert_eq!(report.db_deleted, 1);
    // 会话目录删除，其余会话不受影响
    assert_eq!(report.history_dirs_removed, 1);
    assert!(!history.join("conv1").exists());
    assert!(history.join("conv2").exists());
    // 辅助目录删除
    assert_eq!(report.auxiliary_dirs_removed, 1);
    assert!(!account_root
        .join("check-point")
        .join("ws1")
        .join("conv1")
        .exists());
    // index.json：条目移除 + current 清空
    let index: Value =
        serde_json::from_slice(&std::fs::read(history.join("index.json")).unwrap()).unwrap();
    assert_eq!(index["conversations"].as_array().unwrap().len(), 1);
    assert_eq!(index["conversations"][0]["id"].as_str(), Some("conv2"));
    assert_eq!(index["current"].as_str(), Some(""));
    assert!(report.errors.is_empty());

    let _ = std::fs::remove_dir_all(&dest);
}

// ─── 列表排序：活动时间回退（last_activity_at → updated_at → created_at） ───

fn raw_session(
    id: &str,
    created_at: Option<i64>,
    updated_at: Option<i64>,
    last_activity_at: Option<i64>,
) -> RawSession {
    RawSession {
        conversation_id: id.to_string(),
        title: id.to_string(),
        cwd: "/ws".to_string(),
        user_id: "uid".to_string(),
        status: "Completed".to_string(),
        created_at,
        updated_at,
        last_activity_at,
        is_deleted: false,
        is_playground: false,
    }
}

#[test]
fn session_sort_uses_newest_activity_time() {
    let raws = vec![
        raw_session("only-created", Some(300), None, None),
        raw_session("by-activity", Some(100), Some(200), Some(500)),
        raw_session("by-updated", Some(100), Some(400), None),
    ];
    let records = aggregate_sessions(
        raws,
        &BuddySessionFilter::default(),
        "workbuddy",
        "WorkBuddy",
    );
    let ids: Vec<&str> = records.iter().map(|r| r.conversation_id.as_str()).collect();
    // 活动时间 = 三者取最新；缺失 last_activity_at 时回退 updated_at / created_at
    assert_eq!(ids, vec!["by-activity", "by-updated", "only-created"]);
    assert_eq!(records[0].last_activity_at, Some(500));
    assert_eq!(records[1].last_activity_at, Some(400));
    assert_eq!(records[2].last_activity_at, Some(300));
}

#[test]
fn session_activity_takes_newest_when_updated_beats_last_activity() {
    // 真机模式（WorkBuddy 5.x）：客户端刷新会 bump updated_at，使其新于 last_activity_at；
    // 此时活动时间必须取 updated_at，否则最近用过的会话会沉底
    let raws = vec![raw_session("real", Some(100), Some(900), Some(500))];
    let records = aggregate_sessions(
        raws,
        &BuddySessionFilter::default(),
        "workbuddy",
        "WorkBuddy",
    );
    assert_eq!(records[0].last_activity_at, Some(900));
}

// ─── 正文存在性探测 ───

#[test]
fn collect_local_conversation_ids_requires_index_entry_and_dir() {
    let root = std::env::temp_dir().join(format!(
        "kira-session-probe-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let uid = "uid-a";
    let history = root
        .join(uid)
        .join("VSCode")
        .join(uid)
        .join("history")
        .join("ws1");
    std::fs::create_dir_all(history.join("conv-ok")).unwrap();
    std::fs::write(
        history.join("index.json"),
        r#"{"conversations":[{"id":"conv-ok"},{"id":"conv-no-dir"}]}"#,
    )
    .unwrap();

    let ids = collect_local_conversation_ids_from_roots(&[root.clone()], uid)
        .expect("存在 history 布局时必须返回集合");
    assert!(ids.contains("conv-ok"));
    // index.json 里登记了、但正文目录不存在 → 不算「本地仍有正文」
    assert!(!ids.contains("conv-no-dir"));

    // 没有 history 布局（5.x 只留 DB / 全新装机）→ None，调用方跳过误报
    let empty = root.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    assert!(collect_local_conversation_ids_from_roots(&[empty], uid).is_none());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn content_probe_marks_only_unknown_ids() {
    let make = |id: &str| BuddySessionRecord {
        conversation_id: id.to_string(),
        title: id.to_string(),
        cwd: "/ws".to_string(),
        user_id: "uid".to_string(),
        status: "Completed".to_string(),
        created_at: Some(1),
        updated_at: Some(1),
        last_activity_at: Some(1),
        is_playground: false,
        content_missing: false,
        locations: Vec::new(),
    };
    let mut records = vec![make("a"), make("b")];

    // 定位不到本地布局 → 一律不标记，避免全量误报
    apply_content_probe(&mut records, None);
    assert!(records.iter().all(|r| !r.content_missing));

    let local: HashSet<String> = ["a".to_string()].into_iter().collect();
    apply_content_probe(&mut records, Some(&local));
    assert!(!records[0].content_missing);
    assert!(records[1].content_missing);
}

#[test]
fn delete_workbuddy_session_files_removes_all_content_paths() {
    let root = temp_dir("del-files");
    let id = "conv-del";
    let project = root.join("projects").join("ws-a");
    std::fs::create_dir_all(project.join(id)).unwrap();
    std::fs::write(project.join(format!("{}.jsonl", id)), "{}").unwrap();
    let project2 = root.join("projects").join("ws-b");
    std::fs::create_dir_all(project2.join(id)).unwrap();
    for dir in [
        root.join("workspace").join("sessions").join(id),
        root.join("tasks").join(id),
        root.join("file-history").join(id),
    ] {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x.txt"), "x").unwrap();
    }
    let artifact = root.join("artifact-index");
    std::fs::create_dir_all(&artifact).unwrap();
    std::fs::write(artifact.join(format!("{}.json", id)), "{}").unwrap();

    // 干扰项：别的会话必须原样保留
    std::fs::write(project.join("conv-keep.jsonl"), "{}").unwrap();
    std::fs::create_dir_all(root.join("tasks").join("conv-keep")).unwrap();

    let removed = delete_workbuddy_session_files(&root, id).unwrap();
    // projects/ws-a 的 jsonl + 目录、projects/ws-b 的目录、workspace、tasks、file-history、artifact
    assert_eq!(removed, 7);
    assert!(!project.join(format!("{}.jsonl", id)).exists());
    assert!(!project.join(id).exists());
    assert!(!project2.join(id).exists());
    assert!(!root.join("workspace").join("sessions").join(id).exists());
    assert!(!root.join("tasks").join(id).exists());
    assert!(!root.join("file-history").join(id).exists());
    assert!(!artifact.join(format!("{}.json", id)).exists());
    assert!(project.join("conv-keep.jsonl").exists());
    assert!(root.join("tasks").join("conv-keep").exists());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn delete_workbuddy_session_files_rejects_unsafe_id() {
    let root = temp_dir("del-unsafe");
    assert!(delete_workbuddy_session_files(&root, "../escape").is_err());
    assert!(delete_workbuddy_session_files(&root, "a/b").is_err());
    assert!(delete_workbuddy_session_files(&root, "").is_err());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn delete_workbuddy_session_cache_drops_only_target_entries() {
    let root = temp_dir("del-cache");
    let app = root.join("app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(
        app.join("sessions.json"),
        r#"{"sessions":[{"conversationId":"gone"},{"conversationId":"keep"}]}"#,
    )
    .unwrap();

    assert_eq!(
        remove_workbuddy_session_cache(&root, &["gone".to_string()]).unwrap(),
        1
    );
    let after: Value =
        serde_json::from_slice(&std::fs::read(app.join("sessions.json")).unwrap()).unwrap();
    let ids: Vec<&str> = after["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["conversationId"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["keep"]);
    // 幂等：再调一次没有可删项
    assert_eq!(
        remove_workbuddy_session_cache(&root, &["gone".to_string()]).unwrap(),
        0
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn delete_workbuddy_sessions_at_deletes_files_cache_then_row() {
    let root = temp_dir("del-e2e");
    let id = "conv-e2e";
    make_workbuddy_db(&root, id, "待删");
    let project = root.join("projects").join("ws");
    std::fs::create_dir_all(project.join(id)).unwrap();
    std::fs::write(project.join(format!("{}.jsonl", id)), "{}").unwrap();
    let app = root.join("app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(
        app.join("sessions.json"),
        format!(r#"[{{"conversationId":"{}"}}]"#, id),
    )
    .unwrap();

    let mut report = BuddySessionDeleteReport::default();
    delete_workbuddy_sessions_at(&root, &[], &[id.to_string()], &mut report);

    assert!(report.errors.is_empty(), "errors: {:?}", report.errors);
    assert_eq!(report.db_deleted, 1);
    assert_eq!(report.files_removed, 2);
    assert!(!project.join(id).exists());
    let rows: i64 = Connection::open(root.join("workbuddy.db"))
        .unwrap()
        .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 0);
    // 缓存条目也清了（否则客户端会从缓存把会话"复活"）
    let cache = std::fs::read_to_string(app.join("sessions.json")).unwrap();
    assert!(!cache.contains(id));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn delete_workbuddy_sessions_at_keeps_row_when_cache_broken() {
    let root = temp_dir("del-bad-cache");
    let id = "conv-bad-cache";
    make_workbuddy_db(&root, id, "缓存坏了");
    let app = root.join("app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(app.join("sessions.json"), "not-json").unwrap();

    let mut report = BuddySessionDeleteReport::default();
    delete_workbuddy_sessions_at(&root, &[], &[id.to_string()], &mut report);

    // 缓存清不掉 → 保留 DB 行作为重试锚点，绝不报"删完了"
    assert_eq!(report.db_deleted, 0);
    assert!(!report.errors.is_empty());
    let rows: i64 = Connection::open(root.join("workbuddy.db"))
        .unwrap()
        .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 1);

    let _ = std::fs::remove_dir_all(&root);
}

/// 真机 2026-10-19：一条会话在 14 个账号目录下都有副本。只删当前账号会留下 13 份，
/// 切号后"复活" —— 所以删除必须扫描扩展目录下的**所有**账号。
#[test]
fn delete_codebuddy_sessions_cleans_every_account_copy() {
    let root = temp_dir("del-cn-multi");
    let user_data = root.join("user-data");
    std::fs::create_dir_all(&user_data).unwrap();
    let db_path = user_data.join("codebuddy-sessions.vscdb");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute_batch("CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value BLOB);")
        .unwrap();
    conn.execute(
        "INSERT INTO ItemTable (key, value) VALUES ('session:conv-x', '{}')",
        [],
    )
    .unwrap();
    drop(conn);

    let extension = root.join("extension");
    let uids = ["uid-a", "uid-b"];
    for uid in uids {
        let account_root = extension.join(uid).join("CodeBuddyIDE").join(uid);
        let ws = account_root.join("history").join("ws1");
        std::fs::create_dir_all(ws.join("conv-x")).unwrap();
        std::fs::write(
            ws.join("index.json"),
            r#"{"conversations":[{"id":"conv-x"}],"current":"conv-x"}"#,
        )
        .unwrap();
        std::fs::create_dir_all(account_root.join("check-point").join("ws1").join("conv-x"))
            .unwrap();
    }

    let mut report = BuddySessionDeleteReport::default();
    delete_codebuddy_sessions_at(
        &user_data,
        Some(&extension),
        &["conv-x".to_string()],
        &mut report,
    );

    assert!(report.errors.is_empty(), "errors: {:?}", report.errors);
    assert_eq!(report.db_deleted, 1);
    for uid in uids {
        let account_root = extension.join(uid).join("CodeBuddyIDE").join(uid);
        let ws = account_root.join("history").join("ws1");
        assert!(!ws.join("conv-x").exists(), "{} 的 history 副本没删", uid);
        assert!(
            !account_root
                .join("check-point")
                .join("ws1")
                .join("conv-x")
                .exists(),
            "{} 的 check-point 副本没删",
            uid
        );
        let index: Value =
            serde_json::from_slice(&std::fs::read(ws.join("index.json")).unwrap()).unwrap();
        assert!(index["conversations"].as_array().unwrap().is_empty());
        assert_eq!(index["current"].as_str(), Some(""));
    }

    let _ = std::fs::remove_dir_all(&root);
}
