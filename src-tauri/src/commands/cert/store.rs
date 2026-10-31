use super::*;

// ---------------------------------------------------------------------------
// 存储
// ---------------------------------------------------------------------------

pub(super) fn certs_dir() -> PathBuf {
    let mut p = crate::commands::config::get_data_dir();
    p.push("certs");
    p
}

pub(super) fn ensure_dir() -> PathBuf {
    let d = certs_dir();
    let _ = std::fs::create_dir_all(&d);
    d
}

pub(super) fn load_json_checked<T: serde::de::DeserializeOwned>(
    name: &str,
) -> Result<Vec<T>, String> {
    let _lock = CONFIG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    read_json_at(&ensure_dir().join(name))
}

fn read_json_at<T: serde::de::DeserializeOwned>(path: &std::path::Path) -> Result<Vec<T>, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("读取证书配置失败 {}: {e}", path.display())),
    };
    serde_json::from_slice(&bytes)
        .map_err(|e| format!("证书配置无效，未覆盖原文件 {}: {e}", path.display()))
}

fn write_json_at<T: Serialize>(path: &std::path::Path, v: &[T]) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(v).map_err(|e| e.to_string())?;
    crate::commands::foundation::atomic_write_file(path, &bytes)
}

#[cfg(test)]
mod checked_tests {
    use super::*;
    #[test]
    fn missing_is_empty_but_malformed_or_unreadable_is_an_error() {
        let mut nonce = [0; 12];
        getrandom::getrandom(&mut nonce).unwrap();
        let root = std::env::temp_dir().join(format!("cert-json-check-{}", hex(&nonce)));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("nodes.json");
        assert!(read_json_at::<serde_json::Value>(&path).unwrap().is_empty());
        std::fs::write(&path, b"invalid json").unwrap();
        assert!(read_json_at::<serde_json::Value>(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"invalid json");
        assert!(read_json_at::<serde_json::Value>(&root).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}

pub(super) fn gen_id(prefix: &str) -> String {
    // 毫秒时间戳 + 4 字节随机数：凭据/节点 id 被证书外键引用，
    // 同毫秒内连续创建（快速点击/脚本批量）不得碰撞
    let mut buf = [0u8; 4];
    let _ = getrandom::getrandom(&mut buf[..]);
    format!(
        "{}-{:x}-{:02x}{:02x}{:02x}{:02x}",
        prefix,
        Utc::now().timestamp_millis(),
        buf[0],
        buf[1],
        buf[2],
        buf[3]
    )
}

/// 读取域名列表，并把旧版 `deploy_node_ids` 自动迁移为 `deploy_links`。
///
/// 新模型：域名列表负责「关联部署方式」，部署方式本身不再保存域名；
/// 旧数据只有节点 ID，迁移时 target 留空，部署时回退到证书首个域名。
pub(super) fn load_certs_checked() -> Result<Vec<Certificate>, String> {
    let mut certs = load_json_checked::<Certificate>("certificates.json")?;
    normalize_cert_links(&mut certs);
    Ok(certs)
}

pub(super) fn normalize_cert_links(certs: &mut [Certificate]) {
    for c in certs {
        if c.deploy_links.is_empty() && !c.deploy_node_ids.is_empty() {
            c.deploy_links = c
                .deploy_node_ids
                .iter()
                .map(|id| DeployLink {
                    node_id: id.clone(),
                    target: String::new(),
                })
                .collect();
        }
        c.deploy_node_ids = c.deploy_links.iter().map(|l| l.node_id.clone()).collect();
    }
}

static ACTIVE_CERT_TASKS: std::sync::LazyLock<Mutex<std::collections::HashSet<String>>> =
    std::sync::LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

pub(super) struct CertTaskGuard(String);
impl CertTaskGuard {
    pub(super) fn acquire(id: &str) -> Result<Self, String> {
        let mut active = ACTIVE_CERT_TASKS.lock().unwrap_or_else(|e| e.into_inner());
        if !active.insert(id.into()) {
            return Err("该证书已有签发或部署任务正在运行，请稍后重试".into());
        }
        Ok(Self(id.into()))
    }
}
impl Drop for CertTaskGuard {
    fn drop(&mut self) {
        ACTIVE_CERT_TASKS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.0);
    }
}

static CONFIG_LOCK: Mutex<()> = Mutex::new(());

/// One serial checked read-modify-atomic-write. No network work is allowed in the closure.
pub(super) fn mutate_json<T, R>(
    name: &str,
    change: impl FnOnce(&mut Vec<T>) -> Result<R, String>,
) -> Result<R, String>
where
    T: serde::de::DeserializeOwned + Serialize,
{
    mutate_json_at(&ensure_dir().join(name), change)
}
fn mutate_json_at<T, R>(
    path: &std::path::Path,
    change: impl FnOnce(&mut Vec<T>) -> Result<R, String>,
) -> Result<R, String>
where
    T: serde::de::DeserializeOwned + Serialize,
{
    let _lock = CONFIG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut values = read_json_at(path)?;
    let before = serde_json::to_value(&values).map_err(|e| e.to_string())?;
    let result = change(&mut values)?;
    if serde_json::to_value(&values).map_err(|e| e.to_string())? != before {
        write_json_at(path, &values)?;
    }
    Ok(result)
}

pub(super) fn mutate_certs<R>(
    change: impl FnOnce(&mut Vec<Certificate>) -> Result<R, String>,
) -> Result<R, String> {
    mutate_json("certificates.json", |certificates| {
        normalize_cert_links(certificates);
        change(certificates)
    })
}

/// Runtime writes patch only changed fields into still-existing, unchanged configuration.
pub(super) fn merge_runtime<T>(
    name: &str,
    expected: &[T],
    updated: &[T],
    fields: &[&str],
) -> Result<Vec<T>, String>
where
    T: serde::de::DeserializeOwned + Serialize + Clone,
{
    mutate_json(name, |current: &mut Vec<T>| {
        patch_runtime(current, expected, updated, fields)?;
        Ok(current.clone())
    })
}
fn config_value<T: Serialize>(value: &T, fields: &[&str]) -> Result<serde_json::Value, String> {
    let mut value = serde_json::to_value(value).map_err(|e| e.to_string())?;
    let object = value.as_object_mut().ok_or("配置记录必须为对象")?;
    for field in fields {
        object.remove(*field);
    }
    Ok(value)
}
fn find_id<'a, T: Serialize>(values: &'a [T], id: &str) -> Option<&'a T> {
    values.iter().find(|value| {
        serde_json::to_value(value)
            .ok()
            .and_then(|value| {
                value
                    .get("id")
                    .and_then(|id| id.as_str())
                    .map(str::to_string)
            })
            .as_deref()
            == Some(id)
    })
}
fn patch_runtime<T>(
    current: &mut [T],
    expected: &[T],
    updated: &[T],
    fields: &[&str],
) -> Result<(), String>
where
    T: serde::de::DeserializeOwned + Serialize,
{
    for value in current {
        let mut stored = serde_json::to_value(&*value).map_err(|e| e.to_string())?;
        let id = stored
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or("配置记录缺少id")?
            .to_string();
        let (Some(before), Some(after)) = (find_id(expected, &id), find_id(updated, &id)) else {
            continue;
        };
        if config_value(&*value, fields)? != config_value(before, fields)? {
            continue;
        }
        let before = serde_json::to_value(before).map_err(|e| e.to_string())?;
        let after = serde_json::to_value(after).map_err(|e| e.to_string())?;
        if fields.iter().any(|field| {
            stored.get(*field) != before.get(*field) && stored.get(*field) != after.get(*field)
        }) {
            continue;
        }
        let object = stored.as_object_mut().ok_or("配置记录必须为对象")?;
        for field in fields {
            if before.get(*field) != after.get(*field) && object.get(*field) == before.get(*field) {
                if let Some(updated) = after.get(*field) {
                    object.insert((*field).into(), updated.clone());
                }
            }
        }
        *value = serde_json::from_value(stored).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(super) const CERT_RUNTIME: &[&str] = &[
    "not_before",
    "not_after",
    "last_issue_at",
    "status",
    "last_error",
];
pub(super) const NODE_RUNTIME: &[&str] = &["last_deploy_at", "last_deploy_error"];
pub(super) const MONITOR_RUNTIME: &[&str] = &[
    "not_after",
    "days_left",
    "subject",
    "issuer",
    "last_check_at",
    "last_error",
    "last_notify_bucket",
    "last_notified_at",
];

pub(super) fn merge_certificate_state(
    expected: &Certificate,
    updated: &Certificate,
) -> Result<Certificate, String> {
    mutate_certs(|current| {
        let stored = current
            .iter()
            .find(|c| c.id == expected.id)
            .ok_or("证书已删除，旧任务结果未回写")?;
        if config_value(stored, CERT_RUNTIME)? != config_value(expected, CERT_RUNTIME)? {
            return Err("证书配置已变化，旧任务结果未回写，请使用当前配置重试".into());
        }
        patch_runtime(
            current,
            std::slice::from_ref(expected),
            std::slice::from_ref(updated),
            CERT_RUNTIME,
        )?;
        Ok(current
            .iter()
            .find(|c| c.id == expected.id)
            .unwrap()
            .clone())
    })
}

pub(super) fn delete_node_and_unlink(id: &str) -> Result<(), String> {
    delete_node_at(&ensure_dir(), id)
}
fn delete_node_at(root: &std::path::Path, id: &str) -> Result<(), String> {
    let _lock = CONFIG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let cert_path = root.join("certificates.json");
    let node_path = root.join("deploy_nodes.json");
    let mut certificates = read_json_at::<Certificate>(&cert_path)?;
    let mut nodes = read_json_at::<DeployNode>(&node_path)?;
    normalize_cert_links(&mut certificates);
    nodes.retain(|node| node.id != id);
    for cert in &mut certificates {
        cert.deploy_links.retain(|link| link.node_id != id);
        cert.deploy_node_ids.retain(|node| node != id);
    }
    let previous = match std::fs::read(&cert_path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.to_string()),
    };
    write_json_at(&cert_path, &certificates)?;
    if let Err(error) = write_json_at(&node_path, &nodes) {
        let restored = match previous {
            Some(bytes) => crate::commands::foundation::atomic_write_file(&cert_path, &bytes),
            None => std::fs::remove_file(&cert_path).map_err(|e| e.to_string()),
        };
        return Err(match restored {
            Ok(()) => format!("节点删除失败，证书关联已回滚: {error}"),
            Err(rollback) => format!("节点删除失败: {error}；关联回滚未确认: {rollback}"),
        });
    }
    Ok(())
}

#[cfg(test)]
#[path = "store/concurrency_tests.rs"]
mod concurrency_tests;
