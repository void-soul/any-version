use super::collect_msix_files;
use super::filter_deps_for_arch;
use super::parse_channel_manifest_version;

/// 与真实 channel-rust-stable.toml 结构一致的样例（含提交信息后缀与嵌套 target 段）
const SAMPLE: &str = "manifest-version = \"2\"\ndate = \"2026-09-03\"\n\n\
        [pkg.cargo]\nversion = \"0.99.0 (797e8a9bc 2026-08-05)\"\n\n\
        [pkg.rust]\nversion = \"1.98.1 (48a229cea 2026-09-01)\"\n\
        git_commit_hash = \"48a229ceaefd4985c50990b14116b6d856af0985\"\n\n\
        [pkg.rust.target.x86_64-pc-windows-msvc]\navailable = true\n";

#[test]
fn test_parse_channel_manifest_version_with_commit_suffix() {
    assert_eq!(
        parse_channel_manifest_version(SAMPLE).as_deref(),
        Some("1.98.1")
    );
}

#[test]
fn test_parse_channel_manifest_version_plain_value() {
    let text = "[pkg.rust]\nversion = \"1.91.0\"\n";
    assert_eq!(
        parse_channel_manifest_version(text).as_deref(),
        Some("1.91.0")
    );
}

#[test]
fn test_parse_channel_manifest_version_missing_section() {
    assert_eq!(
        parse_channel_manifest_version("[pkg.cargo]\nversion = \"0.99.0\"\n"),
        None
    );
}

/// 依赖包归档里混着各架构的 .appx/.msix，只挑包文件，且要递归进子目录。
#[test]
fn test_collect_msix_files_recurses_and_filters() {
    let root = std::env::temp_dir().join(format!("msix-collect-{}", std::process::id()));
    let nested = root.join("x64").join("release");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(root.join("keep.txt"), b"x").unwrap();
    for name in ["dep.appx", "dep.msix", "dep.msixbundle"] {
        std::fs::write(root.join(name), b"x").unwrap();
    }
    std::fs::write(nested.join("deep.appxbundle"), b"x").unwrap();

    let mut found = Vec::new();
    collect_msix_files(&root, &mut found);
    let mut names: Vec<String> = found
        .iter()
        .map(|p| {
            std::path::Path::new(p)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string()
        })
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec!["deep.appxbundle", "dep.appx", "dep.msix", "dep.msixbundle"]
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// 依赖归档里三套架构混在一起，只留下与本机匹配的那套（x64 机器筛掉 arm64 / x86）。
#[test]
fn test_filter_deps_for_arch_keeps_matching_arch_only() {
    let deps = vec![
        "C:/t/Microsoft.VCLibs.140.00.UWPDesktop_14.0.30704.0_x64__8wekyb3d8bbwe.appx".to_string(),
        "C:/t/Microsoft.VCLibs.140.00.UWPDesktop_14.0.30704.0_x86__8wekyb3d8bbwe.appx".to_string(),
        "C:/t/Microsoft.VCLibs.140.00.UWPDesktop_14.0.30704.0_arm64__8wekyb3d8bbwe.appx"
            .to_string(),
        "C:/t/Microsoft.UI.Xaml.2.8_8.2310.30001.0_x64__8wekyb3d8bbwe.appx".to_string(),
    ];
    let kept = filter_deps_for_arch(deps, "x64");
    assert_eq!(kept.len(), 2, "只应保留两个 x64 包: {:?}", kept);
    assert!(kept.iter().all(|p| p.contains("_x64__")));
    assert!(!kept
        .iter()
        .any(|p| p.contains("_arm64__") || p.contains("_x86__")));
}

/// WinGet 依赖归档的真实布局（实测 v1.29.380）：**架构在目录名里、文件名只有
/// `_<arch>.appx`**，没有 Store 那套 `_x64__8wekyb3d8bbwe` 后缀。
/// 上一版按 `_x64__` 精确匹配 → 0 命中 → 兜底全量返回 → ARM64 被装 → 0x80073D10。
#[test]
fn test_filter_deps_matches_winget_dependency_zip_layout() {
    let deps = vec![
            "D:/any-versions/.tmp/winget_1/deps/robe/arm64/Microsoft.VCLibs.140.00.UWPDesktop_14.0.33728.0_arm64.appx".to_string(),
            "D:/any-versions/.tmp/winget_1/deps/robe/arm64/Microsoft.WindowsAppRuntime.1.8_8000.616.304.0_arm64.appx".to_string(),
            "D:/any-versions/.tmp/winget_1/deps/robe/x64/Microsoft.VCLibs.140.00.UWPDesktop_14.0.33728.0_x64.appx".to_string(),
            "D:/any-versions/.tmp/winget_1/deps/robe/x64/Microsoft.WindowsAppRuntime.1.8_8000.616.304.0_x64.appx".to_string(),
            "D:/any-versions/.tmp/winget_1/deps/robe/x86/Microsoft.VCLibs.140.00.UWPDesktop_14.0.33728.0_x86.appx".to_string(),
            "D:/any-versions/.tmp/winget_1/deps/robe/x86/Microsoft.WindowsAppRuntime.1.8_8000.616.304.0_x86.appx".to_string(),
        ];
    let kept = filter_deps_for_arch(deps, "x64");
    assert_eq!(kept.len(), 2, "x64 机器只该留下 x64 那两个: {:?}", kept);
    assert!(kept.iter().all(|p| p.contains("/x64/")));

    // x86 机器换 x86 那套；arm64 机器换 arm64 那套（且不能因为文件名里含 "arm" 被误判）
    let kept_x86 = filter_deps_for_arch(kept.clone(), "x86");
    assert!(kept_x86.is_empty(), "x64 包不能留给 x86 机器");
    let arm_deps = filter_deps_for_arch(deps_of_all_archs(), "arm64");
    assert_eq!(
        arm_deps.len(),
        2,
        "arm64 机器该留下 arm64 那两个: {:?}",
        arm_deps
    );
    assert!(arm_deps.iter().all(|p| p.contains("/arm64/")));
}

/// 架构无关的包（名字/目录里都没有架构段）必须保留 —— 排除规则只针对异架构。
#[test]
fn test_filter_deps_keeps_arch_neutral_packages() {
    let deps = vec![
        "C:/t/Microsoft.UI.Xaml.2.8_8.2310.30001.0_neutral__8wekyb3d8bbwe.appx".to_string(),
        "C:/t/deps/Microsoft.WindowsAppRuntime.1.8.appx".to_string(),
    ];
    assert_eq!(filter_deps_for_arch(deps, "x64").len(), 2);
}

fn deps_of_all_archs() -> Vec<String> {
    vec![
        "C:/t/deps/robe/arm64/Microsoft.VCLibs_14.0_arm64.appx".to_string(),
        "C:/t/deps/robe/arm64/Microsoft.WindowsAppRuntime.1.8_arm64.appx".to_string(),
        "C:/t/deps/robe/x64/Microsoft.VCLibs_14.0_x64.appx".to_string(),
        "C:/t/deps/robe/x64/Microsoft.WindowsAppRuntime.1.8_x64.appx".to_string(),
        "C:/t/deps/robe/x86/Microsoft.VCLibs_14.0_x86.appx".to_string(),
        "C:/t/deps/robe/x86/Microsoft.WindowsAppRuntime.1.8_x86.appx".to_string(),
    ]
}

#[test]
fn test_collect_msix_files_on_missing_dir_is_noop() {
    let mut found = Vec::new();
    collect_msix_files(std::path::Path::new("C:/definitely/not/here"), &mut found);
    assert!(found.is_empty());
}

/// 从 release 的 assets 里挑出唯一匹配的那一个（真实数据取自 Ghidra 12.1.4）。
#[test]
fn pick_asset_finds_the_one_zip_among_other_platform_archives() {
    let assets = vec![
        serde_json::json!({ "name": "ghidra_12.1.4_PUBLIC_20260921.zip",
                "browser_download_url": "https://example.com/ghidra_12.1.4_PUBLIC_20260921.zip" }),
        serde_json::json!({ "name": "ghidra_12.1.4_PUBLIC_20260921.tar.gz",
                "browser_download_url": "https://example.com/x.tar.gz" }),
        serde_json::json!({ "name": "ghidra_12.1.4_PUBLIC_20260921.7z",
                "browser_download_url": "https://example.com/x.7z" }),
    ];
    let (name, url) =
        super::pick_asset(&assets, "ghidra_12.1.4_PUBLIC_", ".zip").expect("应能挑出唯一的 zip");
    assert_eq!(name, "ghidra_12.1.4_PUBLIC_20260921.zip");
    assert!(url.ends_with("ghidra_12.1.4_PUBLIC_20260921.zip"));
}

/// **挑不出来必须报错，不能默默取第一个** —— 猜错比留空更糟：
/// 用户会下载到一个不对的东西，且完全看不出哪里错了。
#[test]
fn pick_asset_errors_when_nothing_matches() {
    let assets = vec![serde_json::json!({ "name": "ghidra_12.1.4_PUBLIC_20260921.tar.gz" })];
    let err =
        super::pick_asset(&assets, "ghidra_12.1.4_PUBLIC_", ".zip").expect_err("没有 zip 应报错");
    assert!(err.contains("没有匹配"), "实际: {}", err);
}

/// 多个匹配说明筛选条件不够窄，应把候选列出来而不是随便挑一个。
#[test]
fn pick_asset_errors_and_lists_candidates_when_ambiguous() {
    let assets = vec![
        serde_json::json!({ "name": "ghidra_12.1.4_PUBLIC_20260921.zip",
                "browser_download_url": "https://example.com/a.zip" }),
        serde_json::json!({ "name": "ghidra_12.1.4_PUBLIC_20260922.zip",
                "browser_download_url": "https://example.com/b.zip" }),
    ];
    let err =
        super::pick_asset(&assets, "ghidra_12.1.4_PUBLIC_", ".zip").expect_err("两个 zip 应报错");
    assert!(
        err.contains("20260921.zip") && err.contains("20260922.zip"),
        "应列出全部候选，实际: {}",
        err
    );
}

/// tag 与资产名的清洗规则（Ghidra 专用形态）。
#[test]
fn ghidra_tag_and_asset_prefix_are_derived_from_the_bare_version() {
    let v = "12.1.4";
    // 版本列表侧：tag_name `Ghidra_12.1.4_build` -> `12.1.4`
    let shown = super::apply_transform(
        "Ghidra_12.1.4_build",
        "trim_prefix:Ghidra_;trim_suffix:_build",
    );
    assert_eq!(shown, v);
    // asset 侧：tag 与资产名前缀都由同一个清洗后的版本号拼出
    assert_eq!(format!("Ghidra_{}_build", v), "Ghidra_12.1.4_build");
    assert_eq!(format!("ghidra_{}_PUBLIC_", v), "ghidra_12.1.4_PUBLIC_");
}
