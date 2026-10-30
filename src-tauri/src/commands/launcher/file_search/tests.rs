use super::{
    escape_like, fts_expr, init_schema, is_opaque_id, is_skipped_dir, normalize_drive,
    resolve_scope, search_with_conn, token_drive, FileHit, QueryScope,
};
use rusqlite::{params, Connection};

#[test]
fn drive_is_normalized() {
    assert_eq!(normalize_drive("c:").unwrap(), "C:");
    assert_eq!(normalize_drive("d:\\").unwrap(), "D:");
    // 缺少冒号或长度不对一律拒绝
    assert!(normalize_drive("c").is_err());
    assert!(normalize_drive("C:\\Windows").is_err());
}

#[test]
fn like_wildcards_are_escaped() {
    assert_eq!(escape_like("100%_ok"), "100\\%\\_ok");
    assert_eq!(escape_like("a\\b"), "a\\\\b");
}

#[test]
fn fts_terms_are_quoted() {
    assert_eq!(fts_expr(&["a b".to_string()]), "\"a b\"");
    // 词内的双引号需要成对转义，否则会破坏 FTS5 查询语法
    assert_eq!(
        fts_expr(&["x\"y".to_string(), "z".to_string()]),
        "\"x\"\"y\" AND \"z\""
    );
}

/// 建一个内存索引库并写入若干条目（同时维护 FTS 行，模拟扫描完成后的状态）。
fn seeded_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    init_schema(&conn).unwrap();
    for (path, name, is_dir) in [
        ("C:\\docs", "docs", 1),
        ("C:\\docs\\report.docx", "report.docx", 0),
        ("C:\\docs\\my_report_2024.docx", "my_report_2024.docx", 0),
        ("C:\\apps\\report.exe", "report.exe", 0),
        ("C:\\tmp\\ab.txt", "ab.txt", 0),
    ] {
        conn.execute(
            "INSERT INTO fs_entry (drive, path, name, name_lower, is_dir, size, modified)
                 VALUES ('C:', ?1, ?2, ?3, ?4, 0, 0)",
            params![path, name, name.to_lowercase(), is_dir],
        )
        .unwrap();
        let id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO fs_entry_fts (rowid, name) VALUES (?1, ?2)",
            params![id, name],
        )
        .unwrap();
    }
    conn
}

fn names(hits: &[FileHit]) -> Vec<String> {
    hits.iter().map(|h| h.name.clone()).collect()
}

#[test]
fn fts_finds_infix_and_ranks_prefix_first() {
    let conn = seeded_db();
    let tokens = vec!["report".to_string()];
    let hits = search_with_conn(&conn, "report", &tokens, 10, &[], "all", &[], None).unwrap();
    let got = names(&hits);
    assert!(got.contains(&"my_report_2024.docx".to_string()));
    // 前缀命中排在包含命中之前
    let prefix_pos = got.iter().position(|n| n == "report.exe").unwrap();
    let infix_pos = got.iter().position(|n| n == "my_report_2024.docx").unwrap();
    assert!(prefix_pos < infix_pos);
}

#[test]
fn short_token_falls_back_to_like() {
    let conn = seeded_db();
    let tokens = vec!["ab".to_string()];
    let hits = search_with_conn(&conn, "ab", &tokens, 10, &[], "all", &[], None).unwrap();
    assert_eq!(names(&hits), vec!["ab.txt".to_string()]);
}

#[test]
fn multi_token_requires_all_terms() {
    let conn = seeded_db();
    let tokens = vec!["report".to_string(), "2024".to_string()];
    let hits = search_with_conn(&conn, "report 2024", &tokens, 10, &[], "all", &[], None).unwrap();
    assert_eq!(names(&hits), vec!["my_report_2024.docx".to_string()]);
}

#[test]
fn drive_filter_limits_results() {
    let conn = seeded_db();
    let tokens = vec!["report".to_string()];
    let hits = search_with_conn(
        &conn,
        "report",
        &tokens,
        10,
        &["D:".to_string()],
        "all",
        &[],
        None,
    )
    .unwrap();
    assert!(hits.is_empty());
}

#[test]
fn dir_only_filter_excludes_files() {
    let conn = seeded_db();
    let tokens = vec!["docs".to_string()];
    let hits = search_with_conn(&conn, "docs", &tokens, 10, &[], "dir", &[], None).unwrap();
    assert_eq!(names(&hits), vec!["docs".to_string()]);
}

#[test]
fn ext_filter_matches_suffix_only() {
    let conn = seeded_db();
    let tokens = vec!["report".to_string()];
    let hits = search_with_conn(
        &conn,
        "report",
        &tokens,
        10,
        &[],
        "file",
        &["docx".to_string()],
        None,
    )
    .unwrap();
    let got = names(&hits);
    assert_eq!(got.len(), 2);
    assert!(got.iter().all(|n| n.ends_with(".docx")));
}

#[test]
fn ext_filter_applies_to_like_fallback() {
    let conn = seeded_db();
    let tokens = vec!["ab".to_string()];
    let hits = search_with_conn(
        &conn,
        "ab",
        &tokens,
        10,
        &[],
        "file",
        &["txt".to_string()],
        None,
    )
    .unwrap();
    assert_eq!(names(&hits), vec!["ab.txt".to_string()]);
}

fn insert_entry(conn: &Connection, drive: &str, path: &str, name: &str, is_dir: bool) {
    conn.execute(
        "INSERT INTO fs_entry (drive, path, name, name_lower, is_dir, size, modified)
             VALUES (?1, ?2, ?3, ?4, ?5, 0, 0)",
        params![drive, path, name, name.to_lowercase(), is_dir],
    )
    .unwrap();
    let id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO fs_entry_fts (rowid, name) VALUES (?1, ?2)",
        params![id, name],
    )
    .unwrap();
}

/// 目录范围测试用：E:/D: 各有一组同名目录，确保前缀比较不是全局命中。
fn seeded_scope_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    init_schema(&conn).unwrap();
    for (drive, path, name, is_dir) in [
        ("E:", "E:\\pro", "pro", true),
        ("E:", "E:\\pro\\gld", "gld", true),
        ("E:", "E:\\pro\\gld\\a1234.txt", "a1234.txt", false),
        ("E:", "E:\\pro\\gld\\b.txt", "b.txt", false),
        ("E:", "E:\\pro\\other", "other", true),
        ("E:", "E:\\pro\\other\\c1234.txt", "c1234.txt", false),
        ("E:", "E:\\work\\gld", "gld", true),
        ("E:", "E:\\work\\gld\\d1234.txt", "d1234.txt", false),
        ("D:", "D:\\pro\\gld", "gld", true),
        ("D:", "D:\\pro\\gld\\e1234.txt", "e1234.txt", false),
    ] {
        insert_entry(&conn, drive, path, name, is_dir);
    }
    conn
}

#[test]
fn drive_token_is_recognized() {
    assert_eq!(token_drive("e").as_deref(), Some("E:"));
    assert_eq!(token_drive("E:").as_deref(), Some("E:"));
    assert_eq!(token_drive("e:\\").as_deref(), Some("E:"));
    assert_eq!(token_drive("c:/").as_deref(), Some("C:"));
    assert!(token_drive("pro").is_none());
    assert!(token_drive("1").is_none());
}

#[test]
fn scope_parses_drive_directories_and_keyword() {
    let conn = seeded_scope_db();
    // e pro gld 1234 → E:\pro\gld 下搜索 1234
    let scope = resolve_scope(&conn, "e pro gld 1234", &[]);
    assert_eq!(
        scope,
        QueryScope {
            drive: Some("E:".to_string()),
            path_prefix: Some("E:\\pro\\gld".to_string()),
            tokens: vec!["1234".to_string()],
        }
    );
    // 大小写与 `E:` 写法都等价
    let scope = resolve_scope(&conn, "E: PRO GLD 1234", &[]);
    assert_eq!(scope.drive.as_deref(), Some("E:"));
    assert_eq!(scope.path_prefix.as_deref(), Some("E:\\pro\\gld"));
    assert_eq!(scope.tokens, vec!["1234".to_string()]);
}

#[test]
fn scope_keeps_unknown_directory_as_keyword() {
    let conn = seeded_scope_db();
    // 不存在的目录不吞词，退回普通多关键词搜索
    let scope = resolve_scope(&conn, "e pro nope 1234", &[]);
    assert_eq!(scope.path_prefix.as_deref(), Some("E:\\pro"));
    assert_eq!(scope.tokens, vec!["nope".to_string(), "1234".to_string()]);
}

#[test]
fn scope_uses_drive_dropdown_when_typed_drive_absent() {
    let conn = seeded_scope_db();
    let scope = resolve_scope(&conn, "pro gld 1234", &["E:".to_string()]);
    assert_eq!(scope.path_prefix.as_deref(), Some("E:\\pro\\gld"));
    assert_eq!(scope.tokens, vec!["1234".to_string()]);
    // 没有盘符也没选盘时，词全部按关键词处理
    let scope = resolve_scope(&conn, "pro gld 1234", &[]);
    assert_eq!(scope.drive, None);
    assert_eq!(scope.path_prefix, None);
    assert_eq!(scope.tokens.len(), 3);
}

#[test]
fn path_prefix_limits_both_fts_and_like_paths() {
    let conn = seeded_scope_db();
    // 全文索引路径：同名文件在 E:\pro\other、E:\work\gld、D: 下都不应出现
    let hits = search_with_conn(
        &conn,
        "1234",
        &["1234".to_string()],
        10,
        &["E:".to_string()],
        "all",
        &[],
        Some("E:\\pro\\gld"),
    )
    .unwrap();
    assert_eq!(names(&hits), vec!["a1234.txt".to_string()]);

    // 短词退化 LIKE 路径同样受目录范围限制
    let hits = search_with_conn(
        &conn,
        "b",
        &["b".to_string()],
        10,
        &["E:".to_string()],
        "all",
        &[],
        Some("E:\\pro\\gld"),
    )
    .unwrap();
    assert_eq!(names(&hits), vec!["b.txt".to_string()]);
}

#[test]
fn cache_dirs_are_pruned() {
    assert!(is_skipped_dir("proj/node_modules", "node_modules"));
    assert!(is_skipped_dir("users/me/appdata/local/temp", "temp"));
    assert!(is_skipped_dir("users/me/appdata/local/yarn/cache", "cache"));
    assert!(is_skipped_dir("windows/winsxs", "winsxs"));
    // 同名但不在缓存位置的目录不误伤
    assert!(!is_skipped_dir("proj/temp", "temp"));
    assert!(!is_skipped_dir("proj/build", "build"));
    // 目录边界校验：`attempt` 不能被 `temp` 后缀命中
    assert!(!is_skipped_dir("proj/attempt", "attempt"));
    // 名字含 cache / session 一律跳过
    assert!(is_skipped_dir("proj/src/cache", "cache"));
    assert!(is_skipped_dir("proj/src/sessions", "sessions"));
    assert!(is_skipped_dir("proj/webpack-cache", "webpack-cache"));
    // GUID / 长哈希目录名
    assert!(is_skipped_dir(
        "files/fromc/codebuddyextension/data/1eacae31-92db-4f15-8c46-19edd7ca5276",
        "1eacae31-92db-4f15-8c46-19edd7ca5276"
    ));
    assert!(is_skipped_dir("proj/.dart_tool", ".dart_tool"));
    assert!(is_skipped_dir(
        "users/me/appdata/local/jetbrains",
        "jetbrains"
    ));
    // 普通目录不误伤
    assert!(!is_skipped_dir("proj/src/utils", "utils"));
    assert!(!is_skipped_dir("proj/docs/history", "history"));
    // 带扩展名的哈希文件名不是不透明 ID
    assert!(!is_opaque_id("a1b2c3d4e5f6a7b8.js"));
    assert!(is_opaque_id("a3f9c81b7e2d4f60"));
}

#[test]
fn deleting_entries_clears_fts() {
    let conn = seeded_db();
    conn.execute(
        "DELETE FROM fs_entry_fts WHERE rowid IN (SELECT id FROM fs_entry WHERE drive = ?1)",
        params!["C:"],
    )
    .unwrap();
    conn.execute("DELETE FROM fs_entry WHERE drive = ?1", params!["C:"])
        .unwrap();
    let fts_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM fs_entry_fts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(fts_count, 0);
    let tokens = vec!["report".to_string()];
    assert!(
        search_with_conn(&conn, "report", &tokens, 10, &[], "all", &[], None)
            .unwrap()
            .is_empty()
    );
}
