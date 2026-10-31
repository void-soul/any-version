use super::*;
use serde_json::{json, Value};

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let mut nonce = [0; 12];
        getrandom::getrandom(&mut nonce).unwrap();
        let root = std::env::temp_dir().join(format!("cert-config-cas-{}", hex(&nonce)));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }
    fn table(&self) -> PathBuf {
        self.0.join("table.json")
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn certificate(id: &str) -> Certificate {
    serde_json::from_value(json!({"id":id,"domains":["old.invalid"],"email":"mail@test.invalid","dns_provider":"","deploy_links":[]})).unwrap()
}
fn node(id: &str) -> DeployNode {
    serde_json::from_value(
        json!({"id":id,"type":"local","name":"node","config":{"output_dir":"old"}}),
    )
    .unwrap()
}

#[test]
fn concurrent_configuration_mutations_do_not_lose_other_rows() {
    let root = Root::new();
    let path = root.table();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    std::thread::scope(|scope| {
        for id in 0..8 {
            let path = path.clone();
            let barrier = barrier.clone();
            scope.spawn(move || {
                barrier.wait();
                mutate_json_at(&path, |rows: &mut Vec<Value>| {
                    std::thread::sleep(std::time::Duration::from_millis(2));
                    rows.push(json!({"id":id}));
                    Ok(())
                })
                .unwrap();
            });
        }
    });
    let rows = read_json_at::<Value>(&path).unwrap();
    assert_eq!(rows.len(), 8);
    assert_eq!(
        rows.iter()
            .map(|row| row["id"].as_u64().unwrap())
            .collect::<std::collections::HashSet<_>>()
            .len(),
        8
    );
}

#[test]
fn late_certificate_state_does_not_replace_user_edits_or_new_records() {
    let root = Root::new();
    let path = root.table();
    let expected = certificate("first");
    let mut updated = expected.clone();
    updated.not_after = Some("2099-01-01T00:00:00Z".into());
    updated.status = "issued".into();
    write_json_at(&path, std::slice::from_ref(&expected)).unwrap();
    mutate_json_at(&path, |rows: &mut Vec<Certificate>| {
        rows[0].domains = vec!["edited.invalid".into()];
        rows.push(certificate("new"));
        Ok(())
    })
    .unwrap();
    mutate_json_at(&path, |rows| {
        patch_runtime(
            rows,
            std::slice::from_ref(&expected),
            std::slice::from_ref(&updated),
            CERT_RUNTIME,
        )
    })
    .unwrap();
    let current = read_json_at::<Certificate>(&path).unwrap();
    assert_eq!(current.len(), 2);
    assert_eq!(current[0].domains, ["edited.invalid"]);
    assert!(current[0].not_after.is_none());
}

#[test]
fn deleted_certificate_or_node_is_never_recreated_by_a_late_task() {
    let root = Root::new();
    let path = root.table();
    let expected = node("node");
    let mut updated = expected.clone();
    updated.last_deploy_at = Some("now".into());
    write_json_at(&path, &Vec::<DeployNode>::new()).unwrap();
    mutate_json_at(&path, |rows| {
        patch_runtime(
            rows,
            std::slice::from_ref(&expected),
            std::slice::from_ref(&updated),
            NODE_RUNTIME,
        )
    })
    .unwrap();
    assert!(read_json_at::<DeployNode>(&path).unwrap().is_empty());
    let expected = certificate("cert");
    let mut updated = expected.clone();
    updated.status = "issued".into();
    write_json_at(&path, &Vec::<Certificate>::new()).unwrap();
    mutate_json_at(&path, |rows| {
        patch_runtime(
            rows,
            std::slice::from_ref(&expected),
            std::slice::from_ref(&updated),
            CERT_RUNTIME,
        )
    })
    .unwrap();
    assert!(read_json_at::<Certificate>(&path).unwrap().is_empty());
}

#[test]
fn runtime_merge_preserves_current_node_configuration_and_newer_results() {
    let expected = node("n");
    let mut after = expected.clone();
    after.last_deploy_at = Some("old-task".into());
    let mut current = expected.clone();
    current
        .config
        .insert("output_dir".into(), "user-edited".into());
    let mut rows = vec![current];
    patch_runtime(
        &mut rows,
        std::slice::from_ref(&expected),
        std::slice::from_ref(&after),
        NODE_RUNTIME,
    )
    .unwrap();
    assert_eq!(rows[0].config["output_dir"], "user-edited");
    assert!(rows[0].last_deploy_at.is_none());
    let mut newer = expected.clone();
    newer.last_deploy_at = Some("newer-task".into());
    newer.last_deploy_error = Some("new result".into());
    let mut rows = vec![newer];
    patch_runtime(&mut rows, &[expected], &[after], NODE_RUNTIME).unwrap();
    assert_eq!(rows[0].last_deploy_at.as_deref(), Some("newer-task"));
    assert_eq!(rows[0].last_deploy_error.as_deref(), Some("new result"));
}

#[test]
fn unchanged_configuration_receives_only_runtime_fields() {
    let expected = node("n");
    let mut updated = expected.clone();
    updated.last_deploy_error = Some("deployment failed".into());
    let mut current = vec![expected.clone(), node("other")];
    patch_runtime(&mut current, &[expected.clone()], &[updated], NODE_RUNTIME).unwrap();
    assert_eq!(current.len(), 2);
    assert_eq!(current[0].config, expected.config);
    assert_eq!(
        current[0].last_deploy_error.as_deref(),
        Some("deployment failed")
    );
}

#[test]
fn corrupt_state_and_mutation_errors_leave_existing_bytes_intact() {
    let root = Root::new();
    let path = root.table();
    std::fs::write(&path, b"{broken").unwrap();
    assert!(mutate_json_at::<Value, _>(&path, |_| Ok(())).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"{broken");
    std::fs::write(&path, b"[]").unwrap();
    assert!(mutate_json_at(&path, |rows: &mut Vec<Value>| {
        rows.push(json!(1));
        Err::<(), _>("rejected edit".into())
    })
    .is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"[]");
}

#[test]
fn atomic_write_failure_is_returned_to_the_configuration_caller() {
    let root = Root::new();
    let parent = root.0.join("blocked");
    let path = parent.join("table.json");
    let result = mutate_json_at(&path, |rows: &mut Vec<Value>| {
        std::fs::write(&parent, b"not a directory").unwrap();
        rows.push(json!(1));
        Ok(())
    });
    assert!(result.is_err());
    assert_eq!(std::fs::read(parent).unwrap(), b"not a directory");
    assert!(!path.exists());
}

#[test]
fn task_guard_rejects_overlapping_issuance_and_releases_on_drop() {
    let root = Root::new();
    let id = root.0.to_string_lossy();
    let guard = CertTaskGuard::acquire(&id).unwrap();
    assert!(CertTaskGuard::acquire(&id).is_err());
    drop(guard);
    assert!(CertTaskGuard::acquire(&id).is_ok());
}

#[test]
fn node_deletion_fails_closed_before_changing_any_association() {
    let root = Root::new();
    let nodes = root.0.join("deploy_nodes.json");
    write_json_at(&nodes, &[node("n")]).unwrap();
    let before = std::fs::read(&nodes).unwrap();
    std::fs::write(root.0.join("certificates.json"), b"invalid json").unwrap();
    assert!(delete_node_at(&root.0, "n").is_err());
    assert_eq!(std::fs::read(nodes).unwrap(), before);
}

#[test]
fn unchanged_read_view_does_not_create_or_rewrite_configuration() {
    let root = Root::new();
    let path = root.table();
    let result = mutate_json_at(&path, |rows: &mut Vec<Value>| Ok(rows.clone())).unwrap();
    assert!(result.is_empty());
    assert!(!path.exists());
    std::fs::write(&path, b"[  { \"id\": 1 }  ]").unwrap();
    mutate_json_at(&path, |rows: &mut Vec<Value>| Ok(rows.clone())).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), b"[  { \"id\": 1 }  ]");
}
