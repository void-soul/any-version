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

pub(super) fn save_json<T: Serialize>(name: &str, v: &[T]) {
    let p = ensure_dir().join(name);
    if let Ok(s) = serde_json::to_string_pretty(v) {
        let _ = std::fs::write(&p, s);
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
