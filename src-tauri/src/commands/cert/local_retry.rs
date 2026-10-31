//! Failed local deployment retries consume existing certificate files, never another ACME issuance.
use super::*;

pub(super) fn pending_local_links(
    cert: &Certificate,
    nodes: &[DeployNode],
    pending: &[super::local_pending::PendingLocal],
) -> Vec<DeployLink> {
    let default_target = cert.domains.first().map(|v| v.trim()).unwrap_or("");
    cert_links(cert)
        .into_iter()
        .filter(|link| {
            let target = if link.target.trim().is_empty() {
                default_target
            } else {
                link.target.trim()
            };
            nodes
                .iter()
                .any(|node| node.id == link.node_id && node.node_type == "local")
                && pending.iter().any(|entry| {
                    entry.cert_id == cert.id
                        && entry.node_id == link.node_id
                        && entry.target == target
                })
        })
        .collect()
}

fn report_queue_error(app: &tauri::AppHandle, message: String) {
    eprintln!("[cert] {message}");
    app.state::<CertScheduler>()
        .state
        .lock()
        .unwrap()
        .push_log(SchedulerLogEntry {
            at: Utc::now().to_rfc3339(),
            cert_id: String::new(),
            action: "deploy".into(),
            ok: false,
            message,
        });
}

pub(super) async fn retry_local_deployments(app: &tauri::AppHandle, attempted_issuance: &[String]) {
    let state = (|| {
        let pending = super::local_pending::snapshot()?;
        let nodes = load_json_checked::<DeployNode>("deploy_nodes.json")?;
        let certificates = load_json_checked::<Certificate>("certificates.json")?;
        Ok::<_, String>((pending, nodes, certificates))
    })();
    let (pending, mut nodes, certificates) = match state {
        Ok(state) => state,
        Err(error) => {
            report_queue_error(
                app,
                format!("本机部署重试已停止，未改写待部署队列: {error}"),
            );
            return;
        }
    };
    for cert in certificates {
        if attempted_issuance.contains(&cert.id) {
            continue;
        }
        let links = pending_local_links(&cert, &nodes, &pending);
        if links.is_empty() {
            continue;
        }
        let _operation = match crate::runtime_tasks::begin_operation("cert") {
            Ok(guard) => guard,
            Err(error) => {
                report_queue_error(app, format!("本机重试未执行: {error}"));
                return;
            }
        };
        let _task = match CertTaskGuard::acquire(&cert.id) {
            Ok(guard) => guard,
            Err(error) => {
                eprintln!("[cert] 本机重试暂缓: {error}");
                continue;
            }
        };
        let mut log = IssueLog::new(Some(app.clone()), &cert.id);
        log.line("重试本机部署，使用已有证书，不重新签发");
        let message = match read_lego_pems(
            &ensure_dir().join(&cert.id),
            cert.domains.first().map(String::as_str),
        ) {
            Some(pem) => {
                let expected_nodes = nodes.clone();
                let (summary, records) = deploy_links(
                    &mut log,
                    &cert,
                    &(pem.crt, pem.key, pem.issuer),
                    &mut nodes,
                    &links,
                    "scheduler",
                )
                .await;
                let persisted = append_deploy_history(&records).and_then(|_| {
                    merge_runtime("deploy_nodes.json", &expected_nodes, &nodes, NODE_RUNTIME)
                });
                match persisted {
                    Ok(current) => nodes = current,
                    Err(error) => {
                        report_queue_error(app, format!("本机部署结果保存失败: {error}"));
                        return;
                    }
                }
                (
                    summary.fail_count == 0,
                    format!(
                        "本机部署重试：成功 {} 个，失败 {} 个",
                        summary.ok_count, summary.fail_count
                    ),
                )
            }
            None => (false, "本机部署重试失败：找不到已有证书文件".into()),
        };
        log.line(&message.1);
        app.state::<CertScheduler>()
            .state
            .lock()
            .unwrap()
            .push_log(SchedulerLogEntry {
                at: Utc::now().to_rfc3339(),
                cert_id: cert.id,
                action: "deploy".into(),
                ok: message.0,
                message: message.1,
            });
    }
}

#[cfg(test)]
mod tests {
    use super::super::local_pending::PendingLocal;
    use super::*;
    fn fixtures() -> (Certificate, Vec<DeployNode>, Vec<PendingLocal>) {
        let cert = serde_json::from_value(serde_json::json!({"id":"cert","domains":["a.invalid"],"email":"test@example.invalid","dns_provider":"","deploy_links":[{"node_id":"local","target":""},{"node_id":"remote","target":""}]})).unwrap();
        let nodes = ["local", "remote"].into_iter().map(|id| serde_json::from_value(serde_json::json!({"id":id,"name":id,"type":if id=="local"{"local"}else{"api"},"config":{}})).unwrap()).collect();
        let pending = ["local", "remote"]
            .into_iter()
            .map(|node_id| PendingLocal {
                cert_id: "cert".into(),
                node_id: node_id.into(),
                target: "a.invalid".into(),
                generation: "generation".into(),
            })
            .collect();
        (cert, nodes, pending)
    }
    #[test]
    fn pending_local_deployment_retries_with_existing_unexpired_certificate() {
        let (mut cert, nodes, pending) = fixtures();
        cert.not_after = Some("2099-01-01T00:00:00Z".into());
        let links = pending_local_links(&cert, &nodes, &pending);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].node_id, "local");
    }
    #[test]
    fn other_certificate_or_target_does_not_trigger_a_retry() {
        let (cert, nodes, mut pending) = fixtures();
        pending[0].cert_id = "other".into();
        assert!(pending_local_links(&cert, &nodes, &pending).is_empty());
        pending[0].cert_id = cert.id.clone();
        pending[0].target = "b.invalid".into();
        assert!(pending_local_links(&cert, &nodes, &pending).is_empty());
    }
    #[test]
    fn removed_links_deleted_nodes_and_missing_intents_are_not_executed() {
        let (mut cert, nodes, pending) = fixtures();
        assert!(pending_local_links(&cert, &nodes, &[]).is_empty());
        assert!(pending_local_links(&cert, &[], &pending).is_empty());
        cert.deploy_links.clear();
        cert.deploy_node_ids.clear();
        assert!(pending_local_links(&cert, &nodes, &pending).is_empty());
    }
}
