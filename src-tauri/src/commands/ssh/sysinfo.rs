//! 服务器状态采样的纯解析层：把一段 `/proc` 拼接输出解析成结构化快照。
//!
//! 采集用**一次 exec** 拿全量原始数据（见 [`sample_command`]），解析在本模块完成，
//! CPU/网络速率需要与上一帧比较，因此 [`parse_sample`] 同时返回新的基准样本。
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CpuSample {
    pub total: u64,
    pub idle: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NetSample {
    pub rx: u64,
    pub tx: u64,
}

/// 下一帧计算速率用的基准。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Baseline {
    pub cpu: CpuSample,
    pub net: NetSample,
}

/// 一块挂载点的用量；服务器通常有多块盘，只给根分区会漏掉数据盘。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskUsage {
    pub filesystem: String,
    pub mount: String,
    pub total_kb: u64,
    pub used_kb: u64,
    pub percent: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SysSnapshot {
    pub cpu_percent: Option<f64>,
    pub mem_total_kb: u64,
    pub mem_used_kb: u64,
    pub swap_total_kb: u64,
    pub swap_used_kb: u64,
    /// 根分区（兼容旧字段；多盘明细看 [`Self::disks`]）
    pub disk_total_kb: u64,
    pub disk_used_kb: u64,
    /// 所有真实块设备的挂载点（已过滤 tmpfs / devtmpfs 等内存盘）
    pub disks: Vec<DiskUsage>,
    pub net_rx_bytes: u64,
    pub net_tx_bytes: u64,
    /// 字节/秒；首帧为 None。
    pub net_rx_rate: Option<f64>,
    pub net_tx_rate: Option<f64>,
    pub load1: f64,
    pub uptime_secs: u64,
    pub cores: u32,
    pub cpu_model: String,
}

/// 一次 exec 取全部原始数据；段标记必须与 `parse_sample` 一致。
pub fn sample_command() -> &'static str {
    concat!(
        "echo '---stat'; head -1 /proc/stat; ",
        "echo '---mem'; grep -E '^(MemTotal|MemFree|MemAvailable):' /proc/meminfo; ",
        "echo '---swap'; grep -E '^(SwapTotal|SwapFree):' /proc/meminfo; ",
        "echo '---df'; df -kP 2>/dev/null | tail -n +2; ",
        "echo '---net'; tail -n +3 /proc/net/dev; ",
        "echo '---load'; cat /proc/loadavg; ",
        "echo '---uptime'; cat /proc/uptime; ",
        "echo '---cpuinfo'; grep -m1 'model name' /proc/cpuinfo; ",
        "echo '---cores'; nproc 2>/dev/null || grep -c ^processor /proc/cpuinfo"
    )
}

fn section<'a>(raw: &'a str, name: &str) -> Option<&'a str> {
    let marker = format!("---{name}");
    let start = raw.find(&marker)? + marker.len();
    let rest = &raw[start..];
    let end = rest.find("---").unwrap_or(rest.len());
    Some(rest[..end].trim())
}

fn kb_value(line: &str) -> Option<u64> {
    line.split_whitespace().nth(1)?.parse().ok()
}

fn kv(text: &str, key: &str) -> Option<u64> {
    text.lines().find(|l| l.starts_with(key)).and_then(kb_value)
}

/// 内存盘/只读镜像没有「容量规划」意义，列出来只会淹没真实磁盘。
const SKIP_FS: [&str; 4] = ["tmpfs", "devtmpfs", "squashfs", "udev"];

/// 解析 `df -kP` 的一行：Filesystem 1024-blocks Used Available Capacity Mounted。
fn parse_disk_line(line: &str) -> Option<DiskUsage> {
    let cols: Vec<&str> = line.split_whitespace().collect();
    if cols.len() < 6 {
        return None;
    }
    let filesystem = cols[0];
    if SKIP_FS.contains(&filesystem) {
        return None;
    }
    let total_kb: u64 = cols[1].parse().ok()?;
    if total_kb == 0 {
        return None;
    }
    let used_kb: u64 = cols[2].parse().ok()?;
    let percent: u32 = cols[4].trim_end_matches('%').parse().ok()?;
    Some(DiskUsage {
        filesystem: filesystem.to_string(),
        // 挂载点可能含空格（如 /media/user/My Disk）
        mount: cols[5..].join(" "),
        total_kb,
        used_kb,
        percent,
    })
}

/// 解析采样输出。`prev` 为上一帧基准；首帧传 `None`，此时速率字段为 `None`。
/// 返回 `(快照, 本帧基准)`。
pub fn parse_sample(raw: &str, prev: Option<&Baseline>) -> Option<(SysSnapshot, Baseline)> {
    if raw.trim().is_empty() || !raw.contains("---") {
        return None;
    }

    // /proc/stat 第一行：cpu  user nice system idle iowait irq softirq steal ...
    // 该段缺失时不让整块失败：CPU 显示为不可用，其余指标照常展示。
    let cpu = section(raw, "stat")
        .and_then(|stat| {
            let nums: Vec<u64> = stat
                .lines()
                .next()?
                .split_whitespace()
                .skip(1)
                .filter_map(|v| v.parse().ok())
                .collect();
            if nums.len() < 4 {
                return None;
            }
            Some(CpuSample {
                total: nums.iter().sum(),
                // idle + iowait 视为空闲
                idle: nums[3] + nums.get(4).copied().unwrap_or(0),
            })
        })
        .unwrap_or(CpuSample { total: 0, idle: 0 });

    let mem = section(raw, "mem").unwrap_or_default();
    let mem_total = kv(mem, "MemTotal").unwrap_or(0);
    // 用 MemAvailable：MemFree 不含可回收缓存，Linux 上会长期显示高占用
    let mem_available = kv(mem, "MemAvailable")
        .or_else(|| kv(mem, "MemFree"))
        .unwrap_or(0);
    let mem_used = mem_total.saturating_sub(mem_available);

    let swap = section(raw, "swap").unwrap_or_default();
    let swap_total = kv(swap, "SwapTotal").unwrap_or(0);
    let swap_free = kv(swap, "SwapFree").unwrap_or(0);
    let swap_used = swap_total.saturating_sub(swap_free);

    // df -kP：多块盘；根分区保留在旧字段里供摘要使用
    let df = section(raw, "df").unwrap_or_default();
    let disks: Vec<DiskUsage> = df.lines().filter_map(parse_disk_line).collect();
    let root = disks
        .iter()
        .find(|disk| disk.mount == "/")
        .or_else(|| disks.first());
    let disk_total = root.map(|disk| disk.total_kb).unwrap_or(0);
    let disk_used = root.map(|disk| disk.used_kb).unwrap_or(0);

    // /proc/net/dev：跳过 lo，累加其余接口的收发字节
    let net_text = section(raw, "net").unwrap_or_default();
    let (mut rx, mut tx) = (0u64, 0u64);
    for line in net_text.lines() {
        let Some((iface, rest)) = line.split_once(':') else {
            continue;
        };
        if iface.trim() == "lo" {
            continue;
        }
        let cols: Vec<&str> = rest.split_whitespace().collect();
        if cols.len() < 9 {
            continue;
        }
        rx += cols[0].parse::<u64>().unwrap_or(0);
        tx += cols[8].parse::<u64>().unwrap_or(0);
    }
    let net_sample = NetSample { rx, tx };

    let load = section(raw, "load").unwrap_or_default();
    let load1 = load
        .split_whitespace()
        .next()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0);

    let uptime = section(raw, "uptime").unwrap_or_default();
    let uptime_secs = uptime
        .split_whitespace()
        .next()
        .and_then(|v| v.parse::<f64>().ok())
        .map(|v| v as u64)
        .unwrap_or(0);

    let cpu_model = section(raw, "cpuinfo")
        .unwrap_or_default()
        .split_once(':')
        .map(|(_, v)| v.trim().to_string())
        .unwrap_or_default();

    let cores = section(raw, "cores")
        .unwrap_or_default()
        .lines()
        .next()
        .and_then(|v| v.trim().parse::<u32>().ok())
        .unwrap_or(0);

    // 速率：计数回退（重启/回绕）时判为不可用，不给出负数或 >100 的假值
    let (cpu_percent, rx_rate, tx_rate) = match prev {
        Some(p) => {
            let dt = cpu.total.checked_sub(p.cpu.total).filter(|d| *d > 0);
            let di = cpu.idle.checked_sub(p.cpu.idle);
            let cpu_pct = match (dt, di) {
                (Some(dt), Some(di)) if di <= dt => {
                    Some((100.0 * (dt.saturating_sub(di)) as f64 / dt as f64).clamp(0.0, 100.0))
                }
                _ => None,
            };
            let rx_delta = net_sample.rx.checked_sub(p.net.rx);
            let tx_delta = net_sample.tx.checked_sub(p.net.tx);
            (
                cpu_pct,
                rx_delta.map(|d| d as f64),
                tx_delta.map(|d| d as f64),
            )
        }
        None => (None, None, None),
    };

    let snapshot = SysSnapshot {
        cpu_percent,
        mem_total_kb: mem_total,
        mem_used_kb: mem_used,
        swap_total_kb: swap_total,
        swap_used_kb: swap_used,
        disk_total_kb: disk_total,
        disk_used_kb: disk_used,
        disks,
        net_rx_bytes: rx,
        net_tx_bytes: tx,
        net_rx_rate: rx_rate,
        net_tx_rate: tx_rate,
        load1,
        uptime_secs,
        cores,
        cpu_model,
    };
    Some((
        snapshot,
        Baseline {
            cpu,
            net: net_sample,
        },
    ))
}
