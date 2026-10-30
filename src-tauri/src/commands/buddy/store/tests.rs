use super::*;

/// 进程唯一测试根 + 全局串行锁：写入型测试共享全局 TEST_DATA_ROOT，
/// 并行会互相覆盖，必须串行执行。
struct TestGuard(std::sync::MutexGuard<'static, ()>);

fn test_root() -> (std::path::PathBuf, TestGuard) {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    // 恢复可能的毒锁，避免单个测试失败连锁拖垮后续测试
    let guard = TestGuard(LOCK.lock().unwrap_or_else(|e| e.into_inner()));
    static INIT: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    let dir = INIT.get_or_init(|| {
        let d = std::env::temp_dir().join(format!("kira_buddy_store_test_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        set_test_data_root(d.clone());
        d
    });
    (dir.clone(), guard)
}

fn test_platform() -> BuddyPlatform {
    BuddyPlatform::Workbuddy
}

fn sample_account(id: &str, email: &str) -> BuddyAccount {
    BuddyAccount {
        id: id.to_string(),
        platform: "workbuddy".to_string(),
        email: email.to_string(),
        uid: Some(format!("uid-{}", id)),
        nickname: Some(format!("nick-{}", id)),
        enterprise_id: None,
        enterprise_name: None,
        tags: None,
        access_token: format!("token-{}", id),
        refresh_token: None,
        token_type: Some("Bearer".to_string()),
        expires_at: None,
        domain: None,
        plan_type: None,
        dosage_notify_code: None,
        dosage_notify_zh: None,
        dosage_notify_en: None,
        payment_type: None,
        quota_raw: None,
        usage_raw: None,
        profile_raw: None,
        status: None,
        status_reason: None,
        quota_query_last_error: None,
        quota_query_last_error_at: None,
        usage_updated_at: None,
        last_checkin_time: None,
        checkin_streak: 0,
        checkin_rewards: None,
        auth_raw: None,
        expiry_times: Default::default(),
        created_at: 0,
        last_used: 0,
    }
}

#[test]
fn upsert_and_list_roundtrip() {
    let (_root, _guard) = test_root();
    let account = sample_account("acc1", "a@test.com");
    let saved = upsert_account(test_platform(), account).unwrap();
    assert_eq!(saved.id, "acc1");

    let list = list_accounts(test_platform());
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].email, "a@test.com");

    delete_account(test_platform(), "acc1").unwrap();
    assert!(list_accounts(test_platform()).is_empty());
}

#[test]
fn upsert_dedupes_by_email() {
    let (_root, _guard) = test_root();
    upsert_account(test_platform(), sample_account("acc1", "same@test.com")).unwrap();
    let mut second = sample_account("acc2", "same@test.com");
    second.access_token = "token-acc2".to_string();
    let saved = upsert_account(test_platform(), second).unwrap();
    // 相同 email 去重 → 保留原 id
    assert_eq!(saved.id, "acc1");
    assert_eq!(saved.access_token, "token-acc2");
    assert_eq!(list_accounts(test_platform()).len(), 1);
    delete_account(test_platform(), "acc1").unwrap();
}

#[test]
fn normalize_rejects_path_traversal() {
    let (_root, _guard) = test_root();
    assert!(normalize_account_id("../evil").is_err());
    assert!(normalize_account_id("a/b").is_err());
    assert!(normalize_account_id("").is_err());
    assert_eq!(normalize_account_id("acc-1_x.y").unwrap(), "acc-1_x.y");
}

/// 新增账号（OAuth / 粘贴 token）传进来的 id 是空的：
/// upsert 必须自己按 uid/email 派生，否则会报「账号 ID 不能为空」。
#[test]
fn upsert_derives_id_when_missing() {
    let (_root, _guard) = test_root();
    let mut acc = sample_account("", "new-add@test.com");
    acc.id.clear();
    acc.uid = Some("New-Uid-1".into()); // 大小写不影响派生（统一小写）

    let saved = upsert_account(test_platform(), acc).unwrap();
    assert_eq!(
        saved.id,
        format!("workbuddy_{:x}", md5::compute(b"new-uid-1"))
    );

    // 同一 uid（大小写不同）再 upsert：复用同一个 id，不新增第二条
    let mut again = sample_account("", "another@test.com");
    again.id.clear();
    again.uid = Some("new-uid-1".into());
    let saved2 = upsert_account(test_platform(), again).unwrap();
    assert_eq!(saved2.id, saved.id);
    let hits = load_index(test_platform())
        .accounts
        .into_iter()
        .filter(|a| a.id == saved.id)
        .count();
    assert_eq!(hits, 1);
}

/// 派生规则：uid 优先，其次 email（小写），都没有才退化成 `{prefix}_user`。
#[test]
fn derive_account_id_prefers_uid_then_email() {
    let p = test_platform();
    assert_eq!(
        derive_account_id(p, Some("U-1"), "x@test.com"),
        format!("workbuddy_{:x}", md5::compute(b"u-1"))
    );
    assert_eq!(
        derive_account_id(p, None, "X@Test.com"),
        format!("workbuddy_{:x}", md5::compute(b"x@test.com"))
    );
    let fallback = format!("workbuddy_{:x}", md5::compute(b"workbuddy_user"));
    assert_eq!(derive_account_id(p, None, ""), fallback);
    // "unknown" 是 email 缺失时的占位值，不能拿来当 seed
    assert_eq!(derive_account_id(p, None, "unknown"), fallback);
}

#[test]
fn export_import_roundtrip() {
    let (_root, _guard) = test_root();
    upsert_account(test_platform(), sample_account("exp1", "e@test.com")).unwrap();
    let json = export_accounts(test_platform(), &["exp1".to_string()]).unwrap();
    delete_account(test_platform(), "exp1").unwrap();
    let imported = import_accounts(test_platform(), &json).unwrap();
    assert_eq!(imported.len(), 1);
    assert_eq!(imported[0].email, "e@test.com");
    delete_account(test_platform(), "exp1").unwrap();
}

#[test]
fn import_strips_utf8_bom() {
    let (_root, _guard) = test_root();
    // 有效 JSON 前加 UTF-8 BOM（Windows 工具导出常见）
    let json = format!(
        "\u{feff}{}",
        serde_json::to_string(&vec![sample_account("bom1", "bom@test.com")]).unwrap()
    );
    let imported = import_accounts(test_platform(), &json).unwrap();
    assert_eq!(imported.len(), 1);
    assert_eq!(imported[0].email, "bom@test.com");
    delete_account(test_platform(), "bom1").unwrap();
}

#[test]
fn import_decodes_utf16_bytes() {
    let (_root, _guard) = test_root();
    // 以 UTF-16 LE 字节文件内容传入（模拟其它工具导出的 UTF-16 文件），
    // 走真实链路：read_text_file → decode_text_bytes → import_accounts
    let json_str = serde_json::to_string(&vec![sample_account("u16", "u16@test.com")]).unwrap();
    let mut bytes: Vec<u8> = vec![0xFF, 0xFE]; // UTF-16 LE BOM
    for u in json_str.encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    let decoded = crate::commands::file_io::decode_text_bytes(&bytes);
    assert!(!decoded.contains('\u{0}'));
    let imported = import_accounts(test_platform(), &decoded).unwrap();
    assert_eq!(imported.len(), 1);
    assert_eq!(imported[0].email, "u16@test.com");
    delete_account(test_platform(), "u16").unwrap();
}

#[test]
fn import_accepts_wrapped_object_and_single_account() {
    let (_root, _guard) = test_root();
    // {"accounts": [...]} 包装
    let wrapped = serde_json::json!({
        "accounts": [serde_json::to_value(sample_account("w1", "w@test.com")).unwrap()]
    });
    let imported = import_accounts(test_platform(), &wrapped.to_string()).unwrap();
    assert_eq!(imported.len(), 1);
    assert_eq!(imported[0].email, "w@test.com");
    delete_account(test_platform(), "w1").unwrap();

    // 单个账号对象（非数组）
    let single = serde_json::to_value(sample_account("s1", "s@test.com")).unwrap();
    let imported = import_accounts(test_platform(), &single.to_string()).unwrap();
    assert_eq!(imported.len(), 1);
    assert_eq!(imported[0].email, "s@test.com");
    delete_account(test_platform(), "s1").unwrap();
}

#[test]
fn import_defaults_missing_platform_and_snake_case_keys() {
    let (_root, _guard) = test_root();
    // 手写 snake_case 且无 platform 字段
    let json = r#"[
            {
                "id": "snake1",
                "email": "snake@test.com",
                "uid": "uid-snake1",
                "access_token": "tok-1",
                "refresh_token": "ref-1"
            }
        ]"#;
    let imported = import_accounts(test_platform(), json).unwrap();
    assert_eq!(imported.len(), 1);
    assert_eq!(imported[0].email, "snake@test.com");
    assert_eq!(imported[0].access_token, "tok-1");
    assert_eq!(imported[0].refresh_token.as_deref(), Some("ref-1"));
    // platform 自动补为当前目标平台
    assert_eq!(imported[0].platform, "workbuddy");
    delete_account(test_platform(), "snake1").unwrap();
}

#[test]
fn current_account_persist_and_clear() {
    let (_root, _guard) = test_root();
    let platform = test_platform();
    assert_eq!(get_current_account_id(platform), None);

    set_current_account_id(platform, Some("acc-cur")).unwrap();
    assert_eq!(get_current_account_id(platform).as_deref(), Some("acc-cur"));

    // 写入其它平台互不影响
    set_current_account_id(BuddyPlatform::CodebuddyCn, Some("cn-acc")).unwrap();
    assert_eq!(get_current_account_id(platform).as_deref(), Some("acc-cur"));
    assert_eq!(
        get_current_account_id(BuddyPlatform::CodebuddyCn).as_deref(),
        Some("cn-acc")
    );

    // 清空
    set_current_account_id(platform, None).unwrap();
    assert_eq!(get_current_account_id(platform), None);
    set_current_account_id(BuddyPlatform::CodebuddyCn, None).unwrap();
    assert_eq!(get_current_account_id(BuddyPlatform::CodebuddyCn), None);
}

#[test]
fn delete_current_account_clears_state() {
    let (_root, _guard) = test_root();
    let platform = test_platform();
    upsert_account(platform, sample_account("cur1", "cur@test.com")).unwrap();
    set_current_account_id(platform, Some("cur1")).unwrap();
    assert_eq!(get_current_account_id(platform).as_deref(), Some("cur1"));

    delete_account(platform, "cur1").unwrap();
    assert_eq!(get_current_account_id(platform), None);
}

#[test]
fn import_generates_id_when_missing() {
    let (_root, _guard) = test_root();
    let json = r#"[
            {
                "email": "noid@test.com",
                "accessToken": "tok-2"
            }
        ]"#;
    let imported = import_accounts(test_platform(), json).unwrap();
    assert_eq!(imported.len(), 1);
    assert!(!imported[0].id.is_empty());
    assert_eq!(imported[0].email, "noid@test.com");
    delete_account(test_platform(), &imported[0].id).unwrap();
}

#[test]
fn sync_accounts_between_platforms_resets_checkin_and_dedupes() {
    let (_root, _guard) = test_root();
    let mut wb = sample_account("sync1", "sync@test.com");
    wb.checkin_streak = 7;
    wb.last_checkin_time = Some(123);
    wb.tags = Some(vec!["vip".to_string()]);
    upsert_account(test_platform(), wb).unwrap();

    // WB -> CN：复制成功，签数字段清零，id 按目标平台重新生成
    let n = sync_accounts(BuddyPlatform::Workbuddy, BuddyPlatform::CodebuddyCn, &[]).unwrap();
    assert_eq!(n, 1);
    let cn = list_accounts(BuddyPlatform::CodebuddyCn);
    assert_eq!(cn.len(), 1);
    assert_eq!(cn[0].email, "sync@test.com");
    assert_eq!(cn[0].platform, "codebuddy-cn");
    assert!(cn[0].id.starts_with("codebuddy_cn_"));
    assert_eq!(cn[0].checkin_streak, 0);
    assert_eq!(cn[0].last_checkin_time, None);
    assert_eq!(cn[0].access_token, "token-sync1");

    // 幂等：再次同步按 uid/email 去重，不新增
    let n2 = sync_accounts(BuddyPlatform::Workbuddy, BuddyPlatform::CodebuddyCn, &[]).unwrap();
    assert_eq!(n2, 1);
    assert_eq!(list_accounts(BuddyPlatform::CodebuddyCn).len(), 1);

    // 反向同步回 WB：uid 相同命中已有账号
    let n3 = sync_accounts(BuddyPlatform::CodebuddyCn, test_platform(), &[]).unwrap();
    assert_eq!(n3, 1);
    assert_eq!(list_accounts(test_platform()).len(), 1);

    // 自同步被拒绝
    assert!(sync_accounts(test_platform(), test_platform(), &[]).is_err());

    delete_account(test_platform(), "sync1").unwrap();
    for account in list_accounts(BuddyPlatform::CodebuddyCn) {
        delete_account(BuddyPlatform::CodebuddyCn, &account.id).unwrap();
    }
    assert!(list_accounts(BuddyPlatform::CodebuddyCn).is_empty());
}

fn times(entries: &[(&str, i64)]) -> std::collections::HashMap<String, i64> {
    entries.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

#[test]
fn set_expiry_times_filters_and_roundtrips() {
    let (_root, _guard) = test_root();
    upsert_account(test_platform(), sample_account("tl1", "tl@test.com")).unwrap();

    let updated = set_expiry_times(
        test_platform(),
        "tl1",
        times(&[("c1", 1000), ("c2", 2000), ("c3", -5), ("", 9)]),
    )
    .unwrap();
    // 过滤非正与空 key
    assert_eq!(updated.expiry_times.get("c1"), Some(&1000));
    assert_eq!(updated.expiry_times.get("c2"), Some(&2000));
    assert!(!updated.expiry_times.contains_key("c3"));
    assert!(!updated.expiry_times.contains_key(""));
    // 回读持久化生效
    let reread = load_account(test_platform(), "tl1").unwrap();
    assert_eq!(reread.expiry_times.get("c2"), Some(&2000));

    // 清空
    let cleared = set_expiry_times(test_platform(), "tl1", times(&[])).unwrap();
    assert!(cleared.expiry_times.is_empty());

    // 删除列会清理所有账号该键
    set_expiry_times(test_platform(), "tl1", times(&[("c1", 5), ("c2", 6)])).unwrap();
    prune_expiry_column(test_platform(), "c1");
    let after = load_account(test_platform(), "tl1").unwrap();
    assert!(!after.expiry_times.contains_key("c1"));
    assert_eq!(after.expiry_times.get("c2"), Some(&6));

    // 不存在的账号报错
    assert!(set_expiry_times(test_platform(), "no-such", times(&[("c1", 1)])).is_err());
    delete_account(test_platform(), "tl1").unwrap();
}

#[test]
fn set_expiry_times_shared_syncs_to_twin_by_email() {
    let (_root, _guard) = test_root();
    upsert_account(
        BuddyPlatform::CodebuddyCn,
        sample_account("cn_twin", "twin@test.com"),
    )
    .unwrap();
    upsert_account(
        BuddyPlatform::Workbuddy,
        sample_account("wb_twin", "twin@test.com"),
    )
    .unwrap();
    upsert_account(
        BuddyPlatform::Workbuddy,
        sample_account("wb_other", "other@test.com"),
    )
    .unwrap();

    // CN 侧设置 → WB 同邮箱账号自动同步；不同邮箱不受影响
    set_expiry_times_shared(
        BuddyPlatform::CodebuddyCn,
        "cn_twin",
        times(&[("c1", 1000)]),
    )
    .unwrap();
    let wb = load_account(BuddyPlatform::Workbuddy, "wb_twin").unwrap();
    assert_eq!(wb.expiry_times.get("c1"), Some(&1000));
    let other = load_account(BuddyPlatform::Workbuddy, "wb_other").unwrap();
    assert!(other.expiry_times.is_empty());

    // 反向：WB 侧清空 → CN 侧同步清空
    set_expiry_times_shared(BuddyPlatform::Workbuddy, "wb_twin", times(&[])).unwrap();
    let cn = load_account(BuddyPlatform::CodebuddyCn, "cn_twin").unwrap();
    assert!(cn.expiry_times.is_empty());

    delete_account(BuddyPlatform::CodebuddyCn, "cn_twin").unwrap();
    delete_account(BuddyPlatform::Workbuddy, "wb_twin").unwrap();
    delete_account(BuddyPlatform::Workbuddy, "wb_other").unwrap();
}

#[test]
fn backfill_expiry_times_fills_missing_only() {
    let (_root, _guard) = test_root();
    // CN 有 c1/c2，WB 同邮箱已有 c2（不同值）→ 回填只补 c1，不覆盖 c2
    let mut cn = sample_account("cn_bf", "bf@test.com");
    cn.expiry_times = times(&[("c1", 1000), ("c2", 2000)]);
    upsert_account(BuddyPlatform::CodebuddyCn, cn).unwrap();
    let mut wb = sample_account("wb_bf", "bf@test.com");
    wb.expiry_times = times(&[("c2", 9999)]);
    upsert_account(BuddyPlatform::Workbuddy, wb).unwrap();

    backfill_expiry_times();

    let wb_after = load_account(BuddyPlatform::Workbuddy, "wb_bf").unwrap();
    assert_eq!(wb_after.expiry_times.get("c1"), Some(&1000));
    assert_eq!(wb_after.expiry_times.get("c2"), Some(&9999));

    // 幂等：再跑一次值不变
    backfill_expiry_times();
    let wb_again = load_account(BuddyPlatform::Workbuddy, "wb_bf").unwrap();
    assert_eq!(wb_again.expiry_times.get("c2"), Some(&9999));

    delete_account(BuddyPlatform::CodebuddyCn, "cn_bf").unwrap();
    delete_account(BuddyPlatform::Workbuddy, "wb_bf").unwrap();
}

// ─── 主账号与排序模式 ───

#[test]
fn account_view_roundtrip_and_validation() {
    let (_root, _guard) = test_root();
    let platform = BuddyPlatform::Workbuddy;
    let account = sample_account("view_a", "view-a@example.com");
    upsert_account(platform, account.clone()).unwrap();

    // 默认：无主账号 + 最近使用排序
    let view = load_view(platform);
    assert_eq!(view.primary_account_id, None);
    assert_eq!(view.order_mode, ORDER_MODE_LAST_USED);

    // 设置主账号与排序模式后能读回
    save_view(
        platform,
        &BuddyAccountView {
            primary_account_id: Some(account.id.clone()),
            order_mode: ORDER_MODE_EXPIRY.to_string(),
        },
    )
    .unwrap();
    let view = load_view(platform);
    assert_eq!(
        view.primary_account_id.as_deref(),
        Some(account.id.as_str())
    );
    assert_eq!(view.order_mode, ORDER_MODE_EXPIRY);

    // 非法排序模式被拒绝（写入前校验）
    assert!(validate_order_mode("whatever").is_err());
    assert!(save_view_with_mode(platform, Some(account.id.clone()), "whatever").is_err());

    // 账号已删除 → 读时主账号自动清空
    delete_account(platform, &account.id).unwrap();
    assert_eq!(load_view(platform).primary_account_id, None);
}

#[test]
fn set_primary_rejects_unknown_account() {
    let (_root, _guard) = test_root();
    let platform = BuddyPlatform::CodebuddyCn;
    assert!(save_view_with_mode(
        platform,
        Some("not_exists".to_string()),
        ORDER_MODE_LAST_USED
    )
    .is_err());
}
