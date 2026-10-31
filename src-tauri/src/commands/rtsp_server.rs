#[path = "rtsp_server_lifecycle.rs"]
mod lifecycle;
use lifecycle::{OwnedChild, OutputReaders};
use parking_lot::Mutex;
use std::process::{Command, Stdio, Child};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::io::{BufRead, BufReader};
use std::net::UdpSocket;
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use tauri::State;
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RtspConfig {
    #[serde(default)]
    pub id: Option<String>,          // 实例唯一 ID
    pub source_type: String,         // "camera" | "file" | "testsrc"
    pub camera_name: Option<String>,
    pub file_path: Option<String>,
    pub port: u16,                   // e.g. 8554
    pub path_name: String,           // e.g. "live"
    pub allow_lan: bool,             // true -> 0.0.0.0, false -> 127.0.0.1
    pub loop_file: bool,
    pub include_audio: bool,
    pub audio_device: Option<String>,
    /// 测试画幅（testsrc）专用的合成音源类型，见 testsrc_audio_source。
    /// 多实例时各选不同类型即可凭听感区分是哪个实例。空/未知值回退 1kHz 纯音。
    #[serde(default)]
    pub test_audio_type: Option<String>,
    pub resolution: Option<String>,   // e.g. "1280x720" or "default"
    pub fps: Option<u32>,             // e.g. 30
    pub bitrate_mbps: Option<f64>,    // e.g. 8.0 (8 Mbps)
    pub gop: Option<u32>,             // e.g. 60 (关键帧间隔，默认 15)
    pub transport: Option<String>,    // "tcp" | "udp"
    pub video_codec: Option<String>,  // "h264" | "h265"
    pub gpu_accel: Option<String>,    // "cpu" | "nvenc" | "qsv" | "amf" | "copy"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraDevices {
    pub video_devices: Vec<String>,
    pub audio_devices: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RtspServerStatus {
    pub id: String,
    pub running: bool,
    pub pid: Option<u32>,
    pub mtx_pid: Option<u32>,
    pub ffmpeg_pid: Option<u32>,
    pub local_url: Option<String>,
    pub lan_url: Option<String>,
    pub config: Option<RtspConfig>,
    pub logs: Vec<String>,
    pub last_error: Option<String>,
    pub uptime_seconds: u64,
}

pub struct InnerRtspServer {
    pub mediamtx_child: Option<OwnedChild>,
    pub ffmpeg_child: OwnedChild,
    pub config: RtspConfig,
    pub start_time: std::time::Instant,
    pub logs: Arc<Mutex<Vec<String>>>,
    pub should_stop: Arc<AtomicBool>,
    pub readers: OutputReaders,
}

#[derive(Default, Clone)]
pub struct RtspServerState {
    pub servers: Arc<Mutex<HashMap<String, InnerRtspServer>>>,
    pub last_logs: Arc<Mutex<HashMap<String, Vec<String>>>>,
    pub last_errors: Arc<Mutex<HashMap<String, Option<String>>>>,
}

/// 项目 bin/ 目录可执行文件定位（新布局：bin/<tool>/<tool>.exe）
fn bin_tool(tool: &str) -> PathBuf {
    crate::commands::utils::bin_tool_path(tool)
        .unwrap_or_else(|| crate::commands::utils::get_bin_dir().join(tool).join(format!("{}.exe", tool)))
}

/// 获取 ffmpeg.exe 路径（bin/ffmpeg/ffmpeg.exe）
fn get_ffmpeg_path() -> PathBuf {
    bin_tool("ffmpeg")
}

/// 获取 mediamtx.exe 路径（bin/mediamtx/mediamtx.exe）
fn get_mediamtx_path() -> PathBuf {
    bin_tool("mediamtx")
}

/// 测试画幅（testsrc）可用的合成音源类型。
///
/// 返回 (lavfi 音频输入参数, 可选音频滤镜)。多实例场景下给各实例选不同类型，
/// 即可凭听感分辨是哪一路流。全部为纯软件合成，不依赖麦克风；参数均已实测可用。
fn testsrc_audio_source(kind: &str) -> (Vec<String>, Option<String>) {
    // 可听音源统一衰减到 50%：满幅正弦/扫频直推非常刺耳。
    let (input, af): (&str, &str) = match kind {
        // 440Hz（标准音 A4）：比 1kHz 低沉，便于与其它实例区分
        "tone440" => ("sine=frequency=440:sample_rate=44100", "volume=0.5"),
        // 左右声道分离：左 1kHz / 右 400Hz，可验证声道是否接反或被混成单声道
        "stereo" => (
            "aevalsrc=sin(1000*2*PI*t)|sin(400*2*PI*t):s=44100:c=stereo",
            "volume=0.5",
        ),
        // 20Hz→20kHz 指数扫频（8 秒一轮）。表达式含逗号，必须用单引号包裹，
        // 否则 filtergraph 会把逗号当作 filter 分隔符，报 "No option name near ..."
        "sweep" => (
            "aevalsrc='sin(2*PI*t*20*pow(1000,mod(t,8)/8))':s=44100:c=mono",
            "volume=0.5",
        ),
        // 粉红噪声：anoisesrc 自带 amplitude 控制音量，不再叠加 volume 滤镜
        "noise" => ("anoisesrc=color=pink:sample_rate=44100:amplitude=0.3", ""),
        // 间断滴声：每秒响 0.15 秒，便于察觉卡顿/丢帧；音量门控并入 volume 表达式
        "beep" => (
            "sine=frequency=1000:sample_rate=44100",
            "volume='0.5*if(lt(mod(t,1),0.15),1,0)':eval=frame",
        ),
        // 默认：1kHz 纯音
        _ => ("sine=frequency=1000:sample_rate=44100", "volume=0.5"),
    };
    let args = vec![
        "-f".to_string(),
        "lavfi".to_string(),
        "-i".to_string(),
        input.to_string(),
    ];
    let af = if af.is_empty() { None } else { Some(af.to_string()) };
    (args, af)
}

/// 等待回环 TCP 端口进入监听状态（最多 timeout）。
///
/// MediaMTX 若因配置/端口错误瞬间退出，FFmpeg 会静默卡在 RTSP 连接上——
/// 既不报错也不退出，界面显示运行中、日志正常，但播放器永远连不上。
fn wait_rtsp_listening(port: u16, timeout: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    while std::time::Instant::now() < deadline {
        if std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(200)).is_ok()
        {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(80));
    }
    false
}

/// Windows 保留（排除）的 TCP 端口范围，由 Hyper-V / WinNAT / Docker / WSL
/// 动态申请。落在这些范围的端口在 netstat 里看不到占用者，但 bind 会返回
/// WSAEACCES(10013)，即“以访问权限不允许的方式做了一个访问套接字的尝试”。
fn excluded_tcp_ranges() -> Vec<(u16, u16)> {
    #[cfg(windows)]
    {
        let mut cmd = crate::commands::hidden_cmd::hidden_cmd("netsh");
        cmd.args(["int", "ipv4", "show", "excludedportrange", "protocol=tcp"]);
        if let Ok(output) =
            crate::commands::hidden_cmd::output_capped(cmd, crate::commands::hidden_cmd::DEFAULT_OUTPUT_CAP)
        {
            let mut ranges = Vec::new();
            for line in output.stdout.lines() {
                let nums: Vec<u16> = line
                    .split_whitespace()
                    .filter_map(|t| t.trim_end_matches('*').parse::<u16>().ok())
                    .collect();
                if nums.len() == 2 && nums[0] <= nums[1] {
                    ranges.push((nums[0], nums[1]));
                }
            }
            return ranges;
        }
    }
    Vec::new()
}

/// 启动前确认端口真的可绑定。系统保留端口不在 netstat 占用列表里，
/// 只靠 LISTENING 检查拦不住，MediaMTX 会 bind 失败并退出。
fn check_port_bindable(port: u16, allow_lan: bool) -> Result<(), String> {
    let host = if allow_lan { "0.0.0.0" } else { "127.0.0.1" };
    if std::net::TcpListener::bind((host, port)).is_ok() {
        return Ok(());
    }

    let reserved = excluded_tcp_ranges()
        .into_iter()
        .find(|(start, end)| (*start..=*end).contains(&port));
    let suggestion = (port.saturating_add(1)..=port.saturating_add(200).min(65535))
        .find(|p| std::net::TcpListener::bind((host, *p)).is_ok());

    let mut err = match reserved {
        Some((start, end)) => format!(
            "端口 {} 落在 Windows 保留的端口范围内（{}-{}，通常由 Hyper-V/WinNAT/Docker/WSL 动态占用），无法绑定",
            port, start, end
        ),
        None => format!("端口 {} 无法绑定（{}）", port, host),
    };
    if let Some(free) = suggestion {
        err.push_str(&format!("；建议改用 {} 端口", free));
    }
    err.push_str("。可运行 `netsh int ipv4 show excludedportrange protocol=tcp` 查看保留范围。");
    Err(err)
}

/// 获取本机的局域网 IP
fn get_local_ip() -> String {
    if let Ok(socket) = UdpSocket::bind("0.0.0.0:0") {
        if socket.connect("8.8.8.8:80").is_ok() {
            if let Ok(addr) = socket.local_addr() {
                return addr.ip().to_string();
            }
        }
    }
    "127.0.0.1".to_string()
}

/// 获取本机所有网卡的 IPv4 地址（含接口名）
#[tauri::command]
pub fn get_all_local_ips() -> Vec<(String, String)> {
    use sysinfo::Networks;
    let mut result = Vec::new();
    let networks = Networks::new_with_refreshed_list();
    for (name, data) in networks.iter() {
        for ip_net in data.ip_networks() {
            if let std::net::IpAddr::V4(ip4) = ip_net.addr {
                if !ip4.is_loopback() {
                    result.push((name.clone(), ip4.to_string()));
                }
            }
        }
    }
    result
}

/// 扫描 DirectShow 音视频设备 (Windows)
#[tauri::command]
pub fn get_rtsp_camera_devices() -> Result<CameraDevices, String> {
    let ffmpeg_path = get_ffmpeg_path();
    
    let mut cmd = Command::new(&ffmpeg_path);
    cmd.args(["-f", "dshow", "-list_devices", "true", "-i", "dummy"]);
    
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let output = cmd.output().map_err(|e| format!("执行 ffmpeg 失败 (路径: {}): {}", ffmpeg_path.display(), e))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{}\n{}", stdout, stderr);

    let mut video_devices = Vec::new();
    let mut audio_devices = Vec::new();

    let mut is_audio_section = false;

    for line in combined.lines() {
        if line.contains("(audio)") {
            is_audio_section = true;
        } else if line.contains("(video)") {
            is_audio_section = false;
        }

        if let Some(start_idx) = line.find("\"") {
            if let Some(end_idx) = line[start_idx + 1..].find("\"") {
                let name = &line[start_idx + 1..start_idx + 1 + end_idx];
                if !name.is_empty() && !name.starts_with('@') {
                    if is_audio_section {
                        if !audio_devices.contains(&name.to_string()) {
                            audio_devices.push(name.to_string());
                        }
                    } else {
                        if !video_devices.contains(&name.to_string()) {
                            video_devices.push(name.to_string());
                        }
                    }
                }
            }
        }
    }

    Ok(CameraDevices {
        video_devices,
        audio_devices,
    })
}

/// 启动指定的 RTSP 服务器实例
#[tauri::command]
pub fn start_rtsp_server(
    app: tauri::AppHandle,
    state: State<'_, RtspServerState>,
    mut config: RtspConfig,
) -> Result<RtspServerStatus, String> {
    let _operation = crate::runtime_tasks::begin_operation("rtspServer")?;
    let instance_id = config.id.clone().unwrap_or_else(|| format!("server_{}", config.port));
    config.id = Some(instance_id.clone());

    // 记住最近一次配置，供托盘「启动上次配置」使用
    if let Ok(v) = serde_json::to_value(&config) {
        crate::commands::config::remember_last_server("rtsp", v);
    }

    let mut guard = state.servers.lock();

    // 如果指定 ID 已存在运行中的进程，先彻底杀死清理
    if let Some(inner) = guard.get_mut(&instance_id) {
        lifecycle::stop_instance(inner, false)?;
    }
    guard.remove(&instance_id);

    state.last_errors.lock().insert(instance_id.clone(), None);
    state.last_logs.lock().insert(instance_id.clone(), Vec::new());

    let logs = Arc::new(Mutex::new(Vec::new()));
    let mut readers = OutputReaders::default();
    let path_clean = config.path_name.trim_start_matches('/');

    // 检查是否存在标准 RTSP 流媒体服务器 MediaMTX
    let mediamtx_path = get_mediamtx_path();
    let has_mediamtx = mediamtx_path.exists();

    let mut mediamtx_child = None;

    // 清理占用目标 RTSP 端口的残留进程（上次会话遗留的 mediamtx/ffmpeg 等），
    // 否则新 mediamtx 绑定失败，potplayer 仍连到旧端口看到旧分辨率流。
    if let Some(pid) = crate::commands::utils::port_owner_pid(config.port) {
        if let Some(name) = crate::commands::utils::process_name_by_pid(pid) {
            let n = name.to_lowercase();
            if n.contains("mediamtx") || n.contains("ffmpeg") {
                let _ = std::process::Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/T", "/F"])
                    .creation_flags(0x08000000)
                    .output();
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
        }
    }

    // 端口仍被其他进程占用（Q-0095：Android 模拟器会抢占 127.0.0.1:8554）时直接失败：
    // 否则 MediaMTX 绑 0.0.0.0 "成功"掩盖冲突，ffmpeg 在 RTSP 握手阶段报出
    // 误导性的 `Invalid data found when processing input`。
    if let Some(owner) = crate::commands::utils::port_conflict_description(config.port) {
        let err = format!(
            "端口 {} 已被 {} 占用，RTSP 服务无法启动；请更换端口或结束占用进程后重试",
            config.port, owner
        );
        logs.lock().push(format!("[SERVER ERROR] {}", err));
        if let Some(l) = state.last_logs.lock().get_mut(&instance_id) {
            l.push(format!("[SERVER ERROR] {}", err));
        }
        state.last_errors.lock().insert(instance_id.clone(), Some(err.clone()));
        return Err(err);
    }

    // 系统保留端口（Hyper-V/WinNAT 等）不会出现在 netstat 里，必须实际试绑一次。
    if let Err(err) = check_port_bindable(config.port, config.allow_lan) {
        logs.lock().push(format!("[SERVER ERROR] {}", err));
        if let Some(l) = state.last_logs.lock().get_mut(&instance_id) {
            l.push(format!("[SERVER ERROR] {}", err));
        }
        state.last_errors.lock().insert(instance_id.clone(), Some(err.clone()));
        return Err(err);
    }

    if has_mediamtx {
        let bind_addr = if config.allow_lan {
            format!("0.0.0.0:{}", config.port)
        } else {
            format!("127.0.0.1:{}", config.port)
        };

        let mtx_log = format!("[SERVER] 启动 MediaMTX 高性能 RTSP 服务器 (实例: {}, 监听: {})...", instance_id, bind_addr);
        logs.lock().push(mtx_log.clone());
        if let Some(l) = state.last_logs.lock().get_mut(&instance_id) {
            l.push(mtx_log);
        }

        let rtp_port = 10000 + (config.port % 1000) * 2;
        let rtcp_port = rtp_port + 1;

        let mut mtx_cmd = Command::new(&mediamtx_path);
        // MediaMTX 只会从「当前工作目录」查找 mediamtx.yml。应用进程的 cwd 不可控
        // （开发时是 src-tauri，安装后是快捷方式的工作目录），一旦恰好存在同版本
        // 配置文件，MTX_PATHS_ALL_OTHERS 会与文件里的 all_others 冲突并以
        // `ERR: all_others, all and '~^.*$' are aliases` 直接退出。
        // 固定 cwd 到 bin 根目录（不含 mediamtx.yml），保证配置只来自下面的环境变量。
        let mtx_cwd = crate::commands::utils::get_bin_dir();
        if mtx_cwd.is_dir() {
            mtx_cmd.current_dir(&mtx_cwd);
        }
        // 空配置下必须显式声明 paths.all_others，否则发布任意路径会返回 400
        // "path 'xxx' is not configured"。
        mtx_cmd.env("MTX_PATHS_ALL_OTHERS", "{}");
        mtx_cmd.env("MTX_RTPADDRESS", format!(":{}", rtp_port));
        mtx_cmd.env("MTX_RTCPADDRESS", format!(":{}", rtcp_port));
        mtx_cmd.env("MTX_RTSPADDRESS", &bind_addr);
        mtx_cmd.env("MTX_RTMP", "no");
        mtx_cmd.env("MTX_HLS", "no");
        mtx_cmd.env("MTX_WEBRTC", "no");
        mtx_cmd.env("MTX_SRT", "no");
        mtx_cmd.env("MTX_MOQ", "no");
        mtx_cmd.stdout(Stdio::piped());
        mtx_cmd.stderr(Stdio::piped());

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB：脱离 AnyVersion 生命周期
            mtx_cmd.creation_flags(0x08000000 | 0x01000000);
        }

        #[cfg(unix)] { use std::os::unix::process::CommandExt; mtx_cmd.process_group(0); }
        match mtx_cmd.spawn() {
            Ok(child) => {
                let mut child = OwnedChild::new(child);
                if let Some(out) = child.stdout.take() {
                    readers.spawn(
                        out,
                        Arc::clone(&logs),
                        Arc::clone(&state.last_logs),
                        instance_id.clone(),
                        "[MEDIAMTX] ",
                    );
                }
                if let Some(err) = child.stderr.take() {
                    readers.spawn(
                        err,
                        Arc::clone(&logs),
                        Arc::clone(&state.last_logs),
                        instance_id.clone(),
                        "[MEDIAMTX] ",
                    );
                }
                mediamtx_child = Some(child);
            }
            Err(e) => {
                let mtx_err = format!("[SERVER WARN] 启动 MediaMTX 失败: {}，回退到 FFmpeg 监听模式", e);
                logs.lock().push(mtx_err.clone());
                if let Some(l) = state.last_logs.lock().get_mut(&instance_id) {
                    l.push(mtx_err);
                }
            }
        }

        // 确认 MediaMTX 真的在监听后再拉 FFmpeg：否则 FFmpeg 会静默挂在连接上，
        // 形成「显示运行中 + 无异常日志 + 播放器连不上」的假运行状态。
        if let Some(ref mut mtx) = mediamtx_child {
            let exited = matches!(mtx.try_wait(), Ok(Some(_)));
            let listening = !exited && wait_rtsp_listening(config.port, std::time::Duration::from_secs(3));
            if exited || !listening {
                // 等日志读取线程把 MediaMTX 的报错搬进缓冲
                std::thread::sleep(std::time::Duration::from_millis(200));
                let captured = logs.lock().join("\n");
                let _ = mtx.kill();
                let err = if exited {
                    format!(
                        "MediaMTX 启动即退出，RTSP 服务未就绪。\n{}\n请检查端口 {} 是否被占用、配置是否与 MediaMTX 版本匹配",
                        captured, config.port
                    )
                } else {
                    format!(
                        "MediaMTX 未能在端口 {} 上监听 RTSP（等待超时）。\n{}",
                        config.port, captured
                    )
                };
                state.last_errors.lock().insert(instance_id.clone(), Some(err.clone()));
                return Err(err);
            }
        }
    } else {
        let warn_msg = format!("[SERVER WARN] 未检测到 MediaMTX 可执行文件，使用 FFmpeg listen 模式");
        logs.lock().push(warn_msg.clone());
        if let Some(l) = state.last_logs.lock().get_mut(&instance_id) {
            l.push(warn_msg);
        }
    }

    let ffmpeg_path = get_ffmpeg_path();
    if !ffmpeg_path.exists() && ffmpeg_path.to_string_lossy() != "ffmpeg" {
        let err = format!("找不到 ffmpeg 可执行文件: {}", ffmpeg_path.display());
        state.last_errors.lock().insert(instance_id.clone(), Some(err.clone()));
        return Err(err);
    }

    let mut args: Vec<String> = Vec::new();
    // 测试画幅的音频滤镜（由所选音源类型决定），供第 4 节音频编码使用
    let mut testsrc_audio_af: Option<String> = None;

    // 1. 输入源定义与选项
    if config.source_type == "testsrc" {
        let res = config.resolution.as_deref().unwrap_or("");
        let size = if !res.is_empty() && res != "default" { res.to_string() } else { "1280x720".to_string() };
        let rate = config.fps.unwrap_or(30).max(1);
        args.extend(vec![
            "-re".to_string(),
            "-f".to_string(), "lavfi".to_string(),
            "-i".to_string(), format!("testsrc=size={}:rate={}", size, rate),
        ]);
        // 测试画幅没有音频输入设备，用 lavfi 合成测试音作为第二个输入，
        // 便于在没有麦克风的情况下验证音频链路（开启「包含音频」时生效）。
        // 音源类型可选，多实例时各选不同类型即可凭听感区分。
        if config.include_audio {
            let kind = config.test_audio_type.as_deref().unwrap_or("tone1000");
            let (audio_input, af) = testsrc_audio_source(kind);
            args.extend(audio_input);
            testsrc_audio_af = af;
        }
    } else if config.source_type == "camera" {
        let cam = config.camera_name.as_deref().unwrap_or("");
        if cam.is_empty() {
            let err = "请选择或输入摄像头设备名称".to_string();
            state.last_errors.lock().insert(instance_id.clone(), Some(err.clone()));
            return Err(err);
        }

        args.extend(vec![
            "-f".to_string(), "dshow".to_string(),
            "-rtbufsize".to_string(), "100M".to_string(),
            "-i".to_string(), format!("video={}", cam),
        ]);
        
        let use_audio = config.include_audio && config.audio_device.as_ref().map_or(false, |a| !a.trim().is_empty());

        if use_audio {
            let aud = config.audio_device.as_ref().unwrap().trim();
            args.extend(vec![
                "-f".to_string(), "dshow".to_string(),
                "-i".to_string(), format!("audio={}", aud),
            ]);
        }
    } else {
        // 文件源
        let file = config.file_path.as_deref().unwrap_or("");
        if file.is_empty() || !Path::new(file).exists() {
            let err = "视频文件不存在，请检查路径".to_string();
            state.last_errors.lock().insert(instance_id.clone(), Some(err.clone()));
            return Err(err);
        }

        args.push("-re".to_string());

        if config.loop_file {
            args.extend(vec!["-stream_loop".to_string(), "-1".to_string()]);
        }

        args.extend(vec!["-i".to_string(), file.to_string()]);
    }

    // 2. 滤镜与帧率输出参数
    // 注意：流复制（gpu=copy）模式下 -vf 与 -c:v copy 冲突（ffmpeg 直接报错
    // "Filtering and streamcopy cannot be used together"），因此 copy 模式跳过缩放，
    // 分辨率由源流本身决定；其余模式（CPU/硬件编码）正常应用 scale。
    let gpu = config.gpu_accel.as_deref().unwrap_or("cpu");
    if gpu != "copy" {
        if let Some(ref res) = config.resolution {
            if !res.is_empty() && res != "default" {
                args.extend(vec!["-vf".to_string(), format!("scale={}", res.replace('x', ":"))]);
            }
        }
    }

    if let Some(fps) = config.fps {
        if fps > 0 {
            args.extend(vec!["-r".to_string(), fps.to_string()]);
        }
    }

    // 3. 视频编码器与显卡/硬件加速
    let codec = config.video_codec.as_deref().unwrap_or("h264");
    let gop = config.gop.unwrap_or(15).max(1).to_string();

    if gpu == "copy" {
        args.extend(vec!["-c:v".to_string(), "copy".to_string()]);
    } else if gpu == "nvenc" {
        let v_encoder = if codec == "h265" { "hevc_nvenc" } else { "h264_nvenc" };
        args.extend(vec![
            "-c:v".to_string(), v_encoder.to_string(),
            "-preset".to_string(), "p1".to_string(),
            "-tune".to_string(), "ll".to_string(),
            "-g".to_string(), gop.clone(),
            "-pix_fmt".to_string(), "yuv420p".to_string(),
        ]);
    } else if gpu == "qsv" {
        let v_encoder = if codec == "h265" { "hevc_qsv" } else { "h264_qsv" };
        args.extend(vec![
            "-c:v".to_string(), v_encoder.to_string(),
            "-g".to_string(), gop.clone(),
            "-pix_fmt".to_string(), "nv12".to_string(),
        ]);
    } else if gpu == "amf" {
        let v_encoder = if codec == "h265" { "hevc_amf" } else { "h264_amf" };
        args.extend(vec![
            "-c:v".to_string(), v_encoder.to_string(),
            "-g".to_string(), gop.clone(),
            "-pix_fmt".to_string(), "yuv420p".to_string(),
        ]);
    } else {
        let v_encoder = if codec == "h265" { "libx265" } else { "libx264" };
        args.extend(vec![
            "-c:v".to_string(), v_encoder.to_string(),
            "-preset".to_string(), "ultrafast".to_string(),
            "-tune".to_string(), "zerolatency".to_string(),
            "-g".to_string(), gop.clone(),
            "-keyint_min".to_string(), gop.clone(),
            "-bf".to_string(), "0".to_string(),
            "-flags".to_string(), "+global_header".to_string(),
            "-pix_fmt".to_string(), "yuv420p".to_string(),
        ]);
    }

    // 码率控制（所有编码器通用，stream copy 除外）
    if gpu != "copy" {
        if let Some(mbps) = config.bitrate_mbps {
            if mbps > 0.0 {
                let kbps = (mbps * 1000.0) as u32;
                args.extend(vec!["-b:v".to_string(), format!("{}k", kbps)]);
            }
        }
    }

    // 4. 音频编码
    if config.include_audio {
        args.extend(vec!["-c:a".to_string(), "aac".to_string(), "-ar".to_string(), "44100".to_string()]);
        // 测试画幅：按所选音源类型应用音量衰减 / 滴声门控滤镜
        if let Some(af) = testsrc_audio_af {
            args.extend(vec!["-af".to_string(), af]);
        }
    } else {
        args.push("-an".to_string());
    }

    // 5. RTSP 目标推流地址
    let transport = config.transport.as_deref().unwrap_or("tcp");

    if mediamtx_child.is_some() {
        let rtsp_target = format!("rtsp://127.0.0.1:{}/{}", config.port, path_clean);
        args.extend(vec![
            "-f".to_string(), "rtsp".to_string(),
            "-rtsp_transport".to_string(), transport.to_string(),
            rtsp_target,
        ]);
    } else {
        let bind_host = if config.allow_lan { "0.0.0.0" } else { "127.0.0.1" };
        let rtsp_target = format!("rtsp://{}:{}/{}", bind_host, config.port, path_clean);
        args.extend(vec![
            "-f".to_string(), "rtsp".to_string(),
            "-rtsp_transport".to_string(), transport.to_string(),
            "-rtsp_flags".to_string(), "listen".to_string(),
            rtsp_target,
        ]);
    }

    let full_cmd_line = format!(
        "\"{}\" {}",
        ffmpeg_path.display(),
        args.iter()
            .map(|a| if a.contains(' ') { format!("\"{}\"", a) } else { a.clone() })
            .collect::<Vec<_>>()
            .join(" ")
    );

    let cmd_log = format!("[EXEC_FFMPEG_CMD] {}", full_cmd_line);
    logs.lock().push(cmd_log.clone());
    if let Some(l) = state.last_logs.lock().get_mut(&instance_id) {
        l.push(cmd_log);
    }

    let mut cmd = Command::new(&ffmpeg_path);
    cmd.args(&args);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB：脱离 AnyVersion 生命周期
        cmd.creation_flags(0x08000000 | 0x01000000);
    }

    #[cfg(unix)] { use std::os::unix::process::CommandExt; cmd.process_group(0); }
    let mut child = match cmd.spawn() {
        Ok(c) => OwnedChild::new(c),
        Err(e) => {
            let err = format!("启动 FFmpeg 推流进程失败: {}\n命令: {}", e, full_cmd_line);
            state.last_errors.lock().insert(instance_id.clone(), Some(err.clone()));
            return Err(err);
        }
    };

    let pid = child.id();
    let mtx_pid = mediamtx_child.as_ref().map(|c| c.id());

    if let Some(stdout) = child.stdout.take() {
        readers.spawn(stdout, Arc::clone(&logs), Arc::clone(&state.last_logs), instance_id.clone(), "");
    }
    // 捕获 stderr 日志
    if let Some(stderr) = child.stderr.take() {
        readers.spawn(
            stderr,
            Arc::clone(&logs),
            Arc::clone(&state.last_logs),
            instance_id.clone(),
            "",
        );
    }

    // 延迟 400ms 检测进程是否启动即崩溃
    std::thread::sleep(std::time::Duration::from_millis(400));
    if let Ok(Some(exit_status)) = child.try_wait() {
        let captured = logs.lock().join("\n");
        let err_msg = if captured.trim().is_empty() {
            format!("FFmpeg 推流进程异常退出 (退出码: {})\n命令: {}", exit_status, full_cmd_line)
        } else {
            format!("FFmpeg 启动失败 (退出码: {}):\n{}\n命令: {}", exit_status, captured, full_cmd_line)
        };

        if let Some(ref mut mtx) = mediamtx_child {
            let _ = mtx.kill();
        }
        state.last_errors.lock().insert(instance_id.clone(), Some(err_msg.clone()));
        return Err(err_msg);
    }

    // MediaMTX 在上面的竞态窗口里退出的话，FFmpeg 会一直卡在推流连接上不报错，
    // 这里再确认一次，避免把「假运行」状态写进实例表。
    if let Some(ref mut mtx) = mediamtx_child {
        if matches!(mtx.try_wait(), Ok(Some(_))) {
            let _ = child.kill();
            std::thread::sleep(std::time::Duration::from_millis(150));
            let captured = logs.lock().join("\n");
            let err_msg = format!("MediaMTX 在 FFmpeg 推流前退出，RTSP 服务不可用。\n{}", captured);
            state.last_errors.lock().insert(instance_id.clone(), Some(err_msg.clone()));
            return Err(err_msg);
        }
    }

    let local_url = format!("rtsp://127.0.0.1:{}/{}", config.port, path_clean);
    let lan_url = if config.allow_lan {
        Some(format!("rtsp://{}:{}/{}", get_local_ip(), config.port, path_clean))
    } else {
        None
    };

    let status = RtspServerStatus {
        id: instance_id.clone(),
        running: true,
        pid: Some(pid),
        mtx_pid,
        ffmpeg_pid: Some(pid),
        local_url: Some(local_url),
        lan_url,
        config: Some(config.clone()),
        logs: logs.lock().clone(),
        last_error: None,
        uptime_seconds: 0,
    };

    guard.insert(
        instance_id,
        InnerRtspServer {
            mediamtx_child,
            ffmpeg_child: child,
            config,
            start_time: std::time::Instant::now(),
            logs,
            should_stop: Arc::new(AtomicBool::new(false)),
            readers,
        },
    );
    drop(guard);
    let _ = crate::tray::rebuild_tray_menu(&app);

    Ok(status)
}

/// 停止指定 ID 的 RTSP 服务器实例
#[tauri::command]
pub fn stop_rtsp_server(
    app: tauri::AppHandle,
    state: State<'_, RtspServerState>,
    id: String,
) -> Result<(), String> {
    let mut guard = state.servers.lock();
    if let Some(inner) = guard.get_mut(&id) {
        lifecycle::stop_instance(inner, true)?;
    }
    guard.remove(&id);
    drop(guard);
    state.last_logs.lock().remove(&id);
    state.last_errors.lock().remove(&id);
    let _ = crate::tray::rebuild_tray_menu(&app);
    Ok(())
}

/// 停止所有 RTSP 服务器实例
#[tauri::command]
pub fn stop_all_rtsp_servers(
    state: State<'_, RtspServerState>,
) -> Result<(), String> {
    lifecycle::stop_all(&state)
}

/// Best-effort synchronous cleanup for application exit; module shutdown reports errors below.
pub fn stop_all_rtsp_servers_inner(state: &RtspServerState) {
    if let Err(error) = lifecycle::stop_all(state) { eprintln!("RTSP cleanup: {error}"); }
}

pub async fn shutdown_module(state: RtspServerState) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || lifecycle::stop_all(&state))
        .await.map_err(|e| e.to_string())?
}

/// 获取指定 ID 的 RTSP 服务器状态与日志
#[tauri::command]
pub fn get_rtsp_server_status(
    state: State<'_, RtspServerState>,
    id: String,
) -> Result<RtspServerStatus, String> {
    let mut guard = state.servers.lock();

    let mut is_running = false;
    let mut pid = None;
    let mut mtx_pid = None;
    let mut ffmpeg_pid = None;
    let mut config = None;
    let mut logs = Vec::new();
    let mut uptime = 0;
    let mut local_url = None;
    let mut lan_url = None;

    if let Some(inner) = guard.get_mut(&id) {
        // MediaMTX 退出后 FFmpeg 会永久卡在推流连接上（不退出、不报错），
        // 只看 FFmpeg 存活会把死掉的实例误报为「运行中」。
        let mtx_exited = match inner.mediamtx_child.as_mut() {
            Some(mtx) => matches!(mtx.try_wait(), Ok(Some(_))),
            None => false,
        };

        match inner.ffmpeg_child.try_wait() {
            Ok(Some(_code)) => {
                is_running = false;
            }
            Ok(None) if !mtx_exited => {
                is_running = true;
                pid = Some(inner.ffmpeg_child.id());
                ffmpeg_pid = Some(inner.ffmpeg_child.id());
                mtx_pid = inner.mediamtx_child.as_ref().map(|c| c.id());
                uptime = inner.start_time.elapsed().as_secs();
                let path_clean = inner.config.path_name.trim_start_matches('/');
                local_url = Some(format!("rtsp://127.0.0.1:{}/{}", inner.config.port, path_clean));
                lan_url = if inner.config.allow_lan {
                    Some(format!("rtsp://{}:{}/{}", get_local_ip(), inner.config.port, path_clean))
                } else {
                    None
                };
            }
            Ok(None) => {
                is_running = false;
            }
            Err(_) => {
                is_running = false;
            }
        }

        config = Some(inner.config.clone());
        logs = inner.logs.lock().clone();

        if !is_running && mtx_exited {
            let captured = logs.join("\n");
            let err = format!(
                "MediaMTX 已退出，RTSP 服务不可用（推流进程会卡在连接上），请重新启动该实例。\n{}",
                captured
            );
            state.last_errors.lock().insert(id.clone(), Some(err));
        }
    }

    if !is_running {
        if let Some(inner) = guard.get_mut(&id) {
            lifecycle::stop_instance(inner, false)?;
        }
        guard.remove(&id);
        if let Some(last_l) = state.last_logs.lock().get(&id) {
            logs = last_l.clone();
        }
    }

    let last_error = state.last_errors.lock().get(&id).cloned().flatten();

    Ok(RtspServerStatus {
        id,
        running: is_running,
        pid,
        mtx_pid,
        ffmpeg_pid,
        local_url,
        lan_url,
        config,
        logs,
        last_error,
        uptime_seconds: uptime,
    })
}

/// 获取所有在运行中的 RTSP 服务器状态列表
#[tauri::command]
pub fn get_all_rtsp_server_statuses(
    state: State<'_, RtspServerState>,
) -> Result<Vec<RtspServerStatus>, String> {
    let mut active_ids: Vec<String> = Vec::new();
    {
        let guard = state.servers.lock();
        for id in guard.keys() {
            active_ids.push(id.clone());
        }
    }

    let mut list = Vec::new();
    for id in active_ids {
        if let Ok(st) = get_rtsp_server_status(state.clone(), id) {
            list.push(st);
        }
    }
    Ok(list)
}
