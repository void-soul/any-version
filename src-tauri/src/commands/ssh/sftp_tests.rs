//! SFTP 路径/错误分类的纯函数回归测试（不触网、不读真实凭据）。
use super::sftp_path::{classify_sftp_error, normalize_remote_path, safe_local_name, SftpErrorKind};

#[test]
fn absolute_and_relative_paths_are_normalized() {
    assert_eq!(normalize_remote_path("/home/u", "a.txt").unwrap(), "/home/u/a.txt");
    assert_eq!(normalize_remote_path("/home/u", "/etc/hosts").unwrap(), "/etc/hosts");
    assert_eq!(normalize_remote_path("/home/u", "").unwrap(), "/home/u");
    assert_eq!(normalize_remote_path("/home/u", ".").unwrap(), "/home/u");
}

#[test]
fn dot_dot_never_escapes_root() {
    assert_eq!(normalize_remote_path("/a/b", "..").unwrap(), "/a");
    assert_eq!(normalize_remote_path("/a/b", "../../..").unwrap(), "/");
    assert_eq!(normalize_remote_path("/", "..").unwrap(), "/");
}

#[test]
fn redundant_separators_and_tilde_are_handled() {
    assert_eq!(normalize_remote_path("/a", "//b///c").unwrap(), "/b/c");
    assert_eq!(normalize_remote_path("/a", "~/x").unwrap(), "~/x", "~ 交给远端展开，不本地臆造家目录");
}

#[test]
fn nul_and_control_characters_are_rejected() {
    assert!(normalize_remote_path("/a", "x\0y").is_err());
    assert!(normalize_remote_path("/a", "x\ny").is_err());
}

#[test]
fn local_names_cannot_escape_download_directory() {
    assert_eq!(safe_local_name("report.txt"), "report.txt");
    assert_eq!(safe_local_name("../../etc/passwd"), "passwd");
    assert_eq!(safe_local_name("a/b\\c"), "c", "路径段全部丢弃，只留文件名");
    assert_eq!(safe_local_name("a:b?c"), "a_b_c", "Windows 非法字符替换");
    assert_eq!(safe_local_name(".."), "download");
    assert_eq!(safe_local_name(""), "download");
}

#[test]
fn sftp_status_codes_are_classified_without_leaking_secrets() {
    let cases: &[(&str, SftpErrorKind)] = &[
        ("No such file", SftpErrorKind::NotFound),
        ("Permission denied", SftpErrorKind::Permission),
        ("Failure", SftpErrorKind::Other),
        ("Timeout", SftpErrorKind::Timeout),
        ("Connection closed", SftpErrorKind::Disconnected),
    ];
    for (raw, want) in cases {
        let view = classify_sftp_error(raw);
        assert_eq!(view.kind, *want, "raw={raw}");
        assert!(!view.message.is_empty());
        assert!(!view.message.contains("synth-secret"), "不得回显凭据");
    }
}
