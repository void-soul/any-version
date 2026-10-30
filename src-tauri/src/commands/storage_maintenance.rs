//! Quiescent restore coordinator. Import only stages data; startup applies it before
//! any module opens a database. Existing local path configuration is never imported.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use fs2::FileExt;

pub const MAX_SNAPSHOT_BYTES: usize = 1024 * 1024 * 1024;
const PENDING_FILE: &str = ".pending-storage-operation.json";
static MAINTENANCE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
static APPLICATION_STORAGE_GUARD: std::sync::OnceLock<std::fs::File> = std::sync::OnceLock::new();

#[derive(Serialize, Deserialize)]
pub(crate) struct RestorePlan {
    pub target_root: PathBuf,
    pub staging_root: PathBuf,
    pub files: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "operation")]
pub(crate) enum PendingOperation {
    Restore(RestorePlan),
    Migrate { source: PathBuf, destination: PathBuf },
}

pub fn normalize_relative_path(rel: &str) -> Result<String, String> {
    if rel.is_empty() || rel.starts_with(['/', '\\']) || rel.contains(':') {
        return Err(format!("备份包含非法文件路径: {rel}"));
    }
    let mut parts = Vec::new();
    for part in rel.split(['/', '\\']) {
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && matches!(stem.as_bytes()[3], b'1'..=b'9'));
        if part.is_empty() || part == "." || part == ".." || part.ends_with(['.', ' ']) || device
            || part.chars().any(|c| c.is_control() || matches!(c, '<' | '>' | '"' | '|' | '?' | '*')) {
            return Err(format!("备份包含非法文件路径: {rel}"));
        }
        parts.push(part);
    }
    Ok(parts.join("/"))
}

fn is_reparse(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    { metadata.file_type().is_symlink() }
}

/// Existing ancestors must stay under the trusted root and must not be reparse points.
pub(crate) fn checked_target(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel = normalize_relative_path(rel)?;
    let canonical_root = root.canonicalize().map_err(|e| format!("读取数据目录失败: {e}"))?;
    let mut path = root.to_path_buf();
    for part in rel.split('/') {
        path.push(part);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if is_reparse(&metadata) || !path.canonicalize().map_err(|e| e.to_string())?.starts_with(&canonical_root) {
                    return Err(format!("文件路径经过链接或越出数据目录: {rel}"));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(e) => return Err(format!("检查恢复路径失败: {e}")),
        }
    }
    Ok(path)
}

pub(crate) fn operation_file(base: &Path) -> PathBuf { base.join(PENDING_FILE) }

pub(crate) fn write_pending(base: &Path, operation: &PendingOperation) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(operation).map_err(|e| e.to_string())?;
    // create_new is intentional: never replace a plan that the user has not applied.
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(operation_file(base))
        .map_err(|e| format!("保存待重启操作失败（已有操作时请先重启）: {e}"))?;
    if let Err(e) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
        drop(file);
        let _ = std::fs::remove_file(operation_file(base));
        return Err(format!("保存待重启操作失败: {e}"));
    }
    Ok(())
}

fn staging_dir(base: &Path) -> Result<PathBuf, String> {
    let mut random = [0u8; 8];
    getrandom::getrandom(&mut random).map_err(|e| e.to_string())?;
    let path = base.join(format!(".restore-{:016x}", u64::from_ne_bytes(random)));
    std::fs::create_dir(&path).map_err(|e| format!("创建恢复暂存目录失败: {e}"))?;
    Ok(path)
}

/// Read SQLite through its own snapshot mechanism, not by copying a live main/WAL pair.
pub fn snapshot_file_bytes(path: &Path) -> Result<Vec<u8>, String> {
    let mut header = [0u8; 16];
    let mut file = std::fs::File::open(path).map_err(|e| format!("读取文件失败 {}: {e}", path.display()))?;
    let n = file.read(&mut header).map_err(|e| e.to_string())?;
    drop(file);
    if n == 16 && &header == b"SQLite format 3\0" {
        let dir = staging_dir(&std::env::temp_dir())?;
        let dest = dir.join("snapshot.db");
        let result = (|| {
            let conn = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| e.to_string())?;
            conn.busy_timeout(std::time::Duration::from_secs(10)).map_err(|e| e.to_string())?;
            conn.execute("VACUUM INTO ?1", [dest.to_string_lossy().as_ref()]).map_err(|e| format!("创建一致数据库快照失败 {}: {e}", path.display()))?;
            std::fs::read(&dest).map_err(|e| e.to_string())
        })();
        let _ = std::fs::remove_dir_all(dir);
        result
    } else {
        if path.extension().is_some_and(|e| e == "db" || e == "vscdb") {
            return Err(format!("数据库文件无效，未导出: {}", path.display()));
        }
        std::fs::read(path).map_err(|e| e.to_string())
    }
}

/// Fully decode and validate first. No user data file is changed until next startup.
pub fn stage_restore(base: &Path, target_root: &Path, state: &serde_json::Value, selected: Option<&HashSet<String>>) -> Result<usize, String> {
    let _guard = MAINTENANCE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if operation_file(base).exists() { return Err("已有待重启的恢复或迁移操作，请先重启应用".into()); }
    if state.get("type").and_then(|v| v.as_str()) != Some("any-version-state") {
        return Err("无法识别这个备份文件，请选择本应用导出的快照".into());
    }
    if state.get("version").and_then(|v| v.as_u64()).is_some_and(|version| version > 2) {
        return Err("备份版本高于当前应用，请升级后再导入".into());
    }
    let entries = state.get("files").and_then(|v| v.as_object()).ok_or("备份文件缺少数据内容")?;
    let mut decoded = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut total = 0usize;
    for (raw, value) in entries {
        let rel = normalize_relative_path(raw)?;
        if !seen.insert(rel.to_lowercase()) { return Err(format!("备份包含重复文件: {rel}")); }
        if !super::storage_assets::allows_snapshot_path(&rel) { return Err(format!("备份包含不属于已声明模块的数据文件: {rel}")); }
        if selected.is_some_and(|s| !s.contains(raw)) { continue; }
        // Path settings belong to this machine; do not switch to the exporter's directory.
        if rel == "config.json" { continue; }
        checked_target(target_root, &rel)?;
        let text = value.as_str().ok_or_else(|| format!("备份文件内容无效: {rel}"))?;
        if text.len() > MAX_SNAPSHOT_BYTES.saturating_mul(4) / 3 + 4 { return Err("单个备份文件超过恢复大小限制".into()); }
        let bytes = STANDARD.decode(text).map_err(|e| format!("备份文件解码失败 {rel}: {e}"))?;
        if rel.ends_with(".json") && super::storage_assets::snapshot_files().any(|p| p == rel) {
            let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| format!("备份配置不是有效JSON {rel}: {e}"))?;
            if !value.is_object() && !value.is_array() { return Err(format!("备份配置必须是对象或列表: {rel}")); }
        }
        total = total.checked_add(bytes.len()).filter(|n| *n <= MAX_SNAPSHOT_BYTES).ok_or("备份数据超过1 GiB，请分批恢复")?;
        decoded.insert(rel, bytes);
    }
    if decoded.is_empty() { return Err("没有可恢复的数据；本机路径设置会保留".into()); }
    for rel in decoded.keys().filter(|r| r.ends_with("-wal")) {
        if !decoded.contains_key(rel.trim_end_matches("-wal")) { return Err(format!("数据库恢复缺少主文件: {rel}")); }
    }
    let stage = staging_dir(base)?;
    let result = (|| {
        for (rel, bytes) in &decoded {
            let file = stage.join("payload").join(rel);
            std::fs::create_dir_all(file.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::write(file, bytes).map_err(|e| e.to_string())?;
            if rel.ends_with("/.master_key") { super::secrets::restrict_master_key_acl(&stage.join("payload").join(rel)); }
        }
        let mut files = Vec::new();
        for rel in decoded.keys().filter(|r| !r.ends_with("-wal")) {
            let file = stage.join("payload").join(rel);
            let header = decoded[rel].get(..16);
            if (rel.ends_with(".db") || rel.ends_with(".vscdb")) && header != Some(b"SQLite format 3\0".as_slice()) {
                return Err(format!("备份数据库文件格式无效: {rel}"));
            }
            if header == Some(b"SQLite format 3\0".as_slice()) {
                let conn = rusqlite::Connection::open(&file).map_err(|e| format!("备份数据库无法打开 {rel}: {e}"))?;
                let result: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0)).map_err(|e| e.to_string())?;
                if result != "ok" { return Err(format!("备份数据库校验失败 {rel}: {result}")); }
                conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").map_err(|e| e.to_string())?;
            }
            files.push(rel.clone());
        }
        let count = files.len();
        write_pending(base, &PendingOperation::Restore(RestorePlan { target_root: target_root.to_path_buf(), staging_root: stage.clone(), files }))?;
        Ok(count)
    })();
    if result.is_err() { let _ = std::fs::remove_dir_all(&stage); }
    result
}

#[derive(Serialize, Deserialize)]
struct RollbackFile { relative: String, existed: bool }

fn rollback(plan: &RestorePlan, journal: &[RollbackFile]) -> Result<(), String> {
    for entry in journal.iter().rev() {
        let target = checked_target(&plan.target_root, &entry.relative)?;
        if entry.existed {
            std::fs::copy(plan.staging_root.join("rollback").join(&entry.relative), &target).map_err(|e| format!("回滚失败 {}: {e}", entry.relative))?;
        } else if target.exists() {
            std::fs::remove_file(target).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

pub(crate) fn apply_restore(plan: &RestorePlan) -> Result<(), String> {
    apply_restore_inner(plan, None)
}

/// CLI processes participate in the same lifecycle barrier without applying plans.
pub fn hold_storage_read_lock() -> Result<(), String> {
    if APPLICATION_STORAGE_GUARD.get().is_some() { return Ok(()); }
    let base = super::config::get_base_dir();
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let guard = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false)
        .open(base.join(".application-storage.lock")).map_err(|e| e.to_string())?;
    FileExt::lock_shared(&guard).map_err(|e| format!("应用数据暂不可用: {e}"))?;
    APPLICATION_STORAGE_GUARD.set(guard).map_err(|_| "应用存储已初始化".to_string())
}

fn apply_restore_inner(plan: &RestorePlan, fail_after: Option<usize>) -> Result<(), String> {
    for rel in &plan.files {
        normalize_relative_path(rel)?;
        if rel == "config.json" || !super::storage_assets::allows_snapshot_path(rel) {
            return Err(format!("待恢复计划包含未声明的文件: {rel}"));
        }
    }
    let journal_path = plan.staging_root.join("rollback.json");
    let journal: Vec<RollbackFile> = if journal_path.exists() {
        let entries: Vec<RollbackFile> = serde_json::from_slice(&std::fs::read(&journal_path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        rollback(plan, &entries)?;
        entries
    } else {
        let mut paths = BTreeSet::new();
        for rel in &plan.files {
            paths.insert(rel.clone());
            if rel.ends_with(".db") || rel.ends_with(".vscdb") {
                paths.insert(format!("{rel}-wal"));
                paths.insert(format!("{rel}-shm"));
            }
        }
        let mut entries = Vec::new();
        for relative in paths {
            let target = checked_target(&plan.target_root, &relative)?;
            let existed = target.is_file();
            if target.exists() && !existed { return Err(format!("恢复目标不是文件: {relative}")); }
            if existed {
                let backup = plan.staging_root.join("rollback").join(&relative);
                std::fs::create_dir_all(backup.parent().unwrap()).map_err(|e| e.to_string())?;
                std::fs::copy(&target, backup).map_err(|e| e.to_string())?;
            }
            entries.push(RollbackFile { relative, existed });
        }
        std::fs::write(&journal_path, serde_json::to_vec(&entries).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        entries
    };
    let result: Result<(), String> = (|| {
        // Delete all old WAL/SHM files before replacing their corresponding databases.
        for entry in &journal {
            if entry.relative.ends_with("-wal") || entry.relative.ends_with("-shm") {
                let path = checked_target(&plan.target_root, &entry.relative)?;
                if path.exists() { std::fs::remove_file(path).map_err(|e| e.to_string())?; }
            }
        }
        for (index, rel) in plan.files.iter().enumerate() {
            if fail_after == Some(index) { return Err("injected restore failure".to_string()); }
            let target = checked_target(&plan.target_root, rel)?;
            std::fs::create_dir_all(target.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::copy(checked_target(&plan.staging_root.join("payload"), rel)?, target).map_err(|e| e.to_string())?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        return match rollback(plan, &journal) {
            Ok(()) => Err(format!("恢复失败，已还原原数据，待恢复计划保留: {error}")),
            Err(rollback_error) => Err(format!("恢复失败: {error}；{rollback_error}；请保留暂存目录用于修复")),
        };
    }
    Ok(())
}

/// Called before any module initializes config, workers or SQLite connections.
pub fn apply_pending_operations() -> Result<(), String> {
    let base = super::config::get_base_dir();
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let path = operation_file(&base);
    let guard = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false)
        .open(base.join(".application-storage.lock")).map_err(|e| format!("无法锁定应用存储: {e}"))?;
    if !path.is_file() {
        FileExt::lock_shared(&guard).map_err(|e| format!("无法锁定应用存储: {e}"))?;
        APPLICATION_STORAGE_GUARD.set(guard).map_err(|_| "应用存储已初始化".to_string())?;
        return Ok(());
    }
    // All running instances hold a shared lock. A second launch cannot restore
    // data while the original instance still owns live connections.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        match FileExt::try_lock_exclusive(&guard) {
            Ok(()) => break,
            Err(error) if (error.kind() == std::io::ErrorKind::WouldBlock || error.raw_os_error() == fs2::lock_contended_error().raw_os_error())
                && std::time::Instant::now() < deadline => {
                // Tauri relaunch can start this process slightly before the old one exits.
                std::thread::sleep(std::time::Duration::from_millis(50));
                if !path.exists() {
                    FileExt::lock_shared(&guard).map_err(|e| e.to_string())?;
                    APPLICATION_STORAGE_GUARD.set(guard).map_err(|_| "应用存储已初始化".to_string())?;
                    return Ok(());
                }
            }
            Err(error) => return Err(format!("恢复或迁移前必须关闭所有应用实例，再重新启动: {error}")),
        }
    }
    let _guard = MAINTENANCE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let operation: PendingOperation = serde_json::from_slice(&std::fs::read(&path).map_err(|e| e.to_string())?).map_err(|e| format!("待重启存储操作损坏，已保留原数据: {e}"))?;
    match operation {
        PendingOperation::Restore(plan) => {
            let canonical_base = base.canonicalize().map_err(|e| e.to_string())?;
            let canonical_stage = plan.staging_root.canonicalize().map_err(|e| e.to_string())?;
            if canonical_stage.parent() != Some(canonical_base.as_path())
                || !canonical_stage.file_name().is_some_and(|n| n.to_string_lossy().starts_with(".restore-"))
                || is_reparse(&std::fs::symlink_metadata(&plan.staging_root).map_err(|e| e.to_string())?) {
                return Err("恢复暂存目录不属于应用数据入口，已停止操作".into());
            }
            if plan.target_root.canonicalize().map_err(|e| e.to_string())?
                != super::config::get_data_dir().canonicalize().map_err(|e| e.to_string())? {
                return Err("恢复前数据目录已改变，请恢复原路径后重启".into());
            }
            apply_restore(&plan)?;
            std::fs::remove_file(&path).map_err(|e| e.to_string())?;
            let _ = std::fs::remove_dir_all(&plan.staging_root);
        }
        PendingOperation::Migrate { source, destination } => {
            super::storage_migration::apply_migration(&source, &destination)?;
            std::fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
    }
    FileExt::unlock(&guard).map_err(|e| e.to_string())?;
    FileExt::lock_shared(&guard).map_err(|e| e.to_string())?;
    APPLICATION_STORAGE_GUARD.set(guard).map_err(|_| "应用存储已初始化".to_string())?;
    Ok(())
}

#[cfg(test)]
#[path = "storage_maintenance_tests.rs"]
mod tests;
