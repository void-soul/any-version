//! SSH 连接参数与错误分类的纯逻辑（不触网）。
use std::fmt;

#[derive(Clone)]
pub struct SshTarget {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    /// Some = 私钥认证（口令可选）；None = 密码认证。
    pub key_auth: Option<KeyAuth>,
}

/// 私钥认证参数。口令绝不进 Debug/日志。
#[derive(Clone)]
pub struct KeyAuth {
    pub path: String,
    pub passphrase: Option<String>,
}

impl fmt::Debug for KeyAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyAuth")
            .field("path", &self.path)
            .field("passphrase", &"<redacted>")
            .finish()
    }
}

pub fn validate_key_auth(auth: &KeyAuth) -> Result<(), String> {
    if auth.path.trim().is_empty() {
        return Err("私钥文件路径不能为空".to_string());
    }
    Ok(())
}

/// 密码绝不进 Debug/日志。
impl fmt::Debug for SshTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SshTarget")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SshErrorKind {
    Auth,
    Refused,
    Timeout,
    Dns,
    HostKey,
    Other,
}

pub struct SshErrorView {
    pub kind: SshErrorKind,
    pub message: String,
}

/// 规整并校验连接参数。IPv6 字面量的方括号、首尾空白都容忍，但空值/端口 0 直接拒绝。
pub fn normalize_target(mut t: SshTarget) -> Result<SshTarget, String> {
    t.host = t.host.trim().trim_start_matches('[').trim_end_matches(']').trim().to_string();
    t.username = t.username.trim().to_string();
    if t.host.is_empty() {
        return Err("主机地址不能为空".to_string());
    }
    if t.username.is_empty() {
        return Err("用户名不能为空".to_string());
    }
    if t.port == 0 {
        return Err("端口不能为 0".to_string());
    }
    Ok(t)
}

/// xterm 的 cols/rows 夹在安全区间，避免异常尺寸把 PTY 请求打爆。
pub fn terminal_size(cols: u32, rows: u32) -> (u32, u32) {
    const MAX_COLS: u32 = 500;
    const MAX_ROWS: u32 = 200;
    let cols = if cols == 0 { 80 } else { cols.min(MAX_COLS) };
    let rows = if rows == 0 { 24 } else { rows.min(MAX_ROWS) };
    (cols, rows)
}

/// 把 russh/OS 的原始错误翻成用户可读文案。
/// **只回显错误类别，不回显原始异常串**（可能含用户名、路径、密钥指纹等）。
pub fn classify_ssh_error(raw: &str) -> SshErrorView {
    let lower = raw.to_ascii_lowercase();
    let (kind, message) = if lower.contains("authentication")
        || lower.contains("permission denied")
        || lower.contains("auth failed")
    {
        (SshErrorKind::Auth, "认证失败：用户名或密码不正确，或该用户不允许密码登录".to_string())
    } else if lower.contains("refused") || lower.contains("10061") {
        (SshErrorKind::Refused, "连接被拒绝：目标主机的 SSH 服务未启动或端口不对".to_string())
    } else if lower.contains("timed out") || lower.contains("timeout") || lower.contains("10060") {
        (SshErrorKind::Timeout, "连接超时：主机不可达或被防火墙拦截".to_string())
    } else if lower.contains("no such host") || lower.contains("dns") || lower.contains("name or service not known") {
        (SshErrorKind::Dns, "域名解析失败：请检查主机地址".to_string())
    } else if lower.contains("host key") || lower.contains("key exchange") {
        (SshErrorKind::HostKey, "主机密钥校验或密钥交换失败".to_string())
    } else {
        (SshErrorKind::Other, "SSH 连接失败".to_string())
    };
    SshErrorView { kind, message }
}
