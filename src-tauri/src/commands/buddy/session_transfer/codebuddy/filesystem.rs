use super::*;

pub(super) fn temp_name(prefix: &str) -> String {
    format!(
        ".kira-session-{}-{}-{}",
        prefix,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}

pub(super) fn copy_dir_atomic(source: &Path, target: &Path) -> Result<(), String> {
    if target.exists() {
        return Ok(());
    }
    let parent = target
        .parent()
        .ok_or_else(|| format!("无法定位目标目录: {}", target.display()))?;
    std::fs::create_dir_all(parent).map_err(|e| {
        format!(
            "创建 CodeBuddy CN 会话目标目录失败: path={}, error={}",
            parent.display(),
            e
        )
    })?;
    let tmp = parent.join(temp_name("tmp"));
    let result = copy_dir_recursive(source, &tmp).and_then(|_| {
        std::fs::rename(&tmp, target).map_err(|e| {
            format!(
                "提交 CodeBuddy CN 会话目录失败: from={}, to={}, error={}",
                tmp.display(),
                target.display(),
                e
            )
        })
    });
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    result
}

pub(super) fn replace_dir_atomic(source: &Path, target: &Path) -> Result<(), String> {
    // 共享目录（junction/symlink）不能替换：rename 会先把链接改名、再用真实目录顶替，
    // 等于把共享链接物化。共享模式下复制本来就该被跳过，这里只做兜底。
    if is_reparse_point(target) {
        eprintln!(
            "[Buddy CN Transfer] 目标是共享链接，跳过覆盖: {}",
            target.display()
        );
        return Ok(());
    }
    let parent = target
        .parent()
        .ok_or_else(|| format!("无法定位目标目录: {}", target.display()))?;
    std::fs::create_dir_all(parent).map_err(|e| format!("创建目标目录失败: {}", e))?;
    let tmp = parent.join(temp_name("replace"));
    copy_dir_recursive(source, &tmp)?;
    let old = parent.join(temp_name("old"));
    if target.exists() {
        std::fs::rename(target, &old).map_err(|e| {
            format!(
                "暂存旧 CodeBuddy CN 会话目录失败: from={}, to={}, error={}",
                target.display(),
                old.display(),
                e
            )
        })?;
    }
    if let Err(e) = std::fs::rename(&tmp, target) {
        let _ = std::fs::remove_dir_all(&tmp);
        if old.exists() {
            let _ = std::fs::rename(&old, target);
        }
        return Err(format!(
            "替换 CodeBuddy CN 会话目录失败: path={}, error={}",
            target.display(),
            e
        ));
    }
    if old.exists() {
        let _ = std::fs::remove_dir_all(&old);
    }
    Ok(())
}

pub(super) fn copy_dir_recursive(source: &Path, target: &Path) -> Result<(), String> {
    let source_metadata =
        std::fs::symlink_metadata(source).map_err(|e| format!("读取源文件属性失败: {}", e))?;
    if source_metadata.file_type().is_symlink() {
        return Err(format!("拒绝复制符号链接: {}", source.display()));
    }
    if source_metadata.is_file() {
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建目标目录失败: {}", e))?;
        }
        std::fs::copy(source, target).map_err(|e| {
            format!(
                "复制 CodeBuddy CN 会话文件失败: from={}, to={}, error={}",
                source.display(),
                target.display(),
                e
            )
        })?;
        return Ok(());
    }
    if !source_metadata.is_dir() {
        return Err(format!(
            "不支持的 CodeBuddy CN 会话文件类型: {}",
            source.display()
        ));
    }
    std::fs::create_dir_all(target)
        .map_err(|e| format!("创建 CodeBuddy CN 会话目录失败: {}", e))?;
    for entry in
        std::fs::read_dir(source).map_err(|e| format!("读取 CodeBuddy CN 会话目录失败: {}", e))?
    {
        let entry = entry.map_err(|e| format!("读取 CodeBuddy CN 会话条目失败: {}", e))?;
        copy_dir_recursive(&entry.path(), &target.join(entry.file_name()))?;
    }
    Ok(())
}
