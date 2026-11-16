//! SSH 私钥库：把用户选择的私钥文件复制到应用数据目录统一管理。
//!
//! 只做「复制 + 列出」，不改动用户原文件；私钥口令不在这里存储，
//! 仍由连接记录里的 `key_passphrase`（ENC_V2: 加密）保存。
use crate::commands::foundation;
use std::path::PathBuf;

fn store_dir() -> PathBuf {
    foundation::get_data_dir().join("ssh").join("keys")
}

/// 只保留最后一段文件名并替换路径分隔符/Windows 非法字符，防止写出目标目录。
pub fn safe_key_name(input: &str) -> String {
    let last = input.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = last
        .chars()
        .map(|c| {
            // 空格也替换：密钥路径会被拼进 shell 命令，避免引用问题
            if c.is_control() || c == ' ' || matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches('.').trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        "key".to_string()
    } else {
        trimmed.to_string()
    }
}

/// OpenSSH / RSA / PKCS#8 / EC 私钥都以 PEM 头开始；公钥与随机文件不算。
pub fn is_private_key(text: &str) -> bool {
    text.lines()
        .next()
        .map(|line| {
            let t = line.trim();
            t.starts_with("-----BEGIN") && t.contains("PRIVATE KEY-----")
        })
        .unwrap_or(false)
}

/// 复制一份私钥到应用数据目录，返回库内路径。原文件不动。
pub fn import(src: &str) -> Result<String, String> {
    let text = std::fs::read_to_string(src).map_err(|e| format!("读取私钥失败: {e}"))?;
    if !is_private_key(&text) {
        return Err("该文件不是私钥（缺少 PEM 私钥头）".to_string());
    }
    let dir = store_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建密钥库目录失败: {e}"))?;
    let name = safe_key_name(src);
    let dest = dir.join(&name);
    // 同名覆盖前先写临时文件再改名，避免中途失败留下半个私钥
    let tmp = dir.join(format!("{name}.tmp"));
    std::fs::write(&tmp, text.as_bytes()).map_err(|e| format!("写入密钥失败: {e}"))?;
    std::fs::rename(&tmp, &dest).map_err(|e| format!("保存密钥失败: {e}"))?;
    Ok(dest.to_string_lossy().to_string())
}

/// 列出已导入的密钥库文件（绝对路径），供前端下拉选择。
pub fn list() -> Vec<String> {
    let dir = store_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().and_then(|e| e.to_str()) != Some("tmp"))
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    out.sort();
    out
}
