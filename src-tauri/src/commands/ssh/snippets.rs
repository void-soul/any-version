//! 用户自定义快捷命令（FinalShell 式）：按目录分组，每条命令可多行。
//!
//! 存 `get_data_dir()/ssh/snippets.json`：这是用户资产，不放 WebView 的
//! localStorage——换机器、清缓存都不该丢。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SnippetFile {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub folders: Vec<SnippetFolder>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnippetFolder {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub items: Vec<SnippetItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnippetItem {
    pub id: String,
    pub name: String,
    /// 可多行；发送时逐行执行。
    pub command: String,
}

fn default_version() -> u32 {
    1
}

fn default_path() -> Result<PathBuf, String> {
    Ok(crate::commands::config::get_data_dir()
        .join("ssh")
        .join("snippets.json"))
}

pub fn load() -> Result<SnippetFile, String> {
    load_from(&default_path()?)
}

pub fn save(data: &SnippetFile) -> Result<(), String> {
    save_to(&default_path()?, data)
}

pub fn load_from(path: &Path) -> Result<SnippetFile, String> {
    if !path.exists() {
        return Ok(SnippetFile {
            version: 1,
            folders: Vec::new(),
        });
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("读取快捷命令失败: {e}"))?;
    let data: SnippetFile =
        serde_json::from_str(&text).map_err(|e| format!("快捷命令文件格式错误: {e}"))?;
    validate(&data)?;
    Ok(data)
}

pub fn save_to(path: &Path, data: &SnippetFile) -> Result<(), String> {
    validate(data)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建快捷命令目录失败: {e}"))?;
    }
    let text = serde_json::to_vec_pretty(data).map_err(|e| format!("序列化快捷命令失败: {e}"))?;
    crate::commands::foundation::atomic_write_file(path, &text)
}

/// 目录/命令 id 必须全局唯一，名称与命令内容不能为空——前端整体覆盖保存，
/// 这里做最后一道校验，避免坏数据落盘后整个面板打不开。
pub fn validate(data: &SnippetFile) -> Result<(), String> {
    let mut folder_ids = HashSet::new();
    let mut item_ids = HashSet::new();
    for folder in &data.folders {
        let id = folder.id.trim();
        if id.is_empty() {
            return Err("快捷命令目录 id 不能为空".to_string());
        }
        if !folder_ids.insert(id) {
            return Err(format!("快捷命令目录 id 重复: {id}"));
        }
        if folder.name.trim().is_empty() {
            return Err("快捷命令目录名称不能为空".to_string());
        }
        for item in &folder.items {
            let item_id = item.id.trim();
            if item_id.is_empty() {
                return Err("快捷命令 id 不能为空".to_string());
            }
            if !item_ids.insert(item_id) {
                return Err(format!("快捷命令 id 重复: {item_id}"));
            }
            if item.name.trim().is_empty() {
                return Err("快捷命令名称不能为空".to_string());
            }
            if item.command.trim().is_empty() {
                return Err(format!("快捷命令「{}」内容为空", item.name));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let mut random = [0u8; 8];
        getrandom::getrandom(&mut random).unwrap();
        std::env::temp_dir().join(format!(
            "kira-snippets-{name}-{:016x}.json",
            u64::from_ne_bytes(random)
        ))
    }

    fn sample() -> SnippetFile {
        SnippetFile {
            version: 1,
            folders: vec![SnippetFolder {
                id: "f1".into(),
                name: "服务".into(),
                items: vec![SnippetItem {
                    id: "s1".into(),
                    name: "重启 nginx".into(),
                    command: "systemctl restart nginx\nsystemctl status nginx --no-pager".into(),
                }],
            }],
        }
    }

    #[test]
    fn missing_file_returns_empty() {
        let path = temp_path("missing");
        let data = load_from(&path).unwrap();
        assert!(data.folders.is_empty());
    }

    #[test]
    fn roundtrip_preserves_multiline_command() {
        let path = temp_path("roundtrip");
        save_to(&path, &sample()).unwrap();
        let data = load_from(&path).unwrap();
        assert_eq!(data.folders.len(), 1);
        assert_eq!(data.folders[0].items[0].command.lines().count(), 2);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn validate_rejects_duplicate_ids() {
        let mut data = sample();
        data.folders.push(data.folders[0].clone());
        let err = validate(&data).unwrap_err();
        assert!(err.contains("重复"), "实际: {err}");
    }

    #[test]
    fn validate_rejects_empty_command() {
        let mut data = sample();
        data.folders[0].items[0].command = "   ".into();
        let err = validate(&data).unwrap_err();
        assert!(err.contains("内容为空"), "实际: {err}");
    }
}
