//! Bounded blocking command execution; callers run deployment on a blocking worker.
use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

struct Scratch(std::path::PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.0.join("stdout"));
        let _ = std::fs::remove_file(self.0.join("stderr"));
        let _ = std::fs::remove_dir(&self.0);
    }
}

/// Callers on Unix must start the child with process_group(0).
pub fn terminate_child(child: &mut std::process::Child) -> Result<(), String> {
    #[cfg(windows)]
    let tree_result = {
        use std::os::windows::process::CommandExt;
        let killer = Command::new("taskkill.exe")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .creation_flags(0x08000000)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match killer {
            Ok(mut killer) => {
                let deadline = Instant::now() + Duration::from_secs(2);
                loop {
                    match killer.try_wait() {
                        Ok(Some(status)) if status.success() => break Ok(()),
                        Ok(Some(status)) => {
                            break Err(format!("taskkill 返回 {status}，无法确认子进程树已停止"))
                        }
                        Err(error) => break Err(format!("无法确认 taskkill 结果: {error}")),
                        _ if Instant::now() >= deadline => {
                            let _ = killer.kill();
                            break Err("taskkill 超时，无法确认子进程树已停止".into());
                        }
                        _ => std::thread::sleep(Duration::from_millis(10)),
                    }
                }
            }
            Err(error) => Err(format!("无法启动 taskkill，子进程树状态未知: {error}")),
        }
    };
    #[cfg(unix)]
    {
        // The child starts a new process group below; the negative PGID addresses only that group.
        unsafe {
            libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
        }
    }
    let _ = child.kill();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                #[cfg(windows)]
                return tree_result;
                #[cfg(not(windows))]
                return Ok(());
            }
            Err(error) => return Err(format!("无法确认部署命令已退出: {error}")),
            _ if Instant::now() >= deadline => return Err("终止部署命令后仍未确认退出".into()),
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

pub fn run(program: &str, args: &[&str], cwd: Option<&str>) -> Result<String, String> {
    run_with_timeout(program, args, cwd, Duration::from_secs(30))
}

fn run_with_timeout(
    program: &str,
    args: &[&str],
    cwd: Option<&str>,
    timeout: Duration,
) -> Result<String, String> {
    let mut command = Command::new(program);
    command.args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let output = capture(&mut command, timeout)?;
    let text = [output.stdout, output.stderr]
        .into_iter()
        .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_string())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if !output.status.success() {
        return Err(format!("{program} 返回非零({}): {text}", output.status));
    }
    Ok(text)
}

/// File-backed capture avoids pipe readers surviving a timeout through inherited child handles.
/// Each returned stream is capped at 64 KiB. Commands run in an isolated process group/tree.
pub fn capture(command: &mut Command, timeout: Duration) -> Result<Output, String> {
    capture_observed(command, timeout, || false, |_, _| {})
}

/// Poll file-backed output on the calling thread. `on_output(true, bytes)` denotes stderr.
/// Callbacks must return promptly; cancellation and deadlines are checked every polling round.
pub fn capture_observed(
    command: &mut Command,
    timeout: Duration,
    mut should_cancel: impl FnMut() -> bool,
    mut on_output: impl FnMut(bool, &[u8]),
) -> Result<Output, String> {
    let program = command.get_program().to_string_lossy().into_owned();
    let scratch_path =
        std::env::temp_dir().join(format!("receiver-command-{}", crate::random_hex(12)));
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&scratch_path).map_err(|e| e.to_string())?;
    let scratch = Scratch(scratch_path);
    let create = |name: &str| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options
            .open(scratch.0.join(name))
            .map_err(|e| e.to_string())
    };
    let stdout = create("stdout")?;
    let stderr = create("stderr")?;
    // Independent opens have independent offsets; cloned writer handles would share the cursor.
    let mut stdout_reader =
        std::fs::File::open(scratch.0.join("stdout")).map_err(|e| e.to_string())?;
    let mut stderr_reader =
        std::fs::File::open(scratch.0.join("stderr")).map_err(|e| e.to_string())?;
    command.stdin(Stdio::null()).stdout(stdout).stderr(stderr);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let child = command.spawn();
    // Command itself retains its Stdio configuration; close those file handles before Scratch drops.
    command.stdout(Stdio::null()).stderr(Stdio::null());
    struct ChildGuard(Option<std::process::Child>);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            if let Some(child) = &mut self.0 {
                let _ = terminate_child(child);
            }
        }
    }
    let mut child = ChildGuard(Some(
        child.map_err(|e| format!("启动 {program} 失败: {e}"))?,
    ));
    let deadline = Instant::now() + timeout;
    fn pump(
        input: &mut std::fs::File,
        captured: &mut Vec<u8>,
        stderr: bool,
        on_output: &mut impl FnMut(bool, &[u8]),
    ) -> Result<usize, String> {
        let mut total = 0;
        let mut bytes = [0; 8192];
        // A continuously writing child cannot starve cancellation/deadline checks.
        while total < 64 * 1024 {
            let count = input
                .read(&mut bytes)
                .map_err(|e| format!("读取部署命令输出失败: {e}"))?;
            if count == 0 {
                break;
            }
            on_output(stderr, &bytes[..count]);
            let keep = count.min((64 * 1024_usize).saturating_sub(captured.len()));
            captured.extend_from_slice(&bytes[..keep]);
            total += count;
        }
        Ok(total)
    }
    let result = (|| {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut exited = None;
        loop {
            if should_cancel() {
                return Err(format!("{program} 已取消"));
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "{program} 执行超过 {} 秒，已终止",
                    timeout.as_secs_f64()
                ));
            }
            let read_out = pump(&mut stdout_reader, &mut stdout, false, &mut on_output)?;
            let read_err = pump(&mut stderr_reader, &mut stderr, true, &mut on_output)?;
            if let Some(status) = exited {
                if read_out == 0 && read_err == 0 {
                    return Ok(Output {
                        status,
                        stdout,
                        stderr,
                    });
                }
            } else {
                exited = child
                    .0
                    .as_mut()
                    .unwrap()
                    .try_wait()
                    .map_err(|e| format!("等待 {program} 失败: {e}"))?;
            }
            if exited.is_none() {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    })();
    let mut running = child.0.take().unwrap();
    match result {
        Ok(output) => Ok(output),
        Err(error) => match terminate_child(&mut running) {
            Ok(()) => Err(error),
            Err(cleanup) => Err(format!("{error}; {cleanup}")),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn observed_capture_streams_both_channels_and_drains_final_output() {
        #[cfg(windows)]
        let mut command = {
            let mut command = Command::new("cmd.exe");
            command.args(["/c", "echo progress & echo detail 1>&2 & echo done"]);
            command
        };
        #[cfg(unix)]
        let mut command = {
            let mut command = Command::new("/bin/sh");
            command.args(["-c", "printf progress; printf detail >&2; printf done"]);
            command
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let output = capture_observed(
            &mut command,
            Duration::from_secs(5),
            || false,
            |is_err, bytes| {
                if is_err {
                    stderr.extend_from_slice(bytes);
                } else {
                    stdout.extend_from_slice(bytes);
                }
            },
        )
        .unwrap();
        assert!(output.status.success());
        assert_eq!(stdout, output.stdout);
        assert_eq!(stderr, output.stderr);
        assert!(String::from_utf8_lossy(&stdout).contains("progress"));
        assert!(String::from_utf8_lossy(&stdout).contains("done"));
        assert!(String::from_utf8_lossy(&stderr).contains("detail"));
    }

    #[test]
    fn observed_progress_can_cancel_a_still_running_command_without_reader_threads() {
        #[cfg(windows)]
        let mut command = {
            let mut command = Command::new("cmd.exe");
            command.args(["/c", "echo progress & ping -n 10 127.0.0.1 >nul"]);
            command
        };
        #[cfg(unix)]
        let mut command = {
            let mut command = Command::new("/bin/sh");
            command.args(["-c", "printf progress; sleep 10"]);
            command
        };
        let cancel = std::cell::Cell::new(false);
        let started = Instant::now();
        let result = capture_observed(
            &mut command,
            Duration::from_secs(15),
            || cancel.get(),
            |_, bytes| {
                if !bytes.is_empty() {
                    cancel.set(true);
                }
            },
        );
        assert!(result.unwrap_err().contains("已取消"));
        assert!(cancel.get(), "output must arrive before cancellation");
        assert!(started.elapsed() < Duration::from_secs(5));
    }
    #[test]
    fn capture_keeps_exit_status_and_bounds_each_output_independently() {
        #[cfg(windows)]
        let mut command = {
            let mut command = Command::new("powershell.exe");
            command.args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "[Console]::Out.Write('a'*70000); [Console]::Error.Write('b'*70000); exit 7",
            ]);
            command
        };
        #[cfg(unix)]
        let mut command = {
            let mut command = Command::new("/bin/sh");
            command.args(["-c", "head -c 70000 /dev/zero | tr '\\0' a; head -c 70000 /dev/zero | tr '\\0' b >&2; exit 7"]);
            command
        };
        let output = capture(&mut command, Duration::from_secs(8)).unwrap();
        assert_eq!(output.status.code(), Some(7));
        assert_eq!(output.stdout, vec![b'a'; 64 * 1024]);
        assert_eq!(output.stderr, vec![b'b'; 64 * 1024]);
    }

    #[cfg(windows)]
    #[test]
    fn capture_timeout_kills_a_descendant_inheriting_output_files() {
        struct Probe(std::path::PathBuf);
        impl Drop for Probe {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let root = Probe(
            std::env::temp_dir().join(format!("capture-tree-test-{}", crate::random_hex(12))),
        );
        std::fs::create_dir(&root.0).unwrap();
        let marker = root.0.join("survived");
        let child_started = root.0.join("started");
        let child = root.0.join("child.cmd");
        let parent = root.0.join("parent.cmd");
        // ICMP-based sleeps can fail immediately in the Windows sandbox; use a real local timer.
        std::fs::write(&child, "@echo off\r\necho started > \"%KIRA_CAPTURE_STARTED%\"\r\npowershell.exe -NoProfile -NonInteractive -Command \"Start-Sleep -Seconds 3\"\r\necho survived > \"%KIRA_CAPTURE_MARKER%\"\r\n").unwrap();
        std::fs::write(&parent, "@echo off\r\nstart \"\" /b cmd.exe /c \"%KIRA_CAPTURE_CHILD%\"\r\npowershell.exe -NoProfile -NonInteractive -Command \"Start-Sleep -Seconds 10\"\r\n").unwrap();
        let mut command = Command::new("cmd.exe");
        command
            .arg("/c")
            .arg(parent)
            .env("KIRA_CAPTURE_MARKER", &marker)
            .env("KIRA_CAPTURE_STARTED", &child_started)
            .env("KIRA_CAPTURE_CHILD", child);
        let started = Instant::now();
        assert!(capture(&mut command, Duration::from_millis(500))
            .unwrap_err()
            .contains("已终止"));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(
            child_started.exists(),
            "the test must exercise an actual descendant"
        );
        std::thread::sleep(Duration::from_millis(3300));
        assert!(
            !marker.exists(),
            "a descendant survived the process-tree timeout"
        );
    }
    #[cfg(windows)]
    #[test]
    fn timeout_terminates_command_without_waiting_for_pipe_eof() {
        let start = Instant::now();
        let result = run_with_timeout(
            "powershell.exe",
            &[
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 5",
            ],
            None,
            Duration::from_millis(150),
        );
        assert!(result.unwrap_err().contains("已终止"));
        assert!(start.elapsed() < Duration::from_secs(4));
    }
    #[cfg(windows)]
    #[test]
    fn captures_success_and_reports_nonzero_exit() {
        assert_eq!(run("cmd.exe", &["/c", "echo done"], None).unwrap(), "done");
        assert!(run("cmd.exe", &["/c", "exit 7"], None).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn timeout_terminates_the_shell_process_group() {
        struct Probe(std::path::PathBuf);
        impl Drop for Probe {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(self.0.join("survived"));
                let _ = std::fs::remove_dir(&self.0);
            }
        }
        let root = Probe(
            std::env::temp_dir().join(format!("receiver-group-probe-{}", crate::random_hex(12))),
        );
        std::fs::create_dir(&root.0).unwrap();
        let marker = root.0.join("survived");
        let result = run_with_timeout(
            "/bin/sh",
            &[
                "-c",
                "(sleep 0.6; printf survived > \"$1\") & wait",
                "receiver-timeout-test",
                marker.to_str().unwrap(),
            ],
            None,
            Duration::from_millis(100),
        );
        assert!(result.unwrap_err().contains("已终止"));
        std::thread::sleep(Duration::from_millis(800));
        assert!(!marker.exists(), "a shell descendant survived the timeout");
    }

    #[cfg(unix)]
    #[test]
    fn captures_unix_command_success_and_failure() {
        assert_eq!(
            run("/bin/sh", &["-c", "printf done"], None).unwrap(),
            "done"
        );
        assert!(run("/bin/sh", &["-c", "exit 7"], None).is_err());
    }
}
