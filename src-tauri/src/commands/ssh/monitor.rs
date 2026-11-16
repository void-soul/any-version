//! 服务器状态监控：每条连接一条独立 SSH 会话，按需 exec 一次采样命令。
//!
//! 与交互 shell 分离，避免把采样命令打进用户终端；与 SFTP 分离，
//! 因为 SFTP 走的是 subsystem，而这里需要 exec channel。
use super::session::{authenticate, ClientHandler};
use super::sysinfo::{parse_sample, sample_command, Baseline, SysSnapshot};
use super::target::SshTarget;
use russh::client::Handle;
use russh::{ChannelMsg, Disconnect};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;

pub struct MonitorState {
    handle: Mutex<Handle<ClientHandler>>,
    /// 上一帧基准；首帧为 None（速率不可算）。
    baseline: Mutex<Option<Baseline>>,
}

fn registry() -> &'static Mutex<HashMap<String, Arc<MonitorState>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Arc<MonitorState>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

async fn state(session_id: &str) -> Result<Arc<MonitorState>, String> {
    registry()
        .lock()
        .await
        .get(session_id)
        .cloned()
        .ok_or_else(|| "监控会话不存在或已断开".to_string())
}

pub async fn open(session_id: String, target: SshTarget) -> Result<(), String> {
    if registry().lock().await.contains_key(&session_id) {
        return Err(format!("监控会话已存在: {session_id}"));
    }
    let handle = authenticate(target).await?;
    registry().lock().await.insert(
        session_id,
        Arc::new(MonitorState {
            handle: Mutex::new(handle),
            baseline: Mutex::new(None),
        }),
    );
    Ok(())
}

async fn exec_capture(handle: &Handle<ClientHandler>, cmd: &str) -> Result<String, String> {
    let mut ch = handle
        .channel_open_session()
        .await
        .map_err(|e| format!("打开采样通道失败: {e}"))?;
    ch.exec(true, cmd)
        .await
        .map_err(|e| format!("执行采样命令失败: {e}"))?;
    let mut out = Vec::new();
    while let Some(msg) = ch.wait().await {
        match msg {
            ChannelMsg::Data { data } => out.extend_from_slice(&data),
            ChannelMsg::ExtendedData { .. } => {}
            ChannelMsg::ExitStatus { .. } | ChannelMsg::Eof | ChannelMsg::Close => break,
            _ => {}
        }
    }
    String::from_utf8(out).map_err(|_| "采样输出不是 UTF-8".to_string())
}

/// 采一帧。首帧只给绝对值，速率字段为 None；之后按与上一帧的差值算速率。
pub async fn sample(session_id: &str) -> Result<SysSnapshot, String> {
    let st = state(session_id).await?;
    let raw = {
        let handle = st.handle.lock().await;
        exec_capture(&handle, sample_command()).await?
    };
    let prev = *st.baseline.lock().await;
    let (snapshot, next) =
        parse_sample(&raw, prev.as_ref()).ok_or_else(|| "无法解析服务器状态输出".to_string())?;
    *st.baseline.lock().await = Some(next);
    Ok(snapshot)
}

/// 在监控会话上执行一条只读命令（进程 / 网络面板按需调用）。
pub async fn exec(session_id: &str, command: &str) -> Result<String, String> {
    let state = state(session_id).await?;
    let handle = state.handle.lock().await;
    exec_capture(&handle, command).await
}

pub async fn close(session_id: &str) {
    if let Some(st) = registry().lock().await.remove(session_id) {
        let _ = st
            .handle
            .lock()
            .await
            .disconnect(Disconnect::ByApplication, "", "English")
            .await;
    }
}
