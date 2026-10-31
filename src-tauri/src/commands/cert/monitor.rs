use super::*;
use base64::Engine as _;

// ---------------------------------------------------------------------------
// 证书监控：TLS 到期检查
// ---------------------------------------------------------------------------

/// 把 DER 编码的证书转成 PEM（复用 parse_cert_detail 的解析入口）。
pub(super) fn pem_from_der(der: &[u8]) -> String {
    let b64 = B64.encode(der);
    let mut pem = String::from("-----BEGIN CERTIFICATE-----\n");
    for chunk in b64.as_bytes().chunks(64) {
        pem.push_str(std::str::from_utf8(chunk).unwrap_or_default());
        pem.push('\n');
    }
    pem.push_str("-----END CERTIFICATE-----\n");
    pem
}

/// 解析 `example.com:8443` / `https://example.com` / `[::1]:443` 形式的监控目标。
pub(super) fn split_host_port(raw: &str, default_port: u16) -> (String, u16) {
    let s = raw
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');
    if let Some(rest) = s.strip_prefix('[') {
        if let Some(idx) = rest.find(']') {
            let host = rest[..idx].to_string();
            let port = rest[idx + 1..]
                .trim_start_matches(':')
                .parse::<u16>()
                .unwrap_or(default_port);
            return (host, port);
        }
    }
    if let Some((h, p)) = s.rsplit_once(':') {
        if let Ok(port) = p.parse::<u16>() {
            return (h.to_string(), port);
        }
    }
    (s.to_string(), default_port)
}

pub(super) struct TlsProbe {
    pub(super) subject: String,
    pub(super) issuer: String,
    pub(super) not_after: Option<String>,
}

/// 与目标做 TLS 握手并读取对端证书。
/// 不校验证书链与主机名：监控只关心到期时间，自签/过期证书同样应被检测到。
pub(super) async fn probe_tls(host: &str, port: u16) -> Result<TlsProbe, String> {
    let addr = if host.contains(':') {
        format!("[{}]:{}", host, port)
    } else {
        format!("{}:{}", host, port)
    };
    let tcp = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio::net::TcpStream::connect(&addr),
    )
    .await
    .map_err(|_| format!("连接 {} 超时", addr))?
    .map_err(|e| format!("连接 {} 失败: {}", addr, e))?;

    let builder = native_tls::TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .danger_accept_invalid_hostnames(true)
        .build()
        .map_err(|e| format!("初始化 TLS 失败: {}", e))?;
    let connector = tokio_native_tls::TlsConnector::from(builder);
    let stream = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        connector.connect(host, tcp),
    )
    .await
    .map_err(|_| format!("{} TLS 握手超时", addr))?
    .map_err(|e| format!("{} TLS 握手失败: {}", addr, e))?;

    let cert = stream
        .get_ref()
        .peer_certificate()
        .map_err(|e| format!("读取对端证书失败: {}", e))?
        .ok_or_else(|| "对端未提供证书".to_string())?;
    let der = cert.to_der().map_err(|e| format!("证书编码失败: {}", e))?;
    let detail = parse_cert_detail(&pem_from_der(&der));
    Ok(TlsProbe {
        subject: detail.subject.unwrap_or_default(),
        issuer: detail.issuer.unwrap_or_default(),
        not_after: detail.not_after,
    })
}

/// 剩余天数命中的提醒阈值：返回最小的 `>= days_left` 阈值。
/// 已过期返回 Some(0)；高于所有阈值返回 None（不提醒）。
pub(super) fn notify_bucket(days_left: i64, thresholds: &[i64]) -> Option<i64> {
    if days_left <= 0 {
        return Some(0);
    }
    let mut sorted: Vec<i64> = thresholds.iter().copied().filter(|d| *d > 0).collect();
    sorted.sort_unstable();
    sorted.into_iter().find(|d| days_left <= *d)
}

pub(super) async fn check_one_monitor(
    m: &mut MonitorTarget,
    channels: &[NotifyChannel],
    notify: bool,
) {
    if !m.enabled {
        return;
    }
    let (host, port) = split_host_port(&m.host, if m.port == 0 { 443 } else { m.port });
    if host.is_empty() {
        m.last_error = Some("监控目标缺少域名".to_string());
        return;
    }
    m.last_check_at = Some(Utc::now().to_rfc3339());

    match probe_tls(&host, port).await {
        Ok(p) => {
            m.last_error = None;
            m.subject = p.subject;
            m.issuer = p.issuer;
            m.not_after = p.not_after.clone();
            let days_left = m
                .not_after
                .as_ref()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|d| (d.with_timezone(&Utc) - Utc::now()).num_days());
            m.days_left = days_left;

            let bucket = days_left.and_then(|d| notify_bucket(d, &m.notify_days));
            if bucket.is_none() {
                // 续期成功（剩余天数回到阈值之外）后重置，便于下一次到期重新提醒
                m.last_notify_bucket = None;
                return;
            }
            if notify && bucket != m.last_notify_bucket {
                let na = m.not_after.clone().unwrap_or_default();
                let msg = match days_left {
                    Some(d) if d >= 0 => format!(
                        "{}:{} 的证书将在 {} 天后到期（{}），请及时续期。",
                        host, port, d, na
                    ),
                    Some(d) => format!("{}:{} 的证书已过期 {} 天（{}）。", host, port, -d, na),
                    None => format!("{}:{} 无法解析证书有效期（{}）。", host, port, na),
                };
                notify_all(channels, "证书到期提醒", &msg).await;
                m.last_notify_bucket = bucket;
                m.last_notified_at = Some(Utc::now().to_rfc3339());
            }
        }
        Err(e) => {
            let first = m.last_error.is_none();
            m.last_error = Some(e.clone());
            if notify && first {
                notify_all(
                    channels,
                    "证书监控失败",
                    &format!("{}:{} 检测失败：{}", host, port, e),
                )
                .await;
            }
        }
    }
}

pub(super) async fn run_monitor_checks(notify: bool) -> Result<Vec<MonitorTarget>, String> {
    let mut monitors = load_json_checked::<MonitorTarget>("monitors.json")?;
    let expected = monitors.clone();
    let channels = load_json_checked::<NotifyChannel>("notify_channels.json")?;
    for m in monitors.iter_mut() {
        check_one_monitor(m, &channels, notify).await;
    }
    merge_runtime("monitors.json", &expected, &monitors, MONITOR_RUNTIME)
}
