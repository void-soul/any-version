use super::*;

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
        // Windows 接收端 / API / Docker 拉取，均走独立的 cert-receiver /push
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
        return Err("节点缺少共享 Token（需与 cert-receiver 的 token 一致）".into());
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
    let host = node.config.get("host").cloned().unwrap_or_default();
    let port = node
        .config
        .get("port")
        .cloned()
        .unwrap_or_else(|| "22".into());
    let user = node.config.get("user").cloned().unwrap_or_default();
    let cert_path = node.config.get("cert_path").cloned().unwrap_or_default();
    let key_path = node.config.get("key_path").cloned().unwrap_or_default();
    let reload = node.config.get("reload_cmd").cloned().unwrap_or_default();
    if host.is_empty() || cert_path.is_empty() || key_path.is_empty() {
        return Err("linux 节点缺少 host/cert_path/key_path".into());
    }
    // 使用隔离安全子目录，避免私钥在通用临时文件目录下被其它非特权进程读取
    let isolate_dir = std::env::temp_dir().join(format!(".cert_deploy_{}", node.id));
    let _ = std::fs::create_dir_all(&isolate_dir);
    let tmp = isolate_dir.join("cert.pem");
    let tmp_key = isolate_dir.join("key.pem");
    std::fs::write(&tmp, &pems.0).map_err(|e| e.to_string())?;
    std::fs::write(&tmp_key, &pems.1).map_err(|e| e.to_string())?;

    let target = format!("{}@{}", user, host);
    let mut scp_cert_cmd = Command::new("scp");
    #[cfg(windows)]
    scp_cert_cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let scp_cert = scp_cert_cmd
        .args([
            "-P",
            &port,
            tmp.to_str().unwrap(),
            &format!("{}:{}", target, cert_path),
        ])
        .output();
    let mut scp_key_cmd = Command::new("scp");
    #[cfg(windows)]
    scp_key_cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let scp_key = scp_key_cmd
        .args([
            "-P",
            &port,
            tmp_key.to_str().unwrap(),
            &format!("{}:{}", target, key_path),
        ])
        .output();
    let _ = std::fs::remove_dir_all(&isolate_dir);
    match scp_cert {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            return Err(format!(
                "scp 证书失败 ({}): {}",
                o.status,
                String::from_utf8_lossy(&o.stderr).trim()
            ))
        }
        Err(e) => return Err(format!("scp 证书失败: {}", e)),
    }
    match scp_key {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            return Err(format!(
                "scp 私钥失败 ({}): {}",
                o.status,
                String::from_utf8_lossy(&o.stderr).trim()
            ))
        }
        Err(e) => return Err(format!("scp 私钥失败: {}", e)),
    }
    if !reload.is_empty() {
        let mut ssh_cmd = Command::new("ssh");
        #[cfg(windows)]
        ssh_cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        match ssh_cmd.args(["-p", &port, &target, &reload]).output() {
            Ok(o) if o.status.success() => {}
            Ok(o) => {
                return Err(format!(
                    "远程重载命令失败 ({}): {}",
                    o.status,
                    String::from_utf8_lossy(&o.stderr).trim()
                ))
            }
            Err(e) => return Err(format!("无法执行 ssh: {}", e)),
        }
    }
    Ok(())
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
        if Command::new("sshpass").arg("-V").output().is_err() {
            return Err("密码认证需要系统安装 sshpass；建议改用私钥认证".into());
        }
        let mut c = Command::new("sshpass");
        c.arg("-p").arg(pw).arg("ssh");
        c
    } else {
        Command::new("ssh")
    };
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    if let Some(key) = &auth.key {
        cmd.arg("-i").arg(key);
    }
    cmd.arg("-o").arg("StrictHostKeyChecking=accept-new");
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
        c.arg("-p").arg(pw).arg("scp");
        c
    } else {
        Command::new("scp")
    };
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    if let Some(key) = &auth.key {
        cmd.arg("-i").arg(key);
    }
    cmd.arg("-P").arg(port);
    cmd.arg("-o").arg("StrictHostKeyChecking=accept-new");
    cmd.arg(local).arg(format!("{}:{}", target, remote));
    let out = cmd.output().map_err(|e| format!("scp 失败: {}", e))?;
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
    let auth = ssh_auth(node);
    let cert_dir = {
        let d = cfg(node, "cert_dir");
        if d.is_empty() {
            "/etc/nginx/certificates".to_string()
        } else {
            d
        }
    };
    let name = remote_safe_name(if target.trim().is_empty() {
        cert.domains.first().map(|s| s.as_str()).unwrap_or(&cert.id)
    } else {
        target
    });
    let remote_dir = format!("{}/{}", cert_dir.trim_end_matches('/'), name);
    let quoted_dir = remote_dir.replace('\'', "'\\''");

    let pre = cfg(node, "pre_command");
    if !pre.is_empty() {
        let (mut cmd, _) = ssh_cmd_args(node, &auth, &pre)?;
        let out = cmd
            .output()
            .map_err(|e| format!("执行前置命令失败: {}", e))?;
        if !out.status.success() {
            return Err(format!(
                "前置命令失败 ({}): {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    }

    let (mut mkdir_cmd, _) = ssh_cmd_args(node, &auth, &format!("mkdir -p '{}'", quoted_dir))?;
    let mkdir = mkdir_cmd
        .output()
        .map_err(|e| format!("创建远端目录失败: {}", e))?;
    if !mkdir.status.success() {
        return Err(format!(
            "创建远端目录失败 ({}): {}",
            mkdir.status,
            String::from_utf8_lossy(&mkdir.stderr).trim()
        ));
    }

    let isolate_dir = std::env::temp_dir().join(format!(
        ".cert_deploy_{}_{}",
        node.id,
        Utc::now().timestamp_millis()
    ));
    std::fs::create_dir_all(&isolate_dir).map_err(|e| e.to_string())?;
    let local_cert = isolate_dir.join("fullchain.cer");
    let local_key = isolate_dir.join("cert.key");
    std::fs::write(&local_cert, &pems.0).map_err(|e| e.to_string())?;
    std::fs::write(&local_key, &pems.1).map_err(|e| e.to_string())?;
    let upload = scp_upload(
        node,
        &auth,
        &local_cert.to_string_lossy(),
        &format!("{}/fullchain.cer", remote_dir),
    )
    .and_then(|_| {
        scp_upload(
            node,
            &auth,
            &local_key.to_string_lossy(),
            &format!("{}/cert.key", remote_dir),
        )
    });
    let _ = std::fs::remove_dir_all(&isolate_dir);
    upload?;

    let post = cfg(node, "post_command");
    if !post.is_empty() {
        let (mut cmd, _) = ssh_cmd_args(node, &auth, &post)?;
        let out = cmd
            .output()
            .map_err(|e| format!("执行后置命令失败: {}", e))?;
        if !out.status.success() {
            return Err(format!(
                "后置命令失败 ({}): {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    }
    Ok(())
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
