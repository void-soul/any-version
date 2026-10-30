use super::*;

impl MusicPlayerState {
    /// 播放指定文件（替换当前曲目）。
    ///
    /// symphonia 解不了的编码（E-AC-3 / AC-3 / DTS，常见于「扩展名被改错」的音源）
    /// 会**退回 ffmpeg 转码**：转成 AAC 落在转码缓存里，之后的播放直接命中缓存。
    pub fn play(&self, path: &str) -> Result<PlayerState, String> {
        let mut inner = self.inner.lock();
        // 输出已失效：本调用马上就会 append 新曲目，直接丢坏流即可（不必先续播旧曲）
        if self.output_lost.load(Ordering::SeqCst) {
            self.discard_output(&mut inner);
        }
        match Self::play_locked(&mut inner, &self.eq, &self.output_lost, path) {
            Ok(state) => Ok(state),
            Err(err) => {
                if !crate::commands::music::transcode::worth_retrying(&err) {
                    return Err(err);
                }
                // 转码要跑外部进程（秒级），**必须先把锁放掉**：
                // 占着锁的话前端每 500ms 的状态轮询会被一起卡住，界面像死了一样。
                drop(inner);
                let cached = crate::commands::music::transcode::ensure(path)?;
                crate::exit_log!("[音乐] 原编码无法解码，已转码后播放: {cached}");
                // 传**原路径**：play_locked 会自己找到转码副本，
                // 于是曲库标签、队列对齐、改名/删除都仍以用户的原文件为准。
                let (title, artist) = crate::commands::music::transcode::labels_for(path);
                // 本地文件，没有在线来源
                self.play_named(path, &title, &artist, None)
            }
        }
    }

    /// 播放并**指定显示名**（在线曲目专用）。
    ///
    /// 在线播放落在缓存目录里的文件名是内容哈希，既不在曲库里也没有可读性，
    /// 名字只能由调用方给；登记后连播下一首（走 `play_locked`）也能取到正确名字。
    ///
    /// `online` 是这首曲目的在线来源：留了它，界面上的「下载」才知道该去哪个插件取流
    /// （搜索结果清掉之后，路径本身是不够的）。本地曲目传 None。
    pub fn play_named(
        &self,
        path: &str,
        title: &str,
        artist: &str,
        online: Option<OnlineOrigin>,
    ) -> Result<PlayerState, String> {
        let mut inner = self.inner.lock();
        // 输出已失效：本调用马上就会 append 新曲目，直接丢坏流即可
        if self.output_lost.load(Ordering::SeqCst) {
            self.discard_output(&mut inner);
        }
        inner.names.insert(
            path.to_string(),
            TrackName {
                title: title.to_string(),
                artist: artist.to_string(),
            },
        );
        Self::play_locked(&mut inner, &self.eq, &self.output_lost, path)?;
        // 来源要在 play_locked 之后写：current 是它重建的
        if let Some(origin) = online {
            if let Some(current) = inner.current.as_mut() {
                current.online = Some(origin);
            }
        }
        Ok(Self::snapshot(&mut inner))
    }

    /// 当前曲目的在线来源（本地曲目为 None）——「下载当前曲目」用。
    pub fn current_online_origin(&self) -> Option<OnlineOrigin> {
        let inner = self.inner.lock();
        inner.current.as_ref().and_then(|c| c.online.clone())
    }

    /// 重置播放队列（前端在曲库或播放模式变化时调用）
    ///
    /// `current` 传当前正在播放的曲目路径：随机模式下会保证它仍在队列中且被选中，
    /// 避免重排后把正在播的那首「挤掉」。
    pub fn set_queue(&self, paths: Vec<String>, mode: &str) -> Result<(), String> {
        let items = paths.into_iter().map(QueueItem::Path).collect();
        let mut inner = self.inner.lock();
        let current = inner.current.as_ref().map(|c| c.path.clone());
        let mode = PlayMode::from_str(mode);
        inner.queue.set(items, mode, current.as_deref());
        Ok(())
    }

    /// 用**队列条目**重置队列（在线搜索结果替换播放列表时用）；播放模式沿用当前设置。
    ///
    /// 与 [`Self::set_queue`] 的区别是条目可以是**还没落盘的在线曲目**。
    pub fn set_queue_items(
        &self,
        items: Vec<QueueItem>,
        current: Option<&str>,
    ) -> Result<(), String> {
        let mode = PlayMode::from_str(&settings::load_settings().play_mode);
        let mut inner = self.inner.lock();
        // 没指定就以「正在播的那首」为当前曲：随机模式下它必须留在队列里
        let current = current
            .map(str::to_string)
            .or_else(|| inner.current.as_ref().map(|c| c.path.clone()));
        inner.queue.set(items, mode, current.as_deref());
        Ok(())
    }

    /// 曲目在磁盘上改名后同步当前曲目与队列里的路径（播放不中断）。
    pub fn rename_path(&self, old: &str, new: &str) {
        let mut inner = self.inner.lock();
        if let Some(current) = inner.current.as_mut() {
            if current.path == old {
                current.path = new.to_string();
            }
        }
        // 登记名跟着换 key，否则改名后在线曲目的名字会丢失
        if let Some(name) = inner.names.remove(old) {
            inner.names.insert(new.to_string(), name);
        }
        inner.queue.rename_path(old, new);
    }

    /// 曲目被删除后同步播放器：从队列移除；返回「被删的是不是当前曲目」。
    ///
    /// 调用方据此决定是否切下一首 —— 这里**不能**自己调 `next()`：
    /// `parking_lot::Mutex` 不可重入，在持锁状态下再取一次锁会直接死锁。
    pub fn forget_paths(&self, paths: &[String]) -> bool {
        let mut inner = self.inner.lock();
        let playing_removed = inner
            .current
            .as_ref()
            .map(|c| paths.iter().any(|p| p == &c.path))
            .unwrap_or(false);
        let current_removed = inner.queue.remove_paths(paths) || playing_removed;
        if current_removed {
            // 立刻停掉输出，避免继续播放一个已被删除的文件
            if let Some(player) = inner.player.as_ref() {
                player.clear();
                player.stop();
            }
            inner.current = None;
            inner.status = PlayStatus::Idle;
        }
        current_removed
    }

    /// 下一首（用户点「下一首」）
    pub fn next(&self) -> Result<AdvanceOutcome, String> {
        let mut inner = self.inner.lock();
        let item = inner.queue.advance();
        self.step(&mut inner, item)
    }

    /// 上一首
    pub fn prev(&self) -> Result<AdvanceOutcome, String> {
        let mut inner = self.inner.lock();
        let item = inner.queue.back();
        self.step(&mut inner, item)
    }

    /// 推进到指定条目并播放；条是在线曲目时把取流任务交回调用方。
    pub(super) fn step(
        &self,
        inner: &mut Inner,
        item: Option<QueueItem>,
    ) -> Result<AdvanceOutcome, String> {
        let Some(item) = item else {
            return Ok(AdvanceOutcome::Playing(Self::snapshot(inner)));
        };
        match item {
            QueueItem::Path(path) => Self::play_locked(inner, &self.eq, &self.output_lost, &path)
                .map(AdvanceOutcome::Playing),
            QueueItem::Online {
                file,
                item,
                quality,
            } => {
                // 记下「正在取流」：巡查线程据此停止推进，否则每 250ms 推进一次会连跳好几首
                inner.seq += 1;
                let pending = PendingOnline {
                    file,
                    item,
                    quality,
                    seq: inner.seq,
                };
                inner.pending = Some(pending.clone());
                // 挪出 Playing：输出队列已经空了，留着会让巡查判定「又播完一首」而立即再推进
                inner.status = PlayStatus::Idle;
                Ok(AdvanceOutcome::Online(pending))
            }
        }
    }

    /// 在线曲目已落盘：接上播放（由取流任务在拿到本地文件后调用）。
    ///
    /// `seq` 对不上说明用户已经切到别的曲目 —— 这次结果作废，不打断当前播放。
    pub fn play_online_resolved(
        &self,
        path: &str,
        title: &str,
        artist: &str,
        seq: u64,
    ) -> Result<PlayerState, String> {
        let mut inner = self.inner.lock();
        let matches = inner
            .pending
            .as_ref()
            .map(|p| p.seq == seq)
            .unwrap_or(false);
        let origin = inner.pending.clone();
        inner.pending = None;
        if !matches {
            return Ok(Self::snapshot(&mut inner));
        }
        // 落盘后换成本地路径：回退 / 切模式时不必再取一次流
        inner.queue.materialize_current(path);
        if self.output_lost.load(Ordering::SeqCst) {
            self.discard_output(&mut inner);
        }
        inner.names.insert(
            path.to_string(),
            TrackName {
                title: title.to_string(),
                artist: artist.to_string(),
            },
        );
        Self::play_locked(&mut inner, &self.eq, &self.output_lost, path)?;
        // 记下来源：界面上的「下载」要靠它重新取流（一次播放只查一次插件名，不进轮询）
        if let Some(pending) = origin {
            if let Some(current) = inner.current.as_mut() {
                current.online = Some(OnlineOrigin {
                    file: pending.file.clone(),
                    item: pending.item.clone(),
                    quality: pending.quality.clone(),
                    platform: plugin_display_name(&pending.file),
                });
            }
        }
        Ok(Self::snapshot(&mut inner))
    }

    /// 在线曲目取流失败：清掉「正在取流」标记（不清的话巡查线程会一直等下去）。
    pub fn abandon_online(&self, seq: u64) {
        let mut inner = self.inner.lock();
        if inner
            .pending
            .as_ref()
            .map(|p| p.seq == seq)
            .unwrap_or(false)
        {
            inner.pending = None;
        }
    }
}
