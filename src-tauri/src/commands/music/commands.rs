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

/// ⚠️ 本模块所有命令必须是 `async fn`：同步命令在 Tauri v2 中于**主线程**执行，
/// 而它们共享 `MusicPlayerState::inner` 这把锁，且持锁期间有磁盘 IO / 解码器初始化
/// / rodio 阻塞等待（见 player.rs），任何一个卡住都会冻结整个窗口（音乐面板
/// 每 500ms 还在轮询 music_get_state，卡住即表现为「程序卡死」）。
/// async 命令由 `async_runtime::spawn` 在独立任务执行，主线程不再被阻塞。
/// （Q-0098：暂停后恢复播放间歇性卡死）

#[tauri::command]
pub async fn music_get_library() -> Result<MusicLibrary, String> {
    Ok(library::load_library())
}

#[tauri::command]
pub async fn music_add_folder(path: String) -> Result<AddFolderResult, String> {
    let mut library = library::load_library();
    let added = library::add_folder(&mut library, &path)?;
    library::save_library(&library)?;
    Ok(AddFolderResult { added, library })
}

#[tauri::command]
pub async fn music_remove_folder(path: String) -> Result<MusicLibrary, String> {
    let mut library = library::load_library();
    library::remove_folder(&mut library, &path);
    library::save_library(&library)?;
    Ok(library)
}

/// 手动更新：重扫全部导入文件夹（**不会自动重扫**，只在用户点「重新扫描」时调用）
#[tauri::command]
pub async fn music_refresh_library() -> Result<RefreshResult, String> {
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
pub async fn music_play(
    state: State<'_, MusicPlayerState>,
    path: String,
) -> Result<PlayerState, String> {
    state.play(&path)
}

// ─── 单曲文件操作：重命名 / 删除 ───

/// 重命名弹窗的预填建议
#[derive(Serialize)]
pub struct TrackNameSuggestion {
    /// 磁盘上的现名（含扩展名）
    pub current_name: String,
    /// 推荐新名（含扩展名）；无可解析标签时回退为现名
    pub suggested_name: String,
    /// 建议是否来自音频标签（false = 只能沿用现名）
    pub from_tags: bool,
}

/// 推荐文件名：用音频标签生成 `歌手 - 标题.ext`（前端弹窗预填用）。
#[tauri::command]
pub async fn music_track_name_suggestion(path: String) -> Result<TrackNameSuggestion, String> {
    let file = std::path::Path::new(&path);
    let current_name = file
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .ok_or_else(|| "路径无效".to_string())?;
    if !file.is_file() {
        return Err("文件不存在（可能已被移动或删除）".to_string());
    }
    let suggested = library::suggest_file_name(file);
    Ok(TrackNameSuggestion {
        from_tags: suggested.is_some(),
        suggested_name: suggested.unwrap_or_else(|| current_name.clone()),
        current_name,
    })
}

/// 重命名结果（含最新曲库，前端一次调用即可刷新）
#[derive(Serialize)]
pub struct RenameTrackResult {
    pub old_path: String,
    pub new_path: String,
    /// 文件名是否真的变了（同名提交时为 false）
    pub renamed: bool,
    pub library: MusicLibrary,
}

/// 重命名单曲：改磁盘文件名 + 同步曲库与播放队列（正在播放不中断）。
#[tauri::command]
pub async fn music_rename_track(
    state: State<'_, MusicPlayerState>,
    path: String,
    new_name: String,
) -> Result<RenameTrackResult, String> {
    let new_path = library::rename_track_file(&path, &new_name)?;
    let renamed = !library::same_path(&path, &new_path);

    let mut library = library::load_library();
    if renamed {
        library::replace_track_path(&mut library, &path, &new_path);
        library::save_library(&library)?;
        // 播放器侧同步路径：正在播放时只改记录，不打断输出
        state.rename_path(&path, &new_path);
    }
    Ok(RenameTrackResult {
        old_path: path,
        new_path,
        renamed,
        library,
    })
}

/// 删除结果（含最新曲库与播放器状态）
#[derive(Serialize)]
pub struct DeleteTracksResult {
    pub deleted: usize,
    /// 失败项（`路径（原因）`）：例如文件被占用、回收站不可用
    pub failed: Vec<String>,
    pub library: MusicLibrary,
    /// 删掉正在播放那首时会自动切下一首，这里回传切换后的播放状态
    pub player: PlayerState,
}

/// 删除单曲（支持批量）：移入系统回收站 + 同步曲库；删到正在播放的那首时自动切下一首。
#[tauri::command]
pub async fn music_delete_tracks(
    state: State<'_, MusicPlayerState>,
    paths: Vec<String>,
) -> Result<DeleteTracksResult, String> {
    let mut deleted = 0usize;
    let mut failed = Vec::new();
    for path in &paths {
        match library::delete_track_file(path) {
            Ok(()) => deleted += 1,
            Err(err) => failed.push(format!("{}（{}）", path, err)),
        }
    }

    let mut library = library::load_library();
    library::remove_tracks(&mut library, &paths);
    library::save_library(&library)?;

    // 从队列摘掉；当前曲目被删时切下一首。
    // 注意 `forget_paths` 不能自己调 `next()`：同一把锁不可重入（会死锁）。
    let current_removed = state.forget_paths(&paths);
    let player = if current_removed {
        state.next().unwrap_or_else(|_| state.state())
    } else {
        state.state()
    };

    Ok(DeleteTracksResult {
        deleted,
        failed,
        library,
        player,
    })
}

/// 重置播放队列（曲库或播放模式变化时调用）。
/// 队列常驻后端，托盘/隐藏窗口时也能自动续播下一首。
#[tauri::command]
pub async fn music_set_queue(
    state: State<'_, MusicPlayerState>,
    paths: Vec<String>,
    mode: String,
) -> Result<(), String> {
    state.set_queue(paths, &mode)
}

/// 下一首（用户操作；播完自动切歌由后端巡查线程负责）
#[tauri::command]
pub async fn music_next(state: State<'_, MusicPlayerState>) -> Result<PlayerState, String> {
    state.next()
}

/// 上一首
#[tauri::command]
pub async fn music_prev(state: State<'_, MusicPlayerState>) -> Result<PlayerState, String> {
    state.prev()
}

/// 播放/暂停切换（播放器热键用；空闲时从队列起播）
#[tauri::command]
pub async fn music_toggle(state: State<'_, MusicPlayerState>) -> Result<PlayerState, String> {
    state.toggle()
}

#[tauri::command]
pub async fn music_pause(state: State<'_, MusicPlayerState>) -> Result<PlayerState, String> {
    Ok(state.pause())
}

#[tauri::command]
pub async fn music_resume(state: State<'_, MusicPlayerState>) -> Result<PlayerState, String> {
    Ok(state.resume())
}

#[tauri::command]
pub async fn music_stop(state: State<'_, MusicPlayerState>) -> Result<PlayerState, String> {
    Ok(state.stop())
}

#[tauri::command]
pub async fn music_seek(
    state: State<'_, MusicPlayerState>,
    position_ms: u64,
) -> Result<PlayerState, String> {
    state.seek(position_ms)
}

#[tauri::command]
pub async fn music_set_volume(
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
pub async fn music_get_state(state: State<'_, MusicPlayerState>) -> Result<PlayerState, String> {
    Ok(state.state())
}

#[tauri::command]
pub async fn music_get_settings(state: State<'_, MusicPlayerState>) -> Result<MusicSettings, String> {
    let mut settings = music_settings::load_settings();
    settings.volume = state.state().volume;
    Ok(settings)
}

/// 保存设置（音量 / 播放模式 / 音效），并让播放线程立即采用新的音效参数
#[tauri::command]
pub async fn music_update_settings(
    state: State<'_, MusicPlayerState>,
    settings: MusicSettings,
) -> Result<MusicSettings, String> {
    super::player::persist_settings(&state, &settings)
}

#[tauri::command]
pub async fn music_get_eq(state: State<'_, MusicPlayerState>) -> Result<EqParams, String> {
    Ok(state.eq_handle().lock().clone())
}

/// 立即应用均衡器（不落盘；拖动推子时的实时预览用，松手后再 update_settings 保存）
#[tauri::command]
pub async fn music_preview_eq(
    state: State<'_, MusicPlayerState>,
    eq: EqParams,
) -> Result<EqParams, String> {
    state.set_eq_params(eq.clone());
    Ok(eq.sanitized())
}

#[tauri::command]
pub async fn music_list_presets() -> Result<Vec<EqPresetInfo>, String> {
    Ok(dsp::PRESETS
        .iter()
        .map(|p| EqPresetInfo {
            id: p.id.to_string(),
            bands: p.bands,
        })
        .collect())
}

/// 曲线导入结果
#[derive(Serialize)]
pub struct EqCurveImport {
    /// 折叠到 10 段后的增益（dB）
    pub bands: [f32; dsp::BAND_COUNT],
    /// 解析出的原始频点数量
    pub points: usize,
    /// 原始曲线覆盖的最低 / 最高频率（Hz），供前端提示覆盖范围
    pub min_freq: f32,
    pub max_freq: f32,
}

/// 导入均衡器曲线（GraphicEQ / AutoEq / Equalizer APO 文本）并折叠到 10 段。
///
/// **只解析并回传**，是否套用由前端决定 —— 用户可能只想看一眼，或先改几个推子再保存。
#[tauri::command]
pub async fn music_parse_eq_curve(text: String) -> Result<EqCurveImport, String> {
    let points = dsp::parse_graphic_eq(&text)
        .ok_or_else(|| "没有解析到任何「频率 增益」数据（可尝试 AutoEq 的 GraphicEQ 或 10 段输出）".to_string())?;
    let bands = dsp::fold_curve_to_bands(&points)
        .ok_or_else(|| "曲线数据不足，无法折叠到 10 段".to_string())?;
    let min_freq = points.iter().map(|(f, _)| *f).fold(f32::INFINITY, f32::min);
    let max_freq = points
        .iter()
        .map(|(f, _)| *f)
        .fold(f32::NEG_INFINITY, f32::max);
    Ok(EqCurveImport {
        bands,
        points: points.len(),
        min_freq,
        max_freq,
    })
}

/// 一条内置曲线（原文给前端，套用走 [`music_parse_eq_curve`]，两条路径口径一致）
#[derive(Serialize)]
pub struct BuiltinCurveInfo {
    pub id: String,
    pub text: String,
}

/// 列出内置曲线（GraphicEQ 文本），让不想自己找曲线的用户点一下就能用。
#[tauri::command]
pub async fn music_list_builtin_curves() -> Result<Vec<BuiltinCurveInfo>, String> {
    Ok(dsp::BUILTIN_CURVES
        .iter()
        .map(|c| BuiltinCurveInfo {
            id: c.id.to_string(),
            text: c.text.to_string(),
        })
        .collect())
}
