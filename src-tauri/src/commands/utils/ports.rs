use super::*;

// ─── 端口 / 进程检测（通用 OS 工具，自 node_manager 迁入） ───

/// 检测端口是否被占用（LISTENING），返回占用进程 PID。
/// 列出监听指定端口的所有进程 PID（任意本地地址，含 0.0.0.0 / 127.0.0.1 / ::）。
///
/// 注意：`TcpListener::bind("0.0.0.0", port)` 的试探绑定**检测不到**回环地址上的
/// 具体绑定（Windows 上 `127.0.0.1:port` 被其他进程占用时，通配绑定依然成功），
/// 因此必须枚举原生监听表（Q-0095：Android 模拟器抢占 127.0.0.1:8554）。
#[cfg(windows)]
pub fn port_owner_pids(port: u16) -> Vec<u32> {
    windows_ports::port_owner_pids(port)
}

#[cfg(not(windows))]
pub fn port_owner_pids(port: u16) -> Vec<u32> {
    // netstat 在连接数极多时输出无上界，走带上限的读取（超限也只是少匹配几行，不会撑爆内存）
    let mut netstat = crate::commands::hidden_cmd::hidden_cmd("netstat");
    netstat.args(["-ano", "-p", "tcp"]);
    let Ok(output) = crate::commands::hidden_cmd::output_capped(
        netstat,
        crate::commands::hidden_cmd::DEFAULT_OUTPUT_CAP,
    ) else {
        return Vec::new();
    };
    let stdout = output.stdout;
    let target = format!(":{}", port);
    let mut pids = Vec::new();
    for line in stdout.lines() {
        let l = line.trim();
        if !l.to_uppercase().starts_with("TCP") {
            continue;
        }
        let fields: Vec<&str> = l.split_whitespace().collect();
        if fields.len() < 5 {
            continue;
        }
        if fields[1].ends_with(&target) && fields[3] == "LISTENING" {
            if let Ok(pid) = fields[4].parse::<u32>() {
                if !pids.contains(&pid) {
                    pids.push(pid);
                }
            }
        }
    }
    pids
}

pub fn port_owner_pid(port: u16) -> Option<u32> {
    port_owner_pids(port).into_iter().next()
}

/// 端口被占用时返回可读的占用者描述（如 `qemu-system-x86_64 (PID 25144)`），空闲返回 `None`。
pub fn port_conflict_description(port: u16) -> Option<String> {
    let pids = port_owner_pids(port);
    if pids.is_empty() {
        return None;
    }
    let mut entries: Vec<String> = Vec::new();
    for pid in &pids {
        let name = process_name_by_pid(*pid).unwrap_or_else(|| format!("PID {}", pid));
        let entry = format!("{} (PID {})", name, pid);
        if !entries.contains(&entry) {
            entries.push(entry);
        }
    }
    Some(entries.join(", "))
}
