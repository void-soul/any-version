//! SSH 会话层的纯函数回归测试（不触网、不读真实凭据）。
use super::target::{classify_ssh_error, normalize_target, terminal_size, SshErrorKind, SshTarget};

/// React 开发模式的双挂载会在同一毫秒内连两次；毫秒时间戳撞名会让
/// 第一次卸载的 disconnect 把第二次的会话一起断掉。
#[test]
fn session_ids_do_not_collide_within_same_millisecond() {
    let ids: std::collections::HashSet<String> = (0..1000)
        .map(|_| super::commands::next_session_id("ssh"))
        .collect();
    assert_eq!(ids.len(), 1000, "同一毫秒内的会话 id 也必须唯一");
}

fn base() -> SshTarget {
    SshTarget {
        host: "192.0.2.10".into(),
        port: 22,
        username: "tester".into(),
        password: "synth".into(),
        key_auth: None,
    }
}

#[test]
fn normal_target_is_accepted() {
    let t = normalize_target(base()).expect("合法参数应通过");
    assert_eq!(t.host, "192.0.2.10");
    assert_eq!(t.port, 22);
    assert_eq!(t.username, "tester");
}

#[test]
fn host_is_trimmed_and_brackets_are_tolerated() {
    let mut t = base();
    t.host = "  [192.0.2.10]  ".into();
    assert_eq!(normalize_target(t).unwrap().host, "192.0.2.10");
}

#[test]
fn empty_host_username_or_port_zero_is_rejected() {
    let mut t = base();
    t.host = "   ".into();
    assert!(normalize_target(t).is_err(), "空主机必须拒绝");
    let mut t = base();
    t.username = "".into();
    assert!(normalize_target(t).is_err(), "空用户名必须拒绝");
    let mut t = base();
    t.port = 0;
    assert!(normalize_target(t).is_err(), "端口 0 必须拒绝");
}

#[test]
fn terminal_size_is_clamped_to_safe_bounds() {
    assert_eq!(terminal_size(0, 0), (80, 24), "退化值回落到默认");
    assert_eq!(terminal_size(120, 40), (120, 40), "正常尺寸不改动");
    assert_eq!(terminal_size(5000, 5000), (500, 200), "超大值必须夹住");
}

#[test]
fn known_ssh_failures_are_classified_without_leaking_secrets() {
    let cases: &[(&str, SshErrorKind)] = &[
        ("Authentication failed (password)", SshErrorKind::Auth),
        ("Permission denied (publickey,password)", SshErrorKind::Auth),
        ("Connection refused (os error 10061)", SshErrorKind::Refused),
        ("connection timed out", SshErrorKind::Timeout),
        ("No such host is known", SshErrorKind::Dns),
        ("host key verification failed", SshErrorKind::HostKey),
        ("some other wire error", SshErrorKind::Other),
    ];
    for (raw, want) in cases {
        let classified = classify_ssh_error(raw);
        assert_eq!(classified.kind, *want, "raw={raw}");
        assert!(!classified.message.is_empty());
        assert!(
            !classified.message.contains("synth"),
            "错误信息不得回显凭据"
        );
    }
}

#[test]
fn password_is_never_printed_by_debug() {
    let t = base();
    let shown = format!("{t:?}");
    assert!(!shown.contains("synth"), "Debug 不得泄露密码: {shown}");
}
