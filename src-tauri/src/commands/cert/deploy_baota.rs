use super::*;

// --- 宝塔面板：网站 SSL / 面板 SSL ---

/// 面板地址归一化：补全 scheme、去掉结尾斜杠。
pub(super) fn http_base_url(raw: &str) -> String {
    let mut u = raw.trim().trim_end_matches('/').to_string();
    if !u.is_empty() && !u.starts_with("http://") && !u.starts_with("https://") {
        u = format!("http://{}", u);
    }
    u
}

pub(super) fn baota_sign(api_sk: &str) -> (String, String) {
    // 宝塔 API：request_token = md5(request_time + md5(api_sk))
    let time = Utc::now().timestamp().to_string();
    let sk_md5 = format!("{:x}", md5::compute(api_sk.as_bytes()));
    let token = format!(
        "{:x}",
        md5::compute(format!("{}{}", time, sk_md5).as_bytes())
    );
    (time, token)
}

pub(super) async fn baota_post(
    base: &str,
    path_query: &str,
    mut form: Vec<(String, String)>,
    api_sk: &str,
) -> Result<serde_json::Value, String> {
    let (time, token) = baota_sign(api_sk);
    form.push(("request_time".into(), time));
    form.push(("request_token".into(), token));
    let resp = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .post(format!("{}{}", base, path_query))
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("调用宝塔面板失败: {}", e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    serde_json::from_str(&text).map_err(|_| {
        format!(
            "宝塔返回非 JSON ({}): {}",
            status,
            text.chars().take(300).collect::<String>().trim()
        )
    })
}

pub(super) fn baota_check(v: &serde_json::Value, action: &str) -> Result<(), String> {
    match v.get("status").and_then(|x| x.as_bool()) {
        Some(true) => Ok(()),
        Some(false) => Err(format!(
            "宝塔 {} 失败: {}",
            action,
            v.get("msg").and_then(|x| x.as_str()).unwrap_or("未知错误")
        )),
        // 部分接口不返回 status（直接返回数据），视为成功
        None => Ok(()),
    }
}

pub(super) fn baota_api_sk(node: &DeployNode) -> String {
    let sk = cfg(node, "api_sk");
    if sk.is_empty() {
        cfg(node, "api_key")
    } else {
        sk
    }
}

pub(super) async fn deploy_baota(
    node: &DeployNode,
    cert: &Certificate,
    pems: &(String, String, String),
    target: &str,
) -> Result<(), String> {
    let base = http_base_url(&cfg(node, "url"));
    let api_sk = baota_api_sk(node);
    if base.is_empty() || api_sk.is_empty() {
        return Err("宝塔节点缺少面板地址或 API 密钥".into());
    }
    if cfg(node, "scope") == "panel" {
        // 面板 SSL：不同版本字段名不一致，同时提交常见别名（多余字段会被忽略）
        let form = vec![
            ("key".into(), pems.1.clone()),
            ("csr".into(), pems.0.clone()),
            ("cert".into(), pems.0.clone()),
            ("privateKey".into(), pems.1.clone()),
            ("publicKey".into(), pems.0.clone()),
        ];
        let v = baota_post(&base, "/config?action=SetPanelSSL", form, &api_sk).await?;
        return baota_check(&v, "面板 SSL");
    }
    // 网站 SSL：站点名取域名列表关联时填写的目标域名
    let site = if target.trim().is_empty() {
        cert.domains.first().cloned().unwrap_or_default()
    } else {
        target.trim().to_string()
    };
    if site.is_empty() {
        return Err("宝塔网站 SSL 需要在域名列表关联时填写站点域名".into());
    }
    let form = vec![
        ("siteName".into(), site.clone()),
        ("key".into(), pems.1.clone()),
        ("csr".into(), pems.0.clone()),
        ("type".into(), "1".into()),
    ];
    let v = baota_post(&base, "/site?action=SetSSL", form, &api_sk).await?;
    baota_check(&v, "网站 SSL")?;
    // 部分版本需要再写入一次站点 SSL 配置才生效；失败不影响已保存的证书
    let _ = baota_post(
        &base,
        "/site?action=SetSSLConf",
        vec![("siteName".into(), site)],
        &api_sk,
    )
    .await;
    Ok(())
}

pub(super) async fn baota_test(node: &DeployNode) -> Result<String, String> {
    let base = http_base_url(&cfg(node, "url"));
    let api_sk = baota_api_sk(node);
    if base.is_empty() || api_sk.is_empty() {
        return Err("宝塔节点缺少面板地址或 API 密钥".into());
    }
    let v = baota_post(&base, "/site?action=GetSiteList", vec![], &api_sk).await?;
    baota_check(&v, "API 鉴权")?;
    Ok("宝塔 API 鉴权正常".to_string())
}
