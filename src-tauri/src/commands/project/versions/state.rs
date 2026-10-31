use super::*;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//  数据结构
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use std::collections::HashMap;
use std::sync::Mutex;
use tokio::task::AbortHandle;

/// 下载进度事件
#[derive(Serialize, Clone)]
pub struct DownloadProgress {
    pub sdk: String,
    pub downloaded: u64,
    pub total: u64,
    pub pct: u8,
    pub speed_str: String,
}

#[derive(Serialize, Clone)]
pub(super) struct InstallStepPayload {
    pub(super) sdk: String,
    pub(super) step: String,
}

/// 进行中安装的状态（内存态，供前端挂载时查询恢复进度条）。
/// 卸载/取消/完成后由任务自行移除对应条目。
/// 进行中安装的可查询视图（不含 abort handle）。
#[derive(Serialize, Clone)]
pub struct ActiveInstall {
    pub sdk: String,
    pub version: String,
    pub step: String,
    pub downloaded: u64,
    pub total: u64,
    pub pct: u8,
    pub speed_str: String,
}

// 内部存 abort handle，查询时剥离，避免把 handle 序列化给前端。
pub(super) struct ActiveInstallEntry {
    pub active: ActiveInstall,
    pub abort_handle: Option<AbortHandle>,
    pub cancellation: crate::commands::foundation::owned_process::Cancellation,
}

pub(super) struct TempDirGuard {
    pub(super) path: PathBuf,
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        if self.path.exists() {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

pub(super) fn get_active_downloads() -> &'static Mutex<HashMap<String, ActiveInstallEntry>> {
    static ACTIVE_DOWNLOADS: std::sync::OnceLock<Mutex<HashMap<String, ActiveInstallEntry>>> =
        std::sync::OnceLock::new();
    ACTIVE_DOWNLOADS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 同步进行中安装的步骤到内存态（供前端挂载时恢复），同时广播步骤事件。
pub(super) fn sync_install_step(app: &AppHandle, sdk: &str, step: &str) {
    let _ = app.emit(
        "install-step",
        InstallStepPayload {
            sdk: sdk.to_string(),
            step: step.to_string(),
        },
    );
    if let Some(entry) = get_active_downloads().lock().unwrap().get_mut(sdk) {
        entry.active.step = step.to_string();
    }
}

pub(super) struct InstallRegistrationGuard {
    pub(super) id: String,
    pub(super) cancellation: crate::commands::foundation::owned_process::Cancellation,
}
impl Drop for InstallRegistrationGuard {
    fn drop(&mut self) {
        let mut active = get_active_downloads()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if active
            .get(&self.id)
            .is_some_and(|entry| entry.cancellation.same(&self.cancellation))
        {
            active.remove(&self.id);
        }
    }
}

#[cfg(test)]
mod ownership_tests {
    use super::*;
    #[test]
    fn stale_install_cleanup_cannot_remove_new_registration() {
        let id = "qa-isolated-install-registration".to_string();
        let old = crate::commands::foundation::owned_process::Cancellation::default();
        let new = crate::commands::foundation::owned_process::Cancellation::default();
        get_active_downloads().lock().unwrap().insert(
            id.clone(),
            ActiveInstallEntry {
                active: ActiveInstall {
                    sdk: id.clone(),
                    version: "fake".into(),
                    step: "test".into(),
                    downloaded: 0,
                    total: 0,
                    pct: 0,
                    speed_str: String::new(),
                },
                abort_handle: None,
                cancellation: new.clone(),
            },
        );
        drop(InstallRegistrationGuard {
            id: id.clone(),
            cancellation: old,
        });
        assert!(get_active_downloads().lock().unwrap().contains_key(&id));
        drop(InstallRegistrationGuard {
            id: id.clone(),
            cancellation: new,
        });
        assert!(!get_active_downloads().lock().unwrap().contains_key(&id));
    }
}
