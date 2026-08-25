// mihomo 核心生命周期管理（对齐 clash-party manager.ts）
use crate::commands::mihomo::config::*;
use crate::commands::mihomo::factory::generate_runtime_config;
use crate::commands::mihomo::api::mihomo_api_raw;
use crate::commands::mihomo::emit_state;
use crate::commands::mihomo::MihomoInner;
use crate::commands::hidden_cmd::hidden_cmd;
use std::path::PathBuf;
use std::process::Stdio;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::AppHandle;
use winreg::enums::*;
use winreg::RegKey;

/// 内核变体 → 可执行文件名
pub fn core_file_name(variant: &str) -> String {
    match variant {
        "mihomo-alpha" => "mihomo-alpha.exe".to_string(),
        "mihomo-smart" => "mihomo-smart.exe".to_string(),
        "mihomo-specific" => "mihomo-specific.exe".to_string(),
        _ => "mihomo.exe".to_string(),
    }
}

/// 内核所在目录（bin/mihomo）
pub fn core_dir(app: &AppConfig) -> PathBuf {
    let base = resolve_base_core_path(app);
    base.parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 定位默认 mihomo.exe（不考虑变体）
fn resolve_base_core_path(app: &AppConfig) -> PathBuf {
    if let Some(p) = &app.core_path {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return pb;
        }
    }
    if let Some(p) = crate::commands::utils::bin_tool_path("mihomo") {
        return p;
    }
    // 单一路径策略：不再从程序根目录（CARGO_MANIFEST_DIR/bin）读取，统一 data_dir/bin。
    PathBuf::from("mihomo")
}

pub fn resolve_core_path(app: &AppConfig) -> PathBuf {
    // 0. 内核变体（mihomo-alpha / mihomo-smart / mihomo-specific）
    let variant = app
        .extra
        .get("core")
        .and_then(|v| v.as_str())
        .unwrap_or("mihomo")
        .to_string();
    if variant != "mihomo" {
        let base = resolve_base_core_path(app);
        if let Some(dir) = base.parent() {
            let c = dir.join(core_file_name(&variant));
            if c.exists() {
                return c;
            }
        }
    }
    // 1. 显式配置的 core_path 优先
    if let Some(p) = &app.core_path {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return pb;
        }
    }
    // 2. 复用统一的可执行文件定位逻辑（与 ffmpeg / mediamtx 完全一致）：
    //    bin/<tool>/<tool>.exe，在各候选 bin 目录（resource_dir、exe 向上、cwd、~/.any-version/bin）中查找
    if let Some(p) = crate::commands::utils::bin_tool_path("mihomo") {
        return p;
    }
    // 单一路径策略：不再从程序根目录（CARGO_MANIFEST_DIR/bin）读取，统一 data_dir/bin。
    // 兜底：PATH 上的 mihomo
    PathBuf::from("mihomo")
}

/// 将子进程输出逐行追加到日志文件（带大小上限保护）
/// 内核日志中值得提到前台的关键错误特征
fn extract_warning(line: &str) -> Option<String> {
    let l = line.to_lowercase();
    let hit = (l.contains("tun") || l.contains("tap") || l.contains("wintun"))
        && (l.contains("error")
            || l.contains("failed")
            || l.contains("permission")
            || l.contains("denied")
            || l.contains("administrator"));
    let hit = hit
        || l.contains("access is denied")
        || l.contains("operation not permitted")
        || l.contains("requires elevation");
    if !hit {
        return None;
    }
    let t = line.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.chars().take(300).collect())
    }
}

fn pipe_to_log<R: std::io::Read + Send + 'static>(
    reader: R,
    path: PathBuf,
    inner: Option<Arc<MihomoInner>>,
) {
    use std::io::{BufRead, BufReader, Write};
    let mut br = BufReader::new(reader);
    let mut line = String::new();
    loop {
        line.clear();
        match br.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if let (Some(inner), Some(w)) = (inner.as_ref(), extract_warning(&line)) {
            if let Ok(mut list) = inner.runtime_warnings.lock() {
                if !list.contains(&w) && list.len() < 5 {
                    list.push(w);
                }
            }
        }
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p).ok();
        }
        // 超过 10MB 自动截断，避免无限增长
        if let Ok(meta) = std::fs::metadata(&path) {
            if meta.len() > 10 * 1024 * 1024 {
                std::fs::write(&path, b"").ok();
            }
        }
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = f.write_all(line.as_bytes());
        }
    }
}

/// 设置内核进程 CPU 优先级（对齐 clash-party mihomoCpuPriority）
pub fn set_process_priority(pid: u32, priority: &str) {
    use windows_sys::Win32::System::Threading::{
        OpenProcess, SetPriorityClass, ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS,
        HIGH_PRIORITY_CLASS, IDLE_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS, PROCESS_SET_INFORMATION,
        REALTIME_PRIORITY_CLASS,
    };
    let class = match priority {
        "IDLE_PRIORITY_CLASS" => IDLE_PRIORITY_CLASS,
        "BELOW_NORMAL_PRIORITY_CLASS" => BELOW_NORMAL_PRIORITY_CLASS,
        "ABOVE_NORMAL_PRIORITY_CLASS" => ABOVE_NORMAL_PRIORITY_CLASS,
        "HIGH_PRIORITY_CLASS" => HIGH_PRIORITY_CLASS,
        "REALTIME_PRIORITY_CLASS" => REALTIME_PRIORITY_CLASS,
        _ => NORMAL_PRIORITY_CLASS,
    };
    unsafe {
        let handle = OpenProcess(PROCESS_SET_INFORMATION, 0, pid);
        if !handle.is_null() {
            SetPriorityClass(handle, class);
            windows_sys::Win32::Foundation::CloseHandle(handle);
        }
    }
}

/// 端口是否已被占用（对齐 clash-party checkPortAvailable）
pub fn port_in_use(port: u16) -> bool {
    if port == 0 {
        return false;
    }
    std::net::TcpListener::bind(("127.0.0.1", port)).is_err()
}

/// 判断 mihomo 核心是否处于运行中。
///
/// 判断逻辑必须与 `launch_core` 的幂等检测保持一致：
/// 1. 本应用拉起的子进程句柄有效（`child.id() > 0`）且未标记停止；
/// 2. 或混合端口被监听（核心可能由外部 / 上次会话遗留运行）。
///
/// 仅用 `child` 判断会在「外部/遗留进程占用端口」时误判为未运行，
/// 导致托盘/状态视图显示「启动」而实际核心已在运行（自动启动场景常见）。
/// 因此托盘 `build_mihomo_item` 与状态视图 `build_state_view` 均应复用本函数。
pub fn is_core_running(inner: &crate::commands::mihomo::MihomoInner) -> bool {
    // 锁顺序与 launch_core 保持一致（先 app_config 后 child），避免跨锁顺序反转。
    let mixed_port = inner.app_config.lock().unwrap().mixed_port;
    let has_child = {
        let g = inner.child.lock().unwrap();
        g.as_ref().map(|c| c.id() > 0).unwrap_or(false)
            && !inner.stop_flag.load(std::sync::atomic::Ordering::SeqCst)
    };
    has_child || (mixed_port > 0 && port_in_use(mixed_port))
}

/// 读取内核日志末尾若干行，用于把启动失败原因回传给前端
fn tail_log(path: &std::path::Path, lines: usize) -> String {
    let Ok(content) = std::fs::read_to_string(path) else {
        return String::new();
    };
    let all: Vec<&str> = content.lines().collect();
    let start = all.len().saturating_sub(lines);
    all[start..].join("\n")
}

/// 用 `mihomo -t` 预校验配置（对齐 clash-party testProfileOnStart）
pub fn test_config(core: &std::path::Path, work_dir: &std::path::Path, cfg: &std::path::Path) -> Result<(), String> {
    let mut cmd = hidden_cmd(core);
    if let Some(d) = core.parent() {
        cmd.current_dir(d);
    }
    let out = cmd
        .arg("-t")
        .arg("-d")
        .arg(work_dir)
        .arg("-f")
        .arg(cfg)
        .output()
        .map_err(|e| format!("配置校验执行失败: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let mut msg = String::from_utf8_lossy(&out.stdout).to_string();
    msg.push_str(&String::from_utf8_lossy(&out.stderr));
    let detail = msg
        .lines()
        .filter(|l| l.to_lowercase().contains("error") || l.to_lowercase().contains("fatal"))
        .take(5)
        .collect::<Vec<_>>()
        .join("\n");
    Err(format!(
        "配置校验失败：\n{}",
        if detail.is_empty() { msg.trim().to_string() } else { detail }
    ))
}

pub fn is_admin() -> bool {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags("Software", KEY_READ | KEY_WRITE)
        .is_ok()
}

fn kill_child(inner: &MihomoInner) {
    if let Some(mut c) = inner.child.lock().unwrap_or_else(|e| e.into_inner()).take() {
        let _ = c.kill();
    }
}

pub fn stop_core(inner: &MihomoInner) {
    inner.stop_flag.store(true, Ordering::SeqCst);
    kill_child(inner);
}

/// 退出时兜底：若子进程句柄已丢失（例如被 detach 或看门狗接管），
/// 尝试按核心可执行文件路径 + 混合端口定位残留的 mihomo 进程并结束它，
/// 确保 any-version 退出的同时也关闭自己拉起的核心。
pub fn kill_core_by_port(inner: &MihomoInner) {
    // 仅当本会话由 any-version 拉起过核心时才补杀，避免误杀外部 mihomo
    if !inner.launched_by_us.load(Ordering::SeqCst) {
        return;
    }
    let mixed_port = inner.app_config.lock().unwrap_or_else(|e| e.into_inner()).mixed_port;
    if mixed_port == 0 {
        return;
    }
    // 端口已被释放说明核心已退出，无需处理
    if !port_in_use(mixed_port) {
        return;
    }
    let Some(pid) = crate::commands::node_manager::port_owner_pid(mixed_port) else {
        return;
    };
    // 仅结束我们的核心（避免误杀其它占用同端口的程序）
    if let Some(name) = crate::commands::node_manager::process_name_by_pid(pid) {
        let n = name.to_lowercase();
        if n.contains("mihomo") {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .creation_flags(0x08000000)
                .output();
        }
    }
}

/// 看门狗连续重启计数（超过上限后停止自动重启，避免疯狂拉起崩溃内核）
static RESTART_COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
const MAX_AUTO_RESTART: u32 = 5;

pub fn reset_restart_count() {
    RESTART_COUNT.store(0, Ordering::SeqCst);
}

pub fn start_watchdog(app: AppHandle, inner: Arc<MihomoInner>) {
    if inner.watchdog_running.load(Ordering::SeqCst) {
        return;
    }
    inner.watchdog_running.store(true, Ordering::SeqCst);
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            if inner.stop_flag.load(Ordering::SeqCst) {
                inner.watchdog_running.store(false, Ordering::SeqCst);
                break;
            }
            let alive = {
                let mut g = inner.child.lock().unwrap();
                match g.as_mut() {
                    Some(c) => match c.try_wait() {
                        Ok(Some(_)) => {
                            *g = None;
                            false
                        }
                        Ok(None) => true,
                        Err(_) => {
                            *g = None;
                            false
                        }
                    },
                    None => false,
                }
            };
            if !alive && inner.app_config.lock().unwrap().keep_profile_alive {
                let n = RESTART_COUNT.fetch_add(1, Ordering::SeqCst) + 1;
                if n > MAX_AUTO_RESTART {
                    eprintln!(
                        "[mihomo] 核心连续异常退出 {n} 次，停止自动重启（请检查配置）"
                    );
                    inner.stop_flag.store(true, Ordering::SeqCst);
                    inner.watchdog_running.store(false, Ordering::SeqCst);
                    emit_state(&app, &inner);
                    break;
                }
                // 指数退避：1s/2s/4s/8s/16s
                let backoff = 1u64 << (n - 1).min(4);
                eprintln!("[mihomo] 核心进程异常退出，{backoff}s 后第 {n} 次重启");
                tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                if inner.stop_flag.load(Ordering::SeqCst) {
                    inner.watchdog_running.store(false, Ordering::SeqCst);
                    break;
                }
                if let Err(e) = launch_core(&app, Arc::clone(&inner)).await {
                    eprintln!("[mihomo] 看门狗重启失败: {e}");
                }
            } else if alive {
                RESTART_COUNT.store(0, Ordering::SeqCst);
            }
            emit_state(&app, &inner);
        }
    });
}

pub async fn launch_core(app: &AppHandle, inner: Arc<MihomoInner>) -> Result<(), String> {
    // 自动启动/重复启动时：先判断核心是否已在运行，已在运行则直接跳过，
    // 避免"启动时才报错"。幂等：已在运行就不重复拉起，也不视为错误。
    let app_config = inner.app_config.lock().unwrap().clone();
    // 优先按子进程句柄判定（本应用拉起的核心，最准确）；
    let has_child = {
        let g = inner.child.lock().unwrap();
        g.as_ref().map(|c| c.id() > 0).unwrap_or(false) && !inner.stop_flag.load(Ordering::SeqCst)
    };
    // 兜底：混合端口被监听（核心可能由外部/上次会话遗留运行）
    let port_listening = app_config.mixed_port > 0 && port_in_use(app_config.mixed_port);
    if has_child || port_listening {
        crate::exit_log::exit_log(&format!(
            "[mihomo] launch_core 跳过：核心已在运行 (child={}, port_listening={}, mixed_port={})",
            has_child, port_listening, app_config.mixed_port
        ));
        return Ok(());
    }

    // 启动核心前，确保 geo 数据文件已同步到 data_dir。
    // 核心会读取 data_dir 下的 country.mmdb/geoip.metadb 等，缺失会联网下载
    // MMDB（国内常超时）。这里再保险一次，避免任何时机遗漏导致启动失败。
    crate::commands::utils::sync_mihomo_geo();

    // 按当前内核变体创建/移除 Smart 覆写（对齐 clash-party startCore 内的 manageSmartOverride）
    super::smart::manage_smart_override(&inner);

    let app_config = inner.app_config.lock().unwrap().clone();
    let controled = inner.controled_config.lock().unwrap().clone();
    let overrides = inner.override_config.lock().unwrap().clone();
    let profile = inner.current_profile();

    let cfg = generate_runtime_config(
        &app_config,
        &controled,
        &profile,
        &overrides,
        &inner.data_dir,
    )?;
    std::fs::create_dir_all(&inner.data_dir).map_err(|e| e.to_string())?;
    let work_dir = core_work_dir(&inner.data_dir, app_config.diff_work_dir, &profile.id);
    std::fs::create_dir_all(&work_dir).map_err(|e| e.to_string())?;
    let cfg_path = work_dir.join("config.yaml");
    atomic_write(&cfg_path, &cfg.core).map_err(|e| e.to_string())?;
    *inner.runtime_config_str.lock().unwrap() = cfg.runtime;

    // F8 修复：持锁跨越 kill→spawn（kill_child 内部已持锁，此处不再释放后再 spawn）
    kill_child(&inner);
    // 等待旧进程释放端口（改用 tokio 非阻塞睡眠，避免阻塞 async worker）
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let core = resolve_core_path(&app_config);

    // 端口占用预检（对齐 clash-party checkPortAvailable）
    for (name, port) in [
        ("混合端口", app_config.mixed_port),
        ("外部控制器端口", app_config.controller_port),
    ] {
        if port_in_use(port) {
            return Err(format!(
                "{name} {port} 已被其它程序占用，请更换端口后重试"
            ));
        }
    }

    // 启动前配置校验（可通过 extra.testProfileOnStart=false 关闭）
    let test_on_start = app_config
        .extra
        .get("testProfileOnStart")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    if test_on_start {
        test_config(&core, &work_dir, &cfg_path)?;
    }

    let mut cmd = hidden_cmd(&core);
    // 将工作目录设为核心 exe 所在目录，确保 mihomo 能读到同目录的
    // geoip.dat / geosite.dat / country.mmdb 等内置数据文件
    if let Some(core_dir) = core.parent() {
        cmd.current_dir(core_dir);
    }
    cmd.arg("-d")
        .arg(&work_dir)
        .arg("-f")
        .arg(&cfg_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // 每次启动清空上一轮的运行期告警（TUN 权限类告警由状态视图实时计算）
    inner.runtime_warnings.lock().unwrap().clear();
    if app_config.tun_enabled && !is_admin() {
        eprintln!("[mihomo] TUN 已开启但当前非管理员，可能无法创建虚拟网卡（请以管理员身份运行）");
    }

    let mut child = crate::commands::hidden_cmd::spawn_breakaway_fallback(cmd)
        .map_err(|e| format!("启动核心失败: {e}（路径: {core:?}）"))?;
    let pid = child.id();
    // CPU 优先级（对齐 clash-party mihomoCpuPriority）
    if let Some(priority) = app_config.extra.get("cpuPriority").and_then(|v| v.as_str()) {
        set_process_priority(pid, priority);
    }
    // 采集内核 stdout/stderr 到日志文件（对齐 clash-party 的内核日志落盘）
    {
        let log_path = inner.log_file.clone();
        if let Some(out) = child.stdout.take() {
            let p = log_path.clone();
            let s = Arc::clone(&inner);
            std::thread::spawn(move || pipe_to_log(out, p, Some(s)));
        }
        if let Some(err) = child.stderr.take() {
            let p = log_path.clone();
            let s = Arc::clone(&inner);
            std::thread::spawn(move || pipe_to_log(err, p, Some(s)));
        }
    }
    *inner.child.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
    inner.stop_flag.store(false, Ordering::SeqCst);
    inner.launched_by_us.store(true, Ordering::SeqCst);

    // 启动就绪检测：轮询外部控制器端口，最长 10s；期间若进程退出则回传内核日志
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut ready = false;
    while std::time::Instant::now() < deadline {
        {
            let mut g = inner.child.lock().unwrap();
            if let Some(c) = g.as_mut() {
                if let Ok(Some(status)) = c.try_wait() {
                    *g = None;
                    drop(g);
                    let log = tail_log(&inner.log_file, 20);
                    emit_state(app, &inner);
                    return Err(format!(
                        "核心启动后立即退出（{status}）：\n{}",
                        if log.trim().is_empty() { "无日志输出".into() } else { log }
                    ));
                }
            }
        }
        if std::net::TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], app_config.controller_port)),
            std::time::Duration::from_millis(300),
        )
        .is_ok()
        {
            ready = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    if !ready {
        eprintln!("[mihomo] 10s 内未检测到外部控制器就绪，继续等待核心自行启动");
    } else {
        reset_restart_count();
    }

    emit_state(app, &inner);
    start_watchdog(app.clone(), Arc::clone(&inner));
    Ok(())
}

/// 重新加载配置：用 PUT /configs?force=true 让核心重载整套配置（proxies/rules/providers/dns），
/// 失败则回退重启核心进程
pub async fn reload_config(app: &AppHandle, inner: Arc<MihomoInner>) -> Result<(), String> {
    let app_config = inner.app_config.lock().unwrap().clone();
    let controled = inner.controled_config.lock().unwrap().clone();
    let overrides = inner.override_config.lock().unwrap().clone();
    let profile = inner.current_profile();

    let cfg = generate_runtime_config(
        &app_config,
        &controled,
        &profile,
        &overrides,
        &inner.data_dir,
    )?;
    let work_dir = core_work_dir(&inner.data_dir, app_config.diff_work_dir, &profile.id);
    std::fs::create_dir_all(&work_dir).map_err(|e| e.to_string())?;
    let cfg_path = work_dir.join("config.yaml");
    atomic_write(&cfg_path, &cfg.core).map_err(|e| e.to_string())?;
    *inner.runtime_config_str.lock().unwrap() = cfg.runtime;

    // 真正重载整套配置（proxies / rules / providers / dns）。
    // 注意：mihomo 的 PATCH /configs 只接受 mode/port/allow-lan 等小字段、不认 path，
    // 无法切换配置文件；必须用 PUT /configs?force=true 让核心重新加载整个配置文件，
    // 否则切换订阅后核心仍跑旧配置，代理/规则列表为空。
    let res = mihomo_api_raw(
        &app_config,
        reqwest::Method::PUT,
        "/configs?force=true",
        Some(serde_json::json!({
            "path": cfg_path.to_string_lossy().to_string()
        })),
    )
    .await;
    if res.is_err() {
        eprintln!(
            "[mihomo] PUT 重载配置失败: {:?}，回退重启核心以加载新订阅",
            res.as_ref().err()
        );
        // 兜底：kill 旧进程并以新配置重新启动核心，确保代理/规则生效
        if let Err(e) = launch_core(app, Arc::clone(&inner)).await {
            eprintln!("[mihomo] 重启核心兜底也失败: {e}");
        }
    }
    emit_state(app, &inner);
    Ok(())
}

/// 通知系统代理设置已变更（WinINet），否则部分应用不会立即生效
pub fn refresh_sys_proxy_notify() {
    #[allow(unused_unsafe)]
    unsafe {
        use windows_sys::Win32::Networking::WinInet::{
            InternetSetOptionW, INTERNET_OPTION_REFRESH, INTERNET_OPTION_SETTINGS_CHANGED,
        };
        InternetSetOptionW(
            std::ptr::null_mut(),
            INTERNET_OPTION_SETTINGS_CHANGED,
            std::ptr::null_mut(),
            0,
        );
        InternetSetOptionW(
            std::ptr::null_mut(),
            INTERNET_OPTION_REFRESH,
            std::ptr::null_mut(),
            0,
        );
    }
}

/// 默认绕过列表（对齐 clash-party defaultBypass）
pub fn default_bypass() -> String {
    "localhost;127.*;10.*;172.16.*;172.17.*;172.18.*;172.19.*;172.20.*;172.21.*;172.22.*;172.23.*;172.24.*;172.25.*;172.26.*;172.27.*;172.28.*;172.29.*;172.30.*;172.31.*;192.168.*;<local>".to_string()
}

/// 生成默认 PAC 脚本
pub fn default_pac_script(port: u16) -> String {
    format!(
        "function FindProxyForURL(url, host) {{\n  if (isPlainHostName(host) || shExpMatch(host, \"*.local\") || isInNet(dnsResolve(host), \"127.0.0.0\", \"255.0.0.0\") || isInNet(dnsResolve(host), \"10.0.0.0\", \"255.0.0.0\") || isInNet(dnsResolve(host), \"172.16.0.0\", \"255.240.0.0\") || isInNet(dnsResolve(host), \"192.168.0.0\", \"255.255.0.0\")) {{\n    return \"DIRECT\";\n  }}\n  return \"PROXY 127.0.0.1:{port}; DIRECT\";\n}}\n"
    )
}

/// PAC 文件服务：在本地端口上返回 PAC 脚本
fn start_pac_server(script: String, port: u16) {
    use std::io::{Read, Write};
    std::thread::spawn(move || {
        let listener = match std::net::TcpListener::bind(("127.0.0.1", port)) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[mihomo] PAC 服务启动失败: {e}");
                return;
            }
        };
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut buf = [0u8; 1024];
            let _ = s.read(&mut buf);
            let body = script.clone();
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/x-ns-proxy-autoconfig\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.as_bytes().len(),
                body
            );
            let _ = s.write_all(resp.as_bytes());
        }
    });
}

static PAC_STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_sys_proxy(inner: &MihomoInner, enable: bool) -> Result<(), String> {
    let app = inner.app_config.lock().unwrap();
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu
        .open_subkey_with_flags(
            "Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings",
            KEY_SET_VALUE,
        )
        .map_err(|e| e.to_string())?;
    let pac_mode = app
        .extra
        .get("sysProxyMode")
        .and_then(|v| v.as_str())
        .unwrap_or("manual")
        == "auto";
    if enable && pac_mode {
        let pac_port = app
            .extra
            .get("pacPort")
            .and_then(|v| v.as_u64())
            .unwrap_or(7891) as u16;
        let script = app
            .extra
            .get("pacScript")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| default_pac_script(app.mixed_port));
        if !PAC_STARTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
            start_pac_server(script, pac_port);
        }
        key.set_value("ProxyEnable", &0u32)
            .map_err(|e| e.to_string())?;
        key.set_value(
            "AutoConfigURL",
            &format!("http://127.0.0.1:{pac_port}/pac"),
        )
        .map_err(|e| e.to_string())?;
    } else if enable {
        let server = format!("127.0.0.1:{}", app.mixed_port);
        let mut bypass = default_bypass();
        if !app.sys_proxy_bypass.is_empty() {
            bypass = format!("{};{}", app.sys_proxy_bypass, bypass);
        }
        let _ = key.delete_value("AutoConfigURL");
        key.set_value("ProxyEnable", &1u32)
            .map_err(|e| e.to_string())?;
        key.set_value("ProxyServer", &server)
            .map_err(|e| e.to_string())?;
        key.set_value("ProxyOverride", &bypass)
            .map_err(|e| e.to_string())?;
    } else {
        let _ = key.delete_value("AutoConfigURL");
        key.set_value("ProxyEnable", &0u32)
            .map_err(|e| e.to_string())?;
    }
    refresh_sys_proxy_notify();
    Ok(())
}
