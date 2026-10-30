use super::*;

pub(crate) fn read_port_from_ini(ini_path: &Path, key: &str) -> String {
    if let Ok(content) = fs::read_to_string(ini_path) {
        for line in content.lines() {
            let line_trimmed = line.trim();
            if line_trimmed.starts_with('#') || line_trimmed.starts_with(';') {
                continue;
            }
            if line_trimmed.to_lowercase().starts_with(key) {
                let parts = line_trimmed.splitn(2, '=').collect::<Vec<_>>();
                if parts.len() == 2 {
                    return parts[1].trim().trim_matches('"').to_string();
                }
            }
        }
    }
    String::new()
}

pub(crate) fn read_port_from_conf(conf_path: &Path, key: &str) -> String {
    if let Ok(content) = fs::read_to_string(conf_path) {
        for line in content.lines() {
            let line_trimmed = line.trim();
            if line_trimmed.starts_with('#') || line_trimmed.starts_with(';') {
                continue;
            }
            if line_trimmed.to_lowercase().starts_with(key) {
                if line_trimmed.contains('=') {
                    let parts = line_trimmed.splitn(2, '=').collect::<Vec<_>>();
                    if parts.len() == 2 {
                        return parts[1]
                            .trim()
                            .trim_matches('"')
                            .trim_matches('\'')
                            .to_string();
                    }
                }
                let fields = line_trimmed.split_whitespace().collect::<Vec<_>>();
                if fields.len() >= 2 && fields[0].to_lowercase() == key {
                    return fields[1].trim_matches('"').trim_matches('\'').to_string();
                }
            }
        }
    }
    String::new()
}

pub(crate) fn read_nginx_port(conf_path: &Path) -> String {
    if let Ok(content) = fs::read_to_string(conf_path) {
        for line in content.lines() {
            let line_trimmed = line.trim();
            if line_trimmed.starts_with('#') || !line_trimmed.starts_with("listen") {
                continue;
            }
            let listen_line = line_trimmed
                .trim_start_matches("listen")
                .trim()
                .trim_end_matches(';')
                .trim();
            if let Some(first) = listen_line.split_whitespace().next() {
                let port = first.rsplit(':').next().unwrap_or(first);
                return port.trim().to_string();
            }
        }
    }
    String::new()
}

pub(crate) struct PortOwnerSimple {
    pub(crate) pid: String,
    pub(crate) process_name: String,
}

pub(crate) fn find_port_owner_simple(port_str: &str) -> Option<PortOwnerSimple> {
    let output = crate::commands::hidden_cmd::hidden_cmd("netstat")
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

        let row_port = if local_addr.contains(']') {
            local_addr.split("]:").nth(1).unwrap_or("")
        } else {
            local_addr.rsplit(':').next().unwrap_or("")
        };

        if row_port == port_str && state == "LISTENING" {
            pid = row_pid.to_string();
            break;
        }
    }

    if pid.is_empty() {
        return None;
    }

    let process_name = process_name_by_pid(&pid).unwrap_or_else(|| "Unknown".to_string());
    Some(PortOwnerSimple { pid, process_name })
}
