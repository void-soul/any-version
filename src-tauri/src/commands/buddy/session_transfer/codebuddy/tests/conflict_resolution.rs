use super::*;

#[test]
fn conflicts_stem_strips_full_double_extension() {
    // 回归：Path::file_stem 只剥最后一个扩展名，"x.conflicts.json" 的 stem 是
    // "x.conflicts" 而不是 "x" —— 按它拼回文件名永远读不到，列表恒为空
    // （Q-0202 返工时真实踩坑：后端落盘 6 条、前端列表 0 条）。
    assert_eq!(
        conflicts_stem("4dc9bdfa-7cb0-4961-9b2f-bfcbde7e78b2.conflicts.json"),
        Some("4dc9bdfa-7cb0-4961-9b2f-bfcbde7e78b2")
    );
    assert_eq!(conflicts_stem("whatever.json"), None);
    assert_eq!(conflicts_stem("conflicts.json"), None);
}

#[test]
fn conversation_label_falls_back_to_workspace_not_id() {
    // 无标题的会话以前显示裸 id（一串哈希），用户认不出是哪个会话；
    // 现在回落到所属工作区目录，只有目录也没有时才用 id
    let untitled = serde_json::json!({"id": "abc123"});
    assert_eq!(
        conversation_label(&untitled, "abc123", "my-project"),
        "my-project"
    );
    let titled = serde_json::json!({"id": "abc123", "title": "重构登录"});
    assert_eq!(
        conversation_label(&titled, "abc123", "my-project"),
        "重构登录"
    );
    let blank_workspace = serde_json::json!({"id": "abc123"});
    assert_eq!(
        conversation_label(&blank_workspace, "abc123", "  "),
        "abc123"
    );
}

#[test]
fn resolve_conflict_overwrite_replaces_target_and_advances_baseline() {
    let dest = make_temp();
    let (data_root, conflict) = conflict_fixture(&dest, 1_700_000_000_000, 1_699_000_000_000);
    let mut baseline = std::collections::BTreeMap::new();
    let message = resolve_pending_conflict_at(
        &data_root,
        &conflict,
        ConflictAction::Overwrite,
        &dest.join("backup"),
        &mut baseline,
    )
    .unwrap();
    assert!(
        message.contains("覆盖"),
        "返回文案应说明做了覆盖: {message}"
    );

    // 目标会话正文被来源替换
    let target_conv = data_root
        .join("dst-uid")
        .join("VSCode")
        .join("dst-uid")
        .join("history")
        .join("ws")
        .join("conv-1");
    let body = std::fs::read_to_string(target_conv.join("messages.jsonl")).unwrap();
    assert_eq!(body, "source-body");
    // 目标 index.json 条目同步为来源的（时间戳变成来源侧）
    let index: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(target_conv.parent().unwrap().join("index.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        index["conversations"][0]["lastMessageAt"],
        1_700_000_000_000i64
    );
    // 原目标正文有备份可回滚
    let backup_body = std::fs::read_to_string(
        dest.join("backup")
            .join("conversations")
            .join("conv-1")
            .join("messages.jsonl"),
    )
    .unwrap();
    assert_eq!(backup_body, "target-body");
    // 基线推进到来源指纹：这轮来源改动视为已落地
    // （key 现在带 IDE + 工作区维度，避免跨工作区共用基线）
    assert_eq!(
        baseline
            .get(&conflict_baseline_key(&conflict))
            .map(String::as_str),
        Some("1700000000000")
    );
}

#[test]
fn resolve_conflict_keep_discards_source_and_resolves() {
    let dest = make_temp();
    let (data_root, conflict) = conflict_fixture(&dest, 1_700_000_000_000, 1_699_000_000_000);
    let mut baseline = std::collections::BTreeMap::new();
    let message = resolve_pending_conflict_at(
        &data_root,
        &conflict,
        ConflictAction::Keep,
        &dest.join("backup"),
        &mut baseline,
    )
    .unwrap();
    assert!(
        message.contains("保留"),
        "返回文案应说明保留了目标: {message}"
    );

    // 目标一字未动
    let body = std::fs::read_to_string(
        data_root
            .join("dst-uid")
            .join("VSCode")
            .join("dst-uid")
            .join("history")
            .join("ws")
            .join("conv-1")
            .join("messages.jsonl"),
    )
    .unwrap();
    assert_eq!(body, "target-body");
    // 基线推进到目标指纹：冲突了结，下次切换不再提醒
    assert_eq!(
        baseline
            .get(&conflict_baseline_key(&conflict))
            .map(String::as_str),
        Some("1699000000000")
    );
}

#[test]
fn resolve_conflict_merge_picks_the_newer_side() {
    // 来源较新 → 合并等价于覆盖
    let dest = make_temp();
    let (data_root, conflict) = conflict_fixture(&dest, 1_700_000_000_000, 1_699_000_000_000);
    let mut baseline = std::collections::BTreeMap::new();
    resolve_pending_conflict_at(
        &data_root,
        &conflict,
        ConflictAction::Merge,
        &dest.join("backup"),
        &mut baseline,
    )
    .unwrap();
    let body = std::fs::read_to_string(
        data_root
            .join("dst-uid")
            .join("VSCode")
            .join("dst-uid")
            .join("history")
            .join("ws")
            .join("conv-1")
            .join("messages.jsonl"),
    )
    .unwrap();
    assert_eq!(body, "source-body");

    // 目标较新 → 合并等价于保留
    let dest2 = make_temp();
    let (data_root2, conflict2) = conflict_fixture(&dest2, 1_690_000_000_000, 1_699_000_000_000);
    let mut baseline2 = std::collections::BTreeMap::new();
    resolve_pending_conflict_at(
        &data_root2,
        &conflict2,
        ConflictAction::Merge,
        &dest2.join("backup"),
        &mut baseline2,
    )
    .unwrap();
    let body2 = std::fs::read_to_string(
        data_root2
            .join("dst-uid")
            .join("VSCode")
            .join("dst-uid")
            .join("history")
            .join("ws")
            .join("conv-1")
            .join("messages.jsonl"),
    )
    .unwrap();
    assert_eq!(body2, "target-body");
    assert_eq!(
        baseline2
            .get(&conflict_baseline_key(&conflict2))
            .map(String::as_str),
        Some("1699000000000")
    );
}

/// 「忽略」：不改任何文件，只结束冲突（以前只有三选一，目录没了就永远卡住）。
#[test]
fn resolve_conflict_ignore_keeps_files_and_resolves() {
    let dest = make_temp();
    let (data_root, conflict) = conflict_fixture(&dest, 1_700_000_000_000, 1_699_000_000_000);
    let mut baseline = std::collections::BTreeMap::new();
    let message = resolve_pending_conflict_at(
        &data_root,
        &conflict,
        ConflictAction::Ignore,
        &dest.join("backup"),
        &mut baseline,
    )
    .unwrap();
    assert!(message.contains("忽略"), "返回文案应说明是忽略: {message}");
    // 目标一字未动（与 keep 的区别只是语义，不带"放弃来源改动"的判断）
    let body = std::fs::read_to_string(
        data_root
            .join("dst-uid")
            .join("VSCode")
            .join("dst-uid")
            .join("history")
            .join("ws")
            .join("conv-1")
            .join("messages.jsonl"),
    )
    .unwrap();
    assert_eq!(body, "target-body");
    assert_eq!(
        baseline
            .get(&conflict_baseline_key(&conflict))
            .map(String::as_str),
        Some("1699000000000")
    );
}

/// 两侧会话目录都被清理时，裁决必须能结束（以前这里直接报错 → 横幅永远亮）。
#[test]
fn resolve_conflict_clears_when_both_sides_are_gone() {
    let dest = make_temp();
    let (data_root, mut conflict) = conflict_fixture(&dest, 1_700_000_000_000, 1_699_000_000_000);
    conflict.workspace = "ws-gone".to_string();
    let mut baseline = std::collections::BTreeMap::new();
    for action in [
        ConflictAction::Merge,
        ConflictAction::Overwrite,
        ConflictAction::Keep,
        ConflictAction::Ignore,
    ] {
        let message = resolve_pending_conflict_at(
            &data_root,
            &conflict,
            action,
            &dest.join("backup"),
            &mut baseline,
        )
        .unwrap();
        assert!(
            message.contains("清除") || message.contains("忽略"),
            "{message}"
        );
    }
}
