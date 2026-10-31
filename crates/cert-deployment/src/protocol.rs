//! One certificate deployment engine for local and HTTP adapters.
use crate::deployment::Deployment;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Deserialize)]
pub struct CertificatePush {
    #[serde(default)]
    pub certificate_id: String,
    #[serde(default)]
    pub domain: String,
    #[serde(default)]
    pub domains: Vec<String>,
    pub cert: String,
    pub key: String,
    #[serde(default)]
    pub ca: String,
    #[serde(default)]
    pub expired_time: String,
}

pub struct ReloadOptions<'a> {
    pub mode: &'a str,
    pub command: &'a str,
    pub nginx_prefix: &'a str,
    pub nginx_conf: &'a str,
    pub service_name: &'a str,
    pub post_script: Option<&'a str>,
}
pub struct ReceiverOptions<'a> {
    pub output_dir: &'a Path,
    pub layout: &'a str,
    pub reload: ReloadOptions<'a>,
}
#[derive(Debug)]
pub struct DeploymentError {
    pub invalid_input: bool,
    pub message: String,
}
fn invalid(message: impl Into<String>) -> DeploymentError {
    DeploymentError {
        invalid_input: true,
        message: message.into(),
    }
}
fn internal(message: impl Into<String>) -> DeploymentError {
    DeploymentError {
        invalid_input: false,
        message: message.into(),
    }
}

pub fn safe_name(name: &str) -> String {
    let value: String = name
        .chars()
        .filter(|c| *c != '*')
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let value = value.trim_matches('.');
    if value.is_empty() {
        "cert".into()
    } else {
        value.into()
    }
}
pub fn pem_first_block(pem: &str) -> String {
    let mut block = String::new();
    let mut started = false;
    for line in pem.lines() {
        let line = line.trim();
        if line.starts_with("-----BEGIN") {
            if started {
                break;
            }
            started = true;
        }
        if started {
            block.push_str(line);
            block.push('\n');
            if line.starts_with("-----END") {
                break;
            }
        }
    }
    block
}
pub fn resolve_target_names(
    layout: &str,
    body: &CertificatePush,
) -> Result<(String, String), DeploymentError> {
    // An explicit deployment target supplied by the sender overrides certificate SAN order.
    let domain = if !body.domain.trim().is_empty() {
        body.domain.trim()
    } else {
        body.domains
            .iter()
            .map(|v| v.trim())
            .find(|v| !v.is_empty())
            .unwrap_or("")
    }
    .to_string();
    let id = if body.certificate_id.trim().is_empty() {
        domain.clone()
    } else {
        body.certificate_id.trim().into()
    };
    if id.is_empty() {
        return Err(invalid("certificate_id 与 domain 不能同时为空"));
    }
    if layout == "domain" && domain.is_empty() {
        return Err(invalid("按域名部署时必须提供非空域名"));
    }
    if !["cert", "domain", "both", "flat"].contains(&layout) {
        return Err(invalid("证书部署布局无效"));
    }
    Ok((domain, id))
}

#[derive(Serialize)]
struct Meta<'a> {
    certificate_id: &'a str,
    domain: &'a str,
    domains: Vec<String>,
    expired_time: &'a str,
    updated_at_ms: u128,
}
fn directory(
    deployment: &mut Deployment,
    name: &str,
    body: &CertificatePush,
    domain: &str,
    id: &str,
) -> Result<(), DeploymentError> {
    let meta = Meta {
        certificate_id: id,
        domain,
        domains: {
            let mut domains: Vec<_> = body
                .domains
                .iter()
                .map(|v| v.trim())
                .filter(|v| !v.is_empty())
                .map(str::to_string)
                .collect();
            if domains.is_empty() && !domain.is_empty() {
                domains.push(domain.to_string());
            }
            domains
        },
        expired_time: &body.expired_time,
        updated_at_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    };
    let text = serde_json::to_string_pretty(&meta).map_err(|e| internal(e.to_string()))?;
    deployment
        .directory(
            name,
            vec![
                ("cert.key", body.key.clone()),
                ("fullchain.cer", body.cert.clone()),
                ("cert.cer", pem_first_block(&body.cert)),
                ("chain.cer", body.ca.clone()),
                ("meta.json", text),
            ],
        )
        .map_err(internal)
}

/// Restore pending files and confirm the corresponding service state before accepting work.
pub fn recover(options: &ReceiverOptions<'_>, log: impl Fn(&str)) -> Result<(), String> {
    let plan = crate::reload_plan::plan(&options.reload, crate::reload_plan::Platform::current())?;
    crate::deployment::recover_with(options.output_dir, &plan)?;
    log("证书部署恢复检查完成");
    Ok(())
}

pub fn deploy(
    options: &ReceiverOptions<'_>,
    body: &CertificatePush,
    log: impl Fn(&str),
) -> Result<(Vec<String>, String), DeploymentError> {
    deploy_with(
        options,
        body,
        &log,
        || reload(&options.reload, &log),
        |deployment| deployment.commit_journal(),
    )
}

fn deploy_with(
    options: &ReceiverOptions<'_>,
    body: &CertificatePush,
    log: &impl Fn(&str),
    mut reload_operation: impl FnMut() -> Result<String, String>,
    commit: impl FnOnce(&mut Deployment) -> Result<(), String>,
) -> Result<(Vec<String>, String), DeploymentError> {
    let (domain, id) = resolve_target_names(options.layout, body)?;
    crate::deployment::validate_identity(&body.cert, &body.key).map_err(invalid)?;
    let plan = crate::reload_plan::plan(&options.reload, crate::reload_plan::Platform::current())
        .map_err(invalid)?;
    crate::deployment::recover_with(options.output_dir, &plan).map_err(internal)?;
    let mut deployment = Deployment::new(options.output_dir).map_err(internal)?;
    deployment.set_recovery_plan(plan);
    let mut written = vec![];
    match options.layout {
        "cert" => {
            directory(&mut deployment, &safe_name(&id), body, &domain, &id)?;
            written.push(safe_name(&id));
        }
        "domain" => {
            directory(&mut deployment, &safe_name(&domain), body, &domain, &id)?;
            written.push(safe_name(&domain));
        }
        "both" => {
            directory(&mut deployment, &safe_name(&id), body, &domain, &id)?;
            written.push(safe_name(&id));
            if !domain.is_empty() && safe_name(&domain) != safe_name(&id) {
                directory(&mut deployment, &safe_name(&domain), body, &domain, &id)?;
                written.push(safe_name(&domain));
            }
        }
        "flat" => {
            let name = safe_name(if domain.is_empty() { &id } else { &domain });
            deployment
                .file(&format!("{name}.crt"), &body.cert)
                .map_err(internal)?;
            deployment
                .file(&format!("{name}.key"), &body.key)
                .map_err(internal)?;
            if !body.ca.is_empty() {
                deployment
                    .file(&format!("{name}.issuer.crt"), &body.ca)
                    .map_err(internal)?;
            }
            directory(&mut deployment, &safe_name(&id), body, &domain, &id)?;
            written.push(name);
        }
        _ => return Err(invalid("证书部署布局无效")),
    }
    deployment.install().map_err(internal)?;
    deployment.begin_reload().map_err(internal)?;
    let reload = match reload_operation() {
        Ok(result) => result,
        Err(error) => {
            return Err(restore_after_failure(
                &mut deployment,
                &mut reload_operation,
                format!("重载失败: {error}"),
            ))
        }
    };
    if let Err(error) = commit(&mut deployment) {
        return Err(restore_after_failure(
            &mut deployment,
            &mut reload_operation,
            format!("提交部署日志失败: {error}"),
        ));
    }
    log(&format!("重载：{reload}"));
    Ok((written, reload))
}

fn restore_after_failure(
    deployment: &mut Deployment,
    reload: &mut impl FnMut() -> Result<String, String>,
    error: String,
) -> DeploymentError {
    match deployment.restore_for_reload() {
        Err(rollback) => internal(format!("{error}；{rollback}；未重载未确认的文件")),
        Ok(()) => match reload() {
            Ok(_) => match deployment.confirm_restored() {
                Ok(()) => internal(format!("{error}；已恢复原证书文件并重新加载原代次")),
                Err(commit) => internal(format!(
                    "{error}；旧代已重载但恢复日志确认失败，计划保留供重试: {commit}"
                )),
            },
            Err(recovery) => internal(format!(
                "{error}；已恢复原证书文件；恢复后的重载也失败: {recovery}；运行态恢复未确认"
            )),
        },
    }
}

fn reload(options: &ReloadOptions<'_>, log: &impl Fn(&str)) -> Result<String, String> {
    let plan = crate::reload_plan::plan(options, crate::reload_plan::Platform::current())?;
    if plan.is_empty() {
        return Ok("未配置重载".into());
    }
    crate::reload_plan::execute(&plan, log)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options(root: &Path) -> ReceiverOptions<'_> {
        ReceiverOptions {
            output_dir: root,
            layout: "both",
            reload: ReloadOptions {
                mode: "none",
                command: "",
                nginx_prefix: "",
                nginx_conf: "",
                service_name: "",
                post_script: None,
            },
        }
    }
    fn identity() -> CertificatePush {
        CertificatePush {
            certificate_id: "audit-id".into(),
            domain: "audit-test.invalid".into(),
            domains: vec![],
            cert: include_str!("../tests/fixtures/identity-cert.pem").into(),
            key: include_str!("../tests/fixtures/identity-key.pem").into(),
            ca: String::new(),
            expired_time: String::new(),
        }
    }
    struct Root(std::path::PathBuf);
    impl Root {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("receiver-protocol-{}", crate::random_hex(12)));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Root {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn shared_engine_installs_complete_matching_generations() {
        let root = Root::new();
        let body = identity();
        let (targets, _) = deploy(&options(&root.0), &body, |_| {}).unwrap();
        assert_eq!(targets, vec!["audit-id", "audit-test.invalid"]);
        for name in targets {
            assert_eq!(
                std::fs::read_to_string(root.0.join(&name).join("cert.key")).unwrap(),
                body.key
            );
            assert_eq!(
                std::fs::read_to_string(root.0.join(name).join("fullchain.cer")).unwrap(),
                body.cert
            );
        }
        assert!(!std::fs::read_dir(&root.0)
            .unwrap()
            .flatten()
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with(".receiver-stage-")));
    }
    #[test]
    fn mismatched_identity_does_not_create_deployment_directory() {
        let root = Root::new();
        let output = root.0.join("not-created");
        let mut body = identity();
        body.key = include_str!("../tests/fixtures/mismatch-key.pem").into();
        assert!(
            deploy(&options(&output), &body, |_| {})
                .unwrap_err()
                .invalid_input
        );
        assert!(!output.exists());
    }
    #[test]
    fn domain_layout_requires_a_nonblank_domain() {
        let body: CertificatePush =
            serde_json::from_value(serde_json::json!({"certificate_id":"id","cert":"","key":""}))
                .unwrap();
        assert!(
            resolve_target_names("domain", &body)
                .unwrap_err()
                .invalid_input
        );
    }
    #[test]
    fn blank_domains_fall_back_to_valid_domain() {
        let body:CertificatePush=serde_json::from_value(serde_json::json!({"certificate_id":"id","domain":" example.com ","domains":[""," "],"cert":"","key":""})).unwrap();
        assert_eq!(
            resolve_target_names("domain", &body).unwrap(),
            ("example.com".into(), "id".into())
        );
    }
    #[test]
    fn explicit_deployment_domain_overrides_san_order() {
        let mut body = identity();
        body.domain = " explicit-target.invalid ".into();
        body.domains = vec!["first-san.invalid".into(), "second-san.invalid".into()];
        assert_eq!(
            resolve_target_names("domain", &body).unwrap().0,
            "explicit-target.invalid"
        );
        let root = Root::new();
        let mut config = options(&root.0);
        config.layout = "domain";
        let (written, _) = deploy(&config, &body, |_| {}).unwrap();
        assert_eq!(written, ["explicit-target.invalid"]);
        assert!(root.0.join("explicit-target.invalid/cert.key").is_file());
        assert!(!root.0.join("first-san.invalid").exists());
    }

    #[test]
    fn blank_explicit_domain_uses_first_nonempty_san() {
        let mut body = identity();
        body.domain = "  ".into();
        body.domains = vec!["  ".into(), " first-san.invalid ".into()];
        assert_eq!(
            resolve_target_names("domain", &body).unwrap().0,
            "first-san.invalid"
        );
    }

    #[test]
    fn failed_journal_commit_after_reload_restores_files_and_reloads_old_generation() {
        let root = Root::new();
        std::fs::create_dir(root.0.join("audit-id")).unwrap();
        std::fs::write(root.0.join("audit-id/cert.key"), "old-key").unwrap();
        std::fs::write(root.0.join("audit-id/fullchain.cer"), "old-cert").unwrap();
        let body = identity();
        let mut observed = vec![];
        let result = deploy_with(
            &options(&root.0),
            &body,
            &|_| {},
            || {
                observed.push(std::fs::read_to_string(root.0.join("audit-id/cert.key")).unwrap());
                Ok("fake reload".into())
            },
            |deployment| {
                // Force the actual journal writer to fail, without disk-space or ACL assumptions.
                let stage = std::fs::read_dir(&root.0)
                    .unwrap()
                    .flatten()
                    .find(|entry| {
                        entry
                            .file_name()
                            .to_string_lossy()
                            .starts_with(".receiver-stage-")
                    })
                    .unwrap()
                    .path();
                std::fs::create_dir(stage.join("journal.tmp")).unwrap();
                let result = deployment.commit_journal();
                std::fs::remove_dir(stage.join("journal.tmp")).unwrap();
                result
            },
        );
        let error = result.unwrap_err().message;
        assert!(error.contains("提交部署日志失败"));
        assert!(error.contains("重新加载原代次"));
        assert_eq!(observed, [body.key.clone(), "old-key".into()]);
        assert_eq!(
            std::fs::read_to_string(root.0.join("audit-id/fullchain.cer")).unwrap(),
            "old-cert"
        );
        assert!(!root.0.join("audit-test.invalid").exists());
    }

    #[test]
    fn failed_recovery_reload_is_explicitly_unconfirmed() {
        let root = Root::new();
        let body = identity();
        let mut calls = 0;
        let error = deploy_with(
            &options(&root.0),
            &body,
            &|_| {},
            || {
                calls += 1;
                if calls == 1 {
                    Ok("target loaded".into())
                } else {
                    Err("old reload unavailable".into())
                }
            },
            |_| Err("injected commit failure".into()),
        )
        .unwrap_err();
        assert_eq!(calls, 2);
        assert!(error.message.contains("运行态恢复未确认"));
        assert!(error.message.contains("old reload unavailable"));
        assert!(!root.0.join("audit-id").exists());
    }
    #[test]
    fn domain_only_upload_persists_resolved_listing_metadata() {
        let root = Root::new();
        let mut body = identity();
        body.certificate_id.clear();
        body.domains.clear();
        let (written, _) = deploy(&options(&root.0), &body, |_| {}).unwrap();
        assert_eq!(written, ["audit-test.invalid"]);
        let meta: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.0.join("audit-test.invalid/meta.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(meta["certificate_id"], "audit-test.invalid");
        assert_eq!(meta["domain"], "audit-test.invalid");
        assert_eq!(meta["domains"], serde_json::json!(["audit-test.invalid"]));
    }
}
