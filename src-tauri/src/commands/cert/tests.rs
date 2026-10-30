
use super::*;

/// 构造最小 DER TLV（测试数据用，兼容长格式长度）
fn der_tlv(tag: u8, value: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let len = value.len();
    if len < 0x80 {
        out.push(len as u8);
    } else {
        let mut bytes = Vec::new();
        let mut l = len;
        while l > 0 {
            bytes.insert(0, (l & 0xff) as u8);
            l >>= 8;
        }
        out.push(0x80 | bytes.len() as u8);
        out.extend(bytes);
    }
    out.extend(value);
    out
}

/// 构造结构完整的 X.509 证书 DER（解析器只走 TLV 结构，签名内容可为假）。
/// `with_version` = false 模拟 v1 证书（TBSCertificate 无 [0] version 字段）。
fn test_cert_der(nb: &[u8], na: &[u8], with_version: bool) -> Vec<u8> {
    let mut validity_inner = Vec::new();
    validity_inner.extend(der_tlv(0x17, nb));
    validity_inner.extend(der_tlv(0x17, na));
    let validity = der_tlv(0x30, &validity_inner);
    let serial = der_tlv(0x02, &[0, 1]);
    let sig_alg = der_tlv(0x30, &der_tlv(0x06, &[43, 14, 3, 2, 26]));
    let issuer = der_tlv(0x30, &der_tlv(0x06, &[0x55, 0x04, 0x03, 3]));
    let subject = der_tlv(0x30, &der_tlv(0x06, &[0x55, 0x04, 0x03, 3]));
    let mut tbs_inner = Vec::new();
    if with_version {
        tbs_inner.extend(der_tlv(0xA0, &der_tlv(0x02, &[2]))); // v3
    }
    tbs_inner.extend(&serial);
    tbs_inner.extend(&sig_alg);
    tbs_inner.extend(&issuer);
    tbs_inner.extend(&validity);
    tbs_inner.extend(&subject);
    let tbs = der_tlv(0x30, &tbs_inner);
    // Certificate ::= SEQUENCE { tbsCertificate, signatureAlgorithm, signatureValue }
    let mut cert_inner = Vec::new();
    cert_inner.extend(&tbs);
    cert_inner.extend(&sig_alg);
    cert_inner.extend(der_tlv(0x03, &[0])); // BIT STRING（空签名）
    der_tlv(0x30, &cert_inner)
}

fn der_to_pem(der: &[u8]) -> String {
    let b64 = B64.encode(der);
    let mut pem = String::from("-----BEGIN CERTIFICATE-----\n");
    for chunk in b64.as_bytes().chunks(64) {
        pem.push_str(std::str::from_utf8(chunk).unwrap());
        pem.push('\n');
    }
    pem.push_str("-----END CERTIFICATE-----\n");
    pem
}

/// 回归：extract_validity 必须从真实 X.509 结构（外层 Certificate → tbsCertificate
/// → validity）解析出有效期。曾有一版直接读外层子项（tbs/sigAlg/sigValue），
/// 导致有效期永远解析为 None → 调度器因 not_after 缺失永不触发自动续期。
#[test]
fn extract_validity_parses_real_cert_structure() {
    let pem = der_to_pem(&test_cert_der(b"250801120000Z", b"260801120000Z", true));
    let (nb, na) = extract_validity(&pem);
    assert_eq!(nb.as_deref(), Some("2025-08-01T12:00:00+00:00"));
    assert_eq!(na.as_deref(), Some("2026-08-01T12:00:00+00:00"));
}

/// v1 证书（无 version 字段）同样可解析
#[test]
fn extract_validity_without_version_field() {
    let pem = der_to_pem(&test_cert_der(b"240101000000Z", b"241001000000Z", false));
    let (nb, na) = extract_validity(&pem);
    assert_eq!(nb.as_deref(), Some("2024-01-01T00:00:00+00:00"));
    assert_eq!(na.as_deref(), Some("2024-10-01T00:00:00+00:00"));
}

/// GeneralizedTime（14 位，2050 年后证书常见）
#[test]
fn extract_validity_generalized_time() {
    let pem = der_to_pem(&test_cert_der(b"250101000000Z", b"20801231235959Z", true));
    let (_, na) = extract_validity(&pem);
    assert_eq!(na.as_deref(), Some("2080-12-31T23:59:59+00:00"));
}

#[test]
fn parse_der_time_utc_and_generalized() {
    let d = parse_der_time(b"250801120000Z").unwrap();
    assert_eq!(d.to_rfc3339(), "2025-08-01T12:00:00+00:00");
    let d = parse_der_time(b"20491231235959Z").unwrap();
    assert_eq!(d.to_rfc3339(), "2049-12-31T23:59:59+00:00");
}

/// Windows 接收端 URL 归一化：部署（/push）与连通测试（/health）
/// 必须从同一基础地址派生；兼容用户填写的 /push 后缀与结尾斜杠。
#[test]
fn windows_base_url_normalization() {
    assert_eq!(windows_base_url("http://h:9000"), "http://h:9000");
    assert_eq!(windows_base_url("http://h:9000/"), "http://h:9000");
    assert_eq!(windows_base_url("http://h:9000/push"), "http://h:9000");
    assert_eq!(windows_base_url("http://h:9000/push/"), "http://h:9000");
    assert_eq!(windows_base_url("http://h:9000/PUSH"), "http://h:9000");
    assert_eq!(windows_base_url("  http://h:9000/push  "), "http://h:9000");
    assert_eq!(windows_base_url(""), "");
    // 结尾的 /push 段一律剥离（含反向代理子路径场景）：
    // 部署与连通测试从同一 base 派生，保证两者指向同一个接收端
    assert_eq!(
        windows_base_url("http://h:9000/api/push"),
        "http://h:9000/api"
    );
}

/// 阿里云 RPC 签名：参数按字典序、RFC3986 编码、POST + `/` 参与签名。
/// 期望值由独立实现计算，防止百分号编码或排序被改坏。
#[test]
fn aliyun_rpc_signature_matches_reference() {
    let mut p = BTreeMap::new();
    p.insert("AccessKeyId".to_string(), "ak".to_string());
    p.insert("Action".to_string(), "UploadUserCertificate".to_string());
    p.insert(
        "Cert".to_string(),
        "-----BEGIN CERTIFICATE-----\nAAA\n-----END CERTIFICATE-----".to_string(),
    );
    p.insert("Format".to_string(), "JSON".to_string());
    p.insert(
        "Key".to_string(),
        "-----BEGIN PRIVATE KEY-----\nBBB\n-----END PRIVATE KEY-----".to_string(),
    );
    p.insert("Name".to_string(), "a.com-2026".to_string());
    p.insert("SignatureMethod".to_string(), "HMAC-SHA1".to_string());
    p.insert("SignatureVersion".to_string(), "1.0".to_string());
    p.insert("SignatureNonce".to_string(), "1234".to_string());
    p.insert("Timestamp".to_string(), "2026-01-01T00:00:00Z".to_string());
    p.insert("Version".to_string(), "2020-04-07".to_string());
    assert_eq!(aliyun_rpc_sign(&p, "sk"), "1Td495sRTleqWu7Jt3Zd65Pvwek=");
}

#[test]
fn remote_name_and_cert_name_are_path_safe() {
    assert_eq!(remote_safe_name("*.a.com"), "_.a.com");
    assert_eq!(remote_safe_name("../../x"), ".._.._x");
    let cert = Certificate {
        id: "cert-1".into(),
        domains: vec!["a.com".into()],
        email: String::new(),
        ca: default_ca(),
        dns_provider: "alidns".into(),
        credential_id: String::new(),
        validation: default_validation(),
        http_port: 80,
        renew_before_days: 30,
        not_before: None,
        not_after: None,
        last_issue_at: None,
        status: default_status(),
        last_error: None,
        deploy_links: Vec::new(),
        deploy_node_ids: Vec::new(),
    };
    let name = cert_name_for(&cert, "", "fallback");
    assert!(name.starts_with("a.com-"));
    assert!(name.len() <= 63);
}

fn unique_tmp(prefix: &str) -> PathBuf {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let p = std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), n));
    let _ = std::fs::create_dir_all(&p);
    p
}

/// lego 的 `--path` 即数据根目录：证书落在 `<path>/certificates/`。
/// 必须能从此处读到 PEM，否则签发成功也会报「未找到生成的 PEM 文件」。
#[test]
fn lego_pems_read_from_data_root_certificates_dir() {
    let base = unique_tmp("anyver_lego_root");
    let store = base.join("certificates");
    std::fs::create_dir_all(&store).unwrap();
    std::fs::write(store.join("_.a.com.crt"), "LEAF").unwrap();
    std::fs::write(store.join("_.a.com.key"), "KEY").unwrap();
    std::fs::write(store.join("_.a.com.issuer.crt"), "ISSUER").unwrap();

    let got = read_lego_pems(&base, Some("*.a.com")).expect("应能从 <path>/certificates 读到 PEM");
    assert_eq!(got.crt, "LEAF");
    assert_eq!(got.key, "KEY");
    assert_eq!(got.issuer, "ISSUER");
    assert_eq!(got.crt_file, "_.a.com.crt");

    let _ = std::fs::remove_dir_all(&base);
}

/// 同一数据目录存在多份证书时，按域名精确取名，
/// 不得因 `read_dir` 顺序随机取到别的域名的证书。
#[test]
fn lego_pems_prefer_domain_over_other_certificates() {
    let base = unique_tmp("anyver_lego_prefer");
    let store = base.join("certificates");
    std::fs::create_dir_all(&store).unwrap();
    std::fs::write(store.join("a.com.crt"), "CRT_A").unwrap();
    std::fs::write(store.join("a.com.key"), "KEY_A").unwrap();
    std::fs::write(store.join("b.com.crt"), "CRT_B").unwrap();
    std::fs::write(store.join("b.com.key"), "KEY_B").unwrap();

    let got = read_lego_pems(&base, Some("b.com")).unwrap();
    assert_eq!(got.crt, "CRT_B");
    assert_eq!(got.key, "KEY_B");

    let _ = std::fs::remove_dir_all(&base);
}

/// 兼容历史（错误的）`<path>/.lego/certificates` 布局，避免旧数据读不到。
#[test]
fn lego_pems_legacy_dot_lego_layout_still_readable() {
    let base = unique_tmp("anyver_lego_legacy");
    let store = base.join(".lego").join("certificates");
    std::fs::create_dir_all(&store).unwrap();
    std::fs::write(store.join("x.com.crt"), "L").unwrap();
    std::fs::write(store.join("x.com.key"), "K").unwrap();

    let got = read_lego_pems(&base, None).expect("旧布局应仍可读取");
    assert_eq!(got.crt, "L");
    assert_eq!(got.key, "K");

    let _ = std::fs::remove_dir_all(&base);
}

/// 「跳过续期」必须能被认出来：lego 未到续期时间时仍然以退出码 0 结束（v5.3.1 实测），
/// 只能靠证书文件有没有变来判断。认不出来的话，后端就会把「什么都没做」记成一次成功签发——
/// 界面上的表现正是「点了申请没反应」。
#[test]
fn lego_skip_detected_by_unchanged_cert_files() {
    let base = unique_tmp("anyver_lego_skip");
    let store = base.join("certificates");
    std::fs::create_dir_all(&store).unwrap();
    std::fs::write(store.join("a.com.crt"), "LEAF").unwrap();
    std::fs::write(store.join("a.com.key"), "KEY").unwrap();

    // 已有证书 → 之后按「续期」语义调用 lego
    assert!(has_existing_cert(&base));

    // 被跳过：文件一个都不变
    let before = cert_store_fingerprint(&base);
    assert_eq!(before, cert_store_fingerprint(&base));

    // 真正重签：cert 文件变化
    std::fs::write(store.join("a.com.crt"), "NEW-LEAF").unwrap();
    assert_ne!(before, cert_store_fingerprint(&base));

    let _ = std::fs::remove_dir_all(&base);
}

/// 申请日志要真的落盘：实时面板关掉之后，用户还能从这里回看。
#[test]
fn issue_log_writes_lines_to_file() {
    let base = unique_tmp("anyver_issue_log");
    let path = base.join("logs").join("cert-1.log");
    let mut log = IssueLog::open(None, "cert-1", path.clone());
    log.line("first line");
    log.line("第二行");
    let text = std::fs::read_to_string(&path).expect("日志应已落盘");
    assert!(text.contains("first line"));
    assert!(text.contains("第二行"));
    let _ = std::fs::remove_dir_all(&base);
}

/// 网络类失败要翻译成人话，否则用户只看到 `no such host` 一脸懵。
#[test]
fn network_hint_explains_dns_and_timeout() {
    let dns = lego_network_hint(
        "",
        "alicloud: API call failed: dial tcp: lookup alidns.cn-hangzhou.aliyuncs.com: no such host",
    )
    .expect("应识别 DNS 解析失败");
    assert!(dns.contains("223.5.5.5"));

    let timeout = lego_network_hint(
        "",
        "context deadline exceeded (Client.Timeout exceeded while awaiting headers)",
    )
    .expect("应识别超时");
    assert!(timeout.contains("超时"));

    assert!(lego_network_hint("", "INFO everything is fine").is_none());
}

/// 部署失败要落到三处：日志、节点状态、记录 —— 这样界面和节点卡片都能看到原因。
#[tokio::test]
async fn deploy_links_marks_failure_and_updates_node() {
    let base = unique_tmp("anyver_deploy");
    let log_path = base.join("log.txt");
    let mut log = IssueLog::open(None, "cert-1", log_path.clone());

    let cert = Certificate {
        id: "cert-1".into(),
        domains: vec!["a.com".into()],
        email: String::new(),
        ca: default_ca(),
        dns_provider: "alidns".into(),
        credential_id: String::new(),
        validation: default_validation(),
        http_port: 80,
        renew_before_days: 30,
        not_before: None,
        not_after: None,
        last_issue_at: None,
        status: default_status(),
        last_error: None,
        deploy_links: Vec::new(),
        deploy_node_ids: Vec::new(),
    };
    // 故意少填 url：deploy_http_push 会立刻报「缺少 url」，不碰网络
    let mut nodes = vec![DeployNode {
        id: "n1".into(),
        node_type: "windows".into(),
        name: "没填地址的节点".into(),
        config: HashMap::new(),
        deploy_before_days: 7,
        last_deploy_at: None,
        last_deploy_error: None,
        note: String::new(),
    }];
    let pems = ("CRT".to_string(), "KEY".to_string(), "ISS".to_string());
    let links = vec![DeployLink {
        node_id: "n1".into(),
        target: String::new(),
    }];

    let (summary, records) =
        deploy_links(&mut log, &cert, &pems, &mut nodes, &links, "manual").await;

    assert_eq!(summary.ok_count, 0);
    assert_eq!(summary.fail_count, 1);
    assert_eq!(records.len(), 1);
    assert!(!records[0].ok);
    assert_eq!(records[0].trigger, "manual");
    assert!(
        records[0].message.contains("缺少 url"),
        "实际：{}",
        records[0].message
    );
    assert!(nodes[0].last_deploy_error.is_some(), "失败原因要写回节点");

    let text = std::fs::read_to_string(&log_path).unwrap_or_default();
    assert!(text.contains("部署失败"), "日志里应能看到失败原因");

    let _ = std::fs::remove_dir_all(&base);
}

/// 部署记录只留最近 N 条，且保留的是**新的**那一端。
#[test]
fn deploy_history_is_capped_from_the_front() {
    let mk = |i: usize| DeployRecord {
        at: format!("t{}", i),
        cert_id: "c".into(),
        node_id: "n".into(),
        node_name: "n".into(),
        target: String::new(),
        ok: true,
        message: String::new(),
        trigger: "manual".into(),
    };
    let all: Vec<DeployRecord> = (0..DEPLOY_HISTORY_CAP + 2).map(mk).collect();
    let kept = trim_deploy_history(all);
    assert_eq!(kept.len(), DEPLOY_HISTORY_CAP);
    assert_eq!(kept[0].at, "t2", "应丢掉最旧的记录");
    assert_eq!(
        kept[kept.len() - 1].at,
        format!("t{}", DEPLOY_HISTORY_CAP + 1)
    );
}

/// 旧数据只有 deploy_node_ids 时，也要能推出「要部署到哪些节点」。
#[test]
fn cert_links_falls_back_to_legacy_node_ids() {
    let mut cert = Certificate {
        id: "cert-1".into(),
        domains: vec!["a.com".into()],
        email: String::new(),
        ca: default_ca(),
        dns_provider: "alidns".into(),
        credential_id: String::new(),
        validation: default_validation(),
        http_port: 80,
        renew_before_days: 30,
        not_before: None,
        not_after: None,
        last_issue_at: None,
        status: default_status(),
        last_error: None,
        deploy_links: Vec::new(),
        deploy_node_ids: vec!["n1".into(), "n2".into()],
    };
    let links = cert_links(&cert);
    assert_eq!(links.len(), 2);
    assert_eq!(links[0].node_id, "n1");
    assert!(links[0].target.is_empty());

    // 有 deploy_links 时以它为准（target 保留）
    cert.deploy_links = vec![DeployLink {
        node_id: "n9".into(),
        target: "cdn.example.com".into(),
    }];
    let links = cert_links(&cert);
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].target, "cdn.example.com");
}

/// 空的证书目录不算「已有证书」，否则首次申请会被当成续期。
#[test]
fn empty_cert_dir_has_no_existing_cert() {
    let base = unique_tmp("anyver_lego_empty");
    assert!(!has_existing_cert(&base));
    assert!(cert_store_fingerprint(&base).is_empty());
    let _ = std::fs::remove_dir_all(&base);
}

/// 查看证书：有效期与序列号都能从 PEM 解析出来
#[test]
fn parse_cert_detail_extracts_validity_and_serial() {
    let pem = der_to_pem(&test_cert_der(b"250801120000Z", b"260801120000Z", true));
    let d = parse_cert_detail(&pem);
    assert_eq!(d.not_before.as_deref(), Some("2025-08-01T12:00:00+00:00"));
    assert_eq!(d.not_after.as_deref(), Some("2026-08-01T12:00:00+00:00"));
    assert_eq!(d.serial.as_deref(), Some("0001"));
}

/// cert.cer 只取第一段证书（叶子），不含链
#[test]
fn pem_first_block_keeps_only_leaf() {
    let leaf = der_to_pem(&test_cert_der(b"250801120000Z", b"260801120000Z", true));
    let chain = der_to_pem(&test_cert_der(b"250801120000Z", b"260801120000Z", false));
    let both = format!("{}{}", leaf, chain);

    let first = pem_first_block(&both);
    assert_eq!(first.matches("-----BEGIN CERTIFICATE-----").count(), 1);
    assert_eq!(first, leaf);
}

/// 新版 lego 的 .crt 已含签发者证书 → fullchain 直接用它，不重复拼接
#[test]
fn build_fullchain_skips_duplicate_issuer() {
    let leaf = der_to_pem(&test_cert_der(b"250801120000Z", b"260801120000Z", true));
    let chain = der_to_pem(&test_cert_der(b"250801120000Z", b"260801120000Z", false));
    let crt = format!("{}{}", leaf, chain);
    let full = build_fullchain(&crt, &chain);
    assert_eq!(full, crt);
    assert_eq!(full.matches("-----BEGIN CERTIFICATE-----").count(), 2);
}

/// 老版 lego 的 .crt 只有叶子 → fullchain 追加 issuer，得到叶子 + 中间
#[test]
fn build_fullchain_appends_missing_issuer() {
    let leaf = der_to_pem(&test_cert_der(b"250801120000Z", b"260801120000Z", true));
    let chain = der_to_pem(&test_cert_der(b"250801120000Z", b"260801120000Z", false));
    let full = build_fullchain(&leaf, &chain);
    assert!(full.starts_with(leaf.trim_end()));
    assert!(full.contains(chain.trim_end()));
    assert_eq!(full.matches("-----BEGIN CERTIFICATE-----").count(), 2);
}

/// 导出 zip 的条目可被重新读出（验证打包未损坏）
#[test]
fn build_zip_roundtrip() {
    use std::io::Read;
    let files = vec![
        ("a.com/cert.key".to_string(), "KEY\n".to_string()),
        ("a.com/cert.cer".to_string(), "CERT\n".to_string()),
    ];
    let data = build_zip(&files).unwrap();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(data)).unwrap();
    assert_eq!(archive.len(), 2);
    let mut s = String::new();
    archive
        .by_name("a.com/cert.key")
        .unwrap()
        .read_to_string(&mut s)
        .unwrap();
    assert_eq!(s, "KEY\n");
}

/// 监控目标输入兼容：裸域名、带 scheme、带端口、IPv6 方括号
#[test]
fn split_host_port_parses_common_forms() {
    assert_eq!(
        split_host_port("example.com", 443),
        ("example.com".into(), 443)
    );
    assert_eq!(
        split_host_port("https://example.com/", 443),
        ("example.com".into(), 443)
    );
    assert_eq!(
        split_host_port("example.com:8443", 443),
        ("example.com".into(), 8443)
    );
    assert_eq!(
        split_host_port("  http://a.com:80  ", 443),
        ("a.com".into(), 80)
    );
    assert_eq!(split_host_port("[::1]:8443", 443), ("::1".into(), 8443));
    assert_eq!(split_host_port("[::1]", 443), ("::1".into(), 443));
}

/// 到期提醒按阈值分桶：同一档只提醒一次，续期后自动回到 None
#[test]
fn notify_bucket_crosses_each_threshold_once() {
    let t = [1, 3, 7, 14, 30];
    assert_eq!(notify_bucket(45, &t), None);
    assert_eq!(notify_bucket(30, &t), Some(30));
    assert_eq!(notify_bucket(20, &t), Some(30));
    assert_eq!(notify_bucket(14, &t), Some(14));
    assert_eq!(notify_bucket(2, &t), Some(3));
    assert_eq!(notify_bucket(0, &t), Some(0));
    assert_eq!(notify_bucket(-9, &t), Some(0));
    assert_eq!(notify_bucket(10, &[]), None);
}

/// 各云厂商签名共用：请求头必须按小写名称排序，值去掉首尾空白
#[test]
fn canonical_headers_are_lowercased_and_sorted() {
    let h = vec![
        ("Host".to_string(), " a.com ".to_string()),
        ("Content-Type".to_string(), "application/json".to_string()),
    ];
    let (canonical, signed) = sorted_canonical_headers(&h);
    assert_eq!(signed, "content-type;host");
    assert_eq!(canonical, "content-type:application/json\nhost:a.com\n");
}
