//! SSH 会话生命周期：连接、认证、交互 shell、流式输入输出、断线清理。
//!
//! 明文密码只在内存中交给 russh，不落盘、不进日志、不进 command 返回值。
//! 每个会话是一个 tokio task，前端通过 Tauri `Channel` 收数据、通过命令写数据。
use super::host_keys::{self, encode_host_key_prompt, HostKeyDecision, HostKeyPrompt, TrustDecision};
use super::target::{classify_ssh_error, normalize_target, terminal_size, validate_key_auth, SshTarget};
use base64::{engine::general_purpose, Engine as _};
use russh::client::{self, Handle, Handler};
use russh::{ChannelMsg, ChannelReadHalf, ChannelWriteHalf, Disconnect, Error as SshLibError};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use tauri::ipc::Channel;
use tokio::sync::{mpsc, Mutex};

/// 推给前端的事件。data 为 base64 编码的原始字节（可能是任意二进制/多字节 UTF-8 片段）。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshOutput {
    pub session_id: String,
    pub kind: String,
    pub data: Option<String>,
    pub message: Option<String>,
}

enum SessionCommand {
    Input(Vec<u8>),
    Resize(u32, u32),
    Close,
}

struct SessionHandle {
    tx: mpsc::UnboundedSender<SessionCommand>,
}

fn registry() -> &'static Mutex<HashMap<String, SessionHandle>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, SessionHandle>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 主机密钥校验结果通过共享槽位回传给命令层，供前端弹「首次连接确认」。
pub(crate) type PendingHostKey = Arc<StdMutex<Option<HostKeyPrompt>>>;

/// 全局远程转发目标表：远端监听端口 -> (本地目标主机, 目标端口)。
/// 由 `forward` 模块注册/注销，Handler 的 forwarded-tcpip 回调按端口查询。
pub(crate) fn remote_targets() -> &'static StdMutex<HashMap<u32, (String, u16)>> {
    static TARGETS: OnceLock<StdMutex<HashMap<u32, (String, u16)>>> = OnceLock::new();
    TARGETS.get_or_init(|| StdMutex::new(HashMap::new()))
}

/// 阶段 3：连接上下文，校验服务器指纹；远程转发目标从全局表查询。
pub(crate) struct ClientHandler {
    pub host: String,
    pub port: u16,
    pub pending_host_key: PendingHostKey,
}

impl Handler for ClientHandler {
    type Error = SshLibError;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        use russh::keys::HashAlg;
        let fingerprint = format!(
            "{}",
            server_public_key.public_key().fingerprint(HashAlg::Sha256)
        );
        let decision = match host_keys::trust_key(&host_keys::load_store(), &self.host, self.port, &fingerprint) {
            TrustDecision::Trusted => return Ok(true),
            TrustDecision::Unknown => HostKeyDecision::Unknown,
            TrustDecision::Changed => HostKeyDecision::Changed,
        };
        if let Ok(mut slot) = self.pending_host_key.lock() {
            *slot = Some(HostKeyPrompt {
                host: self.host.clone(),
                port: self.port,
                fingerprint,
                decision,
            });
        }
        // 未信任/已变更一律拒绝，绝不静默放行（防中间人）。
        Ok(false)
    }

    /// 远程转发：服务端收到远端端口的连接后开一个 channel，这里桥接到本地目标。
    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: russh::Channel<client::Msg>,
        _connected_address: &str,
        connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: russh::client::ChannelOpenHandle,
        _session: &mut russh::client::Session,
    ) -> Result<(), Self::Error> {
        let target = remote_targets()
            .lock()
            .ok()
            .and_then(|m| m.get(&connected_port).cloned());
        let Some((host, port)) = target else {
            drop(reply); // 自动拒绝：没有登记过的转发一律不接受
            return Ok(());
        };
        reply.accept().await;
        tokio::spawn(async move {
            let mut stream = channel.into_stream();
            if let Ok(mut tcp) = tokio::net::TcpStream::connect((host.as_str(), port)).await {
                use tokio::io::copy_bidirectional;
                let _ = copy_bidirectional(&mut tcp, &mut stream).await;
            }
        });
        Ok(())
    }
}

fn output(session_id: &str, kind: &str, data: Option<String>, message: Option<String>) -> SshOutput {
    SshOutput {
        session_id: session_id.to_string(),
        kind: kind.to_string(),
        data,
        message,
    }
}

/// 返回 false 表示前端窗口已关闭，调用方必须结束会话，避免残留后台 SSH 连接。
async fn emit(channel: &Channel<SshOutput>, event: SshOutput) -> bool {
    channel.send(event).is_ok()
}

/// 建立会话。返回 session_id；连接失败时返回经分类的用户可读错误。
/// 建立 SSH 连接并完成密码认证。SFTP 与交互 shell 共用这一条认证链路。
/// 返回值不再持有凭据（SshTarget 在函数末尾离开作用域）。
pub async fn authenticate(target: SshTarget) -> Result<Handle<ClientHandler>, String> {
    let target = normalize_target(target)?;
    let pending: PendingHostKey = Arc::new(StdMutex::new(None));
    let handler = ClientHandler {
        host: target.host.clone(),
        port: target.port,
        pending_host_key: pending.clone(),
    };
    let config = Arc::new(client::Config::default());
    let mut handle: Handle<ClientHandler> = match client::connect(
        config,
        (target.host.as_str(), target.port),
        handler,
    )
    .await
    {
        Ok(h) => h,
        Err(e) => {
            // 指纹未信任/已变更时 russh 只会报泛化的 KEX 错误，用槽位还原成可操作提示。
            if let Some(p) = pending.lock().ok().and_then(|mut slot| slot.take()) {
                return Err(encode_host_key_prompt(&p.host, p.port, &p.fingerprint, p.decision));
            }
            return Err(classify_ssh_error(&e.to_string()).message);
        }
    };

    let auth = if let Some(key_auth) = target.key_auth.as_ref() {
        validate_key_auth(key_auth)?;
        // 私钥与口令只在内存中；错误文案不回显路径/口令内容。
        let key = russh::keys::load_secret_key(&key_auth.path, key_auth.passphrase.as_deref())
            .map_err(|_| "读取私钥失败：路径不存在、格式不支持或口令不正确".to_string())?;
        let hash_alg = handle
            .best_supported_rsa_hash()
            .await
            .ok()
            .and_then(|v| v.flatten());
        let private = russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key), hash_alg);
        handle
            .authenticate_publickey(&target.username, private)
            .await
            .map_err(|e| classify_ssh_error(&e.to_string()).message)?
    } else {
        handle
            .authenticate_password(&target.username, &target.password)
            .await
            .map_err(|e| classify_ssh_error(&e.to_string()).message)?
    };
    if !auth.success() {
        let _ = handle.disconnect(Disconnect::ByApplication, "", "English").await;
        return Err(classify_ssh_error("authentication failed").message);
    }
    Ok(handle)
}

/// 建立连接、密码认证并打开交互 shell。
/// 与 Tauri `Channel`/注册表解耦，便于在不启动 WebView 的情况下做真机连通验证。
pub async fn establish(
    target: SshTarget,
    cols: u32,
    rows: u32,
) -> Result<
    (
        Handle<ClientHandler>,
        ChannelReadHalf,
        ChannelWriteHalf<client::Msg>,
    ),
    String,
> {
    let target = normalize_target(target)?;
    let (cols, rows) = terminal_size(cols, rows);
    let handle = authenticate(target).await?;

    let ssh_channel = handle
        .channel_open_session()
        .await
        .map_err(|e| classify_ssh_error(&e.to_string()).message)?;
    ssh_channel
        .request_pty(false, "xterm-256color", cols, rows, 0, 0, &[])
        .await
        .map_err(|e| classify_ssh_error(&e.to_string()).message)?;
    ssh_channel
        .request_shell(true)
        .await
        .map_err(|e| classify_ssh_error(&e.to_string()).message)?;
    let (read_half, write_half) = ssh_channel.split();
    Ok((handle, read_half, write_half))
}

#[allow(clippy::too_many_arguments)]
pub async fn connect(
    session_id: String,
    target: SshTarget,
    cols: u32,
    rows: u32,
    channel: Channel<SshOutput>,
) -> Result<(), String> {
    {
        let mut reg = registry().lock().await;
        if reg.contains_key(&session_id) {
            return Err(format!("会话已存在: {session_id}"));
        }
    }
    let sid = session_id.clone();
    let (handle, read_half, write_half) = establish(target, cols, rows).await?;

    let (tx, rx) = mpsc::unbounded_channel::<SessionCommand>();
    registry().lock().await.insert(sid.clone(), SessionHandle { tx });
    tokio::spawn(async move {
        run_session(sid.clone(), handle, read_half, write_half, rx, channel).await;
        registry().lock().await.remove(&sid);
    });
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_session(
    session_id: String,
    handle: Handle<ClientHandler>,
    mut read: ChannelReadHalf,
    write: ChannelWriteHalf<client::Msg>,
    mut commands: mpsc::UnboundedReceiver<SessionCommand>,
    channel: Channel<SshOutput>,
) {
    if !emit(&channel, output(&session_id, "open", None, None)).await {
        let _ = handle.disconnect(Disconnect::ByApplication, "", "English").await;
        return;
    }
    loop {
        tokio::select! {
            msg = read.wait() => {
                let encoded = match msg {
                    Some(ChannelMsg::Data { data }) => Some(general_purpose::STANDARD.encode(&data)),
                    Some(ChannelMsg::ExtendedData { data, .. }) => Some(general_purpose::STANDARD.encode(&data)),
                    Some(ChannelMsg::ExitStatus { .. }) | Some(ChannelMsg::Eof)
                    | Some(ChannelMsg::Close) | None => break,
                    _ => continue,
                };
                if let Some(data) = encoded {
                    if !emit(&channel, output(&session_id, "data", Some(data), None)).await { break; }
                }
            }
            cmd = commands.recv() => {
                match cmd {
                    Some(SessionCommand::Input(bytes)) => {
                        if write.data_bytes(bytes).await.is_err() { break; }
                    }
                    Some(SessionCommand::Resize(cols, rows)) => {
                        let (c, r) = terminal_size(cols, rows);
                        if write.window_change(c, r, 0, 0).await.is_err() { break; }
                    }
                    Some(SessionCommand::Close) | None => break,
                }
            }
        }
    }
    let _ = handle.disconnect(Disconnect::ByApplication, "", "English").await;
    let _ = emit(&channel, output(&session_id, "closed", None, None)).await;
}

/// 前端键盘输入。
pub async fn write(session_id: &str, bytes: Vec<u8>) -> Result<(), String> {
    let tx = registry()
        .lock()
        .await
        .get(session_id)
        .map(|h| h.tx.clone())
        .ok_or_else(|| "会话不存在或已断开".to_string())?;
    tx.send(SessionCommand::Input(bytes))
        .map_err(|_| "会话已关闭".to_string())
}

/// 终端窗口尺寸变化。
pub async fn resize(session_id: &str, cols: u32, rows: u32) -> Result<(), String> {
    let tx = registry()
        .lock()
        .await
        .get(session_id)
        .map(|h| h.tx.clone())
        .ok_or_else(|| "会话不存在或已断开".to_string())?;
    tx.send(SessionCommand::Resize(cols, rows))
        .map_err(|_| "会话已关闭".to_string())
}

/// 主动断开；幂等。
pub async fn disconnect(session_id: &str) {
    if let Some(handle) = registry().lock().await.remove(session_id) {
        let _ = handle.tx.send(SessionCommand::Close);
    }
}

/// 应用退出时清理全部会话，避免残留后台连接。
pub async fn disconnect_all() {
    let mut reg = registry().lock().await;
    for (_, handle) in reg.drain() {
        let _ = handle.tx.send(SessionCommand::Close);
    }
}
