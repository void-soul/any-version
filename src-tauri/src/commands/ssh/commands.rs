//! SSH 的 Tauri 命令。连接信息从 rdp 连接表读取，密码在内存中解密后立即使用。
use super::forward::ForwardRule;
use super::forward_runtime;
use super::host_keys::{self, decode_host_key_prompt, HostKeyDecision, HostKeyPrompt};
use super::session::{self, SshOutput};
use super::sftp::{self, SftpEntry, SftpProgress};
use super::target::{KeyAuth, SshTarget};
use crate::commands::rdp::finalshell_crypto;
use crate::commands::rdp::models::RdpConnection;
use crate::commands::rdp::store;
use crate::commands::secrets;
use base64::{engine::general_purpose, Engine as _};
use tauri::ipc::Channel;

/// 会话 id 必须进程内唯一。
///
/// 只用毫秒时间戳会在同一毫秒内撞名：React 开发模式的双挂载会连两次，
/// 第一次卸载时 `ssh_disconnect(id)` 会把第二次的同名会话一起断掉，
/// 表现就是「刚打开就显示已断开」。
pub(super) fn next_session_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    format!(
        "{prefix}_{}_{}",
        chrono::Utc::now().timestamp_millis(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

/// 用连接表里已保存的凭据解析出明文密码。
/// FinalShell 密文走 DES 兼容层，自有的走 AES-GCM；解不开就如实报错，不静默降级。
fn resolve_password(conn: &RdpConnection) -> Result<String, String> {
    if let Some(cipher) = conn.finalshell_cipher.as_deref() {
        return finalshell_crypto::decrypt(cipher);
    }
    if let Some(enc) = conn.password.as_deref() {
        if enc.is_empty() {
            return Ok(String::new());
        }
        return secrets::decrypt_secret(enc);
    }
    Err("该连接未保存密码，请先编辑连接填写密码".to_string())
}

/// 从连接表取出一条 SSH 连接并解析凭据；RDP 连接不能开 SSH/SFTP。
fn ssh_target(connection_id: &str) -> Result<SshTarget, String> {
    let conn = store::load()?
        .into_iter()
        .find(|c| c.id == connection_id)
        .ok_or_else(|| format!("连接不存在: {connection_id}"))?;
    if conn.kind != "ssh" {
        return Err("该连接是 Windows 远程桌面，不能用于 SSH/SFTP".to_string());
    }
    let key_auth = if conn.auth_type == "key" {
        let passphrase = if conn.key_passphrase.is_empty() {
            None
        } else {
            Some(secrets::decrypt_secret(&conn.key_passphrase)?)
        };
        Some(KeyAuth {
            path: conn.private_key_path.clone(),
            passphrase,
        })
    } else {
        None
    };
    let password = if key_auth.is_some() {
        String::new()
    } else {
        resolve_password(&conn)?
    };
    Ok(SshTarget {
        host: conn.host.clone(),
        port: conn.port,
        username: conn.username.clone(),
        password,
        key_auth,
    })
}

/// 打开一个 SSH 会话，返回 session_id。数据通过 `onOutput` 流式推给前端。
#[tauri::command]
pub async fn ssh_connect(
    connection_id: String,
    cols: u32,
    rows: u32,
    on_output: Channel<SshOutput>,
) -> Result<String, String> {
    let target = ssh_target(&connection_id)?;
    let session_id = next_session_id("ssh");
    session::connect(session_id.clone(), target, cols, rows, on_output).await?;
    Ok(session_id)
}

/// 键盘输入，data 是 base64 编码的原始字节。
#[tauri::command]
pub async fn ssh_write(session_id: String, data: String) -> Result<(), String> {
    let bytes = general_purpose::STANDARD
        .decode(&data)
        .map_err(|e| format!("终端输入不是有效 Base64: {e}"))?;
    session::write(&session_id, bytes).await
}

#[tauri::command]
pub async fn ssh_resize(session_id: String, cols: u32, rows: u32) -> Result<(), String> {
    session::resize(&session_id, cols, rows).await
}

#[tauri::command]
pub async fn ssh_disconnect(session_id: String) -> Result<(), String> {
    session::disconnect(&session_id).await;
    Ok(())
}

/// 打开 SFTP 会话，返回 (sessionId, 远端当前目录)。
#[tauri::command]
pub async fn sftp_open(connection_id: String) -> Result<(String, String), String> {
    let target = ssh_target(&connection_id)?;
    let session_id = next_session_id("sftp");
    let cwd = sftp::open(session_id.clone(), target).await?;
    Ok((session_id, cwd))
}

#[tauri::command]
pub async fn sftp_list(
    session_id: String,
    current: String,
    path: String,
) -> Result<(String, Vec<SftpEntry>), String> {
    sftp::list(&session_id, &current, &path).await
}

#[tauri::command]
pub async fn sftp_mkdir(session_id: String, current: String, path: String) -> Result<(), String> {
    sftp::mkdir(&session_id, &current, &path).await
}

#[tauri::command]
pub async fn sftp_remove(
    session_id: String,
    current: String,
    path: String,
    is_dir: bool,
) -> Result<(), String> {
    sftp::remove(&session_id, &current, &path, is_dir).await
}

#[tauri::command]
pub async fn sftp_rename(
    session_id: String,
    current: String,
    from: String,
    to: String,
) -> Result<(), String> {
    sftp::rename(&session_id, &current, &from, &to).await
}

/// 下载到本地。`localPath` 由前端保存对话框给出，后端不自行拼接用户目录。
/// `resume=true` 时按侧车元数据判定是否续传。
#[tauri::command]
pub async fn sftp_download(
    session_id: String,
    current: String,
    remote_path: String,
    local_path: String,
    resume: bool,
    on_progress: Channel<SftpProgress>,
) -> Result<sftp::TransferOutcome, String> {
    sftp::download(
        &session_id,
        &current,
        &remote_path,
        &local_path,
        resume,
        on_progress,
    )
    .await
}

/// 压缩下载单文件：远端 gzip → 下载 → 本地解压。不支持续传。
#[tauri::command]
pub async fn sftp_download_compressed(
    session_id: String,
    current: String,
    remote_path: String,
    local_path: String,
    on_progress: Channel<SftpProgress>,
) -> Result<sftp::TransferOutcome, String> {
    super::sftp_archive::download_compressed(
        &session_id,
        &current,
        &remote_path,
        &local_path,
        on_progress,
    )
    .await
}

/// 打包下载目录：远端 tar.gz → 下载 → 解压到本地目录。
#[tauri::command]
pub async fn sftp_download_dir(
    session_id: String,
    current: String,
    remote_dir: String,
    local_dir: String,
    on_progress: Channel<SftpProgress>,
) -> Result<sftp::TransferOutcome, String> {
    super::sftp_archive::download_dir(&session_id, &current, &remote_dir, &local_dir, on_progress)
        .await
}

#[tauri::command]
pub async fn sftp_upload(
    session_id: String,
    current: String,
    local_path: String,
    remote_path: String,
    resume: bool,
    on_progress: Channel<SftpProgress>,
) -> Result<sftp::TransferOutcome, String> {
    sftp::upload(
        &session_id,
        &current,
        &local_path,
        &remote_path,
        resume,
        on_progress,
    )
    .await
}

#[tauri::command]
pub async fn sftp_disconnect(session_id: String) -> Result<(), String> {
    sftp::disconnect(&session_id).await;
    Ok(())
}

/// 解析连接错误里可能携带的「首次连接指纹确认」提示。
#[tauri::command]
pub fn ssh_parse_host_key_prompt(err: String) -> Option<HostKeyPrompt> {
    decode_host_key_prompt(&err)
}

/// 用户看过指纹并确认后写入信任库。
/// `decision=changed` 时前端已做过二次确认，这里不再比较旧值（写入即覆盖）。
#[tauri::command]
pub fn ssh_trust_host(
    host: String,
    port: u16,
    fingerprint: String,
    decision: HostKeyDecision,
) -> Result<(), String> {
    if host.trim().is_empty() || port == 0 {
        return Err("主机或端口不合法".to_string());
    }
    if !fingerprint.starts_with("SHA256:") || fingerprint.len() < 12 {
        return Err("主机指纹格式不正确".to_string());
    }
    let _ = decision; // 前端负责区分「首次信任」和「密钥变更后仍信任」的提示强度
    host_keys::trust(&host, port, &fingerprint)
}

/// 打开服务器状态监控会话，返回 session_id。
#[tauri::command]
pub async fn ssh_monitor_open(connection_id: String) -> Result<String, String> {
    let target = ssh_target(&connection_id)?;
    let session_id = next_session_id("mon");
    super::monitor::open(session_id.clone(), target).await?;
    Ok(session_id)
}

/// 采一帧服务器状态（前端每 2 秒调一次）。
#[tauri::command]
pub async fn ssh_monitor_sample(session_id: String) -> Result<super::sysinfo::SysSnapshot, String> {
    super::monitor::sample(&session_id).await
}

#[tauri::command]
pub async fn ssh_monitor_close(session_id: String) -> Result<(), String> {
    super::monitor::close(&session_id).await;
    Ok(())
}

/// 按需拉一次进程列表（打开/刷新进程面板时调用，不参与 2 秒轮询）。
#[tauri::command]
pub async fn ssh_monitor_processes(
    session_id: String,
) -> Result<Vec<super::procinfo::ProcessRow>, String> {
    let raw = super::monitor::exec(&session_id, super::procinfo::PROCESS_COMMAND).await?;
    Ok(super::procinfo::parse_processes(&raw))
}

/// 按需拉一次监听端口列表。
#[tauri::command]
pub async fn ssh_monitor_netlinks(
    session_id: String,
) -> Result<Vec<super::procinfo::NetLinkRow>, String> {
    let raw = super::monitor::exec(&session_id, super::procinfo::NET_COMMAND).await?;
    Ok(super::procinfo::parse_netlinks(&raw))
}

/// 强杀远端进程（SIGKILL）；PID ≤ 1 直接拒绝，避免误杀 init。
#[tauri::command]
pub async fn ssh_monitor_kill(session_id: String, pid: i64) -> Result<String, String> {
    if pid <= 1 {
        return Err("拒绝终止 PID ≤ 1 的进程".to_string());
    }
    let output = super::monitor::exec(&session_id, &format!("kill -9 {pid} 2>&1")).await?;
    let output = output.trim();
    if output.is_empty() {
        Ok(format!("已向进程 {pid} 发送 SIGKILL"))
    } else {
        Err(format!("终止进程 {pid} 失败: {output}"))
    }
}

/// 读用户自定义快捷命令（按目录分组，命令可多行）。
#[tauri::command]
pub fn ssh_snippets_load() -> Result<super::snippets::SnippetFile, String> {
    super::snippets::load()
}

/// 整体覆盖保存快捷命令；后端做 id 唯一与内容非空校验。
#[tauri::command]
pub fn ssh_snippets_save(data: super::snippets::SnippetFile) -> Result<(), String> {
    super::snippets::save(&data)
}

/// 把用户选择的私钥复制进应用密钥库，返回库内路径。原文件不动。
#[tauri::command]
pub fn ssh_import_key(src: String) -> Result<String, String> {
    super::keys::import(&src)
}

/// 列出密钥库里已导入的私钥路径。
#[tauri::command]
pub fn ssh_list_keys() -> Vec<String> {
    super::keys::list()
}

/// 列出已信任的主机（供用户查看/撤销）。
#[tauri::command]
pub fn ssh_list_trusted_hosts() -> Vec<super::host_keys::TrustedHost> {
    super::host_keys::list_trusted(&host_keys::load_store())
}

/// 撤销某个主机的信任；下次连接会重新要求确认指纹。
#[tauri::command]
pub fn ssh_forget_host(
    host: String,
    port: u16,
) -> Result<Vec<super::host_keys::TrustedHost>, String> {
    let next = super::host_keys::remove_host(&host_keys::load_store(), &host, port);
    host_keys::save_store(&next)?;
    Ok(super::host_keys::list_trusted(&next))
}

/// 新增一条转发。返回实际监听端口（Remote 的 port=0 由服务端分配）。
#[tauri::command]
pub async fn ssh_forward_start(
    id: String,
    connection_id: String,
    rule: ForwardRule,
) -> Result<u16, String> {
    let target = ssh_target(&connection_id)?;
    forward_runtime::start(id, connection_id, rule, target).await
}

#[tauri::command]
pub async fn ssh_forward_stop(id: String) -> Result<(), String> {
    forward_runtime::stop(&id).await;
    Ok(())
}

/// 只返回该连接名下的转发，避免打开 A 的面板看到并误停 B 的转发。
#[tauri::command]
pub fn ssh_forward_list(connection_id: String) -> Vec<ForwardRule> {
    forward_runtime::list(&connection_id)
}
