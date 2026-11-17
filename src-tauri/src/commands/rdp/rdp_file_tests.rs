use super::models::RdpConnection;
use super::rdp_file::build_rdp_file;

fn conn() -> RdpConnection {
    let mut c = RdpConnection::new("n".into(), "10.0.0.1".into(), 3389, "admin".into());
    c.id = "id1".into();
    c
}

#[test]
fn base_lines_always_present() {
    let text = build_rdp_file(&conn(), None);
    assert!(text.contains("full address:s:10.0.0.1:3389\r\n"), "{text}");
    assert!(text.contains("username:s:admin\r\n"), "{text}");
    assert!(text.contains("prompt for credentials:i:1\r\n"), "没有密码行时应要求系统凭据");
}

/// 对齐 FinalShell：驱动器与剪贴板默认都开。
#[test]
fn drives_and_clipboard_are_enabled_by_default() {
    let text = build_rdp_file(&conn(), None);
    assert!(text.contains("drivestoredirect:s:*\r\n"), "默认应映射全部驱动器: {text}");
    assert!(text.contains("redirectclipboard:i:1\r\n"), "默认应共享剪贴板: {text}");
}

#[test]
fn switches_turn_off_explicitly() {
    let mut c = conn();
    c.map_drives = false;
    c.clipboard = false;
    let text = build_rdp_file(&c, None);
    assert!(!text.contains("drivestoredirect"), "{text}");
    // 关闭剪贴板要显式写 0，否则会继承客户端默认（可能仍是开）
    assert!(text.contains("redirectclipboard:i:0\r\n"), "{text}");
}

#[test]
fn password_line_replaces_the_credential_prompt() {
    let line = "password 51:b:00ff\r\n".to_string();
    let text = build_rdp_file(&conn(), Some(&line));
    assert!(text.contains("password 51:b:00ff\r\n"), "{text}");
    assert!(!text.contains("prompt for credentials"), "有密码时不能再弹凭据框");
}

#[test]
fn empty_password_line_falls_back_to_prompt() {
    let text = build_rdp_file(&conn(), Some(""));
    assert!(text.contains("prompt for credentials:i:1\r\n"));
    assert!(!text.contains("password 51:"), "空密码行不应留下半条指令");
}

#[test]
fn domain_is_written_only_when_set() {
    let mut c = conn();
    assert!(!build_rdp_file(&c, None).contains("domain:s:"));
    c.domain = "CORP".into();
    assert!(build_rdp_file(&c, None).contains("domain:s:CORP\r\n"));
}
