//! 网络信息：网卡枚举、出口 IP 查询、延迟测试
//! 对齐 clash-party src/main/utils/ipc.ts 的 getInterfaces / fetchIPInfo / measureLatency

use crate::commands::hidden_cmd::hidden_cmd;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

fn client(timeout_ms: u64) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .danger_accept_invalid_certs(false)
        .no_proxy()
        .build()
        .unwrap_or_default()
}

fn sys_client(timeout_ms: u64) -> reqwest::Client {
    // 走系统代理（用于"经代理出口 IP"）
    reqwest::Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .build()
        .unwrap_or_default()
}

/// 通过 HTTP 获取 JSON（网络信息页各 IP 服务）
#[tauri::command]
pub async fn mihomo_fetch_ip_info(
    url: String,
    use_proxy: Option<bool>,
    timeout_ms: Option<u64>,
) -> Result<Value, String> {
    let t = timeout_ms.unwrap_or(10000);
    let c = if use_proxy.unwrap_or(true) {
        sys_client(t)
    } else {
        client(t)
    };
    let resp = c
        .get(&url)
        .header("User-Agent", "Mozilla/5.0 any-version")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("HTTP {status}: {}", text.chars().take(200).collect::<String>()));
    }
    match serde_json::from_str::<Value>(&text) {
        Ok(v) => Ok(v),
        Err(_) => Ok(json!({ "raw": text })),
    }
}

/// 测量到某 URL 的延迟（毫秒），失败返回 null
#[tauri::command]
pub async fn mihomo_measure_latency(
    url: String,
    use_proxy: Option<bool>,
    timeout_ms: Option<u64>,
) -> Option<u64> {
    let t = timeout_ms.unwrap_or(5000);
    let c = if use_proxy.unwrap_or(true) {
        sys_client(t)
    } else {
        client(t)
    };
    let t0 = Instant::now();
    match c
        .get(&url)
        .header("User-Agent", "Mozilla/5.0 any-version")
        .send()
        .await
    {
        Ok(r) => {
            let _ = r.bytes().await;
            Some(t0.elapsed().as_millis() as u64)
        }
        Err(_) => None,
    }
}

fn ps_json(script: &str) -> Option<Value> {
    let out = hidden_cmd("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).to_string();
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    serde_json::from_str::<Value>(s).ok()
}

/// 一次 PowerShell 进程取回网卡枚举所需的全部数据
/// （此前拆成 4 次调用，每次都要重新加载 NetAdapter/DnsClient 模块，首次进入页面要 30~60s）
const IFACE_SCRIPT: &str = r#"$ErrorActionPreference='SilentlyContinue';
[Console]::OutputEncoding=[System.Text.Encoding]::UTF8;
$PSDefaultParameterValues['Out-File:Encoding']='utf8';
$OutputEncoding=[System.Text.Encoding]::UTF8;
$o=[ordered]@{
adapters=@(Get-NetAdapter | Select-Object Name,InterfaceDescription,InterfaceIndex,Status,MacAddress,LinkSpeed);
addrs=@(Get-NetIPAddress | Select-Object InterfaceIndex,IPAddress,AddressFamily,PrefixLength);
routes=@(Get-NetRoute -DestinationPrefix '0.0.0.0/0' | Select-Object InterfaceIndex,NextHop,RouteMetric);
dns=@(Get-DnsClientServerAddress -AddressFamily IPv4 | Select-Object InterfaceIndex,ServerAddresses)
};
$o | ConvertTo-Json -Compress -Depth 4"#;

/// 网卡信息缓存，默认 8 秒内复用，避免频繁刷新时反复拉起 PowerShell
static IFACE_CACHE: std::sync::Mutex<Option<(Instant, Value)>> = std::sync::Mutex::new(None);
const IFACE_TTL: Duration = Duration::from_secs(8);

fn as_array(v: Option<Value>) -> Vec<Value> {
    match v {
        Some(Value::Array(a)) => a,
        Some(Value::Null) | None => vec![],
        Some(other) => vec![other],
    }
}

/// 枚举本机网卡（名称 / 状态 / MAC / IPv4 / IPv6 / 网关 / DNS）
///
/// 异步命令：真正的枚举工作放在阻塞线程池，不再卡住 IPC/界面；
/// 同时带 8 秒结果缓存，可用 force = true 强制刷新。
#[tauri::command]
pub async fn mihomo_get_interfaces(force: Option<bool>) -> Value {
    if !force.unwrap_or(false) {
        if let Ok(guard) = IFACE_CACHE.lock() {
            if let Some((at, v)) = guard.as_ref() {
                if at.elapsed() < IFACE_TTL {
                    return v.clone();
                }
            }
        }
    }
    let v = tokio::task::spawn_blocking(collect_interfaces)
        .await
        .unwrap_or_else(|_| json!({ "items": [] }));
    if let Ok(mut guard) = IFACE_CACHE.lock() {
        *guard = Some((Instant::now(), v.clone()));
    }
    v
}

fn collect_interfaces() -> Value {
    let root = ps_json(IFACE_SCRIPT).unwrap_or(Value::Null);
    let pick = |k: &str| root.get(k).cloned();
    let adapters = as_array(pick("adapters"));
    let addrs = as_array(pick("addrs"));
    let routes = as_array(pick("routes"));
    let dns = as_array(pick("dns"));

    let idx_of = |v: &Value| v.get("InterfaceIndex").and_then(|x| x.as_i64()).unwrap_or(-1);
    let mut items = Vec::new();
    for a in &adapters {
        let idx = idx_of(a);
        let mut v4 = Vec::new();
        let mut v6 = Vec::new();
        for ad in &addrs {
            if idx_of(ad) != idx {
                continue;
            }
            let ip = ad.get("IPAddress").and_then(|x| x.as_str()).unwrap_or("");
            let fam = ad.get("AddressFamily");
            let is_v6 = ip.contains(':')
                || fam.and_then(|f| f.as_i64()) == Some(23)
                || fam.and_then(|f| f.as_str()) == Some("IPv6");
            let entry = json!({
                "address": ip,
                "prefix": ad.get("PrefixLength").and_then(|x| x.as_i64()),
            });
            if is_v6 {
                v6.push(entry);
            } else {
                v4.push(entry);
            }
        }
        let gateway = routes
            .iter()
            .find(|r| idx_of(r) == idx)
            .and_then(|r| r.get("NextHop").and_then(|x| x.as_str()))
            .unwrap_or("")
            .to_string();
        let dns_list: Vec<String> = dns
            .iter()
            .find(|d| idx_of(d) == idx)
            .and_then(|d| d.get("ServerAddresses").cloned())
            .map(|s| match s {
                Value::Array(a) => a
                    .into_iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect(),
                Value::String(s) => vec![s],
                _ => vec![],
            })
            .unwrap_or_default();
        items.push(json!({
            "name": a.get("Name").and_then(|x| x.as_str()).unwrap_or(""),
            "description": a.get("InterfaceDescription").and_then(|x| x.as_str()).unwrap_or(""),
            "index": idx,
            "status": a.get("Status").and_then(|x| x.as_str()).unwrap_or(""),
            "mac": a.get("MacAddress").and_then(|x| x.as_str()).unwrap_or(""),
            "speed": a.get("LinkSpeed").and_then(|x| x.as_str()).unwrap_or(""),
            "ipv4": v4,
            "ipv6": v6,
            "gateway": gateway,
            "dns": dns_list,
        }));
    }
    json!({ "items": items })
}
