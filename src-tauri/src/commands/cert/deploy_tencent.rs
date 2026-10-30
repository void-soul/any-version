use super::*;

pub(super) fn hmac_sha256(key: &[u8], msg: &str) -> Vec<u8> {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = <HmacSha256 as Mac>::new_from_slice(key).expect("HMAC key");
    mac.update(msg.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

pub(super) fn tencent_auth(node: &DeployNode) -> Result<(String, String, String), String> {
    let id = cfg(node, "secret_id");
    let key = cfg(node, "secret_key");
    if id.is_empty() || key.is_empty() {
        return Err("腾讯云节点缺少 SecretId / SecretKey".into());
    }
    Ok((id, key, cfg(node, "region")))
}

/// 腾讯云 TC3-HMAC-SHA256 通用调用。
pub(super) async fn tencent_rpc(
    host: &str,
    service: &str,
    version: &str,
    action: &str,
    region: &str,
    body: serde_json::Value,
    secret_id: &str,
    secret_key: &str,
) -> Result<serde_json::Value, String> {
    let content_type = "application/json; charset=utf-8";
    let now = Utc::now();
    let timestamp = now.timestamp();
    let date = now.format("%Y-%m-%d").to_string();
    let body_text = serde_json::to_string(&body).map_err(|e| e.to_string())?;

    let mut headers: Vec<(String, String)> = vec![
        ("content-type".into(), content_type.into()),
        ("host".into(), host.into()),
        ("x-tc-action".into(), action.to_lowercase()),
        ("x-tc-timestamp".into(), timestamp.to_string()),
        ("x-tc-version".into(), version.into()),
    ];
    if !region.is_empty() {
        headers.push(("x-tc-region".into(), region.to_string()));
    }
    headers.sort_by(|a, b| a.0.cmp(&b.0));
    let canonical_headers = headers
        .iter()
        .map(|(k, v)| format!("{}:{}\n", k, v))
        .collect::<String>();
    let signed_headers = headers
        .iter()
        .map(|(k, _)| k.clone())
        .collect::<Vec<_>>()
        .join(";");
    let payload_hash = hex(&Sha256::digest(body_text.as_bytes()));
    let canonical_request = format!(
        "POST\n/\n\n{}\n{}\n{}",
        canonical_headers, signed_headers, payload_hash
    );
    let string_to_sign = format!(
        "TC3-HMAC-SHA256\n{}\n{}/{}/tc3_request\n{}",
        timestamp,
        date,
        service,
        hex(&Sha256::digest(canonical_request.as_bytes()))
    );
    let secret_date = hmac_sha256(format!("TC3{}", secret_key).as_bytes(), &date);
    let secret_service = hmac_sha256(&secret_date, service);
    let secret_signing = hmac_sha256(&secret_service, "tc3_request");
    let signature = hex(&hmac_sha256(&secret_signing, &string_to_sign));
    let auth = format!(
        "TC3-HMAC-SHA256 Credential={}/{}/{}/tc3_request, SignedHeaders={}, Signature={}",
        secret_id, date, service, signed_headers, signature
    );

    let mut req = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .post(format!("https://{}", host))
        .header(reqwest::header::CONTENT_TYPE, content_type)
        .header("X-TC-Action", action)
        .header("X-TC-Timestamp", timestamp.to_string())
        .header("X-TC-Version", version)
        .header(reqwest::header::AUTHORIZATION, auth);
    if !region.is_empty() {
        req = req.header("X-TC-Region", region);
    }
    let resp = req
        .body(body_text)
        .send()
        .await
        .map_err(|e| format!("调用腾讯云 {} 失败: {}", action, e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|_| format!("腾讯云返回非 JSON ({}): {}", status, text.trim()))?;
    if let Some(err) = v.pointer("/Response/Error") {
        return Err(format!(
            "腾讯云 {} 失败: {} - {}",
            action,
            err.get("Code").and_then(|x| x.as_str()).unwrap_or(""),
            err.get("Message").and_then(|x| x.as_str()).unwrap_or("")
        ));
    }
    if !status.is_success() {
        return Err(format!(
            "腾讯云 {} 返回 {}: {}",
            action,
            status,
            text.trim()
        ));
    }
    Ok(v.get("Response").cloned().unwrap_or(v))
}

pub(super) async fn tencent_upload_cert(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<String, String> {
    let (id, key, _) = tencent_auth(node)?;
    let body = serde_json::json!({
        "CertificatePublicKey": pems.0,
        "CertificatePrivateKey": pems.1,
        "Alias": cert_name_for(cert, target, &cert.id),
        "CertificateType": "SVR",
    });
    let v = tencent_rpc(
        "ssl.tencentcloudapi.com",
        "ssl",
        "2019-12-05",
        "UploadCertificate",
        "",
        body,
        &id,
        &key,
    )
    .await?;
    v.get("CertificateId")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "腾讯云上传证书成功但未返回 CertificateId".to_string())
}

pub(super) async fn deploy_tencent_ssl(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    tencent_upload_cert(node, cert, pems, target)
        .await
        .map(|_| ())
}

pub(super) async fn deploy_tencent_clb(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    let (id, key, region) = tencent_auth(node)?;
    if region.is_empty() {
        return Err("腾讯云 CLB 节点缺少 region".into());
    }
    let lb = load_balancer_id(node)?;
    let port = listener_port(node)?;
    let v = tencent_rpc(
        "clb.tencentcloudapi.com",
        "clb",
        "2017-03-12",
        "DescribeListeners",
        &region,
        serde_json::json!({ "LoadBalancerId": lb }),
        &id,
        &key,
    )
    .await?;
    let listeners = v
        .get("Listeners")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    let listener = listeners
        .iter()
        .find(|l| {
            l.get("Port").and_then(|x| x.as_i64()) == Some(port as i64)
                || l.get("ListenerPort").and_then(|x| x.as_i64()) == Some(port as i64)
        })
        .and_then(|l| l.get("ListenerId").and_then(|x| x.as_str()))
        .ok_or_else(|| format!("未找到端口 {} 的 CLB 监听", port))?
        .to_string();
    let cert_id = tencent_upload_cert(node, cert, pems, target).await?;
    tencent_rpc(
        "clb.tencentcloudapi.com",
        "clb",
        "2017-03-12",
        "ModifyListener",
        &region,
        serde_json::json!({
            "LoadBalancerId": lb,
            "ListenerId": listener,
            "Certificate": { "SSLMode": "UNIDIRECTIONAL", "CertId": cert_id }
        }),
        &id,
        &key,
    )
    .await?;
    Ok(())
}

pub(super) async fn deploy_tencent_cdn(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    if target.trim().is_empty() {
        return Err("腾讯云 CDN 部署需要在域名列表关联时填写目标域名".into());
    }
    let (id, key, region) = tencent_auth(node)?;
    let cert_id = tencent_upload_cert(node, cert, pems, target).await?;
    // 先打开 HTTPS，再替换证书
    let _ = tencent_rpc(
        "cdn.tencentcloudapi.com",
        "cdn",
        "2018-03-09",
        "ModifyDomainConfig",
        &region,
        serde_json::json!({
            "Domain": target.trim(),
            "Route": "Https.Switch",
            "Value": "{\"update\":\"on\"}"
        }),
        &id,
        &key,
    )
    .await;
    tencent_rpc(
        "cdn.tencentcloudapi.com",
        "cdn",
        "2018-03-09",
        "ModifyDomainConfig",
        &region,
        serde_json::json!({
            "Domain": target.trim(),
            "Route": "Https.CertInfo.CertId",
            "Value": format!("{{\"update\":\"{}\"}}", cert_id)
        }),
        &id,
        &key,
    )
    .await?;
    Ok(())
}

pub(super) async fn deploy_qiniu_ssl(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    let ak = cfg(node, "access_key");
    let sk = cfg(node, "secret_key");
    if ak.is_empty() || sk.is_empty() {
        return Err("七牛 SSL 节点缺少 AccessKey / SecretKey".into());
    }
    let name = {
        let configured = cfg(node, "cert_name");
        if configured.is_empty() {
            cert_name_for(cert, target, &cert.id)
        } else {
            format!("{}-{}", configured, Utc::now().format("%Y%m%d%H%M%S"))
        }
    };
    let body = serde_json::json!({
        "name": name,
        "pri": pems.1,
        "ca": pems.0,
    });
    let body_text = serde_json::to_string(&body).map_err(|e| e.to_string())?;
    let key = format!("{}&", sk);
    let mut mac = <HmacSha1 as Mac>::new_from_slice(key.as_bytes()).unwrap();
    mac.update(format!("/sslcert\n{}", body_text).as_bytes());
    let sign = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());

    let resp = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .post("https://fusion.qiniuapi.com/sslcert")
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(
            reqwest::header::AUTHORIZATION,
            format!("QBox {}:{}", ak, sign),
        )
        .body(body_text)
        .send()
        .await
        .map_err(|e| format!("调用七牛证书服务失败: {}", e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|_| format!("七牛返回非 JSON ({}): {}", status, text.trim()))?;
    if v.get("certID").and_then(|x| x.as_str()).is_some() {
        return Ok(());
    }
    if let Some(code) = v.get("code").and_then(|x| x.as_i64()) {
        return Err(format!(
            "七牛上传证书失败: {} - {}",
            code,
            v.get("error").and_then(|x| x.as_str()).unwrap_or("")
        ));
    }
    if !status.is_success() {
        return Err(format!("七牛返回 {}: {}", status, text.trim()));
    }
    Ok(())
}
