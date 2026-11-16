//! SFTP 路径与错误的纯逻辑（不触网）。
//!
//! SFTP 在线路上永远是 POSIX 风格路径（`/`），与本机 Windows 路径无关；
//! 这里只做规范化，不解析符号链接、不展开 `~`（家目录由远端 canonicalize 决定）。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SftpErrorKind {
    NotFound,
    Permission,
    Timeout,
    Disconnected,
    Other,
}

pub struct SftpErrorView {
    pub kind: SftpErrorKind,
    pub message: String,
}

/// 把任意输入解析成远端绝对路径。
/// - 以 `/` 开头视为绝对；`~` 开头原样交给远端展开
/// - 其余相对当前目录；`.` / `..` 按 POSIX 折叠，`..` 不得越过根
/// - 拒绝 NUL 与其它控制字符（避免把非法字节送进协议包）
pub fn normalize_remote_path(current: &str, input: &str) -> Result<String, String> {
    if input.chars().any(|c| c.is_control()) {
        return Err("远端路径不能包含控制字符".to_string());
    }
    // `~` 的家目录展开由远端 realpath 决定，本地不猜测当前用户的 home。
    if input.starts_with('~') {
        return Ok(input.to_string());
    }
    let combined = if input.starts_with('/') {
        input.to_string()
    } else {
        format!("{}/{}", current.trim_end_matches('/'), input)
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in combined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    Ok(format!("/{}", parts.join("/")))
}

/// 下载到本地时用的文件名：只保留最后一段，去掉路径分隔符与控制字符。
/// 防止远端恶意文件名（`../../...`）写出目标目录之外。
pub fn safe_local_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = last
        .chars()
        .map(|c| if c.is_control() || c == ':' || c == '*' || c == '?' || c == '"' || c == '<' || c == '>' || c == '|' { '_' } else { c })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches('.');
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        "download".to_string()
    } else {
        trimmed.to_string()
    }
}

/// SFTP/SSH 错误分类。**只回显类别，不回显原始异常串**（可能含路径、用户名）。
pub fn classify_sftp_error(raw: &str) -> SftpErrorView {
    let lower = raw.to_ascii_lowercase();
    let (kind, message) = if lower.contains("no such file") || lower.contains("not found") {
        (SftpErrorKind::NotFound, "远端文件或目录不存在".to_string())
    } else if lower.contains("permission denied") || lower.contains("access denied") {
        (SftpErrorKind::Permission, "没有权限执行该操作".to_string())
    } else if lower.contains("timeout") {
        (SftpErrorKind::Timeout, "操作超时，请检查网络后重试".to_string())
    } else if lower.contains("connection closed") || lower.contains("closed") || lower.contains("disconnect") {
        (SftpErrorKind::Disconnected, "SFTP 连接已断开，请重新打开".to_string())
    } else {
        (SftpErrorKind::Other, "SFTP 操作失败".to_string())
    };
    SftpErrorView { kind, message }
}
