use super::*;

#[test]
fn validate_subst_value_accepts_safe_url_and_path() {
    assert!(validate_subst_value("https://mirror.example.com/npm/").is_ok());
    assert!(validate_subst_value(r"D:\any-version-caches\npm").is_ok());
    assert!(validate_subst_value("https://user:p%40ss@host:8080").is_ok());
    assert!(validate_subst_value("https://host:8080?token=abc").is_ok());
}

#[test]
fn truncate_utf8_keeps_char_boundaries() {
    // ASCII：正好切在 4 字节处
    assert_eq!(truncate_utf8("abcdefgh", 4), "abcd");
    // 短于上限：原样返回
    assert_eq!(truncate_utf8("abc", 10), "abc");
    // 上限为 0
    assert_eq!(truncate_utf8("中文", 0), "");
    // 中文（3 字节/字）：上限 4 落在第二个字中间 → 回退到 3
    let s = "中文测试";
    assert_eq!(truncate_utf8(s, 4), "中");
    assert_eq!(truncate_utf8(s, 3), "中");
    assert_eq!(truncate_utf8(s, 6), "中文");
    assert_eq!(truncate_utf8(s, s.len()), s);
}

#[test]
fn truncate_utf8_never_panics_on_any_prefix() {
    // 对任意字节上限都必须安全 —— 这正是 `&s[..n]` 会 panic 的场景
    let s = "上游返回错误 (401): 令牌无效 — 请检查 key";
    for n in 0..=s.len() + 4 {
        let out = truncate_utf8(s, n);
        assert!(s.starts_with(out), "截断结果必须是原串前缀: n={}", n);
        assert!(out.len() <= n.min(s.len()), "截断结果不能超上限: n={}", n);
    }
}

#[test]
fn validate_subst_value_rejects_injection_chars() {
    for bad in [
        "'; rm -rf /; '",
        "x\"&calc",
        "`id`",
        "$(whoami)",
        "a;b",
        "a|b",
        "a&b",
        "a<b",
        "a(b",
        "a\nb",
        "a\rb",
    ] {
        assert!(validate_subst_value(bad).is_err(), "应拒绝: {:?}", bad);
    }
}

#[test]
fn validate_subst_value_rejects_empty() {
    assert!(validate_subst_value("").is_err());
    assert!(validate_subst_value("   ").is_err());
}

#[test]
fn with_git_long_paths_enables_core_longpaths_on_windows() {
    // Windows 的 MAX_PATH(260) 会让深目录仓库 clone/checkout 直接失败；
    // `-c core.longpaths=true` 让 git 改用 \\?\ 前缀的长路径。
    // 抄自 ai-toolbox 95bdfdf3（`fix: Windows端clone仓库path过深的问题`）。
    let args = with_git_long_paths(&["clone", "--depth", "1", "url", "dest"]);
    let expected: Vec<String> = if cfg!(windows) {
        vec![
            "-c",
            "core.longpaths=true",
            "clone",
            "--depth",
            "1",
            "url",
            "dest",
        ]
    } else {
        vec!["clone", "--depth", "1", "url", "dest"]
    }
    .into_iter()
    .map(str::to_string)
    .collect();
    assert_eq!(args, expected);
}

#[test]
fn with_git_long_paths_keeps_original_args_intact() {
    // 前缀不得吞掉或改写原有参数（顺序敏感：git 的全局选项必须在子命令之前）。
    let args = with_git_long_paths(&["pull"]);
    assert_eq!(args.last().map(String::as_str), Some("pull"));
    assert_eq!(args.len(), if cfg!(windows) { 3 } else { 1 });
    if cfg!(windows) {
        assert_eq!(args[0], "-c");
        assert_eq!(args[1], "core.longpaths=true");
    }
}

#[test]
fn resolve_detected_path_parses_json_field_without_leaking_json() {
    let output = serde_json::json!({
        "global_cache_dir": r"D:\zig-cache",
        "version": "0.15.1"
    })
    .to_string();
    assert_eq!(
        resolve_detected_path(&output, Some("global_cache_dir")).as_deref(),
        Some(r"D:\zig-cache")
    );
    assert!(resolve_detected_path(&output, Some("missing")).is_none());
    assert_eq!(
        resolve_detected_path(r"D:\plain-cache", None).as_deref(),
        Some(r"D:\plain-cache")
    );
}

// ─── 端口占用检测（Q-0095：Android 模拟器抢占 127.0.0.1:8554 导致 RTSP 假启动） ───

#[test]
fn port_conflict_detects_own_listener() {
    let me = std::process::id();

    // 回环地址监听必须被识别（qemu 场景：占用 127.0.0.1:8554，通配绑定"成功"掩盖冲突）
    let loopback = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let lport = loopback.local_addr().unwrap().port();
    assert!(
        port_owner_pids(lport).contains(&me),
        "自建回环监听（端口 {}）必须被检测到",
        lport
    );
    let desc = port_conflict_description(lport).expect("端口被占用时应返回占用者描述");
    assert!(
        desc.contains(&format!("PID {}", me)),
        "描述应包含占用进程 PID: {}",
        desc
    );

    // 通配地址监听同样要能被检测到
    let wildcard = std::net::TcpListener::bind(("0.0.0.0", 0)).unwrap();
    let wport = wildcard.local_addr().unwrap().port();
    assert!(port_owner_pids(wport).contains(&me));
}
