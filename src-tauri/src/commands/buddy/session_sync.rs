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
    /// 该会话所属的目录 / 项目。CodeBuddy 侧是 history 下的工作区目录名，
    /// WorkBuddy 侧是 `sessions.cwd`；取不到时为 None，前端显示占位符。
    pub workspace: Option<String>,
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
    /// 「当前工作区」上下文：`set_workspace` 设定后，其后 `record` 的明细都会带上它。
    /// 仅在按工作区目录逐个处理的调用链里使用（见 `set_workspace`）。
    current_workspace: Option<String>,
}

impl SyncTracker {
    pub(crate) fn new(baseline: BTreeMap<String, String>) -> Self {
        Self {
            baseline,
            next: BTreeMap::new(),
            summary: SessionSyncSummary::default(),
            current_workspace: None,
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

    /// 设定「当前工作区」上下文：其后 `record` 记录的明细都会带上它。
    ///
    /// 供「一次调用只处理一个工作区目录」的调用方使用（CodeBuddy 的
    /// `merge_workspace_history` 正是逐个工作区目录处理的）。逐行处理、每条目录
    /// 都可能不同的调用方（如 WorkBuddy 的共享会话库）请用 [`SyncTracker::record_in`]。
    pub(crate) fn set_workspace(&mut self, workspace: Option<String>) {
        self.current_workspace = workspace;
    }

    /// 记录一个会话的结局。`fingerprint` 为 Some 时写入新基线（即"已确认同步到这个状态"）。
    ///
    /// 工作区取当前上下文（见 [`SyncTracker::set_workspace`]）。
    pub(crate) fn record(
        &mut self,
        id: &str,
        label: &str,
        status: SessionSyncStatus,
        reason: &str,
        fingerprint: Option<String>,
    ) {
        let workspace = self.current_workspace.clone();
        self.record_in(id, label, status, reason, fingerprint, workspace);
    }

    /// 同上，但显式指定本条明细所属的工作区（拿不到就传 None）。
    pub(crate) fn record_in(
        &mut self,
        id: &str,
        label: &str,
        status: SessionSyncStatus,
        reason: &str,
        fingerprint: Option<String>,
        workspace: Option<String>,
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
                // 不做 120 字截断：路径尾部（项目名）才是识别关键，且前端要
                // 用完整路径做 title 提示，按宽度折叠交给 CSS。
                workspace,
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

// ─── 待处理冲突 ───

/// 一条待用户处理的会话冲突（切换账号时两侧都改过、被拒绝自动覆盖的会话）。
///
/// 合并流程只负责把它记下来，**如何取舍由用户决定**：查看明细后可选
/// 合并（取较新）/ 覆盖目标（用来源）/ 保留目标（丢弃来源修改）。
/// 记录里带全部定位信息（ide / workspace / 双方 uid），解析命令不必重新扫描全盘。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingConflict {
    pub id: String,
    /// 展示名（标题优先，回落到目录 —— 不再回落到裸 id）
    pub label: String,
    /// 所属工作区目录名（CodeBuddy history 下的一级目录，**原始哈希名**，
    /// 解析命令靠它定位文件系统路径，不要改成显示用路径）
    pub workspace: String,
    /// 工作区对应的**真实项目路径**（由哈希还原，尽力而为；还原失败为 None，
    /// 前端展示时回退到 workspace 原始名）
    #[serde(default)]
    pub workspace_path: Option<String>,
    /// IDE 目录名（uid 下的一级目录，定位来源/目标会话路径必需）
    pub ide: String,
    pub source_uid: String,
    pub target_uid: String,
    /// 来源侧最后消息时间（epoch 毫秒）
    pub source_stamp: i64,
    /// 目标侧最后消息时间（epoch 毫秒）
    pub target_stamp: i64,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PendingConflictFile {
    version: u32,
    conflicts: Vec<PendingConflict>,
}

const PENDING_CONFLICTS_VERSION: u32 = 1;

/// 待处理冲突文件：与基线同目录，`<target_uid>.conflicts.json`。
/// 冲突没有随基线写入（基线写了就等于替用户做了「保留目标」的决定），
/// 单独落盘，切换结束后仍可逐条处理。
pub(crate) fn pending_conflicts_file(platform_label: &str, target_uid: &str) -> Result<PathBuf, String> {
    let dir = crate::commands::config::get_data_dir()
        .join("buddy")
        .join("session-sync")
        .join(platform_label);
    Ok(dir.join(format!("{}.conflicts.json", sanitize_file_component(target_uid)?)))
}

/// 读取待处理冲突；文件缺失或损坏时返回空（损坏只告警，不阻塞列表展示）。
pub(crate) fn load_pending_conflicts(platform_label: &str, target_uid: &str) -> Vec<PendingConflict> {
    let Ok(path) = pending_conflicts_file(platform_label, target_uid) else {
        return Vec::new();
    };
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    match serde_json::from_str::<PendingConflictFile>(&content) {
        Ok(file) if file.version == PENDING_CONFLICTS_VERSION => file.conflicts,
        Ok(_) => {
            eprintln!(
                "[Buddy SessionSync] 待处理冲突文件版本不认识，忽略: {}",
                path.display()
            );
            Vec::new()
        }
        Err(error) => {
            eprintln!(
                "[Buddy SessionSync] 待处理冲突文件解析失败，忽略: path={}, error={}",
                path.display(),
                error
            );
            Vec::new()
        }
    }
}

/// 原子写入待处理冲突；列表为空时直接删文件（「全部处理完」也是一种要记住的状态）。
pub(crate) fn save_pending_conflicts(
    platform_label: &str,
    target_uid: &str,
    conflicts: &[PendingConflict],
) {
    let Ok(path) = pending_conflicts_file(platform_label, target_uid) else {
        return;
    };
    if conflicts.is_empty() {
        if let Err(error) = std::fs::remove_file(&path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!(
                    "[Buddy SessionSync] 清理待处理冲突文件失败: path={}, error={}",
                    path.display(),
                    error
                );
            }
        }
        return;
    }
    let payload = PendingConflictFile {
        version: PENDING_CONFLICTS_VERSION,
        conflicts: conflicts.to_vec(),
    };
    let Ok(serialized) = serde_json::to_string_pretty(&payload) else {
        return;
    };
    if let Err(error) = super::store::write_atomic(&path, &serialized) {
        eprintln!(
            "[Buddy SessionSync] 待处理冲突写入失败: path={}, error={}",
            path.display(),
            error
        );
    }
}

// ─── 冲突处理结果 ───

/// 一条冲突的**处理结果**。
///
/// 没有它，冲突裁决就是「删掉待办」——会话明细里那条永远停在切换那一刻的
/// 初始状态（conflict / bothChanged），用户看不出自己处理过、也看不出怎么处理的。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictResolution {
    /// 会话 id（与 `SessionSyncDetail.id` / `PendingConflict.id` 同一主键）
    pub id: String,
    /// merge | overwrite | keep
    pub action: String,
    /// 处理时间（epoch 毫秒）
    pub at_ms: i64,
    /// 后端给的人类可读结果（如「已用来源会话（…）覆盖目标（…）」）
    pub message: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ConflictResolutionFile {
    version: u32,
    resolutions: Vec<ConflictResolution>,
}

const CONFLICT_RESOLUTIONS_VERSION: u32 = 1;
/// 结果只保留最近这些条：它是「本次可追溯的记录」，不是无限增长的审计日志。
const MAX_RESOLUTIONS: usize = 500;

/// 处理结果文件：`{data_dir}/buddy/session-sync/<platform>/<uid>.resolutions.json`。
pub(crate) fn conflict_resolutions_file(platform_label: &str, target_uid: &str) -> Result<PathBuf, String> {
    let dir = crate::commands::config::get_data_dir()
        .join("buddy")
        .join("session-sync")
        .join(platform_label);
    Ok(dir.join(format!("{}.resolutions.json", sanitize_file_component(target_uid)?)))
}

/// 从**指定文件**读结果（路径参数化，便于测试注入临时目录）。
fn load_resolutions_from(path: &Path) -> Vec<ConflictResolution> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    match serde_json::from_str::<ConflictResolutionFile>(&content) {
        Ok(file) if file.version == CONFLICT_RESOLUTIONS_VERSION => file.resolutions,
        Ok(_) => {
            eprintln!(
                "[Buddy SessionSync] 冲突处理结果文件版本不认识，忽略: {}",
                path.display()
            );
            Vec::new()
        }
        Err(error) => {
            eprintln!(
                "[Buddy SessionSync] 冲突处理结果解析失败，忽略: path={}, error={}",
                path.display(),
                error
            );
            Vec::new()
        }
    }
}

pub(crate) fn load_conflict_resolutions(platform_label: &str, target_uid: &str) -> Vec<ConflictResolution> {
    let Ok(path) = conflict_resolutions_file(platform_label, target_uid) else {
        return Vec::new();
    };
    load_resolutions_from(&path)
}

/// 追加（按 id 覆盖）处理结果：同一会话重复裁决时以最近一次为准。
pub(crate) fn append_conflict_resolutions(
    platform_label: &str,
    target_uid: &str,
    entries: &[ConflictResolution],
) {
    if entries.is_empty() {
        return;
    }
    let Ok(path) = conflict_resolutions_file(platform_label, target_uid) else {
        return;
    };
    append_conflict_resolutions_at(&path, entries);
}

/// 追加核心（路径参数化，便于测试）。
fn append_conflict_resolutions_at(path: &Path, entries: &[ConflictResolution]) {
    let mut all = load_resolutions_from(path);
    for entry in entries {
        all.retain(|r| r.id != entry.id);
        all.push(entry.clone());
    }
    // 只留最近 N 条（按处理时间，旧的先丢）
    if all.len() > MAX_RESOLUTIONS {
        all.sort_by(|a, b| b.at_ms.cmp(&a.at_ms));
        all.truncate(MAX_RESOLUTIONS);
        all.sort_by(|a, b| a.at_ms.cmp(&b.at_ms));
    }
    let payload = ConflictResolutionFile {
        version: CONFLICT_RESOLUTIONS_VERSION,
        resolutions: all,
    };
    let Ok(serialized) = serde_json::to_string_pretty(&payload) else {
        return;
    };
    if let Err(error) = super::store::write_atomic(&path.to_path_buf(), &serialized) {
        eprintln!(
            "[Buddy SessionSync] 冲突处理结果写入失败: path={}, error={}",
            path.display(),
            error
        );
    }
}

/// 当前时间（epoch 毫秒）。
pub(crate) fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
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

    // ─── 冲突处理结果：会话明细要靠它显示「已处理」而不是初始状态 ───

    fn resolution(id: &str, action: &str, at: i64) -> ConflictResolution {
        ConflictResolution {
            id: id.to_string(),
            action: action.to_string(),
            at_ms: at,
            message: format!("已处理 {}", id),
        }
    }

    #[test]
    fn resolutions_roundtrip_and_latest_wins() {
        let dir = make_temp("resolutions");
        let path = dir.join("uid.resolutions.json");

        append_conflict_resolutions_at(&path, &[resolution("a", "merge", 100), resolution("b", "keep", 110)]);
        // 同一会话再次裁决 → 以最近一次为准，不重复
        append_conflict_resolutions_at(&path, &[resolution("a", "overwrite", 200)]);

        let all = load_resolutions_from(&path);
        assert_eq!(all.len(), 2);
        let a = all.iter().find(|r| r.id == "a").unwrap();
        assert_eq!(a.action, "overwrite");
        assert_eq!(a.at_ms, 200);
        assert!(all.iter().any(|r| r.id == "b" && r.action == "keep"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolutions_are_capped_to_the_newest() {
        let dir = make_temp("resolutions-cap");
        let path = dir.join("uid.resolutions.json");
        let mut entries = Vec::new();
        for index in 0..(MAX_RESOLUTIONS + 25) {
            entries.push(resolution(&format!("id-{}", index), "merge", index as i64));
        }
        append_conflict_resolutions_at(&path, &entries);
        let all = load_resolutions_from(&path);
        assert_eq!(all.len(), MAX_RESOLUTIONS);
        // 丢的是最旧的，保留到最新的
        assert!(all.iter().all(|r| r.id != "id-0"));
        assert!(all.iter().any(|r| r.id == format!("id-{}", MAX_RESOLUTIONS + 24)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolutions_file_rejects_unsafe_uid() {
        assert!(conflict_resolutions_file("codebuddy-cn", "../evil").is_err());
        assert!(conflict_resolutions_file("codebuddy-cn", "a/b").is_err());
        assert!(conflict_resolutions_file("codebuddy-cn", "384c6dd0-c1bc-4ae2-a0d0-f70350c62f7b").is_ok());
    }

    #[test]
    fn baseline_file_rejects_unsafe_uid() {
        assert!(baseline_file("workbuddy", "../evil").is_err());
        assert!(baseline_file("workbuddy", "a/b").is_err());
        assert!(baseline_file("workbuddy", "384c6dd0-c1bc-4ae2-a0d0-f70350c62f7b").is_ok());
    }

    #[test]
    fn pending_conflicts_file_rejects_unsafe_uid() {
        assert!(pending_conflicts_file("codebuddy-cn", "../evil").is_err());
        assert!(pending_conflicts_file("codebuddy-cn", "a\\b").is_err());
        assert!(
            pending_conflicts_file("codebuddy-cn", "384c6dd0-c1bc-4ae2-a0d0-f70350c62f7b").is_ok()
        );
    }

    #[test]
    fn pending_conflict_serializes_camel_case_for_frontend() {
        let conflict = PendingConflict {
            id: "conv-1".to_string(),
            label: "工作区 A 的会话".to_string(),
            workspace: "ws-hash".to_string(),
            workspace_path: Some("e:\\pro\\my\\any-version".to_string()),
            ide: "CodeBuddy CN".to_string(),
            source_uid: "uid-a".to_string(),
            target_uid: "uid-b".to_string(),
            source_stamp: 1_700_000_000_000,
            target_stamp: 1_700_000_500_000,
        };
        let value = serde_json::to_value(&conflict).unwrap();
        // 前端类型按 camelCase 读，字段名错了界面会静默显示 undefined
        assert_eq!(value["sourceUid"], "uid-a");
        assert_eq!(value["targetUid"], "uid-b");
        assert_eq!(value["sourceStamp"], 1_700_000_000_000i64);
        assert_eq!(value["workspace"], "ws-hash");
        assert_eq!(value["workspacePath"], "e:\\pro\\my\\any-version");
        // 落盘走同一结构，必须能无损读回
        let back: PendingConflict = serde_json::from_value(value).unwrap();
        assert_eq!(back.id, "conv-1");
        assert_eq!(back.target_stamp, 1_700_000_500_000);
        // 旧版本落盘的文件没有该字段：serde(default) 兜底，读回为 None 而不是报错
        let legacy = serde_json::json!({
            "id": "conv-2", "label": "l", "workspace": "ws", "ide": "ide",
            "sourceUid": "a", "targetUid": "b",
            "sourceStamp": 1i64, "targetStamp": 2i64
        });
        let legacy: PendingConflict = serde_json::from_value(legacy).unwrap();
        assert!(legacy.workspace_path.is_none());
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
