//! Local display metadata is independent of upstream account/token refreshes.
use super::*;
use std::collections::BTreeMap;
use std::path::Path;

fn path(platform: BuddyPlatform) -> Result<PathBuf, String> {
    Ok(buddy_root()?.join(format!("{}_remarks.json", platform.as_str())))
}

fn read(path: &Path) -> Result<BTreeMap<String, String>, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(format!("读取账号备注失败: {error}")),
    };
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("账号备注文件无效，未覆盖原文件: {error}"))
}

pub fn load_account_remarks(platform: BuddyPlatform) -> Result<BTreeMap<String, String>, String> {
    let _lock = STORE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    read(&path(platform)?)
}

pub fn set_account_remark(
    platform: BuddyPlatform,
    account_id: &str,
    remark: &str,
) -> Result<Option<String>, String> {
    let _lock = STORE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let id = normalize_account_id(account_id)?;
    if load_account(platform, &id).is_none() {
        return Err("账号不存在，请刷新账号列表".into());
    }
    update(&path(platform)?, &id, remark)
}

// Caller holds STORE_LOCK across the checked read-modify-atomic-write.
fn update(path: &Path, id: &str, value: &str) -> Result<Option<String>, String> {
    let value = value.trim();
    if value.chars().count() > 200 {
        return Err("账号备注最多200个字符".into());
    }
    let mut remarks = read(path)?;
    let result = if value.is_empty() {
        remarks.remove(id);
        None
    } else {
        remarks.insert(id.into(), value.into());
        Some(value.to_string())
    };
    crate::commands::foundation::atomic_write_file(
        path,
        &serde_json::to_vec_pretty(&remarks).map_err(|error| error.to_string())?,
    )?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Root(PathBuf);
    impl Root {
        fn new() -> Self {
            let mut nonce = [0u8; 16];
            getrandom::getrandom(&mut nonce).unwrap();
            let suffix: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
            let path = std::env::temp_dir().join(format!("buddy-remarks-{suffix}"));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Root {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn remarks_persist_trim_and_clear_without_changing_other_accounts_or_platforms() {
        let root = Root::new();
        let work = root.0.join("workbuddy.json");
        let cn = root.0.join("codebuddy-cn.json");
        assert!(read(&work).unwrap().is_empty());
        assert_eq!(
            update(&work, "a", "  完整号码备注  ").unwrap().as_deref(),
            Some("完整号码备注")
        );
        update(&work, "b", "第二个账号").unwrap();
        update(&cn, "a", "另一平台").unwrap();
        assert_eq!(read(&work).unwrap()["a"], "完整号码备注");
        assert_eq!(update(&work, "a", "  ").unwrap(), None);
        assert_eq!(read(&work).unwrap().len(), 1);
        assert_eq!(read(&work).unwrap()["b"], "第二个账号");
        assert_eq!(read(&cn).unwrap()["a"], "另一平台");
    }
    #[test]
    fn bad_metadata_and_invalid_input_are_not_overwritten() {
        let root = Root::new();
        let file = root.0.join("remarks.json");
        fs::write(&file, "bad json").unwrap();
        assert!(update(&file, "a", "new").is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), "bad json");
        assert!(update(&file, "a", &"长".repeat(201)).is_err());
        assert!(read(&root.0).is_err());
        assert!(set_account_remark(BuddyPlatform::Workbuddy, "../outside", "x").is_err());
    }
    #[test]
    fn serialized_updates_keep_concurrent_account_remarks() {
        let root = Root::new();
        let file = root.0.join("remarks.json");
        let threads: Vec<_> = (0..8)
            .map(|index| {
                let file = file.clone();
                std::thread::spawn(move || {
                    let _lock = STORE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
                    update(&file, &format!("a{index}"), "remark").unwrap();
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(read(&file).unwrap().len(), 8);
    }
}
