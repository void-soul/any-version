use super::*;

#[test]
fn merges_workspace_history_without_dropping_target_conversations() {
    let dest = make_temp();
    let data_root = dest.join("CodeBuddyExtension").join("Data");
    let source_account = data_root.join("source").join("VSCode").join("source");
    let target_account = data_root.join("target").join("VSCode").join("target");
    let source_history = source_account.join("history").join("workspace");
    let target_history = target_account.join("history").join("workspace");

    for (history, id, ts) in [
        (&source_history, "source-only", "2026-07-02T00:00:00Z"),
        (&target_history, "target-only", "2026-07-01T00:00:00Z"),
    ] {
        std::fs::create_dir_all(history.join(id).join("messages")).unwrap();
        std::fs::write(history.join(id).join("index.json"), br#"{"messages":[]}"#).unwrap();
        std::fs::write(
            history.join("index.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "conversations": [{
                    "id": id,
                    "createdAt": ts,
                    "lastMessageAt": ts
                }],
                "current": id
            }))
            .unwrap(),
        )
        .unwrap();
    }
    std::fs::create_dir_all(
        source_account
            .join("check-point")
            .join("workspace")
            .join("source-only"),
    )
    .unwrap();

    let backup_root = dest.join("backup");
    let mut tracker = SyncTracker::new(std::collections::BTreeMap::new());
    let scanned = sync_history_between_accounts(
        &data_root,
        "source",
        "target",
        &backup_root,
        no_progress(),
        &mut tracker,
        &mut Vec::new(),
        ConflictPolicy::Resolvable,
    )
    .unwrap();
    assert_eq!(scanned, 1);
    let (summary, _) = tracker.finish();
    assert_eq!(summary.total, 1);
    assert_eq!(summary.copied, 1);
    assert_eq!(summary.details[0].status, SessionSyncStatus::Copied);
    // 来源工作区在目标不存在 → 整体复制：conversation 目录本体必须存在
    assert!(target_history.join("source-only").is_dir());
    assert!(target_history
        .join("source-only")
        .join("index.json")
        .is_file());
    assert!(target_history.join("target-only").is_dir());
    // 辅助目录（check-point）也应被复制
    assert!(target_account
        .join("check-point")
        .join("workspace")
        .join("source-only")
        .is_dir());

    let merged: serde_json::Value =
        serde_json::from_slice(&std::fs::read(target_history.join("index.json")).unwrap()).unwrap();
    assert_eq!(
        merged
            .get("conversations")
            .and_then(|v| v.as_array())
            .map(Vec::len),
        Some(2)
    );
}

#[test]
fn copies_entire_workspace_dir_when_target_missing() {
    // 回归：目标工作区不存在时，必须复制会话目录本体（此前只复制了 index.json 导致合并失败）
    let dest = make_temp();
    let data_root = dest.join("CodeBuddyExtension").join("Data");
    let source_account = data_root.join("source").join("VSCode").join("source");
    let source_history = source_account.join("history").join("workspace");
    std::fs::create_dir_all(source_history.join("conv1").join("messages")).unwrap();
    std::fs::write(
        source_history.join("conv1").join("messages").join("0.json"),
        "[]",
    )
    .unwrap();
    std::fs::write(
        source_history.join("index.json"),
        serde_json::to_vec(&serde_json::json!({
            "conversations": [{"id": "conv1", "lastMessageAt": 123}],
            "current": "conv1"
        }))
        .unwrap(),
    )
    .unwrap();

    let mut tracker = SyncTracker::new(std::collections::BTreeMap::new());
    let scanned = sync_history_between_accounts(
        &data_root,
        "source",
        "target",
        &dest.join("backup"),
        no_progress(),
        &mut tracker,
        &mut Vec::new(),
        ConflictPolicy::Resolvable,
    )
    .unwrap();
    assert_eq!(scanned, 1);
    let (summary, _) = tracker.finish();
    assert_eq!(summary.copied, 1);
    let target_conv = data_root
        .join("target")
        .join("VSCode")
        .join("target")
        .join("history")
        .join("workspace")
        .join("conv1");
    assert!(target_conv.is_dir());
    assert!(target_conv.join("messages").join("0.json").is_file());
}

/// 参考 WorkDaddy §17 的三分支判定：无变化跳过、两侧都改则冲突。
#[test]
fn decide_sync_skips_unchanged_and_flags_conflicts() {
    // 首次同步（无基线）→ 正常合并
    assert_eq!(decide_sync(None, "5", "3", true), SyncDecision::Apply);
    // 两侧都与基线一致 → 无变化
    assert_eq!(
        decide_sync(Some("3"), "3", "3", true),
        SyncDecision::Unchanged
    );
    // 只有一侧变化 → 正常合并
    assert_eq!(decide_sync(Some("3"), "5", "3", true), SyncDecision::Apply);
    assert_eq!(decide_sync(Some("3"), "3", "5", true), SyncDecision::Apply);
    // 两侧都比基线新 → 冲突，拒绝覆盖
    assert_eq!(
        decide_sync(Some("3"), "5", "6", true),
        SyncDecision::Conflict
    );
    // 目标会话文件缺失时必须补齐，不参与"无变化"判定
    assert_eq!(decide_sync(Some("3"), "3", "3", false), SyncDecision::Apply);
    // 任一侧时间戳未知 → 无法自证"没变"，按有变化处理
    // （以前缺失会回落成 0，两边都是 0 就被判成无变化而静默跳过）
    let unknown = |content: &str| format!("{}|{}", UNKNOWN_STAMP, content);
    assert_eq!(
        decide_sync(Some("3"), &unknown("-"), &unknown("-"), true),
        SyncDecision::Apply
    );
    assert_eq!(
        decide_sync(Some("3"), &unknown("-"), "3", true),
        SyncDecision::Apply
    );
    assert_eq!(
        decide_sync(Some("3"), "3", &unknown("2:40:9"), true),
        SyncDecision::Apply
    );
    assert_eq!(
        decide_sync(Some(&unknown("-")), &unknown("-"), &unknown("-"), true),
        SyncDecision::Apply
    );
}

/// 时间戳没变但目录内容变了（改标题 / 删消息 / 裁剪上下文）也必须判定为"有变化"。
#[test]
fn decide_sync_detects_content_change_without_timestamp_change() {
    let same_ts = "1700000000000";
    let before = format!("{}|{}", same_ts, "2:40:1700000000000");
    let after = format!("{}|{}", same_ts, "3:60:1700000000000");
    assert_eq!(
        decide_sync(Some(&before), &after, &before, true),
        SyncDecision::Apply,
        "内容变了就不再是两侧都一致"
    );
    assert_eq!(
        decide_sync(Some(&before), &after, &after, true),
        SyncDecision::Conflict,
        "两侧内容都偏离基线 → 冲突"
    );
}

/// 目录状态指纹：增 / 删 / 改任一文件都要改变；软链不参与。
#[test]
fn directory_state_fingerprint_tracks_add_remove_modify() {
    let dest = make_temp();
    let dir = dest.join("conv");
    std::fs::create_dir_all(dir.join("messages")).unwrap();
    std::fs::write(dir.join("messages").join("0.json"), "aaaa").unwrap();
    let first = crate::commands::buddy::session_sync::directory_state_fingerprint(&dir);
    assert!(first.is_some());

    // 改内容
    std::fs::write(dir.join("messages").join("0.json"), "bbbbbbbb").unwrap();
    let modified = crate::commands::buddy::session_sync::directory_state_fingerprint(&dir);
    assert_ne!(first, modified, "字节数变了指纹必须变");

    // 删文件（"取 mtime 最大者"那种做法发现不了删除）
    std::fs::remove_file(dir.join("messages").join("0.json")).unwrap();
    std::fs::write(dir.join("messages").join("1.json"), "").unwrap();
    let removed = crate::commands::buddy::session_sync::directory_state_fingerprint(&dir);
    assert_ne!(modified, removed, "删文件也必须被检测到");

    // 空目录 → None
    let empty = dest.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    assert!(crate::commands::buddy::session_sync::directory_state_fingerprint(&empty).is_none());

    let _ = std::fs::remove_dir_all(&dest);
}

/// 基线 key 必须区分 IDE / 工作区：同一个会话 id 出现在两个工作区不能共用基线。
#[test]
fn baseline_key_is_scoped_by_ide_and_workspace() {
    let a = crate::commands::buddy::session_sync::baseline_key(
        &workspace_namespace("VSCode", "ws-a"),
        "conv-1",
    );
    let b = crate::commands::buddy::session_sync::baseline_key(
        &workspace_namespace("VSCode", "ws-b"),
        "conv-1",
    );
    assert_ne!(a, b, "不同工作区的同一会话 id 不能共用基线");
    assert_ne!(
        a,
        crate::commands::buddy::session_sync::baseline_key(
            &workspace_namespace("Cursor", "ws-a"),
            "conv-1",
        )
    );
    // 命名空间为空时退化成会话 id 本身（WorkBuddy 那边的既有行为）
    assert_eq!(
        crate::commands::buddy::session_sync::baseline_key("", "conv-1"),
        "conv-1"
    );
}

/// 第二次合并同一批会话：内容没变 → 全部跳过，不再重复复制。
#[test]
fn second_merge_skips_unchanged_conversations() {
    let dest = make_temp();
    let data_root = dest.join("CodeBuddyExtension").join("Data");
    let source_account = data_root.join("source").join("VSCode").join("source");
    let target_account = data_root.join("target").join("VSCode").join("target");
    let source_history = source_account.join("history").join("workspace");
    let target_history = target_account.join("history").join("workspace");
    std::fs::create_dir_all(source_history.join("conv1").join("messages")).unwrap();
    std::fs::write(
        source_history.join("conv1").join("messages").join("0.json"),
        "[]",
    )
    .unwrap();
    let index = serde_json::to_vec(&serde_json::json!({
        "conversations": [{"id": "conv1", "title": "会话一", "lastMessageAt": 123}],
        "current": "conv1"
    }))
    .unwrap();
    std::fs::write(source_history.join("index.json"), &index).unwrap();

    let backup_root = dest.join("backup");
    let mut first = SyncTracker::new(std::collections::BTreeMap::new());
    sync_history_between_accounts(
        &data_root,
        "source",
        "target",
        &backup_root,
        no_progress(),
        &mut first,
        &mut Vec::new(),
        ConflictPolicy::Resolvable,
    )
    .unwrap();
    let (first_summary, baseline) = first.finish();
    assert_eq!(first_summary.copied, 1);

    // 目标工作区已经存在，来源内容未变：第二次应当什么都不做
    let mut second = SyncTracker::new(baseline);
    sync_history_between_accounts(
        &data_root,
        "source",
        "target",
        &backup_root,
        no_progress(),
        &mut second,
        &mut Vec::new(),
        ConflictPolicy::Resolvable,
    )
    .unwrap();
    let (second_summary, _) = second.finish();
    assert_eq!(second_summary.total, 1);
    assert_eq!(second_summary.skipped, 1);
    assert_eq!(second_summary.copied, 0);
    assert!(second_summary.unchanged);
}

/// 回归：WorkBuddy 不得报出「处理不了」的冲突。
///
/// 用户报过：切 WorkBuddy 账号提示「会话冲突」，但列表里是 codebuddy 会话，点处理冲突又说没有冲突。
/// 根因：共用的合并函数会把「两侧都改过」记成 conflict 进台账，而处理端对 WorkBuddy 恒返回空。
#[test]
fn workbuddy_policy_reports_no_conflict_for_both_changed() {
    // 造「两侧都改过」的现场，并给一份**两侧都偏离**的基线 ——
    // 没有基线时 decide_sync 走 Apply（首次同步），压根不会判成冲突，测试就空过了。
    let baseline_for = || {
        let mut b = std::collections::BTreeMap::new();
        b.insert(
            crate::commands::buddy::session_sync::baseline_key(
                &workspace_namespace("VSCode", "ws"),
                "conv-1",
            ),
            "stale-fingerprint".to_string(),
        );
        b
    };

    let dest = make_temp();
    let (data_root, _conflict) = conflict_fixture(&dest, 1_700_000_000_000, 1_699_000_000_000);
    let mut tracker = SyncTracker::new(baseline_for());
    sync_history_between_accounts(
        &data_root,
        "src-uid",
        "dst-uid",
        &dest.join("backup"),
        no_progress(),
        &mut tracker,
        &mut Vec::new(),
        ConflictPolicy::Suppressed,
    )
    .unwrap();
    let (summary, _) = tracker.finish();
    assert_eq!(
            summary.conflict, 0,
            "WorkBuddy 不得报冲突（处理端对它恒返回空），否则前端会出现「有冲突 → 列表里是 codebuddy 会话 → 点处理说没有冲突」"
        );
    assert_eq!(
        summary.skipped, 1,
        "两侧都改过时仍要保留目标版本（记为跳过）"
    );

    // 反证：同一现场 + 同一基线，CodeBuddy 的策略必须照常记冲突。
    // 少了这一句，上面那条会因为「场景压根没触发冲突」而空过。
    let dest2 = make_temp();
    let (data_root2, _) = conflict_fixture(&dest2, 1_700_000_000_000, 1_699_000_000_000);
    let mut tracker2 = SyncTracker::new(baseline_for());
    sync_history_between_accounts(
        &data_root2,
        "src-uid",
        "dst-uid",
        &dest2.join("backup"),
        no_progress(),
        &mut tracker2,
        &mut Vec::new(),
        ConflictPolicy::Resolvable,
    )
    .unwrap();
    let (summary2, _) = tracker2.finish();
    assert_eq!(summary2.conflict, 1, "CodeBuddy 侧必须照常记冲突");
}
