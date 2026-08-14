use serde::{Serialize, Deserialize};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PortOwner {
    pub port: String,
    pub pid: String,
    pub process_name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PortStatus {
    pub port: i32,
    pub free: bool,
    pub reserved: bool,
    pub occupied: bool,
    pub owner: Option<PortOwner>,
}

#[derive(Debug)]
struct ExcludedPortRange {
    start: i32,
    end: i32,
}

fn get_excluded_port_ranges() -> Vec<ExcludedPortRange> {
    let mut ranges = Vec::new();
    let output = super::hidden_cmd::hidden_cmd("cmd")
        .args(&["/c", "netsh int ipv4 show excludedportrange protocol=tcp"])
        .output();

    if let Ok(out) = output {
        let text = String::from_utf8_lossy(&out.stdout);
        let mut in_table = false;
        for line in text.lines() {
            let line_trimmed = line.trim();
            if line_trimmed.contains("---") {
                in_table = true;
                continue;
            }
            if !in_table {
                continue;
            }
            let fields: Vec<&str> = line_trimmed.split_whitespace().collect();
            if fields.len() < 2 {
                continue;
            }
            if let (Ok(start), Ok(end)) = (fields[0].parse::<i32>(), fields[1].parse::<i32>()) {
                ranges.push(ExcludedPortRange { start, end });
            }
        }
    }
    ranges
}

fn is_port_reserved(port: i32, ranges: &[ExcludedPortRange]) -> bool {
    for r in ranges {
        if port >= r.start && port <= r.end {
            return true
        }
    }
    false
}

fn find_port_owner(port_str: &str) -> Option<PortOwner> {
    eprintln!("[port] ====== find_port_owner port_str=\"{}\" ======", port_str);

    // Use cmd /c wrapper for reliable execution from GUI subsystem
    // Pass command after /c as single string (standard cmd /c behavior)
    // NOTE: do NOT use `-p tcp` — it drops IPv6 (TCPv6) entries, so a port
    // bound only to [::1] would be missed. `netstat -ano` includes both
    // TCP and TCPv6; our parser accepts any line starting with "TCP".
    let output = match super::hidden_cmd::hidden_cmd("cmd")
        .args(&["/c", "chcp 65001 >nul & netstat -ano"])
        .output()
    {
        Ok(out) => {
            eprintln!(
                "[port] netstat exit_code={:?} stdout_len={} stderr_len={}",
                out.status.code(),
                out.stdout.len(),
                out.stderr.len()
            );
            if !out.status.success() {
                eprintln!("[port] netstat stderr: {}", String::from_utf8_lossy(&out.stderr));
            }
            // Log first 500 chars of stdout
            let stdout_preview = String::from_utf8_lossy(&out.stdout);
            let preview = if stdout_preview.len() > 500 { &stdout_preview[..500] } else { &stdout_preview };
            eprintln!("[port] netstat stdout preview (first 500):\n{}", preview);
            // Count matching lines
            let port_keyword = format!(":{}", port_str);
            let matching_lines: Vec<&str> = stdout_preview.lines()
                .filter(|l| l.contains(&port_keyword))
                .collect();
            eprintln!("[port] lines containing \":{}\": {} total", port_str, matching_lines.len());
            for (i, l) in matching_lines.iter().enumerate().take(10) {
                eprintln!("[port]   match[{}]: {}", i, l);
            }
            out
        }
        Err(e) => {
            eprintln!("[port] netstat 启动失败: {}", e);
            return None;
        }
    };

    let text = String::from_utf8_lossy(&output.stdout);
    let mut pid = String::new();
    let mut lines_scanned = 0usize;
    let mut tcp_lines = 0usize;
    let mut lines_skipped_short = 0usize;

    for line in text.lines() {
        lines_scanned += 1;
        let line_trimmed = line.trim();
        if line_trimmed.is_empty() || !line_trimmed.to_uppercase().starts_with("TCP") {
            continue;
        }
        tcp_lines += 1;
        let fields: Vec<&str> = line_trimmed.split_whitespace().collect();
        if fields.len() < 5 {
            lines_skipped_short += 1;
            eprintln!("[port] SKIP (fields<5) line: {}", line_trimmed);
            continue;
        }
        let local_addr = fields[1];
        let state = fields[3];
        let row_pid = fields[4];

        let mut row_port = "";
        if local_addr.contains(']') {
            let parts: Vec<&str> = local_addr.split("]:").collect();
            if parts.len() == 2 {
                row_port = parts[1];
            }
        } else {
            let parts: Vec<&str> = local_addr.split(':').collect();
            if !parts.is_empty() {
                row_port = parts[parts.len() - 1];
            }
        }

        // Log every port we're checking that matches the target number
        if row_port == port_str {
            eprintln!(
                "[port] CANDIDATE local={} state={} pid={} row_port={} MATCH_STATE={}",
                local_addr, state, row_pid, row_port,
                state == "LISTENING"
            );
        }

        if row_port == port_str && state == "LISTENING" {
            pid = row_pid.to_string();
            eprintln!("[port] FOUND LISTENING: port={} pid={}", port_str, pid);
            break;
        }
    }

    eprintln!(
        "[port] scan summary: total_lines={} tcp_lines={} skipped_short={} pid_found=\"{}\"",
        lines_scanned, tcp_lines, lines_skipped_short, pid
    );

    if pid.is_empty() {
        eprintln!("[port] NO LISTENING PID found, returning None");
        return None;
    }

    // tasklist to find process name (via cmd /c wrapper)
    // Use raw_arg so Rust does NOT auto-add outer quotes around the command.
    // `cmd /c "tasklist /fi \"pid eq X\" ..."` (auto-quoted) breaks tasklist's
    // filter parser; we need `cmd /c tasklist /fi "pid eq X" /fo csv /nh`.
    let tasklist_cmd = format!("chcp 65001 >nul & tasklist /fi \"pid eq {}\" /fo csv /nh", pid);
    let task_output = match super::hidden_cmd::hidden_cmd("cmd")
        .raw_arg("/c")
        .raw_arg(&tasklist_cmd)
        .output()
    {
        Ok(out) => {
            eprintln!(
                "[port] tasklist PID={}: exit={:?} stdout_len={}",
                pid, out.status.code(), out.stdout.len()
            );
            let stdout_preview = String::from_utf8_lossy(&out.stdout);
            eprintln!("[port] tasklist stdout: {}", stdout_preview.trim());
            out
        }
        Err(e) => {
            eprintln!("[port] tasklist 启动失败 (PID={}): {}", pid, e);
            return Some(PortOwner {
                port: port_str.to_string(),
                pid: pid.clone(),
                process_name: "Unknown".to_string(),
            });
        }
    };

    let task_text = String::from_utf8_lossy(&task_output.stdout).trim().to_string();
    let mut process_name = "Unknown".to_string();
    if !task_text.is_empty() {
        let parts: Vec<&str> = task_text.split(',').collect();
        if !parts.is_empty() {
            process_name = parts[0].trim_matches('"').to_string();
        }
    }
    eprintln!("[port] RESULT port={} pid={} process={}", port_str, pid, process_name);

    Some(PortOwner {
        port: port_str.to_string(),
        pid,
        process_name,
    })
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ReservedPortRange {
    pub start: i32,
    pub end: i32,
    pub process: String,
}

#[tauri::command]
pub fn get_reserved_ports() -> Result<Vec<ReservedPortRange>, String> {
    let output = super::hidden_cmd::hidden_cmd("cmd")
        .args(&["/c", "netsh int ipv4 show excludedportrange protocol=tcp"])
        .output()
        .map_err(|e| format!("执行 netsh 失败: {}", e))?;

    let text = String::from_utf8_lossy(&output.stdout);
    let mut ranges = Vec::new();
    let mut in_table = false;

    for line in text.lines() {
        let line_trimmed = line.trim();
        if line_trimmed.contains("---") {
            in_table = true;
            continue;
        }
        if !in_table || line_trimmed.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line_trimmed.split_whitespace().collect();
        if fields.len() < 2 {
            continue;
        }
        if let (Ok(start), Ok(end)) = (fields[0].parse::<i32>(), fields[1].parse::<i32>()) {
            let process = if fields.len() > 2 { fields[2..].join(" ") } else { String::new() };
            ranges.push(ReservedPortRange { start, end, process });
        }
    }

    ranges.sort_by_key(|r| r.start);
    Ok(ranges)
}

#[tauri::command]
pub fn check_port_status(port_str: String) -> Result<PortStatus, String> {
    let port = port_str.parse::<i32>().map_err(|_| "端口号无效".to_string())?;
    let mut status = PortStatus {
        port,
        free: true,
        reserved: false,
        occupied: false,
        owner: None,
    };

    if let Some(owner) = find_port_owner(&port_str) {
        status.occupied = true;
        status.free = false;
        status.owner = Some(owner);
    }

    let ranges = get_excluded_port_ranges();
    if is_port_reserved(port, &ranges) {
        status.reserved = true;
        if !status.occupied {
            status.free = false;
        }
    }

    eprintln!(
        "[port] check_port_status({}) => free={} reserved={} occupied={} owner={:?}",
        port_str, status.free, status.reserved, status.occupied, status.owner
    );
    Ok(status)
}

#[tauri::command]
pub fn kill_port_owner(port_str: String) -> Result<String, String> {
    let port = port_str.parse::<i32>().map_err(|_| "端口号无效".to_string())?;
    let ranges = get_excluded_port_ranges();
    if is_port_reserved(port, &ranges) {
        return Err(format!("端口 {} 位于 Windows 保留端口范围内，无法强行释放", port_str));
    }

    let owner = find_port_owner(&port_str).ok_or_else(|| format!("未找到占用端口 {} 的进程", port_str))?;

    // 先尝试普通权限杀死
    let taskkill_cmd = format!("taskkill /f /pid {}", owner.pid);
    let output = super::hidden_cmd::hidden_cmd("cmd")
        .args(&["/c", &taskkill_cmd])
        .output()
        .map_err(|e| e.to_string())?;

    if output.status.success() {
        return Ok(format!("成功终止进程 {} (PID: {})", owner.process_name, owner.pid));
    }

    // taskkill 失败，检查错误原因
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();

    // 检测是否是权限不足（拒绝访问）
    let is_access_denied = stderr.contains("拒绝访问") || stdout.contains("拒绝访问")
        || stderr.contains("Access is denied") || stdout.contains("Access is denied");

    if is_access_denied {
        return Err(format!(
            "权限不足：无法终止进程 {} (PID: {})。该进程可能需要管理员权限才能终止。\n\n解决方案：\n1. 以管理员身份运行 AnyVersion\n2. 或在任务管理器中手动结束该进程\n\n原始错误：{}",
            owner.process_name, owner.pid, stderr.trim()
        ));
    }

    // 其他错误
    Err(format!(
        "终止进程失败 (PID: {})。\n进程: {}\n错误: {}",
        owner.pid,
        owner.process_name,
        stderr.trim().chars().take(500).collect::<String>()
    ))
}
