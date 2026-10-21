//! 本地网络模块（Q-网络）：网卡概览、实时流量、全量连接列表、Ping 与网络诊断。
//!
//! 端口排查（port.rs）专注单个端口占用；本模块提供宏观视角：
//! - 网卡概览：名称 / 描述 / 状态 / MAC / IP / 网关 / DNS / 链路速率
//! - 实时流量：sysinfo 计数器，前端轮询后按差值计算速率（后端不做定时任务）
//! - 所有 TCP/UDP 连接（含进程名映射）
//! - Ping、DNS 解析、TCP 端口连通测试、路由追踪
//!
//! 注：曾内置 GeoLite2 MMDB 离线 IP 库下载与归属地查询，因下载常失败（非 MMDB）且
//! 网卡流量拖慢页面，已整体移除，只保留本地连接分析。当前实时流量为「按需采样」：
//! 只有打开概览页并处于前台时才轮询，离开页签/窗口最小化即停止。

use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// 通用工具
// ---------------------------------------------------------------------------

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 解析一行 CSV（tasklist 输出），支持双引号包裹与字段内逗号。
fn split_csv_line(line: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![String::new()];
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                if in_quotes && chars.peek() == Some(&'"') {
                    out.last_mut().unwrap().push('"');
                    chars.next();
                } else {
                    in_quotes = !in_quotes;
                }
            }
            ',' if !in_quotes => out.push(String::new()),
            _ => out.last_mut().unwrap().push(c),
        }
    }
    out.into_iter().map(|s| s.trim().to_string()).collect()
}

/// 规范化用户输入的主机名：
/// - 支持粘贴完整 URL（取 host）
/// - 去掉 `host:port` 的端口（IPv6 字面量原样保留）
/// - 去掉 IPv6 的方括号
fn normalize_host(input: &str) -> Result<String, String> {
    let raw = input.trim();
    if raw.is_empty() {
        return Err("请输入域名或 IP".into());
    }
    let mut host = raw.to_string();
    if let Ok(u) = url::Url::parse(raw) {
        if let Some(h) = u.host_str() {
            host = h.to_string();
        }
    } else if let Some((h, _)) = raw.split_once('/') {
        host = h.to_string();
    }
    // [::1]:8080 → ::1
    if let Some(rest) = host.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            return finish_host(&rest[..end]);
        }
    }
    // host:8080 → host（IPv6 含多个冒号，不处理）
    if host.matches(':').count() == 1 {
        if let Some((h, p)) = host.split_once(':') {
            if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) {
                return finish_host(h);
            }
        }
    }
    finish_host(&host)
}

fn finish_host(host: &str) -> Result<String, String> {
    let host = host.trim().trim_matches(|c| c == '[' || c == ']');
    if host.is_empty() {
        return Err("请输入域名或 IP".into());
    }
    // 主机名会拼进 cmd /c 命令行，拒绝所有 shell 元字符（保留中文/IDN 域名）。
    // `%` 必须一起挡：cmd 会展开 `%VAR%`，等于把环境变量塞进命令行。
    let unsafe_char = |c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '&' | '|' | '>' | '<' | '^' | '"' | '\'' | '`' | '(' | ')' | ';' | ',' | '='
                    | '!' | '?' | '*' | '#' | '%'
            )
    };
    if host.chars().any(unsafe_char) {
        return Err("域名或 IP 含非法字符".into());
    }
    Ok(host.to_string())
}

fn push_unique(list: &mut Vec<String>, value: String) {
    if !value.is_empty() && !list.iter().any(|v| v == &value) {
        list.push(value);
    }
}

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
            let parts = split_csv_line(l);
            if parts.len() < 2 {
                continue;
            }
            let name = parts[0].clone();
            let pid = parts[1].clone();
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
        let proto_base = if up.starts_with("TCP") {
            "TCP"
        } else if up.starts_with("UDP") {
            "UDP"
        } else {
            continue;
        };
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
            (
                fields[1].to_string(),
                fields[2].to_string(),
                fields[3].to_string(),
                fields[4].to_string(),
            )
        } else {
            if fields.len() < 4 {
                continue;
            }
            (
                fields[1].to_string(),
                fields[2].to_string(),
                "-".to_string(),
                fields[3].to_string(),
            )
        };
        if local == "*" {
            continue;
        }
        let process = pid_map.get(&pid).cloned().unwrap_or_default();
        out.push(NetConnection {
            proto,
            local,
            remote,
            state,
            pid,
            process,
        });
    }
    out
}

/// 状态排序权重：监听最需要被看到，其次已建立，异常/等待状态靠后。
fn state_rank(state: &str) -> u8 {
    match state {
        "LISTENING" => 0,
        "ESTABLISHED" => 1,
        "SYN_SENT" | "SYN_RECEIVED" => 2,
        "TIME_WAIT" => 3,
        "CLOSE_WAIT" | "FIN_WAIT_1" | "FIN_WAIT_2" | "LAST_ACK" | "CLOSING" => 4,
        _ => 5,
    }
}

fn sort_connections(conns: &mut [NetConnection]) {
    conns.sort_by(|a, b| {
        state_rank(&a.state)
            .cmp(&state_rank(&b.state))
            .then_with(|| a.local.cmp(&b.local))
    });
}

fn collect_connections() -> Result<Vec<NetConnection>, String> {
    let output = super::hidden_cmd::hidden_cmd("cmd")
        .args(&["/c", "chcp 65001 >nul & netstat -ano"])
        .output()
        .map_err(|e| format!("执行 netstat 失败: {}", e))?;
    let text = String::from_utf8_lossy(&output.stdout);
    let pid_map = build_pid_map();
    let mut conns = parse_netstat(&text, &pid_map);
    sort_connections(&mut conns);
    // 限制返回数量，避免超大连接数拖垮 UI
    if conns.len() > 5000 {
        conns.truncate(5000);
    }
    Ok(conns)
}

/// 列出所有 TCP/UDP 网络连接（含进程名）。
/// 异步命令：tasklist + netstat 可能耗时 1s 以上，放到阻塞线程池避免卡界面。
#[tauri::command]
pub async fn net_connections() -> Result<Vec<NetConnection>, String> {
    tokio::task::spawn_blocking(collect_connections)
        .await
        .map_err(|e| format!("读取网络连接失败: {}", e))?
}

// ---------------------------------------------------------------------------
// 网卡概览 / 实时流量
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone, Debug)]
pub struct NetAdapter {
    pub name: String,
    pub description: String,
    pub index: i64,
    pub status: String,
    pub mac: String,
    pub speed: String,
    pub ipv4: Vec<String>,
    pub ipv6: Vec<String>,
    pub gateway: Vec<String>,
    pub dns: Vec<String>,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_packets: u64,
    pub tx_packets: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct TrafficCounter {
    pub name: String,
    pub mac: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct TrafficSample {
    /// 采样时间（Unix 毫秒），前端用它计算两次采样的时间差
    pub ts_ms: u64,
    pub adapters: Vec<TrafficCounter>,
}

struct AdapterCounters {
    name: String,
    mac: String,
    ipv4: Vec<String>,
    ipv6: Vec<String>,
    rx_bytes: u64,
    tx_bytes: u64,
    rx_packets: u64,
    tx_packets: u64,
}

fn norm_mac(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect::<String>()
        .to_uppercase()
}

fn format_mac(s: &str) -> String {
    let hex = norm_mac(s);
    if hex.len() == 12 {
        hex.as_bytes()
            .chunks(2)
            .map(|c| std::str::from_utf8(c).unwrap_or(""))
            .collect::<Vec<_>>()
            .join(":")
    } else {
        s.to_string()
    }
}

/// 轻量计数器：sysinfo 直接读系统网卡统计，不拉起任何子进程。
fn read_counters() -> Vec<AdapterCounters> {
    let networks = sysinfo::Networks::new_with_refreshed_list();
    networks
        .iter()
        .map(|(name, data)| {
            let mut ipv4 = Vec::new();
            let mut ipv6 = Vec::new();
            for net in data.ip_networks() {
                match net.addr {
                    IpAddr::V4(a) => push_unique(&mut ipv4, a.to_string()),
                    IpAddr::V6(a) => push_unique(&mut ipv6, a.to_string()),
                }
            }
            AdapterCounters {
                name: name.clone(),
                mac: format_mac(&data.mac_address().to_string()),
                ipv4,
                ipv6,
                rx_bytes: data.total_received(),
                tx_bytes: data.total_transmitted(),
                rx_packets: data.total_packets_received(),
                tx_packets: data.total_packets_transmitted(),
            }
        })
        .collect()
}

fn ps_json(script: &str) -> Option<Value> {
    let out = super::hidden_cmd::hidden_cmd("powershell")
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
    let s = String::from_utf8_lossy(&out.stdout);
    let s = s.trim_start_matches('\u{feff}').trim();
    if s.is_empty() {
        return None;
    }
    serde_json::from_str(s).ok()
}

/// 一次 PowerShell 进程取回网卡枚举所需的全部数据
/// （网卡 / 地址 / 默认路由 / DNS 分开查询，每次都要重新加载 NetTCPIP 模块，会很慢）。
const ADAPTER_SCRIPT: &str = r#"$ErrorActionPreference='SilentlyContinue';
[Console]::OutputEncoding=[System.Text.Encoding]::UTF8;
$PSDefaultParameterValues['Out-File:Encoding']='utf8';
$OutputEncoding=[System.Text.Encoding]::UTF8;
$o=[ordered]@{
adapters=@(Get-NetAdapter | Select-Object Name,InterfaceDescription,InterfaceIndex,Status,MacAddress,LinkSpeed);
addrs=@(Get-NetIPAddress | Select-Object InterfaceIndex,IPAddress,AddressFamily,PrefixLength);
routes=@(Get-NetRoute | Where-Object { $_.DestinationPrefix -eq '0.0.0.0/0' -or $_.DestinationPrefix -eq '::/0' } | Select-Object InterfaceIndex,NextHop,RouteMetric);
dns=@(Get-DnsClientServerAddress | Select-Object InterfaceIndex,AddressFamily,ServerAddresses)
};
$o | ConvertTo-Json -Compress -Depth 4"#;

fn as_array(v: Option<Value>) -> Vec<Value> {
    match v {
        Some(Value::Array(a)) => a,
        Some(Value::Null) | None => vec![],
        Some(other) => vec![other],
    }
}

fn idx_of(v: &Value) -> i64 {
    v.get("InterfaceIndex")
        .and_then(|x| {
            x.as_i64()
                .or_else(|| x.as_str().and_then(|s| s.parse::<i64>().ok()))
        })
        .unwrap_or(-1)
}

fn json_str(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn collect_adapters() -> Vec<NetAdapter> {
    let counters = read_counters();
    let by_name: HashMap<String, &AdapterCounters> = counters
        .iter()
        .map(|c| (c.name.to_lowercase(), c))
        .collect();
    // 全零 MAC（回环 / Teredo / ISATAP 都是 00:00:00:00:00:00）不能进映射：
    // 它们会互相覆盖，把流量算到别的网卡上。
    let by_mac: HashMap<String, &AdapterCounters> = counters
        .iter()
        .filter(|c| {
            let m = norm_mac(&c.mac);
            !m.is_empty() && m != "000000000000"
        })
        .map(|c| (norm_mac(&c.mac), c))
        .collect();

    let root = ps_json(ADAPTER_SCRIPT);
    if let Some(root) = root {
        let adapters = as_array(root.get("adapters").cloned());
        let addrs = as_array(root.get("addrs").cloned());
        let routes = as_array(root.get("routes").cloned());
        let dns = as_array(root.get("dns").cloned());

        if !adapters.is_empty() {
            let mut items = Vec::new();
            for a in &adapters {
                let name = json_str(a, "Name");
                let mac_raw = json_str(a, "MacAddress");
                let idx = idx_of(a);
                let counter = by_name
                    .get(&name.to_lowercase())
                    .copied()
                    .or_else(|| {
                        let m = norm_mac(&mac_raw);
                        if m.is_empty() {
                            None
                        } else {
                            by_mac.get(&m).copied()
                        }
                    });

                let mut ipv4 = Vec::new();
                let mut ipv6 = Vec::new();
                for ad in &addrs {
                    if idx_of(ad) != idx {
                        continue;
                    }
                    let ip = json_str(ad, "IPAddress");
                    if ip.is_empty() {
                        continue;
                    }
                    let is_v6 = ip.contains(':')
                        || json_str(ad, "AddressFamily") == "IPv6"
                        || ad.get("AddressFamily").and_then(|x| x.as_i64()) == Some(23);
                    if is_v6 {
                        push_unique(&mut ipv6, ip);
                    } else {
                        push_unique(&mut ipv4, ip);
                    }
                }

                let mut gateway = Vec::new();
                for r in &routes {
                    if idx_of(r) == idx {
                        push_unique(&mut gateway, json_str(r, "NextHop"));
                    }
                }

                let mut dns_list = Vec::new();
                for d in &dns {
                    if idx_of(d) != idx {
                        continue;
                    }
                    match d.get("ServerAddresses") {
                        Some(Value::Array(list)) => {
                            for s in list {
                                if let Some(s) = s.as_str() {
                                    push_unique(&mut dns_list, s.to_string());
                                }
                            }
                        }
                        Some(Value::String(s)) => push_unique(&mut dns_list, s.clone()),
                        _ => {}
                    }
                }

                // 计数器与 PowerShell 的名称/ MAC 对不上时退回 sysinfo 自己的列表
                if ipv4.is_empty() && ipv6.is_empty() {
                    if let Some(c) = counter {
                        ipv4 = c.ipv4.clone();
                        ipv6 = c.ipv6.clone();
                    }
                }

                let mac = if let Some(c) = counter {
                    if !c.mac.is_empty() {
                        c.mac.clone()
                    } else {
                        format_mac(&mac_raw)
                    }
                } else {
                    format_mac(&mac_raw)
                };

                items.push(NetAdapter {
                    name,
                    description: json_str(a, "InterfaceDescription"),
                    index: idx,
                    status: json_str(a, "Status"),
                    mac,
                    speed: json_str(a, "LinkSpeed"),
                    ipv4,
                    ipv6,
                    gateway,
                    dns: dns_list,
                    rx_bytes: counter.map(|c| c.rx_bytes).unwrap_or(0),
                    tx_bytes: counter.map(|c| c.tx_bytes).unwrap_or(0),
                    rx_packets: counter.map(|c| c.rx_packets).unwrap_or(0),
                    tx_packets: counter.map(|c| c.tx_packets).unwrap_or(0),
                });
            }
            return items;
        }
    }

    // PowerShell 不可用（被裁剪/执行策略限制）时退回 sysinfo：
    // 仍有名称 / IP / MAC / 流量，只是缺少描述、网关、DNS 与链路速率。
    counters
        .into_iter()
        .map(|c| {
            let status = if c.ipv4.is_empty() && c.ipv6.is_empty() {
                "Unknown".to_string()
            } else {
                "Up".to_string()
            };
            NetAdapter {
                name: c.name,
                description: String::new(),
                index: -1,
                status,
                mac: c.mac,
                speed: String::new(),
                ipv4: c.ipv4,
                ipv6: c.ipv6,
                gateway: Vec::new(),
                dns: Vec::new(),
                rx_bytes: c.rx_bytes,
                tx_bytes: c.tx_bytes,
                rx_packets: c.rx_packets,
                tx_packets: c.tx_packets,
            }
        })
        .collect()
}

/// 枚举本机网卡（名称 / 描述 / 状态 / MAC / IPv4 / IPv6 / 网关 / DNS / 链路速率 / 累计流量）。
#[tauri::command]
pub async fn net_adapters() -> Result<Vec<NetAdapter>, String> {
    tokio::task::spawn_blocking(collect_adapters)
        .await
        .map_err(|e| format!("读取网卡信息失败: {}", e))
}

fn collect_traffic() -> TrafficSample {
    TrafficSample {
        ts_ms: now_ms(),
        adapters: read_counters()
            .into_iter()
            .map(|c| TrafficCounter {
                name: c.name,
                mac: c.mac,
                rx_bytes: c.rx_bytes,
                tx_bytes: c.tx_bytes,
            })
            .collect(),
    }
}

/// 采样一次网卡计数器（只读 sysinfo，无子进程；前端按差值算速率）。
#[tauri::command]
pub async fn net_traffic() -> TrafficSample {
    tokio::task::spawn_blocking(collect_traffic)
        .await
        .unwrap_or_else(|_| TrafficSample {
            ts_ms: now_ms(),
            adapters: Vec::new(),
        })
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

/// 解析每次 RTT：time=3ms / 时间=3ms / time<1ms / 时间<1ms。
/// 注意 `<1` 必须保留符号：直接按数字捕获会把它显示成 `1ms`。
fn parse_ping_rtts(text: &str) -> Vec<String> {
    let mut rtts = Vec::new();
    if let Ok(re) = Regex::new(r"(?i)(?:time|时间)\s*([=<])\s*(\d+)\s*ms") {
        for c in re.captures_iter(text) {
            // 只有 `<` 需要保留；`=` 是普通分隔符，不能显示成 "=3ms"
            let sign = if &c[1] == "<" { "<" } else { "" };
            rtts.push(format!("{}{}ms", sign, &c[2]));
        }
    }
    rtts
}

/// 解析 ping 统计行：`Sent = 4, Received = 4, Lost = 0` /
/// `已发送 = 4，已接收 = 4，丢失 = 0`。三个数字都要，缺了 Lost 就无法表达「100% 丢包」。
fn parse_ping_counts(text: &str) -> (u32, u32, u32) {
    let lower = text.to_lowercase();
    let stat = |en: &str, zh: &str| -> u32 {
        Regex::new(&format!(r"(?:{}|{})\s*=\s*(\d+)", en, zh))
            .ok()
            .and_then(|re| re.captures(&lower))
            .and_then(|c| c.get(1))
            .and_then(|m| m.as_str().parse::<u32>().ok())
            .unwrap_or(0)
    };
    (
        stat("sent", "已发送"),
        stat("received", "已接收"),
        stat("lost", "丢失"),
    )
}

fn run_ping(host: &str, n: u32) -> Result<PingResult, String> {
    let host = normalize_host(host)?;
    let command = format!("chcp 65001 >nul & ping -n {} {}", n, host);
    let output = super::hidden_cmd::hidden_cmd("cmd")
        .args(&["/c", &command])
        .output()
        .map_err(|e| format!("执行 ping 失败: {}", e))?;
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    let lower = text.to_lowercase();

    // 解析目标 IP：Pinging xxx [1.2.3.4] / 正在 Ping xxx [1.2.3.4]
    let resolved = Regex::new(r"\[([0-9a-fA-F:.]+)\]")
        .ok()
        .and_then(|re| re.captures(&text))
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string());

    let rtts = parse_ping_rtts(&text);

    let (sent_raw, received_raw, lost) = parse_ping_counts(&text);
    let mut sent = sent_raw;
    let mut received = received_raw;

    // 全丢包时 Received=0 且没有 rtt，只有 Lost 能还原发送数；否则界面显示 0/0。
    if sent == 0 && (received > 0 || lost > 0) {
        sent = received + lost;
    }
    // 兜底：未解析到统计时按 rtt 数量估计
    if sent == 0 && !rtts.is_empty() {
        sent = rtts.len() as u32;
    }
    if received == 0 && !rtts.is_empty() {
        received = rtts.len() as u32;
    }

    // 失败判定
    if !output.status.success() && rtts.is_empty() {
        if lower.contains("不是内部或外部命令") {
            return Err("系统缺少 ping 命令".into());
        }
        return Err(format!("ping {} 失败（无法解析主机或网络不可达）", host));
    }

    Ok(PingResult {
        host,
        resolved,
        sent,
        received,
        rtts,
        raw: text,
    })
}

/// Ping 域名 / IP（Windows 内置 ping，默认 4 次）。
/// 异步命令：最多 10 次 ping 可能阻塞数秒，不能占用主线程。
#[tauri::command]
pub async fn ping_host(host: String, count: Option<u32>) -> Result<PingResult, String> {
    let n = count.unwrap_or(4).clamp(1, 10);
    tokio::task::spawn_blocking(move || run_ping(&host, n))
        .await
        .map_err(|e| format!("ping 任务失败: {}", e))?
}

// ---------------------------------------------------------------------------
// DNS 解析
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone, Debug)]
pub struct DnsLookupResult {
    pub host: String,
    pub addresses: Vec<String>,
    pub elapsed_ms: u64,
}

fn run_dns_lookup(host: String) -> Result<DnsLookupResult, String> {
    let started = Instant::now();
    // 与 ping / tracert 同一套归一：粘贴 `https://x/a`、`x:8080`、`[::1]:80` 都能解析
    let host = normalize_host(&host)?;
    let addrs = (host.as_str(), 0u16)
        .to_socket_addrs()
        .map_err(|e| format!("无法解析 {}: {}", host, e))?;
    let mut addresses = Vec::new();
    for a in addrs {
        push_unique(&mut addresses, a.ip().to_string());
    }
    if addresses.is_empty() {
        return Err(format!("{} 没有解析结果", host));
    }
    Ok(DnsLookupResult {
        host,
        addresses,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

/// 解析域名 / IP（系统 DNS，返回全部去重地址）。
#[tauri::command]
pub async fn dns_lookup(host: String) -> Result<DnsLookupResult, String> {
    tokio::task::spawn_blocking(move || run_dns_lookup(host))
        .await
        .map_err(|e| format!("DNS 查询任务失败: {}", e))?
}

// ---------------------------------------------------------------------------
// TCP 端口连通测试
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone, Debug)]
pub struct TcpTestResult {
    pub host: String,
    pub resolved: Option<String>,
    pub port: u16,
    pub ok: bool,
    pub latency_ms: u64,
    pub error: Option<String>,
}

fn run_tcp_test(host: String, port: u16, timeout: Duration) -> Result<TcpTestResult, String> {
    let addrs: Vec<SocketAddr> = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|e| format!("无法解析 {}: {}", host, e))?
        .collect();
    if addrs.is_empty() {
        return Err(format!("{} 没有可用的地址", host));
    }
    let first_ip = addrs[0].ip().to_string();
    let mut last_error: Option<String> = None;
    // 失败时也报「最后一次握手尝试」的耗时：把逐个地址的总和当握手耗时是误导
    let mut last_elapsed = Duration::ZERO;
    for addr in &addrs {
        let attempt = Instant::now();
        match TcpStream::connect_timeout(addr, timeout) {
            Ok(_) => {
                return Ok(TcpTestResult {
                    host,
                    resolved: Some(addr.ip().to_string()),
                    port,
                    ok: true,
                    latency_ms: attempt.elapsed().as_millis() as u64,
                    error: None,
                });
            }
            Err(e) => {
                last_elapsed = attempt.elapsed();
                last_error = Some(format!("{} → {}", addr, e));
            }
        }
    }
    Ok(TcpTestResult {
        host,
        resolved: Some(first_ip),
        port,
        ok: false,
        latency_ms: last_elapsed.as_millis() as u64,
        error: last_error,
    })
}

/// 测试 TCP 端口是否可连通（依次尝试解析出的全部地址）。
#[tauri::command]
pub async fn tcp_port_test(
    host: String,
    port: u16,
    timeout_ms: Option<u64>,
) -> Result<TcpTestResult, String> {
    if port == 0 {
        return Err("端口需在 1-65535 之间".into());
    }
    let host = normalize_host(&host)?;
    let timeout = Duration::from_millis(timeout_ms.unwrap_or(3000).clamp(200, 30_000));
    tokio::task::spawn_blocking(move || run_tcp_test(host, port, timeout))
        .await
        .map_err(|e| format!("端口测试任务失败: {}", e))?
}

// ---------------------------------------------------------------------------
// 路由追踪
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone, Debug)]
pub struct TracertHop {
    pub hop: u32,
    pub ip: Option<String>,
    pub rtts: Vec<String>,
    pub raw: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct TracertResult {
    pub host: String,
    pub hops: Vec<TracertHop>,
    pub raw: String,
    pub truncated: bool,
}

/// 解析 tracert -d 输出。中文/英文的跳数行都以「序号 + 3 个时延」开头，
/// 末尾是 IP（-d 不做反向解析）或 `*`（超时），因此按结构解析而不依赖文案。
fn parse_tracert(text: &str) -> Vec<TracertHop> {
    let mut hops = Vec::new();
    let hop_re = match Regex::new(r"^\s*(\d{1,2})\s+(.+?)\s*$") {
        Ok(re) => re,
        Err(_) => return hops,
    };
    let rtt_re = match Regex::new(r"(?i)(<\s*\d+|\d+)\s*ms") {
        Ok(re) => re,
        Err(_) => return hops,
    };
    let ipv4_re = match Regex::new(r"(\d{1,3}(?:\.\d{1,3}){3})") {
        Ok(re) => re,
        Err(_) => return hops,
    };
    let ipv6_re = match Regex::new(r"([0-9a-fA-F]{1,4}(?::[0-9a-fA-F]{1,4}){2,})") {
        Ok(re) => re,
        Err(_) => return hops,
    };

    for line in text.lines() {
        let raw = line.trim();
        if raw.is_empty() {
            continue;
        }
        let caps = match hop_re.captures(raw) {
            Some(c) => c,
            None => continue,
        };
        let hop: u32 = match caps[1].parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let rest = &caps[2];
        // 跳过标题/统计行（它们不以序号开头，这里只是防御性判断）
        let has_marker = rest.contains("ms")
            || rest.contains('*')
            || rest.contains("超时")
            || rest.to_lowercase().contains("timed out");
        if !has_marker {
            continue;
        }
        let mut rtts = Vec::new();
        for c in rtt_re.captures_iter(rest) {
            rtts.push(format!("{}ms", c[1].replace(' ', "")));
        }
        let ip = ipv4_re
            .captures(rest)
            .map(|c| c[1].to_string())
            .or_else(|| ipv6_re.captures(rest).map(|c| c[1].to_string()));
        hops.push(TracertHop {
            hop,
            ip,
            rtts,
            raw: raw.to_string(),
        });
    }
    hops
}

fn run_tracert(host: String, max_hops: u8) -> Result<TracertResult, String> {
    let command = format!("chcp 65001 >nul & tracert -d -w 500 -h {} {}", max_hops, host);
    let mut cmd = super::hidden_cmd::hidden_cmd("cmd");
    cmd.args(&["/c", &command]);
    let out = super::hidden_cmd::output_capped(cmd, 2 * 1024 * 1024)
        .map_err(|e| format!("执行 tracert 失败: {}", e))?;
    let text = out.stdout;
    if text.trim().is_empty() {
        let detail = out.stderr.trim();
        return Err(if detail.is_empty() {
            format!("tracert {} 没有输出", host)
        } else {
            format!("tracert {} 失败: {}", host, detail)
        });
    }
    if text.contains("不是内部或外部命令") {
        return Err("系统缺少 tracert 命令".into());
    }
    Ok(TracertResult {
        host,
        hops: parse_tracert(&text),
        raw: text,
        truncated: out.truncated,
    })
}

/// 路由追踪（Windows 内置 tracert -d，不做反向解析以加快速度）。
#[tauri::command]
pub async fn tracert_host(host: String, max_hops: Option<u8>) -> Result<TracertResult, String> {
    let host = normalize_host(&host)?;
    let hops = max_hops.unwrap_or(16).clamp(1, 30);
    tokio::task::spawn_blocking(move || run_tracert(host, hops))
        .await
        .map_err(|e| format!("路由追踪任务失败: {}", e))?
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_line_keeps_quoted_comma() {
        let parts = split_csv_line("\"a,b.exe\",\"1234\",\"Console\",\"1\",\"1,024 K\"");
        assert_eq!(parts[0], "a,b.exe");
        assert_eq!(parts[1], "1234");
        assert_eq!(parts[4], "1,024 K");
    }

    #[test]
    fn netstat_listening_states_sort_first() {
        let map = HashMap::new();
        let text = "\
  TCP    10.0.0.2:5000    1.1.1.1:443    ESTABLISHED    4321
  TCP    0.0.0.0:80        0.0.0.0:0      LISTENING      1234
  TCP    10.0.0.2:5001    2.2.2.2:443    TIME_WAIT      0
  UDP    0.0.0.0:5355      *:*                            999
";
        let mut conns = parse_netstat(text, &map);
        assert_eq!(conns.len(), 4);
        sort_connections(&mut conns);
        assert_eq!(conns[0].state, "LISTENING");
        assert_eq!(conns[1].state, "ESTABLISHED");
        assert_eq!(conns[3].state, "-");
        assert!(conns.iter().any(|c| c.pid == "999" && c.remote == "*:*"));
    }

    #[test]
    fn ping_rtts_keep_less_than_sign() {
        let en = "Reply from 1.1.1.1: time<1ms\nReply from 1.1.1.1: time=3ms";
        assert_eq!(parse_ping_rtts(en), vec!["<1ms", "3ms"]);
        let zh = "来自 1.1.1.1 的回复: 时间<1ms\n来自 1.1.1.1 的回复: 时间=12ms";
        assert_eq!(parse_ping_rtts(zh), vec!["<1ms", "12ms"]);
    }

    /// 100% 丢包时 Received=0 且没有 rtt：只有 Lost 能让界面显示 4/0 而不是 0/0。
    #[test]
    fn ping_counts_include_lost_for_total_failure() {
        assert_eq!(
            parse_ping_counts("Packets: Sent = 4, Received = 0, Lost = 4 (100% loss),"),
            (4, 0, 4)
        );
        assert_eq!(
            parse_ping_counts("数据包: 已发送 = 4，已接收 = 0，丢失 = 4 (100% 丢失)，"),
            (4, 0, 4)
        );
        assert_eq!(
            parse_ping_counts("Packets: Sent = 4, Received = 4, Lost = 0 (0% loss),"),
            (4, 4, 0)
        );
    }

    #[test]
    fn tracert_parses_hops_and_timeouts() {
        let text = "\
Tracing route to example.com [93.184.216.34]
over a maximum of 16 hops:

  1    <1 ms    <1 ms    <1 ms  192.168.1.1
  2     3 ms     4 ms     3 ms  10.0.0.1
  3     *        *        *     Request timed out.
  4    12 ms    11 ms    12 ms  93.184.216.34

Trace complete.
";
        let hops = parse_tracert(text);
        assert_eq!(hops.len(), 4);
        assert_eq!(hops[0].hop, 1);
        assert_eq!(hops[0].ip.as_deref(), Some("192.168.1.1"));
        assert_eq!(hops[0].rtts, vec!["<1ms", "<1ms", "<1ms"]);
        assert_eq!(hops[2].ip, None);
        assert_eq!(hops[3].ip.as_deref(), Some("93.184.216.34"));
    }

    #[test]
    fn host_normalization_accepts_urls_and_ports() {
        assert_eq!(normalize_host("https://github.com/a/b").unwrap(), "github.com");
        assert_eq!(normalize_host("example.com:8080").unwrap(), "example.com");
        assert_eq!(normalize_host("[::1]:8080").unwrap(), "::1");
        assert_eq!(normalize_host("8.8.8.8").unwrap(), "8.8.8.8");
        assert_eq!(normalize_host("测试.中国").unwrap(), "测试.中国");
        assert!(normalize_host("a.com & calc").is_err());
        assert!(normalize_host("   ").is_err());
        // `%` 会被 cmd 展开成环境变量，必须挡掉
        assert!(normalize_host("%COMSPEC%").is_err());
        assert!(normalize_host("a.com%PATH%").is_err());
    }

    #[test]
    fn mac_formatting_is_consistent() {
        assert_eq!(format_mac("aa-bb-cc-dd-ee-ff"), "AA:BB:CC:DD:EE:FF");
        assert_eq!(format_mac("AA:BB:CC:DD:EE:FF"), "AA:BB:CC:DD:EE:FF");
    }
}
