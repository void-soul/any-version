use super::*;
use std::time::{Duration, Instant};

pub struct OwnedChild(Child);
impl OwnedChild {
    pub(super) fn new(child: Child) -> Self {
        Self(child)
    }
    pub(super) fn stop(&mut self) -> Result<(), String> {
        if matches!(self.0.try_wait(), Ok(Some(_))) {
            return Ok(());
        }
        cert_deployment::reload_command::terminate_child(&mut self.0)
    }
}
impl std::ops::Deref for OwnedChild {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.0
    }
}
impl std::ops::DerefMut for OwnedChild {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.0
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[derive(Default)]
pub struct OutputReaders {
    stopped: Arc<AtomicBool>,
    threads: Vec<std::thread::JoinHandle<()>>,
}
impl OutputReaders {
    pub(super) fn spawn<R: std::io::Read + Send + 'static>(
        &mut self,
        stream: R,
        logs: Arc<Mutex<Vec<String>>>,
        last_logs: Arc<Mutex<HashMap<String, Vec<String>>>>,
        instance_id: String,
        prefix: &'static str,
    ) {
        let stopped = self.stopped.clone();
        self.threads.push(std::thread::spawn(move || {
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                let line = format!("{prefix}{line}");
                {
                    let mut entries = logs.lock();
                    if stopped.load(Ordering::SeqCst) {
                        break;
                    }
                    if entries.len() >= 300 {
                        entries.remove(0);
                    }
                    entries.push(line.clone());
                }
                let mut last = last_logs.lock();
                if stopped.load(Ordering::SeqCst) {
                    break;
                }
                let entries = last.entry(instance_id.clone()).or_default();
                if entries.len() >= 300 {
                    entries.remove(0);
                }
                entries.push(line);
            }
        }));
    }
    pub(super) fn stop_writes(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }
    pub(super) fn finish(&mut self) -> Result<(), String> {
        self.stop_writes();
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.threads.iter().any(|thread| !thread.is_finished()) {
            if Instant::now() >= deadline {
                return Err("RTSP 日志读取线程未确认退出".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        for thread in self.threads.drain(..) {
            thread
                .join()
                .map_err(|_| "RTSP 日志线程异常退出".to_string())?;
        }
        Ok(())
    }
}
impl Drop for OutputReaders {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

pub(super) fn stop_instance(inner: &mut InnerRtspServer, clear_logs: bool) -> Result<(), String> {
    inner.should_stop.store(true, Ordering::SeqCst);
    inner.readers.stop_writes();
    let mut errors = Vec::new();
    if let Err(error) = inner.ffmpeg_child.stop() {
        errors.push(error);
    }
    if let Some(child) = &mut inner.mediamtx_child {
        if let Err(error) = child.stop() {
            errors.push(error);
        }
    }
    if let Err(error) = inner.readers.finish() {
        errors.push(error);
    }
    if clear_logs {
        *inner.logs.lock() = Vec::new();
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

pub(super) fn stop_all(state: &RtspServerState) -> Result<(), String> {
    let mut servers = state.servers.lock();
    let mut errors = Vec::new();
    servers.retain(|_, inner| match stop_instance(inner, true) {
        Ok(()) => false,
        Err(error) => {
            errors.push(error);
            true
        }
    });
    // All output writers are stopped before dropping any shared snapshots.
    state.last_logs.lock().clear();
    state.last_errors.lock().clear();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owned_fake_child_stop_reaps_process_and_reader_before_clearing_logs() {
        let mut command = Command::new(if cfg!(windows) { "cmd.exe" } else { "/bin/sh" });
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command
                .args(["/c", "echo synthetic-log & ping -n 10 127.0.0.1 >nul"])
                .creation_flags(0x08000000);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command
                .args(["-c", "printf 'synthetic-log\\n'; sleep 10"])
                .process_group(0);
        }
        let mut child = OwnedChild::new(command.stdout(Stdio::piped()).spawn().unwrap());
        let logs = Arc::new(Mutex::new(Vec::new()));
        let last = Arc::new(Mutex::new(HashMap::new()));
        let mut readers = OutputReaders::default();
        readers.spawn(
            child.stdout.take().unwrap(),
            logs.clone(),
            last.clone(),
            "fake".into(),
            "",
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        while logs.lock().is_empty() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        readers.stop_writes();
        child.stop().unwrap();
        readers.finish().unwrap();
        assert!(child.try_wait().unwrap().is_some());
        assert!(readers.threads.is_empty());
        logs.lock().clear();
        last.lock().clear();
        std::thread::sleep(Duration::from_millis(30));
        assert!(logs.lock().is_empty());
        assert!(last.lock().is_empty());
    }
    #[test]
    fn stopped_readers_cannot_repopulate_cleared_snapshots() {
        let logs = Arc::new(Mutex::new(Vec::new()));
        let last = Arc::new(Mutex::new(HashMap::new()));
        let mut readers = OutputReaders::default();
        readers.stop_writes();
        readers.spawn(
            std::io::Cursor::new(b"late-log\n"),
            logs.clone(),
            last.clone(),
            "fake".into(),
            "",
        );
        readers.finish().unwrap();
        assert!(logs.lock().is_empty());
        assert!(last.lock().is_empty());
    }

    #[test]
    fn stop_all_clears_stale_log_and_error_snapshots_without_running_children() {
        let state = RtspServerState::default();
        state
            .last_logs
            .lock()
            .insert("finished".into(), vec!["synthetic-log".into()]);
        state
            .last_errors
            .lock()
            .insert("finished".into(), Some("synthetic-error".into()));
        stop_all(&state).unwrap();
        assert!(state.servers.lock().is_empty());
        assert!(state.last_logs.lock().is_empty());
        assert!(state.last_errors.lock().is_empty());
    }

    #[test]
    fn unconfirmed_reader_is_retained_for_a_later_cleanup_retry() {
        struct Gate(std::sync::mpsc::Receiver<()>);
        impl std::io::Read for Gate {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                let _ = self.0.recv();
                Ok(0)
            }
        }
        let state = RtspServerState::default();
        let mut command = Command::new(if cfg!(windows) { "cmd.exe" } else { "/bin/sh" });
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.args(["/c", "exit 0"]).creation_flags(0x08000000);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.args(["-c", "exit 0"]).process_group(0);
        }
        let child = OwnedChild::new(command.spawn().unwrap());
        let logs = Arc::new(Mutex::new(Vec::new()));
        let (release, gate) = std::sync::mpsc::channel();
        let mut readers = OutputReaders::default();
        readers.spawn(
            Gate(gate),
            logs.clone(),
            state.last_logs.clone(),
            "retry".into(),
            "",
        );
        let config = serde_json::from_value(serde_json::json!({ "sourceType": "testsrc", "port": 0, "pathName": "fake", "allowLan": false, "loopFile": false, "includeAudio": false })).unwrap();
        state.servers.lock().insert(
            "retry".into(),
            InnerRtspServer {
                mediamtx_child: None,
                ffmpeg_child: child,
                config,
                start_time: Instant::now(),
                logs,
                should_stop: Arc::new(AtomicBool::new(false)),
                readers,
            },
        );
        assert!(stop_all(&state).is_err());
        assert!(state.servers.lock().contains_key("retry"));
        release.send(()).unwrap();
        stop_all(&state).unwrap();
        assert!(state.servers.lock().is_empty());
    }
}
