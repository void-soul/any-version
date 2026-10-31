//! Stage complete certificate generations, then install with rollback. No live
//! certificate/private-key pair is overwritten one file at a time.
use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
use std::path::{Path, PathBuf};

pub fn validate_identity(cert: &str, key: &str) -> Result<(), String> {
    let certs = CertificateDer::pem_slice_iter(cert.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("证书PEM格式无效: {e}"))?;
    if certs.is_empty() {
        return Err("证书内容为空或未包含PEM证书".into());
    }
    let key = PrivateKeyDer::from_pem_slice(key.as_bytes())
        .map_err(|e| format!("私钥PEM格式无效: {e}"))?;
    let identity =
        rustls::sign::CertifiedKey::from_der(certs, key, &rustls::crypto::ring::default_provider())
            .map_err(|e| format!("证书与私钥无法使用或不匹配: {e}"))?;
    identity
        .keys_match()
        .map_err(|e| format!("证书与私钥不匹配: {e}"))
}

struct Operation {
    target: PathBuf,
    staged: PathBuf,
    backup: PathBuf,
    installed: bool,
    saved_old: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Journal {
    committed: bool,
    targets: Vec<(String, bool)>,
}

fn write_journal(staging: &Path, journal: &Journal) -> Result<(), String> {
    use std::io::Write;
    let bytes = serde_json::to_vec(journal).map_err(|e| e.to_string())?;
    let temp = staging.join("journal.tmp");
    let mut file = std::fs::File::create(&temp).map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    std::fs::rename(temp, staging.join("journal.json")).map_err(|e| e.to_string())
}

fn recover_pending(root: &Path) -> Result<(), String> {
    for entry in std::fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with(".receiver-stage-")
        {
            continue;
        }
        let staging = entry.path();
        if is_link(&staging)? || !staging.is_dir() {
            return Err("遗留证书暂存路径无效，未继续部署".into());
        }
        let journal = staging.join("journal.json");
        if journal.exists() {
            let journal: Journal =
                serde_json::from_slice(&std::fs::read(&journal).map_err(|e| e.to_string())?)
                    .map_err(|e| format!("无法读取遗留部署计划 {}: {e}", staging.display()))?;
            if !journal.committed {
                for (index, (name, had_old)) in journal.targets.iter().enumerate().rev() {
                    validate_name(name)?;
                    let target = root.join(name);
                    let backup = staging.join(format!("old-{index}"));
                    let prepared = staging.join(format!("new-{index}"));
                    if backup.exists() || (!had_old && !prepared.exists()) {
                        if target.exists() {
                            if is_link(&target)? {
                                return Err("遗留部署目标已变为链接，未继续恢复".into());
                            }
                            std::fs::rename(&target, staging.join(format!("interrupted-{index}")))
                                .map_err(|e| e.to_string())?;
                        }
                        if backup.exists() {
                            std::fs::rename(backup, target).map_err(|e| e.to_string())?;
                        }
                    }
                }
            }
        }
        std::fs::remove_dir_all(staging).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn recover(root: &Path) -> Result<(), String> {
    if !root.exists() {
        return Ok(());
    }
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let _lock = OutputLock::try_acquire(&root)?;
    recover_pending(&root)
}

pub struct Deployment {
    root: PathBuf,
    staging: PathBuf,
    operations: Vec<Operation>,
    finished: bool,
    preserve: bool,
    _output_lock: OutputLock,
}

/// OS file lock shared by the embedded and standalone receiver processes.
pub struct OutputLock {
    _file: std::fs::File,
}
impl OutputLock {
    pub fn try_acquire(root: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        let path = root.join(".receiver-deployment.lock");
        match std::fs::symlink_metadata(&path) {
            Ok(_) if is_link(&path)? => return Err("证书目录锁不能是链接（含悬空链接）".into()),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("无法检查证书目录锁: {error}")),
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // Open the reparse point itself and keep the active lock file from being replaced.
            options.custom_flags(0x00200000).share_mode(0x1 | 0x2);
        }
        let file = options.open(&path).map_err(|e| e.to_string())?;
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("证书目录锁不能是链接".into());
            }
        }
        if !metadata.is_file() {
            return Err("证书目录锁必须是普通文件".into());
        }
        fs2::FileExt::try_lock_exclusive(&file)
            .map_err(|e| format!("其它接收端正在使用此证书目录，请稍后重试: {e}"))?;
        Ok(Self { _file: file })
    }
}

fn is_link(path: &Path) -> Result<bool, String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        return Ok(metadata.file_attributes() & 0x400 != 0);
    }
    #[cfg(not(windows))]
    {
        Ok(metadata.file_type().is_symlink())
    }
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.starts_with(".receiver-")
        || name.contains(['/', '\\', ':', '\0'])
        || name == "."
        || name == ".."
        || name.ends_with(['.', ' '])
    {
        return Err("证书目标名称无效".into());
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
    {
        return Err("证书目标名称是系统保留名称".into());
    }
    Ok(())
}

fn create_private_dir(path: &Path, recursive: bool) -> Result<(), String> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(recursive);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn private_file(path: &Path) -> Result<std::fs::File, String> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let output = options.open(path).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Existing files copied from the previous generation may have broader permissions.
        output
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    Ok(output)
}

fn names_alias(left: &str, right: &str, windows: bool) -> bool {
    if windows {
        left.to_uppercase() == right.to_uppercase()
    } else {
        left == right
    }
}

fn copy_tree(source: &Path, dest: &Path, depth: usize, count: &mut usize) -> Result<(), String> {
    if depth > 8 {
        return Err("证书目录层级过深，未修改现有证书".into());
    }
    create_private_dir(dest, true)?;
    for entry in std::fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        *count += 1;
        if *count > 1000 {
            return Err("证书目录文件过多，未修改现有证书".into());
        }
        if is_link(&entry.path())? {
            return Err("证书目录含链接，未修改现有证书".into());
        }
        let target = dest.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target, depth + 1, count)?;
        } else {
            std::fs::copy(entry.path(), target).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

impl Deployment {
    pub fn new(root: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        let output_lock = OutputLock::try_acquire(&root)?;
        recover_pending(&root)?;
        let staging = root.join(format!(".receiver-stage-{}", crate::random_hex(16)));
        create_private_dir(&staging, false)?;
        Ok(Self {
            root,
            staging,
            operations: vec![],
            finished: false,
            preserve: false,
            _output_lock: output_lock,
        })
    }

    fn add(&mut self, name: &str, staged: PathBuf) -> Result<(), String> {
        validate_name(name)?;
        let target = self.root.join(name);
        if self.operations.iter().any(|op| {
            names_alias(
                op.target.file_name().unwrap().to_str().unwrap(),
                name,
                cfg!(windows),
            )
        }) {
            return Err("证书目标名称重复".into());
        }
        if target.exists() && is_link(&target)? {
            return Err("证书目标不能是链接".into());
        }
        let backup = self.staging.join(format!("old-{}", self.operations.len()));
        self.operations.push(Operation {
            target,
            staged,
            backup,
            installed: false,
            saved_old: false,
        });
        Ok(())
    }

    pub fn directory(&mut self, name: &str, files: Vec<(&str, String)>) -> Result<(), String> {
        validate_name(name)?;
        let target = self.root.join(name);
        let staged = self.staging.join(format!("new-{}", self.operations.len()));
        if target.exists() {
            if is_link(&target)? || !target.is_dir() {
                return Err("证书目标不是普通目录".into());
            }
            copy_tree(&target, &staged, 0, &mut 0)?;
        } else {
            create_private_dir(&staged, false)?;
        }
        let mut names = Vec::new();
        for (file, content) in files {
            if names
                .iter()
                .any(|name: &&str| names_alias(name, file, cfg!(windows)))
            {
                return Err("证书目录内文件名称重复".into());
            }
            names.push(file);
            validate_name(file)?;
            let path = staged.join(file);
            let mut output = private_file(&path)?;
            use std::io::Write;
            output
                .write_all(content.as_bytes())
                .and_then(|_| output.sync_all())
                .map_err(|e| e.to_string())?;
        }
        self.add(name, staged)
    }

    pub fn file(&mut self, name: &str, content: &str) -> Result<(), String> {
        let staged = self.staging.join(format!("new-{}", self.operations.len()));
        let mut output = private_file(&staged)?;
        use std::io::Write;
        output
            .write_all(content.as_bytes())
            .and_then(|_| output.sync_all())
            .map_err(|e| e.to_string())?;
        self.add(name, staged)
    }

    pub fn install(&mut self) -> Result<(), String> {
        let journal = Journal {
            committed: false,
            targets: self
                .operations
                .iter()
                .map(|op| {
                    (
                        op.target.file_name().unwrap().to_string_lossy().to_string(),
                        op.target.exists(),
                    )
                })
                .collect(),
        };
        write_journal(&self.staging, &journal)?;
        for index in 0..self.operations.len() {
            let op = &mut self.operations[index];
            let result: Result<(), String> = (|| {
                if op.target.exists() {
                    if is_link(&op.target)? {
                        return Err("证书目标已变为链接".into());
                    }
                    std::fs::rename(&op.target, &op.backup).map_err(|e| e.to_string())?;
                    op.saved_old = true;
                }
                std::fs::rename(&op.staged, &op.target).map_err(|e| e.to_string())?;
                op.installed = true;
                Ok(())
            })();
            if let Err(error) = result {
                return match self.rollback() {
                    Ok(()) => Err(format!("部署未完成，已恢复原文件: {error}")),
                    Err(rollback) => Err(format!("部署失败: {error}; {rollback}")),
                };
            }
        }
        Ok(())
    }

    pub fn rollback(&mut self) -> Result<(), String> {
        let mut errors = vec![];
        for (index, op) in self.operations.iter_mut().enumerate().rev() {
            if op.installed {
                if let Err(e) =
                    std::fs::rename(&op.target, self.staging.join(format!("failed-{index}")))
                {
                    errors.push(e.to_string());
                    continue;
                }
                op.installed = false;
            }
            if op.saved_old {
                match std::fs::rename(&op.backup, &op.target) {
                    Ok(()) => op.saved_old = false,
                    Err(e) => errors.push(e.to_string()),
                }
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            self.preserve = true;
            Err(format!(
                "回滚未完成，恢复文件保留在 {}: {}",
                self.staging.display(),
                errors.join("; ")
            ))
        }
    }

    pub fn finish(mut self) -> Result<(), String> {
        self.commit_journal()
    }

    pub(crate) fn commit_journal(&mut self) -> Result<(), String> {
        let journal = Journal {
            committed: true,
            targets: self
                .operations
                .iter()
                .map(|op| {
                    (
                        op.target.file_name().unwrap().to_string_lossy().to_string(),
                        op.saved_old,
                    )
                })
                .collect(),
        };
        write_journal(&self.staging, &journal)?;
        self.finished = true;
        Ok(())
    }
}

impl Drop for Deployment {
    fn drop(&mut self) {
        if !self.finished && !self.preserve {
            let _ = self.rollback();
        }
        if !self.preserve {
            let _ = std::fs::remove_dir_all(&self.staging);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_same_output_directory_cannot_have_two_deployers() {
        let root = Root::new();
        let first = OutputLock::try_acquire(&root.0).unwrap();
        assert!(OutputLock::try_acquire(&root.0).is_err());
        drop(first);
        assert!(OutputLock::try_acquire(&root.0).is_ok());
    }
    #[test]
    fn child_process_cannot_acquire_an_active_output_lock() {
        let root = Root::new();
        let _lock = OutputLock::try_acquire(&root.0).unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child
            .args([
                "--exact",
                "deployment::tests::output_lock_child_probe",
                "--nocapture",
            ])
            .env("CERT_DEPLOYMENT_LOCK_PROBE", &root.0);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            child.creation_flags(0x08000000);
        }
        let output = child.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
    #[test]
    fn output_lock_child_probe() {
        if let Some(path) = std::env::var_os("CERT_DEPLOYMENT_LOCK_PROBE") {
            assert!(OutputLock::try_acquire(Path::new(&path)).is_err());
        }
    }
    struct Root(PathBuf);
    impl Root {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("receiver-deploy-{}", crate::random_hex(12)));
            std::fs::create_dir(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Root {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn partial_install_failure_restores_previous_generation() {
        let root = Root::new();
        std::fs::create_dir(root.0.join("example.com")).unwrap();
        std::fs::write(root.0.join("example.com/cert.key"), "old key").unwrap();
        std::fs::write(root.0.join("example.com/fullchain.cer"), "old cert").unwrap();
        let mut deploy = Deployment::new(&root.0).unwrap();
        deploy
            .directory(
                "example.com",
                vec![
                    ("cert.key", "new key".into()),
                    ("fullchain.cer", "new cert".into()),
                ],
            )
            .unwrap();
        deploy.file("second.crt", "new").unwrap();
        std::fs::remove_file(&deploy.operations[1].staged).unwrap();
        assert!(deploy.install().is_err());
        assert_eq!(
            std::fs::read_to_string(root.0.join("example.com/cert.key")).unwrap(),
            "old key"
        );
        assert_eq!(
            std::fs::read_to_string(root.0.join("example.com/fullchain.cer")).unwrap(),
            "old cert"
        );
    }
    #[test]
    fn rollback_restores_both_files_after_reload_failure() {
        let root = Root::new();
        std::fs::write(root.0.join("old.crt"), "old cert").unwrap();
        std::fs::write(root.0.join("old.key"), "old key").unwrap();
        let mut deploy = Deployment::new(&root.0).unwrap();
        deploy.file("old.crt", "new cert").unwrap();
        deploy.file("old.key", "new key").unwrap();
        deploy.install().unwrap();
        deploy.rollback().unwrap();
        assert_eq!(
            std::fs::read_to_string(root.0.join("old.crt")).unwrap(),
            "old cert"
        );
        assert_eq!(
            std::fs::read_to_string(root.0.join("old.key")).unwrap(),
            "old key"
        );
    }
    #[test]
    fn malformed_identity_is_rejected_before_deployment() {
        assert!(validate_identity("not a certificate", "not a key").is_err());
        assert!(validate_name("NUL.key").is_err());
        assert!(validate_name("../outside").is_err());
    }
    #[test]
    fn valid_test_identity_matches_and_unrelated_key_is_rejected() {
        // Public, generated fixtures for audit-test.invalid; never deployment credentials.
        let cert = include_str!("../tests/fixtures/identity-cert.pem");
        let key = include_str!("../tests/fixtures/identity-key.pem");
        let wrong = include_str!("../tests/fixtures/mismatch-key.pem");
        assert!(validate_identity(cert, key).is_ok());
        assert!(validate_identity(cert, wrong).is_err());
    }
    #[test]
    fn interrupted_install_is_recovered_before_next_deployment() {
        let root = Root::new();
        std::fs::write(root.0.join("certificate.crt"), "old cert").unwrap();
        let mut deploy = Deployment::new(&root.0).unwrap();
        deploy.file("certificate.crt", "new cert").unwrap();
        deploy.install().unwrap();
        // Simulate process death: keep on-disk journal without running rollback.
        let staging = deploy.staging.clone();
        deploy.preserve = true;
        drop(deploy);
        assert_eq!(
            std::fs::read_to_string(root.0.join("certificate.crt")).unwrap(),
            "new cert"
        );
        let next = Deployment::new(&root.0).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.0.join("certificate.crt")).unwrap(),
            "old cert"
        );
        assert!(!staging.exists());
        drop(next);
    }
    #[test]
    fn windows_name_aliases_are_detected_before_live_installation() {
        assert!(names_alias("example.com.crt", "EXAMPLE.COM.CRT", true));
        assert!(!names_alias("example.com.crt", "EXAMPLE.COM.CRT", false));
        assert!(validate_name(".receiver-deployment.lock").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn flat_file_directory_case_alias_is_rejected_before_install() {
        let root = Root::new();
        let mut deployment = Deployment::new(&root.0).unwrap();
        deployment
            .file("example.com.crt", "fake certificate")
            .unwrap();
        assert!(deployment.directory("EXAMPLE.COM.CRT", vec![]).is_err());
        assert!(!root.0.join("example.com.crt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn dangling_lock_symlink_does_not_create_its_external_target() {
        let root = Root::new();
        let output = root.0.join("output");
        std::fs::create_dir(&output).unwrap();
        let unrelated = root.0.join("must-not-create");
        std::os::unix::fs::symlink(&unrelated, output.join(".receiver-deployment.lock")).unwrap();
        assert!(OutputLock::try_acquire(&output).is_err());
        assert!(!unrelated.exists());
    }

    #[cfg(unix)]
    #[test]
    fn staged_directories_and_flat_private_keys_have_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let root = Root::new();
        for old_mode in [0o600, 0o644] {
            let key = root.0.join("example.key");
            std::fs::write(&key, "fake old key").unwrap();
            std::fs::set_permissions(&key, std::fs::Permissions::from_mode(old_mode)).unwrap();
            let mut deployment = Deployment::new(&root.0).unwrap();
            assert_eq!(
                std::fs::metadata(&deployment.staging)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            deployment.file("example.key", "fake new key").unwrap();
            deployment.install().unwrap();
            deployment.finish().unwrap();
            assert_eq!(
                std::fs::metadata(&key).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        std::fs::create_dir(root.0.join("certificate")).unwrap();
        std::fs::write(root.0.join("certificate/cert.key"), "fake copied key").unwrap();
        let mut deployment = Deployment::new(&root.0).unwrap();
        deployment
            .directory("certificate", vec![("cert.key", "fake replacement".into())])
            .unwrap();
        deployment.install().unwrap();
        deployment.finish().unwrap();
        assert_eq!(
            std::fs::metadata(root.0.join("certificate"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(root.0.join("certificate/cert.key"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
