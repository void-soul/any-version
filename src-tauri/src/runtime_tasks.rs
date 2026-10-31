//! App-lifetime task ownership; no business or storage dependencies.
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Mutex, OnceLock};
use tauri::async_runtime::JoinHandle;

#[derive(Default)]
struct Registry {
    disabled: HashSet<String>,
    tasks: HashMap<(String, String), JoinHandle<()>>,
    epochs: HashMap<String, tokio::sync::watch::Sender<u64>>,
    operations: HashMap<String, usize>,
}
fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Registry::default()))
}
pub fn configure(disabled: &[String]) {
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .disabled = disabled.iter().cloned().collect();
}
pub fn is_enabled(module: &str) -> bool {
    !registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .disabled
        .contains(module)
}
pub fn require_enabled(module: &str) -> Result<(), String> {
    if is_enabled(module) {
        Ok(())
    } else {
        Err("该模块已禁用，请先在设置中启用".into())
    }
}
pub fn enable(module: &str) {
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .disabled
        .remove(module);
}

fn idle_signal() -> &'static tokio::sync::Notify {
    static IDLE: OnceLock<tokio::sync::Notify> = OnceLock::new();
    IDLE.get_or_init(tokio::sync::Notify::new)
}

/// A blocking transaction holds this guard inside its worker until completion/recovery.
pub struct OperationGuard {
    module: String,
}
impl Drop for OperationGuard {
    fn drop(&mut self) {
        let mut state = registry().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(count) = state.operations.get_mut(&self.module) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                state.operations.remove(&self.module);
            }
        }
        idle_signal().notify_waiters();
    }
}

fn lease(module: &str) -> Result<(OperationGuard, tokio::sync::watch::Receiver<u64>), String> {
    let mut state = registry().lock().unwrap_or_else(|e| e.into_inner());
    if state.disabled.contains(module) {
        return Err("该模块已禁用，请先在设置中启用".into());
    }
    let receiver = state
        .epochs
        .entry(module.into())
        .or_insert_with(|| tokio::sync::watch::channel(0).0)
        .subscribe();
    *state.operations.entry(module.into()).or_default() += 1;
    Ok((
        OperationGuard {
            module: module.into(),
        },
        receiver,
    ))
}

pub fn begin_operation(module: &str) -> Result<OperationGuard, String> {
    lease(module).map(|(guard, _)| guard)
}

/// Cancel network/streaming work even when it was invoked directly through IPC rather than spawned.
pub async fn run_if_enabled<T>(
    module: &str,
    future: impl Future<Output = Result<T, String>>,
) -> Result<T, String> {
    let (_operation, mut stopped) = lease(module)?;
    tokio::select! {
        biased;
        _ = stopped.changed() => Err("模块已停用，操作已取消".into()),
        result = future => result,
    }
}

pub async fn wait_for_idle(module: &str, timeout: std::time::Duration) -> Result<(), String> {
    let wait = async {
        loop {
            let notified = idle_signal().notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if registry()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .operations
                .get(module)
                .copied()
                .unwrap_or(0)
                == 0
            {
                return;
            }
            notified.await;
        }
    };
    tokio::time::timeout(timeout, wait)
        .await
        .map_err(|_| "模块仍有操作正在安全收尾，尚未完全停止，请稍后重试".into())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleStatus {
    pub module: String,
    pub enabled: bool,
    pub tasks: usize,
    pub active_operations: usize,
}
pub fn status(module: &str) -> ModuleStatus {
    let mut state = registry().lock().unwrap_or_else(|e| e.into_inner());
    state.tasks.retain(|_, task| !task.inner().is_finished());
    ModuleStatus {
        module: module.into(),
        enabled: !state.disabled.contains(module),
        tasks: state
            .tasks
            .keys()
            .filter(|(owner, _)| owner == module)
            .count(),
        active_operations: state.operations.get(module).copied().unwrap_or(0),
    }
}
pub fn spawn_task(
    module: &str,
    name: &str,
    future: impl Future<Output = ()> + Send + 'static,
) -> bool {
    spawn_owned(module, name, future).is_some()
}
pub fn spawn_owned(
    module: &str,
    name: &str,
    future: impl Future<Output = ()> + Send + 'static,
) -> Option<tokio::task::AbortHandle> {
    let mut registry = registry().lock().unwrap_or_else(|e| e.into_inner());
    registry
        .tasks
        .retain(|_, handle| !handle.inner().is_finished());
    let key = (module.to_string(), name.to_string());
    if registry.disabled.contains(module) || registry.tasks.contains_key(&key) {
        return None;
    }
    let handle = tauri::async_runtime::spawn(future);
    let abort = handle.inner().abort_handle();
    registry.tasks.insert(key, handle);
    Some(abort)
}
pub fn track_task(module: &str, name: &str, task: tokio::task::JoinHandle<()>) -> bool {
    let mut registry = registry().lock().unwrap_or_else(|e| e.into_inner());
    registry
        .tasks
        .retain(|_, handle| !handle.inner().is_finished());
    let key = (module.to_string(), name.to_string());
    if registry.disabled.contains(module) || registry.tasks.contains_key(&key) {
        task.abort();
        return false;
    }
    registry.tasks.insert(key, JoinHandle::Tokio(task));
    true
}
pub async fn disable(module: &str) -> Result<(), String> {
    let handles = {
        let mut registry = registry().lock().unwrap_or_else(|e| e.into_inner());
        registry.disabled.insert(module.into());
        if let Some(epoch) = registry.epochs.get(module) {
            epoch.send_modify(|value| *value = value.wrapping_add(1));
        }
        let keys = registry
            .tasks
            .keys()
            .filter(|(owner, _)| owner == module)
            .cloned()
            .collect::<Vec<_>>();
        keys.into_iter()
            .filter_map(|key| registry.tasks.remove(&key).map(|handle| (key, handle)))
            .collect::<Vec<_>>()
    };
    for (_, handle) in &handles {
        handle.abort();
    }
    let mut waiting = 0;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    for (key, mut handle) in handles {
        if tokio::time::timeout_at(deadline, &mut handle)
            .await
            .is_err()
        {
            registry()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .tasks
                .insert(key, handle);
            waiting += 1;
        }
    }
    if waiting == 0 {
        Ok(())
    } else {
        Err(format!("模块仍有 {waiting} 个后台任务正在停止，请稍后重试"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn disabling_owned_tasks_does_not_abort_another_module() {
        let owner = "runtime-test-disabled";
        let other = "runtime-test-other";
        enable(owner);
        enable(other);
        let (started, ready) = tokio::sync::oneshot::channel();
        let (other_started, other_ready) = tokio::sync::oneshot::channel();
        let token = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        struct DropFlag(std::sync::Arc<std::sync::atomic::AtomicBool>);
        impl Drop for DropFlag {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let flag = token.clone();
        assert!(spawn_task(owner, "poll", async move {
            let _guard = DropFlag(flag);
            let _ = started.send(());
            std::future::pending::<()>().await;
        }));
        assert!(spawn_task(other, "poll", async move {
            let _ = other_started.send(());
            std::future::pending::<()>().await;
        }));
        ready.await.unwrap();
        other_ready.await.unwrap();
        disable(owner).await.unwrap();
        assert!(token.load(std::sync::atomic::Ordering::SeqCst));
        assert!(!spawn_task(owner, "new", async {}));
        assert!(registry()
            .lock()
            .unwrap()
            .tasks
            .contains_key(&(other.into(), "poll".into())));
        enable(owner);
        assert!(spawn_task(owner, "again", async {}));
        disable(owner).await.unwrap();
        disable(other).await.unwrap();
    }

    #[tokio::test]
    async fn direct_operations_cancel_and_do_not_revive_after_reenable() {
        let owner = "runtime-direct-cancel";
        enable(owner);
        let dropped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        struct Flag(std::sync::Arc<std::sync::atomic::AtomicBool>);
        impl Drop for Flag {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let flag = dropped.clone();
        let (started, ready) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            run_if_enabled(owner, async move {
                let _flag = Flag(flag);
                let _ = started.send(());
                std::future::pending::<Result<(), String>>().await
            })
            .await
        });
        ready.await.unwrap();
        assert_eq!(status(owner).active_operations, 1);
        disable(owner).await.unwrap();
        enable(owner);
        assert!(task.await.unwrap().is_err());
        assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(status(owner).active_operations, 0);
    }

    #[tokio::test]
    async fn critical_worker_must_finish_before_idle_can_be_confirmed() {
        let owner = "runtime-critical-worker";
        enable(owner);
        let guard = begin_operation(owner).unwrap();
        disable(owner).await.unwrap();
        assert!(begin_operation(owner).is_err());
        assert!(wait_for_idle(owner, std::time::Duration::from_millis(10))
            .await
            .is_err());
        assert_eq!(status(owner).active_operations, 1);
        drop(guard);
        wait_for_idle(owner, std::time::Duration::from_millis(50))
            .await
            .unwrap();
        assert_eq!(status(owner).active_operations, 0);
    }

    #[tokio::test]
    async fn disabled_operation_is_not_polled() {
        let owner = "runtime-not-polled";
        disable(owner).await.unwrap();
        let mut touched = false;
        assert!(run_if_enabled(owner, async {
            touched = true;
            Ok(())
        })
        .await
        .is_err());
        assert!(!touched);
    }
}
