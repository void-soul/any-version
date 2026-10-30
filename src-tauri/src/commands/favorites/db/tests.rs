use super::{
    apply_status, apply_tags, category_briefs, category_paths, clear_deleted, count_all,
    create_category, delete, delete_category, find_category_by_name, get_credential,
    get_import_cursor, is_deleted, link_item_category, list, list_category_tree, list_deleted,
    mark_deleted, migrate, move_category, prune_all_empty_categories, rename_category,
    reset_classification, select_unclassified, set_credential, set_import_cursor,
    set_item_categories, set_item_categories_manual, set_starred, set_tags, stats, upsert,
    CategoryNode, FavoriteRow, FavoriteStats, ListFilter, NewFavorite, UpsertOutcome,
};

/// 前后端的字段契约：这两个结构按 camelCase 序列化，前端读的是 `bySource` / `aiLocked`。
/// 一旦 rename_all 被去掉或字段改名，前端只会静默拿到 undefined（来源下拉框只剩
/// 「全部来源」、锁定徽标不显示），所以在这里钉死。
#[test]
fn list_and_stats_serialize_as_camel_case() {
    let overview = FavoriteStats {
        total: 3,
        unclassified: 1,
        gone: 0,
        by_source: vec![("github".to_string(), 3)],
        tags: vec![("CLI".to_string(), 2)],
        categories: vec![],
    };
    let value = serde_json::to_value(&overview).unwrap();
    assert!(value.get("by_source").is_none());
    assert_eq!(value["bySource"][0][0], "github");
    assert_eq!(value["bySource"][0][1], 3);

    let row = FavoriteRow {
        id: 1,
        source: "github".to_string(),
        external_id: "42".to_string(),
        url: "https://github.com/a/b".to_string(),
        title: "a/b".to_string(),
        subtitle: None,
        description: None,
        status: "ok".to_string(),
        checked_at: None,
        ai_locked: true,
        ai_model: Some("m".to_string()),
        created_at: "2026-01-01".to_string(),
        updated_at: "2026-01-01".to_string(),
        favorited_at: None,
        is_starred: false,
        tags: vec![],
    };
    let value = serde_json::to_value(&row).unwrap();
    assert!(value.get("ai_locked").is_none());
    assert_eq!(value["aiLocked"], true);
    assert_eq!(value["externalId"], "42");
    // 收藏时间字段也要能被前端读到（列表里要显示、排序要用）
    assert!(value.get("favoritedAt").is_some());
}

fn sample(external_id: &str, title: &str) -> NewFavorite {
    NewFavorite {
        source: "github".to_string(),
        external_id: external_id.to_string(),
        url: format!("https://github.com/{}", title),
        title: title.to_string(),
        subtitle: Some(title.to_string()),
        description: Some("desc".to_string()),
        extra_json: Some("{}".to_string()),
        favorited_at: None,
        initial_status: None,
    }
}

// 造一条收藏条目（测试用；is_starred 走默认 0）
fn insert_fav(conn: &rusqlite::Connection, id: i64, external_id: &str) {
    conn.execute(
            "INSERT INTO favorite (id, source, external_id, url, title, extra_json, created_at, updated_at) \
             VALUES (?1, 'github', ?2, 'https://x', ?2, '{}', 'now', 'now')",
            rusqlite::params![id, external_id],
        )
        .unwrap();
}

// 星标置顶：星标过的条目不该被新收藏的涌到后面（否则“关注的那几条”越用越难找）
#[test]
fn starred_items_sort_above_newer_ones() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();
    insert_fav(&conn, 1, "old");
    insert_fav(&conn, 2, "new");
    set_starred(&conn, 1, true).unwrap();
    let rows = list(&conn, &ListFilter::default()).unwrap();
    assert_eq!(rows[0].id, 1, "星标条目应置顶，即使它更旧");
    assert!(rows[0].is_starred);
    assert!(!rows[1].is_starred);
}

/// 只筛星标：星标 + 非星标混合时，只有星标进结果（左侧「星标」入口靠它）。
#[test]
fn starred_only_filter_returns_only_starred() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();
    insert_fav(&conn, 1, "a");
    insert_fav(&conn, 2, "b");
    set_starred(&conn, 1, true).unwrap();
    let rows = list(
        &conn,
        &ListFilter {
            starred_only: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, 1);
}

// ─── 删除墓碑：删掉的条目不该被下次导入捞回来 ───

/// 删掉的条目再次 upsert 应被墓碑挡住，清碑后又能进来。
#[test]
fn deleted_item_does_not_come_back_on_reimport() {
    let mut conn = rusqlite::Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();

    let item = sample("42", "a/b");
    assert_eq!(upsert(&conn, &item).unwrap(), UpsertOutcome::Added);

    let id: i64 = conn
        .query_row(
            "SELECT id FROM favorite WHERE external_id = '42'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(delete(&mut conn, id).unwrap());
    assert_eq!(count_all(&conn).unwrap(), 0);

    // 平台上的收藏还在，导入一定会再拉到它 —— 但墓碑要把它挡住
    assert_eq!(upsert(&conn, &item).unwrap(), UpsertOutcome::Deleted);
    assert_eq!(count_all(&conn).unwrap(), 0, "删掉的条目被导入复活了");

    // 墓碑可查、可清；清掉之后这条才会重新进来
    let tombs = list_deleted(&conn).unwrap();
    assert_eq!(tombs.len(), 1);
    assert_eq!(tombs[0].source, "github");
    assert_eq!(tombs[0].external_id, "42");
    assert_eq!(clear_deleted(&conn, Some("github"), Some("42")).unwrap(), 1);
    assert_eq!(upsert(&conn, &item).unwrap(), UpsertOutcome::Added);
    assert_eq!(count_all(&conn).unwrap(), 1);
}

/// 只清某个来源：别的来源的墓碑不受影响。
#[test]
fn clearing_one_source_keeps_other_tombstones() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();
    mark_deleted(&conn, "github", "1", "a/b").unwrap();
    mark_deleted(&conn, "bilibili", "BV1", "视频").unwrap();
    assert_eq!(clear_deleted(&conn, Some("github"), None).unwrap(), 1);
    let left = list_deleted(&conn).unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].source, "bilibili");
    // 都不传 = 全部恢复
    assert_eq!(clear_deleted(&conn, None, None).unwrap(), 1);
    assert!(list_deleted(&conn).unwrap().is_empty());
}

/// 书签的 external_id 是 URL：`#片段` / 末尾斜杠 / 大小写差异都要算同一条。
#[test]
fn bookmark_tombstone_key_ignores_fragment_and_trailing_slash() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();
    mark_deleted(&conn, "bookmark", "https://Example.com/docs/", "Docs").unwrap();
    assert!(is_deleted(&conn, "bookmark", "https://example.com/docs").unwrap());
    assert!(is_deleted(&conn, "bookmark", "https://example.com/docs/#section").unwrap());
    // GitHub 用仓库 id 当 external_id，不能被 URL 归一化改坏
    mark_deleted(&conn, "github", "12345", "o/r").unwrap();
    assert!(is_deleted(&conn, "github", "12345").unwrap());
}

/// 墓碑与「失效(status=gone)」是两回事：后者条目仍在库里。
#[test]
fn tombstone_is_not_the_same_as_gone_status() {
    let mut conn = rusqlite::Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();
    let mut item = sample("7", "c/d");
    item.initial_status = Some("gone".to_string());
    assert_eq!(upsert(&conn, &item).unwrap(), UpsertOutcome::Added);
    assert_eq!(count_all(&conn).unwrap(), 1);
    assert!(!is_deleted(&conn, "github", "7").unwrap());
    // 失效条目被用户清理掉 → 也要立碑，下次导入不再出现
    let id: i64 = conn
        .query_row("SELECT id FROM favorite WHERE external_id = '7'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(delete(&mut conn, id).unwrap());
    assert!(is_deleted(&conn, "github", "7").unwrap());
    assert_eq!(upsert(&conn, &item).unwrap(), UpsertOutcome::Deleted);
}

#[test]
fn list_sorts_and_filters_by_favorited_time() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();

    let mut old = sample("1", "old");
    old.favorited_at = Some("2020-05-01T10:00:00".to_string());
    let mut recent = sample("2", "recent");
    recent.favorited_at = Some("2026-05-01T10:00:00".to_string());
    upsert(&conn, &old).unwrap();
    upsert(&conn, &recent).unwrap();

    // 按收藏时间排序：最近收藏的在前（而不是按入库/更新时间）
    let sorted = list(
        &conn,
        &ListFilter {
            sort: Some("favorited".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(sorted[0].external_id, "2");
    assert_eq!(
        sorted[0].favorited_at.as_deref(),
        Some("2026-05-01T10:00:00")
    );
    assert_eq!(sorted[1].external_id, "1");

    // 时间过滤：只留收藏时间不早于 2026 的
    let filtered = list(
        &conn,
        &ListFilter {
            favorited_since: Some("2026-01-01T00:00:00".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].external_id, "2");

    // 没有平台收藏时间的条目回退用入库时间判断，不会被时间过滤直接漏掉
    let no_time = sample("3", "no-time");
    upsert(&conn, &no_time).unwrap();
    let all = list(
        &conn,
        &ListFilter {
            favorited_since: Some("2000-01-01T00:00:00".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(all.len(), 3);
}

#[test]
fn reimport_backfills_favorited_time_without_touching_known_values() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();

    // 老库（或升级前导入）没有收藏时间：再次导入带上时间时应算一次更新并补齐
    let mut item = sample("9", "backfill");
    assert_eq!(upsert(&conn, &item).unwrap(), UpsertOutcome::Added);
    item.favorited_at = Some("2021-03-04T05:06:07".to_string());
    assert_eq!(upsert(&conn, &item).unwrap(), UpsertOutcome::Updated);
    let rows = list(&conn, &ListFilter::default()).unwrap();
    assert_eq!(rows[0].favorited_at.as_deref(), Some("2021-03-04T05:06:07"));

    // 本次没取到时间（如平台接口不再返回）时保留库里已有的值，不覆盖成 NULL
    let mut without_time = sample("9", "backfill");
    without_time.description = Some("changed".to_string());
    assert_eq!(
        upsert(&conn, &without_time).unwrap(),
        UpsertOutcome::Updated
    );
    let rows = list(&conn, &ListFilter::default()).unwrap();
    assert_eq!(rows[0].favorited_at.as_deref(), Some("2021-03-04T05:06:07"));
}

#[test]
fn schema_creates_all_three_tables() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' \
                 AND name IN ('favorite','favorite_tag','favorite_import_state')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 3);
}

/// 建表必须幂等：命令层每次调用都会走到这里。
#[test]
fn migrate_is_idempotent() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();
    migrate(&conn).unwrap();
}

/// 去重靠的是 (source, external_id) 唯一约束，缺了它整个模块的前提就没了。
#[test]
fn duplicate_source_and_external_id_is_rejected() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();
    let insert = |external_id: &str| -> rusqlite::Result<usize> {
        conn.execute(
            "INSERT INTO favorite (source, external_id, url, title, created_at, updated_at) \
                 VALUES ('github', ?1, 'https://x', 't', 'now', 'now')",
            [external_id],
        )
    };
    assert!(insert("42").is_ok());
    let err = insert("42").unwrap_err();
    assert!(
        matches!(err, rusqlite::Error::SqliteFailure(_, _)),
        "重复 (source, external_id) 必须被唯一约束拦下，实际: {:?}",
        err
    );
    // 不同 source 的同 id 不算冲突
    conn.execute(
        "INSERT INTO favorite (source, external_id, url, title, created_at, updated_at) \
             VALUES ('zhihu', '42', 'https://x', 't', 'now', 'now')",
        [],
    )
    .unwrap();
}

/// 第二次导入同一批数据必须「一个都不新增」——这是用户对重复导入的核心要求。
#[test]
fn second_import_is_idempotent() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();
    let item = sample("1", "o/r");
    assert_eq!(upsert(&conn, &item).unwrap(), UpsertOutcome::Added);
    assert_eq!(upsert(&conn, &item).unwrap(), UpsertOutcome::Skipped);
    assert_eq!(count_all(&conn).unwrap(), 1);
}

/// 内容变了（仓库改名 / 描述更新）要更新，而不是无视。
#[test]
fn changed_fields_are_updated_in_place() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();
    upsert(&conn, &sample("1", "o/r")).unwrap();
    let mut renamed = sample("1", "o/r");
    renamed.title = "o/renamed".to_string();
    assert_eq!(upsert(&conn, &renamed).unwrap(), UpsertOutcome::Updated);
    assert_eq!(count_all(&conn).unwrap(), 1, "更新不能变成新增一行");
    let title: String = conn
        .query_row(
            "SELECT title FROM favorite WHERE external_id = '1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(title, "o/renamed");
}

/// GitHub 用仓库 id 做 external_id：仓库改名（full_name 变了）仍是同一条记录。
#[test]
fn renaming_full_name_does_not_duplicate() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();
    let first = sample("42", "old/name");
    let mut renamed = sample("42", "old/name");
    renamed.title = "new/name".to_string();
    renamed.url = "https://github.com/new/name".to_string();
    upsert(&conn, &first).unwrap();
    upsert(&conn, &renamed).unwrap();
    assert_eq!(count_all(&conn).unwrap(), 1, "改名不能导进重复条目");
}

/// 不同平台的 id 撞车不算同一条（外部 id 只在同 source 内唯一）。
#[test]
fn same_external_id_across_sources_stays_separate() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();
    let mut zhihu = sample("42", "x/y");
    zhihu.source = "zhihu".to_string();
    upsert(&conn, &sample("42", "x/y")).unwrap();
    upsert(&conn, &zhihu).unwrap();
    assert_eq!(count_all(&conn).unwrap(), 2);
}

fn insert_raw(conn: &rusqlite::Connection, id: i64, external_id: &str, extra: &str) {
    conn.execute(
            "INSERT INTO favorite (id, source, external_id, url, title, extra_json, created_at, updated_at) \
             VALUES (?1, 'github', ?2, 'https://x', ?2, ?3, 'now', 'now')",
            rusqlite::params![id, external_id, extra],
        )
        .unwrap();
}

/// 断点游标：按键读写、可清除；跨天续传靠它把配额花在新内容上。
#[test]
fn import_cursor_roundtrip_and_clear() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();
    let key = "zhihu:123456789";
    assert_eq!(get_import_cursor(&conn, key).unwrap(), None);

    set_import_cursor(&conn, key, Some("40")).unwrap();
    assert_eq!(
        get_import_cursor(&conn, key).unwrap().as_deref(),
        Some("40")
    );

    // 不同键互不影响（B站/其它收藏夹各有各的断点）
    set_import_cursor(&conn, "zhihu:999", Some("20")).unwrap();
    assert_eq!(
        get_import_cursor(&conn, key).unwrap().as_deref(),
        Some("40")
    );

    // 导完后清除：下次导入从头检查是否有新内容
    set_import_cursor(&conn, key, None).unwrap();
    assert_eq!(get_import_cursor(&conn, key).unwrap(), None);
    // 清除不能动别的键
    assert_eq!(
        get_import_cursor(&conn, "zhihu:999").unwrap().as_deref(),
        Some("20")
    );
}

/// 凭证按 source 隔离：收藏模块的 GitHub Token 与 B站 Cookie 互不影响，
/// 也就不会出现「换了 B站 Cookie 把 GitHub Token 顶掉」这种事。
#[test]
fn credentials_are_stored_per_source() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();
    assert_eq!(get_credential(&conn, "github").unwrap(), None);

    set_credential(&conn, "github", "ghp_xxx").unwrap();
    set_credential(&conn, "bilibili", "SESSDATA=yyy").unwrap();
    assert_eq!(
        get_credential(&conn, "github").unwrap().as_deref(),
        Some("ghp_xxx")
    );
    assert_eq!(
        get_credential(&conn, "bilibili").unwrap().as_deref(),
        Some("SESSDATA=yyy")
    );

    // 覆盖同一个 source 不影响另一个；空串即清除
    set_credential(&conn, "github", "ghp_new").unwrap();
    assert_eq!(
        get_credential(&conn, "bilibili").unwrap().as_deref(),
        Some("SESSDATA=yyy")
    );
    set_credential(&conn, "github", "").unwrap();
    assert_eq!(get_credential(&conn, "github").unwrap(), None);
    assert_eq!(get_credential(&conn, "bilibili").unwrap().is_some(), true);
}

/// 已失效的条目不再送归类（给它分类没意义，还白花 token）。
#[test]
fn select_unclassified_skips_gone_items() {
    let conn = seeded();
    apply_status(&conn, 2, "gone", None).unwrap();
    let pending = select_unclassified(&conn, 10).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, 1);
}

/// 多级分类：父子层级、按层级筛选（父级要能看到子级的东西）、成环一律拒绝。
#[test]
fn category_tree_supports_nesting_and_filtering() {
    let conn = seeded();
    let root = create_category(&conn, "技术", None, "topic").unwrap();
    let child = create_category(&conn, "编译器", Some(root), "topic").unwrap();
    let grand = create_category(&conn, "LLVM", Some(child), "topic").unwrap();

    // 同级重名要拦住（父级不同则可以同名）
    assert!(create_category(&conn, "技术", None, "topic").is_err());
    assert!(create_category(&conn, "编译器", Some(grand), "topic").is_ok());

    set_item_categories(&conn, 1, &[grand]).unwrap();
    set_item_categories(&conn, 2, &[]).unwrap();

    // 点父分类要能把子孙分类下的条目一起筛出来
    assert_eq!(
        list(
            &conn,
            &ListFilter {
                category_id: Some(root),
                ..Default::default()
            }
        )
        .unwrap()
        .len(),
        1,
        "按根分类筛选应包含子分类里的条目"
    );
    assert_eq!(
        list(
            &conn,
            &ListFilter {
                category_id: Some(grand),
                ..Default::default()
            }
        )
        .unwrap()
        .len(),
        1
    );

    // 不能移到自己 / 自己子孙下面（成环）
    assert!(move_category(&conn, root, Some(root)).is_err());
    assert!(move_category(&conn, root, Some(grand)).is_err());
    assert!(move_category(&conn, grand, None).is_ok());

    // 删分类只解开关联，不删条目
    // grand 已被移到顶层，所以这次删的是 root + child 两级
    let removed = delete_category(&conn, root).unwrap();
    assert_eq!(removed, 2, "子孙分类要一起删");
    assert!(
        find_category_by_name(&conn, "LLVM").unwrap().is_some(),
        "移走的分类不受影响"
    );
    assert_eq!(count_all(&conn).unwrap(), 2, "条目不能跟着分类一起没");
}

/// 整理类目依赖的概览：完整路径要沿父级拼出来（路径只给预览用，不进 prompt）。
#[test]
fn category_briefs_and_paths_include_hierarchy() {
    let conn = seeded();
    let root = create_category(&conn, "技术", None, "topic").unwrap();
    let child = create_category(&conn, "编译器", Some(root), "topic").unwrap();
    let grand = create_category(&conn, "LLVM", Some(child), "topic").unwrap();
    set_item_categories(&conn, 1, &[grand]).unwrap();
    set_item_categories(&conn, 2, &[child]).unwrap();

    let paths = category_paths(&conn, 20).unwrap();
    assert!(paths.contains(&"技术/编译器/LLVM".to_string()));
    assert!(paths.contains(&"技术/编译器".to_string()));

    let briefs = category_briefs(&conn).unwrap();
    let hit = briefs.iter().find(|b| b.id == child).unwrap();
    assert_eq!(hit.path, "技术/编译器");
    assert_eq!(hit.parent_id, Some(root));
}

/// 分类树的条目数：自己挂的 + 所有子孙的（侧栏显示这个才有意义）。
#[test]
fn category_tree_counts_include_descendants() {
    let conn = seeded();
    let root = create_category(&conn, "A", None, "topic").unwrap();
    let child = create_category(&conn, "B", Some(root), "topic").unwrap();
    set_item_categories(&conn, 1, &[root]).unwrap();
    set_item_categories(&conn, 2, &[child]).unwrap();

    let tree = list_category_tree(&conn).unwrap();
    assert_eq!(tree.len(), 1);
    assert_eq!(tree[0].count, 1);
    assert_eq!(tree[0].total, 2, "父级要算上子分类的条目");
    assert_eq!(tree[0].children.len(), 1);
    assert_eq!(tree[0].children[0].total, 1);

    rename_category(&conn, child, "B2").unwrap();
    let tree = list_category_tree(&conn).unwrap();
    assert_eq!(tree[0].children[0].name, "B2");
    let _: CategoryNode = tree[0].clone();
}

/// 旧库升级：扁平标签要变成顶层分类，条目关联不能丢。
#[test]
fn legacy_tags_are_migrated_into_category_tree() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    // 先建一张只有旧标签表的库，再补插旧数据，最后跑新版 migrate
    conn.execute_batch(
            "CREATE TABLE favorite_tag (favorite_id INTEGER NOT NULL, tag TEXT NOT NULL, PRIMARY KEY (favorite_id, tag));",
        )
        .unwrap();
    conn.execute("INSERT INTO favorite_tag VALUES (1, 'CLI')", [])
        .unwrap();
    conn.execute("INSERT INTO favorite_tag VALUES (1, 'Web')", [])
        .unwrap();
    migrate(&conn).unwrap();

    assert_eq!(find_category_by_name(&conn, "CLI").unwrap(), Some(1));
    let linked: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM favorite_item_category WHERE favorite_id = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(linked, 2, "两个旧标签都要变成分类关联");
    // 再迁移一次不能重复建
    migrate(&conn).unwrap();
    assert_eq!(list_category_tree(&conn).unwrap().len(), 2);
}

/// 归类候选 = 「AI 还没归类过」且未人工锁定、未失效。
///
/// 关键回归：**只有分类（浏览器书签带来的目录）但没有 ai_model 的条目仍然要进候选** ——
/// 否则浏览器收藏在收藏库里永远是二等公民，AI 归类永远碰不到它们。
#[test]
fn select_unclassified_skips_locked_classified_and_gone() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();
    insert_raw(&conn, 1, "1", "{}");
    insert_raw(&conn, 2, "2", "{}");
    insert_raw(&conn, 3, "3", "{}");
    insert_raw(&conn, 4, "4", "{}");

    let pending = select_unclassified(&conn, 10).unwrap();
    assert_eq!(pending.len(), 4);

    // 只有目录分类（书签导入形态）、没有 ai_model → 仍要归类
    let cli = create_category(&conn, "CLI", None, "topic").unwrap();
    set_item_categories(&conn, 1, &[cli]).unwrap();
    // 人工改分类（界面选择器）会顺手锁定 → 永远不再归类
    set_item_categories_manual(&conn, 2, &[cli]).unwrap();
    // AI 归类过（ai_model 非空）→ 不再重复归类
    conn.execute("UPDATE favorite SET ai_model = 'gpt-x' WHERE id = 3", [])
        .unwrap();
    // 已失效 → 跳过
    apply_status(&conn, 4, "gone", None).unwrap();

    let pending = select_unclassified(&conn, 10).unwrap();
    assert_eq!(pending.len(), 1, "只剩「有目录分类但没 AI 归类过」的那条");
    assert_eq!(pending[0].id, 1);
}

/// 多标签：一个条目落两个分类要写两行；写完之后它就不再是「待归类」。
#[test]
fn apply_tags_writes_every_label_and_clears_pending() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();
    insert_raw(&conn, 1, "1", "{}");
    let items = select_unclassified(&conn, 10).unwrap();
    let groups = vec![
        (vec!["LLM".to_string()], vec![0usize]),
        (vec!["运维".to_string()], vec![0usize]),
    ];
    let written = apply_tags(&conn, &items, &groups, "gpt-x", false).unwrap();
    assert_eq!(written, 2, "多标签每条都要写");

    let tags: Vec<String> = conn
        .prepare(
            "SELECT c.name FROM favorite_item_category ic \
                 JOIN favorite_category c ON c.id = ic.category_id \
                 WHERE ic.favorite_id = 1 ORDER BY c.name",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();
    assert_eq!(tags, vec!["LLM".to_string(), "运维".to_string()]);
    assert!(select_unclassified(&conn, 10).unwrap().is_empty());

    let model: String = conn
        .query_row("SELECT ai_model FROM favorite WHERE id = 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(model, "gpt-x");
}

/// 多级分类名要建成真正的父子层级：条目挂到叶子，父分类是叶子分类的 parent。
#[test]
fn apply_tags_builds_multi_level_tree() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();
    insert_raw(&conn, 1, "1", "{}");
    let items = select_unclassified(&conn, 10).unwrap();

    let groups = vec![(
        vec!["编程语言".to_string(), "Rust".to_string()],
        vec![0usize],
    )];
    let written = apply_tags(&conn, &items, &groups, "gpt-x", false).unwrap();
    assert_eq!(written, 1);

    let root_id = find_category_by_name(&conn, "编程语言").unwrap().unwrap();
    let leaf_id = find_category_by_name(&conn, "Rust").unwrap().unwrap();
    let parent: Option<i64> = conn
        .query_row(
            "SELECT parent_id FROM favorite_category WHERE id = ?1",
            [leaf_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(parent, Some(root_id), "叶子分类应挂在父分类下");

    // 条目挂在叶子分类上，且来源是 ai
    let (linked, source): (i64, String) = conn
        .query_row(
            "SELECT COUNT(*), COALESCE(MAX(source), '') FROM favorite_item_category \
                 WHERE favorite_id = 1 AND category_id = ?1",
            [leaf_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(linked, 1);
    assert_eq!(source, "ai");
}

/// 重新归类只清 AI 加的关联，书签目录与人工选择都保留。
#[test]
fn reclassify_clears_only_ai_links() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();
    insert_raw(&conn, 1, "1", "{}");

    // 书签目录分类（bookmark）
    let bm = create_category(&conn, "书签栏", None, "topic").unwrap();
    link_item_category(&conn, 1, bm).unwrap();

    // 第一次 AI 归类（ai）→ 旧分类
    let items = select_unclassified(&conn, 10).unwrap();
    apply_tags(
        &conn,
        &items,
        &[(vec!["旧分类".to_string()], vec![0usize])],
        "gpt-x",
        false,
    )
    .unwrap();
    let old_id = find_category_by_name(&conn, "旧分类").unwrap().unwrap();

    // 重新归类：先清掉 AI 标记，已归类过的条目重新进「未归类」批次
    let reset = reset_classification(&conn).unwrap();
    assert_eq!(reset, 1, "只有 AI 归类过的那条被重置");
    let items = select_unclassified(&conn, 10).unwrap();
    assert_eq!(items.len(), 1, "重置后已归类的条目也要进重归批次");
    apply_tags(
        &conn,
        &items,
        &[(vec!["新分类".to_string()], vec![0usize])],
        "gpt-x",
        true,
    )
    .unwrap();

    let link_count = |cat_id: i64| -> i64 {
        conn.query_row(
                "SELECT COUNT(*) FROM favorite_item_category WHERE favorite_id = 1 AND category_id = ?1",
                [cat_id],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_eq!(link_count(old_id), 0, "旧的 AI 分类要被清掉");
    assert_eq!(link_count(bm), 1, "书签目录分类要保留");
    assert_eq!(
        link_count(find_category_by_name(&conn, "新分类").unwrap().unwrap()),
        1
    );
}

/// 重新归类后的空分类清理：只删「既无关联又无子分类」的，且级联向上收掉变空的祖先。
#[test]
fn prune_all_empty_categories_removes_only_empty() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();

    // 有内容的分类（保留）
    let used = create_category(&conn, "有内容", None, "topic").unwrap();
    link_item_category(&conn, 1, used).unwrap();

    // 空叶子（删）
    let _ = create_category(&conn, "空叶子", None, "topic").unwrap();

    // 空父 + 空子：父子都空 → 级联删
    let empty_parent = create_category(&conn, "空父", None, "topic").unwrap();
    let _ = create_category(&conn, "空子", Some(empty_parent), "topic").unwrap();

    // 父分类下有「有内容」的子分类：父不删
    let parent = create_category(&conn, "父", None, "topic").unwrap();
    let used_child = create_category(&conn, "有内容子", Some(parent), "topic").unwrap();
    link_item_category(&conn, 1, used_child).unwrap();

    let removed = prune_all_empty_categories(&conn).unwrap();
    assert_eq!(removed, 3, "空叶子 + 空父 + 空子 三个被删");

    assert!(find_category_by_name(&conn, "有内容").unwrap().is_some());
    assert!(find_category_by_name(&conn, "父").unwrap().is_some());
    assert!(find_category_by_name(&conn, "有内容子").unwrap().is_some());
    assert!(find_category_by_name(&conn, "空叶子").unwrap().is_none());
    assert!(find_category_by_name(&conn, "空父").unwrap().is_none());
    assert!(find_category_by_name(&conn, "空子").unwrap().is_none());
}

/// 归类要能读到语言与 topics（这是判断分类最有用的两个信号）。
#[test]
fn select_unclassified_reads_language_and_topics() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();
    insert_raw(
        &conn,
        1,
        "o/r",
        r#"{"language":"Rust","topics":["cli","rust"]}"#,
    );
    let items = select_unclassified(&conn, 10).unwrap();
    assert_eq!(items[0].language, "Rust");
    assert_eq!(items[0].topics, vec!["cli".to_string(), "rust".to_string()]);
}

fn seeded() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();
    insert_raw(&conn, 1, "1", r#"{"language":"Rust"}"#);
    insert_raw(&conn, 2, "2", r#"{"language":"Go"}"#);
    conn.execute(
        "UPDATE favorite SET title = 'o/cli', description = 'a fast cli' WHERE id = 1",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE favorite SET title = 'o/web', description = 'web framework' WHERE id = 2",
        [],
    )
    .unwrap();
    conn
}

/// 人工改标签 = 全量替换 + 锁定（用户的选择优先于模型）。
#[test]
fn set_tags_replaces_and_locks() {
    let conn = seeded();
    set_tags(&conn, 1, &["CLI".to_string(), "工具".to_string()]).unwrap();
    let row = &list(&conn, &ListFilter::default())
        .unwrap()
        .into_iter()
        .find(|r| r.id == 1)
        .unwrap();
    assert_eq!(row.tags, vec!["CLI".to_string(), "工具".to_string()]);
    assert!(row.ai_locked);
    assert!(select_unclassified(&conn, 10)
        .unwrap()
        .iter()
        .all(|i| i.id != 1));

    // 再设一次：旧标签被替换而不是叠加
    set_tags(&conn, 1, &["CLI".to_string()]).unwrap();
    let row = &list(&conn, &ListFilter::default())
        .unwrap()
        .into_iter()
        .find(|r| r.id == 1)
        .unwrap();
    assert_eq!(row.tags, vec!["CLI".to_string()]);
}

/// 删本地条目要连标签一起删（不留孤儿行）。
#[test]
fn delete_removes_tags_too() {
    let mut conn = seeded();
    set_tags(&conn, 1, &["CLI".to_string()]).unwrap();
    assert!(delete(&mut conn, 1).unwrap());
    assert!(!delete(&mut conn, 1).unwrap(), "删第二次应返回 false");
    let left: i64 = conn
        .query_row("SELECT COUNT(*) FROM favorite_tag", [], |r| r.get(0))
        .unwrap();
    assert_eq!(left, 0);
    assert_eq!(count_all(&conn).unwrap(), 1);
}

#[test]
fn list_filters_by_source_tag_and_keyword() {
    let conn = seeded();
    set_tags(&conn, 1, &["CLI".to_string()]).unwrap();
    set_tags(&conn, 2, &["Web".to_string()]).unwrap();

    assert_eq!(list(&conn, &ListFilter::default()).unwrap().len(), 2);
    let cli_id = find_category_by_name(&conn, "CLI")
        .unwrap()
        .expect("CLI 分类已建");
    assert_eq!(
        list(
            &conn,
            &ListFilter {
                category_id: Some(cli_id),
                ..Default::default()
            }
        )
        .unwrap()
        .len(),
        1
    );
    // 关键字同时匹配标题与描述
    assert_eq!(
        list(
            &conn,
            &ListFilter {
                keyword: Some("cli".into()),
                ..Default::default()
            }
        )
        .unwrap()
        .len(),
        1
    );
    assert_eq!(
        list(
            &conn,
            &ListFilter {
                keyword: Some("framework".into()),
                ..Default::default()
            }
        )
        .unwrap()
        .len(),
        1
    );
    assert_eq!(
        list(
            &conn,
            &ListFilter {
                source: Some("zhihu".into()),
                ..Default::default()
            }
        )
        .unwrap()
        .len(),
        0
    );
}

#[test]
fn stats_counts_unclassified_and_gone() {
    let conn = seeded();
    let before = stats(&conn).unwrap();
    assert_eq!(before.total, 2);
    assert_eq!(before.unclassified, 2);
    assert_eq!(before.gone, 0);

    set_tags(&conn, 1, &["CLI".to_string()]).unwrap();
    apply_status(&conn, 2, "gone", None).unwrap();
    let after = stats(&conn).unwrap();
    assert_eq!(after.unclassified, 0, "已归类的 + 已失效的都不再送归类");
    assert_eq!(after.gone, 1);
    assert_eq!(after.tags, vec![("CLI".to_string(), 1)]);
}

/// 多标签：同一条目可挂多个标签，但完全相同的 (条目, 标签) 只能有一条。
#[test]
fn tags_allow_multiple_labels_per_item() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();
    conn.execute(
        "INSERT INTO favorite (id, source, external_id, url, title, created_at, updated_at) \
             VALUES (1, 'github', '42', 'https://x', 't', 'now', 'now')",
        [],
    )
    .unwrap();
    let tag = |t: &str| {
        conn.execute(
            "INSERT INTO favorite_tag (favorite_id, tag) VALUES (1, ?1)",
            [t],
        )
    };
    assert!(tag("LLM").is_ok());
    assert!(tag("运维").is_ok(), "多标签必须允许同一条目挂第二个分类");
    assert!(tag("LLM").is_err(), "同一标签不能重复挂");
}
