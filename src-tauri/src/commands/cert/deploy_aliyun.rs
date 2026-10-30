use super::*;
use base64::Engine as _;

// --- 云厂商：阿里云 CAS / 腾讯云 SSL / 七牛 Fusion ---

pub(super) fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// 阿里云 RPC 签名（POST + form）。参数需按 key 字典序排列。
pub(super) fn aliyun_rpc_sign(params: &BTreeMap<String, String>, secret: &str) -> String {
    let canonical = params
        .iter()
        .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    let string_to_sign = format!(
        "POST&{}&{}",
        percent_encode("/"),
        percent_encode(&canonical)
    );
    let key = format!("{}&", secret);
    let mut mac = <HmacSha1 as Mac>::new_from_slice(key.as_bytes()).unwrap();
    mac.update(string_to_sign.as_bytes());
    B64.encode(mac.finalize().into_bytes())
}

pub(super) fn aliyun_auth(node: &DeployNode) -> Result<(String, String, String), String> {
    let ak = cfg(node, "access_key_id");
    let sk = cfg(node, "access_key_secret");
    if ak.is_empty() || sk.is_empty() {
        return Err("阿里云节点缺少 AccessKey ID / Secret".into());
    }
    Ok((ak, sk, cfg(node, "region")))
}

/// 阿里云 RPC 通用调用：自动补公共参数、签名，并以 form 方式 POST。
pub(super) async fn aliyun_rpc(
    host: &str,
    version: &str,
    action: &str,
    params: BTreeMap<String, String>,
    ak: &str,
    sk: &str,
) -> Result<serde_json::Value, String> {
    let mut p = params;
    p.insert("Action".into(), action.into());
    p.insert("Version".into(), version.into());
    p.insert("Format".into(), "JSON".into());
    p.insert("AccessKeyId".into(), ak.into());
    p.insert("SignatureMethod".into(), "HMAC-SHA1".into());
    p.insert("SignatureVersion".into(), "1.0".into());
    p.insert(
        "SignatureNonce".into(),
        format!("{}-{}", Utc::now().timestamp_millis(), std::process::id()),
    );
    p.insert(
        "Timestamp".into(),
        Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    );
    let signature = aliyun_rpc_sign(&p, sk);
    p.insert("Signature".into(), signature);
    let body = p
        .iter()
        .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    let resp = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .post(format!("https://{}/", host))
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(body)
        .send()
        .await
        .map_err(|e| format!("调用阿里云 {} 失败: {}", action, e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|_| format!("阿里云返回非 JSON ({}): {}", status, text.trim()))?;
    if let Some(code) = v.get("Code").and_then(|x| x.as_str()) {
        return Err(format!(
            "阿里云 {} 失败: {} - {}",
            action,
            code,
            v.get("Message").and_then(|x| x.as_str()).unwrap_or("")
        ));
    }
    if !status.is_success() {
        return Err(format!(
            "阿里云 {} 返回 {}: {}",
            action,
            status,
            text.trim()
        ));
    }
    Ok(v)
}

pub(super) async fn aliyun_upload_cert(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<String, String> {
    let (ak, sk, _) = aliyun_auth(node)?;
    let name = cert_name_for(cert, target, &cert.id);
    let mut p = BTreeMap::new();
    p.insert("Name".into(), name.chars().take(63).collect());
    p.insert("Cert".into(), pems.0.clone());
    p.insert("Key".into(), pems.1.clone());
    let v = aliyun_rpc(
        "cas.aliyuncs.com",
        "2020-04-07",
        "UploadUserCertificate",
        p,
        &ak,
        &sk,
    )
    .await?;
    v.get("CertId")
        .map(|x| x.to_string().trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "阿里云上传证书成功但未返回 CertId".to_string())
}

pub(super) async fn deploy_aliyun_ssl(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    aliyun_upload_cert(node, cert, pems, target)
        .await
        .map(|_| ())
}

pub(super) async fn deploy_aliyun_cdn_like(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
    host: &str,
    version: &str,
    action: &str,
) -> Result<(), String> {
    if target.trim().is_empty() {
        return Err("CDN 部署需要在域名列表关联时填写目标域名".into());
    }
    let (ak, sk, _) = aliyun_auth(node)?;
    let name = cert_name_for(cert, target, &cert.id);
    let mut p = BTreeMap::new();
    p.insert("DomainName".into(), target.trim().to_string());
    p.insert("CertName".into(), name.chars().take(63).collect());
    p.insert("CertType".into(), "upload".into());
    p.insert("SSLPub".into(), pems.0.clone());
    p.insert("SSLPri".into(), pems.1.clone());
    p.insert("SSLProtocol".into(), "on".into());
    aliyun_rpc(host, version, action, p, &ak, &sk).await?;
    Ok(())
}

pub(super) async fn deploy_aliyun_cdn(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    deploy_aliyun_cdn_like(
        node,
        cert,
        pems,
        target,
        "cdn.aliyuncs.com",
        "2018-05-10",
        "SetCdnDomainSSLCertificate",
    )
    .await
}

pub(super) async fn deploy_aliyun_dcdn(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    deploy_aliyun_cdn_like(
        node,
        cert,
        pems,
        target,
        "dcdn.aliyuncs.com",
        "2018-01-15",
        "SetDcdnDomainSSLCertificate",
    )
    .await
}

pub(super) fn listener_port(node: &DeployNode) -> Result<u16, String> {
    let s = {
        let p = cfg(node, "listener_port");
        if p.is_empty() {
            cfg(node, "port")
        } else {
            p
        }
    };
    s.parse::<u16>()
        .map_err(|_| "节点缺少有效的监听端口（listener_port / port）".to_string())
}

pub(super) fn load_balancer_id(node: &DeployNode) -> Result<String, String> {
    let id = {
        let v = cfg(node, "load_balancer_id");
        if v.is_empty() {
            cfg(node, "instance_id")
        } else {
            v
        }
    };
    if id.is_empty() {
        Err("节点缺少负载均衡实例 ID".into())
    } else {
        Ok(id)
    }
}

pub(super) async fn aliyun_listener_id(
    node: &DeployNode,
    host: &str,
    version: &str,
    lb_id: &str,
    port: u16,
    protocol: Option<&str>,
) -> Result<String, String> {
    let (ak, sk, region) = aliyun_auth(node)?;
    let mut p = BTreeMap::new();
    if !region.is_empty() {
        p.insert("RegionId".into(), region);
    }
    p.insert("LoadBalancerIds.1".into(), lb_id.to_string());
    if let Some(proto) = protocol {
        p.insert("ListenerProtocol".into(), proto.to_string());
    }
    let v = aliyun_rpc(host, version, "ListListeners", p, &ak, &sk).await?;
    let listeners = v
        .get("Listeners")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    for l in &listeners {
        if l.get("ListenerPort").and_then(|x| x.as_i64()) == Some(port as i64) {
            if let Some(id) = l.get("ListenerId").and_then(|x| x.as_str()) {
                return Ok(id.to_string());
            }
        }
    }
    let ports: Vec<String> = listeners
        .iter()
        .filter_map(|l| l.get("ListenerPort").map(|p| p.to_string()))
        .collect();
    Err(format!(
        "未找到端口 {} 的监听；当前监听端口: {}",
        port,
        if ports.is_empty() {
            "无".to_string()
        } else {
            ports.join(", ")
        }
    ))
}

pub(super) async fn deploy_aliyun_slb(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    let (ak, sk, region) = aliyun_auth(node)?;
    if region.is_empty() {
        return Err("SLB 节点缺少 region".into());
    }
    let lb = load_balancer_id(node)?;
    let port = listener_port(node)?;
    let cert_id = aliyun_upload_cert(node, cert, pems, target).await?;
    let mut p = BTreeMap::new();
    p.insert("RegionId".into(), region);
    p.insert("LoadBalancerId".into(), lb);
    p.insert("ListenerPort".into(), port.to_string());
    p.insert("ServerCertificateId".into(), cert_id);
    aliyun_rpc(
        "slb.aliyuncs.com",
        "2014-05-15",
        "SetLoadBalancerHTTPSListenerAttribute",
        p,
        &ak,
        &sk,
    )
    .await?;
    Ok(())
}

pub(super) async fn deploy_aliyun_alb(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    let (ak, sk, region) = aliyun_auth(node)?;
    if region.is_empty() {
        return Err("ALB 节点缺少 region".into());
    }
    let lb = load_balancer_id(node)?;
    let port = listener_port(node)?;
    let listener =
        aliyun_listener_id(node, "alb.aliyuncs.com", "2020-06-16", &lb, port, None).await?;
    let cert_id = aliyun_upload_cert(node, cert, pems, target).await?;
    let mut p = BTreeMap::new();
    p.insert("RegionId".into(), region);
    p.insert("ListenerId".into(), listener);
    p.insert("Certificates.1.CertificateId".into(), cert_id);
    p.insert("Certificates.1.IsDefault".into(), "true".into());
    aliyun_rpc(
        "alb.aliyuncs.com",
        "2020-06-16",
        "UpdateListenerAttribute",
        p,
        &ak,
        &sk,
    )
    .await?;
    Ok(())
}

pub(super) async fn deploy_aliyun_nlb(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    let (ak, sk, region) = aliyun_auth(node)?;
    if region.is_empty() {
        return Err("NLB 节点缺少 region".into());
    }
    let lb = load_balancer_id(node)?;
    let port = listener_port(node)?;
    let listener = aliyun_listener_id(
        node,
        "nlb.aliyuncs.com",
        "2022-04-30",
        &lb,
        port,
        Some("TCPSSL"),
    )
    .await?;
    let cert_id = aliyun_upload_cert(node, cert, pems, target).await?;
    let mut p = BTreeMap::new();
    p.insert("RegionId".into(), region);
    p.insert("ListenerId".into(), listener);
    p.insert("CertificateIds.1".into(), cert_id);
    aliyun_rpc(
        "nlb.aliyuncs.com",
        "2022-04-30",
        "UpdateListenerAttribute",
        p,
        &ak,
        &sk,
    )
    .await?;
    Ok(())
}

pub(super) async fn deploy_aliyun_api_gateway(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    let group_id = cfg(node, "group_id");
    if group_id.is_empty() {
        return Err("API 网关节点缺少 group_id".into());
    }
    if target.trim().is_empty() {
        return Err("API 网关部署需要在域名列表关联时填写目标域名".into());
    }
    let (ak, sk, _) = aliyun_auth(node)?;
    let mut p = BTreeMap::new();
    p.insert("GroupId".into(), group_id);
    p.insert("DomainName".into(), target.trim().to_string());
    p.insert(
        "CertificateName".into(),
        cert_name_for(cert, target, &cert.id)
            .chars()
            .take(63)
            .collect(),
    );
    p.insert("CertificateBody".into(), pems.0.clone());
    p.insert("PrivateKey".into(), pems.1.clone());
    aliyun_rpc(
        "apigateway.aliyuncs.com",
        "2016-07-14",
        "SetDomainCertificate",
        p,
        &ak,
        &sk,
    )
    .await?;
    Ok(())
}
