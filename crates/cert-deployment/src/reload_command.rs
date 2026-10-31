//! Bounded blocking command execution; callers run deployment on a blocking worker.
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct Scratch(std::path::PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.0.join("stdout"));
        let _ = std::fs::remove_file(self.0.join("stderr"));
        let _ = std::fs::remove_dir(&self.0);
    }
}

fn terminate(child: &mut std::process::Child) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill.exe")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .creation_flags(0x08000000)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(unix)]
    {
        // The child starts a new process group below; the negative PGID addresses only that group.
        unsafe {
            libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
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
    let scratch =
        Scratch(std::env::temp_dir().join(format!("receiver-command-{}", crate::random_hex(12))));
    std::fs::create_dir(&scratch.0).map_err(|e| e.to_string())?;
    let stdout = std::fs::File::create(scratch.0.join("stdout")).map_err(|e| e.to_string())?;
    let stderr = std::fs::File::create(scratch.0.join("stderr")).map_err(|e| e.to_string())?;
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr);
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
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("启动 {program} 失败: {e}"))?;
    drop(command);
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Err(e) => {
                terminate(&mut child);
                return Err(format!("等待 {program} 失败: {e}"));
            }
            Ok(None) => {}
        }
        if Instant::now() >= deadline {
            terminate(&mut child);
            return Err(format!(
                "{program} 执行超过 {} 秒，已终止",
                timeout.as_secs_f64()
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let read = |file: &str| -> String {
        let mut bytes = Vec::new();
        if let Ok(input) = std::fs::File::open(scratch.0.join(file)) {
            let _ = input.take(64 * 1024).read_to_end(&mut bytes);
        }
        String::from_utf8_lossy(&bytes).trim().to_string()
    };
    let output = [read("stdout"), read("stderr")]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if !status.success() {
        return Err(format!("{program} 返回非零({status}): {output}"));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
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
