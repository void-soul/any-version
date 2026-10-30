//! App-lifetime task ownership; no business or storage dependencies.
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Mutex, OnceLock};
use tauri::async_runtime::JoinHandle;

#[derive(Default)]
struct Registry {
    disabled: HashSet<String>,
    tasks: HashMap<(String, String), JoinHandle<()>>,
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
pub async fn disable(module: &str) {
    let handles = {
        let mut registry = registry().lock().unwrap_or_else(|e| e.into_inner());
        registry.disabled.insert(module.into());
        let keys = registry
            .tasks
            .keys()
            .filter(|(owner, _)| owner == module)
            .cloned()
            .collect::<Vec<_>>();
        keys.into_iter()
            .filter_map(|key| registry.tasks.remove(&key))
            .collect::<Vec<_>>()
    };
    for handle in &handles {
        handle.abort();
    }
    for handle in handles {
        let _ = handle.await;
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
        disable(owner).await;
        assert!(token.load(std::sync::atomic::Ordering::SeqCst));
        assert!(!spawn_task(owner, "new", async {}));
        assert!(registry()
            .lock()
            .unwrap()
            .tasks
            .contains_key(&(other.into(), "poll".into())));
        enable(owner);
        assert!(spawn_task(owner, "again", async {}));
        disable(owner).await;
        disable(other).await;
    }
}
