use super::*;

// ─── 命令执行辅助 ───

pub(super) fn hidden_cmd(program: &str) -> std::process::Command {
    let mut c = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    c
}

/// 执行命令并捕获 stdout/stderr，返回 (stdout, stderr, 是否成功)。
pub(super) fn run_capture(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
) -> (String, String, bool) {
    let mut cmd = hidden_cmd(program);
    cmd.args(args);
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    match cmd.output() {
        Ok(out) => (
            String::from_utf8_lossy(&out.stdout).trim().to_string(),
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
            out.status.success(),
        ),
        Err(e) => (String::new(), format!("无法执行 {}: {}", program, e), false),
    }
}

/// 读取某命令的版本号（如 `node --version` / `git --version`）。
pub(super) fn command_version(exe: &str, args: &[&str]) -> Option<String> {
    let (stdout, _, ok) = run_capture(exe, args, None);
    if !ok {
        return None;
    }
    let text = stdout.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

/// 检测单个依赖（exe_name 可为 path 或命令名）。
pub(super) fn check_dep(exe_name: &str, version_args: &[&str]) -> DepCheck {
    let path = find_in_path(exe_name);
    let exists = path.is_some();
    let version = if exists {
        command_version(&path.as_ref().unwrap().to_string_lossy(), version_args)
    } else {
        None
    };
    DepCheck {
        name: exe_name.to_string(),
        exists,
        path: path.map(|p| p.to_string_lossy().to_string()),
        version,
        satisfies: exists, // git/包管理器只要存在即满足；node 的约束由调用方补充
        requirement: None,
    }
}

// ─── 进度事件 ───

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeProgress {
    pub project_id: String,
    pub phase: String,
    #[serde(default)]
    pub detail: String,
}

pub(super) fn emit_progress(app: &tauri::AppHandle, project_id: &str, phase: &str, detail: &str) {
    let _ = app.emit(
        "npm-progress",
        NodeProgress {
            project_id: project_id.to_string(),
            phase: phase.to_string(),
            detail: detail.to_string(),
        },
    );
}

/// 实时日志事件（git pull / install / build / start 的输出逐行回传前端）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeLog {
    pub project_id: String,
    pub phase: String,
    pub line: String,
}

pub(super) fn emit_log(app: &tauri::AppHandle, project_id: &str, phase: &str, line: &str) {
    let line = line.trim_end_matches(['\r', '\n']);
    if line.is_empty() {
        return;
    }
    let _ = app.emit(
        "npm-log",
        NodeLog {
            project_id: project_id.to_string(),
            phase: phase.to_string(),
            line: line.to_string(),
        },
    );
}

// ─── 控制台 URL 捕获（带凭据主页地址）───

/// 控制台 URL 事件：从启动输出中捕获到带凭据地址时通知前端。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeConsoleUrl {
    pub project_id: String,
    pub url: String,
}

pub(super) fn emit_console_url(app: &tauri::AppHandle, project_id: &str, url: &str) {
    let _ = app.emit(
        "npm-console-url",
        NodeConsoleUrl {
            project_id: project_id.to_string(),
            url: url.to_string(),
        },
    );
}

/// 各项目最近一次捕获的控制台 URL（进程级状态；服务重启后由新输出覆盖）。
pub(super) static CONSOLE_URLS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

pub(super) fn console_urls() -> std::sync::MutexGuard<'static, Option<HashMap<String, String>>> {
    CONSOLE_URLS.lock().unwrap()
}

/// 单锁内完成「与已捕获值比较 + 覆盖记录」，返回是否发生变化。
/// stdout/stderr 两个读取线程可能并发命中同一 URL，原子比较避免重复自动打开浏览器。
pub(super) fn record_console_url_if_changed(project_id: &str, url: &str) -> bool {
    let mut g = console_urls();
    let m = g.get_or_insert_with(HashMap::new);
    let changed = m.get(project_id).map(String::as_str) != Some(url);
    m.insert(project_id.to_string(), url.to_string());
    changed
}

/// 控制台 URL 的持久化文件（项目托管目录内）：应用重启后仍能取回最近捕获的地址，
/// 避免「服务在跑、主页按钮却提示未捕获/回落到无 token 地址」。
pub(super) fn console_url_file_path(project_id: &str) -> Option<PathBuf> {
    find_project(project_id).map(|d| d.managed_dir().join(".console-url.json"))
}

/// 持久化最近捕获的控制台 URL。
pub(super) fn persist_console_url(project_id: &str, url: &str) {
    if let Some(path) = console_url_file_path(project_id) {
        let data = serde_json::json!({ "url": url }).to_string();
        let _ = fs::write(&path, data);
    }
}

/// 读取某项目最近捕获的控制台 URL：内存优先，回落到持久化文件。
pub(super) fn console_url_of(project_id: &str) -> Option<String> {
    if let Some(u) = console_urls()
        .as_ref()
        .and_then(|m| m.get(project_id).cloned())
    {
        return Some(u);
    }
    let path = console_url_file_path(project_id)?;
    let raw = fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    value
        .get("url")
        .and_then(|v| v.as_str())
        .map(String::from)
        .filter(|s| s.starts_with("http://") || s.starts_with("https://"))
}

/// 清除某项目捕获的控制台 URL（停止/卸载时调用；内存 + 持久化文件一并清除）。
pub(super) fn clear_console_url(project_id: &str) {
    if let Some(m) = console_urls().as_mut() {
        m.remove(project_id);
    }
    if let Some(path) = console_url_file_path(project_id) {
        let _ = fs::remove_file(path);
    }
}

/// 逐行读取子进程输出流（**按字节、lossy 解码**）。
///
/// 禁止用 `BufRead::lines()`：它按 UTF-8 解码，Windows 上子进程往管道写
/// locale 编码（GBK）时，中文/emoji 是非法 UTF-8 字节 → `lines()` 返回 Err →
/// reader 线程静默退出并关闭管道读端 → 子进程写 stderr 变 broken pipe →
/// 退出时 flush 失败（**exit code 120**）且尾部日志全部丢失。
/// 字节读取 + `from_utf8_lossy` 永不因编码断流。
pub(super) fn read_child_stream<R: std::io::Read>(
    stream: &mut R,
    app: &tauri::AppHandle,
    project_id: &str,
    level: &str,
    pattern: &str,
    auto_open: bool,
) {
    use std::io::{BufRead, BufReader};
    let mut reader = BufReader::new(stream);
    let mut buf: Vec<u8> = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let line = String::from_utf8_lossy(&buf);
                let line = line.trim_end_matches(['\r', '\n']);
                maybe_capture_console_url(app, project_id, pattern, auto_open, line);
                emit_log(app, project_id, level, line);
            }
        }
    }
}

/// Python 启动参数统一加 `-X utf8`：让子进程的 stdio 强制 UTF-8，
/// 否则 Windows 管道默认 locale 编码（GBK），日志在 Rust 侧显示为乱码。
/// （嵌入式 Python 的 ._pth 隔离模式忽略 PYTHONUTF8 等环境变量，只能用 -X。）
pub(crate) fn with_utf8_flag(mut args: Vec<String>) -> Vec<String> {
    let mut out = vec!["-X".to_string(), "utf8".to_string()];
    out.append(&mut args);
    out
}

/// 在控制台输出行中提取捕获组 URL；无 pattern 或不匹配返回 None。
pub(super) fn extract_console_url(pattern: &str, line: &str) -> Option<String> {
    let re = regex::Regex::new(pattern).ok()?;
    let m = re.captures(line)?;
    let url = m.get(1)?.as_str().trim().trim_end_matches([')', ';', ',']);
    if url.starts_with("http://") || url.starts_with("https://") {
        Some(url.to_string())
    } else {
        None
    }
}

/// 尝试从一行控制台输出中捕获带凭据主页地址。
/// pattern 为空时直接返回；捕获到则覆盖记录、通知前端，并按配置自动打开。
/// 同一行最多命中一次；同一次输出的重复行以后出现的为准（服务重启后 token 会更新）。
pub(super) fn maybe_capture_console_url(
    app: &tauri::AppHandle,
    project_id: &str,
    pattern: &str,
    auto_open: bool,
    line: &str,
) {
    if pattern.trim().is_empty() {
        return;
    }
    let Some(url) = extract_console_url(pattern, line) else {
        return;
    };
    // 原子「比较+记录」：仅当地址变化（服务重启后 token 更新）时自动打开，
    // stdout/stderr 并发命中同一行不会重复打开浏览器。
    let changed = record_console_url_if_changed(project_id, &url);
    persist_console_url(project_id, &url);
    emit_console_url(app, project_id, &url);
    if changed && auto_open {
        open_url_in_browser(&url);
    }
}

/// 用系统默认浏览器打开 URL。
///
/// Windows 上**不要**用 `explorer.exe <url>`：explorer 对带 query（如 `?token=...`）
/// 的 URL 处理不可靠，常退化成打开「此电脑」/资源管理器。首选 `rundll32 url.dll,
/// FileProtocolHandler`（无窗口、URL 语义交给默认浏览器），回退 `cmd /c start`。
pub(super) fn open_url_in_browser(url: &str) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        if let Some(rundll) = find_in_path("rundll32") {
            let _ = std::process::Command::new(rundll)
                .args(["url.dll,FileProtocolHandler", url])
                .creation_flags(0x08000000)
                .spawn();
            return;
        }
        let mut c = std::process::Command::new("cmd");
        c.creation_flags(0x08000000);
        let _ = c.args(&["/c", "start", "", url]).spawn();
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
}

pub(super) fn format_command(program: &str, args: &[&str]) -> String {
    let mut command = program.to_string();
    for arg in args {
        if arg.is_empty() || arg.chars().any(|c| c.is_whitespace() || c == '"') {
            command.push(' ');
            command.push('"');
            command.push_str(&arg.replace('"', "\\\""));
            command.push('"');
        } else {
            command.push(' ');
            command.push_str(arg);
        }
    }
    command
}

/// 按换行或回车分隔实时读取命令输出。npm 的进度渲染通常只使用 `\\r`，
/// `BufRead::lines()` 会一直等到 `\\n`，导致前端看起来像卡住；这里两种分隔符都立即转发。
/// 返回 (最后一行, 完整输出)。完整输出用于判断命令是否只是「警告性」失败
/// （如 pnpm 跳过构建脚本），仅看最后一行会漏掉真正的错误码。
pub(super) fn emit_reader_live<R: std::io::Read>(
    reader: R,
    app: &tauri::AppHandle,
    project_id: &str,
    phase: &str,
) -> (String, String) {
    use std::io::Read;

    let mut reader = std::io::BufReader::new(reader);
    let mut bytes = [0u8; 4096];
    let mut pending = Vec::new();
    let mut last = String::new();
    let mut full = String::new();
    let mut previous_was_cr = false;

    loop {
        let count = match reader.read(&mut bytes) {
            Ok(0) => break,
            Ok(count) => count,
            Err(_) => break,
        };
        for byte in &bytes[..count] {
            if *byte == b'\r' || *byte == b'\n' {
                // CRLF 只作为一条记录；单独的 CR（npm 进度条）也要立即发送。
                if !pending.is_empty() {
                    let line = String::from_utf8_lossy(&pending).to_string();
                    if !line.trim().is_empty() {
                        last = line.clone();
                        full.push_str(&line);
                        full.push('\n');
                        emit_log(app, project_id, phase, &line);
                    }
                    pending.clear();
                }
                previous_was_cr = *byte == b'\r';
            } else {
                // 某些程序会在 CR 后紧接新内容；previous_was_cr 仅用于说明分隔已处理。
                previous_was_cr = false;
                pending.push(*byte);
            }
        }
    }
    if !pending.is_empty() {
        let line = String::from_utf8_lossy(&pending).to_string();
        if !line.trim().is_empty() {
            last = line.clone();
            full.push_str(&line);
            full.push('\n');
            emit_log(app, project_id, phase, &line);
        }
    }
    let _ = previous_was_cr;
    (last, full)
}

/// 杀掉子进程及其整棵进程树（npm/npx 经 `cmd /c` 启动，真正的 node 进程是孙进程，
/// 仅 `child.kill()` 会留下孤儿的 npm/node 进程继续占用网络与文件）。
pub(super) fn kill_process_tree(child: &mut std::process::Child) {
    let pid = child.id().to_string();
    #[cfg(windows)]
    {
        let _ = run_capture("taskkill", &["/f", "/t", "/pid", &pid], None);
    }
    #[cfg(not(windows))]
    {
        let _ = child.kill();
    }
}

/// 执行命令并把 stdout/stderr 实时 emit（支持 newline/CR 分隔）。
/// 返回 (是否成功, 末尾错误行, stdout+stderr 完整输出)。
/// `timeout`：总超时；超时后杀掉整棵进程树并返回失败（npm 网络请求挂死时避免永久转圈）。
pub(super) fn run_capture_live(
    app: &tauri::AppHandle,
    project_id: &str,
    phase: &str,
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
    extra_env: &[(&str, &str)],
    timeout: Option<Duration>,
) -> (bool, String, String) {
    let command = format_command(program, args);
    emit_log(app, project_id, phase, &format!("$ {}", command));
    if let Some(dir) = cwd {
        emit_log(
            app,
            project_id,
            phase,
            &format!("工作目录: {}", dir.display()),
        );
    }

    let mut cmd = hidden_cmd(program);
    cmd.args(args);
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    run_cmd_live(app, project_id, phase, &command, cmd, timeout)
}

/// 执行调用方预构建的 `Command`（npm_exec 等需要 raw_arg / 自定义 env 的场景）。
/// 与 run_capture_live 相同：实时转发 stdout/stderr、心跳、超时杀进程树。
pub(super) fn run_cmd_live(
    app: &tauri::AppHandle,
    project_id: &str,
    phase: &str,
    display_command: &str,
    mut cmd: std::process::Command,
    timeout: Option<Duration>,
) -> (bool, String, String) {
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            emit_log(app, project_id, phase, &format!("无法执行命令: {}", e));
            let msg = format!("无法执行命令: {}", e);
            return (false, msg.clone(), msg);
        }
    };

    let stdout_thread = child.stdout.take().map(|so| {
        let app_out = app.clone();
        let pid = project_id.to_string();
        let ph = phase.to_string();
        std::thread::spawn(move || emit_reader_live(so, &app_out, &pid, &ph))
    });
    let stderr_thread = child.stderr.take().map(|se| {
        let app_err = app.clone();
        let pid = project_id.to_string();
        let ph = phase.to_string();
        std::thread::spawn(move || emit_reader_live(se, &app_err, &pid, &ph))
    });

    // npm 在解析依赖、下载大文件时可能暂时没有任何输出；定期发送心跳，
    // 让服务面板明确知道进程仍在运行，而不是看起来像卡死。
    let started_at = Instant::now();
    let mut last_report = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {
                if let Some(t) = timeout {
                    if started_at.elapsed() >= t {
                        emit_log(
                            app,
                            project_id,
                            phase,
                            &format!("命令超时（{} 秒），终止进程树…", t.as_secs()),
                        );
                        kill_process_tree(&mut child);
                        timed_out = true;
                        break child.wait();
                    }
                }
                if last_report.elapsed() >= Duration::from_secs(5) {
                    emit_progress(
                        app,
                        project_id,
                        phase,
                        &format!(
                            "命令仍在运行…已用时 {} 秒：{}",
                            started_at.elapsed().as_secs(),
                            display_command
                        ),
                    );
                    last_report = Instant::now();
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(error) => break Err(error),
        }
    };
    let stdout_all = stdout_thread
        .and_then(|thread| thread.join().ok())
        .unwrap_or((String::new(), String::new()));
    let stderr_all = stderr_thread
        .and_then(|thread| thread.join().ok())
        .unwrap_or((String::new(), String::new()));
    let mut full = stdout_all.1;
    full.push_str(&stderr_all.1);
    let last_err = if stderr_all.0.trim().is_empty() {
        stdout_all.0
    } else {
        stderr_all.0
    };
    if timed_out {
        let secs = timeout.map(|t| t.as_secs()).unwrap_or(0);
        let msg = format!("命令超时（{} 秒）已终止，请检查网络后重试", secs);
        return (false, msg.clone(), full);
    }
    let ok = matches!(status, Ok(s) if s.success());
    (ok, last_err, full)
}
