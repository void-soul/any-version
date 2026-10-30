use super::*;

impl MusicPlayerState {
    pub fn eq_handle(&self) -> Arc<Mutex<EqParams>> {
        self.eq.clone()
    }

    /// 设置均衡器参数（播放线程会在下一批同步时读取）
    pub fn set_eq_params(&self, params: EqParams) {
        *self.eq.lock() = params.sanitized();
    }

    /// 懒加载音频设备与播放器
    ///
    /// 这里必须自己装 cpal 流错误回调：rodio 的默认回调只把错误打进日志，
    /// 而「设备被拔掉 / 默认输出被切换」正是靠这个信号才能知道该重建输出流。
    pub(super) fn ensure_player(
        inner: &mut Inner,
        output_lost: &Arc<AtomicBool>,
    ) -> Result<(), String> {
        if inner.device.is_none() {
            let flag = output_lost.clone();
            let device = DeviceSinkBuilder::from_default_device()
                .map_err(|e| format!("打开音频输出设备失败: {}", e))?
                .with_error_callback(move |err| {
                    crate::exit_log!("[音乐] 音频输出流错误（设备被移除或切换？）: {}", err);
                    flag.store(true, Ordering::SeqCst);
                })
                .open_stream()
                .map_err(|e| format!("打开音频输出设备失败: {}", e))?;
            inner.device = Some(device);
            // 新流已经建立：清掉失效标记
            output_lost.store(false, Ordering::SeqCst);
        }
        if inner.player.is_none() {
            let mixer = inner.device.as_ref().expect("device").mixer();
            inner.player = Some(Player::connect_new(mixer));
        }
        Ok(())
    }

    /// 丢弃当前（已失效的）输出流，让下一次 [`Self::ensure_player`] 重开默认设备。
    pub(super) fn discard_output(&self, inner: &mut Inner) {
        inner.player = None;
        inner.device = None;
        self.output_lost.store(false, Ordering::SeqCst);
    }

    /// 音频输出失效后重建：丢弃旧流 → 重开默认设备 → 从原位置续播。
    ///
    /// 为什么必须重建：`MixerDeviceSink` 绑定的是**创建那一刻**那个设备的流，
    /// 设备消失后它既不拉样本也不报错（`Player::play()` 只是翻个标志就返回，
    /// 「恢复」看起来是成功的），于是表现为「拔耳机后再也放不出声，只能退出重进」。
    pub(super) fn recover_output(&self, inner: &mut Inner) {
        // 只有「正在播 / 已暂停」才需要续播。`Ended` 曲目若在这里重播，
        // 会把已经放完的歌重新拉起来（位置在末尾，等于又从头震一下）；
        // `Idle` 同理只丢坏流。
        let resumable = should_resume_after_recover(inner.status);
        let Some(path) = inner
            .current
            .as_ref()
            .filter(|_| resumable)
            .map(|c| c.path.clone())
        else {
            // 没有当前曲目（或已经播完）：丢掉坏流即可，下次播放时自然重开
            self.discard_output(inner);
            inner.last_pos_ms = 0;
            inner.last_progress_at = Instant::now();
            return;
        };
        let resume_ms = inner
            .player
            .as_ref()
            .map(|p| p.get_pos().as_millis() as u64)
            .unwrap_or(inner.last_pos_ms);
        let was_paused = inner.status == PlayStatus::Paused;
        self.discard_output(inner);

        match Self::play_locked(inner, &self.eq, &self.output_lost, &path) {
            Ok(_) => {
                if let Some(player) = inner.player.as_ref() {
                    let _ = player.try_seek(Duration::from_millis(resume_ms));
                    if was_paused {
                        player.pause();
                    }
                }
                if was_paused {
                    inner.status = PlayStatus::Paused;
                }
                inner.last_pos_ms = resume_ms;
                inner.last_progress_at = Instant::now();
                crate::exit_log!("[音乐] 音频输出已重建，从 {} ms 续播: {}", resume_ms, path);
            }
            Err(e) => {
                inner.status = PlayStatus::Idle;
                inner.current = None;
                crate::exit_log!("[音乐] 音频输出重建失败: {}", e);
            }
        }
    }

    /// 更新停摆看门狗并返回「已停摆多久」；未在播放时返回 None（并重置计时）。
    pub(super) fn stalled_for(&self, inner: &mut Inner) -> Option<Duration> {
        let pos = inner
            .player
            .as_ref()
            .map(|p| p.get_pos().as_millis() as u64);
        if inner.status != PlayStatus::Playing {
            inner.last_pos_ms = pos.unwrap_or(0);
            inner.last_progress_at = Instant::now();
            return None;
        }
        let pos = pos?;
        if pos != inner.last_pos_ms {
            inner.last_pos_ms = pos;
            inner.last_progress_at = Instant::now();
            return None;
        }
        Some(inner.last_progress_at.elapsed())
    }
}
