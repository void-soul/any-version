// ─── 串口调试器模块 ───
// 基于 `serialport` crate。读取在独立线程中进行，通过
// `serial://data` / `serial://closed` 事件推送给前端。

use serde::{Deserialize, Serialize};
use serialport::{DataBits, FlowControl, Parity, StopBits};
use std::collections::HashMap;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tracing::{info, warn};

#[derive(Serialize)]
pub struct SerialPortInfo {
    pub name: String,
    pub description: String,
}

/// 枚举系统可用串口。
#[tauri::command]
pub fn serial_list_ports() -> Result<Vec<SerialPortInfo>, String> {
    let ports = serialport::available_ports().map_err(|e| format!("枚举串口失败: {}", e))?;
    Ok(ports
        .into_iter()
        .map(|p| {
            let description = match &p.port_type {
                serialport::SerialPortType::UsbPort(info) => info
                    .product
                    .clone()
                    .unwrap_or_else(|| "USB 串口设备".to_string()),
                _ => "串口".to_string(),
            };
            SerialPortInfo {
                name: p.port_name,
                description,
            }
        })
        .collect())
}

struct SerialConnection {
    /// 写句柄（读线程持有 clone）
    writer: Mutex<Box<dyn serialport::SerialPort>>,
    should_close: std::sync::Arc<AtomicBool>,
}

#[derive(Default)]
pub struct SerialState {
    connections: Mutex<HashMap<String, SerialConnection>>,
}

fn parse_parity(s: &str) -> Parity {
    match s {
        "even" => Parity::Even,
        "odd" => Parity::Odd,
        _ => Parity::None,
    }
}

fn parse_data_bits(n: u8) -> DataBits {
    match n {
        5 => DataBits::Five,
        6 => DataBits::Six,
        7 => DataBits::Seven,
        _ => DataBits::Eight,
    }
}

fn parse_stop_bits(n: u8) -> StopBits {
    match n {
        2 => StopBits::Two,
        _ => StopBits::One,
    }
}

fn parse_flow(s: &str) -> FlowControl {
    match s {
        "hardware" => FlowControl::Hardware,
        "software" => FlowControl::Software,
        _ => FlowControl::None,
    }
}

/// 打开串口并启动接收线程。重复打开同一端口会先关闭旧连接。
#[tauri::command]
pub fn serial_open(
    app: AppHandle,
    state: tauri::State<'_, SerialState>,
    port_name: String,
    baud_rate: u32,
    data_bits: u8,
    parity: String,
    stop_bits: u8,
    flow_control: String,
) -> Result<(), String> {
    // 已有连接则先关
    let _ = serial_close_inner(&state, &port_name);

    let port = serialport::new(&port_name, baud_rate)
        .data_bits(parse_data_bits(data_bits))
        .parity(parse_parity(&parity))
        .stop_bits(parse_stop_bits(stop_bits))
        .flow_control(parse_flow(&flow_control))
        .timeout(Duration::from_millis(50))
        .open()
        .map_err(|e| format!("打开 {} 失败: {}（可能被占用或权限不足）", port_name, e))?;

    let reader = port.try_clone().map_err(|e| format!("复制串口句柄失败: {}", e))?;

    let conn = SerialConnection {
        writer: Mutex::new(port),
        should_close: std::sync::Arc::new(AtomicBool::new(false)),
    };
    state
        .connections
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(port_name.clone(), conn);

    let close_flag = state
        .connections
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&port_name)
        .map(|c| c.should_close.clone())
        .expect("刚插入的连接必然存在");

    let app_for_thread = app.clone();
    let port_for_thread = port_name.clone();
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        info!("[serial] 开始监听 {}", port_for_thread);
        loop {
            if close_flag.load(Ordering::Relaxed) {
                break;
            }
            match reader.read(&mut buf) {
                Ok(0) => continue,
                Ok(n) => {
                    let bytes = &buf[..n];
                    let hex: String = bytes.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
                    let _ = app_for_thread.emit(
                        "serial://data",
                        serde_json::json!({
                            "port": port_for_thread,
                            "hex": hex,
                            // 非法 UTF-8 以替换字符展示，前端可切 HEX 视图看原始字节
                            "text": String::from_utf8_lossy(bytes),
                            "len": n,
                        }),
                    );
                }
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                Err(e) => {
                    warn!("[serial] {} 读错误: {}，停止接收", port_for_thread, e);
                    break;
                }
            }
        }
        info!("[serial] {} 接收线程退出", port_for_thread);
        let _ = app_for_thread.emit("serial://closed", serde_json::json!({ "port": port_for_thread }));
    });

    info!("[serial] {} 已打开 @{}", port_name, baud_rate);
    Ok(())
}

fn serial_close_inner(state: &SerialState, port_name: &str) -> Result<(), String> {
    let mut map = state.connections.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(conn) = map.remove(port_name) {
        conn.should_close.store(true, Ordering::Relaxed);
        // drop(writer) 关闭端口 → 读线程 read 返回错误后自然退出
    }
    Ok(())
}

/// 关闭串口。
#[tauri::command]
pub fn serial_close(state: tauri::State<'_, SerialState>, port_name: String) -> Result<(), String> {
    serial_close_inner(&state, &port_name)
}

/// 写数据：hex_mode=true 时按十六进制解析（支持空格/逗号分隔），否则按 UTF-8 文本发送。
#[tauri::command]
pub fn serial_write(
    state: tauri::State<'_, SerialState>,
    port_name: String,
    data: String,
    hex_mode: bool,
    append_newline: bool,
) -> Result<usize, String> {
    let bytes: Vec<u8> = if hex_mode {
        let clean: String = data.chars().filter(|c| !c.is_whitespace() && *c != ',' && *c != ':' && *c != '-').collect();
        if clean.len() % 2 != 0 {
            return Err("HEX 数据长度必须为偶数位".to_string());
        }
        (0..clean.len() / 2)
            .map(|i| u8::from_str_radix(&clean[i * 2..i * 2 + 2], 16))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("HEX 解析失败: {}", e))?
    } else {
        let mut b = data.into_bytes();
        if append_newline {
            b.push(b'\n');
        }
        b
    };

    let map = state.connections.lock().unwrap_or_else(|e| e.into_inner());
    let conn = map
        .get(&port_name)
        .ok_or_else(|| format!("{} 未打开", port_name))?;
    let mut writer = conn.writer.lock().unwrap_or_else(|e| e.into_inner());
    writer
        .write_all(&bytes)
        .map_err(|e| format!("写入失败: {}", e))?;
    let _ = writer.flush();
    Ok(bytes.len())
}

// ─── 模拟设备（脚本应答） ───
// 不占用真实串口：前端发来的数据在进程内匹配应答脚本，
// 收到（rx）与设备应答（tx）均通过 serial://sim-data 事件推送。

/// 一条应答规则：收到匹配 pattern 的数据时回复 response。
#[derive(Deserialize, Clone)]
pub struct SimRule {
    pub pattern: String,
    pub pattern_hex: bool,
    /// contains | prefix | exact | regex（regex 仅对文本有效）
    pub match_type: String,
    pub response: String,
    pub response_hex: bool,
    pub append_newline: bool,
    pub delay_ms: u64,
}

#[derive(Serialize, Clone)]
pub struct SimStartResult {
    pub rule_count: usize,
}

#[derive(Default)]
pub struct SimState {
    active: AtomicBool,
}

fn hex_to_bytes(data: &str) -> Result<Vec<u8>, String> {
    let clean: String = data
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ',' && *c != ':' && *c != '-')
        .collect();
    if clean.len() % 2 != 0 {
        return Err("HEX 数据长度必须为偶数位".into());
    }
    (0..clean.len() / 2)
        .map(|i| u8::from_str_radix(&clean[i * 2..i * 2 + 2], 16))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("HEX 解析失败: {}", e))
}

fn build_payload(data: &str, hex_mode: bool, append_newline: bool) -> Result<Vec<u8>, String> {
    if hex_mode {
        hex_to_bytes(data)
    } else {
        let mut b = data.as_bytes().to_vec();
        if append_newline {
            b.push(b'\n');
        }
        Ok(b)
    }
}

fn bytes_contain(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}

fn emit_sim(app: &AppHandle, port_label: &str, dir: &str, bytes: &[u8]) {
    let hex: String = bytes.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
    let _ = app.emit(
        "serial://sim-data",
        serde_json::json!({
            "port": port_label,
            "dir": dir,
            "hex": hex,
            "text": String::from_utf8_lossy(bytes),
            "len": bytes.len(),
        }),
    );
}

/// 启动模拟设备（保存规则并标记激活）。
#[tauri::command]
pub fn serial_sim_start(
    state: tauri::State<'_, SimState>,
    rules: Vec<SimRule>,
) -> Result<SimStartResult, String> {
    for (i, r) in rules.iter().enumerate() {
        // 提前校验，避免运行期才报错
        build_payload(&r.pattern, r.pattern_hex, false)
            .map_err(|e| format!("规则 {} 匹配内容错误: {}", i + 1, e))?;
        build_payload(&r.response, r.response_hex, r.append_newline)
            .map_err(|e| format!("规则 {} 应答内容错误: {}", i + 1, e))?;
        if r.match_type == "regex" && !r.pattern_hex {
            regex::Regex::new(&r.pattern).map_err(|e| format!("规则 {} 正则无效: {}", i + 1, e))?;
        }
    }
    state.active.store(true, Ordering::Relaxed);
    info!("[serial-sim] 模拟设备已启动，规则数={}", rules.len());
    Ok(SimStartResult { rule_count: rules.len() })
}

/// 停止模拟设备。
#[tauri::command]
pub fn serial_sim_stop(state: tauri::State<'_, SimState>) -> Result<(), String> {
    state.active.store(false, Ordering::Relaxed);
    info!("[serial-sim] 模拟设备已停止");
    Ok(())
}

/// 向模拟设备写入数据：立即回显 rx 事件；命中第一条规则则延迟应答 tx。
/// 返回命中的规则序号（0 起），未命中返回 -1。
#[tauri::command]
pub fn serial_sim_write(
    app: AppHandle,
    state: tauri::State<'_, SimState>,
    data: String,
    hex_mode: bool,
    append_newline: bool,
    rules: Vec<SimRule>,
) -> Result<i64, String> {
    if !state.active.load(Ordering::Relaxed) {
        return Err("模拟设备未启动".into());
    }
    let bytes = build_payload(&data, hex_mode, append_newline)?;
    emit_sim(&app, "SIM", "rx", &bytes);

    let text = String::from_utf8_lossy(&bytes).to_string();
    for (i, r) in rules.iter().enumerate() {
        // 正则仅对文本模式有效；其余按字节比较（HEX/文本统一转字节）
        let matched = if r.match_type == "regex" && !r.pattern_hex {
            match regex::Regex::new(&r.pattern) {
                Ok(re) => re.is_match(&text),
                Err(_) => false,
            }
        } else {
            let pat = build_payload(&r.pattern, r.pattern_hex, false)?;
            match r.match_type.as_str() {
                "prefix" => bytes.starts_with(&pat),
                "exact" => bytes == pat,
                _ => bytes_contain(&bytes, &pat),
            }
        };
        if matched {
            let payload = build_payload(&r.response, r.response_hex, r.append_newline)
                .map_err(|e| format!("规则 {} 应答内容错误: {}", i + 1, e))?;
            let app2 = app.clone();
            let delay = r.delay_ms.min(10_000);
            std::thread::spawn(move || {
                if delay > 0 {
                    std::thread::sleep(Duration::from_millis(delay));
                }
                emit_sim(&app2, "SIM", "tx", &payload);
            });
            return Ok(i as i64);
        }
    }
    Ok(-1)
}
