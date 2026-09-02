//! 思维导图 AI 探索参数的全局设置：存储在数据目录 `mindmap_settings.json`。
//!
//! 探索循环（AI 点单读文件）的轮数与每批预算原本是硬编码常量，现开放为可调参数，
//! 让用户按模型窗口/项目规模自行权衡「分析深度 ↔ 成本」。所有字段带硬钳制，
//! 防止极端值（如 0 轮、单文件 100MB）拖垮上下文或产生天价账单。

use serde::{Deserialize, Serialize};

/// 探索参数默认值（与历史硬编码常量一致，行为无迁移断层）。
pub const DEFAULT_EXPLORER_ROUNDS: u32 = 6;
pub const DEFAULT_EXPLORER_FILES_PER_ROUND: u32 = 8;
pub const DEFAULT_EXPLORER_CHARS_PER_FILE: u32 = 4000;
pub const DEFAULT_EXPLORER_BATCH_CHARS: u32 = 24000;

// 钳制范围：下限保证最小可用性，上限防止上下文爆炸/账单失控。
// 每轮上限 24 → 最多 24×6=144 文件，接近大仓库分析的现实上限。
pub const MIN_EXPLORER_ROUNDS: u32 = 1;
pub const MAX_EXPLORER_ROUNDS: u32 = 12;
pub const MIN_FILES_PER_ROUND: u32 = 1;
pub const MAX_FILES_PER_ROUND: u32 = 24;
pub const MIN_CHARS_PER_FILE: u32 = 500;
pub const MAX_CHARS_PER_FILE: u32 = 20_000;
pub const MIN_BATCH_CHARS: u32 = 4_000;
pub const MAX_BATCH_CHARS: u32 = 60_000;

/// 思维导图 AI 探索参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExplorerSettings {
    /// 探索最大轮数（AI 每轮点单一次；done 提前结束不受影响）
    pub explorer_rounds: u32,
    /// 每轮最多请求的文件数
    pub explorer_files_per_round: u32,
    /// 单文件压缩读取的字符上限
    pub explorer_chars_per_file: u32,
    /// 每轮追加内容的总字符预算
    pub explorer_batch_chars: u32,
}

impl Default for ExplorerSettings {
    fn default() -> Self {
        Self {
            explorer_rounds: DEFAULT_EXPLORER_ROUNDS,
            explorer_files_per_round: DEFAULT_EXPLORER_FILES_PER_ROUND,
            explorer_chars_per_file: DEFAULT_EXPLORER_CHARS_PER_FILE,
            explorer_batch_chars: DEFAULT_EXPLORER_BATCH_CHARS,
            }
    }
}

impl ExplorerSettings {
    /// 应用硬钳制：越界值拉回边界（而不是报错），保证任何来源的配置都安全可用。
    pub fn clamped(mut self) -> Self {
        self.explorer_rounds = self.explorer_rounds.clamp(MIN_EXPLORER_ROUNDS, MAX_EXPLORER_ROUNDS);
        self.explorer_files_per_round = self
            .explorer_files_per_round
            .clamp(MIN_FILES_PER_ROUND, MAX_FILES_PER_ROUND);
        self.explorer_chars_per_file = self
            .explorer_chars_per_file
            .clamp(MIN_CHARS_PER_FILE, MAX_CHARS_PER_FILE);
        // 每批预算不得小于单文件上限，否则 per_file 计算恒为 0
        self.explorer_batch_chars = self.explorer_batch_chars.clamp(
            self.explorer_chars_per_file.max(MIN_BATCH_CHARS),
            MAX_BATCH_CHARS,
        );
        self
    }
}

fn settings_path() -> std::path::PathBuf {
    crate::commands::config::get_data_dir().join("mindmap_settings.json")
}

/// 从磁盘读取（缺失/损坏时回退默认值并钳制）。
pub fn load_explorer_settings() -> ExplorerSettings {
    let path = settings_path();
    if path.exists() {
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(cfg) = serde_json::from_str::<ExplorerSettings>(&data) {
                return cfg.clamped();
            }
        }
    }
    ExplorerSettings::default().clamped()
}

/// 保存前钳制，防止把越界值写进磁盘。
fn store_explorer_settings(cfg: &ExplorerSettings) -> Result<(), String> {
    let clamped = cfg.clone().clamped();
    let data = serde_json::to_string_pretty(&clamped).map_err(|e| e.to_string())?;
    crate::commands::config::atomic_write_file(&settings_path(), data.as_bytes())
}

#[tauri::command]
pub fn mm_get_explorer_settings() -> ExplorerSettings {
    load_explorer_settings()
}

#[tauri::command]
pub fn mm_save_explorer_settings(settings: ExplorerSettings) -> Result<ExplorerSettings, String> {
    store_explorer_settings(&settings)?;
    Ok(load_explorer_settings())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_legacy_constants() {
        let d = ExplorerSettings::default();
        assert_eq!(d.explorer_rounds, 6);
        assert_eq!(d.explorer_files_per_round, 8);
        assert_eq!(d.explorer_chars_per_file, 4000);
        assert_eq!(d.explorer_batch_chars, 24_000);
    }

    #[test]
    fn clamping_pulls_out_of_range_values_to_bounds() {
        let s = ExplorerSettings {
            explorer_rounds: 0,
            explorer_files_per_round: 99,
            explorer_chars_per_file: 1,
            explorer_batch_chars: 1,
        }
        .clamped();
        assert_eq!(s.explorer_rounds, 1);
        assert_eq!(s.explorer_files_per_round, 24);
        assert_eq!(s.explorer_chars_per_file, 500);
        // batch_chars 被拉到 chars_per_file 之上（防 per_file 恒为 0）
        assert!(s.explorer_batch_chars >= s.explorer_chars_per_file);
    }

    #[test]
    fn batch_floor_respects_per_file_cap() {
        let s = ExplorerSettings {
            explorer_rounds: 6,
            explorer_files_per_round: 8,
            explorer_chars_per_file: 20_000,
            explorer_batch_chars: 4_000,
        }
        .clamped();
        assert_eq!(s.explorer_chars_per_file, 20_000);
        assert_eq!(s.explorer_batch_chars, 20_000);
    }

    #[test]
    fn round_trip_preserves_values() {
        let cfg = ExplorerSettings {
            explorer_rounds: 9,
            explorer_files_per_round: 12,
            explorer_chars_per_file: 6_000,
            explorer_batch_chars: 40_000,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: ExplorerSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.explorer_rounds, 9);
        assert_eq!(back.explorer_batch_chars, 40_000);
    }
}
