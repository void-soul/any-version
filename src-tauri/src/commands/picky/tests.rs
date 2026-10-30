use super::*;

/// 回归测试：Flutter 端导出的 state.json 用 0/1 整数表示 refined/metaFetched，
/// 且带 lingzuCode 等扩展字段。此前 Rust 端 bool 字段解析整数失败，
/// 导致 8 条书签全部被静默丢弃、上传空数据覆盖云端（真实事故）。
#[test]
fn bookmark_parses_flutter_int_bools() {
    let json = serde_json::json!([{
        "id": "e5d94a2f-d53b-4a3e-ab75-4b8e7a6df2ca",
        "title": "测试收藏",
        "description": "描述",
        "url": "https://example.com/a",
        "imageUrl": null,
        "faviconUrl": null,
        "createdAt": "2026-08-22T23:21:55.243827",
        "updatedAt": "2026-08-22T23:22:10.358403",
        "refined": 0,
        "metaFetched": 1,
        "lingzuCode": 8978,
        "lingzuOrigin": "mp.weixin.qq.com",
        "aiCopy": null,
        "aiCopyAt": ""
    }]);
    let arr = json.as_array().unwrap();
    let bm: Bookmark = serde_json::from_value(arr[0].clone()).expect("应能解析 Flutter 整数布尔");
    assert!(!bm.refined);
    assert!(bm.meta_fetched);
    // 扩展字段原样保留
    assert_eq!(
        bm.extra.get("lingzuCode").and_then(|v| v.as_i64()),
        Some(8978)
    );
    assert_eq!(
        bm.extra.get("lingzuOrigin").and_then(|v| v.as_str()),
        Some("mp.weixin.qq.com")
    );
}

/// 序列化方向：any-version 上传的数据必须输出 0/1 整数（与 Flutter toJson 一致），
/// 否则 Flutter 端 `(map['refined'] as int?)` 解析失败。
#[test]
fn bookmark_serializes_int_bools() {
    let bm = Bookmark {
        id: "abc".to_string(),
        refined: true,
        meta_fetched: false,
        ..Default::default()
    };
    let v = serde_json::to_value(&bm).unwrap();
    assert_eq!(v.get("refined").and_then(|x| x.as_i64()), Some(1));
    assert_eq!(v.get("metaFetched").and_then(|x| x.as_i64()), Some(0));
}

/// 兼容旧数据：布尔 true/false 也应能解析。
#[test]
fn bookmark_parses_legacy_bools() {
    let json = serde_json::json!({
        "id": "x",
        "title": "t",
        "url": "https://example.com",
        "createdAt": "2026-01-01T00:00:00",
        "updatedAt": "2026-01-01T00:00:00",
        "refined": true,
        "metaFetched": false,
    });
    let bm: Bookmark = serde_json::from_value(json).expect("布尔也应可解析");
    assert!(bm.refined);
    assert!(!bm.meta_fetched);
}

/// 建一张最小化的内存库（只含合并测试用到的表）。
fn test_conn() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
            "CREATE TABLE picky_bookmarks (
                id TEXT PRIMARY KEY, title TEXT NOT NULL DEFAULT '', description TEXT,
                url TEXT, image_url TEXT, favicon_url TEXT, created_at TEXT NOT NULL DEFAULT '',
                updated_at TEXT NOT NULL DEFAULT '', refined INTEGER NOT NULL DEFAULT 0,
                meta_fetched INTEGER NOT NULL DEFAULT 0, extra TEXT NOT NULL DEFAULT '{}', content TEXT
            );
            CREATE TABLE picky_comments (
                id TEXT PRIMARY KEY, bookmark_id TEXT NOT NULL DEFAULT '',
                content TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL DEFAULT '',
                updated_at TEXT NOT NULL DEFAULT '', parent_id TEXT
            );
            CREATE TABLE picky_tags (
                id TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT '',
                color TEXT NOT NULL DEFAULT '#4FC3F7', created_at TEXT NOT NULL DEFAULT ''
            );
            CREATE TABLE picky_bookmark_tags (
                bookmark_id TEXT NOT NULL, tag_id TEXT NOT NULL, PRIMARY KEY (bookmark_id, tag_id)
            );
            CREATE TABLE picky_tombstones (
                entity TEXT NOT NULL, id TEXT NOT NULL, deleted_at TEXT NOT NULL,
                PRIMARY KEY (entity, id)
            );",
        )
        .unwrap();
    conn
}

/// 回归测试（真实事故）：云同步合并是「并集 + LWW」，本地没有的从云端补。
/// 删掉 A 后同步，云端（还没收到删除）会把 A 补回来 —— 表现就是
/// 「删 A，再删 B，A 又出现了；再删 C，A、B 都在」。删除必须留墓碑。
#[test]
fn merge_cloud_state_does_not_resurrect_deleted_items() {
    let conn = test_conn();
    let a = Bookmark {
        id: "A".into(),
        title: "A".into(),
        ..Default::default()
    };
    let b = Bookmark {
        id: "B".into(),
        title: "B".into(),
        ..Default::default()
    };
    insert_bookmark(&conn, &a).unwrap();
    insert_bookmark(&conn, &b).unwrap();

    // 本地删掉 A：正文清掉 + 留墓碑（与 picky_delete_bookmark 同一套动作）
    conn.execute("DELETE FROM picky_bookmarks WHERE id='A'", [])
        .unwrap();
    record_tombstones(&conn, ENT_BOOKMARK, &["A".to_string()]).unwrap();

    // 云端快照仍是删除前的 A、B
    let cloud = serde_json::json!({ "bookmarks": [a, b] });
    let counts = merge_cloud_state(&conn, &cloud).unwrap();
    assert_eq!(counts.skipped_deleted, 1, "云端旧数据应被墓碑挡住");

    let alive: Vec<String> = conn
        .prepare("SELECT id FROM picky_bookmarks ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|x| x.unwrap())
        .collect();
    assert_eq!(alive, vec!["B".to_string()], "A 不该被复活");
}

/// 墓碑不是「永久封杀」：另一端在删除之后又改过条目（updatedAt 更晚），
/// 说明是对方有意加回来的，应当恢复并清掉墓碑。
#[test]
fn merge_cloud_state_resurrects_when_cloud_is_newer_than_tombstone() {
    let conn = test_conn();
    let mut a = Bookmark {
        id: "A".into(),
        title: "A".into(),
        ..Default::default()
    };
    insert_bookmark(&conn, &a).unwrap();
    conn.execute("DELETE FROM picky_bookmarks WHERE id='A'", [])
        .unwrap();
    record_tombstones(&conn, ENT_BOOKMARK, &["A".to_string()]).unwrap();
    let deleted_at = tombstone_time(&conn, ENT_BOOKMARK, "A").unwrap();

    // 云端这条的 updatedAt 晚于删除时间
    a.updated_at = (chrono::Utc::now() + chrono::Duration::hours(1))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    assert!(incoming_is_newer(&a.updated_at, &deleted_at));
    let counts = merge_cloud_state(&conn, &serde_json::json!({ "bookmarks": [a] })).unwrap();
    assert_eq!(counts.added_bookmarks, 1);
    assert!(
        tombstone_time(&conn, ENT_BOOKMARK, "A").is_none(),
        "复活后墓碑应被清掉"
    );
}

/// 收藏删掉后，它的评论也不该被云端补回来（否则列表里会冒出孤儿评论）。
#[test]
fn merge_cloud_state_skips_comments_of_deleted_bookmark() {
    let conn = test_conn();
    record_tombstones(&conn, ENT_BOOKMARK, &["A".to_string()]).unwrap();
    let cloud = serde_json::json!({
        "comments": [{ "id": "c1", "bookmarkId": "A", "content": "hi",
                       "createdAt": "2026-01-01T00:00:00", "updatedAt": "2026-01-01T00:00:00" }]
    });
    let counts = merge_cloud_state(&conn, &cloud).unwrap();
    assert_eq!(counts.skipped_deleted, 1);
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM picky_comments", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
}
