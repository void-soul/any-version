use std::process::Command;
use serde::{Serialize, Deserialize};

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
    let output = Command::new("netsh")
        .args(&["int", "ipv4", "show", "excludedportrange", "protocol=tcp"])
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
    let output = Command::new("netstat")
        .args(&["-ano", "-p", "tcp"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let mut pid = String::new();

    for line in text.lines() {
        let line_trimmed = line.trim();
        if line_trimmed.is_empty() || !line_trimmed.to_uppercase().starts_with("TCP") {
            continue;
        }
        let fields: Vec<&str> = line_trimmed.split_whitespace().collect();
        if fields.len() < 5 {
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

        if row_port == port_str && state == "LISTENING" {
            pid = row_pid.to_string();
            break;
        }
    }

    if pid.is_empty() {
        return None;
    }

    // tasklist to find process name
    let task_output = Command::new("tasklist")
        .args(&["/fi", &format!("pid eq {}", pid), "/fo", "csv", "/nh"])
        .output()
        .ok()?;
    let task_text = String::from_utf8_lossy(&task_output.stdout).trim().to_string();
    let mut process_name = "Unknown".to_string();
    if !task_text.is_empty() {
        let parts: Vec<&str> = task_text.split(',').collect();
        if !parts.is_empty() {
            process_name = parts[0].trim_matches('"').to_string();
        }
    }

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
    let output = Command::new("netsh")
        .args(&["int", "ipv4", "show", "excludedportrange", "protocol=tcp"])
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

    Ok(status)
}

#[tauri::command]
pub fn kill_port_owner(port_str: String) -> Result<(), String> {
    let port = port_str.parse::<i32>().map_err(|_| "端口号无效".to_string())?;
    let ranges = get_excluded_port_ranges();
    if is_port_reserved(port, &ranges) {
        return Err(format!("端口 {} 位于 Windows 保留端口范围内，无法强行释放", port_str));
    }

    let owner = find_port_owner(&port_str).ok_or_else(|| format!("未找到占用端口 {} 的进程", port_str))?;
    let output = Command::new("taskkill")
        .args(&["/f", "/pid", &owner.pid])
        .output()
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }

    Ok(())
}
