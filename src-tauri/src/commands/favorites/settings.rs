//! 收藏模块的界面设置：`get_data_dir()/favorites_settings.json`。
//!
//! 刻意与收藏库（`favorites.db`）分开：这里全是**界面偏好**（分类栏宽度、
//! 上次归类用的 AI 供应商/模型），丢了只是回默认值，不该和收藏数据同生共死；
//! 单独一个 JSON 也方便用户直接查看与修改。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::commands::config::{atomic_write_file, get_data_dir};

/// 分类栏默认宽度（px）。原先是固定的 160px（Tailwind `w-40`），这里加宽 20px 后
/// 作为新的默认值；用户拖动后以设置里的值为准。
pub const DEFAULT_LEFT_WIDTH: f64 = 180.0;
/// 下限保证分类名还能看全，上限避免把右侧条目列表挤没。
pub const MIN_LEFT_WIDTH: f64 = 140.0;
pub const MAX_LEFT_WIDTH: f64 = 420.0;

/// 收藏模块的界面设置。字段缺失时各自回默认，方便旧文件升级。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FavoriteSettings {
    /// 分类栏宽度（px）
    #[serde(default = "default_left_width")]
    pub left_width: f64,
    /// 上次 AI 归类用的供应商；None = 沿用全局默认 / 首个可用供应商
    #[serde(default)]
    pub provider_id: Option<String>,
    /// 上次 AI 归类用的模型（与 `provider_id` 成对保存）
    #[serde(default)]
    pub model_id: Option<String>,
}

fn default_left_width() -> f64 {
    DEFAULT_LEFT_WIDTH
}

impl Default for FavoriteSettings {
    fn default() -> Self {
        Self {
            left_width: DEFAULT_LEFT_WIDTH,
            provider_id: None,
            model_id: None,
        }
    }
}

/// 设置文件路径。
pub fn settings_path() -> PathBuf {
    get_data_dir().join("favorites_settings.json")
}

/// 宽度钳制：坏值（NaN / 无穷 / 非正数）回默认，越界收敛到上下限。
/// 前端拖动已经限过范围，这里再兜一次——文件是用户可以手改的。
fn clamp_left_width(width: f64) -> f64 {
    if !width.is_finite() || width <= 0.0 {
        DEFAULT_LEFT_WIDTH
    } else {
        width.clamp(MIN_LEFT_WIDTH, MAX_LEFT_WIDTH)
    }
}

/// 读设置：文件不存在 / JSON 坏 / 字段类型不对，一律回默认值，不报错。
pub fn load_settings() -> FavoriteSettings {
    let mut settings = std::fs::read_to_string(settings_path())
        .ok()
        .and_then(|data| serde_json::from_str::<FavoriteSettings>(&data).ok())
        .unwrap_or_default();
    settings.left_width = clamp_left_width(settings.left_width);
    settings
}

/// 写设置（原子写）。
pub fn save_settings(settings: &FavoriteSettings) -> Result<(), String> {
    let mut normalized = settings.clone();
    normalized.left_width = clamp_left_width(normalized.left_width);
    let data = serde_json::to_string_pretty(&normalized).map_err(|e| e.to_string())?;
    atomic_write_file(&settings_path(), data.as_bytes())
}

#[tauri::command]
pub fn fav_get_settings() -> FavoriteSettings {
    load_settings()
}

#[tauri::command]
pub fn fav_save_settings(settings: FavoriteSettings) -> Result<(), String> {
    save_settings(&settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_use_widened_column() {
        let settings = FavoriteSettings::default();
        assert_eq!(settings.left_width, DEFAULT_LEFT_WIDTH);
        assert!(settings.provider_id.is_none());
        assert!(settings.model_id.is_none());
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let settings: FavoriteSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings.left_width, DEFAULT_LEFT_WIDTH);
        // 只写了模型没写供应商：允许（读侧会整体回退），但值要原样保留
        let partial: FavoriteSettings =
            serde_json::from_str(r#"{"providerId":"p1","modelId":"m1"}"#).unwrap();
        assert_eq!(partial.provider_id.as_deref(), Some("p1"));
        assert_eq!(partial.model_id.as_deref(), Some("m1"));
    }

    #[test]
    fn width_is_clamped_and_bad_values_fall_back() {
        assert_eq!(clamp_left_width(10.0), MIN_LEFT_WIDTH);
        assert_eq!(clamp_left_width(9999.0), MAX_LEFT_WIDTH);
        assert_eq!(clamp_left_width(200.0), 200.0);
        assert_eq!(clamp_left_width(0.0), DEFAULT_LEFT_WIDTH);
        assert_eq!(clamp_left_width(-5.0), DEFAULT_LEFT_WIDTH);
        assert_eq!(clamp_left_width(f64::NAN), DEFAULT_LEFT_WIDTH);
        assert_eq!(clamp_left_width(f64::INFINITY), DEFAULT_LEFT_WIDTH);
    }

    #[test]
    fn serializes_as_camel_case() {
        let settings = FavoriteSettings {
            left_width: 200.0,
            provider_id: Some("p1".to_string()),
            model_id: Some("m1".to_string()),
        };
        let json = serde_json::to_string(&settings).unwrap();
        assert!(json.contains("\"leftWidth\""));
        assert!(json.contains("\"providerId\""));
        assert!(json.contains("\"modelId\""));
        let back: FavoriteSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back, settings);
    }
}
