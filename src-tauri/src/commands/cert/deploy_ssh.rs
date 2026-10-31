use super::*;
#[path = "deploy_ssh_transaction.rs"]
mod transaction;
use std::time::Duration;
pub(super) use transaction::run_bounded;
use transaction::{quote, PrivateFiles};

// ---------------------------------------------------------------------------
// 部署
// ---------------------------------------------------------------------------

pub(super) fn find_cert<'a>(certs: &'a [Certificate], id: &str) -> Option<&'a Certificate> {
    certs.iter().find(|c| c.id == id)
}
pub(super) fn find_node<'a>(nodes: &'a [DeployNode], id: &str) -> Option<&'a DeployNode> {
    nodes.iter().find(|n| n.id == id)
}

pub(super) async fn deploy_to_node(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    match node.node_type.as_str() {
        "local" => super::deploy_local::deploy_local(node, cert, pems, target).await,
        // Windows 接收端 / API / Docker 拉取，均调用目标 Kira 内置接收端的 /push
        "windows" | "api" | "docker" => deploy_http_push(node, cert, pems, target).await,
        "ssh" => deploy_ssh(node, cert, pems, target),
        // 旧版 linux 节点保留原有「证书路径 + 私钥路径 + 重载命令」语义
        "linux" => deploy_linux(node, pems),
        "webhook" => deploy_webhook(node, cert, pems).await,
        "baota" => deploy_baota(node, cert, pems, target).await,
        "one_panel" => deploy_one_panel(node, cert, pems, target).await,
        "synology" => deploy_synology(node, cert, pems, target).await,
        "upyun" | "upyun_ssl" => deploy_upyun(node, cert, pems, target).await,
        "baidu_cdn" => deploy_baidu_cdn(node, cert, pems, target).await,
        "huawei_cdn" => deploy_huawei_cdn(node, cert, pems, target).await,
        "volcengine_cdn" => deploy_volcengine_cdn(node, cert, pems, target).await,
        "doge_cdn" => deploy_doge_cdn(node, cert, pems, target).await,
        "aliyun_oss" => deploy_aliyun_oss(node, cert, pems, target).await,
        "aliyun_fc" => deploy_aliyun_fc(node, cert, pems, target).await,
        "aliyun" | "aliyun_ssl" => deploy_aliyun_ssl(node, cert, pems, target).await,
        "aliyun_cdn" => deploy_aliyun_cdn(node, cert, pems, target).await,
        "aliyun_dcdn" => deploy_aliyun_dcdn(node, cert, pems, target).await,
        "aliyun_slb" => deploy_aliyun_slb(node, cert, pems, target).await,
        "aliyun_alb" => deploy_aliyun_alb(node, cert, pems, target).await,
        "aliyun_nlb" => deploy_aliyun_nlb(node, cert, pems, target).await,
        "aliyun_api_gateway" => deploy_aliyun_api_gateway(node, cert, pems, target).await,
        "tencent_ssl" => deploy_tencent_ssl(node, cert, pems, target).await,
        "tencent_clb" => deploy_tencent_clb(node, cert, pems, target).await,
        "tencent_cdn" => deploy_tencent_cdn(node, cert, pems, target).await,
        "qiniu" | "qiniu_ssl" => deploy_qiniu_ssl(node, cert, pems, target).await,
        other => Err(format!("未知部署节点类型: {}", other)),
    }
}

/// 把接收端 URL 归一化为「基础地址」（不含 /push 段、无结尾斜杠）。
/// 兼容用户三种输入：`http://h:9000` / `http://h:9000/` / `http://h:9000/push`。
/// 部署与连通性测试必须从同一基础地址派生（/push 与 /health），
/// 否则按占位提示输入完整 /push URL 时，连通测试会打到 `.../push/health`（404）。
pub(super) fn windows_base_url(url: &str) -> String {
    let mut u = url.trim().to_string();
    while u.ends_with('/') {
        u.pop();
    }
    if let Some(idx) = u.rfind('/') {
        if u[idx..].eq_ignore_ascii_case("/push") {
            u.truncate(idx);
        }
    }
    u
}

pub(super) fn cfg(node: &DeployNode, key: &str) -> String {
    node.config
        .get(key)
        .cloned()
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// 通用云厂商 AK/SK（字段名与阿里云/百度/华为/火山一致）。
pub(super) fn ak_sk(node: &DeployNode, provider: &str) -> Result<(String, String), String> {
    let ak = cfg(node, "access_key_id");
    let sk = cfg(node, "access_key_secret");
    if ak.is_empty() || sk.is_empty() {
        return Err(format!("{} 节点缺少 AccessKey ID / Secret", provider));
    }
    Ok((ak, sk))
}

pub(super) fn cert_name_for(cert: &Certificate, target: &str, fallback: &str) -> String {
    let base = if !target.trim().is_empty() {
        target.trim()
    } else {
        cert.domains.first().map(|s| s.as_str()).unwrap_or(fallback)
    };
    let mut name: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    name.push('-');
    name.push_str(&Utc::now().format("%Y%m%d%H%M%S").to_string());
    name.chars().take(63).collect()
}

pub(super) async fn deploy_http_push(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    let base = windows_base_url(&cfg(node, "url"));
    let token = cfg(node, "token");
    if base.is_empty() {
        return Err(format!("{} 节点缺少 url", node.node_type));
    }
    if token.is_empty() {
        return Err("节点缺少共享 Token（需与目标 Kira 接收端的 Token 一致）".into());
    }
    let push_url = format!("{}/push", base);
    let body = serde_json::json!({
        "certificate_id": cert.id,
        "domain": if target.is_empty() { cert.domains.first().cloned().unwrap_or_default() } else { target.to_string() },
        "domains": cert.domains,
        "cert": pems.0,
        "key": pems.1,
        "ca": pems.2,
        "expired_time": cert.not_after.clone().unwrap_or_default(),
        "cert_name": cert_name_for(cert, target, &cert.id),
    });
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .connect_timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .post(&push_url)
        .bearer_auth(&token)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("推送证书到接收端失败: {}", e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("接收端返回 {}: {}", status, text.trim()));
    }
    Ok(())
}

pub(super) fn deploy_linux(
    node: &DeployNode,
    pems: &(String, String, String),
) -> Result<(), String> {
    let cert_path = cfg(node, "cert_path");
    let key_path = cfg(node, "key_path");
    if cert_path.is_empty() || key_path.is_empty() {
        return Err("linux node requires cert_path/key_path".into());
    }
    deploy_pair(
        node,
        pems,
        &cert_path,
        &key_path,
        "",
        &cfg(node, "reload_cmd"),
    )
}

pub(super) fn remote_safe_name(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() {
        "cert".into()
    } else {
        out
    }
}

pub(super) struct SshAuth {
    pub(super) password: Option<String>,
    pub(super) key: Option<String>,
}

pub(super) fn ssh_auth(node: &DeployNode) -> SshAuth {
    let method = cfg(node, "auth_method");
    if method == "password" {
        SshAuth {
            password: Some(cfg(node, "password")),
            key: None,
        }
    } else {
        let key = cfg(node, "private_key");
        SshAuth {
            password: None,
            key: if key.is_empty() { None } else { Some(key) },
        }
    }
}

pub(super) fn ssh_cmd_args(
    node: &DeployNode,
    auth: &SshAuth,
    remote_cmd: &str,
) -> Result<(Command, String), String> {
    let host = cfg(node, "host");
    let port = {
        let p = cfg(node, "port");
        if p.is_empty() {
            "22".to_string()
        } else {
            p
        }
    };
    let user = {
        let u = cfg(node, "user");
        if u.is_empty() {
            "root".to_string()
        } else {
            u
        }
    };
    if host.is_empty() {
        return Err("ssh 节点缺少 host".into());
    }
    let target = format!("{}@{}", user, host);
    let mut cmd = if let Some(pw) = &auth.password {
        if pw.is_empty() {
            return Err("ssh 密码认证需要填写 password".into());
        }
        let mut c = Command::new("sshpass");
        c.env("SSHPASS", pw).args(["-e", "ssh"]);
        c
    } else {
        Command::new("ssh")
    };
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    if let Some(key) = &auth.key {
        cmd.arg("-i").arg(key);
    }
    transport_options(&mut cmd, auth);
    cmd.arg("-p").arg(port);
    cmd.arg(&target).arg(remote_cmd);
    Ok((cmd, target))
}

pub(super) fn scp_upload(
    node: &DeployNode,
    auth: &SshAuth,
    local: &str,
    remote: &str,
) -> Result<(), String> {
    let host = cfg(node, "host");
    let port = {
        let p = cfg(node, "port");
        if p.is_empty() {
            "22".to_string()
        } else {
            p
        }
    };
    let user = {
        let u = cfg(node, "user");
        if u.is_empty() {
            "root".to_string()
        } else {
            u
        }
    };
    let target = format!("{}@{}", user, host);
    let mut cmd = if let Some(pw) = &auth.password {
        let mut c = Command::new("sshpass");
        c.env("SSHPASS", pw).args(["-e", "scp"]);
        c
    } else {
        Command::new("scp")
    };
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    if let Some(key) = &auth.key {
        cmd.arg("-i").arg(key);
    }
    cmd.arg("-O").arg("-P").arg(port);
    transport_options(&mut cmd, auth);
    cmd.arg(local).arg(format!("{}:{}", target, remote));
    let out = run_bounded(&mut cmd, Duration::from_secs(60))?;
    if !out.status.success() {
        return Err(format!(
            "scp 失败 ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// SSH 部署（对齐 ohttps）：前置命令 → 上传 cert.key/fullchain.cer → 后置命令。
pub(super) fn deploy_ssh(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    let cert_dir = cfg(node, "cert_dir");
    let cert_dir = if cert_dir.is_empty() {
        "/etc/nginx/certificates"
    } else {
        &cert_dir
    };
    let name = remote_safe_name(if target.trim().is_empty() {
        cert.domains.first().map(|s| s.as_str()).unwrap_or(&cert.id)
    } else {
        target
    });
    let remote_dir = format!("{}/{}", cert_dir.trim_end_matches('/'), name);
    deploy_pair(
        node,
        pems,
        &format!("{remote_dir}/fullchain.cer"),
        &format!("{remote_dir}/cert.key"),
        &cfg(node, "pre_command"),
        &cfg(node, "post_command"),
    )
}

fn transport_options(cmd: &mut Command, auth: &SshAuth) {
    for option in [
        "StrictHostKeyChecking=accept-new",
        "ConnectTimeout=10",
        "ConnectionAttempts=1",
        "ServerAliveInterval=5",
        "ServerAliveCountMax=2",
        "NumberOfPasswordPrompts=1",
        "RequestTTY=no",
        "KbdInteractiveAuthentication=no",
    ] {
        cmd.args(["-o", option]);
    }
    cmd.args([
        "-o",
        if auth.password.is_some() {
            "BatchMode=no"
        } else {
            "BatchMode=yes"
        },
    ]);
}

fn remote_run(node: &DeployNode, auth: &SshAuth, script: &str) -> Result<(), String> {
    let (mut cmd, _) = ssh_cmd_args(
        node,
        auth,
        &format!(
            "timeout --signal=TERM --kill-after=5s 75s sh -c {}",
            quote(script)
        ),
    )?;
    let out = run_bounded(&mut cmd, Duration::from_secs(90))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "SSH transaction failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ))
    }
}

fn deploy_pair(
    node: &DeployNode,
    pems: &(String, String, String),
    cert_path: &str,
    key_path: &str,
    pre: &str,
    post: &str,
) -> Result<(), String> {
    if cert_path == key_path || !cert_path.starts_with('/') || !key_path.starts_with('/') {
        return Err("SSH certificate/key targets must be distinct absolute paths".into());
    }
    let auth = ssh_auth(node);
    let files = PrivateFiles::new()?;
    let cert = files.write("cert.pem", &pems.0)?;
    let key = files.write("key.pem", &pems.1)?;
    let suffix = transaction::nonce()?;
    let cs = format!("{cert_path}.kira-{suffix}.new");
    let ks = format!("{key_path}.kira-{suffix}.new");
    let prepare = format!("set -eu; umask 077; {}\nmkdir -p -- \"$(dirname -- {})\" \"$(dirname -- {})\"; : > {}; : > {}",
        if pre.trim().is_empty() { ":" } else { pre }, quote(cert_path), quote(key_path), quote(&cs), quote(&ks));
    if let Err(error) = remote_run(node, &auth, &prepare) {
        let _ = remote_run(
            node,
            &auth,
            &format!("rm -f -- {} {}", quote(&cs), quote(&ks)),
        );
        return Err(format!("{error}; live pair untouched"));
    }
    let uploaded = scp_upload(node, &auth, &cert.to_string_lossy(), &quote(&cs))
        .and_then(|_| scp_upload(node, &auth, &key.to_string_lossy(), &quote(&ks)));
    if let Err(error) = uploaded {
        let cleanup = remote_run(
            node,
            &auth,
            &format!("rm -f -- {} {}", quote(&cs), quote(&ks)),
        );
        return Err(format!(
            "{error}; live pair untouched; stage cleanup: {}",
            if cleanup.is_ok() {
                "confirmed"
            } else {
                "unconfirmed"
            }
        ));
    }
    remote_run(
        node,
        &auth,
        &transaction::install_script(cert_path, key_path, &suffix, post),
    )
    .map_err(|e| format!("{e}; if KIRA_ROLLBACK_CONFIRMED is absent, recovery is unconfirmed"))
}

pub(super) async fn deploy_webhook(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
) -> Result<(), String> {
    let url = cfg(node, "url");
    if url.is_empty() {
        return Err("webhook 节点缺少 url".into());
    }
    let token = cfg(node, "token");
    let ts = Utc::now().timestamp_millis();
    let sign = format!("{:x}", md5::compute(format!("{}:{}", ts, token).as_bytes()));
    let body = serde_json::json!({
        "timestamp": ts,
        "payload": {
            "certificateName": cert.id,
            "certificateDomains": cert.domains,
            "certificateCertKey": pems.1,
            "certificateFullchainCerts": pems.0,
            "certificateExpireAt": cert.not_after.clone().unwrap_or_default(),
        },
        "sign": sign,
    });
    let resp = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("调用 webhook 失败: {}", e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("webhook 返回 {}: {}", status, text.trim()));
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
        if v.get("success").and_then(|x| x.as_bool()) == Some(false) {
            return Err(format!(
                "webhook 返回失败: {}",
                v.get("msg").and_then(|x| x.as_str()).unwrap_or("")
            ));
        }
    }
    Ok(())
}
