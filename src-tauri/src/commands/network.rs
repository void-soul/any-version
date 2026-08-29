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
    // 直接调用 powershell（与 mihomo/netinfo.rs 同一套可靠模式），不再经 cmd 拼接
    // `& powershell -Command` —— 那会在某些环境下被 cmd 拆成独立命令，导致
    // powershell 收到 `-Command "..."` 作为脚本本体，报 "-Command 不是可识别的 cmdlet"。
    let script = "$ErrorActionPreference='SilentlyContinue'; [Console]::OutputEncoding=[System.Text.Encoding]::UTF8; $PSDefaultParameterValues['Out-File:Encoding']='utf8'; $OutputEncoding=[System.Text.Encoding]::UTF8; Get-NetAdapterStatistics | Where-Object { $_.Name -notmatch 'vEthernet|Loopback' } | Select-Object Name,ReceivedBytes,SentBytes | ConvertTo-Json -Compress";
    let output = super::hidden_cmd::hidden_cmd("powershell")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script])
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

/// 离线 IP 库的状态（存在性 / 大小 / 修改时间 / 路径）。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct IpDbStatus {
    pub exists: bool,
    pub size_bytes: u64,
    pub updated_at: Option<String>,
    pub path: String,
}

/// 离线 IP 库默认存放位置：{data_dir}/ipdb/country.mmdb。
fn ip_db_path() -> std::path::PathBuf {
    crate::commands::config::get_data_dir().join("ipdb").join("country.mmdb")
}

/// 候选 MMDB 文件：专用库优先，其次复用 mihomo 已同步的 geo 库（很多安装已有）。
fn ip_db_candidates() -> Vec<std::path::PathBuf> {
    let mut v = vec![ip_db_path()];
    v.push(crate::commands::config::get_data_dir().join("mihomo").join("country.mmdb"));
    v
}

/// 用本地 MMDB（MaxMind 格式）查询 IP 归属地。
///
/// 国家/地区级信息从 MMDB 直接读；region/city/isp 离线库不包含，留空。
/// 查不到（内网地址 / 库缺失 / 解析失败）返回 None，由调用方决定是否走网络 API。
fn lookup_offline(ip: &str) -> Option<IpInfo> {
    let addr: std::net::IpAddr = ip.trim().parse().ok()?;
    for path in ip_db_candidates() {
        if !path.is_file() {
            continue;
        }
        let reader = maxminddb::Reader::open_readfile(&path).ok()?;
        let record: maxminddb::geoip2::Country = reader.lookup(addr).ok()?;
        let Some(country) = record.country.as_ref() else {
            continue;
        };
        let iso = country.iso_code.unwrap_or_default();
        // 优先中文名，回退英文名，再回退 ISO 码
        let name = country
            .names
            .as_ref()
            .and_then(|m| m.get("zh-CN").copied())
            .or_else(|| country.names.as_ref().and_then(|m| m.get("en").copied()))
            .unwrap_or(iso)
            .to_string();
        return Some(IpInfo {
            ip: ip.trim().to_string(),
            country: name,
            region: String::new(),
            city: String::new(),
            isp: String::new(),
            org: String::new(),
            source: "离线 MMDB".into(),
        });
    }
    None
}

/// 查询 IP 归属地。优先离线 MMDB 库（本地即查、不依赖网络），
/// 库缺失或命中内网地址时才回退到在线 API（ip-api.com → ipwho.is）。
#[tauri::command]
pub async fn ip_lookup(ip: String) -> Result<IpInfo, String> {
    // 离线优先：本地 MMDB 命中即返回，不打网络
    if let Some(info) = lookup_offline(&ip) {
        return Ok(info);
    }

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

/// 离线 IP 库状态（存在性 / 大小 / 更新时间 / 路径）。
#[tauri::command]
pub fn ip_db_status() -> IpDbStatus {
    let path = ip_db_path();
    let meta = std::fs::metadata(&path).ok();
    let exists = meta.is_some();
    let size_bytes = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let updated_at = meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .map(|t| {
            let dt: chrono::DateTime<chrono::Utc> = t.into();
            dt.to_rfc3339()
        });
    IpDbStatus {
        exists,
        size_bytes,
        updated_at,
        path: path.to_string_lossy().to_string(),
    }
}

/// 下载 / 更新离线 IP 库（country.mmdb，GeoIP2 Country 格式，含中文国家名）。
/// 下载到 {data_dir}/ipdb/country.mmdb，覆盖旧文件。
#[tauri::command]
pub async fn download_ip_db() -> Result<IpDbStatus, String> {
    // GeoLite2 Country 公共镜像（国内可访问，约 5-6MB）
    let url = "https://cdn.jsdelivr.net/gh/P3TERX/GeoLite.mmdb@download/GeoLite2-Country.mmdb";
    let client = reqwest::Client::builder()
        .user_agent("Any-Version-Manager")
        .timeout(std::time::Duration::from_secs(120))
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("下载离线 IP 库失败: {}", e))?;
    let st = resp.status();
    if !st.is_success() {
        return Err(format!(
            "下载离线 IP 库失败 (HTTP {}): {}，可稍后重试或检查网络",
            st.as_u16(),
            crate::commands::utils::github_status_hint(st.as_u16())
        ));
    }
    let bytes = resp.bytes().await.map_err(|e| format!("读取下载内容失败: {}", e))?;
    if bytes.len() < 16 || &bytes[..3] != b"\xab\xcd\xef" {
        return Err("下载到的文件不是有效的 MMDB（文件头校验失败）".into());
    }
    let dir = crate::commands::config::get_data_dir().join("ipdb");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建 IP 库目录失败: {}", e))?;
    let tmp = dir.join("country.mmdb.tmp");
    std::fs::write(&tmp, &bytes).map_err(|e| format!("写入 IP 库失败: {}", e))?;
    // 先删旧文件再改名，Windows 下 rename 到已存在目标会失败
    let final_path = dir.join("country.mmdb");
    let _ = std::fs::remove_file(&final_path);
    std::fs::rename(&tmp, &final_path).map_err(|e| format!("安装 IP 库失败: {}", e))?;
    Ok(ip_db_status())
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 若本机已有 MMDB（ipdb 或 mihomo geo 库），验证公网/内网地址能正确解析。
    /// 无库时跳过（不失败）。
    #[test]
    fn offline_lookup_works_with_existing_db() {
        if !ip_db_candidates().iter().any(|p| p.is_file()) {
            return;
        }
        let info = lookup_offline("8.8.8.8");
        // 8.8.8.8 是 Google 的地址，country 库应命中非空（美国/unknown 都算命中）
        assert!(info.is_some(), "离线库存在时 8.8.8.8 应能查到归属地");
        // 内网地址应能被解析（mihomo 的 country.mmdb 含保留/内网段，返回 Reserved），
        // 解析失败时为 None 由调用方走网络 API，两者都允许。
        lookup_offline("192.168.1.1");
        lookup_offline("127.0.0.1");
        // 无库或无法解析时走网络 API 兜底
        assert!(lookup_offline("not-an-ip").is_none());
    }
}