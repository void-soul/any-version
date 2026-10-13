//! 数据库设计器设置：存储在数据目录 `db_designer_settings.json`。
//!
//! 目前只有一件事要记：**上次使用的 AI 模型**。
//!
//! 为什么需要它：主模块切换用的是 React 19.2 的 `<Activity>` keep-alive，
//! 切走时只销毁 Effects、DOM 与 state 原样保留，切回时重建 Effects。
//! 于是面板挂载时那个「读 AI 配置 → 选中默认模型」的 effect 会**再跑一遍**，
//! 把用户刚选的模型覆盖回第一个供应商的激活模型 —— 表现就是
//! 「选了模型 A，切走再切回来变成默认模型」。同一套写法在思维导图
//! （`mindmap_settings.json`）与收藏（`favorites_settings.json`）里早就有了，
//! 这里补上，行为才对得齐。

use serde::{Deserialize, Serialize};

/// 数据库设计器设置（AI 模型记忆）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DbDesignerSettings {
    /// 上次使用的 AI 供应商 id（None = 尚未选择过，前端回退到默认供应商）
    pub last_provider_id: Option<String>,
    /// 上次使用的 AI 模型 id（None = 尚未选择过，前端回退到激活模型）
    pub last_model_id: Option<String>,
}

impl Default for DbDesignerSettings {
    fn default() -> Self {
        Self {
            last_provider_id: None,
            last_model_id: None,
        }
    }
}

fn settings_path() -> std::path::PathBuf {
    crate::commands::config::get_data_dir().join("db_designer_settings.json")
}

/// 从磁盘读取（缺失/损坏时回退默认值）。
pub fn load_settings() -> DbDesignerSettings {
    let path = settings_path();
    if path.exists() {
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(cfg) = serde_json::from_str::<DbDesignerSettings>(&data) {
                return cfg;
            }
        }
    }
    DbDesignerSettings::default()
}

fn store_settings(cfg: &DbDesignerSettings) -> Result<(), String> {
    let data = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    crate::commands::config::atomic_write_file(&settings_path(), data.as_bytes())
}

#[tauri::command]
pub fn dbd_get_settings() -> DbDesignerSettings {
    load_settings()
}

#[tauri::command]
pub fn dbd_save_settings(settings: DbDesignerSettings) -> Result<DbDesignerSettings, String> {
    store_settings(&settings)?;
    Ok(load_settings())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 旧文件（没有 last* 字段）必须能读 —— 不因新增字段而破坏已有配置。
    #[test]
    fn legacy_json_without_last_model_fields_is_accepted() {
        let legacy = r#"{}"#;
        let cfg: DbDesignerSettings = serde_json::from_str(legacy).unwrap();
        assert!(cfg.last_provider_id.is_none());
        assert!(cfg.last_model_id.is_none());
    }

    #[test]
    fn round_trip_preserves_values() {
        let cfg = DbDesignerSettings {
            last_provider_id: Some("openai".to_string()),
            last_model_id: Some("gpt-5".to_string()),
        };
        let back: DbDesignerSettings =
            serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
        assert_eq!(back.last_provider_id.as_deref(), Some("openai"));
        assert_eq!(back.last_model_id.as_deref(), Some("gpt-5"));
    }
}
