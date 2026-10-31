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

pub(super) fn load_json<T: serde::de::DeserializeOwned>(name: &str) -> Vec<T> {
    let p = ensure_dir().join(name);
    if let Ok(s) = std::fs::read_to_string(&p) {
        if let Ok(v) = serde_json::from_str::<Vec<T>>(&s) {
            return v;
        }
    }
    Vec::new()
}

pub(super) fn load_json_checked<T: serde::de::DeserializeOwned>(name: &str) -> Result<Vec<T>, String> {
    read_json_at(&ensure_dir().join(name))
}

fn read_json_at<T: serde::de::DeserializeOwned>(path: &std::path::Path) -> Result<Vec<T>, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("读取证书配置失败 {}: {e}", path.display())),
    };
    serde_json::from_slice(&bytes).map_err(|e| format!("证书配置无效，未覆盖原文件 {}: {e}", path.display()))
}

pub(super) fn save_json_checked<T: Serialize>(name: &str, v: &[T]) -> Result<(), String> {
    let path = ensure_dir().join(name);
    let bytes = serde_json::to_vec_pretty(v).map_err(|e| e.to_string())?;
    crate::commands::foundation::atomic_write_file(&path, &bytes)
}

pub(super) fn save_json<T: Serialize>(name: &str, v: &[T]) {
    if let Err(error) = save_json_checked(name, v) {
        eprintln!("[cert] 保存配置失败: {error}");
    }
}

#[cfg(test)]
mod checked_tests {
    use super::*;
    #[test]
    fn missing_is_empty_but_malformed_or_unreadable_is_an_error() {
        let mut nonce = [0; 12]; getrandom::getrandom(&mut nonce).unwrap();
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
pub(super) fn load_certs() -> Vec<Certificate> {
    let mut certs = load_json::<Certificate>("certificates.json");
    let mut changed = false;
    for c in certs.iter_mut() {
        if c.deploy_links.is_empty() && !c.deploy_node_ids.is_empty() {
            c.deploy_links = c
                .deploy_node_ids
                .iter()
                .map(|id| DeployLink {
                    node_id: id.clone(),
                    target: String::new(),
                })
                .collect();
            changed = true;
        }
        if c.deploy_node_ids.len() != c.deploy_links.len() {
            c.deploy_node_ids = c.deploy_links.iter().map(|l| l.node_id.clone()).collect();
            changed = true;
        }
    }
    if changed {
        save_json("certificates.json", &certs);
    }
    certs
}
