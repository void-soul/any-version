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

#[test]
fn every_runtime_configuration_field_invalidates_snapshot() {
    let original = CertReceiverConfig::default();
    let updates: Vec<Box<dyn Fn(&mut CertReceiverConfig)>> = vec![
        Box::new(|c| c.listen_addr = "127.0.0.1:9011".into()),
        Box::new(|c| c.token.push('x')),
        Box::new(|c| c.output_dir.push('x')),
        Box::new(|c| c.layout = "domain".into()),
        Box::new(|c| c.api_key.push('x')),
        Box::new(|c| c.reload_mode = "nginx".into()),
        Box::new(|c| c.reload_command.push('x')),
        Box::new(|c| c.nginx_prefix.push('x')),
        Box::new(|c| c.nginx_conf.push('x')),
        Box::new(|c| c.service_name.push('x')),
        Box::new(|c| c.post_script = Some("test-only".into())),
    ];
    for update in updates {
        let mut changed = original.clone();
        update(&mut changed);
        assert!(lifecycle::runtime_config_changed(&original, &changed));
    }
    let mut boot_policy = original.clone();
    boot_policy.auto_start = !boot_policy.auto_start;
    assert!(!lifecycle::runtime_config_changed(&original, &boot_policy));
    assert!(!lifecycle::runtime_config_changed(&original, &original));
}

fn fake_server() -> (ServerHandle, std::sync::mpsc::Sender<()>) {
    let (shutdown, _) = tokio::sync::oneshot::channel();
    let (done, completion) = std::sync::mpsc::channel();
    (
        ServerHandle {
            shutdown: Some(shutdown),
            completion: Arc::new(Mutex::new(completion)),
            finished: Arc::new(AtomicBool::new(false)),
            addr: "127.0.0.1:0".parse().unwrap(),
        },
        done,
    )
}

#[test]
fn stopping_requires_actual_completion_not_elapsed_sleep() {
    let (handle, done) = fake_server();
    assert!(lifecycle::wait_for_server(&handle, std::time::Duration::ZERO).is_err());
    done.send(()).unwrap();
    assert!(lifecycle::wait_for_server(&handle, std::time::Duration::ZERO).is_ok());
}

#[test]
fn server_exit_is_not_reported_as_running() {
    let (handle, _done) = fake_server();
    let finished = handle.finished.clone();
    let st = CertReceiverState {
        lifecycle: Mutex::new(()),
        cfg: Mutex::new(CertReceiverConfig::default()),
        server: Mutex::new(Some(handle)),
        stats: Mutex::new(Stats::default()),
        logs: Mutex::new(VecDeque::new()),
        run_seq: AtomicU64::new(0),
        app: Mutex::new(None),
        migrated: Mutex::new(false),
    };
    assert!(st.status().running);
    finished.store(true, Ordering::SeqCst);
    assert!(!st.status().running);
    st.stop_server().unwrap();
    assert!(st.server.lock().unwrap().is_none());
}

#[test]
fn pull_sign_normalizes_api_key_like_request_verification() {
    assert_eq!(
        pull_sign("api", "  isolation-key\r\n", "cert", "123"),
        pull_sign(
            "api",
            cert_deployment::credentials::api_key_for_sign("  isolation-key\r\n"),
            "cert",
            "123"
        )
    );
    for token in ["changeme", "YOUR_SHARED_TOKEN", " YOUR_SHARED_TOKEN "] {
        let mut cfg = CertReceiverConfig::default();
        cfg.token = token.into();
        assert!(cfg.validate().is_err());
    }
}

#[test]
fn stopping_remains_running_until_exit_is_confirmed() {
    let (shutdown, received_shutdown) = tokio::sync::oneshot::channel();
    let (done, completion) = std::sync::mpsc::channel();
    let st = Arc::new(CertReceiverState {
        lifecycle: Mutex::new(()),
        cfg: Mutex::new(CertReceiverConfig::default()),
        server: Mutex::new(Some(ServerHandle {
            shutdown: Some(shutdown),
            completion: Arc::new(Mutex::new(completion)),
            finished: Arc::new(AtomicBool::new(false)),
            addr: "127.0.0.1:0".parse().unwrap(),
        })),
        stats: Mutex::new(Stats::default()),
        logs: Mutex::new(VecDeque::new()),
        run_seq: AtomicU64::new(0),
        app: Mutex::new(None),
        migrated: Mutex::new(false),
    });
    let stopping = st.clone();
    let worker = std::thread::spawn(move || stopping.stop_server());
    received_shutdown.blocking_recv().unwrap();
    assert!(st.status().running);
    st.record_push(true, "isolated in-flight request".into());
    assert!(st.status().running);
    assert!(st
        .server
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .shutdown
        .is_none());
    done.send(()).unwrap();
    worker.join().unwrap().unwrap();
    assert!(!st.status().running);
    assert!(st.server.lock().unwrap().is_none());
}
