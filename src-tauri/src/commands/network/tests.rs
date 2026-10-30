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
    assert_eq!(
        normalize_host("https://github.com/a/b").unwrap(),
        "github.com"
    );
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
