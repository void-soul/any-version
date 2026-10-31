use std::time::Instant;

use super::*;

#[test]
fn disabling_releases_queue_names_and_pending_online_generation() {
    let state = MusicPlayerState::default();
    {
        let mut inner = state.inner.lock();
        inner.seq = 10;
        inner.names.insert(
            "fake".into(),
            TrackName {
                title: "title".into(),
                artist: "artist".into(),
            },
        );
        inner.queue.set(
            vec![QueueItem::Path("fake".into())],
            PlayMode::Sequence,
            None,
        );
        inner.pending = Some(PendingOnline {
            file: "fake".into(),
            item: serde_json::json!({}),
            quality: "fake".into(),
            seq: 10,
        });
        inner.prefetched = Some("fake".into());
        inner.status = PlayStatus::Paused;
    }
    state.release_resources();
    let inner = state.inner.lock();
    assert_eq!(inner.status, PlayStatus::Idle);
    assert_eq!(inner.queue.len(), 0);
    assert_eq!(inner.names.capacity(), 0);
    assert!(inner.pending.is_none() && inner.prefetched.is_none());
    assert!(inner.player.is_none() && inner.device.is_none());
    assert_ne!(inner.seq, 10);
}

/// 设备报错一定触发重建（不管播放状态）。
#[test]
fn test_should_recover_on_output_lost() {
    assert!(should_recover(true, PlayStatus::Playing, None));
    assert!(should_recover(true, PlayStatus::Paused, None));
    assert!(should_recover(true, PlayStatus::Idle, None));
}

/// 停摆只有 Playing 才算：暂停本来就不前进，播完交给推进逻辑。
#[test]
fn test_should_recover_on_stall_only_while_playing() {
    let stalled = Some(Duration::from_millis(STALL_RECOVER_MS));
    assert!(should_recover(false, PlayStatus::Playing, stalled));
    assert!(!should_recover(false, PlayStatus::Paused, stalled));
    assert!(!should_recover(false, PlayStatus::Ended, stalled));
    assert!(!should_recover(false, PlayStatus::Idle, stalled));
}

/// 只有正在播/已暂停才在重建后续播：已播完的曲目不能被「拉起来」。
#[test]
fn test_recover_resumes_only_for_active_playback() {
    assert!(should_resume_after_recover(PlayStatus::Playing));
    assert!(should_resume_after_recover(PlayStatus::Paused));
    assert!(!should_resume_after_recover(PlayStatus::Ended));
    assert!(!should_resume_after_recover(PlayStatus::Idle));
}

/// 未达到阈值不重建（正常播放中位置每 250ms 都会前进，这里模拟「刚查过一次」）。
#[test]
fn test_should_not_recover_before_stall_threshold() {
    assert!(!should_recover(
        false,
        PlayStatus::Playing,
        Some(Duration::from_millis(STALL_RECOVER_MS - 1))
    ));
    // 位置在前进（stalled_for 返回 None）→ 不重建
    assert!(!should_recover(false, PlayStatus::Playing, None));
}

#[test]
fn test_initial_state_is_idle() {
    let state = MusicPlayerState::default();
    let snapshot = state.state();
    assert_eq!(snapshot.status, PlayStatus::Idle);
    assert!(snapshot.path.is_none());
    assert_eq!(snapshot.position_ms, 0);
    assert!(snapshot.volume >= 0.0 && snapshot.volume <= 1.0);
}

/// 在线缓存文件的名字是哈希，必须显示登记的歌名；没有登记时才回退文件名。
#[test]
fn registered_name_beats_the_cache_file_name() {
    let named = TrackName {
        title: "Abyss".to_string(),
        artist: "Yungblud".to_string(),
    };
    let (title, artist) = track_labels("D:/cache/a3f9e1c0.m4a", Some(&named), None);
    assert_eq!(title, "Abyss");
    assert_eq!(artist, "Yungblud");

    // 没登记 → 回退文件名（曲库外的本地文件）
    let (title, _) = track_labels("D:/music/周杰伦 - 稻香.mp3", None, None);
    assert_eq!(title, "周杰伦 - 稻香");

    // 插件给了空标题 → 视同没登记，继续回退
    let (title, _) = track_labels(
        "D:/cache/x.m4a",
        Some(&TrackName {
            title: "  ".to_string(),
            artist: String::new(),
        }),
        None,
    );
    assert_eq!(title, "x");
}

/// 本地曲目没有在线来源 → 界面据此隐藏「下载」；
/// 也只有在线曲目才拿得到重新取流需要的插件与曲目对象。
#[test]
fn local_track_has_no_online_origin() {
    let state = MusicPlayerState::default();
    assert!(state.current_online_origin().is_none());
    // 快照里的在线标记同样为空（界面用它决定按钮是否出现）
    assert!(state.state().online.is_none());
}

#[test]
fn test_play_missing_file_reports_error() {
    let state = MusicPlayerState::default();
    let err = state.play("D:/definitely/missing/song.mp3").unwrap_err();
    assert!(err.contains("打开文件失败"), "实际错误: {err}");
}

#[test]
fn test_set_volume_clamps() {
    let state = MusicPlayerState::default();
    assert_eq!(state.set_volume(5.0).volume, 1.0);
    assert_eq!(state.set_volume(-1.0).volume, 0.0);
    assert_eq!(state.set_volume(f32::NAN).volume, 0.0);
}

#[test]
fn test_stop_clears_current_track() {
    let state = MusicPlayerState::default();
    let snapshot = state.stop();
    assert_eq!(snapshot.status, PlayStatus::Idle);
    assert!(snapshot.title.is_none());
}

#[test]
fn test_resume_without_pause_does_nothing() {
    let state = MusicPlayerState::default();
    assert_eq!(state.resume().status, PlayStatus::Idle);
    assert_eq!(state.pause().status, PlayStatus::Idle);
}

#[test]
fn test_toggle_with_empty_queue_is_noop() {
    // 空队列下按播放键不应崩溃，也不应尝试打开音频设备
    let state = MusicPlayerState::default();
    assert_eq!(state.toggle().unwrap().status, PlayStatus::Idle);
}

#[test]
fn test_set_queue_then_next_reports_missing_file() {
    let state = MusicPlayerState::default();
    state
        .set_queue(
            vec!["D:/definitely/missing/song.mp3".to_string()],
            "sequence",
        )
        .unwrap();
    let err = state.next().unwrap_err();
    assert!(err.contains("打开文件失败"), "实际错误: {err}");
}

#[test]
fn test_tick_without_playing_does_nothing() {
    // 未在播放时巡查不应改变状态（也不会误触发切歌）
    let state = MusicPlayerState::default();
    state
        .set_queue(vec!["D:/missing/a.mp3".to_string()], "sequence")
        .unwrap();
    state.tick(None);
    assert_eq!(state.state().status, PlayStatus::Idle);
}

#[test]
fn test_snapshot_is_read_only() {
    // 回归（Q-0078 第二轮）：快照/读状态曾经会把 Playing 判成 Ended，
    // 而 tick() 只在 Playing 时推进 —— 一次轮询就能让自动切歌永久失效。
    // 这里退化到无音频设备也能断言的部分：未播放时读状态不改任何状态。
    let state = MusicPlayerState::default();
    let before = state.state();
    let after = state.state();
    assert_eq!(before.status, after.status);
    assert_eq!(after.status, PlayStatus::Idle);
}

/// 写一个最小 16-bit 单声道 PCM WAV（测试用，避免依赖 ffmpeg）
#[cfg(test)]
fn write_sine_wav(path: &std::path::Path, seconds: f32) {
    use std::io::Write;
    const SAMPLE_RATE: u32 = 44_100;
    let total = (SAMPLE_RATE as f32 * seconds) as u32;
    let data_len = total * 2;
    let mut buf = Vec::with_capacity(44 + data_len as usize);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&(36 + data_len).to_le_bytes());
    buf.extend_from_slice(b"WAVEfmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
    buf.extend_from_slice(&1u16.to_le_bytes()); // 单声道
    buf.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    buf.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    buf.extend_from_slice(&2u16.to_le_bytes());
    buf.extend_from_slice(&16u16.to_le_bytes());
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..total {
        let t = i as f32 / SAMPLE_RATE as f32;
        let s = (t * 440.0 * std::f32::consts::TAU).sin() * 0.2;
        buf.extend_from_slice(&((s * i16::MAX as f32) as i16).to_le_bytes());
    }
    std::fs::File::create(path)
        .and_then(|mut f| f.write_all(&buf))
        .expect("写测试音频");
}

/// 端到端回归：**同时模拟前端轮询与后台巡查**，播完必须自动切下一首。
///
/// 需要音频输出设备，因此默认忽略；在本机手动验证：
/// `cargo test --lib -- --ignored test_auto_advance_survives_state_polling`
#[test]
#[ignore = "需要音频输出设备（真实播放），用 `cargo test --lib -- --ignored` 手动跑"]
fn test_auto_advance_survives_state_polling() {
    let dir = std::env::temp_dir().join(format!("kira-music-race-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建临时目录");
    let a_path = dir.join("a.wav");
    let b_path = dir.join("b.wav");
    write_sine_wav(&a_path, 1.0);
    write_sine_wav(&b_path, 1.0);
    let a = a_path.to_string_lossy().to_string();
    let b = b_path.to_string_lossy().to_string();

    let state = MusicPlayerState::default();
    state
        .set_queue(vec![a.clone(), b.clone()], "sequence")
        .unwrap();
    state.play(&a).expect("应能开始播放（需要音频输出设备）");

    let start = Instant::now();
    let mut last_tick = Instant::now();
    let mut last_poll = Instant::now();
    let mut advanced = false;
    while start.elapsed() < Duration::from_secs(15) {
        std::thread::sleep(Duration::from_millis(25));
        // 后台巡查线程（真实实现 250ms 一次）
        if last_tick.elapsed() >= Duration::from_millis(250) {
            last_tick = Instant::now();
            state.tick(None);
        }
        // 前端轮询 music_get_state（隐藏窗口下会被节流，可见时 500ms 一次）
        if last_poll.elapsed() >= Duration::from_millis(25) {
            last_poll = Instant::now();
            assert_eq!(
                state.state().status,
                PlayStatus::Playing,
                "读状态不得改变播放状态（旧 bug：轮询把它置成 Ended 后永久不切歌）"
            );
        }
        if state.state().path.as_deref() == Some(b.as_str()) {
            advanced = true;
            break;
        }
    }
    state.stop();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(advanced, "有前端轮询时也必须能自动切歌");
}

#[test]
fn test_tick_skips_broken_tracks_and_ends_on_empty() {
    // 队列里全是坏文件：巡查切换时应逐个跳过，最终置为 Ended 而不是卡在 Playing
    let state = MusicPlayerState::default();
    state
        .set_queue(
            vec![
                "D:/missing/a.mp3".to_string(),
                "D:/missing/b.mp3".to_string(),
            ],
            "sequence",
        )
        .unwrap();
    state.tick(None);
    assert_eq!(
        state.state().status,
        PlayStatus::Idle,
        "未在播放时不应被巡查改变"
    );
}
