use super::*;
use crate::commands::project::types::EnvVarDef;
/// MSIX 型（如 WinGet）不写 PATH，也就没有「托管写入的条目」可展示。
/// 不短路的话：`bin_dirs` 里的空串会生成 `links_dir\<id>` 这个**永远不存在**的候选目录，
/// 而完全托管的项目是全量展示候选（含「目录缺失」告警）—— 界面上就会挂出一条假的环境变量。
#[test]
fn msix_projects_have_no_managed_path_entries() {
    let def = crate::commands::project::types::ProjectDef {
        id: "probe".to_string(),
        display_name: "probe".to_string(),
        install_mode: Some("msix".to_string()),
        bin_dirs: Some(vec![String::new()]),
        ..Default::default()
    };
    assert!(
        managed_path_entries_for_sdk("probe", "D:\\any-versions\\sdk\\probe", &def).is_empty(),
        "MSIX 型不该有托管 PATH 条目"
    );
}

/// 对照：普通项目（bin_dirs 含空串 = 托管目录本身）仍要给出候选条目。
#[test]
fn regular_projects_still_report_their_bin_entries() {
    let def = crate::commands::project::types::ProjectDef {
        id: "probe".to_string(),
        display_name: "probe".to_string(),
        bin_dirs: Some(vec![String::new()]),
        ..Default::default()
    };
    let entries = managed_path_entries_for_sdk("probe", "D:\\any-versions\\sdk\\probe", &def);
    assert_eq!(entries.len(), 1, "bin_dirs 的空串应产出托管目录这一条");
    assert_eq!(entries[0].kind, "sdk_bin");
}

fn var(name: &str, sub_dir: Option<&str>) -> EnvVarDef {
    EnvVarDef {
        name: name.to_string(),
        desc: String::new(),
        check_type: "path".to_string(),
        tier: None,
        sub_dir: sub_dir.map(|s| s.to_string()),
    }
}

fn temp_cache_root(tag: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("kira-env-{}-{}", tag, nanos));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn test_sub_dir_bin_paths_only_returns_existing_dirs() {
    let root = temp_cache_root("subdir");
    std::fs::create_dir_all(root.join("cargo").join("bin")).unwrap();

    let vars = vec![
        var("CARGO_HOME", Some("cargo")),      // 存在 → 产出
        var("RUSTUP_HOME", Some("rustup")),    // 无 bin 目录 → 不产出
        var("ANDROID_HOME", None),             // 非 sub_dir → 不产出
        var("CARGO_HOME_TWIN", Some("cargo")), // 重复 → 去重
    ];
    let got = sub_dir_bin_paths(&vars, &root);
    assert_eq!(
        got,
        vec![root.join("cargo").join("bin").to_string_lossy().to_string()]
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn test_path_list_contains_ignores_case_and_trailing_separator() {
    let entries = vec![
        r"C:\Windows".to_string(),
        r"D:\any-versions\caches\cargo\bin\".to_string(),
    ];
    // 大小写不同、结尾是否带反斜杠都视为同一条（与 add/remove_from_user_path 的判定一致）
    assert!(path_list_contains(
        &entries,
        r"d:\any-versions\caches\cargo\bin"
    ));
    assert!(path_list_contains(
        &entries,
        r"D:\any-versions\caches\cargo\bin\"
    ));
    assert!(!path_list_contains(
        &entries,
        r"D:\any-versions\sdk\rust\bin"
    ));
}

#[test]
fn test_sub_dir_bin_paths_empty_when_nothing_matches() {
    let root = temp_cache_root("subdir-empty");
    std::fs::create_dir_all(&root).unwrap();
    let vars = vec![var("RUSTUP_HOME", Some("rustup"))];
    assert!(sub_dir_bin_paths(&vars, &root).is_empty());
    let _ = std::fs::remove_dir_all(&root);
}
