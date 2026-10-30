use super::*;

#[cfg(windows)]
pub(super) fn find_running_system_service(def: &ProjectDef) -> Option<String> {
    if def.service_names.is_empty() {
        return None;
    }
    let patterns: Vec<regex::Regex> = def
        .service_names
        .iter()
        .filter_map(|p| {
            regex::RegexBuilder::new(p)
                .case_insensitive(true)
                .build()
                .ok()
        })
        .collect();
    if patterns.is_empty() {
        return None;
    }

    let output = crate::commands::hidden_cmd::hidden_cmd("sc")
        .args(&["query", "type=", "service", "state=", "all"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);

    let mut current_service_name = String::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("SERVICE_NAME:") {
            current_service_name = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("STATE") {
            let state_val = rest.trim_start_matches(':').trim();
            if state_val.contains("RUNNING") && !current_service_name.is_empty() {
                if patterns.iter().any(|re| re.is_match(&current_service_name)) {
                    return Some(current_service_name);
                }
            }
        }
    }
    None
}

#[cfg(windows)]
pub(super) fn find_registered_system_service(def: &ProjectDef) -> Option<String> {
    if def.service_names.is_empty() {
        return None;
    }
    let patterns: Vec<regex::Regex> = def
        .service_names
        .iter()
        .filter_map(|p| {
            regex::RegexBuilder::new(p)
                .case_insensitive(true)
                .build()
                .ok()
        })
        .collect();
    if patterns.is_empty() {
        return None;
    }

    let output = crate::commands::hidden_cmd::hidden_cmd("sc")
        .args(&["query", "type=", "service", "state=", "all"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);

    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("SERVICE_NAME:") {
            let name = rest.trim().to_string();
            if patterns.iter().any(|re| re.is_match(&name)) {
                return Some(name);
            }
        }
    }
    None
}

#[cfg(not(windows))]
pub(super) fn find_registered_system_service(_def: &ProjectDef) -> Option<String> {
    None
}

/// 服务进程已在后台启动后，轮询等待其真正就绪（端口 LISTENING / 进程匹配），
/// 避免 detached 模式下启动慢导致前端刷新到未就绪状态而产生「启动中→停止」闪烁。
/// 最多等待 `timeout` 秒（默认 15s），每 500ms 探测一次；一旦探测到 running 立即返回 Ok。
/// 超时后若进程仍存在（pid 非空），仍视为启动成功（端口探测可能尚未稳定或未配置端口）；
/// 进程也不存在则判定启动失败并返回错误。
pub(super) fn wait_for_service_running(def: &ProjectDef) -> Result<(), String> {
    let timeout = Duration::from_secs(15);
    let deadline = Instant::now() + timeout;
    loop {
        if service_status_for_def(def).running {
            return Ok(());
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }

    // 超时兜底：端口可能尚未探测到，但进程确实起来了，认为启动成功
    if service_status_for_def(def).pid.is_some() {
        return Ok(());
    }
    Err(format!(
        "服务 {} 启动后未在 {} 秒内就绪，请检查日志或端口是否被占用。",
        def.display_name,
        timeout.as_secs()
    ))
}
