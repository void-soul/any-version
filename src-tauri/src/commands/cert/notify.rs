use super::*;
use base64::Engine as _;

// ---------------------------------------------------------------------------
// 通知渠道
// ---------------------------------------------------------------------------

pub(super) async fn notify_send(
    channel: &NotifyChannel,
    title: &str,
    content: &str,
) -> Result<(), String> {
    let cfg_get = |k: &str| {
        channel
            .config
            .get(k)
            .cloned()
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let mut url = cfg_get("url");
    if url.is_empty() {
        return Err(format!("通知渠道「{}」缺少 url", channel.name));
    }

    let body = match channel.channel_type.as_str() {
        "dingtalk" => {
            let secret = cfg_get("secret");
            if !secret.is_empty() {
                // 钉钉加签：timestamp + "\n" + secret 做 HMAC-SHA256，Base64 后再 URL 编码
                let ts = Utc::now().timestamp_millis();
                let sign = B64.encode(hmac_sha256(
                    secret.as_bytes(),
                    &format!("{}\n{}", ts, secret),
                ));
                let sep = if url.contains('?') { '&' } else { '?' };
                url = format!(
                    "{}{}timestamp={}&sign={}",
                    url,
                    sep,
                    ts,
                    percent_encode(&sign)
                );
            }
            serde_json::json!({
                "msgtype": "markdown",
                "markdown": { "title": title, "text": format!("### {}\n\n{}", title, content) }
            })
        }
        "wecom" => serde_json::json!({
            "msgtype": "markdown",
            "markdown": { "content": format!("**{}**\n\n{}", title, content) }
        }),
        "slack" => serde_json::json!({ "text": format!("*{}*\n{}", title, content) }),
        "discord" => serde_json::json!({ "content": format!("**{}**\n{}", title, content) }),
        "teams" => serde_json::json!({
            "@type": "MessageCard",
            "@context": "https://schema.org/extensions",
            "summary": title,
            "themeColor": "0078D7",
            "sections": [{ "activityTitle": title, "text": content }]
        }),
        "webhook" => serde_json::json!({
            "event": "cert_notify",
            "title": title,
            "content": content,
            "timestamp": Utc::now().to_rfc3339()
        }),
        other => return Err(format!("未知通知类型: {}", other)),
    };

    let mut req = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?
        .post(&url)
        .json(&body);
    let token = cfg_get("token");
    if channel.channel_type == "webhook" && !token.is_empty() {
        req = req.bearer_auth(token);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("发送通知失败: {}", e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("通知接口返回 {}: {}", status, text.trim()));
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
        if let Some(code) = v.get("errcode").and_then(|x| x.as_i64()) {
            if code != 0 {
                return Err(format!(
                    "通知接口返回错误: {} - {}",
                    code,
                    v.get("errmsg").and_then(|x| x.as_str()).unwrap_or("")
                ));
            }
        }
    }
    Ok(())
}

pub(super) async fn notify_all(channels: &[NotifyChannel], title: &str, content: &str) {
    for ch in channels.iter().filter(|c| c.enabled) {
        if let Err(e) = notify_send(ch, title, content).await {
            eprintln!("[cert] 通知「{}」发送失败: {}", ch.name, e);
        }
    }
}
