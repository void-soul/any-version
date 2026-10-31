use super::*;

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let mut bytes = [0; 12];
        getrandom::getrandom(&mut bytes).unwrap();
        let path = std::env::temp_dir().join(format!("kira-local-cert-{}", hex(&bytes)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn config(&self) -> HashMap<String, String> {
        HashMap::from([("output_dir".into(), self.0.to_string_lossy().to_string())])
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn body() -> CertificatePush {
    CertificatePush {
        certificate_id: "test-id".into(),
        domain: "second.invalid".into(),
        domains: vec!["first.invalid".into(), "second.invalid".into()],
        cert: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../crates/cert-deployment/tests/fixtures/identity-cert.pem"
        ))
        .into(),
        key: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../crates/cert-deployment/tests/fixtures/identity-key.pem"
        ))
        .into(),
        ca: String::new(),
        expired_time: String::new(),
    }
}

#[test]
fn validates_local_config_without_creating_output() {
    assert!(LocalDeployment::from_config(&HashMap::new()).is_err());
    assert!(LocalDeployment::from_config(&HashMap::from([(
        "output_dir".into(),
        "relative".into()
    )]))
    .is_err());
    let root = Root::new();
    for (key, value) in [
        ("layout", "bad"),
        ("reload_mode", "bad"),
        ("reload_mode", "command"),
        ("reload_mode", "service"),
    ] {
        let mut config = root.config();
        config.insert(key.into(), value.into());
        assert!(LocalDeployment::from_config(&config).is_err());
    }
    let config = LocalDeployment::from_config(&root.config()).unwrap();
    assert_eq!(config.layout, "cert");
    assert_eq!(config.reload_mode, "none");
    assert_eq!(std::fs::read_dir(&root.0).unwrap().count(), 0);
}

#[test]
fn all_layouts_share_the_receiver_engine_and_respect_target() {
    for layout in ["cert", "domain", "both", "flat"] {
        let root = Root::new();
        let mut config = root.config();
        config.insert("layout".into(), layout.into());
        let config = LocalDeployment::from_config(&config).unwrap();
        let body = body();
        config.deploy(&body).unwrap();
        let key = match layout {
            "domain" => root.0.join("second.invalid/cert.key"),
            "flat" => root.0.join("second.invalid.key"),
            _ => root.0.join("test-id/cert.key"),
        };
        assert_eq!(std::fs::read_to_string(key).unwrap(), body.key);
        if layout == "both" {
            assert!(root.0.join("second.invalid/fullchain.cer").is_file());
        }
    }
}

#[test]
fn mismatched_identity_never_changes_live_files() {
    let root = Root::new();
    let config = LocalDeployment::from_config(&root.config()).unwrap();
    let original = body();
    config.deploy(&original).unwrap();
    let mut wrong = body();
    wrong.key = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../crates/cert-deployment/tests/fixtures/mismatch-key.pem"
    ))
    .into();
    assert!(config.deploy(&wrong).is_err());
    assert_eq!(
        std::fs::read_to_string(root.0.join("test-id/cert.key")).unwrap(),
        original.key
    );
}

#[test]
fn failed_post_action_recovers_previous_files() {
    let root = Root::new();
    let original = body();
    LocalDeployment::from_config(&root.config())
        .unwrap()
        .deploy(&original)
        .unwrap();
    let mut config = root.config();
    config.insert("reload_mode".into(), "command".into());
    config.insert("reload_command".into(), "exit 9".into());
    let mut changed = body();
    changed.cert.push('\n');
    let error = LocalDeployment::from_config(&config)
        .unwrap()
        .deploy(&changed)
        .unwrap_err();
    assert!(error.contains("重载失败"));
    assert_eq!(
        std::fs::read_to_string(root.0.join("test-id/fullchain.cer")).unwrap(),
        original.cert
    );
}

#[test]
fn directory_probe_never_runs_reload_or_writes_certificates() {
    let root = Root::new();
    let mut config = root.config();
    config.insert("reload_mode".into(), "command".into());
    config.insert("reload_command".into(), "exit 9".into());
    assert!(LocalDeployment::from_config(&config)
        .unwrap()
        .probe()
        .is_ok());
    let entries: Vec<_> = std::fs::read_dir(&root.0)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(
        entries,
        vec![std::ffi::OsString::from(".receiver-deployment.lock")]
    );
}

#[test]
fn local_and_network_receiver_share_the_same_output_lock() {
    let root = Root::new();
    let lock = cert_deployment::deployment::OutputLock::try_acquire(&root.0).unwrap();
    assert!(LocalDeployment::from_config(&root.config())
        .unwrap()
        .deploy(&body())
        .is_err());
    assert!(!root.0.join("test-id").exists());
    drop(lock);
    LocalDeployment::from_config(&root.config())
        .unwrap()
        .deploy(&body())
        .unwrap();
}

#[tokio::test]
async fn local_node_dispatch_works_without_url_token_or_http() {
    let root = Root::new();
    let node: DeployNode = serde_json::from_value(
        serde_json::json!({"id":"local-test","type":"local","name":"local","config":root.config()}),
    )
    .unwrap();
    let cert: Certificate = serde_json::from_value(serde_json::json!({"id":"test-id","domains":["first.invalid","second.invalid"],"email":"test@example.invalid","dns_provider":"","not_after":"2099-01-01T00:00:00Z"})).unwrap();
    let body = body();
    super::super::deploy_ssh::deploy_to_node(
        &node,
        &cert,
        &(body.cert, body.key, body.ca),
        "second.invalid",
    )
    .await
    .unwrap();
    assert!(root.0.join("test-id/fullchain.cer").is_file());
    let meta: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.0.join("test-id/meta.json")).unwrap(),
    ).unwrap();
    assert_eq!(meta["expired_time"], "2099-01-01T00:00:00Z");
}
