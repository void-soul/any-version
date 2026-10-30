use super::{
    build_family_name_script, parse_msix_family_name, parse_msix_info, parse_msix_version,
};

/// `<版本>|<InstallLocation>` 一次拿全：版本与安装目录都要，BOM / CRLF / 空行都要扛住。
///
/// 安装目录是界面「检测到的路径」的唯一来源（Store 应用没法在 `paths.json` 里声明
/// WindowsApps 路径），所以不能因为分隔符位置不理想就整条丢掉。
#[test]
fn msix_info_parses_version_and_install_location() {
    assert_eq!(
        parse_msix_info(
            "2.19675.0.0|C:\\Program Files\\WindowsApps\\Claude_2.19675.0.0_x64__pzs8sxrjxfjjc\r\n"
        ),
        (
            Some("2.19675.0.0".to_string()),
            Some(
                "C:\\Program Files\\WindowsApps\\Claude_2.19675.0.0_x64__pzs8sxrjxfjjc".to_string()
            )
        )
    );
    assert_eq!(
        parse_msix_info("\u{feff}1.29.380.0|C:\\Program Files\\WindowsApps\\Codex\r\n"),
        (
            Some("1.29.380.0".to_string()),
            Some("C:\\Program Files\\WindowsApps\\Codex".to_string())
        ),
        "BOM 要剥掉"
    );
    // 没装 → 两项都没有（绝不能把空串当成路径显示出去）
    assert_eq!(parse_msix_info("\r\n\r\n"), (None, None));
    assert_eq!(parse_msix_info(""), (None, None));
    // 路径为空（异常输出）→ 只认版本
    assert_eq!(
        parse_msix_info("1.2.3.4|\r\n"),
        (Some("1.2.3.4".to_string()), None)
    );
    // 没有 `|`（脚本被截断）→ 版本仍可用，路径不猜
    assert_eq!(
        parse_msix_info("1.2.3.4"),
        (Some("1.2.3.4".to_string()), None)
    );
    // WindowsApps 路径里本来就有 `|` 之外的字符，用 split_once 只切第一个分隔符
    assert_eq!(
        parse_msix_info("1.0.0.0|D:\\a\\b|c\r\n").1.as_deref(),
        Some("D:\\a\\b|c")
    );
}

/// PowerShell 的输出形态：CRLF、前导空行、可能有 BOM；没装时只有空行。
#[test]
fn msix_version_parses_first_non_empty_line() {
    assert_eq!(
        parse_msix_version("1.29.380.0\r\n").as_deref(),
        Some("1.29.380.0")
    );
    assert_eq!(
        parse_msix_version("\r\n\r\n").as_deref(),
        None,
        "没装时输出为空"
    );
    assert_eq!(parse_msix_version("").as_deref(), None);
    assert_eq!(
        parse_msix_version("\u{feff}1.29.380.0\r\n").as_deref(),
        Some("1.29.380.0"),
        "BOM 要剥掉"
    );
    // 多个包命中时（同名多版本 / 多用户）只取第一行
    assert_eq!(
        parse_msix_version("2.0.0.0\r\n1.29.380.0\r\n").as_deref(),
        Some("2.0.0.0")
    );
}

/// 族名必须是 `<identity>_<hash>` 形态：PowerShell 报错或输出别的东西时不能
/// 让它变成一个假 AUMID（那比「找不到」更难排查）。
#[test]
fn msix_family_name_requires_identity_hash_shape() {
    assert_eq!(
        parse_msix_family_name("OpenAI.Codex_2p2nqsd0c76g0\r\n").as_deref(),
        Some("OpenAI.Codex_2p2nqsd0c76g0")
    );
    assert_eq!(
        parse_msix_family_name("\u{feff}OpenAI.Codex_abc\r\n").as_deref(),
        Some("OpenAI.Codex_abc"),
        "BOM 要剥掉"
    );
    assert_eq!(
        parse_msix_family_name("\r\n\r\n").as_deref(),
        None,
        "没装时输出为空"
    );
    // 报错信息 / 命令回显不是族名
    assert_eq!(
        parse_msix_family_name("Get-AppxPackage : 找不到").as_deref(),
        None
    );
}

/// 一次 PowerShell 调用覆盖全部候选：首个用 `if`，其余 `elseif`（stable 优先）。
#[test]
fn family_name_script_chains_candidates_in_one_call() {
    let script = build_family_name_script(&["OpenAI.Codex", "OpenAI.CodexBeta"]);
    assert!(
        script.starts_with("if($p=Get-AppxPackage -Name 'OpenAI.Codex'"),
        "首个候选必须是 if"
    );
    assert!(script.contains("elseif($p=Get-AppxPackage -Name 'OpenAI.CodexBeta'"));
    assert_eq!(
        script.matches("Get-AppxPackage").count(),
        2,
        "两个候选一次查完"
    );
    // 单引号里的单引号要转义，否则 PowerShell 语法错、整条脚本静默失效
    assert!(build_family_name_script(&["O'Brien.App"]).contains("O''Brien.App"));
}
