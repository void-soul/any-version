//! SFTP 会话层：复用已保存的连接凭据，独立建立一条 subsystem=sftp 的 SSH 连接。
//!
//! 每个会话自己持有 `russh::client::Handle` 保活（drop 会让 session 结束）。
//! 上传/下载按 64 KiB 分块流式处理，避免把整个文件读进内存；进度经 Tauri Channel 推送。
//! 支持断点续传：下载靠本地侧车记录的「远端 size+mtime」判定部分文件可信，
//! 上传靠「远端已有部分尾部窗口哈希」与本地对比，判定是否同一版本。
use super::session::{authenticate, ClientHandler};
use super::sftp_path::{classify_sftp_error, normalize_remote_path};
use super::sftp_resume::{
    parse_meta, plan_transfer_with_resume, serialize_meta, sidecar_path, TransferPlan,
};
use super::target::SshTarget;
use russh::client::Handle;
use russh::Disconnect;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::OpenFlags;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tauri::ipc::Channel;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::Mutex;

const CHUNK: usize = 64 * 1024;
/// 上传续传前比对远端已有部分的窗口大小（只在尾部比对，代价恒定）。
const VERIFY_WINDOW: u64 = 256 * 1024;

pub struct SftpState {
    /// 显式持有 SSH 句柄保活。用互斥量而非独占所有权：即使传输仍持有本状态的 Arc，
    /// disconnect 也能真正关掉底层连接，避免残留后台 SSH 会话。
    pub(super) handle: Mutex<Handle<ClientHandler>>,
    pub(super) session: Arc<SftpSession>,
}

fn registry() -> &'static Mutex<HashMap<String, Arc<SftpState>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Arc<SftpState>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) async fn state(session_id: &str) -> Result<Arc<SftpState>, String> {
    registry()
        .lock()
        .await
        .get(session_id)
        .cloned()
        .ok_or_else(|| "SFTP 会话不存在或已断开，请重新打开文件管理".to_string())
}

fn sftp_err<E: std::fmt::Display>(e: E) -> String {
    classify_sftp_error(&e.to_string()).message
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub mtime: Option<u32>,
}

/// 一次传输的结果，便于调用方判断「是续传还是重传」。
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferOutcome {
    /// 0 表示从头传；>0 表示从该偏移续传。
    pub resumed_from: u64,
    /// 本次传输结束后的总字节数（含续传前已有的部分）。
    pub transferred: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpProgress {
    pub session_id: String,
    pub direction: String,
    pub transferred: u64,
    pub total: u64,
    pub done: bool,
    /// 本次是从该偏移续传的（0 表示从头）。
    pub resumed_from: u64,
    /// 压缩/解压等阶段提示（"compress" / "extract"）；普通传输为 None。
    pub stage: Option<String>,
    pub message: Option<String>,
}

/// 打开 SFTP 会话，返回 session_id 与远端当前目录（由服务端 canonicalize 决定）。
pub async fn open(session_id: String, target: SshTarget) -> Result<String, String> {
    if registry().lock().await.contains_key(&session_id) {
        return Err(format!("SFTP 会话已存在: {session_id}"));
    }
    let mut handle = authenticate(target).await?;
    let channel = handle.channel_open_session().await.map_err(sftp_err)?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(sftp_err)?;
    let session = Arc::new(
        SftpSession::new(channel.into_stream())
            .await
            .map_err(sftp_err)?,
    );
    let cwd = session.canonicalize(".").await.map_err(sftp_err)?;
    registry().lock().await.insert(
        session_id,
        Arc::new(SftpState {
            handle: Mutex::new(handle),
            session,
        }),
    );
    Ok(cwd)
}

/// 列目录。`path` 相对当前目录或绝对路径，统一走规范化。
pub async fn list(
    session_id: &str,
    current: &str,
    path: &str,
) -> Result<(String, Vec<SftpEntry>), String> {
    let st = state(session_id).await?;
    let full = normalize_remote_path(current, path)?;
    let entries = st.session.read_dir(&full).await.map_err(sftp_err)?;
    let mut out: Vec<SftpEntry> = entries
        .map(|e| {
            let meta = e.metadata();
            let ft = meta.file_type();
            SftpEntry {
                name: e.file_name(),
                path: e.path(),
                is_dir: ft.is_dir(),
                is_symlink: ft.is_symlink(),
                size: meta.size.unwrap_or(0),
                mtime: meta.mtime,
            }
        })
        .filter(|e| e.name != "." && e.name != "..")
        .collect();
    // 目录优先、同类按名称，避免每次打开顺序抖动
    out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    Ok((full, out))
}

pub async fn mkdir(session_id: &str, current: &str, path: &str) -> Result<(), String> {
    let st = state(session_id).await?;
    let full = normalize_remote_path(current, path)?;
    st.session.create_dir(&full).await.map_err(sftp_err)
}

pub async fn remove(
    session_id: &str,
    current: &str,
    path: &str,
    is_dir: bool,
) -> Result<(), String> {
    let st = state(session_id).await?;
    let full = normalize_remote_path(current, path)?;
    if full == "/" {
        return Err("不能删除根目录".to_string());
    }
    if is_dir {
        st.session.remove_dir(&full).await.map_err(sftp_err)
    } else {
        st.session.remove_file(&full).await.map_err(sftp_err)
    }
}

pub async fn rename(session_id: &str, current: &str, from: &str, to: &str) -> Result<(), String> {
    let st = state(session_id).await?;
    let src = normalize_remote_path(current, from)?;
    // 目标通常已经是同目录的文件名，规范化时沿用源目录
    let dst = normalize_remote_path(current, to)?;
    st.session.rename(&src, &dst).await.map_err(sftp_err)
}

async fn emit(channel: &Channel<SftpProgress>, ev: SftpProgress) -> bool {
    channel.send(ev).is_ok()
}

/// 下载远端文件到本地路径。`resume=true` 时若本地已有部分且侧车元数据与远端一致，
/// 则从本地大小处续传；否则整份重下。调用方保证 local 来自用户保存对话框。
pub async fn download(
    session_id: &str,
    current: &str,
    remote: &str,
    local: &str,
    resume: bool,
    channel: Channel<SftpProgress>,
) -> Result<TransferOutcome, String> {
    let st = state(session_id).await?;
    let full = normalize_remote_path(current, remote)?;
    let mut rf = st.session.open(&full).await.map_err(sftp_err)?;
    let meta = rf.metadata().await.map_err(sftp_err)?;
    let total = meta.size.unwrap_or(0);
    let remote_mtime = meta.mtime;

    // 侧车记录的是「上次完整下载时的远端指纹」，不一致就说明部分文件可能来自旧版本。
    // mtime 只到秒级，因此再比对一次「远端尾部窗口 vs 本地同区间」，防止同秒改内容漏检。
    let local_size = tokio::fs::metadata(local)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    let sidecar_ok = resume
        && std::fs::read_to_string(sidecar_path(local))
            .ok()
            .and_then(|t| parse_meta(&t))
            .map(|m| m.size == total && m.mtime == remote_mtime)
            .unwrap_or(false);
    let window_ok = if sidecar_ok && local_size > 0 && local_size < total {
        remote_window_matches(&st.session, &full, local, local_size)
            .await
            .unwrap_or(false)
    } else {
        false
    };
    let meta_ok = sidecar_ok && window_ok;
    // resume=false 是用户明确要求覆盖，不能因为大小相同就跳过
    let start = match plan_transfer_with_resume(resume, local_size, total, meta_ok) {
        TransferPlan::Skip => {
            let _ = emit(
                &channel,
                progress(session_id, "download", total, total, true, 0, None),
            )
            .await;
            return Ok(TransferOutcome {
                resumed_from: 0,
                transferred: total,
            });
        }
        TransferPlan::Resume(n) => n,
        TransferPlan::Fresh => 0,
    };

    if start > 0 {
        rf.seek(std::io::SeekFrom::Start(start))
            .await
            .map_err(sftp_err)?;
    }
    let mut lf = if start > 0 {
        tokio::fs::OpenOptions::new()
            .append(true)
            .open(local)
            .await
            .map_err(|e| format!("打开本地文件续传失败: {e}"))?
    } else {
        tokio::fs::File::create(local)
            .await
            .map_err(|e| format!("创建本地文件失败: {e}"))?
    };

    let mut buf = vec![0u8; CHUNK];
    let mut sent = start;
    loop {
        let n = rf.read(&mut buf).await.map_err(sftp_err)?;
        if n == 0 {
            break;
        }
        lf.write_all(&buf[..n])
            .await
            .map_err(|e| format!("写本地文件失败: {e}"))?;
        sent += n as u64;
        if !emit(
            &channel,
            progress(session_id, "download", sent, total, false, start, None),
        )
        .await
        {
            return Err("前端已关闭，下载已中止（已传部分保留，可再次续传）".to_string());
        }
    }
    lf.flush()
        .await
        .map_err(|e| format!("刷新本地文件失败: {e}"))?;
    // 只有完整传完才写侧车：中途中断时下次仍按旧元数据判断，不会误信半个文件。
    if sent == total {
        let _ = std::fs::write(sidecar_path(local), serialize_meta(total, remote_mtime));
    }
    let _ = emit(
        &channel,
        progress(session_id, "download", sent, total, true, start, None),
    )
    .await;
    Ok(TransferOutcome {
        resumed_from: start,
        transferred: sent,
    })
}

/// 上传本地文件到远端路径。`resume=true` 且远端已有部分与本地尾部窗口哈希一致时续传。
pub async fn upload(
    session_id: &str,
    current: &str,
    local: &str,
    remote: &str,
    resume: bool,
    channel: Channel<SftpProgress>,
) -> Result<TransferOutcome, String> {
    let st = state(session_id).await?;
    let full = normalize_remote_path(current, remote)?;
    let mut lf = tokio::fs::File::open(local)
        .await
        .map_err(|e| format!("打开本地文件失败: {e}"))?;
    let total = lf
        .metadata()
        .await
        .map_err(|e| format!("读取本地文件信息失败: {e}"))?
        .len();

    let remote_size = st
        .session
        .metadata(&full)
        .await
        .ok()
        .and_then(|m| m.size)
        .unwrap_or(0);
    let meta_ok = resume
        && remote_size > 0
        && remote_size < total
        && remote_window_matches(&st.session, &full, local, remote_size)
            .await
            .unwrap_or(false);
    let start = match plan_transfer_with_resume(resume, remote_size, total, meta_ok) {
        TransferPlan::Skip => {
            let _ = emit(
                &channel,
                progress(session_id, "upload", total, total, true, 0, None),
            )
            .await;
            return Ok(TransferOutcome {
                resumed_from: 0,
                transferred: total,
            });
        }
        TransferPlan::Resume(n) => n,
        TransferPlan::Fresh => 0,
    };

    let mut rf = if start > 0 {
        // 续传只能写，不能 CREATE|TRUNCATE，否则会把已有字节清空
        st.session
            .open_with_flags(&full, OpenFlags::WRITE)
            .await
            .map_err(sftp_err)?
    } else {
        st.session.create(&full).await.map_err(sftp_err)?
    };
    if start > 0 {
        rf.seek(std::io::SeekFrom::Start(start))
            .await
            .map_err(sftp_err)?;
        lf.seek(std::io::SeekFrom::Start(start))
            .await
            .map_err(|e| format!("定位本地文件失败: {e}"))?;
    }

    let mut buf = vec![0u8; CHUNK];
    let mut sent = start;
    loop {
        let n = lf
            .read(&mut buf)
            .await
            .map_err(|e| format!("读本地文件失败: {e}"))?;
        if n == 0 {
            break;
        }
        rf.write_all(&buf[..n]).await.map_err(sftp_err)?;
        sent += n as u64;
        if !emit(
            &channel,
            progress(session_id, "upload", sent, total, false, start, None),
        )
        .await
        {
            return Err("前端已关闭，上传已中止（已传部分保留，可再次续传）".to_string());
        }
    }
    rf.flush().await.map_err(sftp_err)?;
    let _ = emit(
        &channel,
        progress(session_id, "upload", sent, total, true, start, None),
    )
    .await;
    Ok(TransferOutcome {
        resumed_from: start,
        transferred: sent,
    })
}

/// 比对远端已有部分「末尾窗口」与本地同区间的 SHA-256。
/// 只读一小段，代价恒定；能识别绝大多数「远端是另一个版本」的情况。
async fn remote_window_matches(
    session: &SftpSession,
    remote_path: &str,
    local_path: &str,
    remote_size: u64,
) -> Result<bool, String> {
    let from = remote_size.saturating_sub(VERIFY_WINDOW);
    let want = remote_size - from;
    if want == 0 {
        return Ok(true);
    }

    let mut rf = session.open(remote_path).await.map_err(sftp_err)?;
    rf.seek(std::io::SeekFrom::Start(from))
        .await
        .map_err(sftp_err)?;
    let remote_hash = hash_reader(&mut rf, want).await?;

    let mut lf = tokio::fs::File::open(local_path)
        .await
        .map_err(|e| format!("打开本地文件失败: {e}"))?;
    lf.seek(std::io::SeekFrom::Start(from))
        .await
        .map_err(|e| format!("定位本地文件失败: {e}"))?;
    let local_hash = hash_reader(&mut lf, want).await?;

    Ok(remote_hash == local_hash)
}

async fn hash_reader<R: AsyncReadExt + Unpin>(reader: &mut R, len: u64) -> Result<Vec<u8>, String> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    let mut left = len;
    while left > 0 {
        let want = left.min(buf.len() as u64) as usize;
        let n = reader.read(&mut buf[..want]).await.map_err(sftp_err)?;
        if n == 0 {
            // 读到的字节数比预期少，说明两端不一致
            return Ok(Vec::new());
        }
        hasher.update(&buf[..n]);
        left -= n as u64;
    }
    Ok(hasher.finalize().to_vec())
}

fn progress(
    session_id: &str,
    direction: &str,
    transferred: u64,
    total: u64,
    done: bool,
    resumed_from: u64,
    message: Option<String>,
) -> SftpProgress {
    SftpProgress {
        session_id: session_id.to_string(),
        direction: direction.to_string(),
        transferred,
        total,
        done,
        resumed_from,
        stage: None,
        message,
    }
}

pub async fn disconnect(session_id: &str) {
    if let Some(st) = registry().lock().await.remove(session_id) {
        let _ = st
            .handle
            .lock()
            .await
            .disconnect(Disconnect::ByApplication, "", "English")
            .await;
    }
}
