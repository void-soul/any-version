//! 音乐播放器对前端暴露的命令。

use serde::Serialize;
use tauri::State;

use super::dsp::{self, EqParams};
use super::library::{self, MusicLibrary};
use super::player::{MusicPlayerState, PlayerState};
use super::settings::{self as music_settings, MusicSettings};

/// 导入文件夹的结果（含最新曲库，前端一次调用即可刷新界面）
#[derive(Serialize)]
pub struct AddFolderResult {
    pub added: usize,
    pub library: MusicLibrary,
}

/// 手动重扫结果
#[derive(Serialize)]
pub struct RefreshResult {
    pub added: usize,
    pub removed: usize,
    pub library: MusicLibrary,
}

/// 均衡器预设（频段增益由后端提供，避免前后端两份数据不一致）
#[derive(Serialize)]
pub struct EqPresetInfo {
    pub id: String,
    pub bands: [f32; dsp::BAND_COUNT],
}

#[tauri::command]
pub fn music_get_library() -> Result<MusicLibrary, String> {
    Ok(library::load_library())
}

#[tauri::command]
pub fn music_add_folder(path: String) -> Result<AddFolderResult, String> {
    let mut library = library::load_library();
    let added = library::add_folder(&mut library, &path)?;
    library::save_library(&library)?;
    Ok(AddFolderResult { added, library })
}

#[tauri::command]
pub fn music_remove_folder(path: String) -> Result<MusicLibrary, String> {
    let mut library = library::load_library();
    library::remove_folder(&mut library, &path);
    library::save_library(&library)?;
    Ok(library)
}

/// 手动更新：重扫全部导入文件夹（**不会自动重扫**，只在用户点「重新扫描」时调用）
#[tauri::command]
pub fn music_refresh_library() -> Result<RefreshResult, String> {
    let mut library = library::load_library();
    let (added, removed) = library::refresh_library(&mut library)?;
    library::save_library(&library)?;
    Ok(RefreshResult {
        added,
        removed,
        library,
    })
}

#[tauri::command]
pub fn music_play(state: State<'_, MusicPlayerState>, path: String) -> Result<PlayerState, String> {
    state.play(&path)
}

#[tauri::command]
pub fn music_pause(state: State<'_, MusicPlayerState>) -> Result<PlayerState, String> {
    Ok(state.pause())
}

#[tauri::command]
pub fn music_resume(state: State<'_, MusicPlayerState>) -> Result<PlayerState, String> {
    Ok(state.resume())
}

#[tauri::command]
pub fn music_stop(state: State<'_, MusicPlayerState>) -> Result<PlayerState, String> {
    Ok(state.stop())
}

#[tauri::command]
pub fn music_seek(
    state: State<'_, MusicPlayerState>,
    position_ms: u64,
) -> Result<PlayerState, String> {
    state.seek(position_ms)
}

#[tauri::command]
pub fn music_set_volume(
    state: State<'_, MusicPlayerState>,
    volume: f32,
) -> Result<PlayerState, String> {
    let snapshot = state.set_volume(volume);
    // 音量变化即时落盘（失败不影响播放）
    let mut settings = music_settings::load_settings();
    settings.volume = snapshot.volume;
    let _ = music_settings::save_settings(&settings);
    Ok(snapshot)
}

#[tauri::command]
pub fn music_get_state(state: State<'_, MusicPlayerState>) -> Result<PlayerState, String> {
    Ok(state.state())
}

#[tauri::command]
pub fn music_get_settings(state: State<'_, MusicPlayerState>) -> Result<MusicSettings, String> {
    let mut settings = music_settings::load_settings();
    settings.volume = state.state().volume;
    Ok(settings)
}

/// 保存设置（音量 / 播放模式 / 音效），并让播放线程立即采用新的音效参数
#[tauri::command]
pub fn music_update_settings(
    state: State<'_, MusicPlayerState>,
    settings: MusicSettings,
) -> Result<MusicSettings, String> {
    super::player::persist_settings(&state, &settings)
}

#[tauri::command]
pub fn music_get_eq(state: State<'_, MusicPlayerState>) -> Result<EqParams, String> {
    Ok(state.eq_handle().lock().clone())
}

/// 立即应用均衡器（不落盘；拖动推子时的实时预览用，松手后再 update_settings 保存）
#[tauri::command]
pub fn music_preview_eq(
    state: State<'_, MusicPlayerState>,
    eq: EqParams,
) -> Result<EqParams, String> {
    state.set_eq_params(eq.clone());
    Ok(eq.sanitized())
}

#[tauri::command]
pub fn music_list_presets() -> Result<Vec<EqPresetInfo>, String> {
    Ok(dsp::PRESETS
        .iter()
        .map(|p| EqPresetInfo {
            id: p.id.to_string(),
            bands: p.bands,
        })
        .collect())
}
