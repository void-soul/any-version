//! Owned background work: leases live inside blocking workers, and only their own children are reaped.
use std::future::Future;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
    pub fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
tokio::task_local! { static CURRENT: Cancellation; }
pub fn ensure_running(module: &str) -> Result<(), String> {
    crate::runtime_tasks::require_enabled(module)?;
    if CURRENT
        .try_with(Cancellation::is_cancelled)
        .unwrap_or(false)
    {
        Err("操作已取消".into())
    } else {
        Ok(())
    }
}

struct CancelAwaiter(Cancellation);
impl Drop for CancelAwaiter {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

pub async fn run<T, F, U>(module: &'static str, token: Cancellation, work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> U + Send + 'static,
    U: Future<Output = Result<T, String>>,
{
    let cancel_awaiter = CancelAwaiter(token.clone());
    crate::runtime_tasks::run_if_enabled(module, async move {
        let _cancel_awaiter = cancel_awaiter;
        tauri::async_runtime::spawn_blocking(move || {
            // This lease survives cancellation/drop of the awaiting IPC/task future.
            if token.is_cancelled() {
                return Err("操作已取消".into());
            }
            let _operation = crate::runtime_tasks::begin_operation(module)?;
            let task_token = token.clone();
            let work_with_cancel = async move {
                let cancelled = async {
                    loop {
                        if task_token.is_cancelled() {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(25)).await;
                    }
                };
                tokio::select! {
                    biased;
                    _ = cancelled => Err("操作已取消".into()),
                    result = work() => result,
                }
            };
            tauri::async_runtime::block_on(CURRENT.scope(
                token.clone(),
                crate::runtime_tasks::run_if_enabled(module, work_with_cancel),
            ))
        })
        .await
        .map_err(|e| format!("后台工作异常: {e}"))?
    })
    .await
}

static UNCONFIRMED: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, String>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));
/// Never report a stopped module when the OS refused to confirm its child tree's exit.
pub fn ensure_released(module: &str) -> Result<(), String> {
    match UNCONFIRMED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(module)
    {
        Some(error) => Err(format!("子进程退出尚未确认，请检查进程后重启应用: {error}")),
        None => Ok(()),
    }
}

pub fn capture(
    module: &str,
    command: &mut std::process::Command,
    timeout: Duration,
) -> Result<std::process::Output, String> {
    capture_observed(module, command, timeout, |_, _| {})
}
pub fn capture_observed(
    module: &str,
    command: &mut std::process::Command,
    timeout: Duration,
    on_output: impl FnMut(bool, &[u8]),
) -> Result<std::process::Output, String> {
    ensure_running(module)?;
    let _operation = crate::runtime_tasks::begin_operation(module)?;
    let token = CURRENT.try_with(Clone::clone).unwrap_or_default();
    let result = cert_deployment::reload_command::capture_observed(
        command,
        timeout,
        || !crate::runtime_tasks::is_enabled(module) || token.is_cancelled(),
        on_output,
    );
    if let Err(error) = &result {
        if error.contains("无法确认") || error.contains("仍未确认") {
            UNCONFIRMED
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(module.into(), error.clone());
        }
    }
    result
}

/// Own a new temporary directory; cancellation never removes an unrelated pre-existing path.
pub struct ScratchDir(std::path::PathBuf);
impl ScratchDir {
    pub fn new(prefix: &str) -> Result<Self, String> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        for _ in 0..1000 {
            let path = std::env::temp_dir().join(format!(
                "kira-{prefix}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
        Err("无法创建独占临时目录".into())
    }
    pub fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn worker_lease_and_temp_resources_outlive_dropped_awaiter() {
        const MODULE: &str = "qa-owned-worker-safe-finish";
        crate::runtime_tasks::enable(MODULE);
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, finish) = std::sync::mpsc::channel();
        let task = tokio::spawn(run(MODULE, Cancellation::default(), move || async move {
            let scratch = ScratchDir::new("owned-worker-test")?;
            std::fs::write(scratch.path().join("partial"), "fake download").unwrap();
            let _ = started.send(scratch.path().to_path_buf());
            finish
                .recv_timeout(Duration::from_secs(5))
                .map_err(|e| e.to_string())?;
            Ok(())
        }));
        let path = ready.await.unwrap();
        task.abort();
        let _ = task.await;
        assert!(path.exists());
        assert!(crate::runtime_tasks::wait_for_idle(MODULE, Duration::ZERO)
            .await
            .is_err());
        release.send(()).unwrap();
        crate::runtime_tasks::wait_for_idle(MODULE, Duration::from_secs(5))
            .await
            .unwrap();
        assert!(!path.exists());
    }
    #[tokio::test]
    async fn token_cancels_pending_download_and_cleans_scratch() {
        const MODULE: &str = "qa-owned-worker-download-cancel";
        crate::runtime_tasks::enable(MODULE);
        let token = Cancellation::default();
        let worker_token = token.clone();
        let (started, ready) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(run(MODULE, worker_token, move || async move {
            let scratch = ScratchDir::new("owned-pending-test")?;
            let _ = started.send(scratch.path().to_path_buf());
            std::future::pending::<Result<(), String>>().await
        }));
        let path = ready.await.unwrap();
        token.cancel();
        assert!(task.await.unwrap().is_err());
        assert!(!path.exists());
        crate::runtime_tasks::wait_for_idle(MODULE, Duration::from_secs(2))
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn cancelled_queued_work_never_runs_after_reenable() {
        const MODULE: &str = "qa-owned-worker-old-generation";
        let token = Cancellation::default();
        token.cancel();
        crate::runtime_tasks::enable(MODULE);
        let polled = Arc::new(AtomicBool::new(false));
        let observed = polled.clone();
        let result = run(MODULE, token, move || async move {
            observed.store(true, Ordering::SeqCst);
            Ok(())
        })
        .await;
        assert!(result.is_err());
        assert!(!polled.load(Ordering::SeqCst));
    }
}
