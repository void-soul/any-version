//! Fallible history IO: only a missing file means empty history.
use super::{new_entry_id, TranslateHistoryEntry};
use std::path::Path;

pub(super) fn persist(path: &Path, list: &[TranslateHistoryEntry]) -> Result<(), String> {
    let data = serde_json::to_vec_pretty(list).map_err(|e| format!("序列化翻译历史失败: {e}"))?;
    crate::commands::config::atomic_write_file(path, &data)
}

pub(super) fn load(path: &Path) -> Result<Vec<TranslateHistoryEntry>, String> {
    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("读取翻译历史失败: {e}")),
    };
    let mut list: Vec<TranslateHistoryEntry> = serde_json::from_slice(&data)
        .map_err(|e| format!("解析翻译历史失败: {e}"))?;
    let mut changed = false;
    for entry in &mut list {
        if entry.id.is_empty() {
            entry.id = new_entry_id(entry.ts);
            changed = true;
        }
    }
    if changed { persist(path, &list)?; }
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
            let path = std::env::temp_dir().join(format!("kira-translate-history-{}-{nonce}", std::process::id()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }
    #[test]
    fn missing_is_empty_but_corrupt_and_directory_are_errors() {
        let f = Fixture::new();
        let path = f.0.join("history.json");
        assert!(load(&path).unwrap().is_empty());
        std::fs::write(&path, b"broken").unwrap();
        assert!(load(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"broken");
        assert!(load(&f.0).is_err());
        assert!(persist(&f.0, &[]).is_err());
    }
    #[test]
    fn legacy_ids_are_migrated_and_empty_clear_is_persisted() {
        let f = Fixture::new();
        let path = f.0.join("history.json");
        std::fs::write(&path, r#"[{"source":"a","result":"b","target":"en","provider":"p","model":"m","ts":1}]"#).unwrap();
        let migrated = load(&path).unwrap();
        assert!(!migrated[0].id.is_empty());
        assert_eq!(load(&path).unwrap()[0].id, migrated[0].id);
        persist(&path, &[]).unwrap();
        assert!(load(&path).unwrap().is_empty());
    }
}
