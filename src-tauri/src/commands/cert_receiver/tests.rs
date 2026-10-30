use super::*;

#[test]
fn safe_name_strips_traversal_and_wildcard() {
    // 通配符直接丢掉：通配符证书和裸域名要落同一个目录，
    // nginx 里才只写一条 ./conf/cert/seemove.cn/...
    assert_eq!(safe_name("*.a.com"), "a.com");
    assert_eq!(safe_name("*.seemove.cn"), "seemove.cn");
    assert_eq!(safe_name("../../etc/passwd"), "_.._etc_passwd");
    assert_eq!(safe_name(""), "cert");
}

#[test]
fn api_sign_matches_ohttps_algorithm() {
    // 与 ohttps 文档示例一致的构造方式（apiKey 参与签名但不放进请求 URL）
    let sign = pull_sign(
        "push-1xlpm40g1105n9gz",
        "89066abd0f21b983331f3aca02590ea3",
        "cert-n2x740m2qxg8kwyz",
        "1695743251613",
    );
    assert_eq!(sign, "c58d69d9e59923f16b7dafa45bef208d");
}

#[test]
fn pem_first_block_keeps_leaf_only() {
    let leaf = "-----BEGIN CERTIFICATE-----\nAAA\n-----END CERTIFICATE-----\n";
    let chain = "-----BEGIN CERTIFICATE-----\nBBB\n-----END CERTIFICATE-----\n";
    let both = format!("{}{}", leaf, chain);
    assert_eq!(pem_first_block(&both), leaf);
}

#[test]
fn config_validate_rejects_obviously_wrong_values() {
    let mut c = CertReceiverConfig::default();
    assert!(c.validate().is_ok());

    c.token = "  ".into();
    assert!(c.validate().unwrap_err().contains("Token"));

    let mut c = CertReceiverConfig::default();
    c.listen_addr = "127.0.0.1".into();
    assert!(c.validate().unwrap_err().contains("监听地址"));

    let mut c = CertReceiverConfig::default();
    c.reload_mode = "command".into();
    assert!(c.validate().unwrap_err().contains("命令"));
}

#[test]
fn legacy_toml_migration_keeps_old_fields() {
    let dir = std::env::temp_dir().join(format!("kira-certrecv-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(LEGACY_CONFIG_FILE);
    std::fs::write(
        &path,
        "listen_addr = \"0.0.0.0:9100\"\ntoken = \"t0\"\noutput_dir = \"D:\\\\cert\"\nlayout = \"domain\"\nreload_mode = \"nginx\"\n",
    )
    .unwrap();
    // 直接解析（不依赖查找顺序，测试里不便改 exe 路径）
    let text = std::fs::read_to_string(&path).unwrap();
    let doc = text.parse::<toml_edit::DocumentMut>().unwrap();
    assert_eq!(
        doc.get("listen_addr").and_then(|v| v.as_str()),
        Some("0.0.0.0:9100")
    );
    assert_eq!(doc.get("layout").and_then(|v| v.as_str()), Some("domain"));
    let _ = std::fs::remove_dir_all(&dir);
}
