use super::super::types::{FindRule, ProjectDef, ResolvePattern};
use super::is_foreign_path_entry;

fn def_with(rules: Vec<FindRule>, msix: bool) -> ProjectDef {
    ProjectDef {
        id: "probe".to_string(),
        display_name: "probe".to_string(),
        install_mode: if msix { Some("msix".to_string()) } else { None },
        find_rules: rules,
        ..Default::default()
    }
}

/// 造一个「里面真的有 winget.exe」的目录（清理判定要求 exe 真实存在）
fn dir_with_exe(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("anyver-path-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("winget.exe"), b"").unwrap();
    dir
}

#[test]
fn msix_projects_never_clean_path_entries() {
    // WinGet 的 exe 在 WindowsApps（应用执行别名）：它会被 find_rules 命中，
    // 但当成「外部副本」摘掉会让所有应用执行别名一起失效。
    let dir = dir_with_exe("msix");
    let def = def_with(
        vec![FindRule {
            pattern: ResolvePattern::FixedPath {
                path: dir.to_string_lossy().to_string(),
                exe_name: "winget.exe".to_string(),
            },
            source_label: "Windows 应用执行别名".to_string(),
            priority: 5,
            root_offset: 0,
        }],
        true,
    );
    assert!(
        !is_foreign_path_entry(&dir.to_string_lossy(), &def, "D:\\any-versions\\sdk"),
        "MSIX 型不该清理任何 PATH 条目"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn empty_path_key_never_drives_a_deletion() {
    // `contains("")` 恒为真 —— 等于「PATH 里任何目录只要有这个 exe 就删」。
    // 清理是破坏性操作，宽匹配规则不该触发删除。
    let dir = dir_with_exe("emptykey");
    let def = def_with(
        vec![FindRule {
            pattern: ResolvePattern::PathContains {
                path_key: String::new(),
                exe_name: "winget.exe".to_string(),
            },
            source_label: "系统 PATH".to_string(),
            priority: 90,
            root_offset: 0,
        }],
        false,
    );
    assert!(!is_foreign_path_entry(
        &dir.to_string_lossy(),
        &def,
        "D:\\any-versions\\sdk"
    ));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_real_foreign_install_is_still_cleaned_up() {
    // 常规（archive）项目：明确命中 path_key 且 exe 真的在 → 仍然要清理
    let dir = dir_with_exe("foreign");
    let def = def_with(
        vec![FindRule {
            pattern: ResolvePattern::PathContains {
                path_key: dir.file_name().unwrap().to_string_lossy().to_string(),
                exe_name: "winget.exe".to_string(),
            },
            source_label: "外部安装".to_string(),
            priority: 90,
            root_offset: 0,
        }],
        false,
    );
    assert!(is_foreign_path_entry(
        &dir.to_string_lossy(),
        &def,
        "D:\\any-versions\\sdk"
    ));

    // 自己的 links 目录不算「外部」
    assert!(!is_foreign_path_entry(
        "D:\\any-versions\\sdk\\nodejs",
        &def,
        "D:\\any-versions\\sdk"
    ));
    let _ = std::fs::remove_dir_all(&dir);
}
