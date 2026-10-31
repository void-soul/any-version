use super::*;

// ---------------------------------------------------------------------------
// 第三方面板：1Panel / 群晖 DSM / 又拍云
// ---------------------------------------------------------------------------

pub(super) fn one_panel_base(raw: &str) -> String {
    let mut u = http_base_url(raw);
    if !u.is_empty() && !u.ends_with("/api/v1") {
        while u.ends_with('/') {
            u.pop();
        }
        u.push_str("/api/v1");
    }
    u
}

pub(super) async fn one_panel_post(
    base: &str,
    path: &str,
    api_key: &str,
    body: serde_json::Value,
) -> Result<serde_json::Value, String> {
    one_panel_request(base, path, api_key, Some(body)).await
}

async fn one_panel_request(
    base: &str,
    path: &str,
    api_key: &str,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    crate::runtime_tasks::require_enabled("cert")?;
    // 1Panel：Timestamp 为 Unix 秒，Token = md5("1panel" + api_key + timestamp)
    let ts = Utc::now().timestamp().to_string();
    let token = format!(
        "{:x}",
        md5::compute(format!("1panel{}{}", api_key, ts).as_bytes())
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("{}{}", base, path);
    let request = match body {
        Some(body) => client.post(url).json(&body),
        None => client.get(url),
    };
    let resp = request
        .header("1Panel-Token", token)
        .header("1Panel-Timestamp", ts)
        .send()
        .await
        .map_err(|e| format!("调用 1Panel 失败: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|_| format!("1Panel 返回非 JSON ({}): {}", status, text.trim()))?;
    if let Some(code) = v.get("code").and_then(|x| x.as_i64()) {
        if code != 200 {
            return Err(format!(
                "1Panel {} 失败: {} - {}",
                path,
                code,
                v.get("message").and_then(|x| x.as_str()).unwrap_or("")
            ));
        }
    }
    if !status.is_success() {
        return Err(format!("1Panel 返回 {}: {}", status, text.trim()));
    }
    Ok(v)
}

pub(super) fn json_id_str(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn record_id(record: &serde_json::Value) -> Result<String, String> {
    let value = record.get("id").unwrap_or(record);
    let id = value
        .as_i64()
        .or_else(|| value.as_str().and_then(|v| v.parse::<i64>().ok()))
        .filter(|id| *id > 0)
        .ok_or("1Panel 返回无效的记录ID，未绑定证书")?;
    Ok(id.to_string())
}

fn fingerprint(pem: &str) -> Result<String, String> {
    use base64::Engine as _;
    let der = pem_first_body_b64(pem).ok_or("1Panel证书详情缺少有效PEM，无法确认上传结果")?;
    let der = B64.decode(der).map_err(|_| "1Panel证书PEM无法解码")?;
    Ok(hex(&Sha256::digest(der)))
}
fn record_pem(record: &serde_json::Value) -> Option<&str> {
    ["pem", "certificate", "cert"].iter().find_map(|key| {
        record
            .get(*key)
            .and_then(|v| v.as_str())
            .filter(|v| v.contains("-----BEGIN CERTIFICATE-----"))
    })
}
fn unique_site(items: &[serde_json::Value], domain: &str) -> Result<String, String> {
    let normalized = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    let matches: Vec<_> = items
        .iter()
        .filter(|item| {
            item.get("primaryDomain")
                .or_else(|| item.get("name"))
                .and_then(|v| v.as_str())
                .is_some_and(|value| {
                    value
                        .trim()
                        .trim_end_matches('.')
                        .eq_ignore_ascii_case(&normalized)
                })
        })
        .collect();
    match matches.as_slice() {
        [site] => record_id(site),
        [] => Err(format!(
            "1Panel 中没有精确匹配的网站：{domain}，未绑定任何证书"
        )),
        _ => Err(format!(
            "1Panel 中有多个精确匹配的网站：{domain}，无法安全选择"
        )),
    }
}
fn verify_ssl(
    record: &serde_json::Value,
    expected_id: &str,
    expected_fingerprint: &str,
) -> Result<(), String> {
    if record_id(record)? != expected_id {
        return Err("1Panel证书详情ID不匹配，未绑定证书".into());
    }
    let pem = record_pem(record).ok_or("1Panel证书详情未返回PEM，无法确认上传结果")?;
    if fingerprint(pem)? != expected_fingerprint {
        return Err("1Panel证书指纹与本次上传不一致，未绑定证书".into());
    }
    Ok(())
}
fn unique_fingerprinted_ssl(
    records: &[serde_json::Value],
    expected: &str,
) -> Result<String, String> {
    let mut matches = Vec::new();
    for record in records {
        let id = record_id(record)?;
        let pem = record_pem(record).ok_or("1Panel证书详情未返回PEM，无法确认上传结果")?;
        if fingerprint(pem)? == expected {
            matches.push(id);
        }
    }
    match matches.as_slice() {
        [id] => Ok(id.clone()),
        [] => Err("1Panel未找到本次上传的精确指纹证书，未绑定任何记录".into()),
        _ => Err("1Panel有多个相同描述和指纹的证书记录，无法安全选择".into()),
    }
}

async fn search_all(
    base: &str,
    path: &str,
    api_key: &str,
    mut body: serde_json::Value,
) -> Result<Vec<serde_json::Value>, String> {
    let mut all = Vec::new();
    for page in 1..=20 {
        body["page"] = serde_json::json!(page);
        body["pageSize"] = serde_json::json!(100);
        let response = one_panel_post(base, path, api_key, body.clone()).await?;
        let items = response
            .pointer("/data/items")
            .and_then(|v| v.as_array())
            .ok_or("1Panel搜索结果结构无效，未选择目标")?;
        let count = items.len();
        all.extend(items.iter().cloned());
        let total = response.pointer("/data/total").and_then(|v| v.as_u64());
        if total.is_some_and(|total| all.len() as u64 >= total) || (total.is_none() && count < 100)
        {
            return Ok(all);
        }
        if count == 0 {
            return Err("1Panel分页结果不完整，无法安全选择目标".into());
        }
    }
    Err("1Panel结果超过安全分页上限，无法确定唯一目标，请缩小面板记录范围".into())
}

pub(super) async fn deploy_one_panel(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    let base = one_panel_base(&cfg(node, "url"));
    let api_key = cfg(node, "api_key");
    if base.is_empty() || api_key.is_empty() {
        return Err("1Panel 节点缺少面板地址或 API 密钥".into());
    }
    cert_deployment::deployment::validate_identity(&pems.0, &pems.1)?;
    if cfg(node, "scope") == "panel" {
        one_panel_post(&base, "/settings/ssl/update", &api_key, serde_json::json!({"ssl":"enable","sslType":"import-paste","cert":pems.0,"key":pems.1,"autoRestart":"true"})).await?;
        return Ok(());
    }
    let domain = if target.trim().is_empty() {
        cert.domains.first().map(|s| s.trim()).unwrap_or("")
    } else {
        target.trim()
    };
    if domain.is_empty() {
        return Err("1Panel 网站 SSL 需要填写目标域名".into());
    }
    // Resolve the exact site before uploading, so ambiguous searches have no deployment side effects.
    let sites = search_all(
        &base,
        "/websites/search",
        &api_key,
        serde_json::json!({"name":domain}),
    )
    .await?;
    let site_id = unique_site(&sites, domain)?;
    let description = cert_name_for(cert, domain, &cert.id);
    let expected = fingerprint(&pems.0)?;
    let uploaded = one_panel_post(&base, "/websites/ssl/upload", &api_key, serde_json::json!({"type":"paste","description":description,"certificate":pems.0,"privateKey":pems.1})).await?;
    let direct_id = uploaded.get("data").and_then(|v| record_id(v).ok());
    let ssl_id = if let Some(id) = direct_id {
        let detail =
            one_panel_request(&base, &format!("/websites/ssl/{id}"), &api_key, None).await?;
        verify_ssl(
            detail.get("data").ok_or("1Panel缺少证书详情")?,
            &id,
            &expected,
        )?;
        id
    } else {
        let items = search_all(
            &base,
            "/websites/ssl/search",
            &api_key,
            serde_json::json!({"orderBy":"created_at","order":"desc"}),
        )
        .await?;
        let mut candidates = Vec::new();
        for item in items.iter().filter(|item| {
            item.get("description").and_then(|v| v.as_str()) == Some(description.as_str())
        }) {
            let id = record_id(item)?;
            let detail = if record_pem(item).is_some() {
                item.clone()
            } else {
                one_panel_request(&base, &format!("/websites/ssl/{id}"), &api_key, None)
                    .await?
                    .get("data")
                    .cloned()
                    .ok_or("1Panel缺少证书详情")?
            };
            if record_id(&detail)? != id {
                return Err("1Panel证书查询ID不一致，已停止绑定".into());
            }
            candidates.push(detail);
        }
        unique_fingerprinted_ssl(&candidates, &expected)?
    };
    one_panel_post(&base, &format!("/websites/{site_id}/https"), &api_key, serde_json::json!({"websiteId":site_id.parse::<i64>().map_err(|e| e.to_string())?,"enable":true,"type":"existed","websiteSSLId":ssl_id.parse::<i64>().map_err(|e| e.to_string())?,"httpConfig":"HTTPToHTTPS","SSLProtocol":[],"algorithm":""})).await?;
    Ok(())
}

#[cfg(test)]
mod tests;
