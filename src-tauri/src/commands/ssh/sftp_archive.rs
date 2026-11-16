//! SFTP 压缩下载：远端 `gzip` / `tar` 打包 → 下载压缩包 → 本地解压。
//!
//! 日志类文本压缩比通常 8–15 倍，弱网下收益明显；代价是**放弃续传**
//! （gz 流不能随机定位），以及远端 `/tmp` 需要容纳压缩包。

use super::session::ClientHandler;
use super::sftp::{self, SftpProgress, SftpState, TransferOutcome};
use super::sftp_path::{classify_sftp_error, normalize_remote_path};
use russh::client::Handle;
use russh::ChannelMsg;
use tauri::ipc::Channel;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const CHUNK: usize = 64 * 1024;

fn sftp_err<E: std::fmt::Display>(e: E) -> String {
    classify_sftp_error(&e.to_string()).message
}

/// POSIX shell 单引号转义：`it's` → `'it'\''s'`。
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// 远端临时文件：只保留 ASCII 字母数字，避免会话 id 里的符号参与 shell。
pub fn remote_temp_path(session_id: &str, suffix: &str) -> String {
    let safe: String = session_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("/tmp/kira-{safe}.{suffix}")
}

/// 把绝对路径拆成 `(父目录, 名字)`，供 `tar -C parent name` 使用。
pub fn split_parent_name(path: &str) -> Result<(String, String), String> {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "/" {
        return Err("不能打包根目录，请选择具体的子目录".to_string());
    }
    let (parent, name) = trimmed
        .rsplit_once('/')
        .ok_or_else(|| format!("不是绝对路径: {path}"))?;
    if name.is_empty() {
        return Err(format!("路径缺少目录名: {path}"));
    }
    Ok((
        if parent.is_empty() {
            "/".to_string()
        } else {
            parent.to_string()
        },
        name.to_string(),
    ))
}

fn progress(
    session_id: &str,
    transferred: u64,
    total: u64,
    done: bool,
    stage: Option<&str>,
) -> SftpProgress {
    SftpProgress {
        session_id: session_id.to_string(),
        direction: "download".to_string(),
        transferred,
        total,
        done,
        resumed_from: 0,
        stage: stage.map(str::to_string),
        message: None,
    }
}

async fn execute(st: &SftpState, command: &str) -> Result<(u32, String), String> {
    let handle = st.handle.lock().await;
    open_and_exec(&handle, command).await
}

async fn open_and_exec(
    handle: &Handle<ClientHandler>,
    command: &str,
) -> Result<(u32, String), String> {
    let mut channel = handle
        .channel_open_session()
        .await
        .map_err(|e| format!("打开远端命令通道失败: {e}"))?;
    channel
        .exec(true, command)
        .await
        .map_err(|e| format!("执行远端命令失败: {e}"))?;
    let mut output = Vec::new();
    let mut code = 0u32;
    while let Some(msg) = channel.wait().await {
        match msg {
            ChannelMsg::Data { data } => output.extend_from_slice(&data),
            ChannelMsg::ExtendedData { data, .. } => output.extend_from_slice(&data),
            ChannelMsg::ExitStatus { exit_status } => code = exit_status,
            ChannelMsg::Eof | ChannelMsg::Close => break,
            _ => {}
        }
    }
    Ok((code, String::from_utf8_lossy(&output).into_owned()))
}

async fn copy_remote_to_local(
    st: &SftpState,
    session_id: &str,
    remote: &str,
    local: &str,
    channel: &Channel<SftpProgress>,
) -> Result<u64, String> {
    let mut remote_file = st.session.open(remote).await.map_err(sftp_err)?;
    let total = remote_file
        .metadata()
        .await
        .ok()
        .and_then(|meta| meta.size)
        .unwrap_or(0);
    let mut local_file = tokio::fs::File::create(local)
        .await
        .map_err(|e| format!("创建本地临时文件失败: {e}"))?;
    let mut buffer = vec![0u8; CHUNK];
    let mut transferred = 0u64;
    loop {
        let read = remote_file.read(&mut buffer).await.map_err(sftp_err)?;
        if read == 0 {
            break;
        }
        local_file
            .write_all(&buffer[..read])
            .await
            .map_err(|e| format!("写本地临时文件失败: {e}"))?;
        transferred += read as u64;
        let _ = channel.send(progress(session_id, transferred, total, false, None));
    }
    local_file
        .flush()
        .await
        .map_err(|e| format!("刷新本地临时文件失败: {e}"))?;
    Ok(transferred)
}

fn extract_gz(src: &str, dst: &str) -> Result<(), String> {
    let input = std::fs::File::open(src).map_err(|e| format!("打开压缩包失败: {e}"))?;
    let mut decoder = flate2::read::GzDecoder::new(input);
    let mut output = std::fs::File::create(dst).map_err(|e| format!("创建本地文件失败: {e}"))?;
    std::io::copy(&mut decoder, &mut output).map_err(|e| format!("解压失败: {e}"))?;
    Ok(())
}

/// 单文件：远端 `gzip` → 下载 → 本地解压到 `local`。
pub async fn download_compressed(
    session_id: &str,
    current: &str,
    remote: &str,
    local: &str,
    channel: Channel<SftpProgress>,
) -> Result<TransferOutcome, String> {
    let st = sftp::state(session_id).await?;
    let full = normalize_remote_path(current, remote)?;
    let remote_tmp = remote_temp_path(session_id, "gz");
    let local_tmp = format!("{local}.kira-dl");

    let _ = channel.send(progress(session_id, 0, 0, false, Some("compress")));
    let command = format!(
        "gzip -c -- {} > {} 2>&1",
        shell_quote(&full),
        shell_quote(&remote_tmp)
    );
    let (code, output) = execute(&st, &command).await?;
    if code != 0 {
        let _ = execute(&st, &format!("rm -f -- {}", shell_quote(&remote_tmp))).await;
        return Err(format!(
            "远端压缩失败（gzip 退出码 {code}）：{}",
            output.trim()
        ));
    }

    let downloaded = copy_remote_to_local(&st, session_id, &remote_tmp, &local_tmp, &channel).await;
    let _ = execute(&st, &format!("rm -f -- {}", shell_quote(&remote_tmp))).await;

    let downloaded = match downloaded {
        Ok(value) => value,
        Err(error) => {
            let _ = std::fs::remove_file(&local_tmp);
            return Err(error);
        }
    };

    let _ = channel.send(progress(
        session_id,
        downloaded,
        downloaded,
        false,
        Some("extract"),
    ));
    if let Err(error) = extract_gz(&local_tmp, local) {
        // 保留压缩包，避免解压失败后用户还得重新下一次
        return Err(format!("{error}；压缩包已保留: {local_tmp}"));
    }
    let _ = std::fs::remove_file(&local_tmp);
    let _ = channel.send(progress(session_id, downloaded, downloaded, true, None));
    Ok(TransferOutcome {
        resumed_from: 0,
        transferred: downloaded,
    })
}

/// 目录：远端 `tar -czf` → 下载 → 解压到 `local_dir`（得到 `远程目录名/...`）。
pub async fn download_dir(
    session_id: &str,
    current: &str,
    remote_dir: &str,
    local_dir: &str,
    channel: Channel<SftpProgress>,
) -> Result<TransferOutcome, String> {
    let st = sftp::state(session_id).await?;
    let full = normalize_remote_path(current, remote_dir)?;
    let (parent, name) = split_parent_name(&full)?;
    let remote_tmp = remote_temp_path(session_id, "tar.gz");
    let local_tmp = format!(
        "{}.kira-{}-{}.tar.gz",
        local_dir.trim_end_matches(['/', '\\']),
        std::process::id(),
        chrono::Utc::now().timestamp_millis()
    );

    let _ = channel.send(progress(session_id, 0, 0, false, Some("compress")));
    let command = format!(
        "tar -czf {} -C {} {} 2>&1",
        shell_quote(&remote_tmp),
        shell_quote(&parent),
        shell_quote(&name)
    );
    let (code, output) = execute(&st, &command).await?;
    if code != 0 {
        let _ = execute(&st, &format!("rm -f -- {}", shell_quote(&remote_tmp))).await;
        return Err(format!(
            "远端打包失败（tar 退出码 {code}）：{}",
            output.trim()
        ));
    }

    let downloaded = copy_remote_to_local(&st, session_id, &remote_tmp, &local_tmp, &channel).await;
    let _ = execute(&st, &format!("rm -f -- {}", shell_quote(&remote_tmp))).await;

    let downloaded = match downloaded {
        Ok(value) => value,
        Err(error) => {
            let _ = std::fs::remove_file(&local_tmp);
            return Err(error);
        }
    };

    let _ = channel.send(progress(
        session_id,
        downloaded,
        downloaded,
        false,
        Some("extract"),
    ));
    let unpacked = (|| -> Result<(), String> {
        let input = std::fs::File::open(&local_tmp).map_err(|e| format!("打开压缩包失败: {e}"))?;
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(input));
        // `unpack` 内部走 `unpack_in`，会拒绝 `..` 与绝对路径，避免解压穿越
        archive
            .unpack(local_dir)
            .map_err(|e| format!("解压失败（压缩包已保留: {local_tmp}）: {e}"))
    })();
    if let Err(error) = unpacked {
        return Err(error);
    }
    let _ = std::fs::remove_file(&local_tmp);
    let _ = channel.send(progress(session_id, downloaded, downloaded, true, None));
    Ok(TransferOutcome {
        resumed_from: 0,
        transferred: downloaded,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote("/data/a;rm -rf /"), "'/data/a;rm -rf /'");
    }

    #[test]
    fn split_parent_name_handles_root_and_nested() {
        assert_eq!(
            split_parent_name("/data/logs").unwrap(),
            ("/data".to_string(), "logs".to_string())
        );
        assert_eq!(
            split_parent_name("/data/").unwrap(),
            ("/".to_string(), "data".to_string())
        );
        assert!(split_parent_name("/").is_err());
    }

    #[test]
    fn remote_temp_path_is_shell_safe() {
        let path = remote_temp_path("sftp_1710000000000_1", "tar.gz");
        assert_eq!(path, "/tmp/kira-sftp_1710000000000_1.tar.gz");
        assert!(!path.contains(' '));
    }
}
