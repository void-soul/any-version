// mihomo controller REST 代理（对齐 clash-party mihomoApi.ts）
use crate::commands::mihomo::config::AppConfig;
use reqwest::Method;
use serde_json::Value;
use std::time::Duration;

pub async fn mihomo_api_raw(
    app: &AppConfig,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<String, String> {
    let url = format!(
        "http://127.0.0.1:{}{}",
        app.controller_port,
        if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        }
    );
    // 必须绕开系统代理：开启系统代理后 127.0.0.1 的控制器请求若被代理会失败
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap_or_default();
    let mut req = client
        .request(method, &url)
        .timeout(Duration::from_secs(15));
    if !app.secret.is_empty() {
        req = req.header("Authorization", format!("Bearer {}", app.secret));
    }
    if let Some(b) = body {
        req = req
            .header("Content-Type", "application/json")
            .json(&b);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("请求核心失败: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("核心返回 {status}: {text}"));
    }
    Ok(text)
}
