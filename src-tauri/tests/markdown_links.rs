//! 验证 Markdown 阅读器的同目录发现与相对链接解析。
//! 这两处逻辑涉及路径拼接、扩展名回退与 URL 转义，容易出边界问题，故用真实文件系统验证。

use std::fs;
use std::path::PathBuf;

use tauri_app_lib::commands::file_io::{list_sibling_markdown, resolve_markdown_link};

/// 建一个临时目录树，返回根路径。
fn setup(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("av_md_test_{}_{}", tag, std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("guide")).unwrap();
    fs::create_dir_all(root.join("node_modules")).unwrap();
    fs::create_dir_all(root.join("deep/a/b/c")).unwrap();

    fs::write(root.join("index.md"), "# Index").unwrap();
    fs::write(root.join("other.markdown"), "# Other").unwrap();
    fs::write(root.join("notes.txt"), "not markdown").unwrap();
    fs::write(root.join("with space.md"), "# Spaced").unwrap();
    fs::write(root.join("guide/README.md"), "# Guide").unwrap();
    fs::write(root.join("guide/intro.md"), "# Intro").unwrap();
    // 应被忽略的目录
    fs::write(root.join("node_modules/pkg.md"), "# Should be ignored").unwrap();
    // 超出默认深度 3 的文件
    fs::write(root.join("deep/a/b/c/too-deep.md"), "# Too deep").unwrap();
    root
}

#[test]
fn lists_markdown_and_skips_noise() {
    let root = setup("list");
    let from = root.join("index.md").to_string_lossy().to_string();
    let list = list_sibling_markdown(from, None).unwrap();
    let rels: Vec<String> = list.iter().map(|e| e.rel.clone()).collect();

    // 各种 markdown 扩展名都要收录
    assert!(rels.contains(&"index.md".to_string()), "rels={:?}", rels);
    assert!(rels.contains(&"other.markdown".to_string()), "rels={:?}", rels);
    assert!(rels.contains(&"with space.md".to_string()), "rels={:?}", rels);
    // 子目录用 / 分隔
    assert!(rels.contains(&"guide/intro.md".to_string()), "rels={:?}", rels);
    // 非 markdown 不收
    assert!(!rels.iter().any(|r| r.ends_with(".txt")), "rels={:?}", rels);
    // node_modules 被忽略
    assert!(!rels.iter().any(|r| r.contains("node_modules")), "rels={:?}", rels);

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn respects_max_depth() {
    let root = setup("depth");
    let from = root.join("index.md").to_string_lossy().to_string();

    // 默认深度 3 够不到 deep/a/b/c/too-deep.md（需要下降 4 层）
    let shallow = list_sibling_markdown(from.clone(), None).unwrap();
    assert!(
        !shallow.iter().any(|e| e.rel.contains("too-deep")),
        "默认深度不应命中 too-deep: {:?}",
        shallow.iter().map(|e| &e.rel).collect::<Vec<_>>()
    );

    // 放宽深度后应能找到
    let deep = list_sibling_markdown(from, Some(9)).unwrap();
    assert!(
        deep.iter().any(|e| e.rel.contains("too-deep")),
        "放宽深度后应命中 too-deep"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn accepts_directory_as_input() {
    let root = setup("dir");
    // 直接传目录也应可用
    let list = list_sibling_markdown(root.to_string_lossy().to_string(), None).unwrap();
    assert!(list.iter().any(|e| e.rel == "index.md"));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn resolves_plain_relative_link() {
    let root = setup("plain");
    let from = root.join("index.md").to_string_lossy().to_string();
    let got = resolve_markdown_link(from, "guide/intro.md".into()).unwrap();
    assert!(got.is_some(), "应解析出 guide/intro.md");
    assert!(got.unwrap().ends_with("intro.md"));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn strips_anchor_and_query() {
    let root = setup("anchor");
    let from = root.join("index.md").to_string_lossy().to_string();
    let got = resolve_markdown_link(from, "guide/intro.md#section-1".into()).unwrap();
    assert!(got.is_some(), "带锚点的链接应能解析");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn falls_back_to_md_extension() {
    let root = setup("ext");
    let from = root.join("index.md").to_string_lossy().to_string();
    // 链接写的是 "other"，实际文件是 other.markdown -> 补 .md 失败，应返回 None
    assert!(resolve_markdown_link(from.clone(), "nonexistent".into()).unwrap().is_none());
    // 链接写 "guide/intro"（无扩展名），补 .md 后应命中
    let got = resolve_markdown_link(from, "guide/intro".into()).unwrap();
    assert!(got.is_some(), "无扩展名链接应回退补 .md");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn falls_back_to_readme_in_directory() {
    let root = setup("readme");
    let from = root.join("index.md").to_string_lossy().to_string();
    // 链接指向目录，应回退到该目录下的 README.md
    let got = resolve_markdown_link(from, "guide".into()).unwrap();
    assert!(got.is_some(), "指向目录的链接应回退 README.md");
    assert!(got.unwrap().to_lowercase().ends_with("readme.md"));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn decodes_percent_escapes() {
    let root = setup("escape");
    let from = root.join("index.md").to_string_lossy().to_string();
    // markdown 里空格常被写成 %20
    let got = resolve_markdown_link(from, "with%20space.md".into()).unwrap();
    assert!(got.is_some(), "%20 应被解码为空格并命中 with space.md");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn resolves_parent_relative_link() {
    let root = setup("parent");
    let from = root.join("guide/intro.md").to_string_lossy().to_string();
    // 从子目录用 ../ 回到上级
    let got = resolve_markdown_link(from, "../index.md".into()).unwrap();
    assert!(got.is_some(), "../ 相对链接应能解析");
    assert!(got.unwrap().ends_with("index.md"));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn returns_none_for_missing_target() {
    let root = setup("missing");
    let from = root.join("index.md").to_string_lossy().to_string();
    assert!(resolve_markdown_link(from.clone(), "nope/absent.md".into()).unwrap().is_none());
    // 空链接不应 panic
    assert!(resolve_markdown_link(from, "#only-anchor".into()).unwrap().is_none());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn resolved_path_has_no_unc_prefix() {
    let root = setup("unc");
    let from = root.join("index.md").to_string_lossy().to_string();
    let got = resolve_markdown_link(from, "guide/intro.md".into()).unwrap().unwrap();
    // canonicalize 在 Windows 会带 \\?\ 前缀，必须已被剥掉，否则前端路径比较会失配
    assert!(!got.starts_with(r"\\?\"), "解析结果不应带 UNC 前缀: {}", got);
    let _ = fs::remove_dir_all(&root);
}
