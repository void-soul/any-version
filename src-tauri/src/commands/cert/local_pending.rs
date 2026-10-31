//! Durable local deployment intent; audit history is never a retry queue.
use super::*;
use std::path::Path;

static PENDING_LOCK: Mutex<()> = Mutex::new(());
const FILE: &str = "local_deploy_pending.json";

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub(super) struct PendingLocal {
    pub cert_id: String,
    pub node_id: String,
    pub target: String,
    pub generation: String,
}

fn read_at(path: &Path) -> Result<Vec<PendingLocal>, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(error) => return Err(format!("读取本机待部署队列失败，未覆盖原文件: {error}")),
    };
    let pending: Vec<PendingLocal> = serde_json::from_slice(&bytes)
        .map_err(|error| format!("本机待部署队列损坏，已停止部署且未覆盖原文件: {error}"))?;
    let mut keys = std::collections::HashSet::new();
    for entry in &pending {
        if entry.cert_id.trim().is_empty()
            || entry.node_id.trim().is_empty()
            || entry.target != entry.target.trim()
            || entry.generation.len() != 32
            || !entry
                .generation
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || !keys.insert((&entry.cert_id, &entry.node_id, &entry.target))
        {
            return Err("本机待部署队列内容无效或重复，已停止部署且未覆盖原文件".into());
        }
    }
    Ok(pending)
}
fn write_at(path: &Path, pending: &[PendingLocal]) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(pending).map_err(|e| e.to_string())?;
    crate::commands::foundation::atomic_write_file(path, &bytes)
}
fn register_at(path: &Path, cert_id: &str, node_id: &str, target: &str) -> Result<String, String> {
    let mut pending = read_at(path)?;
    let mut random = [0; 16];
    getrandom::getrandom(&mut random).map_err(|e| e.to_string())?;
    let generation = hex(&random);
    let entry = PendingLocal {
        cert_id: cert_id.into(),
        node_id: node_id.into(),
        target: target.trim().into(),
        generation: generation.clone(),
    };
    if entry.cert_id.trim().is_empty() || entry.node_id.trim().is_empty() {
        return Err("本机待部署任务缺少证书或节点 ID".into());
    }
    if let Some(old) = pending.iter_mut().find(|old| same_key(old, &entry)) {
        *old = entry;
    } else {
        pending.push(entry);
    }
    write_at(path, &pending)?;
    Ok(generation)
}
fn same_key(left: &PendingLocal, right: &PendingLocal) -> bool {
    left.cert_id == right.cert_id && left.node_id == right.node_id && left.target == right.target
}
fn complete_at(
    path: &Path,
    cert_id: &str,
    node_id: &str,
    target: &str,
    generation: &str,
) -> Result<(), String> {
    let mut pending = read_at(path)?;
    let count = pending.len();
    pending.retain(|entry| {
        !(entry.cert_id == cert_id
            && entry.node_id == node_id
            && entry.target == target.trim()
            && entry.generation == generation)
    });
    if pending.len() != count {
        write_at(path, &pending)?;
    }
    Ok(())
}

pub(super) fn snapshot() -> Result<Vec<PendingLocal>, String> {
    let _lock = PENDING_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    read_at(&ensure_dir().join(FILE))
}
pub(super) fn begin(cert_id: &str, node_id: &str, target: &str) -> Result<String, String> {
    let _lock = PENDING_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    register_at(&ensure_dir().join(FILE), cert_id, node_id, target)
}
pub(super) fn complete(
    cert_id: &str,
    node_id: &str,
    target: &str,
    generation: &str,
) -> Result<(), String> {
    let _lock = PENDING_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    complete_at(
        &ensure_dir().join(FILE),
        cert_id,
        node_id,
        target,
        generation,
    )
}

pub(super) async fn run_registered(
    cert_id: &str,
    node_id: &str,
    target: &str,
    operation: impl std::future::Future<Output = Result<(), String>>,
) -> Result<(), String> {
    run_with(
        || begin(cert_id, node_id, target),
        |generation| complete(cert_id, node_id, target, generation),
        operation,
    )
    .await
}

async fn run_with(
    register: impl FnOnce() -> Result<String, String>,
    complete: impl FnOnce(&str) -> Result<(), String>,
    operation: impl std::future::Future<Output = Result<(), String>>,
) -> Result<(), String> {
    let generation =
        register().map_err(|error| format!("无法登记本机待部署任务，已停止部署: {error}"))?;
    operation.await?;
    complete(&generation)
        .map_err(|error| format!("本机部署已执行，但待部署状态清理失败，将保留重试: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Root(PathBuf);
    impl Root {
        fn new() -> Self {
            let mut nonce = [0; 12];
            getrandom::getrandom(&mut nonce).unwrap();
            let path = std::env::temp_dir().join(format!("cert-local-pending-{}", hex(&nonce)));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn queue(&self) -> PathBuf {
            self.0.join(FILE)
        }
    }
    impl Drop for Root {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn pending_survives_history_deletion_and_process_restart_read() {
        let root = Root::new();
        let generation = register_at(&root.queue(), "cert", "node", "target").unwrap();
        let history = root.0.join("deploy_history.json");
        std::fs::write(&history, b"[]").unwrap();
        std::fs::remove_file(history).unwrap();
        let restored = read_at(&root.queue()).unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].generation, generation);
    }
    #[test]
    fn success_removes_only_the_completed_generation() {
        let root = Root::new();
        let first = register_at(&root.queue(), "cert", "node", "target").unwrap();
        let second = register_at(&root.queue(), "cert", "node", "target").unwrap();
        complete_at(&root.queue(), "cert", "node", "target", &first).unwrap();
        assert_eq!(read_at(&root.queue()).unwrap()[0].generation, second);
        complete_at(&root.queue(), "cert", "node", "target", &second).unwrap();
        assert!(read_at(&root.queue()).unwrap().is_empty());
    }
    #[test]
    fn corrupt_queue_is_not_replaced_or_treated_as_empty() {
        let root = Root::new();
        let bytes = b"{invalid json";
        std::fs::write(root.queue(), bytes).unwrap();
        assert!(read_at(&root.queue()).is_err());
        assert!(register_at(&root.queue(), "cert", "node", "target").is_err());
        assert!(complete_at(&root.queue(), "cert", "node", "target", "gen").is_err());
        assert_eq!(std::fs::read(root.queue()).unwrap(), bytes);
    }
    #[test]
    fn unreadable_queue_does_not_replace_an_existing_intent() {
        let root = Root::new();
        let old = register_at(&root.queue(), "cert", "node", "target").unwrap();
        // An unreadable destination cannot be interpreted as an empty queue.
        let directory = root.0.join("directory");
        std::fs::create_dir(&directory).unwrap();
        assert!(register_at(&directory, "cert", "node", "target").is_err());
        assert_eq!(read_at(&root.queue()).unwrap()[0].generation, old);
    }
    #[tokio::test]
    async fn broken_queue_prevents_deployment_operation_from_running() {
        let root = Root::new();
        std::fs::write(root.queue(), b"broken").unwrap();
        let invoked = std::cell::Cell::new(false);
        let result = run_with(
            || register_at(&root.queue(), "cert", "node", "target"),
            |generation| complete_at(&root.queue(), "cert", "node", "target", generation),
            async {
                invoked.set(true);
                Ok(())
            },
        )
        .await;
        assert!(result.is_err());
        assert!(!invoked.get());
        assert_eq!(std::fs::read(root.queue()).unwrap(), b"broken");
    }
    #[tokio::test]
    async fn failed_deployment_keeps_registered_intent() {
        let root = Root::new();
        let result = run_with(
            || register_at(&root.queue(), "cert", "node", "target"),
            |generation| complete_at(&root.queue(), "cert", "node", "target", generation),
            async { Err("fake reload failed".into()) },
        )
        .await;
        assert_eq!(result.unwrap_err(), "fake reload failed");
        assert_eq!(read_at(&root.queue()).unwrap().len(), 1);
    }
}
