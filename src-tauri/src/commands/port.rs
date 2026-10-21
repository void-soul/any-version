use serde::{Serialize, Deserialize};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PortOwner {
    pub port: String,
    pub pid: String,
    pub process_name: String,
    /// true = 占用者就是 Kira 自己（HTTP 静态服务 / 聚合服务都跑在主进程里）。
    /// 前端据此不给「结束进程」按钮 —— 否则 taskkill 会把宿主自己杀掉。
    #[serde(default)]
    pub self_owned: bool,
}

/// 该 PID 是不是本进程。内置服务（HTTP / 聚合）监听的端口在 netstat 里
/// 显示为本进程 PID，杀它就是自杀，必须显式识别出来。
fn is_self_pid(pid: &str) -> bool {
    !pid.trim().is_empty() && pid.trim() == std::process::id().to_string()
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PortStatus {
    pub port: i32,
    pub free: bool,
    /// true = 落在**系统动态保留段**内：这类端口真正绑不上（WSAEACCES 10013）。
    pub reserved: bool,
    /// true = 已被「禁止被预留」加进**管理员排除段**：系统不会再把它动态分出去，
    /// 应用仍可显式 bind —— 和 `reserved` 是两件相反的事，不能合并。
    #[serde(default)]
    pub protected: bool,
    pub occupied: bool,
    pub owner: Option<PortOwner>,
}

/// `netsh ... show excludedportrange` 的一行结果。
#[derive(Debug, Clone)]
struct ExcludedPortRange {
    start: i32,
    end: i32,
    /// true = 系统**动态**保留（Hyper-V / WSL / 容器网络在运行时占用），
    /// 不是用户手工加的段，通常也删不掉。判定逻辑见 `parse_excluded_ranges` ——
    /// 新旧 Windows 的 `*` 位置与含义相反，不能只看行首。
    managed: bool,
    /// 行尾备注（多为触发保留的组件名），没有则空串
    label: String,
}

/// 解析 `netsh int ipv4 show excludedportrange` 输出。
///
/// **两种格式都要兼容**（星号含义在新旧 Windows 上是反的）：
/// - 旧版（Win10 早期）：行首 `*` = 系统动态保留（Hyper-V / WSL 抢的那批），
///   没有标记的是管理员静态排除；
/// - 新版（Win10 2004+ / Win11）：行尾独立 `*` = **管理员手工排除**（脚注
///   `* - Administered port exclusions.`），反而**没有**标记的行才是动态保留。
///
/// 原实现只认行首 `*`，在新格式机器上会把动态段当静态段（「来源」列错、删除按钮
/// 错放），行尾 `*` 还会漏进「关联进程」列。格式判定用脚注行（`* - ...`，与
/// 显示语言无关），单条数据行则按「行首星 > 行尾星 > 组件名标签 > 格式默认」归类。
fn parse_excluded_ranges(text: &str) -> Vec<ExcludedPortRange> {
    let mut ranges = Vec::new();
    let mut in_table = false;
    // 脚注行 `* - Administered port exclusions.` 只在新格式里出现
    let new_format = text.lines().any(|l| {
        let t = l.trim();
        t.starts_with("* -") || t.starts_with("*-")
    });
    for line in text.lines() {
        let line_trimmed = line.trim();
        if line_trimmed.contains("---") {
            in_table = true;
            continue;
        }
        if !in_table || line_trimmed.is_empty() {
            continue;
        }
        let leading_star = line_trimmed.starts_with('*');
        let fields: Vec<&str> = line_trimmed
            .trim_start_matches('*')
            .split_whitespace()
            .collect();
        if fields.len() < 2 {
            continue;
        }
        let (Ok(start), Ok(end)) = (fields[0].parse::<i32>(), fields[1].parse::<i32>()) else {
            continue;
        };
        let rest = &fields[2..];
        let trailing_star = rest.last() == Some(&"*");
        let label = if trailing_star {
            rest[..rest.len() - 1].join(" ")
        } else {
            rest.join(" ")
        };
        let managed = if leading_star {
            true // 旧格式：行首 * = 系统动态保留
        } else if trailing_star {
            false // 新格式：行尾 * = 管理员手工排除
        } else if !label.is_empty() {
            true // 带组件名（Hyper-V 等）= 动态保留
        } else {
            new_format // 新格式无标记行 = 动态保留；旧格式无标记行 = 静态排除
        };
        ranges.push(ExcludedPortRange { start, end, managed, label });
    }
    ranges.sort_by_key(|r| r.start);
    ranges
}

fn get_excluded_port_ranges() -> Vec<ExcludedPortRange> {
    // chcp 65001 与 netstat / tasklist 两处保持一致：不加的话中文 Windows 上
    // 「关联进程」列（Hyper-V 之类）会按 GBK 落到 from_utf8_lossy 里变乱码。
    let output = super::hidden_cmd::hidden_cmd("cmd")
        .args(&[
            "/c",
            "chcp 65001 >nul & netsh int ipv4 show excludedportrange protocol=tcp",
        ])
        .output();

    match output {
        Ok(out) => parse_excluded_ranges(&String::from_utf8_lossy(&out.stdout)),
        Err(_) => Vec::new(),
    }
}

fn is_port_reserved(port: i32, ranges: &[ExcludedPortRange]) -> bool {
    for r in ranges {
        if port >= r.start && port <= r.end {
            return true
        }
    }
    false
}

/// 端口是否已被**静态排除段**（管理员手工添加、持久生效的那种）覆盖。
/// 动态保留段不算 —— 它随时可能消失，不能当作「已禁止被预留」的凭据。
fn is_port_statically_excluded(port: i32, ranges: &[ExcludedPortRange]) -> bool {
    ranges.iter().any(|r| !r.managed && port >= r.start && port <= r.end)
}

/// 覆盖该端口的**系统动态保留段**（Hyper-V / WSL / 容器网络抢的那批）。
/// 在段内 = 这个端口真的绑不上，是「禁止被预留」唯一要消灭的目标。
fn find_dynamic_range(port: i32, ranges: &[ExcludedPortRange]) -> Option<&ExcludedPortRange> {
    ranges
        .iter()
        .find(|r| r.managed && port >= r.start && port <= r.end)
}

pub(crate) fn find_port_owner(port_str: &str) -> Option<PortOwner> {
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
                self_owned: is_self_pid(&pid),
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
        self_owned: is_self_pid(&pid),
        pid,
        process_name,
    })
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ReservedPortRange {
    pub start: i32,
    pub end: i32,
    pub process: String,
    /// true = 系统**动态**保留（netsh 输出带 `*`），由 Hyper-V / WSL 等占用，
    /// 不是用户添加的段，删除通常会被系统拒绝。
    #[serde(default)]
    pub managed: bool,
}

#[tauri::command]
pub fn get_reserved_ports() -> Result<Vec<ReservedPortRange>, String> {
    let output = super::hidden_cmd::hidden_cmd("cmd")
        .args(&[
            "/c",
            "chcp 65001 >nul & netsh int ipv4 show excludedportrange protocol=tcp",
        ])
        .output()
        .map_err(|e| format!("执行 netsh 失败: {}", e))?;

    let text = String::from_utf8_lossy(&output.stdout).to_string();
    let ranges = parse_excluded_ranges(&text)
        .into_iter()
        .map(|r| ReservedPortRange {
            start: r.start,
            end: r.end,
            process: r.label,
            managed: r.managed,
        })
        .collect();
    Ok(ranges)
}

/// 校验并规范化协议参数（只支持 tcp / udp，默认 tcp）。
fn normalize_protocol(protocol: Option<&str>) -> Result<&'static str, String> {
    match protocol.map(|p| p.trim().to_lowercase()).as_deref() {
        None | Some("") | Some("tcp") => Ok("tcp"),
        Some("udp") => Ok("udp"),
        Some(other) => Err(format!("不支持的协议: {}（只支持 tcp / udp）", other)),
    }
}

/// 校验起始端口与数量：1..65535 且不能越界；数量上限 1000（一次抢太多会拖垮系统端口池）。
fn validate_range(start: i32, count: i32) -> Result<(), String> {
    if start < 1 || start > 65535 {
        return Err(format!("起始端口必须在 1..65535 之间，当前为 {}", start));
    }
    if count < 1 {
        return Err("端口数量必须大于 0".to_string());
    }
    if count > 1000 {
        return Err("一次最多只能操作 1000 个端口".to_string());
    }
    if start + count - 1 > 65535 {
        return Err(format!(
            "端口范围越界：{}-{} 超出了 65535",
            start,
            start + count - 1
        ));
    }
    Ok(())
}

/// 执行 `netsh int ipv4 <add|delete> excludedportrange`（**需要管理员权限**）。
///
/// 失败时优先给出可操作提示：权限不足 → 提示以管理员身份运行；其余原样返回 netsh 输出。
fn run_netsh_range(verb: &str, protocol: Option<&str>, start: i32, count: i32) -> Result<(), String> {
    let proto = normalize_protocol(protocol)?;
    validate_range(start, count)?;

    let cmdline = format!(
        "netsh int ipv4 {} excludedportrange protocol={} startport={} numberofports={}",
        verb, proto, start, count
    );
    let output = super::hidden_cmd::hidden_cmd("cmd")
        .args(&["/c", &cmdline])
        .output()
        .map_err(|e| format!("执行 netsh 失败: {}", e))?;

    if output.status.success() {
        return Ok(());
    }
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let lower = text.to_lowercase();
    if lower.contains("拒绝访问")
        || lower.contains("access is denied")
        || lower.contains("需要提升")
        || lower.contains("elevation")
        || lower.contains("run as administrator")
    {
        return Err("需要管理员权限：请以管理员身份运行 AnyVersion 后再操作保留端口".to_string());
    }
    Err(text.trim().to_string())
}

/// 添加一段 Windows 保留端口（排除范围）：这些端口不会被系统分配给其它程序。
/// 需要管理员权限。
#[tauri::command]
pub fn add_reserved_ports(start: i32, count: i32, protocol: Option<String>) -> Result<(), String> {
    run_netsh_range("add", protocol.as_deref(), start, count)
}

/// 删除一段 Windows 保留端口。系统动态保留的段（列表里带 `*`）通常删不掉，
/// 此时 netsh 会报错，错误原样返回给用户。需要管理员权限。
#[tauri::command]
pub fn delete_reserved_ports(start: i32, count: i32, protocol: Option<String>) -> Result<(), String> {
    run_netsh_range("delete", protocol.as_deref(), start, count)
}

/// 执行 `net stop|start <service>`，返回 (是否成功, stdout + stderr)。
fn run_net_service(verb: &str, service: &str) -> (bool, String) {
    let cmdline = format!("chcp 65001 >nul & net {} {}", verb, service);
    match super::hidden_cmd::hidden_cmd("cmd")
        .args(&["/c", &cmdline])
        .output()
    {
        Ok(out) => {
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            (out.status.success(), text.trim().to_string())
        }
        Err(e) => (false, format!("执行 net {} {} 失败: {}", verb, service, e)),
    }
}

/// `net stop winnat` 报这些时说明服务本来就没在跑，可以继续（不算失败）。
fn is_winnat_already_stopped(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    lower.contains("未启动")
        || lower.contains("没有启动")
        || lower.contains("尚未启动")
        || lower.contains("服务名无效")
        || lower.contains("not been started")
        || lower.contains("not started")
        || lower.contains("service name is invalid")
}

/// `net start winnat` 报这些时说明服务已经在跑（错误码 1056），视为已恢复。
fn is_winnat_already_running(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    lower.contains("已经启动")
        || lower.contains("已在运行")
        || lower.contains("already been started")
        || lower.contains("already running")
}

/// WinNAT 服务当前是否处于 RUNNING 状态（`sc query winnat`）。
fn winnat_is_running() -> bool {
    match super::hidden_cmd::hidden_cmd("cmd")
        .args(&["/c", "chcp 65001 >nul & sc query winnat"])
        .output()
    {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout).to_uppercase();
            text.contains("RUNNING")
        }
        Err(_) => false,
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct PortProtectResult {
    pub port: i32,
    pub protocol: String,
    /// 端口此前已经在静态排除段里，本次什么都没做
    pub already_protected: bool,
    /// 开始时 WinNAT 是否在运行（决定是否需要「恢复」）
    pub winnat_was_running: bool,
    /// WinNAT 是否真的被停掉了（原本就没运行时为 false）
    pub winnat_stopped: bool,
    /// WinNAT 是否已恢复运行（原本就没运行时为 false —— 不能谎报「已恢复」）
    pub winnat_started: bool,
    /// 各步骤的原始输出，便于排查
    pub detail: String,
}

/// 禁止某个端口被系统 / WinNAT 预留占用：把端口写进 Windows 排除端口范围
/// （`netsh ... add excludedportrange`），这样动态预留（Hyper-V / WSL / Docker）就再也抢不走它。
///
/// WinNAT 运行时会把已占用的段算进动态排除列表，此时直接 add 会被拒绝，
/// 所以顺序与手工排障一致：`net stop winnat` → `netsh add excludedportrange` → `net start winnat`。
/// 若 WinNAT 原本没在跑则跳过停/启（不去启动用户本来没开的服务），并在 detail 里说明。
/// **需要管理员权限。**
#[tauri::command]
pub fn protect_port_from_reservation(
    port: i32,
    protocol: Option<String>,
) -> Result<PortProtectResult, String> {
    let proto = normalize_protocol(protocol.as_deref())?;
    validate_range(port, 1)?;

    if !super::env::is_admin() {
        return Err(
            "需要管理员权限：请以管理员身份运行 AnyVersion 后再禁止端口被预留".to_string(),
        );
    }

    // 已经有了静态排除段：重复操作只会得到 netsh 的「已存在」报错，直接幂等返回。
    let before = get_excluded_port_ranges();
    if is_port_statically_excluded(port, &before) {
        return Ok(PortProtectResult {
            port,
            protocol: proto.to_string(),
            already_protected: true,
            winnat_was_running: winnat_is_running(),
            winnat_stopped: false,
            winnat_started: false,
            detail: format!("端口 {} 已在管理员排除范围内，无需重复操作", port),
        });
    }
    // 记录当前压着该端口的动态段：add 怎么都成功不了时，这是唯一能解释原因的信息。
    let blocking = find_dynamic_range(port, &before).cloned();

    let mut detail = String::new();
    let was_running = winnat_is_running();
    let mut stopped = false;

    if was_running {
        let (ok, msg) = run_net_service("stop", "winnat");
        detail.push_str(&format!(
            "net stop winnat -> {}\n",
            if msg.is_empty() { "(无输出)" } else { msg.as_str() }
        ));
        if !ok && !is_winnat_already_stopped(&msg) {
            return Err(format!(
                "停止 WinNAT 服务失败，已中止（未修改端口排除范围）：{}",
                msg
            ));
        }
        stopped = ok;
    } else {
        detail.push_str("WinNAT 服务未运行，跳过 net stop winnat\n");
    }

    // WinNAT 已停 → 它的动态预留段已释放，此时 add 才不会被"与现有排除范围重叠"挡住。
    // 但 net stop 返回≠驱动已落定：排除状态释放有秒级延迟，立刻 add 会**偶发**
    // ERROR_SHARING_VIOLATION（「正被另一个进程使用」）——这就是本功能时灵时不灵的根因。
    // 更坑的是 netsh 偶发「报错但实际已写入」，再重试又会报「已存在」，
    // 因此**每一次尝试之后都以 show 的回读结果为准**，而不是只看退出码：
    // 只认退出码会在「端口其实还压在动态段里」时谎报成功（旧的「功能没用」就是这个）。
    let mut add_err: Option<String> = None;
    let mut excluded = false;
    for attempt in 0..5u8 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(700));
        }
        let attempt_res = run_netsh_range("add", Some(proto), port, 1);
        if let Err(e) = &attempt_res {
            detail.push_str(&format!("netsh add excludedportrange(第{}次) -> {}\n", attempt + 1, e));
            add_err = Some(e.clone());
        }
        if is_port_statically_excluded(port, &get_excluded_port_ranges()) {
            excluded = true;
            detail.push_str(&format!(
                "netsh add excludedportrange -> 回读确认已生效（第{}次，退出码{}）\n",
                attempt + 1,
                if attempt_res.is_ok() { "成功" } else { "报错但实际已写入" }
            ));
            break;
        }
    }

    let mut started = false;
    if was_running {
        let (ok, msg) = run_net_service("start", "winnat");
        detail.push_str(&format!(
            "net start winnat -> {}\n",
            if msg.is_empty() { "(无输出)" } else { msg.as_str() }
        ));
        started = ok || is_winnat_already_running(&msg);
    }

    if !excluded {
        let blocked_hint = match &blocking {
            Some(r) => format!(
                "\n端口 {} 正落在系统动态保留段 {}-{} 内（Hyper-V / WSL / Docker 占用），停 WinNAT 后仍未释放。\
                 建议先关闭 WSL / Docker 桌面端与 Hyper-V 虚拟网卡，或重启电脑后立即重试。",
                port, r.start, r.end
            ),
            None => String::new(),
        };
        return Err(format!(
            "{}{}{}",
            add_err.unwrap_or_else(|| "未能把端口加入排除范围".to_string()),
            blocked_hint,
            if was_running && !started {
                "\nWinNAT 未能恢复运行，请手动执行 `net start winnat`"
            } else {
                ""
            }
        ));
    }

    Ok(PortProtectResult {
        port,
        protocol: proto.to_string(),
        already_protected: false,
        winnat_was_running: was_running,
        winnat_stopped: stopped,
        // 原本没在跑时不算「已恢复」——前端据此提示用户，不能谎报。
        winnat_started: started,
        detail: detail.trim_end().to_string(),
    })
}

#[tauri::command]
pub fn check_port_status(port_str: String) -> Result<PortStatus, String> {
    let port = port_str.parse::<i32>().map_err(|_| "端口号无效".to_string())?;
    let mut status = PortStatus {
        port,
        free: true,
        reserved: false,
        protected: false,
        occupied: false,
        owner: None,
    };

    if let Some(owner) = find_port_owner(&port_str) {
        status.occupied = true;
        status.free = false;
        status.owner = Some(owner);
    }

    let ranges = get_excluded_port_ranges();
    // 「系统动态保留」才是真的绑不上；「管理员排除段」只是不会被动态分走，
    // 应用仍可显式 bind。两者必须分开报，否则禁止成功后会显示成「无法使用」。
    if find_dynamic_range(port, &ranges).is_some() {
        status.reserved = true;
        if !status.occupied {
            status.free = false;
        }
    } else if is_port_statically_excluded(port, &ranges) {
        status.protected = true;
    }

    eprintln!(
        "[port] check_port_status({}) => free={} reserved={} protected={} occupied={} owner={:?}",
        port_str, status.free, status.reserved, status.protected, status.occupied, status.owner
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
    // 内置服务（HTTP / 聚合）监听的端口属于本进程，杀它等于自杀 —— 后端兜底拒绝，
    // 前端就算绕过 self_owned 判断也杀不掉自己。
    if owner.self_owned {
        return Err(format!("端口 {} 的占用进程就是 Kira 自己，已拒绝终止", port_str));
    }

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
            "权限不足：无法终止进程 {} (PID: {})。该进程可能需要管理员权限才能终止。\n\n解决方案：\n1. 以管理员身份运行 Kira\n2. 或在任务管理器中手动结束该进程\n\n原始错误：{}",
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

#[cfg(test)]
mod tests {
    use super::{
        find_dynamic_range, is_port_statically_excluded, is_self_pid, normalize_protocol,
        parse_excluded_ranges, validate_range,
    };

    /// 内置服务（HTTP / 聚合）占用端口时 netstat 给出的 PID 就是本进程，
    /// 识别不出来会被「结束进程」按钮杀掉 —— 这是自杀，必须挡住。
    #[test]
    fn self_pid_is_detected_and_others_are_not() {
        assert!(is_self_pid(&std::process::id().to_string()));
        assert!(is_self_pid(&format!("  {} ", std::process::id())));
        assert!(!is_self_pid(""));
        assert!(!is_self_pid("999999"));
        assert!(!is_self_pid("not-a-pid"));
    }

    /// `netsh int ipv4 show excludedportrange` 的旧格式（Win10 早期）：
    /// `---` 分隔线之后才是数据，行首 `*` 表示该段由系统动态保留（Hyper-V / WSL 等）。
    const SAMPLE_LEGACY: &str = "\
协议 tcp 端口排除范围

开始端口    结束端口
----------    --------
      1024        1123
*     5000        5099     *
*     51800       51899     Hyper-V
";

    /// 新格式（Win10 2004+ / Win11）真实输出：行尾独立 `*` 反而是**管理员手工排除**
    /// （脚注 `* - Administered port exclusions.`），没有标记的才是动态保留。
    const SAMPLE_MODERN: &str = "\
Protocol tcp Port Exclusion Ranges

Start Port    End Port
----------    --------
      1455        1455     *
     51800       51899
     50000       50059     *

* - Administered port exclusions.
";

    #[test]
    fn parses_ranges_and_flags_system_managed_ones() {
        let ranges = parse_excluded_ranges(SAMPLE_LEGACY);
        assert_eq!(ranges.len(), 3, "带 * 的动态段不能被丢掉: {:?}", ranges);
        assert_eq!((ranges[0].start, ranges[0].end), (1024, 1123));
        assert!(!ranges[0].managed, "旧格式无标记行 = 手工静态排除");
        assert_eq!(ranges[1].start, 5000);
        assert!(ranges[1].managed, "旧格式行首 * = 系统动态保留");
        assert_eq!(ranges[2].start, 51800);
        assert!(ranges[2].managed);
        assert!(ranges[2].label.contains("Hyper-V"));
    }

    #[test]
    fn modern_format_star_means_administered_not_dynamic() {
        let ranges = parse_excluded_ranges(SAMPLE_MODERN);
        assert_eq!(ranges.len(), 3, "脚注行不能被当成数据行: {:?}", ranges);
        // 结果按起始端口排序：1455 / 50000 / 51800
        assert_eq!(ranges[0].start, 1455);
        assert!(!ranges[0].managed, "新格式行尾 * = 管理员手工排除，可删");
        assert!(ranges[0].label.is_empty(), "行尾 * 不能漏进标签列");
        assert_eq!(ranges[1].start, 50000);
        assert!(!ranges[1].managed);
        assert_eq!(ranges[2].start, 51800);
        assert!(ranges[2].managed, "新格式无标记行 = 系统动态保留");
        // 静态覆盖判定：动态段不算「已禁止被预留」
        assert!(is_port_statically_excluded(1455, &ranges));
        assert!(!is_port_statically_excluded(51850, &ranges));
    }

    /// 旧实现把「没有星」的行一律当静态段，于是**系统动态保留的端口**也被认成
    /// 「已禁止被预留」：保护流程的失败分支一回读就以为生效了，界面报成功，
    /// 端口其实依旧绑不上 —— 用户看到的就是「这个功能没用」。
    /// 这条用例把两种段彻底分开：动态段只挡绑定，静态段才是受保护。
    #[test]
    fn only_dynamic_segments_block_binding() {
        let ranges = parse_excluded_ranges(SAMPLE_MODERN);

        let blocking = find_dynamic_range(51850, &ranges).expect("动态段必须被识别出来");
        assert_eq!((blocking.start, blocking.end), (51800, 51899));
        assert!(
            !is_port_statically_excluded(51850, &ranges),
            "动态段不能算作已禁止被预留（否则会谎报成功）"
        );

        assert!(find_dynamic_range(1455, &ranges).is_none(), "受保护端口不在动态段里");
        assert!(is_port_statically_excluded(1455, &ranges));

        assert!(find_dynamic_range(1234, &ranges).is_none(), "没被任何段覆盖");
        assert!(!is_port_statically_excluded(1234, &ranges));
    }

    #[test]
    fn ignores_everything_before_the_table() {
        assert!(parse_excluded_ranges("").is_empty());
        assert!(parse_excluded_ranges("开始端口    结束端口").is_empty());
    }

    #[test]
    fn protocol_defaults_to_tcp_and_rejects_others() {
        assert_eq!(normalize_protocol(None).unwrap(), "tcp");
        assert_eq!(normalize_protocol(Some("UDP")).unwrap(), "udp");
        assert!(normalize_protocol(Some("sctp")).is_err());
    }

    #[test]
    fn range_validation_rejects_out_of_bounds() {
        assert!(validate_range(0, 10).is_err());
        assert!(validate_range(1, 0).is_err());
        assert!(validate_range(65530, 10).is_err(), "越界到 65535 之外");
        assert!(validate_range(1, 5000).is_err(), "单次数量上限");
        assert!(validate_range(50000, 100).is_ok());
    }
}
