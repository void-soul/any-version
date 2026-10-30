use super::*;

#[test]
fn fingerprint_is_stable_and_change_sensitive() {
    let dir = std::env::temp_dir().join(format!("mm_scan_fp_{}", std::process::id()));
    let src = dir.join("src");
    let _ = std::fs::create_dir_all(&src);
    std::fs::write(src.join("a.ts"), "export const a = 1;\n").unwrap();
    let entries = collect_entries(&dir);
    let fp1 = scan_fingerprint(&dir, &entries);
    // 同状态重复计算 → 稳定
    let fp2 = scan_fingerprint(&dir, &collect_entries(&dir));
    assert_eq!(fp1, fp2, "同一状态的指纹应稳定");
    // 修改文件内容（mtime/size 变化）→ 指纹变化
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(src.join("a.ts"), "export const a = 2;\n").unwrap();
    let fp3 = scan_fingerprint(&dir, &collect_entries(&dir));
    assert_ne!(fp1, fp3, "文件内容变更应使指纹变化");
    // 新增文件 → 指纹变化
    std::fs::write(src.join("b.ts"), "export const b = 1;\n").unwrap();
    let fp4 = scan_fingerprint(&dir, &collect_entries(&dir));
    assert_ne!(fp3, fp4, "新增文件应使指纹变化");
    // ignore 规则变更 → 指纹变化（即使条目集不变）
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(dir.join(".gitignore"), "b.ts\n").unwrap();
    let fp5 = scan_fingerprint(&dir, &collect_entries(&dir));
    assert_ne!(fp4, fp5, "ignore 规则变更应使指纹变化");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cache_roundtrip_replays_context() {
    let fp = "deadbeef_cache_roundtrip";
    assert!(read_scan_cache(fp).is_none(), "未写入时不应命中");
    write_scan_cache(fp, "## 目录结构\n（缓存的上下文）");
    let hit = read_scan_cache(fp).expect("写入后应命中");
    assert!(hit.contains("缓存的上下文"));
    // 指纹不匹配的缓存应被拒绝（防串文件/手改）
    let path = scan_cache_dir().join(format!("{}.json", fp));
    let mut v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    v["fingerprint"] = "tampered".into();
    std::fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    assert!(read_scan_cache(fp).is_none(), "指纹不匹配应拒绝");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn scan_with_hint_not_cached_verbatim() {
    // user_hint 不参与指纹：命中缓存的文本 + 运行时拼接 hint
    let s = append_hint("BASE".to_string(), Some(" 聚焦 src "));
    assert!(s.contains("BASE"));
    assert!(s.contains("聚焦 src"));
    assert!(s.contains("用户附加说明"));
    // 空/空白 hint 不应追加任何标记
    assert_eq!(append_hint("BASE".to_string(), Some("   ")), "BASE");
}

#[test]
fn project_name_from_path() {
    assert_eq!(project_name("E:/pro/demo"), "demo");
    assert_eq!(project_name("E:/pro/demo/"), "demo");
}

#[test]
fn skips_known_dirs() {
    assert!(should_skip_dir("node_modules"));
    assert!(should_skip_dir(".git"));
    assert!(!should_skip_dir("src"));
}

#[test]
fn scans_self_repo() {
    // 以本仓库为样本做冒烟测试：应产出目录树、目录规模与技术栈摘要
    let ctx = scan_project(".").unwrap_or_else(|e| panic!("scan failed: {e}"));
    assert!(ctx.contains("项目名称"));
    assert!(ctx.contains("## 目录规模"));
    assert!(ctx.contains("## 目录结构"));
}

#[test]
fn top_stats_counts_files() {
    let entries = vec![
        Entry {
            rel: "src/a.ts".into(),
            is_dir: false,
        },
        Entry {
            rel: "src/b.ts".into(),
            is_dir: false,
        },
        Entry {
            rel: "src/c/d.ts".into(),
            is_dir: false,
        },
        Entry {
            rel: "src".into(),
            is_dir: true,
        },
        Entry {
            rel: "docs/readme.md".into(),
            is_dir: false,
        },
    ];
    let s = top_dir_stats(&entries);
    assert!(s.contains("src/ · 3 files"), "got: {s}");
    assert!(s.contains("docs/ · 1 files"), "got: {s}");
}

fn tmp_project(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mm_scan_test_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn coupling_ts_project() {
    let dir = tmp_project("ts");
    for (rel, content) in [
        (
            "src/main.ts",
            "import { x } from \"./utils/date\";\nimport y from \"react\";\n",
        ),
        ("src/utils/date.ts", "export const x = 1;\n"),
        ("src/pages/home.tsx", "import d from \"../utils/date\";\n"),
        (
            "src/pages/about.tsx",
            "import d from \"../utils/date\";\nimport h from \"./home\";\n",
        ),
    ] {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }
    let entries = collect_entries(&dir);
    let out = analyze_coupling(&dir, &entries).unwrap_or_else(|| panic!("no coupling"));
    let _ = std::fs::remove_dir_all(&dir);
    // 相对导入解析到二级模块键；外部依赖 react 被过滤；同模块 ./home 不计跨模块
    assert!(out.contains("src → src/utils"), "got:\n{out}");
    assert!(out.contains("src/pages → src/utils"), "got:\n{out}");
    assert!(!out.contains("react"), "got:\n{out}");
    assert!(out.contains("跨模块 3 条"), "got:\n{out}");
}

#[test]
fn coupling_rust_crate() {
    let dir = tmp_project("rust");
    for (rel, content) in [
        ("mylib/Cargo.toml", "[package]\nname = \"mylib\"\n"),
        (
            "mylib/src/lib.rs",
            "pub mod commands;\npub mod utils;\nuse crate::commands::network;\n",
        ),
        (
            "mylib/src/commands/mod.rs",
            "pub mod network;\nuse super::utils;\n",
        ),
        (
            "mylib/src/commands/network.rs",
            "use crate::utils::fmt;\nuse std::collections::HashMap;\n",
        ),
        ("mylib/src/utils/mod.rs", "pub fn fmt() {}\n"),
    ] {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }
    let entries = collect_entries(&dir);
    let out = analyze_coupling(&dir, &entries).unwrap_or_else(|| panic!("no coupling"));
    let _ = std::fs::remove_dir_all(&dir);
    // crate::commands 命中 crate 根模块；super:: 解析到父模块；std:: 外部被过滤
    assert!(
        out.contains("mylib/src → mylib/src/commands"),
        "got:\n{out}"
    );
    assert!(
        out.contains("mylib/src/commands → mylib/src/utils"),
        "got:\n{out}"
    );
    assert!(!out.contains("std::"), "got:\n{out}");
}

#[test]
fn extract_js_and_rust_imports() {
    let mut v = Vec::new();
    extract_line_imports("import { a } from './x';", "js", &mut v);
    extract_line_imports("const b = require('../y');", "js", &mut v);
    extract_line_imports("export * from '@lib/z';", "js", &mut v);
    assert_eq!(v, vec!["./x", "../y", "@lib/z"]);
    let mut r = Vec::new();
    extract_line_imports("use crate::commands::network;", "rust", &mut r);
    extract_line_imports("use super::*;", "rust", &mut r);
    extract_line_imports("use std::io;", "rust", &mut r);
    assert_eq!(r, vec!["crate::commands::network", "super::*", "std::io"]);
}

#[test]
fn parse_request_filters_and_normalizes_paths() {
    let dir = tmp_project("reqparse");
    std::fs::create_dir_all(dir.join("src/services")).unwrap();
    std::fs::write(dir.join("src/services/order.ts"), "export class Order {}\n").unwrap();
    let pf = collect_project_files(dir.to_str().unwrap()).unwrap();
    let json = serde_json::json!({
        "paths": ["./src/services/order.ts", "src\\services\\order.ts", "src/services/order.ts", "src/missing.ts", "", "src/services/order.ts"],
        "dirs": ["src/services", "src/missing", "src/services/"]
    });
    let (files, dirs, rejected) = parse_ai_file_request(&json, &pf);
    let _ = std::fs::remove_dir_all(&dir);
    // 去重 + 归一化（./ 前缀与反斜杠等价）+ 不在文件集内的路径被过滤进 rejected
    assert_eq!(files, vec!["src/services/order.ts"], "got: {files:?}");
    assert_eq!(dirs, vec!["src/services"], "got: {dirs:?}");
    assert_eq!(
        rejected,
        vec!["src/missing.ts", "src/missing"],
        "got: {rejected:?}"
    );
}

#[test]
fn read_file_compressed_and_guarded() {
    let dir = tmp_project("readguard");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/a.ts"), "line1\n\n\n\nline2   \n  indented\n").unwrap();
    let (text, truncated) = read_file_compressed(&dir, "src/a.ts", 10_000);
    assert_eq!(
        text.as_deref(),
        Some("line1\n\nline2\n  indented\n"),
        "连续空行应折叠，行尾空白应去除"
    );
    assert!(!truncated);
    // 截断标记
    let (text2, truncated2) = read_file_compressed(&dir, "src/a.ts", 5);
    assert!(truncated2);
    assert_eq!(text2.as_deref().map(|t| t.chars().count()), Some(5));
    // 不可读路径
    let (missing, _) = read_file_compressed(&dir, "src/missing.ts", 1000);
    assert!(missing.is_none());
    // 超大文件不可读
    let big = dir.join("src/big.ts");
    std::fs::write(&big, "x".repeat(300 * 1024)).unwrap();
    let (big_text, _) = read_file_compressed(&dir, "src/big.ts", 1000);
    assert!(big_text.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn format_batch_reports_unreadable_and_dirs() {
    let files = vec![
        ("src/a.ts".to_string(), Some("code".to_string()), false),
        ("src/bin.dat".to_string(), None, false),
    ];
    let out = format_file_batch(&files, &["src/services".to_string()], &[]);
    assert!(out.contains("## 目录确认"));
    assert!(out.contains("src/services"));
    assert!(out.contains("=== src/a.ts ==="));
    assert!(out.contains("code"));
    assert!(out.contains("不可读"));
    let empty = format_file_batch(&[], &[], &[]);
    assert!(empty.contains("没有可读文件"));
}
