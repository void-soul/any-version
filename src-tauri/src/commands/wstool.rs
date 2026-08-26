// ─── WebSocket / SSE 调试器模块 ───
// WS 基于 tokio-tungstenite；SSE 基于既有 reqwest 的流式响应。
// 收到的数据通过 `wstool://message` / `sstool://event` 等事件推给前端。

use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::{http::HeaderValue, Message};
use tracing::{info, warn};

#[derive(Serialize)]
pub struct ConnInfo {
    pub id: String,
    pub target: String,
}

#[derive(Default)]
pub struct WsState {
    /// 活跃会话：出站消息通道 + 取消句柄
    sessions: Mutex<HashMap<String, Session>>,
    /// 原始 TCP/UDP 连接
    raw: Mutex<HashMap<String, RawConn>>,
}

/// 原始 TCP/UDP 会话。
struct RawConn {
    #[allow(dead_code)] // 保留对端地址供后续扩展（如 UDP 重定向目标）
    udp_target: Option<SocketAddr>,
    tx: mpsc::UnboundedSender<Vec<u8>>,
    handles: Vec<tokio::task::JoinHandle<()>>,
}

struct Session {
    target: String,
    tx: Option<mpsc::UnboundedSender<WsOut>>,
    handles: Vec<tokio::task::JoinHandle<()>>,
}

enum WsOut {
    Text(String),
    Binary(Vec<u8>),
    Close,
}

fn insert_session(state: &WsState, id: &str, target: String, tx: mpsc::UnboundedSender<WsOut>, handle: tokio::task::JoinHandle<()>) {
    state
        .sessions
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id.to_string(), Session { target, tx: Some(tx), handles: vec![handle] });
}

/// 列出当前活跃连接。
#[tauri::command]
pub fn wstool_list(state: tauri::State<'_, WsState>) -> Vec<ConnInfo> {
    state
        .sessions
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .map(|(id, s)| ConnInfo { id: id.clone(), target: s.target.clone() })
        .collect()
}

/// 建立 WebSocket 连接（支持 ws:// 与 wss://）。
/// 连接成功后返回；后续消息经 `wstool://message` 推送，关闭经 `wstool://closed`。
#[tauri::command]
pub async fn ws_connect(
    app: AppHandle,
    state: tauri::State<'_, WsState>,
    id: String,
    url: String,
    headers: Option<Vec<Vec<String>>>,
) -> Result<(), String> {
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|e| format!("URL 无效（应为 ws:// 或 wss://）: {}", e))?;
    if let Some(hdrs) = headers {
        for pair in hdrs {
            if pair.len() == 2 && !pair[0].is_empty() {
                if let (Ok(name), Ok(value)) = (
                    pair[0].parse::<tokio_tungstenite::tungstenite::http::HeaderName>(),
                    HeaderValue::from_str(&pair[1]),
                ) {
                    request.headers_mut().insert(name, value);
                }
            }
        }
    }

    let (ws_stream, _resp) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|e| format!("WebSocket 连接失败: {}", e))?;
    info!("[wstool] {} 已连接 {}", id, url);
    let _ = app.emit("wstool://open", serde_json::json!({ "id": id, "url": url }));

    let (mut sink, mut stream) = ws_stream.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<WsOut>();

    // 写任务
    let app_writer = app.clone();
    let id_writer = id.clone();
    let writer_task = tauri::async_runtime::spawn(async move {
        while let Some(out) = rx.recv().await {
            let (msg, is_close) = match out {
                WsOut::Text(t) => (Message::Text(t), false),
                WsOut::Binary(b) => (Message::Binary(b), false),
                WsOut::Close => (Message::Close(None), true),
            };
            if sink.send(msg).await.is_err() {
                break;
            }
            if is_close {
                break;
            }
        }
        let _ = app_writer.emit("wstool://closed", serde_json::json!({ "id": id_writer }));
    });

    // 读任务：收到消息即推送事件
    let app_reader = app;
    let id_reader = id.clone();
    let reader_task = tauri::async_runtime::spawn(async move {
        while let Some(item) = stream.next().await {
            match item {
                Ok(Message::Text(text)) => {
                    let _ = app_reader.emit("wstool://message", serde_json::json!({ "id": id_reader, "kind": "text", "data": text }));
                }
                Ok(Message::Binary(bytes)) => {
                    let hex: String = bytes.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
                    let _ = app_reader.emit(
                        "wstool://message",
                        serde_json::json!({ "id": id_reader, "kind": "binary", "hex": hex, "text": String::from_utf8_lossy(&bytes), "len": bytes.len() }),
                    );
                }
                Ok(Message::Ping(_)) | Ok(Message::Pong(_)) | Ok(_) => {}
                Err(e) => {
                    warn!("[wstool] {} 读错误: {}", id_reader, e);
                    break;
                }
            }
        }
        let _ = app_reader.emit("wstool://closed", serde_json::json!({ "id": id_reader }));
    });

    // 合并两个任务的句柄：断开时一并 abort
    let joined = tokio::spawn(async move {
        let _ = tokio::join!(writer_task, reader_task);
    });
    insert_session(&state, &id, url, tx, joined);
    Ok(())
}

/// 通过 WebSocket 发送数据。
#[tauri::command]
pub fn ws_send(state: tauri::State<'_, WsState>, id: String, data: String, hex_mode: bool) -> Result<(), String> {
    let map = state.sessions.lock().unwrap_or_else(|e| e.into_inner());
    let session = map.get(&id).ok_or_else(|| format!("{} 未连接", id))?;
    let out = if hex_mode {
        let clean: String = data.chars().filter(|c| !c.is_whitespace() && *c != ',').collect();
        let bytes = (0..clean.len() / 2)
            .map(|i| u8::from_str_radix(&clean[i * 2..i * 2 + 2], 16))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("HEX 解析失败: {}", e))?;
        WsOut::Binary(bytes)
    } else {
        WsOut::Text(data)
    };
    session.tx.as_ref().ok_or("通道已关闭")?.send(out).map_err(|_| "发送失败：连接已关闭".to_string())
}

/// 断开指定连接（WS / SSE / TCP / UDP 共用注册表）。
/// 核心逻辑与运行时无关，命令包装与测试共用。
pub fn wstool_disconnect_core(state: &WsState, id: String) -> Result<(), String> {
    if let Some(session) = state.sessions.lock().unwrap_or_else(|e| e.into_inner()).remove(&id) {
        if let Some(tx) = &session.tx {
            let _ = tx.send(WsOut::Close);
        }
        for h in session.handles {
            h.abort();
        }
    }
    // 原始连接：drop 通道 + abort 任务即可关闭
    if let Some(conn) = state.raw.lock().unwrap_or_else(|e| e.into_inner()).remove(&id) {
        drop(conn.tx);
        for h in conn.handles {
            h.abort();
        }
    }
    Ok(())
}

/// 断开指定连接（命令入口）。
#[tauri::command]
pub fn wstool_disconnect(
    state: tauri::State<'_, WsState>,
    id: String,
) -> Result<(), String> {
    wstool_disconnect_core(state.inner(), id)
}

fn parse_hex_bytes(data: &str) -> Result<Vec<u8>, String> {
    let clean: String = data.chars().filter(|c| !c.is_whitespace() && *c != ',').collect();
    (0..clean.len() / 2)
        .map(|i| u8::from_str_radix(&clean[i * 2..i * 2 + 2], 16))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("HEX 解析失败: {}", e))
}

/// 事件发射器抽象：核心逻辑不依赖 Tauri 运行时，测试可注入普通通道。
pub type EmitFn = Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>;

/// 把 AppHandle 包装成发射器（命令入口使用）。
fn app_emitter<R: tauri::Runtime>(app: &AppHandle<R>) -> EmitFn {
    let app = app.clone();
    Arc::new(move |event, payload| {
        let _ = app.emit(event, payload);
    })
}

/// 建立 TCP/UDP 连接。数据经 `wstool://message` 推送，关闭经 `wstool://closed`。
/// 核心逻辑与运行时无关（通过注入的 emit 回调推送事件），测试与命令包装共用。
pub async fn net_connect_core(
    emit: EmitFn,
    state: &WsState,
    id: String,
    protocol: String,
    host: String,
    port: u16,
) -> Result<(), String> {
    match protocol.as_str() {
        "tcp" => {
            let stream = TcpStream::connect((host.as_str(), port))
                .await
                .map_err(|e| format!("TCP 连接失败: {}", e))?;
            info!("[nettool] {} 已连接 tcp://{}:{}", id, host, port);
            emit("wstool://open", serde_json::json!({ "id": id, "url": format!("tcp://{}:{}", host, port), "proto": "tcp" }));
            let (mut rd, mut wr) = stream.into_split();
            let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();

            let emit_w = emit.clone();
            let id_w = id.clone();
            let writer = tokio::spawn(async move {
                while let Some(data) = rx.recv().await {
                    if wr.write_all(&data).await.is_err() {
                        break;
                    }
                    let _ = wr.flush().await;
                }
                emit_w("wstool://closed", serde_json::json!({ "id": id_w }));
            });

            let emit_r = emit.clone();
            let id_r = id.clone();
            let reader = tokio::spawn(async move {
                let mut buf = [0u8; 8192];
                loop {
                    match rd.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let hex: String = buf[..n].iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
                            emit_r(
                                "wstool://message",
                                serde_json::json!({ "id": id_r, "kind": "text", "proto": "tcp", "hex": hex, "data": String::from_utf8_lossy(&buf[..n]), "len": n }),
                            );
                        }
                    }
                }
                emit_r("wstool://closed", serde_json::json!({ "id": id_r }));
            });

            state.raw.lock().unwrap_or_else(|e| e.into_inner()).insert(
                id.clone(),
                RawConn { udp_target: None, tx, handles: vec![writer, reader] },
            );
            Ok(())
        }
        "udp" => {
            let sock = UdpSocket::bind("0.0.0.0:0")
                .await
                .map_err(|e| format!("UDP 绑定失败: {}", e))?;
            sock.connect((host.as_str(), port))
                .await
                .map_err(|e| format!("UDP 连接失败: {}", e))?;
            let peer: SocketAddr = sock.peer_addr().map_err(|e| e.to_string())?;
            info!("[nettool] {} 已绑定 udp → {}", id, peer);
            emit("wstool://open", serde_json::json!({ "id": id, "url": format!("udp://{}:{}", host, port), "proto": "udp" }));
            let sock = Arc::new(sock);
            let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();

            let sock_w = sock.clone();
            let emit_w = emit.clone();
            let id_w = id.clone();
            let writer = tokio::spawn(async move {
                while let Some(data) = rx.recv().await {
                    if sock_w.send(&data).await.is_err() {
                        break;
                    }
                }
                emit_w("wstool://closed", serde_json::json!({ "id": id_w }));
            });

            let emit_r = emit.clone();
            let id_r = id.clone();
            let reader = tokio::spawn(async move {
                let mut buf = [0u8; 65535];
                loop {
                    match sock.recv_from(&mut buf).await {
                        Ok((0, _)) | Err(_) => break,
                        Ok((n, from)) => {
                            let hex: String = buf[..n].iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
                            emit_r(
                                "wstool://message",
                                serde_json::json!({ "id": id_r, "kind": "text", "proto": "udp", "from": from.to_string(), "hex": hex, "data": String::from_utf8_lossy(&buf[..n]), "len": n }),
                            );
                        }
                    }
                }
                emit_r("wstool://closed", serde_json::json!({ "id": id_r }));
            });

            state.raw.lock().unwrap_or_else(|e| e.into_inner()).insert(
                id,
                RawConn { udp_target: Some(peer), tx, handles: vec![writer, reader] },
            );
            Ok(())
        }
        other => Err(format!("不支持的协议: {}", other)),
    }
}

/// 通过 TCP/UDP 发送数据（hex_mode=true 时按十六进制解析）。
/// 核心逻辑与运行时无关，测试与命令包装共用。
pub fn net_send_core(state: &WsState, id: String, data: String, hex_mode: bool) -> Result<(), String> {
    let map = state.raw.lock().unwrap_or_else(|e| e.into_inner());
    let conn = map.get(&id).ok_or_else(|| format!("{} 未连接", id))?;
    let bytes = if hex_mode {
        parse_hex_bytes(&data)?
    } else {
        data.into_bytes()
    };
    conn.tx.send(bytes).map_err(|_| "发送失败：连接已关闭".to_string())
}

/// 建立 TCP/UDP 连接（命令入口）。
#[tauri::command]
pub async fn net_connect(
    app: AppHandle,
    state: tauri::State<'_, WsState>,
    id: String,
    protocol: String,
    host: String,
    port: u16,
) -> Result<(), String> {
    net_connect_core(app_emitter(&app), state.inner(), id, protocol, host, port).await
}

/// 通过 TCP/UDP 发送数据（命令入口）。
#[tauri::command]
pub fn net_send(
    state: tauri::State<'_, WsState>,
    id: String,
    data: String,
    hex_mode: bool,
) -> Result<(), String> {
    net_send_core(state.inner(), id, data, hex_mode)
}

/// 订阅 SSE。事件经 `sstool://open` / `sstool://event` / `sstool://closed` 推送。
#[tauri::command]
pub async fn sse_connect(
    app: AppHandle,
    state: tauri::State<'_, WsState>,
    id: String,
    url: String,
    headers: Option<Vec<Vec<String>>>,
) -> Result<(), String> {
    // SSE 需要禁用整体超时，仅限制连接超时
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;

    let mut req = client.get(&url).header("Accept", "text/event-stream").header("Cache-Control", "no-cache");
    if let Some(hdrs) = headers {
        for pair in hdrs {
            if pair.len() == 2 && !pair[0].is_empty() {
                req = req.header(pair[0].as_str(), pair[1].as_str());
            }
        }
    }

    let resp = req.send().await.map_err(|e| format!("请求失败: {}", e))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("服务端返回 {}", status));
    }

    let _ = app.emit("sstool://open", serde_json::json!({ "id": id, "url": url }));

    let mut byte_stream = resp.bytes_stream();
    let app_task = app;
    let id_task = id.clone();
    let task = tokio::spawn(async move {
        let mut buffer = String::new();
        let mut decoder_buf: Vec<u8> = Vec::new();
        while let Some(chunk) = byte_stream.next().await {
            match chunk {
                Ok(bytes) => {
                    decoder_buf.extend_from_slice(&bytes);
                    // 按 \n 分割处理完整行
                    while let Some(pos) = decoder_buf.iter().position(|&b| b == b'\n') {
                        let line_bytes: Vec<u8> = decoder_buf.drain(..=pos).collect();
                        let mut line = String::from_utf8_lossy(&line_bytes[..line_bytes.len().saturating_sub(1)]).to_string();
                        if line.ends_with('\r') {
                            line.pop();
                        }
                        buffer.push_str(&line);
                        buffer.push('\n');
                    }
                    // 事件以空行结束
                    while let Some(end) = find_event_end(&buffer) {
                        let event_block = buffer[..end].to_string();
                        buffer.drain(..end);
                        if event_block.trim().is_empty() {
                            continue;
                        }
                        let _ = app_task.emit("sstool://event", serde_json::json!({ "id": id_task, "raw": event_block }));
                    }
                }
                Err(e) => {
                    warn!("[sstool] {} 流错误: {}", id_task, e);
                    break;
                }
            }
        }
        let _ = app_task.emit("sstool://closed", serde_json::json!({ "id": id_task }));
    });

    // SSE 无发送通道，tx 用占位（None）
    state
        .sessions
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id, Session { target: url, tx: None, handles: vec![task] });
    Ok(())
}

fn find_event_end(buffer: &str) -> Option<usize> {
    // 找到 "\n\n"（含行尾空行）位置，返回其结束索引
    let bytes = buffer.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'\n' && bytes[i + 1] == b'\n' {
            // 跳过连续空行
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] == b'\n' {
                j += 1;
            }
            return Some(j);
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod net_echo_tests {
    //! 网络调试器 TCP/UDP 收发链路端到端验证：
    //! 本地回显服务器 + 注入 emit 回调驱动与命令同一份核心逻辑（net_connect_core /
    //! net_send_core / wstool_disconnect_core），通过回调收集的消息断言回环数据。
    //! 不依赖 Tauri 运行时（mock_app 在 Windows 测试二进制上无法启动）。

    use super::*;
    use std::sync::mpsc as std_mpsc;

    /// tokio TCP 回显服务器：读到什么就写回什么。
    async fn tcp_echo(listener: tokio::net::TcpListener) {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { break };
            tokio::spawn(async move {
                let mut buf = [0u8; 8192];
                loop {
                    match sock.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if sock.write_all(&buf[..n]).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            });
        }
    }

    /// UDP 回显：收到数据报原样发回来源地址。
    async fn udp_echo(sock: Arc<UdpSocket>) {
        let mut buf = [0u8; 65535];
        loop {
            match sock.recv_from(&mut buf).await {
                Ok((0, _)) | Err(_) => break,
                Ok((n, from)) => {
                    let _ = sock.send_to(&buf[..n], from).await;
                }
            }
        }
    }

    /// 建立 WsState + emit 回调收集器：收到 wstool://message 事件即把 data 字段送入通道。
    fn setup() -> (WsState, EmitFn, std_mpsc::Receiver<String>) {
        let state = WsState::default();
        let (tx, rx) = std_mpsc::channel::<String>();
        let emit: EmitFn = Arc::new(move |event, payload| {
            if event == "wstool://message" {
                if let Some(data) = payload.get("data").and_then(|d| d.as_str()) {
                    let _ = tx.send(data.to_string());
                }
            }
        });
        (state, emit, rx)
    }

    fn wait_for(rx: &std_mpsc::Receiver<String>, timeout_ms: u64) -> String {
        rx.recv_timeout(std::time::Duration::from_millis(timeout_ms))
            .expect("等待回显数据超时")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tcp_roundtrip_text_and_hex() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(tcp_echo(listener));

        let (state, emit, rx) = setup();

        net_connect_core(emit.clone(), &state, "t1".into(), "tcp".into(), "127.0.0.1".into(), port)
            .await
            .expect("TCP 连接失败");

        // 文本收发
        net_send_core(&state, "t1".into(), "hello-tcp-文本".into(), false).expect("TCP 发送失败");
        assert_eq!(wait_for(&rx, 3000), "hello-tcp-文本", "TCP 文本回显不一致");

        // HEX 收发：01 02 FF → 服务端原样返回 → data 为替换字符展示，但 hex 应一致
        net_send_core(&state, "t1".into(), "01 02 FF".into(), true).expect("HEX 发送失败");
        let echoed = wait_for(&rx, 3000);
        // 与读取端一致：data 字段经 from_utf8_lossy，无效字节映射为 U+FFFD
        let expected = String::from_utf8_lossy(&[0x01u8, 0x02, 0xff]).into_owned();
        assert_eq!(echoed, expected, "TCP HEX 回显字节不一致");

        wstool_disconnect_core(&state, "t1".into()).unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn udp_roundtrip_text_and_hex() {
        let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let port = sock.local_addr().unwrap().port();
        tokio::spawn(udp_echo(sock));

        let (state, emit, rx) = setup();

        net_connect_core(emit.clone(), &state, "u1".into(), "udp".into(), "127.0.0.1".into(), port)
            .await
            .expect("UDP 连接失败");

        net_send_core(&state, "u1".into(), "hello-udp".into(), false).expect("UDP 发送失败");
        assert_eq!(wait_for(&rx, 3000), "hello-udp", "UDP 文本回显不一致");

        net_send_core(&state, "u1".into(), "AA BB".into(), true).expect("UDP HEX 发送失败");
        let echoed = wait_for(&rx, 3000);
        let expected = String::from_utf8_lossy(&[0xaau8, 0xbb]).into_owned();
        assert_eq!(echoed, expected, "UDP HEX 回显字节不一致");

        wstool_disconnect_core(&state, "u1".into()).unwrap();
    }
}
