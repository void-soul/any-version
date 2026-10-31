//! Read a complete, validated generation while holding the deployment lock.
use std::path::{Path, PathBuf};
#[derive(Debug, PartialEq)]
pub enum ErrorKind {
    NotFound,
    Busy,
    Invalid,
    Internal,
}
#[derive(Debug)]
pub struct ReadError {
    pub kind: ErrorKind,
    pub message: String,
}
fn error(kind: ErrorKind, message: impl Into<String>) -> ReadError {
    ReadError {
        kind,
        message: message.into(),
    }
}
pub struct Certificate {
    pub key: String,
    pub full_chain: String,
    pub expired_time: String,
}
fn regular(path: &Path, directory: bool) -> Result<(), ReadError> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| {
        error(
            ErrorKind::Internal,
            format!("读取证书路径 {} 失败: {e}", path.display()),
        )
    })?;
    #[cfg(windows)]
    let linked = {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let linked = meta.file_type().is_symlink();
    if linked || (directory && !meta.is_dir()) || (!directory && !meta.is_file()) {
        return Err(error(
            ErrorKind::Internal,
            "证书路径必须是普通文件或目录，不能是链接",
        ));
    }
    Ok(())
}
fn read(path: &Path) -> Result<String, ReadError> {
    regular(path, false)?;
    std::fs::read_to_string(path).map_err(|e| {
        error(
            ErrorKind::Internal,
            format!("读取证书文件 {} 失败: {e}", path.display()),
        )
    })
}
fn metadata(dir: &Path) -> Result<serde_json::Value, ReadError> {
    serde_json::from_str(&read(&dir.join("meta.json"))?)
        .map_err(|e| error(ErrorKind::Internal, format!("证书元数据无效: {e}")))
}
fn find(root: &Path, id: &str) -> Result<PathBuf, ReadError> {
    let direct = root.join(crate::protocol::safe_name(id));
    match std::fs::symlink_metadata(&direct) {
        Ok(_) => {
            regular(&direct, true)?;
            return Ok(direct);
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(error(ErrorKind::Internal, e.to_string())),
    }
    for entry in std::fs::read_dir(root).map_err(|e| error(ErrorKind::Internal, e.to_string()))? {
        let entry = entry.map_err(|e| error(ErrorKind::Internal, e.to_string()))?;
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let kind = entry
            .file_type()
            .map_err(|e| error(ErrorKind::Internal, e.to_string()))?;
        if !kind.is_dir() || kind.is_symlink() {
            continue;
        }
        let dir = entry.path();
        regular(&dir, true)?;
        match std::fs::symlink_metadata(dir.join("meta.json")) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(error(ErrorKind::Internal, e.to_string())),
        }
        if metadata(&dir)?
            .get("certificate_id")
            .and_then(|v| v.as_str())
            == Some(id)
        {
            return Ok(dir);
        }
    }
    Err(error(ErrorKind::NotFound, format!("证书 {id} 不存在")))
}
pub fn read_certificate(root: &Path, id: &str) -> Result<Certificate, ReadError> {
    if id.trim().is_empty() {
        return Err(error(ErrorKind::Invalid, "证书 ID 不能为空"));
    }
    match std::fs::metadata(root) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(error(ErrorKind::NotFound, "证书目录不存在"))
        }
        Err(e) => return Err(error(ErrorKind::Internal, e.to_string())),
        Ok(_) => {}
    }
    let _lock =
        crate::deployment::OutputLock::try_acquire(root).map_err(|e| error(ErrorKind::Busy, e))?;
    let dir = find(root, id)?;
    let key = read(&dir.join("cert.key"))?;
    let full_chain = read(&dir.join("fullchain.cer"))?;
    // Every directory layout writes chain.cer, even when no separate CA is supplied.
    // Read it to surface missing files and access errors; an empty chain is valid.
    let _chain = read(&dir.join("chain.cer"))?;
    let meta = metadata(&dir)?;
    crate::deployment::validate_identity(&full_chain, &key)
        .map_err(|e| error(ErrorKind::Internal, e))?;
    Ok(Certificate {
        key,
        full_chain,
        expired_time: meta
            .get("expired_time")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Root(PathBuf);
    impl Root {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("receiver-read-{}", crate::random_hex(12)));
            let dir = root.join("test-id");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("cert.key"),
                include_str!("../tests/fixtures/identity-key.pem"),
            )
            .unwrap();
            std::fs::write(
                dir.join("fullchain.cer"),
                include_str!("../tests/fixtures/identity-cert.pem"),
            )
            .unwrap();
            std::fs::write(dir.join("chain.cer"), "").unwrap();
            std::fs::write(
                dir.join("meta.json"),
                r#"{"certificate_id":"test-id","expired_time":"test-expiry"}"#,
            )
            .unwrap();
            Self(root)
        }
        fn read(&self) -> Result<Certificate, ReadError> {
            read_certificate(&self.0, "test-id")
        }
    }
    impl Drop for Root {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn matching_generation_with_empty_chain_is_readable() {
        let root = Root::new();
        let cert = root.read().unwrap();
        assert!(cert.key.contains("PRIVATE KEY"));
        assert!(cert.full_chain.contains("CERTIFICATE"));
        assert_eq!(cert.expired_time, "test-expiry");
    }
    #[test]
    fn missing_required_files_never_produce_a_successful_payload() {
        for name in ["cert.key", "fullchain.cer", "chain.cer", "meta.json"] {
            let root = Root::new();
            std::fs::remove_file(root.0.join("test-id").join(name)).unwrap();
            assert_eq!(
                root.read().err().unwrap().kind,
                ErrorKind::Internal,
                "{name}"
            );
        }
    }
    #[test]
    fn mismatched_or_unreadable_identity_is_rejected() {
        let root = Root::new();
        std::fs::write(
            root.0.join("test-id/cert.key"),
            include_str!("../tests/fixtures/mismatch-key.pem"),
        )
        .unwrap();
        assert_eq!(root.read().err().unwrap().kind, ErrorKind::Internal);
        std::fs::remove_file(root.0.join("test-id/cert.key")).unwrap();
        std::fs::create_dir(root.0.join("test-id/cert.key")).unwrap();
        assert_eq!(root.read().err().unwrap().kind, ErrorKind::Internal);
    }
    #[test]
    fn reads_do_not_cross_an_active_deployment() {
        let root = Root::new();
        let lock = crate::deployment::OutputLock::try_acquire(&root.0).unwrap();
        assert_eq!(root.read().err().unwrap().kind, ErrorKind::Busy);
        drop(lock);
        assert!(root.read().is_ok());
    }
    #[test]
    fn domain_layout_is_found_by_metadata_and_absent_ids_are_not_found() {
        let root = Root::new();
        std::fs::rename(root.0.join("test-id"), root.0.join("test.invalid")).unwrap();
        assert!(root.read().is_ok());
        assert_eq!(
            read_certificate(&root.0, "absent").err().unwrap().kind,
            ErrorKind::NotFound
        );
    }
}
