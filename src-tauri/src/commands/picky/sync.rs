use super::*;

// ─── 命令：云同步配置 ───

#[tauri::command]
pub fn picky_get_sync_config() -> Result<PickySyncConfig, String> {
    let conn = open_db()?;
    Ok(load_sync_config(&conn))
}

#[tauri::command]
pub fn picky_save_sync_config(config: PickySyncConfig) -> Result<(), String> {
    let conn = open_db()?;
    save_sync_config_db(&conn, &config)
}

// ─── 自动同步调度（防抖） ───

/// 防抖窗口：内容变更后合并等待这段时间再同步，避免连续操作触发多次上传。
pub(super) const AUTO_SYNC_DEBOUNCE: Duration = Duration::from_secs(3);

pub(super) static LAST_CHANGE: Mutex<Option<Instant>> = Mutex::new(None);
pub(super) static SYNC_RUNNING: AtomicBool = AtomicBool::new(false);
/// 同步进行中又到达变更时置位，确保同步结束后补一轮（修复漏同步窗口）。
pub(super) static PENDING_RESYNC: AtomicBool = AtomicBool::new(false);

struct SyncRunningGuard<'a>(&'a AtomicBool);
impl Drop for SyncRunningGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}
pub(super) fn owned_task_name(prefix: &str) -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!("{prefix}-{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

/// 内容变更后调用：记录变更时间并安排一次防抖同步。
/// 连续变更只触发一次同步（距最后一次变更 3 秒后才执行）。
pub fn schedule_auto_sync() {
    if !crate::runtime_tasks::is_enabled("picky") {
        return;
    }
    *LAST_CHANGE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    crate::runtime_tasks::spawn_task("picky", &owned_task_name("auto-sync"), async {
        tokio::time::sleep(AUTO_SYNC_DEBOUNCE).await;
        // 防抖：睡眠期间有新变更则放弃本次，由最新变更的任务接手
        let should = LAST_CHANGE
            .lock()
            .unwrap()
            .is_some_and(|t| t.elapsed() >= AUTO_SYNC_DEBOUNCE);
        if !should {
            return;
        }
        if SYNC_RUNNING.swap(true, Ordering::SeqCst) {
            // 已有同步在跑：置位待补同步，由运行中的同步结束后接手，
            // 避免本任务提前返回后该变更从此被漏掉。
            PENDING_RESYNC.store(true, Ordering::SeqCst);
            return;
        }
        let running = SyncRunningGuard(&SYNC_RUNNING);
        let sync_started = Instant::now();
        if let Err(e) = picky_sync_now().await {
            crate::exit_log::exit_log(&format!("[picky-sync] 自动同步失败（稍后手动同步）: {}", e));
        }
        drop(running);
        // 同步期间又产生了新变更（防抖任务置位或变更时间晚于同步开始）→ 再调度一轮
        let has_new = PENDING_RESYNC.swap(false, Ordering::SeqCst)
            || LAST_CHANGE
                .lock()
                .unwrap()
                .is_some_and(|t| t > sync_started);
        if has_new {
            schedule_auto_sync();
        }
    });
}

/// 启动/退出时调用：仅当已配置并启用云同步时执行一次同步（静默忽略未配置）。
/// 用于应用启动拉取云端、退出前推送本地。
pub async fn picky_sync_if_enabled() {
    let conn = open_db();
    let cfg = match conn {
        Ok(c) => load_sync_config(&c),
        Err(_) => return,
    };
    if !cfg.is_configured() {
        return;
    }
    let _ = picky_sync_now().await;
}

// ─── 命令：云同步 ───

/// 双向同步（一个按钮内部自动合并）：
/// 1) 先下载云端状态并合并到本地（LWW：本地缺失补入，云端 updatedAt 更新则覆盖）；
/// 2) 再上传合并后的全量状态。
/// 这样无论哪端先/后同步，云端始终是两端数据的并集，绝不互相覆盖丢数据。
/// 返回结果文本。
#[tauri::command]
pub async fn picky_sync_now() -> Result<String, String> {
    crate::runtime_tasks::run_if_enabled("picky", async move {
        crate::exit_log::exit_log("[picky-sync] 开始云同步");
        let conn = open_db()?;
        let cfg = load_sync_config(&conn);
        if !cfg.is_configured() {
            crate::exit_log::exit_log("[picky-sync] 未配置云同步，终止");
            return Err(
                "云同步未配置（需启用并填写 endpoint / AccessKey / SecretKey / Bucket）"
                    .to_string(),
            );
        }
        crate::exit_log!(
            "[picky-sync] endpoint={:?} bucket={:?} prefix={:?} enabled={}",
            cfg.endpoint,
            cfg.bucket_name,
            cfg.prefix,
            cfg.enabled
        );

        // 1) 先合并云端到本地（不存在则下载并合并，404 视为首次同步）
        let client = s3::PickyS3Client::new(cfg.clone());
        let merged = match client.download_full_state().await? {
            Some(cloud) => {
                let counts = merge_cloud_state(&conn, &cloud)?;
                // 老墓碑到期清理：本次已按墓碑跳过云端旧数据，接下来只留近期删除记录
                prune_tombstones(&conn);
                let s = format_merge_summary(&counts, "已合并云端");
                crate::exit_log!("[picky-sync] 下载并合并云端成功: {}", s);
                s
            }
            None => {
                crate::exit_log::exit_log("[picky-sync] 云端暂无数据（首次同步）");
                "云端暂无数据（首次同步）".to_string()
            }
        };

        // 2) 上传合并后的全量状态
        let state = state_to_json(&conn)?;
        let bookmarks: Vec<JsonValue> = state
            .bookmarks
            .iter()
            .map(|b| serde_json::to_value(b).unwrap_or_default())
            .collect();
        let comments: Vec<JsonValue> = state
            .comments
            .iter()
            .map(|c| serde_json::to_value(c).unwrap_or_default())
            .collect();
        let tags: Vec<JsonValue> = state
            .tags
            .iter()
            .map(|t| serde_json::to_value(t).unwrap_or_default())
            .collect();
        let bm_tags: JsonMap<String, JsonValue> = state
            .bookmark_tags
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::json!(v)))
            .collect();

        crate::exit_log!(
            "[picky-sync] 准备上传: bookmarks={} comments={} tags={}",
            bookmarks.len(),
            comments.len(),
            tags.len()
        );
        let synced_at = client
            .upload_full_state(&bookmarks, &comments, &tags, &bm_tags)
            .await?;
        crate::exit_log!("[picky-sync] 上传成功，syncedAt={:?}", synced_at);

        // 记录上次同步时间
        let mut updated = cfg.clone();
        updated.last_sync_at = Some(synced_at.clone());
        let conn2 = open_db()?;
        save_sync_config_db(&conn2, &updated)?;

        crate::exit_log::exit_log("[picky-sync] 云同步完成");
        Ok(format!(
            "同步完成：bookmarks={} comments={} tags={} · {} · 同步时间 {}",
            bookmarks.len(),
            comments.len(),
            tags.len(),
            merged,
            synced_at
        ))
    })
    .await
}

#[cfg(test)]
mod ownership_tests {
    use super::*;
    #[test]
    fn dropping_pending_sync_releases_running_flag() {
        let flag = AtomicBool::new(true);
        let guard = SyncRunningGuard(&flag);
        let pending = async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        };
        drop(pending);
        assert!(!flag.load(Ordering::SeqCst));
    }
}
