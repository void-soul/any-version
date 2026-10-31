//! Runtime-independent exit acknowledgement shared by both receiver adapters.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};
pub struct ExitGuard {
    finished: Arc<AtomicBool>,
    done: mpsc::Sender<()>,
}
impl ExitGuard {
    pub fn new(finished: Arc<AtomicBool>, done: mpsc::Sender<()>) -> Self {
        Self { finished, done }
    }
    pub fn mark_finished(&self) {
        self.finished.store(true, Ordering::SeqCst);
    }
}
impl Drop for ExitGuard {
    fn drop(&mut self) {
        self.mark_finished();
        let _ = self.done.send(());
    }
}
pub fn wait_for_exit(
    finished: &AtomicBool,
    completion: &Mutex<mpsc::Receiver<()>>,
    timeout: std::time::Duration,
) -> Result<(), String> {
    if finished.load(Ordering::SeqCst) {
        return Ok(());
    }
    completion
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .recv_timeout(timeout)
        .map_err(|_| {
            "Receiver is still stopping; retry after the active operation finishes".to_string()
        })
}
/// The caller serializes lifecycle changes; callbacks never run under a status lock.
pub fn apply_snapshot<C: Clone>(
    old: C,
    requested: C,
    restart: bool,
    stop: impl FnOnce() -> Result<(), String>,
    persist: impl Fn(&C) -> Result<(), String>,
    set: impl Fn(C),
    start: impl Fn() -> Result<(), String>,
) -> Result<(), String> {
    if restart {
        stop()?;
    }
    if let Err(error) = persist(&requested) {
        if restart {
            if let Err(resume) = start() {
                return Err(format!("配置写入失败: {error}；恢复服务失败: {resume}"));
            }
        }
        return Err(error);
    }
    set(requested);
    if restart {
        if let Err(error) = start() {
            set(old.clone());
            let rollback = persist(&old);
            let resume = start();
            return Err(format!(
                "新配置启动失败: {error}；恢复配置: {}；恢复服务: {}",
                rollback.err().unwrap_or_else(|| "成功".into()),
                resume.err().unwrap_or_else(|| "成功".into())
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn acknowledgement_requires_actual_exit_and_survives_repeated_waits() {
        let finished = Arc::new(AtomicBool::new(false));
        let (done, completion) = mpsc::channel();
        let completion = Mutex::new(completion);
        let guard = ExitGuard::new(finished.clone(), done);
        assert!(wait_for_exit(&finished, &completion, std::time::Duration::ZERO).is_err());
        drop(guard);
        assert!(wait_for_exit(&finished, &completion, std::time::Duration::ZERO).is_ok());
        assert!(wait_for_exit(&finished, &completion, std::time::Duration::ZERO).is_ok());
    }
    #[test]
    fn failed_new_snapshot_restores_persisted_and_running_snapshot() {
        use std::cell::{Cell, RefCell};
        let current = Cell::new(1);
        let writes = RefCell::new(vec![]);
        let starts = RefCell::new(vec![]);
        let result = apply_snapshot(
            1,
            2,
            true,
            || Ok(()),
            |c| {
                writes.borrow_mut().push(*c);
                Ok(())
            },
            |c| current.set(c),
            || {
                let c = current.get();
                starts.borrow_mut().push(c);
                if c == 2 {
                    Err("fake bind failure".into())
                } else {
                    Ok(())
                }
            },
        );
        assert!(result.is_err());
        assert_eq!(current.get(), 1);
        assert_eq!(*writes.borrow(), [2, 1]);
        assert_eq!(*starts.borrow(), [2, 1]);
    }
    #[test]
    fn stopped_snapshot_save_never_starts_and_failed_stop_never_persists() {
        let result = apply_snapshot(
            1,
            2,
            false,
            || panic!("stop"),
            |_| Ok(()),
            |_| {},
            || panic!("start"),
        );
        assert!(result.is_ok());
        let result = apply_snapshot(
            1,
            2,
            true,
            || Err("still draining".into()),
            |_| panic!("persist"),
            |_| panic!("set"),
            || panic!("start"),
        );
        assert!(result.is_err());
    }
}
