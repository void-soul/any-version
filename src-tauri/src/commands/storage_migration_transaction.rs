//! Filesystem transaction separated from platform configuration/environment changes.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use super::{canonical_roots, copy_tree, directory_entry, reparse};
use crate::commands::storage_maintenance::{checked_target, normalize_relative_path};

pub(super) trait MigrationAdapter {
    fn commit(&mut self, source: &Path, destination: &Path) -> Result<(), String>;
    fn rollback(&mut self) -> Result<(), String>;
}

pub(super) fn migrate_with_adapter(
    source: &Path,
    destination: &Path,
    roots: &BTreeSet<&str>,
    adapter: &mut impl MigrationAdapter,
    fail_after: Option<usize>,
) -> Result<PathBuf, String> {
    let (source, destination) = canonical_roots(source, destination)?;
    let mut random = [0u8; 8];
    getrandom::getrandom(&mut random).map_err(|e| e.to_string())?;
    let stage = destination.join(format!(".migration-stage-{:016x}", u64::from_ne_bytes(random)));
    std::fs::create_dir(&stage).map_err(|e| e.to_string())?;
    let payload = stage.join("payload");
    let backup = stage.join("rollback");
    std::fs::create_dir_all(&payload).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&backup).map_err(|e| e.to_string())?;
    let mut committed: Vec<(String, bool)> = Vec::new();
    let result: Result<(), String> = (|| {
        let mut ready = Vec::new();
        for rel in roots {
            normalize_relative_path(rel)?;
            let src = source.join(rel);
            if !src.exists() { continue; }
            checked_target(&destination, rel)?;
            copy_tree(&src, &payload.join(rel), &source, &destination)?;
            ready.push(rel.to_string());
        }
        let inventory: Vec<_> = ready.iter().map(|rel| (rel, destination.join(rel).exists())).collect();
        std::fs::write(stage.join("inventory.json"), serde_json::to_vec(&inventory).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        for (index, rel) in ready.into_iter().enumerate() {
            if fail_after == Some(index) { return Err("injected migration failure".to_string()); }
            if rel.ends_with(".db") || rel.ends_with(".vscdb") {
                for suffix in ["-wal", "-shm"] {
                    let sidecar = format!("{rel}{suffix}");
                    let target = checked_target(&destination, &sidecar)?;
                    if target.exists() {
                        std::fs::rename(&target, backup.join(&sidecar)).map_err(|e| e.to_string())?;
                        committed.push((sidecar, true));
                    }
                }
            }
            let target = destination.join(&rel);
            let existed = target.exists();
            if existed { std::fs::rename(&target, backup.join(&rel)).map_err(|e| format!("备份目标数据失败 {rel}: {e}"))?; }
            committed.push((rel.clone(), existed));
            std::fs::rename(payload.join(&rel), &target).map_err(|e| format!("应用迁移数据失败 {rel}: {e}"))?;
        }
        adapter.commit(&source, &destination)?;
        Ok(())
    })();
    if let Err(error) = result {
        let mut failures = Vec::new();
        if let Err(e) = adapter.rollback() { failures.push(e); }
        for (rel, existed) in committed.iter().rev() {
            let target = destination.join(rel);
            if target.exists() {
                let metadata = match std::fs::symlink_metadata(&target) {
                    Ok(value) => value,
                    Err(e) => { failures.push(e.to_string()); continue; }
                };
                if !reparse(&metadata) {
                    match target.canonicalize() {
                        Ok(path) if path.starts_with(&destination) => {},
                        Ok(_) => { failures.push(format!("拒绝回滚数据目录以外的路径: {}", target.display())); continue; },
                        Err(e) => { failures.push(e.to_string()); continue; },
                    }
                }
                // Verify the concrete recursive-delete target. Reparse points are
                // removed as links; ordinary trees must resolve inside destination.
                let removed = if reparse(&metadata) && directory_entry(&metadata) { std::fs::remove_dir(&target) }
                    else if reparse(&metadata) { std::fs::remove_file(&target) }
                    else if metadata.is_dir() { std::fs::remove_dir_all(&target) }
                    else { std::fs::remove_file(&target) };
                if let Err(e) = removed { failures.push(e.to_string()); continue; }
            }
            if *existed {
                if let Err(e) = std::fs::rename(backup.join(rel), &target) { failures.push(e.to_string()); }
            }
        }
        return Err(format!("目录迁移失败，原目录保留：{error}{}", if failures.is_empty() { String::new() } else { format!("；回滚错误：{}；请保留{}", failures.join("；"), stage.display()) }));
    }
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestAdapter { entry: PathBuf, old: Vec<u8>, commits: usize }
    impl MigrationAdapter for TestAdapter {
        fn commit(&mut self, _: &Path, destination: &Path) -> Result<(), String> {
            self.commits += 1;
            let bytes = serde_json::to_vec(&serde_json::json!({"data_dir": destination.to_string_lossy()})).map_err(|e| e.to_string())?;
            std::fs::write(&self.entry, bytes).map_err(|e| e.to_string())
        }
        fn rollback(&mut self) -> Result<(), String> { std::fs::write(&self.entry, &self.old).map_err(|e| e.to_string()) }
    }

    fn fixture() -> (PathBuf, PathBuf, PathBuf, TestAdapter) {
        let mut bytes = [0u8; 8]; getrandom::getrandom(&mut bytes).unwrap();
        let root = std::env::temp_dir().join(format!("kira-migration-tests-{:016x}", u64::from_ne_bytes(bytes)));
        let source = root.join("source"); let target = root.join("target");
        std::fs::create_dir_all(&source).unwrap(); std::fs::create_dir(&target).unwrap();
        let entry = root.join("config-entry.json"); let old = serde_json::to_vec(&serde_json::json!({"data_dir": source.to_string_lossy()})).unwrap();
        std::fs::write(&entry, &old).unwrap();
        (root, source, target, TestAdapter { entry, old, commits: 0 })
    }

    #[test]
    fn copies_module_data_and_switches_root_only_after_success() {
        let (root, source, target, mut adapter) = fixture();
        std::fs::create_dir(source.join("buddy")).unwrap();
        std::fs::write(source.join("buddy/accounts.json"), "accounts").unwrap();
        std::fs::write(source.join("settings.json"), "settings").unwrap();
        migrate_with_adapter(&source, &target, &BTreeSet::from(["buddy", "settings.json"]), &mut adapter, None).unwrap();
        assert_eq!(std::fs::read_to_string(target.join("buddy/accounts.json")).unwrap(), "accounts");
        assert_eq!(std::fs::read_to_string(target.join("settings.json")).unwrap(), "settings");
        let config: serde_json::Value = serde_json::from_slice(&std::fs::read(&adapter.entry).unwrap()).unwrap();
        assert_eq!(config["data_dir"].as_str().unwrap(), target.canonicalize().unwrap().to_string_lossy());
        assert_eq!(adapter.commits, 1);
        assert_eq!(std::fs::read_to_string(source.join("buddy/accounts.json")).unwrap(), "accounts");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn existing_destination_payload_is_retained_in_rollback_directory() {
        let (root, source, target, mut adapter) = fixture();
        std::fs::write(source.join("settings.json"), "new").unwrap();
        std::fs::write(target.join("settings.json"), "old-target").unwrap();
        let backup = migrate_with_adapter(&source, &target, &BTreeSet::from(["settings.json"]), &mut adapter, None).unwrap();
        assert_eq!(std::fs::read_to_string(target.join("settings.json")).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(backup.join("settings.json")).unwrap(), "old-target");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn injected_mid_commit_failure_restores_destination_and_keeps_old_root() {
        let (root, source, target, mut adapter) = fixture();
        for name in ["backups.json", "settings.json"] {
            std::fs::write(source.join(name), format!("new-{name}")).unwrap();
            std::fs::write(target.join(name), format!("old-{name}")).unwrap();
        }
        assert!(migrate_with_adapter(&source, &target, &BTreeSet::from(["backups.json", "settings.json"]), &mut adapter, Some(1)).is_err());
        for name in ["backups.json", "settings.json"] {
            assert_eq!(std::fs::read_to_string(target.join(name)).unwrap(), format!("old-{name}"));
            assert_eq!(std::fs::read_to_string(source.join(name)).unwrap(), format!("new-{name}"));
        }
        assert_eq!(std::fs::read(&adapter.entry).unwrap(), adapter.old);
        assert_eq!(adapter.commits, 0);
        std::fs::remove_dir_all(root).unwrap();
    }
}
