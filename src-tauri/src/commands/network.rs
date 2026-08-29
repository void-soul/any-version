//! 本地网络模块（Q-网络）：全量连接列表、网卡流量、IP 归属地查询、ping。
//!
//! 端口排查（port.rs）专注单个端口占用；本模块提供宏观视角：
//! - 所有 TCP/UDP 连接（含进程名映射）
//! - 网卡收发字节数（前端轮询计算速率）
//! - IP 归属地查询（免费公共 API，中文优先）
//! - ping 域名/IP（Windows 内置 ping，解析中文/英文输出）

use regex::Regex;
use serde::Serialize;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// 网络连接
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone, Debug)]
pub struct NetConnection {
    pub proto: String, // TCP / TCPv6 / UDP / UDPv6
    pub local: String, // address:port
    pub remote: String,
    pub state: String, // LISTENING / ESTABLISHED / ... 或 "-"
    pub pid: String,
    pub process: String,
}

/// 一次 tasklist 全量输出，建立 PID → 进程名 映射（避免逐 PID 查询）。
fn build_pid_map() -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let Ok(out) = super::hidden_cmd::hidden_cmd("cmd")
        .args(&["/c", "chcp 65001 >nul & tasklist /fo csv /nh"])
        .output()
    {
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            let l = line.trim();
            if l.is_empty() {
                continue;
            }
            let parts: Vec<&str> = l.split(',').collect();
            if parts.len() < 2 {
                continue;
            }
            let name = parts[0].trim_matches('"').trim().to_string();
            let pid = parts[1].trim_matches('"').trim().to_string();
            if !pid.is_empty() {
                map.insert(pid, name);
            }
        }
    }
    map
}

/// 全量解析 netstat 输出。
fn parse_netstat(text: &str, pid_map: &HashMap<String, String>) -> Vec<NetConnection> {
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let up = t.to_uppercase();
        // IPv6 地址形如 [::1]:port（含 `]:`），IPv4 无括号
        let is_v6 = t.contains("]:");
        let proto_base = if up.starts_with("TCP") { "TCP" } else if up.starts_with("UDP") { "UDP" } else { continue };
        let proto = if is_v6 {
            format!("{}v6", proto_base)
        } else {
            proto_base.to_string()
        };
        let fields: Vec<&str> = t.split_whitespace().collect();
        let (local, remote, state, pid) = if proto_base == "TCP" {
            if fields.len() < 5 {
                continue;
            }
            (fields[1].to_string(), fields[2].to_string(), fields[3].to_string(), fields[4].to_string())
        } else {
            if fields.len() < 4 {
                continue;
            }
            (fields[1].to_string(), fields[2].to_string(), "-".to_string(), fields[3].to_string())
        };
        if local == "*" {
            continue;
        }
        let process = pid_map.get(&pid).cloned().unwrap_or_default();
        out.push(NetConnection { proto, local, remote, state, pid, process });
    }
    out
}

/// 列出所有 TCP/UDP 网络连接（含进程名）。
#[tauri::command]
pub fn net_connections() -> Result<Vec<NetConnection>, String> {
    let output = super::hidden_cmd::hidden_cmd("cmd")
        .args(&["/c", "chcp 65001 >nul & netstat -ano"])
        .output()
        .map_err(|e| format!("执行 netstat 失败: {}", e))?;
    let text = String::from_utf8_lossy(&output.stdout);
    let pid_map = build_pid_map();
    let mut conns = parse_netstat(&text, &pid_map);
    // 排序：监听优先，然后按本地地址
    conns.sort_by(|a, b| {
        b.state.cmp(&a.state).then_with(|| a.local.cmp(&b.local))
    });
    // 限制返回数量，避免超大连接数拖垮 UI
    if conns.len() > 5000 {
        conns.truncate(5000);
    }
    Ok(conns)
}

// ---------------------------------------------------------------------------
// 网卡流量
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone, Debug)]
pub struct IfaceTraffic {
    pub name: String,
    pub received_bytes: u64,
    pub sent_bytes: u64,
}

/// 网卡收发字节数（累计值；前端轮询计算速率）。
#[tauri::command]
pub fn net_iface_traffic() -> Result<Vec<IfaceTraffic>, String> {
    let script = "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; Get-NetAdapterStatistics | Where-Object { $_.Name -notmatch 'vEthernet|Loopback' } | Select-Object Name,ReceivedBytes,SentBytes | ConvertTo-Json -Compress";
    let output = super::hidden_cmd::hidden_cmd("cmd")
        .args(&["/c", "chcp 65001 >nul & powershell -NoProfile -Command", &format!("\"{}\"", script)])
        .output()
        .map_err(|e| format!("执行 Get-NetAdapterStatistics 失败: {}", e))?;
    if !output.status.success() {
        return Err(format!(
            "获取网卡统计失败: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let text_trim = text.trim();
    if text_trim.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(text_trim) {
        let items: Vec<&serde_json::Value> = match &v {
            serde_json::Value::Array(arr) => arr.iter().collect(),
            serde_json::Value::Object(_) => vec![&v],
            _ => vec![],
        };
        for it in items {
            let name = it.get("Name").and_then(|n| n.as_str()).unwrap_or("Unknown").to_string();
            let recv = it.get("ReceivedBytes").and_then(|n| n.as_u64()).unwrap_or(0);
            let sent = it.get("SentBytes").and_then(|n| n.as_u64()).unwrap_or(0);
            out.push(IfaceTraffic { name, received_bytes: recv, sent_bytes: sent });
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// IP 归属地查询
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone, Debug)]
pub struct IpInfo {
    pub ip: String,
    pub country: String,
    pub region: String,
    pub city: String,
    pub isp: String,
    pub org: String,
    pub source: String,
}

/// 查询 IP 归属地。主源 ip-api.com（中文），回退 ipwho.is。
#[tauri::command]
pub async fn ip_lookup(ip: String) -> Result<IpInfo, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .connect_timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;

    // 主源：ip-api.com（支持中文）
    let url = format!(
        "https://ip-api.com/json/{}?lang=zh-CN&fields=status,message,country,regionName,city,query,isp,org",
        ip
    );
    if let Ok(resp) = client.get(&url).send().await {
        if let Ok(v) = resp.json::<serde_json::Value>().await {
            if v.get("status").and_then(|s| s.as_str()) == Some("success") {
                let g = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
                return Ok(IpInfo {
                    ip: g("query"),
                    country: g("country"),
                    region: g("regionName"),
                    city: g("city"),
                    isp: g("isp"),
                    org: g("org"),
                    source: "ip-api.com".into(),
                });
            }
        }
    }

    // 回退：ipwho.is
    let url2 = format!("https://ipwho.is/{}", ip);
    if let Ok(resp) = client.get(&url2).send().await {
        if let Ok(v) = resp.json::<serde_json::Value>().await {
            if v.get("success").and_then(|s| s.as_bool()) == Some(true) {
                let g = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
                let conn = v.get("connection");
                let conn_g = |k: &str| conn.and_then(|c| c.get(k)).and_then(|x| x.as_str()).unwrap_or("").to_string();
                return Ok(IpInfo {
                    ip: g("ip"),
                    country: g("country"),
                    region: g("region"),
                    city: g("city"),
                    isp: conn_g("isp"),
                    org: conn_g("org"),
                    source: "ipwho.is".into(),
                });
            }
        }
    }

    Err(format!("IP 归属地查询失败（网络不可用或 IP 无效: {}）", ip))
}

// ---------------------------------------------------------------------------
// Ping
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone, Debug)]
pub struct PingResult {
    pub host: String,
    pub resolved: Option<String>,
    pub sent: u32,
    pub received: u32,
    pub rtts: Vec<String>, // 原始延迟文本，如 "3ms" / "<1ms"
    pub raw: String,       // ping 原始输出
}

/// Ping 域名 / IP（Windows 内置 ping，默认 4 次）。
#[tauri::command]
pub fn ping_host(host: String, count: Option<u32>) -> Result<PingResult, String> {
    let host = host.trim();
    if host.is_empty() {
        return Err("请输入要 ping 的域名或 IP".into());
    }
    let n = count.unwrap_or(4).clamp(1, 10);
    let output = super::hidden_cmd::hidden_cmd("cmd")
        .args(&["/c", "chcp 65001 >nul & ping", &format!("-n {}", n), host])
        .output()
        .map_err(|e| format!("执行 ping 失败: {}", e))?;
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    let lower = text.to_lowercase();

    // 解析目标 IP：Pinging xxx [1.2.3.4] / 正在 Ping xxx [1.2.3.4]
    let mut resolved = None;
    if let Some(caps) = Regex::new(r"\[([0-9a-fA-F:.]+)\]").ok().and_then(|re| re.captures(&text)) {
        resolved = caps.get(1).map(|m| m.as_str().to_string());
    }

    // 解析每次 RTT：time=3ms / 时间=3ms / time<1ms / 时间<1ms
    let mut rtts = Vec::new();
    if let Some(re) = Regex::new(r"(?:time|时间)\s*[=<]\s*(\d+|<1)\s*ms").ok() {
        for c in re.captures_iter(&text) {
            if let Some(m) = c.get(1) {
                rtts.push(format!("{}ms", m.as_str()));
            }
        }
    }

    // 丢包：Lost = 0 (0% loss) / 丢失 = 0 (0% 丢失)
    let (sent, received) = {
        let re = Regex::new(r"(?:lost|丢失)\s*=\s*(\d+)").ok();
        if let Some(re) = re {
            if let Some(caps) = re.captures(&lower) {
                let _l = caps; // 成功匹配即认为统计段存在
                // 需要"发送 N，接收 M，丢失 L"模式：
                // 英文: Sent = 4, Received = 4, Lost = 0
                // 中文: 已发送 = 4，已接收 = 4，丢失 = 0
                let s = Regex::new(r"(?:sent|已发送)\s*=\s*(\d+)").ok()
                    .and_then(|r| r.captures(&lower))
                    .and_then(|c| c.get(1))
                    .and_then(|m| m.as_str().parse::<u32>().ok())
                    .unwrap_or(0);
                let r = Regex::new(r"(?:received|已接收)\s*=\s*(\d+)").ok()
                    .and_then(|r| r.captures(&lower))
                    .and_then(|c| c.get(1))
                    .and_then(|m| m.as_str().parse::<u32>().ok())
                    .unwrap_or(0);
                (s, r)
            } else {
                (0, 0)
            }
        } else {
            (0, 0)
        }
    };
    // 兜底：未解析到统计时按 rtt 数量估计
    let sent = if sent == 0 && !rtts.is_empty() { rtts.len() as u32 } else { sent };
    let received = if received == 0 && !rtts.is_empty() { rtts.len() as u32 } else { received };

    // 失败判定
    if !output.status.success() && rtts.is_empty() {
        if lower.contains("不是内部或外部命令") {
            return Err("系统缺少 ping 命令".into());
        }
        return Err(format!("ping {} 失败（无法解析主机或网络不可达）", host));
    }
    if lower.contains("无法 ping 通目标主机") || lower.contains("destination host unreachable")
        || lower.contains("无法访问目标主机") || lower.contains("一般故障")
    {
        return Ok(PingResult {
            host: host.to_string(),
            resolved,
            sent,
            received,
            rtts,
            raw: text,
        });
    }

    Ok(PingResult {
        host: host.to_string(),
        resolved,
        sent,
        received,
        rtts,
        raw: text,
    })
}