//! 音乐播放器的用户设置（音量 / 播放模式 / 音效），持久化到 `data_dir/music/settings.json`。

use std::fs;

use serde::{Deserialize, Serialize};

use super::dsp::EqParams;

/// 支持的播放模式
pub const PLAY_MODES: [&str; 3] = ["sequence", "shuffle", "single"];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MusicSettings {
    /// 音量 0.0 ~ 1.0
    pub volume: f32,
    /// 播放模式：sequence（顺序）/ shuffle（随机）/ single（单曲循环）
    pub play_mode: String,
    pub eq: EqParams,
}

impl Default for MusicSettings {
    fn default() -> Self {
        Self {
            volume: 0.8,
            play_mode: "sequence".to_string(),
            eq: EqParams::default(),
        }
    }
}

impl MusicSettings {
    /// 夹回合法范围（防手改配置/前端传错）
    pub fn sanitized(mut self) -> Self {
        if !self.volume.is_finite() {
            self.volume = 0.8;
        }
        self.volume = self.volume.clamp(0.0, 1.0);
        if !PLAY_MODES.contains(&self.play_mode.as_str()) {
            self.play_mode = "sequence".to_string();
        }
        self.eq = self.eq.sanitized();
        self
    }
}

fn settings_path() -> std::path::PathBuf {
    super::library::music_dir().join("settings.json")
}

pub fn load_settings() -> MusicSettings {
    match fs::read_to_string(settings_path()) {
        Ok(text) => serde_json::from_str::<MusicSettings>(&text)
            .map(|s| s.sanitized())
            .unwrap_or_default(),
        Err(_) => MusicSettings::default(),
    }
}

pub fn save_settings(settings: &MusicSettings) -> Result<(), String> {
    let value = serde_json::to_value(settings).map_err(|e| e.to_string())?;
    super::library::write_json_atomic(&settings_path(), &value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_clamps_volume_and_mode() {
        let settings = MusicSettings {
            volume: 3.0,
            play_mode: "weird".to_string(),
            ..Default::default()
        }
        .sanitized();
        assert_eq!(settings.volume, 1.0);
        assert_eq!(settings.play_mode, "sequence");

        let settings = MusicSettings {
            volume: f32::NAN,
            play_mode: "shuffle".to_string(),
            ..Default::default()
        }
        .sanitized();
        assert_eq!(settings.volume, 0.8);
        assert_eq!(settings.play_mode, "shuffle");
    }

    #[test]
    fn test_default_is_valid() {
        let settings = MusicSettings::default().sanitized();
        assert_eq!(settings, MusicSettings::default());
    }
}
