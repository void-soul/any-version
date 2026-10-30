//! One receiver deployment protocol for both application adapters.
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
    let domain = body
        .domains
        .iter()
        .map(|v| v.trim())
        .find(|v| !v.is_empty())
        .unwrap_or_else(|| body.domain.trim())
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
    domains: &'a [String],
    expired_time: &'a str,
    updated_at_ms: u128,
}
fn directory(
    deployment: &mut Deployment,
    name: &str,
    body: &CertificatePush,
) -> Result<(), DeploymentError> {
    let meta = Meta {
        certificate_id: &body.certificate_id,
        domains: &body.domains,
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

pub fn deploy(
    options: &ReceiverOptions<'_>,
    body: &CertificatePush,
    log: impl Fn(&str),
) -> Result<(Vec<String>, String), DeploymentError> {
    let (domain, id) = resolve_target_names(options.layout, body)?;
    crate::deployment::validate_identity(&body.cert, &body.key).map_err(invalid)?;
    let mut deployment = Deployment::new(options.output_dir).map_err(internal)?;
    let mut written = vec![];
    match options.layout {
        "cert" => {
            directory(&mut deployment, &safe_name(&id), body)?;
            written.push(safe_name(&id));
        }
        "domain" => {
            directory(&mut deployment, &safe_name(&domain), body)?;
            written.push(safe_name(&domain));
        }
        "both" => {
            directory(&mut deployment, &safe_name(&id), body)?;
            written.push(safe_name(&id));
            if !domain.is_empty() && safe_name(&domain) != safe_name(&id) {
                directory(&mut deployment, &safe_name(&domain), body)?;
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
            directory(&mut deployment, &safe_name(&id), body)?;
            written.push(name);
        }
        _ => return Err(invalid("证书部署布局无效")),
    }
    deployment.install().map_err(internal)?;
    let reload = match reload(&options.reload, &log) {
        Ok(result) => result,
        Err(error) => {
            let rolled_back = deployment.rollback();
            let recovery = if rolled_back.is_ok() {
                reload(&options.reload, &log)
            } else {
                Ok(String::new())
            };
            return Err(internal(format!(
                "重载失败: {error}；{}{}",
                rolled_back
                    .err()
                    .unwrap_or_else(|| "已恢复原证书文件".into()),
                recovery
                    .err()
                    .map(|e| format!("；恢复后的重载也失败: {e}"))
                    .unwrap_or_default()
            )));
        }
    };
    deployment.finish().map_err(internal)?;
    log(&format!("重载：{reload}"));
    Ok((written, reload))
}

fn reload(options: &ReloadOptions<'_>, log: &impl Fn(&str)) -> Result<String, String> {
    use crate::reload_command::run;
    let prefix = options.nginx_prefix.trim();
    let cwd = if prefix.is_empty() {
        None
    } else {
        Some(prefix)
    };
    if let Some(script) = options.post_script.filter(|s| !s.trim().is_empty()) {
        log("正在执行证书部署后脚本");
        return run("cmd.exe", &["/c", script], cwd);
    }
    match options.mode {
        "none" | "" => Ok("未配置重载".into()),
        "command" => {
            if options.command.trim().is_empty() {
                return Err("重载命令不能为空".into());
            }
            log("正在执行重载命令");
            run("cmd.exe", &["/c", options.command.trim()], cwd)
        }
        "nginx" => {
            let exe = if prefix.is_empty() {
                "nginx.exe".into()
            } else {
                Path::new(prefix)
                    .join("nginx.exe")
                    .to_string_lossy()
                    .to_string()
            };
            let mut args = vec![];
            if !prefix.is_empty() {
                args.extend(["-p", prefix]);
            }
            if !options.nginx_conf.trim().is_empty() {
                args.extend(["-c", options.nginx_conf.trim()]);
            }
            let mut check = args.clone();
            check.push("-t");
            log("正在检查Nginx配置");
            run(&exe, &check, cwd)?;
            args.extend(["-s", "reload"]);
            log("正在重载Nginx");
            run(&exe, &args, cwd)
        }
        "service" => {
            let name = options.service_name.trim();
            if name.is_empty() {
                return Err("服务名不能为空".into());
            }
            log("正在重启证书服务");
            run("net.exe", &["stop", name], None)?;
            run("net.exe", &["start", name], None)
        }
        _ => Err("重载方式无效".into()),
    }
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
}
