use super::*;

/// 浏览器 UA：Suno 对非浏览器 UA 可能返回不同的内容，统一伪装成桌面浏览器。
pub(super) const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

// ─── 已下载缓存 ───

/// 已下载记录缓存：`<data_dir>/suno/downloaded.json`，存一个 id 数组。
pub(super) fn downloaded_path() -> std::path::PathBuf {
    crate::commands::config::get_data_dir()
        .join("suno")
        .join("downloaded.json")
}

pub(super) fn load_downloaded() -> std::collections::HashSet<String> {
    let Ok(content) = std::fs::read_to_string(downloaded_path()) else {
        return std::collections::HashSet::new();
    };
    serde_json::from_str::<Vec<String>>(&content)
        .unwrap_or_default()
        .into_iter()
        .collect()
}

pub(super) fn mark_downloaded(id: &str) {
    let mut set = load_downloaded();
    if set.contains(id) {
        return;
    }
    set.insert(id.to_string());
    let path = downloaded_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut ids: Vec<String> = set.into_iter().collect();
    ids.sort();
    if let Ok(json) = serde_json::to_string_pretty(&ids) {
        if let Err(e) = std::fs::write(&path, json) {
            eprintln!("[Suno] 写入已下载缓存失败: {e}");
        }
    }
}

// ─── 主页收藏 ───

/// 只认 Suno 用户主页：`https://suno.com/@username`（支持 www 前缀、无协议、http）。
pub(super) fn is_profile_url(url: &str) -> bool {
    let lower = url.trim().to_lowercase();
    let no_proto = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
        .unwrap_or(&lower);
    let (host, rest) = match no_proto.split_once('/') {
        Some((h, r)) => (h, r),
        None => (no_proto, ""),
    };
    let host_ok = host == "suno.com" || host == "www.suno.com";
    let name = rest.trim_start_matches('/').trim_end_matches('/');
    host_ok && name.starts_with('@') && name.len() > 1
}

/// 归一化主页 URL（去尾斜杠、统一小写），用于收藏去重。
pub(super) fn normalize_profile_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_lowercase()
}

pub(super) fn profiles_path() -> std::path::PathBuf {
    crate::commands::config::get_data_dir()
        .join("suno")
        .join("profiles.json")
}

pub(super) fn load_profiles() -> Vec<String> {
    let Ok(content) = std::fs::read_to_string(profiles_path()) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<String>>(&content).unwrap_or_default()
}

pub(super) fn save_profiles(profiles: &[String]) -> Result<(), String> {
    let path = profiles_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let json = serde_json::to_string_pretty(profiles).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("写入收藏失败: {e}"))
}
