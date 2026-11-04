use std::process::{Command, Stdio};
use std::time::Duration;

/// Bound both pipe reads and process exit, including an inherited stdout pipe.
pub(super) fn run(mut command: Command, timeout: Duration) -> Result<Vec<u8>, String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    // This synchronous API is also called from Tokio tasks. Keep its temporary
    // runtime off the caller's runtime thread.
    std::thread::spawn(move || {
        use tokio::io::AsyncReadExt;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| "无法启动官方密钥读取任务".to_string())?;
        runtime.block_on(async move {
            let mut command = tokio::process::Command::from(command);
            command.kill_on_drop(true);
            let mut child = command
                .spawn()
                .map_err(|_| "无法启动 WorkBuddy 密钥读取进程".to_string())?;
            let stdout = child.stdout.take().ok_or("密钥读取管道不可用")?;
            let result = tokio::time::timeout(timeout, async {
                let read = async {
                    let mut bytes = Vec::new();
                    stdout
                        .take(65537)
                        .read_to_end(&mut bytes)
                        .await
                        .map_err(|_| "密钥读取失败")?;
                    if bytes.len() > 65536 {
                        return Err("官方密钥输出超出限制");
                    }
                    Ok(bytes)
                };
                let wait = async {
                    let status = child.wait().await.map_err(|_| "密钥读取进程异常")?;
                    if !status.success() {
                        return Err("密钥读取进程未正常退出");
                    }
                    Ok(())
                };
                tokio::try_join!(read, wait).map(|(bytes, ())| bytes)
            })
            .await;
            // On timeout or read failure, explicitly kill and reap the helper.
            if !matches!(result, Ok(Ok(_))) {
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
            result
                .map_err(|_| "读取 WorkBuddy 官方密钥超时".to_string())?
                .map_err(str::to_string)
        })
    })
    .join()
    .map_err(|_| "密钥读取任务异常退出".to_string())?
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    fn command(script: &str) -> Command {
        let mut command = Command::new("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-Command", script]);
        command
    }
    #[test]
    fn output_is_preserved_and_failure_is_sanitized() {
        assert_eq!(
            run(
                command("[Console]::Out.Write('fixture')"),
                Duration::from_secs(10)
            )
            .unwrap(),
            b"fixture"
        );
        assert!(!run(
            command("[Console]::Error.Write('secret'); exit 1"),
            Duration::from_secs(10)
        )
        .unwrap_err()
        .contains("secret"));
    }
    #[test]
    fn hung_or_oversized_helpers_fail() {
        assert!(run(
            command("Start-Sleep -Seconds 20"),
            Duration::from_millis(100)
        )
        .unwrap_err()
        .contains("超时"));
        assert!(run(
            command("[Console]::Out.Write(('x' * 70000)); Start-Sleep -Seconds 20"),
            Duration::from_secs(10)
        )
        .unwrap_err()
        .contains("超出限制"));
    }
}
