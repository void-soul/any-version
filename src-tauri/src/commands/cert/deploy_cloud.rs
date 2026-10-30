use super::*;
use base64::Engine as _;

// ---------------------------------------------------------------------------
// 更多云厂商：百度智能云 / 华为云 / 火山引擎 / 多吉云 / 阿里云 OSS / 函数计算
// ---------------------------------------------------------------------------

pub(super) fn hmac_sha1_hex(key: &[u8], msg: &str) -> String {
    let mut mac = <HmacSha1 as Mac>::new_from_slice(key).unwrap();
    mac.update(msg.as_bytes());
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

pub(super) fn sorted_canonical_headers(headers: &[(String, String)]) -> (String, String) {
    let mut hs = headers.to_vec();
    hs.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    let canonical = hs
        .iter()
        .map(|(k, v)| format!("{}:{}\n", k.to_lowercase(), v.trim()))
        .collect::<String>();
    let signed = hs
        .iter()
        .map(|(k, _)| k.to_lowercase())
        .collect::<Vec<_>>()
        .join(";");
    (canonical, signed)
}

/// 百度智能云 BCE 签名（bce-auth-v1 / HMAC-SHA256；V1 规范不签 body）。
pub(super) fn bce_headers(
    method: &str,
    uri: &str,
    query: &str,
    mut headers: Vec<(String, String)>,
    ak: &str,
    sk: &str,
) -> Vec<(String, String)> {
    let ts = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    headers.push(("x-bce-date".into(), ts.clone()));
    let (canonical_headers, signed_headers) = sorted_canonical_headers(&headers);
    let canonical_request = format!("{}\n{}\n{}\n{}", method, uri, query, canonical_headers);
    let auth_prefix = format!("bce-auth-v1/{}/{}/1800", ak, ts);
    let signing_key = hex(&hmac_sha256(sk.as_bytes(), &auth_prefix));
    let signature = hex(&hmac_sha256(
        signing_key.as_bytes(),
        &format!(
            "{}\n{}",
            auth_prefix,
            hex(&Sha256::digest(canonical_request.as_bytes()))
        ),
    ));
    headers.push((
        "Authorization".into(),
        format!("{}/{}/{}", auth_prefix, signed_headers, signature),
    ));
    headers
}

pub(super) async fn deploy_baidu_cdn(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    if target.trim().is_empty() {
        return Err("百度智能云 CDN 需要在域名列表关联时填写目标域名".into());
    }
    let (ak, sk) = ak_sk(node, "百度智能云")?;
    let domain = target.trim();
    let uri = format!("/v2/{}/certificates", domain);
    let leaf = pem_first_block(&pems.0);
    let chain = pems.0.replacen(leaf.as_str(), "", 1);
    let body = serde_json::json!({
        "httpsEnable": "ON",
        "certificate": {
            "certName": cert_name_for(cert, domain, &cert.id),
            "certServerData": leaf,
            "certPrivateData": pems.1,
            "certLinkData": chain
        }
    });
    let body_text = serde_json::to_string(&body).map_err(|e| e.to_string())?;
    let headers = bce_headers(
        "PUT",
        &uri,
        "",
        vec![
            ("host".into(), "cdn.baidubce.com".into()),
            ("content-type".into(), "application/json".into()),
        ],
        &ak,
        &sk,
    );
    let mut req = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .put(format!("https://cdn.baidubce.com{}", uri));
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let resp = req
        .body(body_text)
        .send()
        .await
        .map_err(|e| format!("调用百度智能云 CDN 失败: {}", e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "百度智能云 CDN 返回 {}: {}",
            status,
            text.chars().take(300).collect::<String>()
        ));
    }
    Ok(())
}

/// 华为云 AK/SK 签名（SDK-HMAC-SHA256）。
pub(super) fn huawei_headers(
    method: &str,
    uri: &str,
    query: &str,
    mut headers: Vec<(String, String)>,
    body: &[u8],
    ak: &str,
    sk: &str,
) -> Vec<(String, String)> {
    let date = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    headers.push(("X-Sdk-Date".into(), date.clone()));
    let (canonical_headers, signed_headers) = sorted_canonical_headers(&headers);
    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        method,
        uri,
        query,
        canonical_headers,
        signed_headers,
        hex(&Sha256::digest(body))
    );
    let string_to_sign = format!(
        "SDK-HMAC-SHA256\n{}\n{}",
        date,
        hex(&Sha256::digest(canonical_request.as_bytes()))
    );
    let signature = hex(&hmac_sha256(sk.as_bytes(), &string_to_sign));
    headers.push((
        "Authorization".into(),
        format!(
            "SDK-HMAC-SHA256 Access={}, SignedHeaders={}, Signature={}",
            ak, signed_headers, signature
        ),
    ));
    headers
}

pub(super) async fn deploy_huawei_cdn(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    if target.trim().is_empty() {
        return Err("华为云 CDN 需要在域名列表关联时填写目标域名".into());
    }
    let (ak, sk) = ak_sk(node, "华为云")?;
    let domain = target.trim();
    let uri = "/v1.0/cdn/domains/config-https-info";
    let query = "enterprise_project_id=all";
    let body = serde_json::json!({
        "https": {
            "domain_name": domain,
            "https_switch": 1,
            "cert_name": cert_name_for(cert, domain, &cert.id),
            "certificate": pems.0,
            "private_key": pems.1,
            "certificate_type": 0
        }
    });
    let body_text = serde_json::to_string(&body).map_err(|e| e.to_string())?;
    let headers = huawei_headers(
        "PUT",
        uri,
        query,
        vec![
            ("content-type".into(), "application/json".into()),
            ("host".into(), "cdn.myhuaweicloud.com".into()),
        ],
        body_text.as_bytes(),
        &ak,
        &sk,
    );
    let mut req = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .put(format!("https://cdn.myhuaweicloud.com{}?{}", uri, query));
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let resp = req
        .body(body_text)
        .send()
        .await
        .map_err(|e| format!("调用华为云 CDN 失败: {}", e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "华为云 CDN 返回 {}: {}",
            status,
            text.chars().take(300).collect::<String>()
        ));
    }
    Ok(())
}

/// 火山引擎 V4 签名（service=cdn, region=cn-north-1）。
pub(super) fn volc_headers(
    method: &str,
    uri: &str,
    query: &str,
    mut headers: Vec<(String, String)>,
    body: &[u8],
    ak: &str,
    sk: &str,
) -> Vec<(String, String)> {
    let date = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let short = &date[..8];
    headers.push(("X-Date".into(), date.clone()));
    let (canonical_headers, signed_headers) = sorted_canonical_headers(&headers);
    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        method,
        uri,
        query,
        canonical_headers,
        signed_headers,
        hex(&Sha256::digest(body))
    );
    let scope = format!("{}/cn-north-1/cdn/request", short);
    let string_to_sign = format!(
        "HMAC-SHA256\n{}\n{}\n{}",
        date,
        scope,
        hex(&Sha256::digest(canonical_request.as_bytes()))
    );
    let k_date = hmac_sha256(sk.as_bytes(), short);
    let k_region = hmac_sha256(&k_date, "cn-north-1");
    let k_service = hmac_sha256(&k_region, "cdn");
    let k_signing = hmac_sha256(&k_service, "request");
    let signature = hex(&hmac_sha256(&k_signing, &string_to_sign));
    headers.push((
        "Authorization".into(),
        format!(
            "HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
            ak, scope, signed_headers, signature
        ),
    ));
    headers
}

pub(super) async fn volc_call(
    node: &DeployNode,
    action: &str,
    body: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let (ak, sk) = ak_sk(node, "火山引擎")?;
    let query = format!("Action={}&Version=2021-03-01", action);
    let body_text = serde_json::to_string(&body).map_err(|e| e.to_string())?;
    let headers = volc_headers(
        "POST",
        "/",
        &query,
        vec![
            ("content-type".into(), "application/json".into()),
            ("host".into(), "cdn.volcengineapi.com".into()),
        ],
        body_text.as_bytes(),
        &ak,
        &sk,
    );
    let mut req = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .post(format!("https://cdn.volcengineapi.com/?{}", query));
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let resp = req
        .body(body_text)
        .send()
        .await
        .map_err(|e| format!("调用火山引擎 {} 失败: {}", action, e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    if let Some(err) = v.pointer("/ResponseMetadata/Error") {
        return Err(format!(
            "火山引擎 {} 失败: {} - {}",
            action,
            err.get("Code").and_then(|x| x.as_str()).unwrap_or(""),
            err.get("Message").and_then(|x| x.as_str()).unwrap_or("")
        ));
    }
    if !status.is_success() {
        return Err(format!(
            "火山引擎 {} 返回 {}: {}",
            action,
            status,
            text.chars().take(300).collect::<String>()
        ));
    }
    Ok(v)
}

pub(super) async fn deploy_volcengine_cdn(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    if target.trim().is_empty() {
        return Err("火山引擎 CDN 需要在域名列表关联时填写目标域名".into());
    }
    let domain = target.trim();
    let uploaded = volc_call(
        node,
        "AddCertificate",
        serde_json::json!({
            "Certificate": pems.0,
            "PrivateKey": pems.1,
            "Source": "volc_cert_center",
            "CertType": "server_cert",
            "Desc": cert_name_for(cert, domain, &cert.id)
        }),
    )
    .await?;
    let cert_id = uploaded
        .pointer("/Result/CertId")
        .map(json_id_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "火山引擎上传证书成功但未返回 CertId".to_string())?;
    volc_call(
        node,
        "UpdateCdnConfig",
        serde_json::json!({
            "Domain": domain,
            "HTTPS": { "Switch": true, "CertInfo": { "CertId": cert_id } }
        }),
    )
    .await?;
    Ok(())
}

/// 多吉云：Authorization: TOKEN {AK}:{hmac_sha1_hex(SK, uri + "\n" + body)}
pub(super) async fn doge_call(
    path: &str,
    body: serde_json::Value,
    ak: &str,
    sk: &str,
) -> Result<serde_json::Value, String> {
    let body_text = serde_json::to_string(&body).map_err(|e| e.to_string())?;
    let sign = hmac_sha1_hex(sk.as_bytes(), &format!("{}\n{}", path, body_text));
    let resp = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .post(format!("https://api.dogecloud.com{}", path))
        .header(
            reqwest::header::AUTHORIZATION,
            format!("TOKEN {}:{}", ak, sign),
        )
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body_text)
        .send()
        .await
        .map_err(|e| format!("调用多吉云失败: {}", e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|_| format!("多吉云返回非 JSON ({}): {}", status, text.trim()))?;
    if let Some(code) = v.get("code").and_then(|x| x.as_i64()) {
        if code != 200 {
            return Err(format!(
                "多吉云 {} 失败: {} - {}",
                path,
                code,
                v.get("msg").and_then(|x| x.as_str()).unwrap_or("")
            ));
        }
    }
    if !status.is_success() {
        return Err(format!("多吉云返回 {}: {}", status, text.trim()));
    }
    Ok(v)
}

pub(super) async fn deploy_doge_cdn(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    if target.trim().is_empty() {
        return Err("多吉云 CDN 需要在域名列表关联时填写目标域名".into());
    }
    let ak = cfg(node, "access_key");
    let sk = cfg(node, "secret_key");
    if ak.is_empty() || sk.is_empty() {
        return Err("多吉云节点缺少 AccessKey / SecretKey".into());
    }
    let domain = target.trim();
    let uploaded = doge_call(
        "/cdn/cert/upload.json",
        serde_json::json!({
            "note": cert_name_for(cert, domain, &cert.id),
            "cert": pems.0,
            "private": pems.1
        }),
        &ak,
        &sk,
    )
    .await?;
    let cert_id = uploaded
        .pointer("/data/id")
        .map(json_id_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "多吉云上传证书成功但未返回证书 ID".to_string())?;
    doge_call(
        "/cdn/cert/bind.json",
        serde_json::json!({ "id": cert_id, "domain": domain }),
        &ak,
        &sk,
    )
    .await?;
    Ok(())
}

pub(super) async fn deploy_aliyun_oss(
    node: &DeployNode,
    _cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    if target.trim().is_empty() {
        return Err("阿里云 OSS 需要在域名列表关联时填写目标域名".into());
    }
    let bucket = cfg(node, "bucket");
    let endpoint = cfg(node, "endpoint");
    let ak = cfg(node, "access_key_id");
    let sk = cfg(node, "access_key_secret");
    if bucket.is_empty() || endpoint.is_empty() || ak.is_empty() || sk.is_empty() {
        return Err("阿里云 OSS 节点缺少 Bucket / Endpoint / AccessKey".into());
    }
    let host = http_base_url(&endpoint)
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .to_string();
    let date = Utc::now().format("%a, %d %b %Y %H:%M:%S GMT").to_string();
    let content_type = "application/xml";
    let canonical_resource = format!("/{}/?cname&comp=add", bucket);
    let string_to_sign = format!("POST\n\n{}\n{}\n{}", content_type, date, canonical_resource);
    let mut mac = <HmacSha1 as Mac>::new_from_slice(format!("{}&", sk).as_bytes()).unwrap();
    mac.update(string_to_sign.as_bytes());
    let signature = B64.encode(mac.finalize().into_bytes());
    let domain = target.trim();
    let body = format!(
        "<BucketCnameConfiguration><Cname><Domain>{}</Domain><CertificateConfiguration><Certificate>{}</Certificate><PrivateKey>{}</PrivateKey><Force>true</Force></CertificateConfiguration></Cname></BucketCnameConfiguration>",
        domain.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;"),
        pems.0,
        pems.1
    );
    let resp = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .post(format!("https://{}.{}/?cname&comp=add", bucket, host))
        .header(reqwest::header::DATE, date)
        .header(reqwest::header::CONTENT_TYPE, content_type)
        .header(
            reqwest::header::AUTHORIZATION,
            format!("OSS {}:{}", ak, signature),
        )
        .body(body)
        .send()
        .await
        .map_err(|e| format!("调用阿里云 OSS 失败: {}", e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "阿里云 OSS 返回 {}: {}",
            status,
            text.chars().take(300).collect::<String>()
        ));
    }
    Ok(())
}

/// 阿里云 ACS3-HMAC-SHA256 签名（函数计算自定义域名使用）。
pub(super) fn acs3_headers(
    method: &str,
    path: &str,
    mut headers: Vec<(String, String)>,
    body: &[u8],
    ak: &str,
    sk: &str,
) -> Vec<(String, String)> {
    let date = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let mut nonce = [0u8; 16];
    let _ = getrandom::getrandom(&mut nonce);
    headers.push(("x-acs-date".into(), date));
    headers.push(("x-acs-signature-nonce".into(), hex(&nonce)));
    headers.push(("x-acs-content-sha256".into(), hex(&Sha256::digest(body))));
    let (canonical_headers, signed_headers) = sorted_canonical_headers(&headers);
    let canonical_request = format!(
        "{}\n{}\n\n{}\n{}\n{}",
        method,
        path,
        canonical_headers,
        signed_headers,
        hex(&Sha256::digest(body))
    );
    let string_to_sign = format!(
        "ACS3-HMAC-SHA256\n{}",
        hex(&Sha256::digest(canonical_request.as_bytes()))
    );
    let signature = hex(&hmac_sha256(sk.as_bytes(), &string_to_sign));
    headers.push((
        "Authorization".into(),
        format!(
            "ACS3-HMAC-SHA256 Credential={},SignedHeaders={},Signature={}",
            ak, signed_headers, signature
        ),
    ));
    headers
}

pub(super) async fn deploy_aliyun_fc(
    node: &DeployNode,
    _cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    if target.trim().is_empty() {
        return Err("阿里云函数计算需要在域名列表关联时填写自定义域名".into());
    }
    let account = cfg(node, "account_id");
    let region = cfg(node, "region");
    let ak = cfg(node, "access_key_id");
    let sk = cfg(node, "access_key_secret");
    if account.is_empty() || region.is_empty() || ak.is_empty() || sk.is_empty() {
        return Err("函数计算节点缺少 Account ID / Region / AccessKey".into());
    }
    let host = format!("{}.{}.fc.aliyuncs.com", account, region);
    let domain = target.trim();
    let path = format!("/2023-03-30/custom-domains/{}", domain);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    // 先读取现有配置，避免 PUT 覆盖路由/TLS 配置
    let headers = acs3_headers(
        "GET",
        &path,
        vec![("host".into(), host.clone())],
        b"",
        &ak,
        &sk,
    );
    let mut get_req = client.get(format!("https://{}{}", host, path));
    for (k, v) in headers {
        get_req = get_req.header(k, v);
    }
    let existing = get_req.send().await.ok();
    let mut cfg_json = match existing {
        Some(r) if r.status().is_success() => r
            .json::<serde_json::Value>()
            .await
            .unwrap_or_else(|_| serde_json::json!({})),
        _ => serde_json::json!({}),
    };
    if !cfg_json.is_object() {
        cfg_json = serde_json::json!({});
    }
    let obj = cfg_json.as_object_mut().unwrap();
    obj.insert("protocol".into(), serde_json::json!("HTTP,HTTPS"));
    obj.insert(
        "certConfig".into(),
        serde_json::json!({
            "certName": cert_name_for(_cert, domain, &_cert.id),
            "certificate": pems.0,
            "privateKey": pems.1
        }),
    );
    let body_text = serde_json::to_string(&cfg_json).map_err(|e| e.to_string())?;
    let headers = acs3_headers(
        "PUT",
        &path,
        vec![
            ("content-type".into(), "application/json".into()),
            ("host".into(), host.clone()),
        ],
        body_text.as_bytes(),
        &ak,
        &sk,
    );
    let mut req = client.put(format!("https://{}{}", host, path));
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let resp = req
        .body(body_text)
        .send()
        .await
        .map_err(|e| format!("调用函数计算失败: {}", e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "函数计算返回 {}: {}",
            status,
            text.chars().take(300).collect::<String>()
        ));
    }
    Ok(())
}
