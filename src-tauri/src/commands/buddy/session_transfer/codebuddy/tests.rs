use super::*;

fn make_temp() -> std::path::PathBuf {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "kira-codebuddy-transfer-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        id
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn no_progress() -> TransferProgress<'static> {
    TransferProgress {
        app: None,
        account_id: "",
        platform: BuddyPlatform::CodebuddyCn,
    }
}

/// 本机能否建 junction（非 Windows / 无 mklink 时，依赖链接的测试直接跳过）。
fn mklink_supported(root: &std::path::Path) -> bool {
    let target = root.join("probe-target");
    let link = root.join("probe-link");
    std::fs::create_dir_all(&target).unwrap();
    let ok = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&link)
        .arg(&target)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let _ = std::fs::remove_dir(&link);
    let _ = std::fs::remove_dir(&target);
    ok
}

/// 构造一个「同 id 会话在来源/目标两侧都有」的冲突现场。
/// 返回 (扩展数据根目录, 冲突记录)。
fn conflict_fixture(
    dest: &Path,
    source_ts: i64,
    target_ts: i64,
) -> (std::path::PathBuf, PendingConflict) {
    let data_root = dest.join("CodeBuddyExtension").join("Data");
    let source_history = data_root
        .join("src-uid")
        .join("VSCode")
        .join("src-uid")
        .join("history")
        .join("ws");
    let target_history = data_root
        .join("dst-uid")
        .join("VSCode")
        .join("dst-uid")
        .join("history")
        .join("ws");
    for (history, body, ts) in [
        (&source_history, "source-body", source_ts),
        (&target_history, "target-body", target_ts),
    ] {
        std::fs::create_dir_all(history.join("conv-1")).unwrap();
        std::fs::write(history.join("conv-1").join("messages.jsonl"), body).unwrap();
        std::fs::write(
            history.join("index.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "conversations": [{ "id": "conv-1", "lastMessageAt": ts }]
            }))
            .unwrap(),
        )
        .unwrap();
    }
    let conflict = PendingConflict {
        id: "conv-1".to_string(),
        label: "ws".to_string(),
        workspace: "ws".to_string(),
        workspace_path: None,
        ide: "VSCode".to_string(),
        source_uid: "src-uid".to_string(),
        target_uid: "dst-uid".to_string(),
        source_stamp: source_ts,
        target_stamp: target_ts,
        source_fingerprint: source_ts.to_string(),
        target_fingerprint: target_ts.to_string(),
    };
    (data_root, conflict)
}

mod conflict_resolution;

mod workspace_movement;

mod database;

mod merging;

mod junctions;
