//! 远端进程与监听端口：按需在监控会话上执行一条只读命令，再解析成表格行。
//!
//! 与 [`super::sysinfo`] 的区别：状态采样每 2 秒跑一次，进程/网络列表通常只在
//! 用户打开面板时拉一次，所以单独放，避免把重命令塞进轮询。

/// 200 条足够覆盖常见服务器；按 CPU 降序，和 FinalShell 的默认排序一致。
pub const PROCESS_COMMAND: &str =
    "ps -eo pid=,user=,rss=,pcpu=,comm=,args= --sort=-pcpu 2>/dev/null | head -n 200";

/// `ss` 在主流发行版都有；没有时前端会显示空表而不是报错。
pub const NET_COMMAND: &str = "ss -tulnp 2>/dev/null";

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessRow {
    pub pid: i64,
    pub user: String,
    /// 常驻内存（KB）
    pub rss_kb: u64,
    pub cpu: f64,
    pub name: String,
    pub command: String,
    /// 可执行文件路径（命令行第一段）
    pub location: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetLinkRow {
    pub pid: i64,
    pub name: String,
    pub proto: String,
    pub state: String,
    pub local_ip: String,
    pub port: u16,
}

pub fn parse_processes(output: &str) -> Vec<ProcessRow> {
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let pid: i64 = parts.next()?.parse().ok()?;
            let user = parts.next()?.to_string();
            let rss_kb: u64 = parts.next()?.parse().ok()?;
            let cpu: f64 = parts.next()?.parse().ok()?;
            let name = parts.next()?.to_string();
            let command = parts.collect::<Vec<_>>().join(" ");
            let location = command.split_whitespace().next().unwrap_or("").to_string();
            Some(ProcessRow {
                pid,
                user,
                rss_kb,
                cpu,
                name,
                command,
                location,
            })
        })
        .collect()
}

pub fn parse_netlinks(output: &str) -> Vec<NetLinkRow> {
    output
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            // 有表头就跳过，没有表头也能解析（不同发行版的 ss 输出略有差异）
            !trimmed.is_empty() && !trimmed.starts_with("Netid")
        })
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let proto = parts.next()?.to_string();
            let state = parts.next()?.to_string();
            let _recv_q = parts.next()?;
            let _send_q = parts.next()?;
            let local = parts.next()?;
            let _peer = parts.next()?;
            let rest = parts.collect::<Vec<_>>().join(" ");
            let (local_ip, port) = local.rsplit_once(':')?;
            let port: u16 = port.parse().ok()?;
            let (name, pid) = parse_process(&rest);
            Some(NetLinkRow {
                pid,
                name,
                proto,
                state,
                local_ip: local_ip.to_string(),
                port,
            })
        })
        .collect()
}

/// 从 `users:(("sshd",pid=1110,fd=3))` 里取进程名与 pid；无权限时两者都为空/0。
fn parse_process(text: &str) -> (String, i64) {
    let name = text.split('"').nth(1).unwrap_or("").to_string();
    let pid = text
        .split("pid=")
        .nth(1)
        .and_then(|rest| {
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            digits.parse().ok()
        })
        .unwrap_or(0);
    (name, pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PS: &str = "\
2329136 root 186508 17.0 node node --no-deprecation --require /data/services/x.js\n\
192669 mysql 53100 1.0 mysqld /usr/sbin/mysqld\n\
940 redis 6410 0.3 redis-server /usr/bin/redis-server 127.0.0.1:6379\n";

    #[test]
    fn parse_processes_reads_pid_user_rss_cpu() {
        let rows = parse_processes(PS);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].pid, 2329136);
        assert_eq!(rows[0].user, "root");
        assert_eq!(rows[0].rss_kb, 186508);
        assert!((rows[0].cpu - 17.0).abs() < 0.01);
        assert_eq!(rows[0].name, "node");
        assert!(rows[0].command.starts_with("node --no-deprecation"));
        assert_eq!(rows[0].location, "node");
        assert_eq!(rows[1].location, "/usr/sbin/mysqld");
    }

    #[test]
    fn parse_processes_skips_blank_and_malformed_lines() {
        let rows = parse_processes("\nnot-a-process\n\n");
        assert!(rows.is_empty());
    }

    const SS: &str = "\
Netid State  Recv-Q Send-Q Local Address:Port Peer Address:Port Process\n\
tcp   LISTEN 0      511          0.0.0.0:80        0.0.0.0:*     users:((\"nginx\",pid=2643998,fd=6))\n\
tcp   LISTEN 0      128          0.0.0.0:22        0.0.0.0:*     users:((\"sshd\",pid=1110,fd=3))\n\
udp   UNCONN 0      0            127.0.0.1:6379    0.0.0.0:*     users:((\"redis-server\",pid=940,fd=6))\n\
tcp   LISTEN 0      4096               [::]:9090         [::]:*     users:((\"frps\",pid=1881948,fd=3))\n\
";

    #[test]
    fn parse_netlinks_reads_listeners() {
        let rows = parse_netlinks(SS);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].proto, "tcp");
        assert_eq!(rows[0].state, "LISTEN");
        assert_eq!(rows[0].local_ip, "0.0.0.0");
        assert_eq!(rows[0].port, 80);
        assert_eq!(rows[0].name, "nginx");
        assert_eq!(rows[0].pid, 2643998);
        assert_eq!(rows[3].local_ip, "[::]");
        assert_eq!(rows[3].port, 9090);
    }

    #[test]
    fn parse_netlinks_keeps_rows_without_process_info() {
        let rows = parse_netlinks("tcp LISTEN 0 128 0.0.0.0:8080 0.0.0.0:*\n");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].pid, 0);
        assert_eq!(rows[0].name, "");
    }
}
