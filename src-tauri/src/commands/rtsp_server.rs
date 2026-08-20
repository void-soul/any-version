use parking_lot::Mutex;
use std::process::{Command, Stdio, Child};
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
    pub resolution: Option<String>,   // e.g. "1280x720" or "default"
    pub fps: Option<u32>,             // e.g. 30
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
    pub mediamtx_child: Option<Child>,
    pub ffmpeg_child: Child,
    pub config: RtspConfig,
    pub start_time: std::time::Instant,
    pub logs: Arc<Mutex<Vec<String>>>,
    pub should_stop: Arc<AtomicBool>,
}

#[derive(Default)]
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
    let instance_id = config.id.clone().unwrap_or_else(|| format!("server_{}", config.port));
    config.id = Some(instance_id.clone());

    // 记住最近一次配置，供托盘「启动上次配置」使用
    if let Ok(v) = serde_json::to_value(&config) {
        crate::commands::config::remember_last_server("rtsp", v);
    }

    let mut guard = state.servers.lock();

    // 如果指定 ID 已存在运行中的进程，先彻底杀死清理
    if let Some(mut inner) = guard.remove(&instance_id) {
        inner.should_stop.store(true, Ordering::SeqCst);
        let _ = inner.ffmpeg_child.kill();
        if let Some(ref mut mtx) = inner.mediamtx_child {
            let _ = mtx.kill();
        }
    }

    state.last_errors.lock().insert(instance_id.clone(), None);
    state.last_logs.lock().insert(instance_id.clone(), Vec::new());

    let logs = Arc::new(Mutex::new(Vec::new()));
    let path_clean = config.path_name.trim_start_matches('/');

    // 检查是否存在标准 RTSP 流媒体服务器 MediaMTX
    let mediamtx_path = get_mediamtx_path();
    let has_mediamtx = mediamtx_path.exists();

    let mut mediamtx_child = None;

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

        match mtx_cmd.spawn() {
            Ok(child) => {
                mediamtx_child = Some(child);
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
            Err(e) => {
                let mtx_err = format!("[SERVER WARN] 启动 MediaMTX 失败: {}，回退到 FFmpeg 监听模式", e);
                logs.lock().push(mtx_err.clone());
                if let Some(l) = state.last_logs.lock().get_mut(&instance_id) {
                    l.push(mtx_err);
                }
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

    // 1. 输入源定义与选项
    if config.source_type == "testsrc" {
        args.extend(vec![
            "-re".to_string(),
            "-f".to_string(), "lavfi".to_string(),
            "-i".to_string(), "testsrc=size=1280x720:rate=30".to_string(),
        ]);
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
    if let Some(ref res) = config.resolution {
        if !res.is_empty() && res != "default" {
            args.extend(vec!["-vf".to_string(), format!("scale={}", res.replace('x', ":"))]);
        }
    }

    if let Some(fps) = config.fps {
        if fps > 0 {
            args.extend(vec!["-r".to_string(), fps.to_string()]);
        }
    }

    // 3. 视频编码器与显卡/硬件加速
    let codec = config.video_codec.as_deref().unwrap_or("h264");
    let gpu = config.gpu_accel.as_deref().unwrap_or("cpu");

    if gpu == "copy" {
        args.extend(vec!["-c:v".to_string(), "copy".to_string()]);
    } else if gpu == "nvenc" {
        let v_encoder = if codec == "h265" { "hevc_nvenc" } else { "h264_nvenc" };
        args.extend(vec![
            "-c:v".to_string(), v_encoder.to_string(),
            "-preset".to_string(), "p1".to_string(),
            "-tune".to_string(), "ll".to_string(),
            "-g".to_string(), "15".to_string(),
            "-pix_fmt".to_string(), "yuv420p".to_string(),
        ]);
    } else if gpu == "qsv" {
        let v_encoder = if codec == "h265" { "hevc_qsv" } else { "h264_qsv" };
        args.extend(vec![
            "-c:v".to_string(), v_encoder.to_string(),
            "-g".to_string(), "15".to_string(),
            "-pix_fmt".to_string(), "nv12".to_string(),
        ]);
    } else if gpu == "amf" {
        let v_encoder = if codec == "h265" { "hevc_amf" } else { "h264_amf" };
        args.extend(vec![
            "-c:v".to_string(), v_encoder.to_string(),
            "-g".to_string(), "15".to_string(),
            "-pix_fmt".to_string(), "yuv420p".to_string(),
        ]);
    } else {
        let v_encoder = if codec == "h265" { "libx265" } else { "libx264" };
        args.extend(vec![
            "-c:v".to_string(), v_encoder.to_string(),
            "-preset".to_string(), "ultrafast".to_string(),
            "-tune".to_string(), "zerolatency".to_string(),
            "-g".to_string(), "15".to_string(),
            "-keyint_min".to_string(), "15".to_string(),
            "-bf".to_string(), "0".to_string(),
            "-flags".to_string(), "+global_header".to_string(),
            "-pix_fmt".to_string(), "yuv420p".to_string(),
        ]);
    }

    // 4. 音频编码
    if config.include_audio {
        args.extend(vec!["-c:a".to_string(), "aac".to_string(), "-ar".to_string(), "44100".to_string()]);
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

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let err = format!("启动 FFmpeg 推流进程失败: {}\n命令: {}", e, full_cmd_line);
            state.last_errors.lock().insert(instance_id.clone(), Some(err.clone()));
            return Err(err);
        }
    };

    let pid = child.id();
    let mtx_pid = mediamtx_child.as_ref().map(|c| c.id());

    // 捕获 stderr 日志
    if let Some(stderr) = child.stderr.take() {
        let logs_clone = Arc::clone(&logs);
        let last_logs_arc = Arc::clone(&state.last_logs);
        let inst_id = instance_id.clone();
        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().flatten() {
                {
                    let mut l = logs_clone.lock();
                    if l.len() > 300 {
                        l.remove(0);
                    }
                    l.push(line.clone());
                }
                {
                    let mut last_map = last_logs_arc.lock();
                    let ll = last_map.entry(inst_id.clone()).or_insert_with(Vec::new);
                    if ll.len() > 300 {
                        ll.remove(0);
                    }
                    ll.push(line);
                }
            }
        });
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

        state.last_errors.lock().insert(instance_id.clone(), Some(err_msg.clone()));
        return Err(err_msg);
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
    if let Some(mut inner) = guard.remove(&id) {
        inner.should_stop.store(true, Ordering::SeqCst);
        let _ = inner.ffmpeg_child.kill();
        if let Some(ref mut mtx) = inner.mediamtx_child {
            let _ = mtx.kill();
        }
    }
    drop(guard);
    state.last_errors.lock().insert(id, None);
    let _ = crate::tray::rebuild_tray_menu(&app);
    Ok(())
}

/// 停止所有 RTSP 服务器实例
#[tauri::command]
pub fn stop_all_rtsp_servers(
    state: State<'_, RtspServerState>,
) -> Result<(), String> {
    stop_all_rtsp_servers_inner(&state);
    Ok(())
}

/// 停止所有 RTSP 服务器实例（核心逻辑，供命令与退出清理线程复用）
pub fn stop_all_rtsp_servers_inner(state: &RtspServerState) {
    let mut guard = state.servers.lock();
    for (id, mut inner) in guard.drain() {
        inner.should_stop.store(true, Ordering::SeqCst);
        let _ = inner.ffmpeg_child.kill();
        if let Some(ref mut mtx) = inner.mediamtx_child {
            let _ = mtx.kill();
        }
        state.last_errors.lock().insert(id, None);
    }
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
        match inner.ffmpeg_child.try_wait() {
            Ok(Some(_code)) => {
                is_running = false;
            }
            Ok(None) => {
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
            Err(_) => {
                is_running = false;
            }
        }

        config = Some(inner.config.clone());
        logs = inner.logs.lock().clone();
    }

    if !is_running {
        if let Some(mut inner) = guard.remove(&id) {
            if let Some(ref mut mtx) = inner.mediamtx_child {
                let _ = mtx.kill();
            }
        }
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
