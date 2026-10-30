use super::*;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  远程版本缓存（磁盘 JSON）
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 磁盘缓存格式
#[derive(Serialize, Deserialize)]
pub(super) struct VersionCache {
    pub(super) versions: Vec<String>,
    pub(super) updated_at: u64, // Unix 时间戳（秒）
}

/// 命令返回值（含 updated_at 供前端显示"上次更新时间"）
#[derive(Serialize, Clone)]
pub struct RemoteVersionsResult {
    pub versions: Vec<String>,
    pub updated_at: u64,
    pub from_cache: bool,
}

pub(super) fn version_cache_path(project_id: &str) -> PathBuf {
    get_data_dir()
        .join("version_cache")
        .join(format!("{}.json", project_id))
}

/// 版本缓存有效期：超过后视为过期，下次读取强制联网刷新（否则新版本永不出现）。
pub(super) const VERSION_CACHE_TTL_SECS: u64 = 24 * 3600;

pub(super) fn load_version_cache(project_id: &str) -> Option<VersionCache> {
    let path = version_cache_path(project_id);
    let content = fs::read_to_string(&path).ok()?;
    let cache: VersionCache = serde_json::from_str(&content).ok()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if now.saturating_sub(cache.updated_at) > VERSION_CACHE_TTL_SECS {
        return None; // 过期，视为无缓存
    }
    Some(cache)
}

pub(super) fn save_version_cache(project_id: &str, versions: &[String]) -> u64 {
    let updated_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let cache = VersionCache {
        versions: versions.to_vec(),
        updated_at,
    };
    let path = version_cache_path(project_id);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string(&cache) {
        let _ = fs::write(&path, json);
    }
    updated_at
}
