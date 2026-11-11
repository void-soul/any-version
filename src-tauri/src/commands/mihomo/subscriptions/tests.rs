use super::*;

const VALID_CLASH_YAML: &str = "\
proxies:
  - name: 节点A
    type: ss
    server: 1.2.3.4
    port: 443
    cipher: aes-128-gcm
    password: test
proxy-groups:
  - name: Proxy
    type: select
    proxies: [节点A]
rules:
  - MATCH,Proxy
";

fn item(ua: Option<&str>) -> ProfileItem {
    let mut p = ProfileItem::default();
    p.id = "sub_test".into();
    p.url = Some("https://example.com/sub".into());
    p.user_agent = ua.map(str::to_string);
    p
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("av_sub_test_{}_{}", label, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ---- 订阅内容结构校验：错误页不能当作配置落盘 ----

#[test]
fn validate_rejects_html_error_page() {
    let err = validate_processed_content("<html><body>403 Forbidden</body></html>").unwrap_err();
    assert!(err.contains("clash") || err.contains("配置"), "{err}");
}

#[test]
fn validate_rejects_json_error_reply() {
    assert!(validate_processed_content("{\"error\":\"token expired\"}").is_err());
}

#[test]
fn validate_accepts_clash_yaml_with_proxies() {
    assert!(validate_processed_content(VALID_CLASH_YAML).is_ok());
}

#[test]
fn validate_accepts_provider_only_yaml() {
    let yaml = "proxy-providers:\n  p:\n    type: http\n    url: https://example.com/p\n    path: ./p.yaml\n";
    assert!(validate_processed_content(yaml).is_ok());
}

// ---- 导入与刷新必须产出同一份内容 ----

#[test]
fn prepare_preserves_clash_yaml_verbatim() {
    assert_eq!(
        prepare_subscription_content(VALID_CLASH_YAML, None).unwrap(),
        VALID_CLASH_YAML
    );
}

#[test]
fn prepare_base64_subscription_is_idempotent() {
    use base64::Engine;
    let plain = "ss://aes-256-gcm:pass@1.2.3.4:443#节点A\n";
    let encoded = base64::engine::general_purpose::STANDARD.encode(plain);
    let first = prepare_subscription_content(&encoded, None).unwrap();
    let second = prepare_subscription_content(&first, None).unwrap();
    assert_eq!(first, second);
    assert!(first.contains("proxies:"));
}

// ---- 四条下载路径统一请求头 ----

#[test]
fn default_ua_applied_when_unset() {
    assert_eq!(effective_user_agent(None), DEFAULT_SUB_UA);
}

#[test]
fn default_ua_applied_when_blank() {
    assert_eq!(effective_user_agent(Some("   ")), DEFAULT_SUB_UA);
}

#[test]
fn explicit_ua_overrides_default() {
    assert_eq!(effective_user_agent(Some("my-client/1.0")), "my-client/1.0");
}

#[test]
fn subscription_headers_carry_default_ua_and_identity_encoding() {
    let headers = subscription_headers(&item(None));
    assert!(headers
        .iter()
        .any(|(k, v)| *k == "User-Agent" && v == DEFAULT_SUB_UA));
    assert!(headers
        .iter()
        .any(|(k, v)| *k == "Accept-Encoding" && v == "identity"));
}

// ---- 刷新失败不得破坏已有配置 ----

#[test]
fn commit_rejects_error_page_and_keeps_existing_config() {
    let dir = temp_dir("commit");
    let it = item(None);
    write_profile_content(&dir, &it, VALID_CLASH_YAML).unwrap();
    let path = profile_content_path(&dir, &it.id);

    let result = commit_subscription_content(&dir, &it, "<html>503 Service Unavailable</html>");

    assert!(result.is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), VALID_CLASH_YAML);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn commit_writes_processed_content_and_backs_up_previous() {
    let dir = temp_dir("backup");
    let it = item(None);
    write_profile_content(&dir, &it, "proxies: []\n").unwrap();
    let path = profile_content_path(&dir, &it.id);
    let backup = path.with_extension("yaml.bak");

    commit_subscription_content(&dir, &it, VALID_CLASH_YAML).unwrap();

    assert_eq!(std::fs::read_to_string(&path).unwrap(), VALID_CLASH_YAML);
    assert_eq!(std::fs::read_to_string(&backup).unwrap(), "proxies: []\n");
    std::fs::remove_dir_all(&dir).unwrap();
}

// ---- 同秒导入多条订阅不能共用同一个 profiles/<id>.yaml ----

#[test]
fn unique_profile_id_appends_suffix_on_collision() {
    let mut a = ProfileItem::default();
    a.id = "sub_1000".into();
    assert_eq!(
        unique_profile_id("sub_1000", std::slice::from_ref(&a)),
        "sub_1000-2"
    );

    let mut b = ProfileItem::default();
    b.id = "sub_1000-2".into();
    assert_eq!(unique_profile_id("sub_1000", &[a, b]), "sub_1000-3");
}

#[test]
fn unique_profile_id_keeps_candidate_when_free() {
    assert_eq!(unique_profile_id("sub_1000", &[]), "sub_1000");
}

#[test]
fn new_profile_ids_are_unique_in_same_millisecond() {
    let a = new_profile_id(&[], "sub");
    let b = new_profile_id(&[], "sub");
    assert_ne!(a, b, "同一毫秒内连续生成两个 id 不应相同");
    assert!(a.starts_with("sub_"));
    assert!(b.starts_with("sub_"));
}

#[test]
fn new_profile_id_never_returns_an_existing_id() {
    let mut existing = ProfileItem::default();
    existing.id = new_profile_id(&[], "file");
    let next = new_profile_id(std::slice::from_ref(&existing), "file");
    assert_ne!(next, existing.id);
}
