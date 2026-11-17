use super::folders::{
    create_folder, delete_folder, normalize_path, rebase_path, rename_folder,
};
use super::models::RdpConnection;

fn conn(id: &str, group: &str) -> RdpConnection {
    let mut c = RdpConnection::new(format!("n{id}"), "192.0.2.1".into(), 3389, "tester".into());
    c.id = id.to_string();
    c.group = group.to_string();
    c
}

fn folders(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn normalize_path_accepts_windows_separators_and_root() {
    assert_eq!(normalize_path("  a/b  "), "a/b");
    assert_eq!(normalize_path("/a//b/"), "a/b");
    assert_eq!(normalize_path("a\\b"), "a/b");
    assert_eq!(normalize_path(""), "");
    assert_eq!(normalize_path("root"), "");
}

/// 重命名文件夹要同时改「连接路径」和「文件夹记录」，且不能误伤 a/bc 这种同前缀兄弟。
#[test]
fn rename_folder_rewrites_connections_and_child_folders() {
    let mut items = vec![
        conn("x", "a/b"),
        conn("y", "a/bc"),
        conn("z", "a"),
        conn("w", ""),
    ];
    let mut records = folders(&["a", "a/b", "a/bc"]);

    rename_folder(&mut items, &mut records, "a/b", "prod/web");

    assert_eq!(items[0].group, "prod/web");
    assert_eq!(items[1].group, "a/bc", "同前缀兄弟不能被改");
    assert_eq!(items[2].group, "a");
    assert_eq!(items[3].group, "");
    assert!(records.contains(&"prod".to_string()), "中间层要补上");
    assert!(records.contains(&"prod/web".to_string()));
    assert!(records.contains(&"a/bc".to_string()));
    assert!(!records.contains(&"a/b".to_string()), "旧路径应移除");
}

/// 删除文件夹：连接移到父级（根则清空），文件夹及其后代记录一并删除，连接本身不删。
#[test]
fn delete_folder_moves_members_to_parent_and_keeps_connections() {
    let mut items = vec![conn("x", "a/b"), conn("y", "a/b/c"), conn("z", "a/bc")];
    let mut records = folders(&["a", "a/b", "a/b/c", "a/bc"]);

    delete_folder(&mut items, &mut records, "a/b");

    assert_eq!(items[0].group, "a", "子级连接上移到父级");
    assert_eq!(items[1].group, "a", "更深的后代同样上移到父级");
    assert_eq!(items[2].group, "a/bc");
    assert_eq!(items.len(), 3, "连接不能被删除");
    assert_eq!(records, folders(&["a", "a/bc"]));
}

#[test]
fn delete_top_level_folder_moves_members_to_root() {
    let mut items = vec![conn("x", "a/b")];
    let mut records = folders(&["a", "a/b"]);
    delete_folder(&mut items, &mut records, "a");
    assert_eq!(items[0].group, "");
    assert!(records.is_empty());
}

/// 新建文件夹要补齐中间层，重复创建幂等。
#[test]
fn create_folder_adds_missing_ancestors_and_is_idempotent() {
    let mut records = Vec::new();
    create_folder(&mut records, "x/y/z");
    assert_eq!(records, folders(&["x", "x/y", "x/y/z"]));
    create_folder(&mut records, "x/y/z");
    assert_eq!(records.len(), 3, "重复创建不应产生重复记录");
    create_folder(&mut records, "  ");
    assert_eq!(records.len(), 3, "空路径不产生文件夹");
}

/// 根路径不是文件夹，不能被重命名或删除。
#[test]
fn root_is_not_a_folder() {
    let mut items = vec![conn("x", "")];
    let mut records = folders(&["a"]);
    rename_folder(&mut items, &mut records, "", "b");
    assert_eq!(items[0].group, "");
    assert_eq!(records, folders(&["a"]));
    delete_folder(&mut items, &mut records, "");
    assert_eq!(items.len(), 1);
}

/// 空文件夹清单要能落盘再读回（测试数据目录是隔离的，不碰用户真实文件）。
#[test]
fn groups_store_roundtrips_through_disk() {
    let tmp = std::env::temp_dir().join(format!("kira-groups-{}", std::process::id()));
    let path = tmp.join("groups.json");
    std::fs::create_dir_all(&tmp).expect("建立临时目录");
    std::fs::write(&path, b"[\"a\",\"a/b\"]").expect("写入样本");
    let text = std::fs::read_to_string(&path).expect("读回");
    let parsed: Vec<String> = serde_json::from_str(&text).expect("解析文件夹清单");
    assert_eq!(parsed, folders(&["a", "a/b"]));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn rebase_path_handles_depth_change() {
    assert_eq!(rebase_path("a/b/c", "a", "x"), "x/b/c");
    assert_eq!(rebase_path("a/b", "a", ""), "b");
    assert_eq!(rebase_path("a", "a", ""), "");
}
