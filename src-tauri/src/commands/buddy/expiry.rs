//! CodeBuddy CN 的「过期时间列」schema：全局一份，列 = { id, 自定义名称 }。
//! 每个账号按列 id 存一个时间值（见 BuddyAccount::expiry_times）。WorkBuddy 不使用。

use std::collections::HashSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::store;
use crate::commands::config::get_data_dir;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpiryColumn {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExpiryColumnFile {
    version: String,
    columns: Vec<ExpiryColumn>,
}

fn columns_path() -> PathBuf {
    get_data_dir().join("buddy").join("expiry_columns.json")
}

/// 读取列定义（无文件返回空列表）。
pub fn load_columns() -> Vec<ExpiryColumn> {
    let path = columns_path();
    if !path.exists() {
        return Vec::new();
    }
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|c| serde_json::from_str::<ExpiryColumnFile>(&c).ok())
        .map(|f| f.columns)
        .unwrap_or_default()
}

/// 校验并保存列定义，返回规范化后的列表。id 非空且唯一、name 非空。
pub fn save_columns(columns: Vec<ExpiryColumn>) -> Result<Vec<ExpiryColumn>, String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut cleaned: Vec<ExpiryColumn> = Vec::new();
    for col in columns {
        let id = col.id.trim().to_string();
        let name = col.name.trim().to_string();
        if id.is_empty() {
            return Err("列 id 不能为空".to_string());
        }
        if name.is_empty() {
            return Err(format!("列名称不能为空: id={}", id));
        }
        if !seen.insert(id.clone()) {
            return Err(format!("列 id 重复: {}", id));
        }
        cleaned.push(ExpiryColumn { id, name });
    }
    let file = ExpiryColumnFile {
        version: "1.0".to_string(),
        columns: cleaned.clone(),
    };
    let content = serde_json::to_string_pretty(&file)
        .map_err(|e| format!("序列化过期时间列失败: {}", e))?;
    store::write_atomic(&columns_path(), &content)?;
    Ok(cleaned)
}
