use super::*;

/// Detail files are authoritative. The JSON index is a recoverable projection.
/// Reading never rewrites user files; the next successful mutation saves the repaired index.
pub(super) fn load_index_checked(platform: BuddyPlatform) -> Result<BuddyAccountIndex, String> {
    let directory = accounts_dir(platform)?;
    let entries =
        fs::read_dir(&directory).map_err(|e| format!("读取账号目录失败，拒绝将其视为空库: {e}"))?;
    let mut index = BuddyAccountIndex::default();
    for entry in entries {
        let entry = entry.map_err(|e| format!("读取账号目录条目失败: {e}"))?;
        let path = entry.path();
        if path.extension().and_then(|v| v.to_str()) != Some("json") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|v| v.to_str()) else {
            continue;
        };
        if normalize_account_id(id).is_err() {
            continue;
        }
        let bytes =
            fs::read(&path).map_err(|e| format!("读取账号文件 {} 失败: {e}", path.display()))?;
        // Corrupt details are preserved on disk, never overwritten or deleted by recovery.
        let Ok(account) = serde_json::from_slice::<BuddyAccount>(&bytes) else {
            continue;
        };
        if account.id != id {
            continue;
        }
        index.accounts.push(account.summary());
    }
    index.accounts.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(index)
}

pub(super) fn load_index(platform: BuddyPlatform) -> BuddyAccountIndex {
    load_index_checked(platform).unwrap_or_else(|_| {
        index_path(platform)
            .ok()
            .and_then(|p| fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    })
}
