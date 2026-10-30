//! Application-level data-root migration, executed only under the startup storage lock.
//! Source data is copied, never deleted; destination data is retained for rollback.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use super::storage_maintenance::{checked_target, PendingOperation};
#[path = "storage_migration_links.rs"]
mod links;
use links::directory_entry;

fn canonical_roots(source: &Path, destination: &Path) -> Result<(PathBuf, PathBuf), String> {
    if !source.is_absolute() || !destination.is_absolute() { return Err("数据目录必须是绝对路径".into()); }
    let source = source.canonicalize().map_err(|e| format!("读取旧数据目录失败: {e}"))?;
    std::fs::create_dir_all(destination).map_err(|e| format!("创建目标数据目录失败: {e}"))?;
    let destination = destination.canonicalize().map_err(|e| e.to_string())?;
    if source != destination && (source.starts_with(&destination) || destination.starts_with(&source)) {
        return Err("新旧数据目录不能互相包含，请选择独立目录".into());
    }
    Ok((source, destination))
}

pub fn schedule_migration(source: &Path, destination: &Path) -> Result<(), String> {
    let (source, destination) = canonical_roots(source, destination)?;
    if source == destination { return Ok(()); }
    let base = super::config::get_base_dir();
    super::storage_maintenance::write_pending(&base, &PendingOperation::Migrate { source, destination })
}

fn reparse(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    { use std::os::windows::fs::MetadataExt; metadata.file_attributes() & 0x400 != 0 }
    #[cfg(not(windows))]
    { metadata.file_type().is_symlink() }
}

fn plain_path(path: &Path) -> PathBuf {
    let value = path.to_string_lossy();
    if let Some(rest) = value.strip_prefix(r"\\?\UNC\") { PathBuf::from(format!(r"\\{}", rest)) }
    else { PathBuf::from(value.trim_start_matches(r"\\?\")) }
}

fn replace_root(path: &Path, source: &Path, destination: &Path) -> PathBuf {
    if let Ok(relative) = path.strip_prefix(source) { return plain_path(destination).join(relative); }
    // Windows path spelling may differ in case or use/non-use of the extended prefix.
    let input = path.to_string_lossy().trim_start_matches(r"\\?\").replace('/', "\\");
    let old = source.to_string_lossy().trim_start_matches(r"\\?\").replace('/', "\\");
    if input.to_lowercase().starts_with(&(old.to_lowercase() + "\\")) {
        plain_path(destination).join(&input[old.len()+1..])
    } else { path.to_path_buf() }
}

fn copy_tree(source: &Path, destination: &Path, old_root: &Path, new_root: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(source).map_err(|e| e.to_string())?;
    if reparse(&metadata) {
        let target = std::fs::read_link(source).map_err(|e| format!("读取链接失败 {}: {e}", source.display()))?;
        let absolute = if target.is_absolute() { target } else { source.parent().unwrap().join(target) };
        let target = replace_root(&absolute, old_root, new_root);
        if let Some(parent) = destination.parent() { std::fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
        #[cfg(windows)]
        {
            if directory_entry(&metadata) {
                // Preserve the junction itself, without touching its final target.
                links::create_directory_link(destination, &target)?;
            } else {
                std::os::windows::fs::symlink_file(target, destination).map_err(|e| format!("复制文件链接失败: {e}"))?;
            }
        }
        #[cfg(not(windows))]
        { std::os::unix::fs::symlink(target, destination).map_err(|e| e.to_string())?; }
    } else if metadata.is_dir() {
        std::fs::create_dir_all(destination).map_err(|e| e.to_string())?;
        for entry in std::fs::read_dir(source).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name();
            let text = name.to_string_lossy();
            if ["-wal", "-shm"].iter().any(|suffix| text.strip_suffix(suffix).is_some_and(|main| main.ends_with(".db") || main.ends_with(".vscdb"))) { continue; }
            copy_tree(&entry.path(), &destination.join(name), old_root, new_root)?;
        }
    } else if metadata.is_file() {
        if let Some(parent) = destination.parent() { std::fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
        std::fs::write(destination, super::storage_maintenance::snapshot_file_bytes(source)?)
            .map_err(|e| format!("复制数据文件失败 {}: {e}", source.display()))?;
        if source.file_name().is_some_and(|name| name == ".master_key") { super::secrets::restrict_master_key_acl(destination); }
    }
    Ok(())
}

fn owned_roots() -> BTreeSet<&'static str> {
    let mut roots = super::storage_assets::migration_roots();
    roots.extend(["sdk", "bin", "version_cache", "caches"]);
    roots
}

fn environment_names(config: &super::config::Config) -> BTreeSet<String> {
    let mut names = BTreeSet::from(["PATH".to_string()]);
    for id in &config.managed_items {
        if let Some(def) = super::project::registry::find_by_id(id) {
            names.extend(def.env_vars.into_iter().map(|v| v.name));
        }
    }
    names
}

#[path = "storage_migration_transaction.rs"]
mod transaction;
use transaction::MigrationAdapter;

struct PlatformAdapter {
    config: super::config::Config,
    entry: PathBuf,
    original_entry: Vec<u8>,
    environments: Vec<(String, String)>,
}

impl MigrationAdapter for PlatformAdapter {
    fn commit(&mut self, source: &Path, destination: &Path) -> Result<(), String> {
        for name in environment_names(&self.config) {
            if let Some(old) = super::env::get_registry_env(&name) {
                let new = if name == "PATH" {
                    let paths: Vec<_> = std::env::split_paths(&old).map(|p| replace_root(&p, source, destination)).collect();
                    std::env::join_paths(paths).map_err(|e| e.to_string())?.to_string_lossy().into_owned()
                } else { replace_root(Path::new(&old), source, destination).to_string_lossy().into_owned() };
                if new != old {
                    self.environments.push((name.clone(), old));
                    super::env::set_registry_env(&name, &new)?;
                }
            }
        }
        let destination = plain_path(destination);
        super::config::mutate_config(|config| {
            config.data_dir = destination.to_string_lossy().into_owned();
            config.sdk_dir = destination.join("sdk").to_string_lossy().into_owned();
            config.versions_dir = destination.join("sdk/_versions").to_string_lossy().into_owned();
            config.links_dir = destination.join("sdk").to_string_lossy().into_owned();
            if !config.node_projects_dir.trim().is_empty() {
                config.node_projects_dir = replace_root(Path::new(&config.node_projects_dir), source, &destination).to_string_lossy().into_owned();
            }
            Ok(())
        })
    }
    fn rollback(&mut self) -> Result<(), String> {
        let mut failures = Vec::new();
        if let Err(e) = super::foundation::atomic_write_file(&self.entry, &self.original_entry) { failures.push(e); }
        for (name, value) in self.environments.iter().rev() {
            if let Err(e) = super::env::set_registry_env(name, value) { failures.push(e); }
        }
        if failures.is_empty() { Ok(()) } else { Err(failures.join("; ")) }
    }
}

pub fn apply_migration(source: &Path, destination: &Path) -> Result<(), String> {
    let (source, destination) = canonical_roots(source, destination)?;
    if source == destination { return Ok(()); }
    let current_root = super::config::get_data_dir().canonicalize().map_err(|e| e.to_string())?;
    if current_root == destination { return Ok(()); }
    if current_root != source { return Err("Migration source changed; restore the original configuration before restarting".into()); }
    let entry = super::config::get_base_dir().join("config.json");
    let mut adapter = PlatformAdapter { config: super::config::load_config(), original_entry: std::fs::read(&entry).map_err(|e| e.to_string())?, entry, environments: Vec::new() };
    let backup = transaction::migrate_with_adapter(&source, &destination, &owned_roots(), &mut adapter, None)?;
    tracing::info!(source=%source.display(), destination=%destination.display(), rollback=%backup.display(), "data directory migration applied; source retained");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_migration_roots_cover_independent_module_storage() {
        let roots = owned_roots();
        for name in ["buddy", "music", "suno", "db_designer", "launcher.db", "mindmap.db", "favorites.db", "api.db", "otp", "clipboard", "sdk"] {
            assert!(roots.contains(name), "{name}");
        }
    }

    #[test]
    fn path_relocation_preserves_external_directories() {
        let old = Path::new(r"C:\old"); let new = Path::new(r"D:\new");
        assert_eq!(replace_root(Path::new(r"C:\old\sdk\node"), old, new), new.join(r"sdk\node"));
        assert_eq!(replace_root(Path::new(r"C:\older\node"), old, new), PathBuf::from(r"C:\older\node"));
    }
}


use super::config::{Config, get_sdk_link_dir, get_sdk_versions_dir};
pub fn rebuild_sdk_junctions(config: &Config) -> Vec<String> {
    let mut rebuilt = Vec::new();
    let links_dir = get_sdk_link_dir();
    let versions_dir = get_sdk_versions_dir();
    for (item_id, version) in &config.active_versions {
        let link_path = links_dir.join(item_id);
        let target_path = versions_dir.join(item_id).join(version);
        if !target_path.exists() {
            continue;
        }
        let needs_rebuild = if link_path.is_symlink() {
            // 已是 junction，校验指向是否正确。
            // 注意：fs::canonicalize 在 Windows 返回带 `\\?\` 前缀的路径，
            // 必须去掉前缀再比较，否则恒判断为「指向错误」，每次启动都会误删重建 junction。
            fs::canonicalize(&link_path)
                .map(|c| {
                    let resolved = c.to_string_lossy().to_string();
                    let resolved_clean = resolved.trim_start_matches("\\\\?\\").to_lowercase();
                    let target_clean = target_path.to_string_lossy().to_lowercase();
                    !resolved_clean.starts_with(&target_clean)
                })
                .unwrap_or(false)
        } else {
            // 非 junction（不存在或普通目录）都需要重建
            true
        };
        if needs_rebuild {
            let _ = crate::commands::cache::create_junction(&link_path, &target_path);
            rebuilt.push(item_id.clone());
        }
    }
    rebuilt
}

