use super::*;

impl MusicPlayerState {
    /// 组装状态快照。
    ///
    /// **这是只读路径**（前端每 500ms 轮询 `music_get_state` 都会走到这里），
    /// 绝不能修改播放状态：旧实现在这里把「输出已空」判为 `Ended`，而 `tick()` 只
    /// 在 `Playing` 时推进 —— 于是任何一次轮询抢先置为 `Ended` 都会让自动切歌
    /// **永久失效**（表现就是托盘模式下「播完不切下一首」）。
    /// 播完的判定与推进统一由 [`MusicPlayerState::tick`] 负责。
    pub(super) fn snapshot(inner: &mut Inner) -> PlayerState {
        let duration_ms = inner.current.as_ref().map(|c| c.duration_ms).unwrap_or(0);
        let mut position_ms = 0u64;
        if inner.current.is_some() {
            if let Some(player) = inner.player.as_ref() {
                position_ms = player.get_pos().as_millis() as u64;
            }
        }
        if duration_ms > 0 {
            position_ms = position_ms.min(duration_ms);
        }

        PlayerState {
            status: inner.status,
            path: inner.current.as_ref().map(|c| c.path.clone()),
            title: inner.current.as_ref().map(|c| c.title.clone()),
            artist: inner.current.as_ref().map(|c| c.artist.clone()),
            online: inner
                .current
                .as_ref()
                .and_then(|c| c.online.as_ref())
                .map(|origin| OnlineMeta {
                    platform: origin.platform.clone(),
                    quality: origin.quality.clone(),
                }),
            position_ms,
            duration_ms,
            volume: inner.volume,
        }
    }
}
