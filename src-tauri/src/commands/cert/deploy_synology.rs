use super::*;

// --- 群晖 DSM ---

pub(super) fn synology_base(node: &DeployNode) -> String {
    http_base_url(&cfg(node, "url"))
}

pub(super) async fn synology_login(node: &DeployNode) -> Result<(String, String, String), String> {
    let base = synology_base(node);
    let account = cfg(node, "username");
    let passwd = cfg(node, "password");
    if base.is_empty() || account.is_empty() || passwd.is_empty() {
        return Err("群晖节点缺少地址 / 账号 / 密码".into());
    }
    let mut query = vec![
        ("api", "SYNO.API.Auth".to_string()),
        ("version", "6".to_string()),
        ("method", "login".to_string()),
        ("format", "sid".to_string()),
        ("account", account),
        ("passwd", passwd),
        ("session", "Certificate".to_string()),
        ("enable_syno_token", "yes".to_string()),
    ];
    let otp = cfg(node, "otp_code");
    if !otp.is_empty() {
        query.push(("otp_code", otp));
    }
    let resp = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?
        .get(format!("{}/webapi/auth.cgi", base))
        .query(&query)
        .send()
        .await
        .map_err(|e| format!("连接群晖失败: {}", e))?;
    let text = resp.text().await.unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|_| {
        format!(
            "群晖返回非 JSON: {}",
            text.chars().take(200).collect::<String>()
        )
    })?;
    if v.get("success").and_then(|x| x.as_bool()) != Some(true) {
        let code = v
            .pointer("/error/code")
            .and_then(|x| x.as_i64())
            .unwrap_or(-1);
        return Err(format!("群晖登录失败（错误码 {}）", code));
    }
    let sid = v
        .pointer("/data/sid")
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string();
    if sid.is_empty() {
        return Err("群晖登录成功但未返回 sid".to_string());
    }
    let token = v
        .pointer("/data/synotoken")
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string();
    Ok((base, sid, token))
}

pub(super) async fn synology_cert_id(
    base: &str,
    sid: &str,
    node: &DeployNode,
) -> Result<String, String> {
    let configured = cfg(node, "cert_id");
    if !configured.is_empty() {
        return Ok(configured);
    }
    let resp = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?
        .post(format!("{}/webapi/entry.cgi", base))
        .form(&[
            ("api", "SYNO.Core.Certificate.CRT"),
            ("method", "list"),
            ("version", "1"),
            ("_sid", sid),
        ])
        .send()
        .await
        .map_err(|e| format!("获取群晖证书列表失败: {}", e))?;
    let text = resp.text().await.unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    let certs = v
        .pointer("/data/certificates")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    // 优先默认证书（系统正在使用的那张），否则第一张
    certs
        .iter()
        .find(|c| c.get("is_default").and_then(|x| x.as_bool()) == Some(true))
        .or_else(|| certs.first())
        .and_then(|c| c.get("id"))
        .map(json_id_str)
        .ok_or_else(|| "群晖中未找到可用的证书，请在节点配置里填写证书 ID".to_string())
}

pub(super) async fn deploy_synology(
    node: &DeployNode,
    _cert: &Certificate,
    pems: &(String, String, String),
    _target: &str,
) -> Result<(), String> {
    let (base, sid, token) = synology_login(node).await?;
    let cert_id = synology_cert_id(&base, &sid, node).await?;

    // cert 只放叶子证书，中间链放 inter_cert
    let leaf = pem_first_block(&pems.0);
    let chain = pems.0.replacen(leaf.as_str(), "", 1);
    let mut form = reqwest::multipart::Form::new()
        .text("id", cert_id)
        .text("desc", cert_name_for(_cert, "", &_cert.id))
        .text("key", pems.1.clone())
        .text("cert", leaf)
        .text("inter_cert", chain);
    if cfg(node, "as_default") == "true" {
        form = form.text("as_default", "true");
    }
    let resp = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?
        .post(format!("{}/webapi/entry.cgi", base))
        .query(&[
            ("api", "SYNO.Core.Certificate"),
            ("method", "import"),
            ("version", "1"),
            ("_sid", sid.as_str()),
            ("SynoToken", token.as_str()),
        ])
        .header("X-SYNO-TOKEN", token)
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("导入群晖证书失败: {}", e))?;
    let text = resp.text().await.unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|_| {
        format!(
            "群晖返回非 JSON: {}",
            text.chars().take(200).collect::<String>()
        )
    })?;
    if v.get("success").and_then(|x| x.as_bool()) != Some(true) {
        let code = v
            .pointer("/error/code")
            .and_then(|x| x.as_i64())
            .unwrap_or(-1);
        return Err(format!("群晖导入证书失败（错误码 {}）", code));
    }
    Ok(())
}
