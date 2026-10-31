//! Local deployment is the receiver's deployment engine without its HTTP adapter.
use super::*;
use cert_deployment::protocol::{CertificatePush, ReceiverOptions, ReloadOptions};

pub(super) struct LocalDeployment {
    output_dir: PathBuf,
    layout: String,
    reload_mode: String,
    command: String,
    nginx_prefix: String,
    nginx_conf: String,
    service_name: String,
    post_script: Option<String>,
}

impl LocalDeployment {
    pub(super) fn from_config(config: &HashMap<String, String>) -> Result<Self, String> {
        let get = |key: &str| config.get(key).map(|s| s.trim()).unwrap_or("").to_string();
        let output_dir = PathBuf::from(get("output_dir"));
        if output_dir.as_os_str().is_empty() {
            return Err("请填写本机证书目录".into());
        }
        if !output_dir.is_absolute() {
            return Err("本机证书目录须为绝对路径".into());
        }
        let mut layout = get("layout");
        if layout.is_empty() {
            layout = "cert".into();
        }
        if !["cert", "domain", "both", "flat"].contains(&layout.as_str()) {
            return Err("本机证书目录布局无效".into());
        }
        let mut reload_mode = get("reload_mode");
        if reload_mode.is_empty() {
            reload_mode = "none".into();
        }
        if !["none", "command", "nginx", "service"].contains(&reload_mode.as_str()) {
            return Err("本机证书重载方式无效".into());
        }
        let command = get("reload_command");
        let service_name = get("service_name");
        if reload_mode == "command" && command.is_empty() {
            return Err("请填写重载命令".into());
        }
        if reload_mode == "service" && service_name.is_empty() {
            return Err("请填写服务名称".into());
        }
        Ok(Self {
            output_dir,
            layout,
            reload_mode,
            command,
            service_name,
            nginx_prefix: get("nginx_prefix"),
            nginx_conf: get("nginx_conf"),
            post_script: config
                .get("post_script")
                .filter(|s| !s.trim().is_empty())
                .cloned(),
        })
    }

    fn options(&self) -> ReceiverOptions<'_> {
        ReceiverOptions {
            output_dir: &self.output_dir,
            layout: &self.layout,
            reload: ReloadOptions {
                mode: &self.reload_mode,
                command: &self.command,
                nginx_prefix: &self.nginx_prefix,
                nginx_conf: &self.nginx_conf,
                service_name: &self.service_name,
                post_script: self.post_script.as_deref(),
            },
        }
    }

    fn deploy(&self, body: &CertificatePush) -> Result<(), String> {
        cert_deployment::protocol::deploy(&self.options(), body, |_| {})
            .map(|_| ())
            .map_err(|e| format!("本机证书部署失败：{}", e.message))
    }

    /// Test the configured directory, never execute reload commands or touch live certificate files.
    fn probe(&self) -> Result<String, String> {
        let _lock = cert_deployment::deployment::OutputLock::try_acquire(&self.output_dir)?;
        let mut nonce = [0; 16];
        getrandom::getrandom(&mut nonce).map_err(|e| e.to_string())?;
        let path = self
            .output_dir
            .join(format!(".kira-cert-probe-{}", hex(&nonce)));
        struct Probe(PathBuf);
        impl Drop for Probe {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| format!("本机证书目录不可写：{e}"))?;
        let _probe = Probe(path);
        use std::io::Write;
        let written = file
            .write_all(b"Kira certificate directory probe")
            .and_then(|_| file.sync_all());
        drop(file);
        written.map_err(|e| format!("本机证书目录写入检查失败：{e}"))?;
        Ok(format!("本机证书目录可写：{}", self.output_dir.display()))
    }
}

pub(super) async fn deploy_local(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    let deployment = LocalDeployment::from_config(&node.config)?;
    let body = CertificatePush {
        certificate_id: cert.id.clone(),
        domain: if target.trim().is_empty() {
            cert.domains.first().cloned().unwrap_or_default()
        } else {
            target.trim().into()
        },
        domains: cert.domains.clone(),
        cert: pems.0.clone(),
        key: pems.1.clone(),
        ca: pems.2.clone(),
        expired_time: cert.not_after.clone().unwrap_or_default(),
    };
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = crate::runtime_tasks::begin_operation("cert")?;
        deployment.deploy(&body)
    })
    .await
    .map_err(|e| format!("本机证书部署任务失败：{e}"))?
}

pub(super) async fn test_local(config: HashMap<String, String>) -> Result<String, String> {
    let deployment = LocalDeployment::from_config(&config)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = crate::runtime_tasks::begin_operation("cert")?;
        deployment.probe()
    })
    .await
    .map_err(|e| format!("本机证书目录检查失败：{e}"))?
}

#[cfg(test)]
mod tests;
