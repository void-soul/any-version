// ─── 串口调试器模块 ───
// 基于 `serialport` crate。读取在独立线程中进行，通过
// `serial://data` / `serial://closed` 事件推送给前端。
//
// 事件里同时给出 base64 原始字节：前端可以按 UTF-8 / GBK 自行解码，
// 不再依赖后端的 `from_utf8_lossy`（那会把 GBK 中文显示成乱码）。

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use encoding_rs::GBK;
use serde::{Deserialize, Serialize};
use serialport::{DataBits, FlowControl, Parity, StopBits};
use std::collections::HashMap;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tracing::{info, warn};

#[derive(Serialize)]
pub struct SerialPortInfo {
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vid: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
}

/// 枚举系统可用串口。
#[tauri::command]
pub fn serial_list_ports() -> Result<Vec<SerialPortInfo>, String> {
    let ports = serialport::available_ports().map_err(|e| format!("枚举串口失败: {}", e))?;
    Ok(ports
        .into_iter()
        .map(|p| {
            let (description, vid, pid, serial_number, manufacturer) = match &p.port_type {
                serialport::SerialPortType::UsbPort(info) => (
                    info.product
                        .clone()
                        .unwrap_or_else(|| "USB 串口设备".to_string()),
                    Some(info.vid),
                    Some(info.pid),
                    info.serial_number.clone(),
                    info.manufacturer.clone(),
                ),
                serialport::SerialPortType::PciPort => ("PCI 串口".to_string(), None, None, None, None),
                serialport::SerialPortType::BluetoothPort => {
                    ("蓝牙串口".to_string(), None, None, None, None)
                }
                serialport::SerialPortType::Unknown => ("串口".to_string(), None, None, None, None),
            };
            SerialPortInfo {
                name: p.port_name,
                description,
                vid,
                pid,
                serial_number,
                manufacturer,
            }
        })
        .collect())
}

struct SerialConnection {
    /// 写句柄（读线程持有 clone）
    writer: Mutex<Box<dyn serialport::SerialPort>>,
    should_close: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct SerialState {
    /// 读线程也需要能清理自己，因此用 Arc 共享。
    connections: Arc<Mutex<HashMap<String, SerialConnection>>>,
}

pub fn close_all(state: &SerialState, simulation: &SimState) {
    simulation.active.store(false,Ordering::Relaxed);
    let connections=std::mem::take(&mut *state.connections.lock().unwrap_or_else(|e|e.into_inner()));
    for (_,connection) in connections {connection.should_close.store(true,Ordering::Relaxed);}
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

fn line_ending_bytes(kind: &str) -> &'static [u8] {
    match kind {
        "lf" | "\n" => b"\n",
        "crlf" | "\r\n" => b"\r\n",
        "cr" | "\r" => b"\r",
        _ => b"",
    }
}

fn encode_text(text: &str, encoding: &str) -> Vec<u8> {
    if encoding.eq_ignore_ascii_case("gbk") {
        let (bytes, _, _) = GBK.encode(text);
        bytes.into_owned()
    } else {
        text.as_bytes().to_vec()
    }
}

fn decode_text(bytes: &[u8], encoding: &str) -> String {
    if encoding.eq_ignore_ascii_case("gbk") {
        let (text, _, _) = GBK.decode(bytes);
        text.into_owned()
    } else {
        String::from_utf8_lossy(bytes).into_owned()
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
    crate::runtime_tasks::require_enabled("serial")?;
    if baud_rate == 0 {
        return Err("波特率必须大于 0".into());
    }
    // 已有连接则先关；读线程以 50ms 超时轮询，等待它释放句柄，
    // 否则 Windows 上立即重开同一端口会因句柄未完全关闭而报「拒绝访问」。
    if serial_close_inner(&state, &port_name) {
        std::thread::sleep(Duration::from_millis(80));
    }

    let port = serialport::new(&port_name, baud_rate)
        .data_bits(parse_data_bits(data_bits))
        .parity(parse_parity(&parity))
        .stop_bits(parse_stop_bits(stop_bits))
        .flow_control(parse_flow(&flow_control))
        .timeout(Duration::from_millis(50))
        .open()
        .map_err(|e| format!("打开 {} 失败: {}（可能被占用或权限不足）", port_name, e))?;

    let reader = port.try_clone().map_err(|e| format!("复制串口句柄失败: {}", e))?;
    let close_flag = Arc::new(AtomicBool::new(false));

    let conn = SerialConnection {
        writer: Mutex::new(port),
        should_close: close_flag.clone(),
    };
    state
        .connections
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(port_name.clone(), conn);

    let connections = state.connections.clone();
    let app_for_thread = app.clone();
    let port_for_thread = port_name.clone();
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        let mut read_error: Option<String> = None;
        info!("[serial] 开始监听 {}", port_for_thread);
        loop {
            if close_flag.load(Ordering::Relaxed) {
                break;
            }
            match reader.read(&mut buf) {
                Ok(0) => continue,
                Ok(n) => {
                    let bytes = &buf[..n];
                    emit_serial_data(&app_for_thread, &port_for_thread, bytes);
                }
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                Err(e) => {
                    warn!("[serial] {} 读错误: {}，停止接收", port_for_thread, e);
                    // 读错误通常意味着设备被拔出；原因留到清理时统一发一次 closed 事件
                    read_error = Some(e.to_string());
                    break;
                }
            }
        }
        info!("[serial] {} 接收线程退出", port_for_thread);
        drop(reader);

        // 只有当前连接仍是「自己」时才移除映射：
        // 用户主动关闭或已重开同一端口时，旧线程不能把新连接的状态一起清掉，
        // 更不能补发 closed 事件（会把前端刚建立的连接显示成已断开）。
        let removed = {
            let mut map = connections.lock().unwrap_or_else(|e| e.into_inner());
            match map.get(&port_for_thread) {
                Some(c) if Arc::ptr_eq(&c.should_close, &close_flag) => {
                    map.remove(&port_for_thread);
                    true
                }
                _ => false,
            }
        }; // 这里 drop 掉 writer，确保端口句柄真正释放
        if removed {
            emit_closed(
                &app_for_thread,
                &port_for_thread,
                read_error.as_deref(),
            );
        }
    });

    info!("[serial] {} 已打开 @{}", port_name, baud_rate);
    Ok(())
}

fn emit_serial_data(app: &AppHandle, port: &str, bytes: &[u8]) {
    let hex: String = bytes.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
    let _ = app.emit(
        "serial://data",
        serde_json::json!({
            "port": port,
            "hex": hex,
            // 原始字节（base64）：前端按所选编码解码，GBK 中文才不会乱码
            "data": B64.encode(bytes),
            "text": String::from_utf8_lossy(bytes),
            "len": bytes.len(),
        }),
    );
}

fn emit_closed(app: &AppHandle, port: &str, reason: Option<&str>) {
    let _ = app.emit(
        "serial://closed",
        serde_json::json!({ "port": port, "reason": reason }),
    );
}

fn serial_close_inner(state: &SerialState, port_name: &str) -> bool {
    let mut map = state.connections.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(conn) = map.remove(port_name) {
        conn.should_close.store(true, Ordering::Relaxed);
        // drop(conn) 关闭端口 → 读线程 read 返回错误后自然退出
        true
    } else {
        false
    }
}

/// 关闭串口。
#[tauri::command]
pub fn serial_close(state: tauri::State<'_, SerialState>, port_name: String) -> Result<(), String> {
    serial_close_inner(&state, &port_name);
    Ok(())
}

/// 写数据：hex_mode=true 时按十六进制解析（支持空格/逗号分隔），否则按所选编码发送文本。
#[tauri::command]
pub fn serial_write(
    state: tauri::State<'_, SerialState>,
    port_name: String,
    data: String,
    hex_mode: bool,
    encoding: String,
    line_ending: String,
) -> Result<usize, String> {
    let bytes = build_payload(&data, hex_mode, &encoding, &line_ending)?;

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

/// 控制 DTR / RTS 电平（单片机复位、硬件流控调试常用）。
#[tauri::command]
pub fn serial_set_signals(
    state: tauri::State<'_, SerialState>,
    port_name: String,
    dtr: Option<bool>,
    rts: Option<bool>,
) -> Result<(), String> {
    let map = state.connections.lock().unwrap_or_else(|e| e.into_inner());
    let conn = map
        .get(&port_name)
        .ok_or_else(|| format!("{} 未打开", port_name))?;
    let mut writer = conn.writer.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(level) = dtr {
        writer
            .write_data_terminal_ready(level)
            .map_err(|e| format!("设置 DTR 失败: {}", e))?;
    }
    if let Some(level) = rts {
        writer
            .write_request_to_send(level)
            .map_err(|e| format!("设置 RTS 失败: {}", e))?;
    }
    Ok(())
}

// ─── 模拟设备（脚本应答） ───
// 不占用真实串口：前端发来的数据在进程内匹配应答脚本，
// 收到（rx）与设备应答（tx）均通过 serial://sim-data 事件推送。

/// 一条应答规则：收到匹配 pattern 的数据时回复 response。
#[derive(Deserialize, Clone, Debug)]
pub struct SimRule {
    #[serde(default)]
    pub pattern: String,
    #[serde(default)]
    pub pattern_hex: bool,
    /// contains | prefix | exact | regex（regex 仅对文本有效）
    #[serde(default = "default_match_type")]
    pub match_type: String,
    #[serde(default)]
    pub response: String,
    #[serde(default)]
    pub response_hex: bool,
    #[serde(default)]
    pub append_newline: bool,
    #[serde(default)]
    pub delay_ms: u64,
}

fn default_match_type() -> String {
    "contains".to_string()
}

#[derive(Serialize, Clone)]
pub struct SimStartResult {
    pub rule_count: usize,
}

pub struct SimState {
    active: Arc<AtomicBool>,
    rules: Mutex<Vec<SimRule>>,
}

impl Default for SimState {
    fn default() -> Self {
        Self {
            active: Arc::new(AtomicBool::new(false)),
            rules: Mutex::new(Vec::new()),
        }
    }
}

fn hex_to_bytes(data: &str) -> Result<Vec<u8>, String> {
    let clean: String = data
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ',' && *c != ':' && *c != '-')
        .collect();
    // 非 ASCII 必须先挡掉：下面按**字节**下标两两切片，混进多字节字符会切在字符边界
    // 中间，`&s[a..b]` 直接 panic（只查偶数位挡不住它 —— 一个中文字占 3 字节）。
    if !clean.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("HEX 只能包含 0-9 / A-F（可夹杂空格、逗号、冒号或短横作分隔）".into());
    }
    if clean.len() % 2 != 0 {
        return Err("HEX 数据长度必须为偶数位".into());
    }
    (0..clean.len() / 2)
        .map(|i| u8::from_str_radix(&clean[i * 2..i * 2 + 2], 16))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("HEX 解析失败: {}", e))
}

fn build_payload(
    data: &str,
    hex_mode: bool,
    encoding: &str,
    line_ending: &str,
) -> Result<Vec<u8>, String> {
    if hex_mode {
        hex_to_bytes(data)
    } else {
        let mut b = encode_text(data, encoding);
        b.extend_from_slice(line_ending_bytes(line_ending));
        Ok(b)
    }
}

fn bytes_contain(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}

fn rule_matches(
    rule: &SimRule,
    bytes: &[u8],
    text: &str,
    encoding: &str,
) -> Result<bool, String> {
    // 正则仅对文本模式有效；其余按字节比较（HEX/文本统一转字节）
    if rule.match_type == "regex" && !rule.pattern_hex {
        let re = regex::Regex::new(&rule.pattern).map_err(|e| format!("正则无效: {}", e))?;
        return Ok(re.is_match(text));
    }
    let pat = build_payload(&rule.pattern, rule.pattern_hex, encoding, "none")?;
    Ok(match rule.match_type.as_str() {
        "prefix" => bytes.starts_with(&pat),
        "exact" => bytes == pat,
        _ => bytes_contain(bytes, &pat),
    })
}

fn validate_rules(rules: &[SimRule]) -> Result<(), String> {
    for (i, r) in rules.iter().enumerate() {
        // 提前校验，避免运行期才报错
        build_payload(&r.pattern, r.pattern_hex, "utf-8", "none")
            .map_err(|e| format!("规则 {} 匹配内容错误: {}", i + 1, e))?;
        build_payload(&r.response, r.response_hex, "utf-8", "none")
            .map_err(|e| format!("规则 {} 应答内容错误: {}", i + 1, e))?;
        if r.match_type == "regex" && !r.pattern_hex {
            regex::Regex::new(&r.pattern).map_err(|e| format!("规则 {} 正则无效: {}", i + 1, e))?;
        }
    }
    Ok(())
}

fn emit_sim(app: &AppHandle, port_label: &str, dir: &str, bytes: &[u8]) {
    let hex: String = bytes.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
    let _ = app.emit(
        "serial://sim-data",
        serde_json::json!({
            "port": port_label,
            "dir": dir,
            "hex": hex,
            "data": B64.encode(bytes),
            "text": String::from_utf8_lossy(bytes),
            "len": bytes.len(),
        }),
    );
}

/// 启动模拟设备（校验并保存规则，标记激活）。
#[tauri::command]
pub fn serial_sim_start(
    state: tauri::State<'_, SimState>,
    rules: Vec<SimRule>,
) -> Result<SimStartResult, String> {
    validate_rules(&rules)?;
    *state.rules.lock().unwrap_or_else(|e| e.into_inner()) = rules.clone();
    state.active.store(true, Ordering::Relaxed);
    info!("[serial-sim] 模拟设备已启动，规则数={}", rules.len());
    Ok(SimStartResult {
        rule_count: rules.len(),
    })
}

/// 运行中更新应答规则（编辑规则后无需重启设备）。
#[tauri::command]
pub fn serial_sim_update_rules(
    state: tauri::State<'_, SimState>,
    rules: Vec<SimRule>,
) -> Result<(), String> {
    validate_rules(&rules)?;
    *state.rules.lock().unwrap_or_else(|e| e.into_inner()) = rules;
    Ok(())
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
    encoding: String,
    line_ending: String,
) -> Result<i64, String> {
    if !state.active.load(Ordering::Relaxed) {
        return Err("模拟设备未启动".into());
    }
    let bytes = build_payload(&data, hex_mode, &encoding, &line_ending)?;
    emit_sim(&app, "SIM", "rx", &bytes);

    let text = decode_text(&bytes, &encoding);
    let rules = state
        .rules
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    for (i, r) in rules.iter().enumerate() {
        if rule_matches(r, &bytes, &text, &encoding)? {
            let payload = build_payload(
                &r.response,
                r.response_hex,
                &encoding,
                if r.append_newline { "lf" } else { "none" },
            )
            .map_err(|e| format!("规则 {} 应答内容错误: {}", i + 1, e))?;
            let app2 = app.clone();
            let delay = r.delay_ms.min(10_000);
            // 停止设备后不再发出排队中的应答
            let active = state.active.clone();
            std::thread::spawn(move || {
                if delay > 0 {
                    std::thread::sleep(Duration::from_millis(delay));
                }
                if active.load(Ordering::Relaxed) {
                    emit_sim(&app2, "SIM", "tx", &payload);
                }
            });
            return Ok(i as i64);
        }
    }
    Ok(-1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(pattern: &str, pattern_hex: bool, match_type: &str) -> SimRule {
        SimRule {
            pattern: pattern.to_string(),
            pattern_hex,
            match_type: match_type.to_string(),
            response: "OK".to_string(),
            response_hex: false,
            append_newline: true,
            delay_ms: 0,
        }
    }

    #[test]
    fn hex_parsing_accepts_common_separators() {
        assert_eq!(hex_to_bytes("01 A0 ff").unwrap(), vec![0x01, 0xA0, 0xFF]);
        assert_eq!(hex_to_bytes("01,02:03-04").unwrap(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn hex_parsing_rejects_odd_length_and_non_ascii() {
        assert!(hex_to_bytes("01 A").is_err());
        // 中文字符占 3 字节，必须被挡在切片之前，不能 panic
        assert!(hex_to_bytes("01 中文").is_err());
    }

    #[test]
    fn payload_appends_selected_line_ending() {
        assert_eq!(build_payload("AT", false, "utf-8", "crlf").unwrap(), b"AT\r\n");
        assert_eq!(build_payload("AT", false, "utf-8", "none").unwrap(), b"AT");
        // HEX 模式不追加行结束符
        assert_eq!(build_payload("41 54", true, "utf-8", "crlf").unwrap(), b"AT");
    }

    #[test]
    fn gbk_text_is_encoded_and_decoded_roundtrip() {
        let bytes = encode_text("中文", "gbk");
        assert_eq!(bytes, vec![0xD6, 0xD0, 0xCE, 0xC4]);
        assert_eq!(decode_text(&bytes, "gbk"), "中文");
    }

    #[test]
    fn rule_match_modes_work() {
        let hay = b"AT+CSQ\r\n";
        assert!(rule_matches(&rule("CSQ", false, "contains"), hay, "AT+CSQ\r\n", "utf-8").unwrap());
        assert!(rule_matches(&rule("AT+", false, "prefix"), hay, "AT+CSQ\r\n", "utf-8").unwrap());
        assert!(!rule_matches(&rule("AT+", false, "exact"), hay, "AT+CSQ\r\n", "utf-8").unwrap());
        assert!(rule_matches(
            &rule("AT\\+CSQ", false, "regex"),
            hay,
            "AT+CSQ\r\n",
            "utf-8"
        )
        .unwrap());
        assert!(rule_matches(&rule("41 54", true, "prefix"), hay, "AT+CSQ\r\n", "utf-8").unwrap());
    }

    #[test]
    fn rule_text_matching_uses_selected_encoding() {
        let bytes = encode_text("测试", "gbk");
        assert!(rule_matches(&rule("测试", false, "exact"), &bytes, "测试", "gbk").unwrap());
        // 同一段文本按 UTF-8 解释时不再命中
        assert!(
            !rule_matches(&rule("测试", false, "exact"), &bytes, "测试", "utf-8").unwrap()
        );
    }

    #[test]
    fn empty_pattern_never_matches_contains() {
        assert!(!bytes_contain(b"abc", b""));
    }
}
