//! 阶段 3 纯逻辑回归测试：转发规则、主机信任库解析、SOCKS5 目标解析。
use super::forward::{parse_socks5_target, resolve_assigned_port, validate_rule, ForwardKind, ForwardRule};
use super::forward_runtime::{filter_for, OwnedRule};
use super::host_keys::{list_trusted, parse_trusted, remove_host, trust_key, upsert, TrustDecision};

fn rule(kind: ForwardKind, bind: u16, target_host: &str, target_port: u16) -> ForwardRule {
    ForwardRule {
        id: "f1".into(),
        kind,
        bind_host: "127.0.0.1".into(),
        bind_port: bind,
        target_host: target_host.into(),
        target_port,
    }
}

#[test]
fn local_and_remote_rules_require_target() {
    assert!(validate_rule(&rule(ForwardKind::Local, 8080, "10.0.0.2", 80)).is_ok());
    assert!(validate_rule(&rule(ForwardKind::Remote, 8080, "10.0.0.2", 80)).is_ok());
    assert!(validate_rule(&rule(ForwardKind::Local, 8080, "", 80)).is_err());
    assert!(validate_rule(&rule(ForwardKind::Remote, 8080, "10.0.0.2", 0)).is_err());
}

#[test]
fn dynamic_rule_needs_no_target_but_still_needs_a_port() {
    let dynamic = rule(ForwardKind::Dynamic, 1080, "", 0);
    assert!(validate_rule(&dynamic).is_ok());
    let mut bad = dynamic.clone();
    bad.bind_port = 0;
    assert!(validate_rule(&bad).is_err());
}

#[test]
fn bind_port_zero_is_rejected_for_all_kinds() {
    for kind in [ForwardKind::Local, ForwardKind::Remote, ForwardKind::Dynamic] {
        assert!(validate_rule(&rule(kind, 0, "h", 1)).is_err());
    }
}

#[test]
fn trust_store_matches_host_and_port_exactly() {
    let text = "example.com:22 SHA256:AAA\nexample.com:2222 SHA256:BBB\nother:22 SHA256:CCC\n";
    assert_eq!(parse_trusted(text, "example.com", 22), Some("SHA256:AAA".into()));
    assert_eq!(parse_trusted(text, "example.com", 2222), Some("SHA256:BBB".into()));
    assert_eq!(parse_trusted(text, "example.com", 80), None);
    assert_eq!(parse_trusted(text, "unknown", 22), None);
}

#[test]
fn comments_and_blank_lines_are_ignored() {
    let text = "# comment\n\n example.com:22 SHA256:AAA \nmalformed-line\n";
    assert_eq!(parse_trusted(text, "example.com", 22), Some("SHA256:AAA".into()));
}

#[test]
fn trust_decision_detects_changed_key() {
    assert_eq!(trust_key("host:22 SHA256:OLD\n", "host", 22, "SHA256:OLD"), TrustDecision::Trusted);
    assert_eq!(trust_key("host:22 SHA256:OLD\n", "host", 22, "SHA256:NEW"), TrustDecision::Changed);
    assert_eq!(trust_key("", "host", 22, "SHA256:NEW"), TrustDecision::Unknown);
}

#[test]
fn trust_key_is_stable_and_quoted() {
    assert_eq!(trust_key("h:22 SHA256:A\n", "h", 22, "SHA256:A"), TrustDecision::Trusted);
    assert_eq!(super::host_keys::host_key("h", 22), "h:22");
    assert_eq!(super::host_keys::host_key("2001:db8::1", 22), "2001:db8::1:22");
}

#[test]
fn socks5_ipv4_domain_and_ipv6_targets_are_parsed() {
    assert_eq!(parse_socks5_target(&[1, 10, 0, 0, 2, 0, 80]), Some(("10.0.0.2".into(), 80)));
    let mut dom = vec![3, 4];
    dom.extend_from_slice(b"test");
    dom.extend_from_slice(&[1, 187]);
    assert_eq!(parse_socks5_target(&dom), Some(("test".into(), 443)));
    let mut v6 = vec![4];
    v6.extend_from_slice(&[0u8; 16]);
    v6.extend_from_slice(&[0, 22]);
    assert_eq!(parse_socks5_target(&v6), Some(("::".into(), 22)));
}

#[test]
fn malformed_socks5_targets_are_rejected() {
    assert_eq!(parse_socks5_target(&[]), None);
    assert_eq!(parse_socks5_target(&[1, 1, 2]), None);
    assert_eq!(parse_socks5_target(&[9, 1, 2, 3, 4, 0, 80]), None);
    assert_eq!(parse_socks5_target(&[3, 5, b'a', b'b', 0, 80]), None);
}

/// 转发列表必须按连接隔离：否则打开 A 的转发面板会看到并误停 B 的转发。
#[test]
fn forward_list_is_scoped_to_one_connection() {
    let owned = vec![
        OwnedRule { connection_id: "conn_a".into(), rule: rule(ForwardKind::Local, 8080, "h", 80) },
        OwnedRule { connection_id: "conn_b".into(), rule: rule(ForwardKind::Local, 8081, "h", 80) },
        OwnedRule { connection_id: "conn_a".into(), rule: rule(ForwardKind::Local, 8082, "h", 80) },
    ];
    let a = filter_for(&owned, "conn_a");
    assert_eq!(a.len(), 2);
    assert_eq!(a[0].bind_port, 8080);
    assert_eq!(a[1].bind_port, 8082, "过滤后应保持原有顺序");
    assert!(filter_for(&owned, "conn_c").is_empty(), "没有转发的连接应返回空");
}

/// 信任库管理：列表要能跳过注释、空行与坏行，不能因为一行脏数据整个列表打不开。
#[test]
fn list_trusted_skips_comments_blank_and_malformed_lines() {
    let text = "# comment\n\nh1:22 SHA256:aaa\nbadline\nh2:2222 SHA256:bbb extra-field\n";
    let list = list_trusted(text);
    assert_eq!(list.len(), 2, "只应解析出两条有效记录");
    assert_eq!(list[0].host, "h1");
    assert_eq!(list[0].port, 22);
    assert_eq!(list[1].host, "h2");
    assert_eq!(list[1].port, 2222);
    assert_eq!(list[1].fingerprint, "SHA256:bbb");
}

/// 删除只影响目标条目，其它主机必须原样保留。
#[test]
fn remove_host_deletes_only_target_entry() {
    let text = "h1:22 SHA256:aaa\nh2:22 SHA256:bbb\nh3:2222 SHA256:ccc\n";
    let out = remove_host(text, "h2", 22);
    assert!(!out.contains("h2:22"), "目标主机应被删除");
    assert!(out.contains("h1:22") && out.contains("h3:2222"), "其它主机不能丢");
}

#[test]
fn remove_host_on_missing_entry_is_noop() {
    let text = "h1:22 SHA256:aaa\n";
    assert_eq!(remove_host(text, "nope", 22), text);
}

/// IPv6 字面量用「最后一个冒号」切分端口，避免被地址里的冒号干扰。
#[test]
fn ipv6_hosts_roundtrip_through_list_and_remove() {
    let text = upsert("", "::1", 22, "SHA256:v6");
    let list = list_trusted(&text);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].host, "::1");
    assert_eq!(list[0].port, 22);
    assert!(remove_host(&text, "::1", 22).trim().is_empty());
}

/// RFC 4254 §7.1：`tcpip-forward` 成功时服务端可以返回 0，表示「沿用请求的端口」。
/// 真机实测 OpenSSH 对固定端口请求就返回 0；把它当实际端口会让回调查不到目标。
#[test]
fn assigned_port_falls_back_to_requested_when_server_returns_zero() {
    assert_eq!(resolve_assigned_port(19025, 0), 19025);
}

#[test]
fn assigned_port_uses_server_choice_when_non_zero() {
    // 请求 0（由服务端分配）时，返回的才是真正端口
    assert_eq!(resolve_assigned_port(0, 43210), 43210);
    assert_eq!(resolve_assigned_port(19025, 19025), 19025);
}
