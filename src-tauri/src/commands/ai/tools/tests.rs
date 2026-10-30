use super::*;
use std::path::Path;

/// winget 装已存在的包会返回 `0x8A15002B`（转为 i32 即 -1978335189）：
/// 「找不到可用的升级」对「安装工具」来说不是失败。
#[test]
fn winget_no_applicable_upgrade_counts_as_success() {
    let f = CmdFailure {
        exit_code: Some(-1978335189),
        output_tail: "配置的源中没有可用的较新的包版本。".to_string(),
    };
    assert!(f.is_winget_already_latest());
    assert_eq!(WINGET_ALREADY_LATEST, -1978335189);
}

/// 真正的失败不能被误判为「已是最新」
#[test]
fn other_exit_codes_are_not_already_latest() {
    let f = CmdFailure {
        exit_code: Some(1),
        output_tail: "".to_string(),
    };
    assert!(!f.is_winget_already_latest());
    assert_eq!(
        f.message(),
        "命令以 exit code: 1 退出（且未捕获到任何输出）"
    );
}

#[test]
fn pm_command_maps_npm_and_pip() {
    // 只认 npm / pip；其它包管理器回落到工具自带的官方命令
    assert_eq!(
        pm_commands(Some("npm"), "@openai/codex", PmOp::Install),
        vec!["npm install -g @openai/codex@latest"]
    );
    assert_eq!(
        pm_commands(Some("npm"), "@openai/codex", PmOp::Uninstall),
        vec!["npm uninstall -g @openai/codex"]
    );
    // pip 有两条候选：Windows 上 pip.exe 常不在 PATH，第二备用 `python -m pip`
    assert_eq!(
        pm_commands(Some("pip"), "headroom-ai", PmOp::Install),
        vec![
            "pip install --upgrade headroom-ai",
            "python -m pip install --upgrade headroom-ai"
        ]
    );
    assert_eq!(
        pm_commands(Some("pip"), "headroom-ai", PmOp::Uninstall),
        vec![
            "pip uninstall -y headroom-ai",
            "python -m pip uninstall -y headroom-ai"
        ]
    );
    assert!(pm_commands(None, "pi", PmOp::Install).is_empty());
    assert!(pm_commands(Some("go"), "pi", PmOp::Uninstall).is_empty());
}

#[test]
fn still_installed_uses_the_same_verdict_as_detection() {
    // 卸载的「成功」判定必须和 detect 同一口径：PM 记录 / detect_cmd / 声明路径
    // 任一命中都还算已安装。只看 exe 会漏掉「垫片删了、包还在 npm 里」的情况，
    // 表现为界面报卸载成功、却仍显示已安装。
    assert!(still_installed_from(true, false, false));
    assert!(still_installed_from(false, true, false));
    assert!(still_installed_from(false, false, true));
    assert!(!still_installed_from(false, false, false));
}

#[test]
fn pm_command_for_install_dir_uses_the_package_manager_that_owns_the_shim() {
    // 卸载必须用「当初装这个包的那一个」包管理器：目录里有 npm.cmd 就用它
    // （而不是 PATH 里第一个 npm —— 它可能在另一个全局前缀下，卸载毫无效果）
    let dir = std::env::temp_dir().join(format!("kira_pm_probe_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // 目录里什么都没有 → 认不出包管理器
    assert!(pm_command_for_install_dir(&dir, "pkg", PmOp::Uninstall).is_none());

    std::fs::write(dir.join("npm.cmd"), "").unwrap();
    let cmd = pm_command_for_install_dir(&dir, "@openai/codex", PmOp::Uninstall).unwrap();
    assert!(cmd.contains("uninstall -g @openai/codex"), "实际: {cmd}");
    assert!(
        cmd.contains(dir.join("npm.cmd").display().to_string().as_str()),
        "应带完整路径: {cmd}"
    );
    let install = pm_command_for_install_dir(&dir, "@openai/codex", PmOp::Install).unwrap();
    assert!(
        install.contains("install -g @openai/codex@latest"),
        "实际: {install}"
    );

    // pnpm / bun 各自的动词
    std::fs::remove_file(dir.join("npm.cmd")).unwrap();
    std::fs::write(dir.join("pnpm.cmd"), "").unwrap();
    assert!(pm_command_for_install_dir(&dir, "pkg", PmOp::Uninstall)
        .unwrap()
        .contains("remove -g pkg"));
    std::fs::remove_file(dir.join("pnpm.cmd")).unwrap();
    std::fs::write(dir.join("bun.exe"), "").unwrap();
    assert!(pm_command_for_install_dir(&dir, "pkg", PmOp::Uninstall)
        .unwrap()
        .contains("remove -g pkg"));

    // scoop shims 目录里没有包管理器，交给 scoop
    let shims = dir.join("shims");
    std::fs::create_dir_all(&shims).unwrap();
    assert_eq!(
        pm_command_for_install_dir(&shims, "pkg", PmOp::Uninstall),
        Some("scoop uninstall pkg".to_string())
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn package_present_in_dir_checks_global_node_modules_layout() {
    // 预检：npm 卸载「没装的包」也返回成功，不拦会让上层误判渠道有效
    let dir = std::env::temp_dir().join(format!("kira_pkg_probe_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("node_modules").join("@openai/codex")).unwrap();
    assert!(package_present_in_dir(&dir, "@openai/codex"));
    assert!(!package_present_in_dir(&dir, "@openai/other"));
    // 路径穿越一律拒绝
    assert!(!package_present_in_dir(&dir, "../evil"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tool_data_dirs_only_accepts_existing_subdirs_of_home() {
    // 卸载时删的是用户数据（~/.pi、~/.codex），口径必须比删安装文件更严：
    // 只认主目录下真实存在的子目录，绝对路径 / `..` / 主目录本身一律拒绝
    let home = std::env::temp_dir().join(format!("kira_data_dirs_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join(".pi")).unwrap();
    std::fs::create_dir_all(home.join(".config/deveco")).unwrap();

    let got = tool_data_dirs(
        &[
            ".pi".to_string(),
            ".config/deveco".to_string(),
            ".missing".to_string(),
            "..".to_string(),
            "../evil".to_string(),
        ],
        &home,
    );
    let names: Vec<String> = got
        .iter()
        .map(|p| p.strip_prefix(&home).unwrap().display().to_string())
        .collect();
    assert_eq!(names, vec![".pi", ".config/deveco"]);

    // 绝对路径与空串也要挡掉（防止注册表被改坏时误删系统目录）
    let abs = if cfg!(windows) { "C:\\Windows" } else { "/etc" };
    assert!(tool_data_dirs(&[abs.to_string(), "  ".to_string()], &home).is_empty());
    // 主目录本身绝不能被当成「数据目录」删掉
    assert!(tool_data_dirs(&[".".to_string()], &home).is_empty());

    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn find_exe_in_dirs_walks_dirs_and_shim_suffixes_in_order() {
    // 非包管理器安装的工具往往只在 PATH 里（注册表声明的路径找不到）：
    // 按文件清理要能在目录列表里找到它，且优先命中 .cmd 垫片
    let dir = std::env::temp_dir().join(format!("kira_path_probe_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("pi.cmd"), "").unwrap();

    let missing = dir.join("does-not-exist");
    let found = find_exe_in_dirs(vec![missing, dir.clone()], "pi");
    assert_eq!(found.as_deref(), Some(dir.join("pi.cmd").as_path()));

    // 名字对不上就找不到（不能瞎猜）
    assert!(find_exe_in_dirs(vec![dir.clone()], "other-tool").is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn shim_variants_include_existing_siblings() {
    // npm 在 Windows 上放的是 codex / codex.cmd / codex.ps1 三个垫片，
    // 检测只命中一个，卸载必须把同目录的兄弟一起收走
    let shim = Path::new("/npm/codex.cmd");
    let existing = ["codex.cmd", "codex", "codex.ps1"];
    let got = shim_variants_with(shim, |p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| existing.contains(&n))
    });
    let names: Vec<String> = got
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    assert_eq!(names, vec!["codex.cmd", "codex.ps1", "codex"]);
    assert!(got.iter().all(|p| p.parent() == shim.parent()));
}

#[test]
fn shim_variants_do_not_duplicate_the_detected_exe() {
    // 无后缀的 Unix 安装（~/.local/bin/pi）只会命中它自己，不能重复出现
    let shim = Path::new("/home/u/.local/bin/pi");
    let got = shim_variants_with(shim, |p| {
        p.file_name().and_then(|n| n.to_str()) == Some("pi")
    });
    assert_eq!(got.len(), 1);
    assert_eq!(got[0], shim);
}

#[test]
fn pkg_dir_candidates_only_return_existing_package_dirs() {
    // 垫片在 <prefix>/bin 下时，包目录在 <prefix>/lib/node_modules 里
    let shim = Path::new("/home/u/.npm-global/bin/codex");
    let existing = Path::new("/home/u/.npm-global")
        .join("lib")
        .join("node_modules")
        .join("@openai/codex");
    let got = pkg_dir_candidates_with(shim, "@openai/codex", |p| p == existing);
    assert_eq!(got, vec![existing]);
}

#[test]
fn is_protected_path_guards_system_dirs() {
    if cfg!(windows) {
        assert!(is_protected_path(Path::new(
            r"C:\Windows\System32\where.exe"
        )));
        assert!(!is_protected_path(Path::new(
            r"C:\Users\u\AppData\Roaming\npm\codex.cmd"
        )));
    } else {
        assert!(is_protected_path(Path::new("/usr/bin/pi")));
        assert!(!is_protected_path(Path::new("/home/u/.local/bin/pi")));
    }
}
