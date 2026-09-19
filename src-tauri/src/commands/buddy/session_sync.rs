//! 会话同步的「增量判定 + 明细台账」。
//!
//! 语义取自 WorkDaddy `daemon.js` 的 auto-copy job（见
//! `.agents/skills/cockpit-buddy-sync/references/workdaddy.md` §17）：
//! 切换账号时**只处理有变化的会话**，并把每个会话的结局（复制 / 跳过 / 部分失败 /
//! 冲突 / 失败）记成明细返回给前端，而不是只回一个总数。
//!
//! 与参考的差异（适配说明）：
//! - 参考按 lineage 的 `contentMtime` 与 `mapping.updatedAt` 做基线；我们的会话记录在
//!   两个不同位置（WorkBuddy `workbuddy.db` / 扩展目录 `index.json`），因此基线以
//!   **目标账号** 维度落盘：`{data_dir}/buddy/session-sync/<platform>/<target_uid>.json`，
//!   key = 会话 id，value = 指纹（时间戳 + 内容时间）。
//! - 参考的冲突是「两侧都改过」，我们的对应场景是扩展目录里来源与目标各自持有一份同 id
//!   会话且两侧都比基线新 —— 此时**不猜测、不覆盖**，记为 `conflict` 交用户处理。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 明细最多保留多少条（与参考一致，避免一次切换产生超大载荷）
pub(crate) const MAX_DETAILS: usize = 500;

/// 单个会话在一次同步中的结局。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionSyncStatus {
    /// 有实际写入（新增或更新）
    Copied,
    /// 与上次同步相比无变化，本次未处理
    Skipped,
    /// 已处理但有文件级失败 / 内容缺失
    Partial,
    /// 两侧都改过，拒绝覆盖
    Conflict,
    /// 处理抛出错误
    Failed,
}

/// 明细里的一条：某个会话的结局与原因码。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSyncDetail {
    pub id: String,
    /// 展示名（标题优先，其次工作区路径，最后回落到 id）
    pub label: String,
    pub status: SessionSyncStatus,
    /// 机器可读原因码，前端用 `buddy.syncReason.<reason>` 翻译；未知码直接展示原文
    pub reason: String,
}

/// 一次同步的汇总（随 `SessionTransferReport` 返回前端）。
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSyncSummary {
    /// 判定过的会话总数
    pub total: usize,
    pub copied: usize,
    pub skipped: usize,
    pub partial: usize,
    pub conflict: usize,
    pub failed: usize,
    /// 全部会话都无变化（前端可据此跳过明细展开）
    pub unchanged: bool,
    pub details: Vec<SessionSyncDetail>,
}

/// 增量同步台账：读取上次基线、记录本次结局、产出新基线。
pub(crate) struct SyncTracker {
    baseline: BTreeMap<String, String>,
    next: BTreeMap<String, String>,
    summary: SessionSyncSummary,
}

impl SyncTracker {
    pub(crate) fn new(baseline: BTreeMap<String, String>) -> Self {
        Self {
            baseline,
            next: BTreeMap::new(),
            summary: SessionSyncSummary::default(),
        }
    }

    /// 上次同步时该会话的指纹（None = 首次见到）
    pub(crate) fn baseline_of(&self, id: &str) -> Option<&str> {
        self.baseline.get(id).map(String::as_str)
    }

    /// 该会话本次是否与基线一致（一致即无需处理）
    pub(crate) fn is_unchanged(&self, id: &str, fingerprint: &str) -> bool {
        self.baseline_of(id) == Some(fingerprint)
    }

    /// 记录一个会话的结局。`fingerprint` 为 Some 时写入新基线（即"已确认同步到这个状态"）。
    pub(crate) fn record(
        &mut self,
        id: &str,
        label: &str,
        status: SessionSyncStatus,
        reason: &str,
        fingerprint: Option<String>,
    ) {
        self.summary.total += 1;
        match status {
            SessionSyncStatus::Copied => self.summary.copied += 1,
            SessionSyncStatus::Skipped => self.summary.skipped += 1,
            SessionSyncStatus::Partial => self.summary.partial += 1,
            SessionSyncStatus::Conflict => self.summary.conflict += 1,
            SessionSyncStatus::Failed => self.summary.failed += 1,
        }
        if !id.is_empty() {
            if let Some(value) = fingerprint {
                self.next.insert(id.to_string(), value);
            } else {
                // 未给出新指纹时沿用旧值，避免"处理失败"把基线抹掉导致下次重做
                if let Some(previous) = self.baseline.get(id) {
                    self.next.insert(id.to_string(), previous.clone());
                }
            }
        }
        if self.summary.details.len() < MAX_DETAILS {
            self.summary.details.push(SessionSyncDetail {
                id: id.to_string(),
                label: truncate_label(label),
                status,
                reason: reason.to_string(),
            });
        }
    }

    /// 收尾：产出汇总与新基线。
    pub(crate) fn finish(mut self) -> (SessionSyncSummary, BTreeMap<String, String>) {
        self.summary.unchanged = self.summary.total > 0
            && self.summary.copied == 0
            && self.summary.partial == 0
            && self.summary.conflict == 0
            && self.summary.failed == 0;
        (self.summary, self.next)
    }
}

fn truncate_label(label: &str) -> String {
    const MAX: usize = 120;
    let trimmed = label.trim();
    if trimmed.chars().count() <= MAX {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(MAX).collect();
    out.push('…');
    out
}

// ─── 基线落盘 ───

/// 基线文件：`{data_dir}/buddy/session-sync/<platform>/<target_uid>.json`
pub(crate) fn baseline_file(platform_label: &str, target_uid: &str) -> Result<PathBuf, String> {
    let dir = crate::commands::config::get_data_dir()
        .join("buddy")
        .join("session-sync")
        .join(platform_label);
    Ok(dir.join(format!("{}.json", sanitize_file_component(target_uid)?)))
}

/// uid 直接参与文件名，必须防止路径穿越。
fn sanitize_file_component(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
        return Err("会话同步基线：uid 不是安全的文件名".to_string());
    }
    Ok(trimmed.to_string())
}

#[derive(serde::Serialize, serde::Deserialize)]
struct BaselineFile {
    version: u32,
    entries: BTreeMap<String, String>,
}

const BASELINE_VERSION: u32 = 1;

/// 读取上次同步基线；文件缺失或损坏时返回空表（等价首次同步，行为退化为全量）。
pub(crate) fn load_baseline(platform_label: &str, target_uid: &str) -> BTreeMap<String, String> {
    let Ok(path) = baseline_file(platform_label, target_uid) else {
        return BTreeMap::new();
    };
    let Ok(content) = std::fs::read_to_string(&path) else {
        return BTreeMap::new();
    };
    match serde_json::from_str::<BaselineFile>(&content) {
        Ok(file) if file.version == BASELINE_VERSION => file.entries,
        Ok(_) => {
            eprintln!(
                "[Buddy SessionSync] 基线版本不认识，按首次同步处理: {}",
                path.display()
            );
            BTreeMap::new()
        }
        Err(error) => {
            eprintln!(
                "[Buddy SessionSync] 基线解析失败，按首次同步处理: path={}, error={}",
                path.display(),
                error
            );
            BTreeMap::new()
        }
    }
}

/// 原子写入新基线。失败只告警不中断切换（下次退化为全量，安全但慢一点）。
pub(crate) fn save_baseline(
    platform_label: &str,
    target_uid: &str,
    entries: &BTreeMap<String, String>,
) {
    let Ok(path) = baseline_file(platform_label, target_uid) else {
        return;
    };
    let payload = BaselineFile {
        version: BASELINE_VERSION,
        entries: entries.clone(),
    };
    let Ok(serialized) = serde_json::to_string_pretty(&payload) else {
        return;
    };
    if let Err(error) = super::store::write_atomic(&path, &serialized) {
        eprintln!(
            "[Buddy SessionSync] 基线写入失败（下次将退化为全量同步）: path={}, error={}",
            path.display(),
            error
        );
    }
}

// ─── 指纹 ───

/// 文件指纹：`长度:修改时间(毫秒)`。取不到返回 None。
pub(crate) fn file_fingerprint(path: &Path) -> Option<String> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if metadata.file_type().is_symlink() {
        return None;
    }
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    Some(format!("{}:{}", metadata.len(), modified))
}

/// 目录指纹：递归取内部文件指纹里最大者（按修改时间）写入 `best`。
fn collect_directory_fingerprint(
    dir: &Path,
    remaining_depth: usize,
    best: &mut Option<(u128, String)>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            if remaining_depth > 0 {
                collect_directory_fingerprint(&path, remaining_depth - 1, best);
            }
            continue;
        }
        let Some(fingerprint) = file_fingerprint(&path) else {
            continue;
        };
        let mtime = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis())
            .unwrap_or_default();
        if best.as_ref().is_none_or(|(current, _)| mtime > *current) {
            *best = Some((mtime, fingerprint));
        }
    }
}

/// WorkBuddy 会话落盘指纹：`~/.workbuddy` 下的 5 类路径取内容修改时间最大者。
///
/// 参考 `sessionContentMtime`：`projects/<hash>/<id>.jsonl|/`、`workspace/sessions/<id>`、
/// `tasks/<id>`、`file-history/<id>`、`artifact-index/<id>.json`。
/// 返回 None 表示**会话记录存在但正文已丢失**（对应前端 `contentMissing`）。
pub(crate) fn workbuddy_session_fingerprint(data_root: &Path, id: &str) -> Option<String> {
    if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
        return None;
    }
    let mut best: Option<(u128, String)> = None;
    if let Ok(projects) = std::fs::read_dir(data_root.join("projects")) {
        for entry in projects.flatten() {
            let project_root = entry.path();
            if !project_root.is_dir() {
                continue;
            }
            consider_path(&mut best, &project_root.join(format!("{}.jsonl", id)));
            consider_path(&mut best, &project_root.join(id));
        }
    }
    consider_path(&mut best, &data_root.join("workspace").join("sessions").join(id));
    consider_path(&mut best, &data_root.join("tasks").join(id));
    consider_path(&mut best, &data_root.join("file-history").join(id));
    consider_path(
        &mut best,
        &data_root
            .join("artifact-index")
            .join(format!("{}.json", id)),
    );
    best.map(|(_, value)| value)
}

/// 把单个路径的指纹并入 `best`（取修改时间最大者）。目录取其内部文件的最大指纹。
fn consider_path(best: &mut Option<(u128, String)>, path: &Path) {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return;
    };
    if metadata.file_type().is_symlink() {
        return;
    }
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    if metadata.is_dir() {
        // 目录内部文件的 mtime 更权威，直接递归取最大值
        collect_directory_fingerprint(path, 3, best);
        return;
    }
    let Some(fingerprint) = file_fingerprint(path) else {
        return;
    };
    if best.as_ref().is_none_or(|(current, _)| mtime > *current) {
        *best = Some((mtime, fingerprint));
    }
}

/// 组合指纹：`更新时间的毫秒 + 内容指纹`，任一部分变化都算"有变化"。
pub(crate) fn combined_fingerprint(updated_at: i64, content: Option<&str>) -> String {
    format!("{}|{}", updated_at, content.unwrap_or("-"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_temp(tag: &str) -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "kira-session-sync-{}-{}-{}",
            tag,
            std::process::id(),
            id
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn tracker_counts_each_status_and_keeps_baseline_on_failure() {
        let mut baseline = BTreeMap::new();
        baseline.insert("a".to_string(), "v1".to_string());
        let mut tracker = SyncTracker::new(baseline);

        tracker.record("a", "会话 A", SessionSyncStatus::Skipped, "unchanged", Some("v1".into()));
        tracker.record("b", "会话 B", SessionSyncStatus::Copied, "firstSync", Some("v2".into()));
        tracker.record("c", "会话 C", SessionSyncStatus::Conflict, "bothChanged", None);
        tracker.record("d", "会话 D", SessionSyncStatus::Failed, "ioError", None);
        tracker.record("a", "会话 A", SessionSyncStatus::Partial, "contentMissing", Some("v1".into()));

        let (summary, next) = tracker.finish();
        assert_eq!(summary.total, 5);
        assert_eq!(summary.skipped, 1);
        assert_eq!(summary.copied, 1);
        assert_eq!(summary.conflict, 1);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.partial, 1);
        assert!(!summary.unchanged);
        assert_eq!(summary.details.len(), 5);
        // 失败/冲突时沿用旧基线，避免下次重做
        assert_eq!(next.get("a").map(String::as_str), Some("v1"));
        assert_eq!(next.get("b").map(String::as_str), Some("v2"));
        assert!(next.get("c").is_none());
    }

    #[test]
    fn tracker_marks_all_skipped_as_unchanged() {
        let mut tracker = SyncTracker::new(BTreeMap::new());
        tracker.record("a", "A", SessionSyncStatus::Skipped, "unchanged", Some("v1".into()));
        tracker.record("b", "B", SessionSyncStatus::Skipped, "unchanged", Some("v2".into()));
        let (summary, next) = tracker.finish();
        assert!(summary.unchanged);
        assert_eq!(next.len(), 2);
    }

    #[test]
    fn details_are_capped() {
        let mut tracker = SyncTracker::new(BTreeMap::new());
        for index in 0..(MAX_DETAILS + 20) {
            tracker.record(
                &format!("id-{}", index),
                "会话",
                SessionSyncStatus::Copied,
                "firstSync",
                None,
            );
        }
        let (summary, _) = tracker.finish();
        assert_eq!(summary.details.len(), MAX_DETAILS);
        assert_eq!(summary.total, MAX_DETAILS + 20);
    }

    #[test]
    fn baseline_file_rejects_unsafe_uid() {
        assert!(baseline_file("workbuddy", "../evil").is_err());
        assert!(baseline_file("workbuddy", "a/b").is_err());
        assert!(baseline_file("workbuddy", "384c6dd0-c1bc-4ae2-a0d0-f70350c62f7b").is_ok());
    }

    #[test]
    fn session_fingerprint_tracks_message_file_changes() {
        let root = make_temp("fingerprint");
        let project = root.join("projects").join("hash");
        std::fs::create_dir_all(&project).unwrap();
        let message_file = project.join("session-1.jsonl");
        std::fs::write(&message_file, b"{}\n").unwrap();
        let first = workbuddy_session_fingerprint(&root, "session-1");
        assert!(first.is_some(), "存在消息文件时应能取到指纹");

        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&message_file, b"{}\n{}\n").unwrap();
        let second = workbuddy_session_fingerprint(&root, "session-1");
        assert_ne!(first, second, "内容变化后指纹必须变化");

        assert!(workbuddy_session_fingerprint(&root, "session-missing").is_none());
        assert!(workbuddy_session_fingerprint(&root, "../evil").is_none());
    }
}
