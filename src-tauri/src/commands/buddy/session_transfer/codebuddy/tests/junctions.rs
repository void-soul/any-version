use super::*;

/// 2026-10-20 真机：junction 的 `is_symlink()` 不可靠，必须读属性位。
/// 共享判定依赖它，判错就会把链接物化成真实目录（磁盘直接翻倍）。
#[test]
fn reparse_point_detects_junction_and_shared_target() {
    let root = std::env::temp_dir().join(format!("buddy-cn-reparse-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let target = root.join("target");
    std::fs::create_dir_all(&target).unwrap();
    let link = root.join("link");
    let status = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&link)
        .arg(&target)
        .status();
    let supported = status.map(|s| s.success()).unwrap_or(false);
    if !supported {
        // 非 Windows / 无 mklink：跳过（不影响其他平台）
        let _ = std::fs::remove_dir_all(&root);
        return;
    }

    assert!(
        is_reparse_point(&link),
        "junction 必须被识别为 reparse point"
    );
    assert!(!is_reparse_point(&target), "普通目录不能误判");

    let extension = root.join("extension");
    let uid = "uid-shared";
    let account_root = extension.join(uid).join("CodeBuddyIDE").join(uid);
    std::fs::create_dir_all(&account_root).unwrap();
    let history = account_root.join("history");
    let ok = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&history)
        .arg(&target)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(ok, "创建 history junction 失败");

    assert!(
        target_sessions_shared(&extension, uid),
        "共享链接必须被识别"
    );
    assert!(
        !target_sessions_shared(&extension, "uid-plain"),
        "不存在的账号不能误判"
    );

    // 只删链接与临时根，确认目标目录本身没被删
    let _ = std::fs::remove_dir(&history);
    let _ = std::fs::remove_dir(&link);
    assert!(target.is_dir());
    let _ = std::fs::remove_dir_all(&root);
}

/// 切号前的接管（`ensure_shared_target` → `adopt_account_into_shared`）：
/// 客户端给新账号建的真实目录应被并进共享根，并整体换成 6 条 junction。
/// 这条链路此前没有调用点，是「每个账号又留一份副本」的回归口。
#[test]
fn adopt_account_into_shared_replaces_real_dirs_with_junctions() {
    let root = make_temp();

    // 探针：无 mklink 的平台直接跳过（接管依赖 junction）
    if !mklink_supported(&root) {
        let _ = std::fs::remove_dir_all(&root);
        return;
    }

    let extension = root.join("extension");
    let shared_root = root.join("shared");
    let uid = "uid-adopt";
    let account_root = extension.join(uid).join("CodeBuddyIDE").join(uid);

    // 账号侧：一条共享根没有的独有会话 + 一条共享根已有的会话
    let own = account_root.join("history").join("ws1").join("own-id");
    std::fs::create_dir_all(&own).unwrap();
    std::fs::write(own.join("msg.json"), "account-copy").unwrap();
    let account_dup = account_root.join("history").join("ws1").join("shared-id");
    std::fs::create_dir_all(&account_dup).unwrap();
    std::fs::write(account_dup.join("msg.json"), "account-copy").unwrap();
    // 辅助目录同样要接管（check-point 里也是独有内容）
    let checkpoint = account_root.join("check-point").join("ws1").join("own-id");
    std::fs::create_dir_all(&checkpoint).unwrap();
    std::fs::write(checkpoint.join("state.json"), "cp").unwrap();

    // 共享侧：shared-id 是权威副本，内容不能被账号侧覆盖
    let shared_dup = shared_root.join("history").join("ws1").join("shared-id");
    std::fs::create_dir_all(&shared_dup).unwrap();
    std::fs::write(shared_dup.join("msg.json"), "shared-authority").unwrap();

    let adopted = adopt_account_into_shared(&account_root, &shared_root).unwrap();
    assert_eq!(adopted, 2, "history 与 check-point 各并入 1 条独有会话");

    for kind in SHARED_KINDS {
        assert!(
            is_reparse_point(&account_root.join(kind)),
            "{} 应被换成 junction",
            kind
        );
    }
    // 通过链接读到的就是共享根：独有会话已并入，权威副本未被账号侧覆盖
    assert!(shared_root.join("history/ws1/own-id/msg.json").is_file());
    assert!(shared_root
        .join("check-point/ws1/own-id/state.json")
        .is_file());
    assert_eq!(
        std::fs::read_to_string(account_root.join("history/ws1/shared-id/msg.json")).unwrap(),
        "shared-authority"
    );

    // 幂等：再接管一次不复制任何东西（共享根是权威副本）
    assert_eq!(
        adopt_account_into_shared(&account_root, &shared_root).unwrap(),
        0
    );

    // 收尾：先删链接本身，再删临时根（不让 remove_dir_all 碰到链接目标）
    for kind in SHARED_KINDS {
        let _ = std::fs::remove_dir(account_root.join(kind));
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// `ensure_shared_target_at`（切号时的接线点）：共享根为空时不得接管
/// （用户没启用共享 → 必须回落到原逐会话合并），就绪时才换成链接。
#[test]
fn ensure_shared_target_skips_empty_root_then_adopts_when_ready() {
    let root = make_temp();
    if !mklink_supported(&root) {
        let _ = std::fs::remove_dir_all(&root);
        return;
    }

    let extension = root.join("extension");
    let uid = "uid-ensure";
    let account_root = extension.join(uid).join("CodeBuddyIDE").join(uid);
    let real_session = account_root.join("history").join("ws1").join("s1");
    std::fs::create_dir_all(&real_session).unwrap();
    std::fs::write(real_session.join("msg.json"), "only-copy").unwrap();

    // 共享根为空（history 里一条都没有）：返回 false，账号目录原样保留
    let empty_root = root.join("empty-shared");
    std::fs::create_dir_all(empty_root.join("history")).unwrap();
    assert!(
        !ensure_shared_target_at(&extension, uid, &empty_root).unwrap(),
        "共享根为空时必须回落，不得接管"
    );
    assert!(!is_reparse_point(&account_root.join("history")));
    assert!(real_session.is_dir(), "未启用共享时绝不能动账号目录");

    // 共享根就绪：接管，history 变成指向共享根的链接，独有会话并入
    let shared_root = root.join("shared");
    std::fs::create_dir_all(shared_root.join("history").join("ws0").join("dummy")).unwrap();
    assert!(ensure_shared_target_at(&extension, uid, &shared_root).unwrap());
    assert!(is_reparse_point(&account_root.join("history")));
    assert!(history_links_to(&account_root, &shared_root));
    assert!(
        shared_root.join("history/ws1/s1/msg.json").is_file(),
        "账号独有会话应并入共享根"
    );

    for kind in SHARED_KINDS {
        let _ = std::fs::remove_dir(account_root.join(kind));
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// 半接管账号（history 已链接、check-point 是真实目录）不能被判为「已就绪」：
/// 否则切号短路会跳过合并，其余 3 类就永久留在账号目录里各存一份。
#[test]
fn ensure_shared_target_repairs_partially_linked_account() {
    let root = make_temp();
    if !mklink_supported(&root) {
        let _ = std::fs::remove_dir_all(&root);
        return;
    }

    let extension = root.join("extension");
    let shared_root = root.join("shared");
    std::fs::create_dir_all(shared_root.join("history").join("ws0").join("dummy")).unwrap();
    let uid = "uid-partial";
    let account_root = extension.join(uid).join("CodeBuddyIDE").join(uid);

    // 先完整接管一次，拿到 4 条链接
    assert!(ensure_shared_target_at(&extension, uid, &shared_root).unwrap());

    // 再把 check-point 换回真实目录，模拟「只链了 history」的半接管状态
    let check_point = account_root.join("check-point");
    std::fs::remove_dir(&check_point).unwrap();
    let own = check_point.join("ws1").join("cp-1");
    std::fs::create_dir_all(&own).unwrap();
    std::fs::write(own.join("state.json"), "partial").unwrap();
    assert!(!all_shared_kinds_linked(&account_root, &shared_root));

    // 再次接管：4 类必须全变成链接，独有 check-point 内容并入共享根
    assert!(ensure_shared_target_at(&extension, uid, &shared_root).unwrap());
    for kind in SHARED_KINDS {
        assert!(
            kind_links_to(&account_root, &shared_root, kind),
            "{} 应重新接入共享根",
            kind
        );
    }
    assert!(shared_root
        .join("check-point/ws1/cp-1/state.json")
        .is_file());
    assert!(
        !account_root
            .join(format!("check-point{}", TAKEOVER_BAK_SUFFIX))
            .exists(),
        "接管后不能保留真实目录副本"
    );

    for kind in SHARED_KINDS {
        let _ = std::fs::remove_dir(account_root.join(kind));
    }
    let _ = std::fs::remove_dir_all(&root);
}
