use super::*;

/// 保存设置（音量 + 模式 + 音效）并从播放器侧收下音效参数
pub fn persist_settings(
    state: &MusicPlayerState,
    settings: &MusicSettings,
) -> Result<MusicSettings, String> {
    let settings = settings.clone().sanitized();
    state.set_volume(settings.volume);
    state.set_eq_params(settings.eq.clone());
    settings::save_settings(&settings)?;
    Ok(settings)
}

/// 插件脚本文件名 → 展示名（查不到就用文件名）。
///
/// 只在**开始播放时**查一次：`PlayerState` 每 500ms 被轮询一次，
/// 把注册表读盘放进快照会让每次轮询都碰一次磁盘。
pub(in crate::commands::music) fn plugin_display_name(file: &str) -> String {
    let entry = crate::commands::music::plugin_registry::load()
        .plugins
        .into_iter()
        .find(|entry| entry.file == file);
    match entry {
        Some(entry) => entry.display_name(),
        None => file.to_string(),
    }
}

/// 把在线曲目交给异步运行时：取流 → 落缓存 → 接上播放。
///
/// 为什么不在巡查线程里等：巡查线程是**普通线程**（不能 await），而取流是秒级操作；
/// 真要等就得阻塞 250ms 的巡检循环。等待期间由 `Inner.pending` 挡住推进，
/// 任务完成后自己调 `play_online_resolved` 接上 —— 巡查线程不需要知道结果。
pub fn spawn_online_resolve(app: tauri::AppHandle, pending: PendingOnline) {
    crate::runtime_tasks::spawn_task("music", &format!("online-{}", pending.seq), async move {
        let seq = pending.seq;
        let outcome = crate::commands::music::plugin_playback::materialize(
            &app,
            &pending.file,
            &pending.item,
            &pending.quality,
        )
        .await;
        let Some(state) = app.try_state::<MusicPlayerState>() else {
            return;
        };
        match outcome {
            Ok(materialized) => {
                let path = materialized.path.to_string_lossy().to_string();
                if let Err(err) = state.play_online_resolved(
                    &path,
                    &materialized.title,
                    &materialized.artist,
                    seq,
                ) {
                    crate::exit_log!("[音乐] 在线曲目落盘后播放失败: {err}");
                    state.abandon_online(seq);
                }
            }
            Err(err) => {
                crate::exit_log!("[音乐] 在线曲目取流失败: {err}");
                state.abandon_online(seq);
            }
        }
    });
}

/// 启动后台巡查线程：每 250ms 检查一次「本曲是否播完」，播完则由后端自动切下一首。
///
/// 必须由后端驱动（而不是前端轮询后切换）：窗口隐藏 / 最小化到托盘后，
/// WebView2 会节流甚至暂停前端定时器，只有 Rust 侧才能可靠地续播。
pub fn start_queue_watcher(app: tauri::AppHandle) {
    crate::runtime_tasks::spawn_task("music", "queue-watcher", async move {
        crate::exit_log!("[音乐] 后台巡查线程已启动（播完自动切下一首）");
        loop {
            tokio::time::sleep(Duration::from_millis(250)).await;
            match app.try_state::<MusicPlayerState>() {
                Some(state) => state.tick(Some(&app)),
                // 应用已退出（State 已回收）：结束线程
                None => break,
            }
        }
    });
}
