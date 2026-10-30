use super::*;

/// 是否为 URL / shell 协议目标（http(s)/ftp/mailto/file 等）。
/// 避免把 `https://example.com` 误判为 `.com` 可执行程序。
pub(super) fn looks_like_url(file: &str) -> bool {
    let lower = file.trim().to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("ftp://")
        || lower.starts_with("ftps://")
        || lower.starts_with("mailto:")
        || lower.starts_with("file://")
}

/// 抓取网页标题和高清 Favicon
pub async fn fetch_url_metadata(url_str: &str) -> Result<UrlMetadata, String> {
    fetch_url_metadata_with_timeout(url_str, std::time::Duration::from_secs(6)).await
}

/// 抓取网页标题和高清 Favicon（可自定义超时，用于检测时使用更短超时）
pub async fn fetch_url_metadata_with_timeout(
    url_str: &str,
    timeout: std::time::Duration,
) -> Result<UrlMetadata, String> {
    let parsed_url = reqwest::Url::parse(url_str).map_err(|e| format!("无效的网址: {}", e))?;
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .build()
        .map_err(|e| e.to_string())?;

    let res = client
        .get(parsed_url.clone())
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;
    let body = res.text().await.unwrap_or_default();

    // 解析 Title
    let title = if let Some(start) = body.to_lowercase().find("<title") {
        if let Some(tag_end) = body[start..].find('>') {
            let content_start = start + tag_end + 1;
            if let Some(end) = body[content_start..].to_lowercase().find("</title>") {
                body[content_start..content_start + end].trim().to_string()
            } else {
                parsed_url.host_str().unwrap_or(url_str).to_string()
            }
        } else {
            parsed_url.host_str().unwrap_or(url_str).to_string()
        }
    } else {
        parsed_url.host_str().unwrap_or(url_str).to_string()
    };

    // 尝试获取 Favicon
    let mut icon_url = None;
    // 1. 从 link rel="icon" 或 rel="shortcut icon" 查找
    let lower_body = body.to_lowercase();
    if let Some(link_idx) = lower_body
        .find("rel=\"icon\"")
        .or_else(|| lower_body.find("rel=\"shortcut icon\""))
    {
        let tag_start = body[..link_idx].rfind('<').unwrap_or(0);
        let tag_end = body[link_idx..]
            .find('>')
            .map(|i| link_idx + i)
            .unwrap_or(body.len());
        let tag_str = &body[tag_start..tag_end];
        if let Some(href_idx) = tag_str.to_lowercase().find("href=\"") {
            let val_start = href_idx + 6;
            if let Some(quote_end) = tag_str[val_start..].find('"') {
                let href_val = &tag_str[val_start..val_start + quote_end];
                if let Ok(joined) = parsed_url.join(href_val) {
                    icon_url = Some(joined);
                }
            }
        }
    }

    if icon_url.is_none() {
        if let Ok(default_favicon) = parsed_url.join("/favicon.ico") {
            icon_url = Some(default_favicon);
        }
    }

    let mut icon_base64 = None;
    if let Some(fav_url) = icon_url {
        if let Ok(fav_res) = client.get(fav_url).send().await {
            if fav_res.status().is_success() {
                if let Ok(bytes) = fav_res.bytes().await {
                    if !bytes.is_empty() {
                        let mime = "image/x-icon";
                        let encoded = general_purpose::STANDARD.encode(&bytes);
                        icon_base64 = Some(format!("data:{};base64,{}", mime, encoded));
                    }
                }
            }
        }
    }

    Ok(UrlMetadata {
        title,
        icon: icon_base64,
        url: url_str.to_string(),
    })
}
