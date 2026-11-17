use super::finalshell::{export_finalshell, is_finalshell_cipher, parse_finalshell};
use super::models::RdpConnection;

/// 合成样本（不含任何真实主机/账号/密码）：保留 FinalShell 的字段名与拼写。
const CIPHER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef";

fn sample(conn_type: i64, cipher: &str) -> String {
    format!(
        concat!(
            r#"{{"user_name":"tester","conection_type":{},"password":"{}","#,
            r#""host":"192.0.2.10","id":"a1b2c3d4e5f6g7h8","port":3389,"#,
            r#""name":"Demo RDP","parent_id":"root","secret_key_id":"","#,
            r#""authentication_type":1,"terminal_encoding":"UTF-8","#,
            r#""fullscreen":false,"width":1280,"height":720,"description":"","#,
            r#""create_time":1700000000000,"access_time":1700000000001}}"#
        ),
        conn_type, cipher
    )
}

#[test]
fn cipher_shape_is_32_chars_base64_without_padding() {
    assert!(is_finalshell_cipher(CIPHER));
    assert!(is_finalshell_cipher("abcd+/01abcd+/01abcd+/01abcd+/01"));
    assert!(!is_finalshell_cipher("short"));
    assert!(!is_finalshell_cipher("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef0="));
    assert!(!is_finalshell_cipher(&"A".repeat(33)));
}

#[test]
fn parses_rdp_connection_and_keeps_cipher_verbatim() {
    let text = sample(101, CIPHER);
    let (conn, raw) = parse_finalshell(&text).expect("解析应成功").expect("RDP 类型应被接受");
    assert_eq!(conn.name, "Demo RDP");
    assert_eq!(conn.host, "192.0.2.10");
    assert_eq!(conn.port, 3389);
    assert_eq!(conn.username, "tester");
    assert_eq!(conn.finalshell_cipher.as_deref(), Some(CIPHER));
    assert_eq!(conn.source, "finalshell");
    assert_eq!(raw, text);
    assert!(conn.password.is_none(), "FinalShell 密文不能塞进自有密码字段");
}

#[test]
fn ssh_connections_are_imported_as_ssh_kind() {
    let text = sample(100, CIPHER).replace("\"port\":3389", "\"port\":22");
    let (conn, raw) = parse_finalshell(&text).expect("解析应成功").expect("SSH 类型应被接受");
    assert_eq!(conn.kind, "ssh");
    assert_eq!(conn.port, 22);
    assert_eq!(conn.finalshell_cipher.as_deref(), Some(CIPHER));
    assert_eq!(raw, text);
}

#[test]
fn rdp_connections_keep_rdp_kind() {
    let (conn, _) = parse_finalshell(&sample(101, CIPHER)).unwrap().unwrap();
    assert_eq!(conn.kind, "rdp");
}

#[test]
fn unchanged_ssh_export_is_byte_identical() {
    let text = sample(100, CIPHER).replace("\"port\":3389", "\"port\":22");
    let (conn, _) = parse_finalshell(&text).unwrap().unwrap();
    assert_eq!(export_finalshell(&conn).unwrap(), text);
}

#[test]
fn unknown_connection_type_is_still_skipped() {
    let text = sample(999, CIPHER);
    assert!(parse_finalshell(&text).expect("解析应成功").is_none());
}

#[test]
fn unchanged_export_is_byte_identical() {
    let text = sample(101, CIPHER);
    let (conn, _) = parse_finalshell(&text).unwrap().unwrap();
    assert_eq!(export_finalshell(&conn).unwrap(), text);
}

#[test]
fn edited_export_keeps_cipher_and_all_original_keys() {
    let text = sample(101, CIPHER);
    let (mut conn, _) = parse_finalshell(&text).unwrap().unwrap();
    conn.name = "Renamed".to_string();
    conn.host = "198.51.100.7".to_string();
    conn.port = 3390;
    let out = export_finalshell(&conn).unwrap();
    let value: serde_json::Value = serde_json::from_str(&out).unwrap();
    let original: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["password"], original["password"], "密文必须逐字节不变");
    assert_eq!(value["id"], original["id"]);
    assert_eq!(value["create_time"], original["create_time"]);
    assert_eq!(value["conection_type"], original["conection_type"]);
    for (key, _) in original.as_object().unwrap() {
        assert!(value.get(key).is_some(), "原始键 {key} 不能丢");
    }
    assert_eq!(value["name"], "Renamed");
    assert_eq!(value["host"], "198.51.100.7");
    assert_eq!(value["port"], 3390);
}

#[test]
fn malformed_cipher_is_rejected() {
    let text = sample(101, "not-a-valid-cipher");
    assert!(parse_finalshell(&text).is_err());
    assert!(parse_finalshell("not json").is_err());
}

#[test]
fn local_connection_cannot_be_exported_as_finalshell() {
    let conn = RdpConnection::new("Mine".into(), "127.0.0.1".into(), 3389, "u".into());
    assert!(export_finalshell(&conn).is_err());
}

// ─── 分组维护：重命名 / 删除（批量改连接上的 group 标签） ───



// ─── 驱动器映射（对应 FinalShell 的 drivestoredirect） ───

/// FinalShell 里开了磁盘映射的连接，导入后必须保留，否则用户会觉得「功能丢了」。
#[test]
fn finalshell_drive_redirection_is_imported() {
    let text = sample(101, CIPHER).replace("\"port\":3389,", "\"port\":3389,\"drivestoredirect\":true,");
    let (conn, _) = parse_finalshell(&text).unwrap().unwrap();
    assert!(conn.map_drives, "FinalShell 开了磁盘映射，导入后应保留");
}

#[test]
fn finalshell_missing_drive_flag_defaults_off() {
    let (conn, _) = parse_finalshell(&sample(101, CIPHER)).unwrap().unwrap();
    assert!(!conn.map_drives, "没有该字段时按关闭处理");
}

/// 在我们这边切换开关后，导出必须写回，否则 FinalShell 侧看到的还是旧值。
#[test]
fn export_writes_back_drive_redirection_change() {
    let (mut conn, _) = parse_finalshell(&sample(101, CIPHER)).unwrap().unwrap();
    conn.map_drives = true;
    let out = export_finalshell(&conn).unwrap();
    assert!(out.contains("\"drivestoredirect\":true"), "{out}");
}
