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

/// AI 检索栏（常驻右栏，不再是弹窗）：默认 360px，范围同样做了钳制。
pub const DEFAULT_AI_WIDTH: f64 = 360.0;
pub const MIN_AI_WIDTH: f64 = 260.0;
pub const MAX_AI_WIDTH: f64 = 640.0;

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
    /// 收藏检索 Agent 的单轮工具循环上限（与思维导图「Agent 轮数」同一口径：
    /// 每轮一次 LLM 调用，调大更会找但更费 token）
    #[serde(default = "default_agent_rounds")]
    pub agent_rounds: usize,
    /// AI 检索栏宽度（px）：常驻右栏，可拖动
    #[serde(default = "default_ai_width")]
    pub ai_width: f64,
    /// AI 检索栏是否展开（关掉后条目列表占满）
    #[serde(default = "default_ai_open")]
    pub ai_open: bool,
}

fn default_ai_width() -> f64 {
    DEFAULT_AI_WIDTH
}

fn default_ai_open() -> bool {
    true
}

fn default_agent_rounds() -> usize {
    DEFAULT_AGENT_ROUNDS
}

/// Agent 轮数的上下限：1 轮 = 只搜一次就整理；20 轮足够兜住「换好几个词」的场景。
pub const MIN_AGENT_ROUNDS: usize = 1;
pub const MAX_AGENT_ROUNDS: usize = 20;
pub const DEFAULT_AGENT_ROUNDS: usize = 6;

fn default_left_width() -> f64 {
    DEFAULT_LEFT_WIDTH
}

impl Default for FavoriteSettings {
    fn default() -> Self {
        Self {
            left_width: DEFAULT_LEFT_WIDTH,
            provider_id: None,
            model_id: None,
            agent_rounds: DEFAULT_AGENT_ROUNDS,
            ai_width: DEFAULT_AI_WIDTH,
            ai_open: true,
        }
    }
}

/// Agent 轮数钳制：0 / NaN 式的坏值回默认，越界收敛到 1..=20（与 `left_width` 同一套路）。
pub fn clamp_agent_rounds(rounds: usize) -> usize {
    rounds.clamp(MIN_AGENT_ROUNDS, MAX_AGENT_ROUNDS)
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

/// AI 检索栏宽度钳制（与左栏同一套路：坏值回默认，越界收敛）。
fn clamp_ai_width(width: f64) -> f64 {
    if !width.is_finite() || width <= 0.0 {
        DEFAULT_AI_WIDTH
    } else {
        width.clamp(MIN_AI_WIDTH, MAX_AI_WIDTH)
    }
}

/// 读设置：文件不存在 / JSON 坏 / 字段类型不对，一律回默认值，不报错。
pub fn load_settings() -> FavoriteSettings {
    let mut settings = std::fs::read_to_string(settings_path())
        .ok()
        .and_then(|data| serde_json::from_str::<FavoriteSettings>(&data).ok())
        .unwrap_or_default();
    settings.left_width = clamp_left_width(settings.left_width);
    settings.agent_rounds = clamp_agent_rounds(settings.agent_rounds);
    settings.ai_width = clamp_ai_width(settings.ai_width);
    settings
}

/// 写设置（原子写）。
pub fn save_settings(settings: &FavoriteSettings) -> Result<(), String> {
    let mut normalized = settings.clone();
    normalized.left_width = clamp_left_width(normalized.left_width);
    normalized.agent_rounds = clamp_agent_rounds(normalized.agent_rounds);
    normalized.ai_width = clamp_ai_width(normalized.ai_width);
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
            agent_rounds: 8,
            ai_width: 400.0,
            ai_open: true,
        };
        let json = serde_json::to_string(&settings).unwrap();
        assert!(json.contains("\"leftWidth\""));
        assert!(json.contains("\"providerId\""));
        assert!(json.contains("\"modelId\""));
        // Agent 轮数同样按 camelCase 下发，前端按 agentRounds 读
        assert!(json.contains("\"agentRounds\""));
        // AI 检索右栏：宽度与显隐同样按 camelCase 下发
        assert!(json.contains("\"aiWidth\""));
        assert!(json.contains("\"aiOpen\""));
        let back: FavoriteSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back, settings);
    }
}
