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
    // 1Panel：Timestamp 为 Unix 秒，Token = md5("1panel" + api_key + timestamp)
    let ts = Utc::now().timestamp().to_string();
    let token = format!(
        "{:x}",
        md5::compute(format!("1panel{}{}", api_key, ts).as_bytes())
    );
    let resp = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .post(format!("{}{}", base, path))
        .header("1Panel-Token", token)
        .header("1Panel-Timestamp", ts)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("调用 1Panel 失败: {}", e))?;
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
    if cfg(node, "scope") == "panel" {
        one_panel_post(
            &base,
            "/settings/ssl/update",
            &api_key,
            serde_json::json!({
                "ssl": "enable",
                "sslType": "import-paste",
                "cert": pems.0,
                "key": pems.1,
                "autoRestart": "true"
            }),
        )
        .await?;
        return Ok(());
    }

    let domain = if target.trim().is_empty() {
        cert.domains.first().cloned().unwrap_or_default()
    } else {
        target.trim().to_string()
    };
    if domain.is_empty() {
        return Err("1Panel 网站 SSL 需要在域名列表关联时填写目标域名".into());
    }
    let description = cert_name_for(cert, &domain, &cert.id);

    // 1) 上传证书（paste）；2) 查回刚上传的 SSL 记录 ID
    one_panel_post(
        &base,
        "/websites/ssl/upload",
        &api_key,
        serde_json::json!({
            "type": "paste",
            "description": description,
            "certificate": pems.0,
            "privateKey": pems.1
        }),
    )
    .await?;
    let ssl_list = one_panel_post(
        &base,
        "/websites/ssl/search",
        &api_key,
        serde_json::json!({ "page": 1, "pageSize": 100, "orderBy": "created_at", "order": "desc" }),
    )
    .await?;
    let ssl_items = ssl_list
        .pointer("/data/items")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    // 优先按 description 匹配；新版可能改写描述，退回最新一条（列表已按创建时间倒序）
    let ssl_id = ssl_items
        .iter()
        .find(|i| i.get("description").and_then(|d| d.as_str()) == Some(description.as_str()))
        .or_else(|| ssl_items.first())
        .and_then(|i| i.get("id"))
        .map(json_id_str)
        .ok_or_else(|| "1Panel 上传证书后未找到 SSL 记录".to_string())?;

    // 3) 按域名找到网站 ID
    let sites = one_panel_post(
        &base,
        "/websites/search",
        &api_key,
        serde_json::json!({ "page": 1, "pageSize": 10, "name": domain }),
    )
    .await?;
    let site_id = sites
        .pointer("/data/items")
        .and_then(|x| x.as_array())
        .and_then(|items| {
            items
                .iter()
                .find(|i| i.get("name").and_then(|n| n.as_str()) == Some(domain.as_str()))
                .or_else(|| items.first())
        })
        .and_then(|i| i.get("id"))
        .map(json_id_str)
        .ok_or_else(|| format!("1Panel 中未找到网站：{}", domain))?;

    // 4) 绑定证书并开启 HTTPS
    let qid: serde_json::Value = site_id
        .parse::<i64>()
        .map(|n| serde_json::json!(n))
        .unwrap_or_else(|_| serde_json::json!(site_id));
    let qssl: serde_json::Value = ssl_id
        .parse::<i64>()
        .map(|n| serde_json::json!(n))
        .unwrap_or_else(|_| serde_json::json!(ssl_id));
    one_panel_post(
        &base,
        &format!("/websites/{}/https", site_id),
        &api_key,
        serde_json::json!({
            "websiteId": qid,
            "enable": true,
            "type": "existed",
            "websiteSSLId": qssl,
            "httpConfig": "HTTPToHTTPS",
            "SSLProtocol": [],
            "algorithm": ""
        }),
    )
    .await?;
    Ok(())
}
