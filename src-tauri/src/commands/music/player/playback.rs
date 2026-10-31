use super::*;

impl MusicPlayerState {
    /// 播放/暂停切换（播放器热键用）：正在播→暂停；暂停→继续；
    /// 空闲/已播完→接着当前曲目（没有则从队列取下一首）
    pub fn toggle(&self) -> Result<PlayerState, String> {
        let mut inner = self.inner.lock();
        // 输出已失效：先重建，否则下面的 play()/pause() 只是空操作（用户看到的「点了没反应」）
        if self.output_lost.load(Ordering::SeqCst) {
            self.recover_output(&mut inner);
        }
        match inner.status {
            PlayStatus::Playing => {
                if let Some(player) = inner.player.as_ref() {
                    player.pause();
                }
                inner.status = PlayStatus::Paused;
                Ok(Self::snapshot(&mut inner))
            }
            PlayStatus::Paused => {
                if let Some(player) = inner.player.as_ref() {
                    player.play();
                }
                inner.status = PlayStatus::Playing;
                Ok(Self::snapshot(&mut inner))
            }
            _ => {
                // 队列里可能是还没落盘的在线曲目：那种情况这里起不来（没有本地文件），
                // 交给巡查线程或用户的下一次操作去取流
                let path = inner
                    .current
                    .as_ref()
                    .map(|c| c.path.clone())
                    .or_else(|| {
                        inner
                            .queue
                            .current()
                            .and_then(QueueItem::path)
                            .map(str::to_string)
                    })
                    .or_else(|| {
                        inner
                            .queue
                            .first()
                            .and_then(|item| item.path().map(str::to_string))
                    });
                match path {
                    Some(path) => Self::play_locked(&mut inner, &self.eq, &self.output_lost, &path),
                    None => Ok(Self::snapshot(&mut inner)),
                }
            }
        }
    }

    /// 后台巡查（由 [`start_queue_watcher`] 每 250ms 调用）：
    /// 当前曲目已放完（输出队列为空）→ 自动切下一首。
    ///
    /// 判定**不限于 `Playing`**：`Ended` 同样继续推进（自愈）—— 无论谁读过状态、
    /// 或历史状态被置为 Ended，续播都不会被卡死。
    /// 失败（如文件已被删除）时继续尝试后续曲目，最多绕队列一圈。
    /// `app` 为 None 时（单测）在线曲目直接放弃推进 —— 没有句柄就无法取流，
    /// 卡在「等待取流」上更糟。
    pub fn tick(&self, app: Option<&tauri::AppHandle>) {
        let mut inner = self.inner.lock();

        // ⓪ 有在线曲目正在取流：**不推进**。等它落盘后由 `play_online_resolved` 接上；
        //    否则巡查每 250ms 推进一次，一首没下载完就跳过了好几首。
        if inner.pending.is_some() {
            return;
        }

        // ① 输出自愈：设备报错（拔耳机/换默认设备）或位置停摆 → 重建并从原位置续播。
        //    这一段必须在「播完判定」之前：输出死了以后位置不会再前进，
        //    不先自愈的话队列永远等不到 empty()，表现为「彻底卡住」。
        let stalled = self.stalled_for(&mut inner);
        if should_recover(
            self.output_lost.load(Ordering::SeqCst),
            inner.status,
            stalled,
        ) {
            self.recover_output(&mut inner);
        }

        // ② 播完自动切下一首
        let exhausted = inner.current.is_some()
            && matches!(inner.status, PlayStatus::Playing | PlayStatus::Ended)
            && matches!(inner.player.as_ref(), Some(player) if player.empty());
        if !exhausted {
            // 还在播（或暂停中）：趁现在把下一首先准备好
            self.prefetch_next(&mut inner);
            return;
        }

        let attempts = inner.queue.len().max(1);
        for _ in 0..attempts {
            let item = inner.queue.advance();
            match self.step(&mut inner, item) {
                Ok(AdvanceOutcome::Playing(_)) => {
                    if let Some(current) = inner.current.as_ref() {
                        crate::exit_log!("[音乐] 播完自动切下一首: {}", current.path);
                    }
                    return;
                }
                // 在线曲目：交给异步任务取流落盘（巡查线程是普通线程，不能 await）
                Ok(AdvanceOutcome::Online(pending)) => {
                    match app {
                        Some(app) => {
                            crate::exit_log!("[音乐] 播完自动切下一首（在线曲目，取流中）");
                            spawn_online_resolve(app.clone(), pending);
                        }
                        None => inner.pending = None,
                    }
                    return;
                }
                // 解码失败（文件损坏/被删除）：跳过，继续下一首
                Err(_) => {}
            }
        }
        inner.status = PlayStatus::Ended;
    }

    /// 后台预转码下一首。
    ///
    /// **为什么需要它**：转码要跑 ffmpeg（几秒）。只在「用户点播放」时才转的话，
    /// 自动切歌和「下一首」走到解不开的文件上会直接失败 —— 那首根本没转过。
    /// 趁当前这首还在播（通常还有好几分钟）提前转好，轮到它时直接命中缓存。
    ///
    /// 只在**真的解不开**时才跑 ffmpeg：先探一下头部（[`transcode::needs_transcode`]），
    /// 普通 mp3 不值得起一个外部进程。
    pub(super) fn prefetch_next(&self, inner: &mut Inner) {
        let Some(next) = inner.queue.peek_next() else {
            return;
        };
        // 在线曲目轮到时才取流，这里没有文件可转
        let Some(path) = next.path().map(str::to_string) else {
            return;
        };
        if inner.prefetched.as_deref() == Some(path.as_str()) {
            return;
        }
        inner.prefetched = Some(path.clone());
        // 另起线程：ffmpeg 是秒级操作，不能堵住 250ms 的巡查循环
        std::thread::spawn(move || {
            if crate::commands::music::transcode::cached(&path).is_some() {
                return;
            }
            if !crate::commands::music::transcode::needs_transcode(&path) {
                return;
            }
            match crate::commands::music::transcode::ensure(&path) {
                Ok(_) => crate::exit_log!("[音乐] 后台预转码完成: {path}"),
                Err(err) => crate::exit_log!("[音乐] 后台预转码失败（播放时会再试一次）: {err}"),
            }
        });
    }

    /// 真正播放：调用方必须已持有 `inner` 的锁
    pub(super) fn play_locked(
        inner: &mut Inner,
        eq: &Arc<Mutex<EqParams>>,
        output_lost: &Arc<AtomicBool>,
        path: &str,
    ) -> Result<PlayerState, String> {
        // 之前转过码的文件（symphonia 解不了的编码）：播转码后的副本。
        // 报错信息仍指向**原文件** —— 容器、扩展名那些线索都在原文件上。
        let transcoded = crate::commands::music::transcode::cached(path);
        let target = transcoded.as_deref().unwrap_or(path);
        let file = File::open(target).map_err(|e| format!("打开文件失败: {}", e))?;
        // 必须显式告知字节长度：symphonia 只有在已知流长度时才允许随机访问，
        // 否则 try_seek 会返回 RandomAccessNotSupported（进度条拖动失效）。
        let byte_len = file.metadata().map(|m| m.len()).unwrap_or(0);
        let mut builder = Decoder::builder().with_data(BufReader::new(file));
        if byte_len > 0 {
            builder = builder.with_byte_len(byte_len);
        }
        let decoder = builder
            .build()
            .map_err(|e| decode_failure_message(path, &e))?;
        let decoder_duration_ms = decoder
            .total_duration()
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        // 显示名：登记名（在线缓存）→ 曲库标签 → 文件名
        let named = inner.names.get(path).cloned();
        // 已有登记名时不必再读曲库：缓存文件本来就不在曲库里
        let library_track = if named.is_some() {
            None
        } else {
            library::load_library()
                .tracks
                .into_iter()
                .find(|t| library::same_path(&t.path, path))
        };
        let (title, artist) = track_labels(path, named.as_ref(), library_track.as_ref());
        let duration_ms = library_track
            .as_ref()
            .map(|t| t.duration_ms)
            .filter(|d| *d > 0)
            .unwrap_or(decoder_duration_ms);

        let source = EqSource::new(decoder, eq.clone());

        Self::ensure_player(inner, output_lost)?;
        // 每首重建 Player：位置计数归零，避免复用时的残留队列
        let mixer = inner.device.as_ref().expect("device").mixer();
        let player = Player::connect_new(mixer);
        player.set_volume(inner.volume);
        player.append(source);
        inner.player = Some(player);
        inner.status = PlayStatus::Playing;
        // 新曲目从 0 开始：重置看门狗，给输出流留出启动窗口
        inner.last_pos_ms = 0;
        inner.last_progress_at = Instant::now();
        inner.current = Some(CurrentTrack {
            path: path.to_string(),
            title,
            artist,
            duration_ms,
            // 来源由调用方（play_named / play_online_resolved）在播放后补上
            online: None,
        });
        // 让队列下标对齐到本曲，之后的「下一首 / 上一首」都从这里继续
        inner.queue.focus(path);
        Ok(Self::snapshot(inner))
    }

    pub fn pause(&self) -> PlayerState {
        let mut inner = self.inner.lock();
        if let Some(player) = inner.player.as_ref() {
            player.pause();
            if inner.status == PlayStatus::Playing {
                inner.status = PlayStatus::Paused;
            }
        }
        Self::snapshot(&mut inner)
    }

    pub fn resume(&self) -> PlayerState {
        let mut inner = self.inner.lock();
        // 输出已失效：先重建（会从原位置续播）；否则这里的 play() 点下去不会有声
        if self.output_lost.load(Ordering::SeqCst) {
            self.recover_output(&mut inner);
        }
        if let Some(player) = inner.player.as_ref() {
            // 已播完的曲目不允许“继续”
            if inner.status == PlayStatus::Paused {
                player.play();
                inner.status = PlayStatus::Playing;
            }
        }
        Self::snapshot(&mut inner)
    }

    pub fn stop(&self) -> PlayerState {
        let mut inner = self.inner.lock();
        if let Some(player) = inner.player.as_ref() {
            player.clear();
            player.stop();
        }
        inner.status = PlayStatus::Idle;
        inner.current = None;
        Self::snapshot(&mut inner)
    }

    /// Module shutdown releases the native output and decoded buffers as well as stopping playback.
    pub fn release_resources(&self) {
        let mut inner = self.inner.lock();
        inner.seq = inner.seq.wrapping_add(1);
        if let Some(player) = inner.player.take() {
            player.stop();
        }
        inner.device = None;
        inner.current = None;
        inner.pending = None;
        inner.prefetched = None;
        inner.names = HashMap::new();
        inner.queue = PlayQueue::default();
        inner.status = PlayStatus::Idle;
        self.output_lost.store(false, Ordering::SeqCst);
    }

    pub fn seek(&self, position_ms: u64) -> Result<PlayerState, String> {
        let mut inner = self.inner.lock();
        // 输出已失效：先重建再跳转，否则 try_seek 会作用在一个已经死掉的流上
        if self.output_lost.load(Ordering::SeqCst) {
            self.recover_output(&mut inner);
        }
        let was_ended = inner.status == PlayStatus::Ended;
        {
            let player = inner
                .player
                .as_ref()
                .ok_or_else(|| "当前没有正在播放的曲目".to_string())?;
            player
                .try_seek(Duration::from_millis(position_ms))
                .map_err(|e| format!("跳转失败: {}", e))?;
            // 播完后再拖动进度条 → 视为继续播放
            if was_ended {
                player.play();
            }
        }
        if was_ended {
            inner.status = PlayStatus::Playing;
        }
        Ok(Self::snapshot(&mut inner))
    }

    pub fn set_volume(&self, volume: f32) -> PlayerState {
        let mut inner = self.inner.lock();
        let volume = if volume.is_finite() {
            volume.clamp(0.0, 1.0)
        } else {
            inner.volume
        };
        inner.volume = volume;
        if let Some(player) = inner.player.as_ref() {
            player.set_volume(volume);
        }
        Self::snapshot(&mut inner)
    }

    pub fn state(&self) -> PlayerState {
        let mut inner = self.inner.lock();
        Self::snapshot(&mut inner)
    }

    /// 当前曲目的本地路径（在线曲目落盘后才有）
    pub fn current_path(&self) -> Option<String> {
        let inner = self.inner.lock();
        inner.current.as_ref().map(|c| c.path.clone())
    }
}
