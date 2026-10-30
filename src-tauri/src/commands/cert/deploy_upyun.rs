use super::*;

// --- 又拍云（控制台会话） ---

pub(super) async fn upyun_login(node: &DeployNode) -> Result<String, String> {
    let username = cfg(node, "username");
    let password = cfg(node, "password");
    if username.is_empty() || password.is_empty() {
        return Err("又拍云节点缺少账号 / 密码".into());
    }
    let resp = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?
        .post("https://console.upyun.com/accounts/signin/")
        .json(&serde_json::json!({ "username": username, "password": password }))
        .send()
        .await
        .map_err(|e| format!("登录又拍云失败: {}", e))?;
    let status = resp.status();
    let cookies: Vec<String> = resp
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|v| v.split(';').next())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .collect();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("登录又拍云失败 ({}): {}", status, text.trim()));
    }
    if cookies.is_empty() {
        return Err("又拍云登录成功但未返回会话 Cookie（可能需要二次验证）".to_string());
    }
    Ok(cookies.join("; "))
}

pub(super) async fn upyun_upload_cert(
    pems: &(String, String, String),
    cookie: &str,
) -> Result<String, String> {
    let resp = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .post("https://console.upyun.com/api/https/certificate/")
        .header(reqwest::header::COOKIE, cookie)
        .json(&serde_json::json!({
            "certificate": pems.0,
            "private_key": pems.1
        }))
        .send()
        .await
        .map_err(|e| format!("上传又拍云证书失败: {}", e))?;
    let text = resp.text().await.unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|_| {
        format!(
            "又拍云返回非 JSON: {}",
            text.chars().take(200).collect::<String>()
        )
    })?;
    v.pointer("/data/result/certificate_id")
        .map(json_id_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            format!(
                "又拍云上传证书失败: {}",
                text.chars().take(300).collect::<String>()
            )
        })
}

pub(super) async fn deploy_upyun(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    let domain = if target.trim().is_empty() {
        cert.domains.first().cloned().unwrap_or_default()
    } else {
        target.trim().to_string()
    };
    if domain.is_empty() {
        return Err("又拍云部署需要在域名列表关联时填写目标域名".into());
    }
    let cookie = upyun_login(node).await?;
    let cert_id = upyun_upload_cert(pems, &cookie).await?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    // 未开 HTTPS 的域名绑定证书
    let bind = client
        .post("https://console.upyun.com/api/https/certificate/manager")
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({
            "certificate_id": cert_id,
            "domain": domain,
            "https": true,
            "force_https": true
        }))
        .send()
        .await
        .map_err(|e| format!("绑定又拍云证书失败: {}", e))?;
    let status = bind.status();
    let text = bind.text().await.unwrap_or_default();
    if status.is_success() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            if v.get("code").and_then(|x| x.as_i64()).unwrap_or(0) == 0 {
                return Ok(());
            }
        } else {
            return Ok(());
        }
    }
    // 已启用 HTTPS 的域名使用换证接口重试
    let migrate = client
        .post("https://console.upyun.com/api/https/migrate/domain")
        .header(reqwest::header::COOKIE, &cookie)
        .json(&serde_json::json!({ "crt_id": cert_id, "domain_name": domain }))
        .send()
        .await
        .map_err(|e| format!("又拍云换证失败: {}", e))?;
    let mstatus = migrate.status();
    let mtext = migrate.text().await.unwrap_or_default();
    if !mstatus.is_success() {
        return Err(format!(
            "又拍云绑定证书失败 ({}): {}",
            mstatus,
            mtext.chars().take(300).collect::<String>()
        ));
    }
    Ok(())
}
