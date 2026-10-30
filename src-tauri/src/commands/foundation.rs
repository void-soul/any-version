//! Business-independent paths, resource location and atomic file replacement.
//! This layer must not import configuration models or any business module.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::io::Write;

pub mod path_templates;

pub fn get_home_dir() -> PathBuf {
    let home = std::env::var("USERPROFILE").ok().filter(|v| !v.is_empty())
        .or_else(|| std::env::var("HOME").ok().filter(|v| !v.is_empty()));
    home.map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(not(test))]
pub fn get_base_dir() -> PathBuf {
    let user_profile = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_else(|_| {
        let drive = std::env::var("HOMEDRIVE").unwrap_or_default();
        let path = std::env::var("HOMEPATH").unwrap_or_default();
        if drive.is_empty() && path.is_empty() { "C:\\any-version".to_string() } else { format!("{drive}{path}") }
    });
    if user_profile.is_empty() || user_profile == "C:\\any-version" { PathBuf::from("C:\\any-version") }
    else { PathBuf::from(user_profile).join(".any-version") }
}

/// Unit tests never read or write the actual user's application data directory.
#[cfg(test)]
pub fn get_base_dir() -> PathBuf {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let mut bytes = [0u8; 8];
        getrandom::getrandom(&mut bytes).expect("test root randomness");
        std::env::temp_dir().join(format!("kira-unit-data-{}-{:016x}", std::process::id(), u64::from_ne_bytes(bytes)))
    }).clone()
}

fn entry_paths() -> serde_json::Value {
    std::fs::read(get_base_dir().join("config.json")).ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or(serde_json::Value::Null)
}

pub fn get_data_dir() -> PathBuf {
    entry_paths().get("data_dir").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty())
        .map(PathBuf::from).unwrap_or_else(get_base_dir)
}

pub fn get_sdk_dir() -> PathBuf { get_data_dir().join("sdk") }
pub fn get_sdk_versions_dir() -> PathBuf { get_sdk_dir().join("_versions") }
pub fn get_sdk_link_dir() -> PathBuf { get_sdk_dir() }
pub fn get_cache_root() -> PathBuf { get_data_dir().join("caches") }
pub fn get_node_projects_dir() -> PathBuf {
    match entry_paths().get("node_projects_dir").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()) {
        Some(path) if Path::new(path).is_absolute() => PathBuf::from(path),
        Some(path) => get_data_dir().join(path),
        None => get_data_dir().join("node-projects"),
    }
}

static RESOURCE_DIR: OnceLock<PathBuf> = OnceLock::new();
pub fn set_resource_dir(path: PathBuf) { let _ = RESOURCE_DIR.set(path); }
pub fn get_resource_dir() -> Option<PathBuf> { RESOURCE_DIR.get().cloned() }
pub fn dev_repo_root() -> Option<PathBuf> {
    if cfg!(debug_assertions) { Path::new(env!("CARGO_MANIFEST_DIR")).parent().map(Path::to_path_buf) }
    else { None }
}

pub fn expand_path(path: &str) -> String {
    let expanded = path.replace("{home}", &get_home_dir().to_string_lossy())
        .replace("{data_dir}", &get_data_dir().to_string_lossy());
    path_templates::expand(&expanded)
}

/// Failure never falls back to truncating the original file.
pub(crate) fn atomic_write_file(path: &Path, data: &[u8]) -> Result<(), String> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| format!("创建文件目录失败: {e}"))?;
    let name = path.file_name().ok_or("写入路径缺少文件名")?.to_string_lossy();
    let mut bytes = [0u8; 8];
    getrandom::getrandom(&mut bytes).map_err(|e| e.to_string())?;
    let temporary = parent.join(format!(".{name}.{:016x}.tmp", u64::from_ne_bytes(bytes)));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temporary).map_err(|e| e.to_string())?;
        file.write_all(data).and_then(|_| file.sync_all()).map_err(|e| e.to_string())?;
        drop(file);
        std::fs::rename(&temporary, path).map_err(|e| format!("替换文件失败，原文件保留: {e}"))
    })();
    if result.is_err() { let _ = std::fs::remove_file(temporary); }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_complete_files_and_preserves_target_on_failure() {
        let root = get_base_dir().join("atomic-test"); std::fs::create_dir_all(&root).unwrap();
        let path = root.join("settings.json");
        atomic_write_file(&path, b"old").unwrap(); atomic_write_file(&path, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        let directory = root.join("directory"); std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("kept"), b"original").unwrap();
        assert!(atomic_write_file(&directory, b"invalid").is_err());
        assert_eq!(std::fs::read(directory.join("kept")).unwrap(), b"original");
        assert!(!std::fs::read_dir(&root).unwrap().flatten().any(|e| e.file_name().to_string_lossy().ends_with(".tmp")));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn foundation_has_no_business_module_imports() {
        let source = include_str!("foundation.rs").split("\nmod tests {").next().unwrap();
        let template = include_str!("foundation/path_templates.rs").split("\nmod tests {").next().unwrap();
        for file in [source, template] {
            assert!(!file.lines().filter(|l| !l.trim_start().starts_with("//")).any(|l| l.contains("crate::commands::project::") || l.contains("crate::commands::config::")));
        }
    }
}
