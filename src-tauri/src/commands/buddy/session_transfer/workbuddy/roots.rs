use super::*;

pub(crate) fn select_extension_data_roots(
    source_uid: &str,
) -> Result<Vec<(&'static str, std::path::PathBuf)>, String> {
    let home = dirs::home_dir().ok_or_else(|| "无法获取用户主目录".to_string())?;
    #[cfg(target_os = "windows")]
    let base: std::path::PathBuf = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home.join("AppData").join("Local"));
    #[cfg(target_os = "macos")]
    let base = home.join("Library").join("Application Support");
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    let base = home.join(".local").join("share");

    let current = base.join("WorkBuddyExtension").join("Data");
    if account_has_history(&current, source_uid)? {
        return Ok(vec![("workbuddy-extension", current)]);
    }
    let legacy = base.join("CodeBuddyExtension").join("Data");
    if account_has_history(&legacy, source_uid)? {
        return Ok(vec![("legacy-codebuddy-extension", legacy)]);
    }
    Ok(Vec::new())
}

pub(super) fn account_has_history(extension_data_dir: &Path, uid: &str) -> Result<bool, String> {
    let account_root = extension_data_dir.join(uid);
    if !account_root.is_dir() {
        return Ok(false);
    }
    reject_symlink_if_exists(&account_root)?;
    for entry in std::fs::read_dir(&account_root).map_err(|e| {
        format!(
            "读取 WorkBuddy 会话根目录失败: path={}, error={}",
            account_root.display(),
            e
        )
    })? {
        let entry = entry.map_err(|e| format!("读取 WorkBuddy 会话目录失败: {}", e))?;
        let metadata = std::fs::symlink_metadata(entry.path())
            .map_err(|e| format!("读取 WorkBuddy 会话目录属性失败: {}", e))?;
        if metadata.is_dir()
            && !metadata.file_type().is_symlink()
            && entry.path().join(uid).join("history").is_dir()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn reject_symlink_if_exists(path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => Err(format!(
            "拒绝通过符号链接读写 WorkBuddy 会话: {}",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!(
            "读取 WorkBuddy 会话路径属性失败: path={}, error={}",
            path.display(),
            e
        )),
    }
}
