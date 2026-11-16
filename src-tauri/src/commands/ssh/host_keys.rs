//! 应用自管的主机密钥信任库（阶段 3）。
//!
//! 格式：`host:port SHA256:<base64>`，每行一条。
//! 首次连接必须让用户看到指纹后显式信任；已信任但指纹变化一律拒绝，不自动改写。
use crate::commands::foundation;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 连接失败时回给前端的结构化提示前缀（纯字符串，避免为一条错误改整套错误类型）。
pub const HOST_KEY_PROMPT_PREFIX: &str = "__HOST_KEY_PROMPT__:";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostKeyDecision {
    Unknown,
    Changed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyPrompt {
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
    pub decision: HostKeyDecision,
}

pub fn encode_host_key_prompt(host: &str, port: u16, fingerprint: &str, decision: HostKeyDecision) -> String {
    let prompt = HostKeyPrompt {
        host: host.to_string(),
        port,
        fingerprint: fingerprint.to_string(),
        decision,
    };
    format!("{HOST_KEY_PROMPT_PREFIX}{}", serde_json::to_string(&prompt).unwrap_or_default())
}

pub fn decode_host_key_prompt(err: &str) -> Option<HostKeyPrompt> {
    let json = err.strip_prefix(HOST_KEY_PROMPT_PREFIX)?;
    serde_json::from_str(json).ok()
}

fn store_path() -> PathBuf {
    foundation::get_data_dir().join("ssh").join("known_hosts")
}

pub fn load_store() -> String {
    std::fs::read_to_string(store_path()).unwrap_or_default()
}

pub fn save_store(text: &str) -> Result<(), String> {
    let path = store_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("创建 SSH 配置目录失败: {e}"))?;
    }
    foundation::atomic_write_file(&path, text.as_bytes())
}

/// 写入用户已确认的指纹。Changed 也允许，但必须由前端明确二次确认后才调用。
pub fn trust(host: &str, port: u16, fingerprint: &str) -> Result<(), String> {
    let text = load_store();
    save_store(&upsert(&text, host, port, fingerprint))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustDecision {
    Trusted,
    Unknown,
    Changed,
}

/// 信任库里的主键。IPv6 字面量不加方括号（我们自己的格式，不复用 OpenSSH 的 `[host]:port`）。
pub fn host_key(host: &str, port: u16) -> String {
    format!("{host}:{port}")
}

pub fn parse_trusted(text: &str, host: &str, port: u16) -> Option<String> {
    let key = host_key(host, port);
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let k = parts.next()?;
            let fp = parts.next()?;
            (k == key).then(|| fp.to_string())
        })
}

pub fn trust_key(text: &str, host: &str, port: u16, fingerprint: &str) -> TrustDecision {
    match parse_trusted(text, host, port) {
        None => TrustDecision::Unknown,
        Some(recorded) if recorded == fingerprint => TrustDecision::Trusted,
        Some(_) => TrustDecision::Changed,
    }
}

/// 信任库里的一条记录，供 UI 展示与删除。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustedHost {
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
}

/// 解析整个信任库。跳过注释、空行、缺字段或端口非法的行，单行损坏不影响整体。
pub fn list_trusted(text: &str) -> Vec<TrustedHost> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let key = parts.next()?;
            let fingerprint = parts.next()?;
            // `host:port`：用最后一个冒号切分，IPv6 字面量（::1:22）也能正确还原
            let (host, port) = key.rsplit_once(':')?;
            if host.is_empty() {
                return None;
            }
            let port = port.parse::<u16>().ok().filter(|p| *p != 0)?;
            Some(TrustedHost {
                host: host.to_string(),
                port,
                fingerprint: fingerprint.to_string(),
            })
        })
        .collect()
}

/// 删除指定主机记录；其余行（含注释）原样保留。找不到时返回原文。
pub fn remove_host(text: &str, host: &str, port: u16) -> String {
    let key = host_key(host, port);
    let mut kept: Vec<&str> = Vec::new();
    let mut hit = false;
    for line in text.lines() {
        let trimmed = line.trim();
        let is_target = !trimmed.is_empty()
            && !trimmed.starts_with('#')
            && trimmed.split_whitespace().next() == Some(key.as_str());
        if is_target {
            hit = true;
            continue;
        }
        kept.push(line);
    }
    if !hit {
        return text.to_string();
    }
    let body = kept.join("\n");
    if body.trim().is_empty() {
        String::new()
    } else if body.ends_with('\n') {
        body
    } else {
        format!("{body}\n")
    }
}

/// 追加或替换一条信任记录。只在用户显式确认指纹后调用。
pub fn upsert(text: &str, host: &str, port: u16, fingerprint: &str) -> String {
    let key = host_key(host, port);
    let mut out: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter(|line| line.split_whitespace().next() != Some(key.as_str()))
        .map(str::to_string)
        .collect();
    out.push(format!("{key} {fingerprint}"));
    out.join("\n") + "\n"
}
