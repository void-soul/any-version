//! SSP (Simple Stream Protocol) 模拟服务 — 模拟 Z CAM 相机的视频流输出。
//! TCP 协议，默认端口 9999，握手后持续推送 H.264/H.265 视频帧与元数据。
//!
//! 协议格式（按 libssp client API 反推）：
//!   握手: client → [4 bytes: u32le protocol_version]
//!         server → [4 bytes: i32le response_code]  (≥0=ok, <0=error)
//!   元数据包: 4B type=1 + 4B len + 56B SspVideoMeta + 28B SspAudioMeta + 10B SspMeta
//!   视频包:   4B type=2 + 4B len + 4B stream + 8B pts + 8B ntp + 4B frm_no + 4B type + 4B data_len + data
//!   音频包:   4B type=3 + 4B len + 4B stream + 8B pts + 8B ntp + 4B data_len + data
//!   心跳:     4B type=4 + 4B len=8 (每 5s 一发)

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

// ─── 数据结构 ───

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SspConfig {
    pub port: u16,
    pub video_codec: String,           // "h264" | "h265"
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub gop: u32,
    pub bitrate_kbps: u32,             // 0 = ffmpeg 自动
    pub source_type: String,           // "testsrc" | "file"
    pub file_path: Option<String>,
    pub enable_audio: bool,
    pub audio_sample_rate: u32,
    pub timecode: u32,                 // 初始时码 (HHMMSSFF 格式)
}

impl Default for SspConfig {
    fn default() -> Self {
        Self {
            port: 9999,
            video_codec: "h264".into(),
            width: 1920,
            height: 1080,
            fps: 30,
            gop: 15,
            bitrate_kbps: 8000,
            source_type: "testsrc".into(),
            file_path: None,
            enable_audio: false,
            audio_sample_rate: 48000,
            timecode: 0,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SspStatus {
    pub running: bool,
    pub port: u16,
    pub client_connected: bool,
    pub frame_count: u64,
    pub bytes_sent: u64,
    pub fps_actual: f64,
    pub config: SspConfig,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SspLogLine {
    pub time: String,
    pub msg: String,
}

// ─── Server 状态 ───

static SSP_LOGS: std::sync::LazyLock<Mutex<Vec<SspLogLine>>> =
    std::sync::LazyLock::new(|| Mutex::new(Vec::new()));

fn ssp_log(msg: &str) {
    let now = chrono::Local::now().format("%H:%M:%S").to_string();
    let mut logs = SSP_LOGS.lock().unwrap_or_else(|e| e.into_inner());
    logs.push(SspLogLine { time: now, msg: msg.to_string() });
    if logs.len() > 200 {
        logs.remove(0);
    }
    eprintln!("[ssp-server] {}", msg);
}

// ─── ffmpeg 路径 ───

fn find_ffmpeg() -> Result<std::path::PathBuf, String> {
    let path = crate::commands::utils::bin_tool_path("ffmpeg")
        .unwrap_or_else(|| crate::commands::utils::get_bin_dir().join("ffmpeg").join("ffmpeg.exe"));
    if path.exists() {
        Ok(path)
    } else {
        // 最后回退到 PATH 搜索
        let name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
        if let Ok(p) = std::process::Command::new("where").arg(name).output() {
            let out = String::from_utf8_lossy(&p.stdout);
            if let Some(line) = out.lines().next() {
                let candidate = std::path::PathBuf::from(line.trim());
                if candidate.exists() {
                    return Ok(candidate);
                }
            }
        }
        Err(format!("找不到 ffmpeg (已搜索: {})", path.display()))
    }
}

// ─── 协议常量 ───

const SSP_PACKET_META: u32 = 1;
const SSP_PACKET_VIDEO: u32 = 2;
#[allow(dead_code)]
const SSP_PACKET_AUDIO: u32 = 3;
#[allow(dead_code)]
const SSP_PACKET_HEARTBEAT: u32 = 4;
const SSP_PROTOCOL_VERSION: u32 = 1;

/// 写一个 u32 到流 (little-endian)。
fn write_u32(w: &mut dyn Write, v: u32) -> std::io::Result<()> {
    w.write_all(&v.to_le_bytes())
}
fn write_u64(w: &mut dyn Write, v: u64) -> std::io::Result<()> {
    w.write_all(&v.to_le_bytes())
}
fn write_i32(w: &mut dyn Write, v: i32) -> std::io::Result<()> {
    w.write_all(&v.to_le_bytes())
}
fn read_u32(r: &mut dyn Read) -> std::io::Result<u32> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

// ─── NAL 解析器（从 H.264 字节流中拆分帧） ───

/// 从 raw H.264 Annex B 流中提取 NAL 单元，按帧边界分组。
/// 返回 Vec<(pts_offset_ms, nal_units)>，每个帧包含 SPS/PPS/IDR 或 non-IDR 片。
struct H264FrameReader {
    buf: Vec<u8>,
    frame_start_pts: u64,
}

impl H264FrameReader {
    fn new() -> Self {
        Self { buf: Vec::new(), frame_start_pts: 0 }
    }

    /// 追加原始字节，返回已完成的帧列表 (pts_ms, frame_data)。
    /// NAL 起始码统一为 4 字节 0x00_00_00_01。
    fn feed(&mut self, data: &[u8]) -> Vec<(u64, Vec<u8>)> {
        self.buf.extend_from_slice(data);
        let mut frames = Vec::new();
        let mut frame_data = Vec::new();
        let mut in_frame = false;
        let mut i = 0;

        while i + 3 < self.buf.len() {
            if self.buf[i] == 0 && self.buf[i + 1] == 0 {
                let start_len = if self.buf[i + 2] == 1 { 3 } else if self.buf[i + 2] == 0 && self.buf[i + 3] == 1 { 4 } else { i += 1; continue; };

                if in_frame {
                    // 完成当前帧
                    frames.push((self.frame_start_pts, frame_data));
                    frame_data = Vec::new();
                }

                // 提取 NAL 类型
                let nal_start = i + start_len;
                if nal_start < self.buf.len() {
                    let nal_type = self.buf[nal_start] & 0x1f;
                    // 写入 4 字节起始码
                    frame_data.extend_from_slice(&[0, 0, 0, 1]);
                    // 复制到下一个起始码
                    let copy_start = nal_start;
                    let mut copy_end = self.buf.len();
                    for j in nal_start..self.buf.len().saturating_sub(3) {
                        if self.buf[j] == 0 && self.buf[j + 1] == 0 && (self.buf[j + 2] == 1 || (self.buf[j + 2] == 0 && self.buf[j + 3] == 1)) {
                            copy_end = j;
                            break;
                        }
                    }
                    frame_data.extend_from_slice(&self.buf[copy_start..copy_end]);
                    i = copy_end;

                    // 判断帧类型：IDR (5) 或 non-IDR (1) → new frame boundary
                    if nal_type == 5 || nal_type == 1 {
                        in_frame = false; // 这个 NAL 本身已加入 frame_data，下一个起始码会切新帧
                        if !frame_data.is_empty() {
                            frames.push((self.frame_start_pts, frame_data));
                            frame_data = Vec::new();
                        }
                        // 下一个帧从这里开始
                        frame_data.extend_from_slice(&[0, 0, 0, 1]);
                    } else {
                        in_frame = true;
                    }
                } else {
                    i += 1;
                }
            } else {
                i += 1;
            }
        }

        // 移除已处理的数据（保留末尾未完整帧）
        let consumed = self.buf.len().saturating_sub(8);
        self.buf.drain(..consumed.min(self.buf.len()));

        frames
    }
}

// ─── 连接处理 ───

fn handle_connection(
    mut stream: TcpStream,
    ffmpeg_child: Arc<Mutex<Option<Child>>>,
    config: SspConfig,
    stop: Arc<AtomicBool>,
    stats: Arc<Mutex<(u64, u64, u64)>>, // (frames, bytes, start_instant_ms)
    timecode_base: u32,
) {
    let peer = stream.peer_addr().map(|a| a.to_string()).unwrap_or_default();
    ssp_log(&format!("客户端 {} 已连接", peer));

    // 1. 握手
    let client_version = match read_u32(&mut stream) {
        Ok(v) => v,
        Err(e) => { ssp_log(&format!("握手读取失败: {}", e)); return; }
    };
    ssp_log(&format!("客户端协议版本: {}", client_version));

    if client_version != SSP_PROTOCOL_VERSION {
        let code = if client_version > SSP_PROTOCOL_VERSION { -1000i32 } else { -1001i32 };
        let _ = write_i32(&mut stream, code);
        ssp_log(&format!("版本不匹配，发送错误码 {}", code));
        return;
    }
    let _ = write_i32(&mut stream, SSP_PROTOCOL_VERSION as i32);

    // 2. 发送元数据
    {
        let mut meta_buf = Vec::new();
        // type=1, 先占位 len
        write_u32(&mut meta_buf, SSP_PACKET_META).unwrap();
        write_u32(&mut meta_buf, 0).unwrap(); // placeholder

        // VideoMeta: width, height, timescale, unit, gop, encoder (6 × 4 = 24 bytes)
        let encoder_id = if config.video_codec == "h265" { 265u32 } else { 96u32 };
        write_u32(&mut meta_buf, config.width).unwrap();
        write_u32(&mut meta_buf, config.height).unwrap();
        write_u32(&mut meta_buf, config.fps * 1000).unwrap(); // timescale
        write_u32(&mut meta_buf, 1000).unwrap(); // unit
        write_u32(&mut meta_buf, config.gop).unwrap();
        write_u32(&mut meta_buf, encoder_id).unwrap();

        // AudioMeta: timescale, unit, sample_rate, sample_size, channel, bitrate, encoder (7 × 4 = 28)
        let audio_encoder = if config.enable_audio { 37u32 } else { 0u32 };
        write_u32(&mut meta_buf, config.audio_sample_rate).unwrap();
        write_u32(&mut meta_buf, 1).unwrap();
        write_u32(&mut meta_buf, config.audio_sample_rate).unwrap();
        write_u32(&mut meta_buf, 16).unwrap();
        write_u32(&mut meta_buf, if config.enable_audio { 2 } else { 0 }).unwrap();
        write_u32(&mut meta_buf, 128000u32).unwrap();
        write_u32(&mut meta_buf, audio_encoder).unwrap();

        // SspMeta: pts_is_wall_clock(bool=1B), tc_drop_frame(bool=1B), padding 2B, timecode(4B)
        meta_buf.push(0); // pts_is_wall_clock = false
        meta_buf.push(0); // tc_drop_frame = false
        write_u32(&mut meta_buf, timecode_base).unwrap();

        // 回填 total_length
        let total = meta_buf.len() as u32;
        meta_buf[4..8].copy_from_slice(&total.to_le_bytes());

        let _ = stream.write_all(&meta_buf);
    }

    // 3. 视频帧循环
    let start = std::time::Instant::now();
    let mut frm_no: u32 = 0;

    // 读取 ffmpeg stdout 并解析 NAL
    let mut reader = H264FrameReader::new();
    let mut read_buf = vec![0u8; 65536];

    // 先取 ffmpeg stdout
    let mut stdout = {
        let mut guard = ffmpeg_child.lock().unwrap_or_else(|e| e.into_inner());
        guard.as_mut().and_then(|c| c.stdout.take())
    };

    let mut last_heartbeat = start;
    let heart_interval = Duration::from_secs(5);

    'stream: while !stop.load(Ordering::Relaxed) {
        if let Some(ref mut out) = stdout {
            match out.read(&mut read_buf) {
                Ok(0) => break,
                Ok(n) => {
                    let frames = reader.feed(&read_buf[..n]);
                    for (offset_ms, frame_buf) in frames {
                        // 构建 SSP 视频包
                        let pts = offset_ms; // ms
                        let ntp_now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64;

                        let data_len = frame_buf.len() as u32;
                        let total = 8 + 4 + 8 + 8 + 4 + 4 + 4 + data_len; // headers + data
                        let mut pkt = Vec::with_capacity(total as usize);
                        write_u32(&mut pkt, SSP_PACKET_VIDEO).unwrap();
                        write_u32(&mut pkt, total).unwrap();
                        write_u32(&mut pkt, 0).unwrap(); // stream_style = DEFAULT
                        write_u64(&mut pkt, pts).unwrap();
                        write_u64(&mut pkt, ntp_now).unwrap();
                        write_u32(&mut pkt, frm_no).unwrap();
                        // frame type: check if contains IDR (NAL type 5)
                        let is_idr = frame_buf.windows(5).any(|w| w[0] == 0 && w[1] == 0 && w[2] == 0 && w[3] == 1 && (w[4] & 0x1f) == 5);
                        write_u32(&mut pkt, if is_idr { 0 } else { 1 }).unwrap();
                        write_u32(&mut pkt, data_len).unwrap();
                        pkt.extend_from_slice(&frame_buf);

                        if stream.write_all(&pkt).is_err() {
                            ssp_log("客户端断开");
                            break 'stream;
                        }

                        frm_no += 1;
                        {
                            let mut s = stats.lock().unwrap_or_else(|e| e.into_inner());
                            s.0 += 1;
                            s.1 += pkt.len() as u64;
                        }
                    }
                }
                Err(e) => {
                    ssp_log(&format!("ffmpeg 读取错误: {}", e));
                    break;
                }
            }
        }

        // 心跳
        if last_heartbeat.elapsed() >= heart_interval {
            let mut hb = Vec::with_capacity(8);
            write_u32(&mut hb, SSP_PACKET_HEARTBEAT).unwrap();
            write_u32(&mut hb, 8).unwrap();
            if stream.write_all(&hb).is_err() {
                break;
            }
            last_heartbeat = std::time::Instant::now();
        }

        thread::sleep(Duration::from_millis(1));
    }

    ssp_log(&format!("客户端 {} 断开，共发送 {} 帧", peer, frm_no));
}

// ─── Tauri 命令 ───

static SSP_RUNNING: AtomicBool = AtomicBool::new(false);
static SSP_STOP_FLAG: std::sync::LazyLock<AtomicBool> =
    std::sync::LazyLock::new(|| AtomicBool::new(false));

/// 启动 SSP 模拟服务。
#[tauri::command]
pub fn ssp_start(config: SspConfig) -> Result<SspStatus, String> {
    if SSP_RUNNING.swap(true, Ordering::SeqCst) {
        return Err("SSP 服务已在运行".to_string());
    }
    SSP_STOP_FLAG.store(false, Ordering::SeqCst);

    let ffmpeg_path = find_ffmpeg()?;
    ssp_log(&format!("启动 SSP 模拟服务，端口 {}", config.port));

    // 1. 启动 ffmpeg
    let mut ffmpeg_args: Vec<String> = vec![
        "-re".into(),
    ];

    if config.source_type == "testsrc" {
        let size = format!("{}x{}", config.width, config.height);
        ffmpeg_args.extend(vec![
            "-f".into(), "lavfi".into(),
            "-i".into(), format!("testsrc=size={}:rate={}", size, config.fps),
        ]);
    } else {
        let file = config.file_path.as_deref().unwrap_or("");
        if file.is_empty() || !std::path::Path::new(file).exists() {
            SSP_RUNNING.store(false, Ordering::SeqCst);
            return Err("视频文件不存在".to_string());
        }
        ffmpeg_args.extend(vec![
            "-stream_loop".into(), "-1".into(),
            "-i".into(), file.to_string(),
        ]);
    }

    // 视频编码参数
    let encoder = if config.video_codec == "h265" { "libx265" } else { "libx264" };
    ffmpeg_args.extend(vec![
        "-c:v".into(), encoder.into(),
        "-preset".into(), "ultrafast".into(),
        "-tune".into(), "zerolatency".into(),
        "-g".into(), config.gop.to_string(),
        "-keyint_min".into(), config.gop.to_string(),
        "-bf".into(), "0".into(),
        "-pix_fmt".into(), "yuv420p".into(),
        "-r".into(), config.fps.to_string(),
    ]);

    if config.bitrate_kbps > 0 {
        ffmpeg_args.extend(vec!["-b:v".into(), format!("{}k", config.bitrate_kbps)]);
    }

    // 不输出音频（SSP 简单模拟）
    ffmpeg_args.push("-an".into());

    // 输出 raw H.264 Annex B 到 stdout
    ffmpeg_args.extend(vec![
        "-f".into(), "h264".into(),
        "pipe:1".into(),
    ]);

    let mut cmd = Command::new(&ffmpeg_path);
    cmd.args(&ffmpeg_args);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let child = cmd.spawn().map_err(|e| format!("启动 ffmpeg 失败: {}", e))?;
    let ffmpeg_child = Arc::new(Mutex::new(Some(child)));

    // 2. 启动 TCP 监听
    let port = config.port;
    let listener = TcpListener::bind(format!("0.0.0.0:{}", port))
        .map_err(|e| format!("绑定端口失败: {}", e))?;
    listener.set_nonblocking(true).map_err(|e| format!("设置非阻塞失败: {}", e))?;
    ssp_log(&format!("TCP 监听已启动: 0.0.0.0:{}", port));

    // 3. 后台线程：接受连接 + 发送帧
    let config_clone = config.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = stop.clone();
    let ffmpeg_child_clone = ffmpeg_child.clone();
    let stats = Arc::new(Mutex::new((0u64, 0u64, 0u64)));

    thread::spawn(move || {
        // 接受连接循环
        while !stop_clone.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let cfg = config_clone.clone();
                    let ff = ffmpeg_child_clone.clone();
                    let st = stop_clone.clone();
                    let stat = stats.clone();
                    thread::spawn(move || {
                        handle_connection(stream, ff, cfg, st, stat, 0);
                    });
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(200));
                }
                Err(_) => break,
            }
        }
    });

    Ok(SspStatus {
        running: true,
        port,
        client_connected: false,
        frame_count: 0,
        bytes_sent: 0,
        fps_actual: 0.0,
        config,
    })
}

/// 停止 SSP 模拟服务。
#[tauri::command]
pub fn ssp_stop() -> Result<(), String> {
    if !SSP_RUNNING.swap(false, Ordering::SeqCst) {
        return Err("SSP 服务未在运行".to_string());
    }
    SSP_STOP_FLAG.store(true, Ordering::SeqCst);
    ssp_log("SSP 服务已停止");
    Ok(())
}

/// 获取 SSP 服务状态（客户端轮询）。
#[tauri::command]
pub fn ssp_status() -> SspStatus {
    SspStatus {
        running: SSP_RUNNING.load(Ordering::Relaxed),
        port: 9999,
        client_connected: false,
        frame_count: 0,
        bytes_sent: 0,
        fps_actual: 0.0,
        config: SspConfig::default(),
    }
}

/// 获取 SSP 日志。
#[tauri::command]
pub fn ssp_logs() -> Vec<SspLogLine> {
    SSP_LOGS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}