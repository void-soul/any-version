//! 端口转发运行时：每条规则一条独立 SSH 连接，用取消令牌控制生命周期。
//!
//! - Local：本地监听 → `direct-tcpip` 到远端目标
//! - Dynamic：本地 SOCKS5（无认证 CONNECT）→ 每次连接一个 `direct-tcpip`
//! - Remote：`tcpip_forward` 注册远端监听，进来的 channel 由 Handler 桥接到本地目标
use super::forward::{parse_socks5_target, validate_rule, ForwardKind, ForwardRule};
use super::session::{authenticate, remote_targets, ClientHandler};
use super::target::SshTarget;
use russh::client::Handle;
use russh::Disconnect;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

/// 带归属连接的规则，供前端按连接过滤。
#[derive(Clone)]
pub struct OwnedRule {
    pub connection_id: String,
    pub rule: ForwardRule,
}

/// 只返回属于该连接的规则，保持注册顺序。
pub fn filter_for(owned: &[OwnedRule], connection_id: &str) -> Vec<ForwardRule> {
    owned
        .iter()
        .filter(|o| o.connection_id == connection_id)
        .map(|o| o.rule.clone())
        .collect()
}

/// 用 watch 的 `true` 作为取消信号，避免为此引入 tokio-util。
struct Active {
    cancel: watch::Sender<bool>,
    rule: ForwardRule,
    /// 转发归属哪条连接；列表查询按此过滤，避免不同连接互相看到对方的规则。
    connection_id: String,
}

fn registry() -> &'static Mutex<HashMap<String, Arc<Active>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Arc<Active>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 启动一条转发。返回实际生效的端口（Remote 下 port=0 时由服务端分配）。
pub async fn start(
    id: String,
    connection_id: String,
    rule: ForwardRule,
    target: SshTarget,
) -> Result<u16, String> {
    validate_rule(&rule)?;
    {
        let mut reg = registry().lock().map_err(|_| "转发注册表不可用".to_string())?;
        if reg.contains_key(&id) {
            return Err(format!("转发已存在: {id}"));
        }
    }
    let mut handle: Handle<ClientHandler> = authenticate(target).await?;
    let (cancel, cancelled) = watch::channel(false);
    let kind = rule.kind;

    let effective_port = match kind {
        ForwardKind::Local | ForwardKind::Dynamic => {
            let listener = TcpListener::bind((rule.bind_host.as_str(), rule.bind_port))
                .await
                .map_err(|e| format!("监听 {}:{} 失败: {e}", rule.bind_host, rule.bind_port))?;
            let port = listener.local_addr().map(|a| a.port()).unwrap_or(rule.bind_port);
            let mut rx = cancelled.clone();
            let r = rule.clone();
            let h = Arc::new(tokio::sync::Mutex::new(handle));
            tokio::spawn(async move {
                accept_loop(listener, Arc::clone(&h), r, &mut rx).await;
                // 即便还有活跃连接持有 Arc，也显式断开底层 SSH。
                let _ = h
                    .lock()
                    .await
                    .disconnect(Disconnect::ByApplication, "", "English")
                    .await;
            });
            return register(id, connection_id, rule, cancel, port);
        }
        ForwardKind::Remote => {
            let returned = handle
                .tcpip_forward(&rule.bind_host, u32::from(rule.bind_port))
                .await
                .map_err(|e| format!("远端转发注册失败: {e}"))?;
            // 服务端对固定端口请求返回 0（沿用请求端口），不能直接当实际端口用。
            let assigned = super::forward::resolve_assigned_port(rule.bind_port, returned);
            remote_targets()
                .lock()
                .map_err(|_| "转发目标表不可用".to_string())?
                .insert(u32::from(assigned), (rule.target_host.clone(), rule.target_port));
            assigned
        }
    };

    // Remote：保持 handle 存活，直到取消；取消时撤销远端监听并断开。
    let mut rx = cancelled.clone();
    let bind_host = rule.bind_host.clone();
    tokio::spawn(async move {
        let _ = rx.changed().await;
        let _ = handle.cancel_tcpip_forward(&bind_host, u32::from(effective_port)).await;
        let _ = handle.disconnect(Disconnect::ByApplication, "", "English").await;
        if let Ok(mut m) = remote_targets().lock() {
            m.remove(&u32::from(effective_port));
        }
    });
    register(id, connection_id, rule.clone(), cancel, effective_port)
}

fn register(
    id: String,
    connection_id: String,
    rule: ForwardRule,
    cancel: watch::Sender<bool>,
    port: u16,
) -> Result<u16, String> {
    registry()
        .lock()
        .map_err(|_| "转发注册表不可用".to_string())?
        .insert(id, Arc::new(Active { cancel, rule, connection_id }));
    Ok(port)
}

async fn accept_loop(
    listener: TcpListener,
    handle: Arc<tokio::sync::Mutex<Handle<ClientHandler>>>,
    rule: ForwardRule,
    cancelled: &mut watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            _ = cancelled.changed() => break,
            accepted = listener.accept() => {
                let Ok((mut tcp, _)) = accepted else { break };
                let h = Arc::clone(&handle);
                let r = rule.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_one(&mut tcp, h, &r).await {
                        tracing::debug!("forward connection ended: {e}");
                    }
                });
            }
        }
    }
}

async fn handle_one(
    tcp: &mut TcpStream,
    handle: Arc<tokio::sync::Mutex<Handle<ClientHandler>>>,
    rule: &ForwardRule,
) -> Result<(), String> {
    let (host, port) = match rule.kind {
        ForwardKind::Dynamic => socks5_handshake(tcp).await?,
        _ => (rule.target_host.clone(), rule.target_port),
    };
    let channel = {
        let h = handle.lock().await;
        h.channel_open_direct_tcpip(&host, u32::from(port), "127.0.0.1", 0)
            .await
            .map_err(|e| format!("打开转发通道失败: {e}"))?
    };
    let mut stream = channel.into_stream();
    tokio::io::copy_bidirectional(tcp, &mut stream)
        .await
        .map_err(|e| format!("转发数据中断: {e}"))?;
    Ok(())
}

/// SOCKS5 无认证握手：读方法协商 → 回 \x05\x00 → 读 CONNECT 请求 → 回成功。
async fn socks5_handshake(tcp: &mut TcpStream) -> Result<(String, u16), String> {
    let mut head = [0u8; 2];
    tcp.read_exact(&mut head)
        .await
        .map_err(|_| "SOCKS5 协商头读取失败".to_string())?;
    if head[0] != 5 {
        return Err("不是 SOCKS5 客户端".to_string());
    }
    let mut methods = vec![0u8; head[1] as usize];
    tcp.read_exact(&mut methods)
        .await
        .map_err(|_| "SOCKS5 方法列表读取失败".to_string())?;
    tcp.write_all(&[5, 0])
        .await
        .map_err(|_| "SOCKS5 方法回复失败".to_string())?;

    let mut req = [0u8; 4];
    tcp.read_exact(&mut req)
        .await
        .map_err(|_| "SOCKS5 请求头读取失败".to_string())?;
    if req[0] != 5 || req[1] != 1 {
        return Err("仅支持 SOCKS5 CONNECT".to_string());
    }
    // 拼回 `ATYP + 地址 + 端口`，交给纯函数统一解析，避免每种地址类型各写一套。
    let mut blob = vec![req[3]];
    match req[3] {
        1 | 4 => {
            let len = if req[3] == 1 { 4 } else { 16 };
            let mut addr = vec![0u8; len + 2];
            tcp.read_exact(&mut addr)
                .await
                .map_err(|_| "SOCKS5 目标地址读取失败".to_string())?;
            blob.extend_from_slice(&addr);
        }
        3 => {
            let mut n = [0u8; 1];
            tcp.read_exact(&mut n)
                .await
                .map_err(|_| "SOCKS5 域名长度读取失败".to_string())?;
            blob.push(n[0]);
            let mut rest = vec![0u8; n[0] as usize + 2];
            tcp.read_exact(&mut rest)
                .await
                .map_err(|_| "SOCKS5 域名读取失败".to_string())?;
            blob.extend_from_slice(&rest);
        }
        _ => return Err("SOCKS5 地址类型不支持".to_string()),
    }
    let target = parse_socks5_target(&blob).ok_or_else(|| "SOCKS5 目标地址无效".to_string())?;
    tcp.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
        .await
        .map_err(|_| "SOCKS5 成功回复失败".to_string())?;
    Ok(target)
}

pub async fn stop(id: &str) {
    if let Some(active) = registry().lock().ok().and_then(|mut m| m.remove(id)) {
        let _ = active.cancel.send(true);
    }
}

/// 某条连接名下的转发规则（用于面板展示，避免跨连接串台）。
pub fn list(connection_id: &str) -> Vec<ForwardRule> {
    let owned: Vec<OwnedRule> = registry()
        .lock()
        .map(|reg| {
            reg.values()
                .map(|a| OwnedRule {
                    connection_id: a.connection_id.clone(),
                    rule: a.rule.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    filter_for(&owned, connection_id)
}

pub async fn stop_all() {
    let items: Vec<Arc<Active>> = registry()
        .lock()
        .ok()
        .map(|mut m| m.drain().map(|(_, v)| v).collect())
        .unwrap_or_default();
    for item in items {
        let _ = item.cancel.send(true);
    }
}

