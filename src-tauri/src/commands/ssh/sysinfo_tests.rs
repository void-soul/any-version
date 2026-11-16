//! 服务器状态采样的纯解析测试（用合成 /proc 内容，不触网）。
use super::sysinfo::{parse_sample, sample_command, CpuSample, SysSnapshot};

const RAW: &str = "\
---stat
cpu  1000 20 300 8000 100 0 10 0 0 0
---mem
MemTotal:       16000000 kB
MemFree:         2000000 kB
MemAvailable:    6000000 kB
---swap
SwapTotal:       2000000 kB
SwapFree:        1500000 kB
---df
/dev/vda1 100000000 30000000 70000000 30% /
/dev/vdb1 200000000 50000000 150000000 25% /data
tmpfs 8000000 1000 7999000 1% /run
devtmpfs 4000000 0 4000000 0% /dev
---net
    lo: 100 0 0 0 0 0 0 0 200 0 0 0 0 0 0 0
  eth0: 5000 0 0 0 0 0 0 0 3000 0 0 0 0 0 0 0
---load
0.50 0.40 0.30 1/200 9999
---uptime
12345.67 99999.00
---cpuinfo
model name\t: Synthetic CPU @ 2.0GHz
---cores
4
";

fn parsed() -> SysSnapshot {
    parse_sample(RAW, None).expect("应能解析完整样本").0
}

#[test]
fn identity_fields_are_parsed() {
    let s = parsed();
    assert_eq!(s.mem_total_kb, 16_000_000);
    assert_eq!(s.swap_total_kb, 2_000_000);
    assert_eq!(s.disk_total_kb, 100_000_000);
    assert_eq!(s.disk_used_kb, 30_000_000);
    assert_eq!(s.load1, 0.5);
    assert_eq!(s.uptime_secs, 12345);
    assert_eq!(s.cores, 4);
    assert!(s.cpu_model.contains("Synthetic CPU"));
}

/// 服务器通常不止一块盘：所有真实挂载点都要出来，内存盘不要混进去。
#[test]
fn multiple_disks_are_parsed_and_memory_fs_is_skipped() {
    let s = parsed();
    assert_eq!(s.disks.len(), 2, "只保留真实块设备");
    assert_eq!(s.disks[0].mount, "/");
    assert_eq!(s.disks[1].mount, "/data");
    assert_eq!(s.disks[1].total_kb, 200_000_000);
    assert_eq!(s.disks[1].percent, 25);
}

#[test]
fn memory_used_uses_available_not_free() {
    let s = parsed();
    // 用 MemAvailable（含可回收缓存）而不是 MemFree，否则 Linux 上会长期显示 90% 占用
    assert_eq!(s.mem_used_kb, 16_000_000 - 6_000_000);
    assert_eq!(s.swap_used_kb, 500_000);
}

#[test]
fn network_sums_only_non_loopback() {
    let s = parsed();
    assert_eq!(s.net_rx_bytes, 5000, "lo 不能计入");
    assert_eq!(s.net_tx_bytes, 3000);
}

/// 首帧没有上一份样本，CPU/网络速率应为 None 而不是 0（0 会被误读为“完全空闲”）。
#[test]
fn first_sample_has_no_rates() {
    let s = parsed();
    assert_eq!(s.cpu_percent, None);
    assert_eq!(s.net_rx_rate, None);
    assert_eq!(s.net_tx_rate, None);
}

#[test]
fn second_sample_computes_cpu_and_network_rates() {
    let (_, prev) = parse_sample(RAW, None).unwrap();
    // 再过 1 秒：总时间 +100，空闲 +40 → 忙 60%；网络各 +1000 字节
    let next = RAW
        .replace(
            "cpu  1000 20 300 8000 100 0 10 0 0 0",
            "cpu  1060 20 300 8040 100 0 10 0 0 0",
        )
        .replace("eth0: 5000", "eth0: 6000")
        .replace("eth0: 5000", "eth0: 6000")
        .replace("3000 0 0 0 0 0 0 0", "4000 0 0 0 0 0 0 0");
    let (s, _) = parse_sample(&next, Some(&prev)).expect("第二帧应能解析");
    let cpu = s.cpu_percent.expect("应有 CPU 百分比");
    assert!((cpu - 60.0).abs() < 1.0, "CPU 应约 60%，实际 {cpu}");
    assert_eq!(s.net_rx_rate, Some(1000.0));
    assert_eq!(s.net_tx_rate, Some(1000.0));
}

#[test]
fn malformed_sections_are_tolerated() {
    let partial = "---mem\nMemTotal: 1000 kB\nMemAvailable: 400 kB\n";
    let (s, _) = parse_sample(partial, None).expect("缺段也应尽量解析");
    assert_eq!(s.mem_total_kb, 1000);
    assert_eq!(s.mem_used_kb, 600);
    assert_eq!(s.disk_total_kb, 0);
    assert_eq!(s.cores, 0);
}

#[test]
fn empty_input_is_rejected() {
    assert!(parse_sample("", None).is_none());
    assert!(parse_sample("garbage without sections", None).is_none());
}

/// CPU 计数溢出/回绕在一帧内出现时，宁可给 None 也不要给负数或 >100 的值。
#[test]
fn counter_regression_does_not_produce_bogus_rates() {
    let (_, prev) = parse_sample(RAW, None).unwrap();
    let regressed = RAW.replace(
        "cpu  1000 20 300 8000 100 0 10 0 0 0",
        "cpu  900 20 300 7900 100 0 10 0 0 0",
    );
    let (s, _) = parse_sample(&regressed, Some(&prev)).unwrap();
    assert_eq!(s.cpu_percent, None, "计数回退应判为不可用");
}

#[test]
fn sample_command_covers_every_section_the_parser_expects() {
    let cmd = sample_command();
    for marker in [
        "---stat",
        "---mem",
        "---swap",
        "---df",
        "---net",
        "---load",
        "---uptime",
        "---cpuinfo",
        "---cores",
    ] {
        assert!(cmd.contains(marker), "采样命令缺少 {marker}");
    }
}

#[test]
fn cpu_sample_tracks_aggregate_counters() {
    let CpuSample { total, idle } = parsed_sample_cpu();
    assert_eq!(total, 1000 + 20 + 300 + 8000 + 100 + 0 + 10 + 0);
    assert_eq!(idle, 8000 + 100);
}

fn parsed_sample_cpu() -> CpuSample {
    parse_sample(RAW, None).unwrap().1.cpu
}
