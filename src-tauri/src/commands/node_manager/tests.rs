use super::*;

/// 只有这一句才算「无共同祖先」。别的 pull 失败（冲突 / 认证 / 网络）不能被误判 ——
/// 误判会走到 `reset --hard`，把本不该丢的本地提交白扔了。
#[test]
fn detects_only_the_unrelated_histories_failure() {
    assert!(is_unrelated_histories(
        "fatal: refusing to merge unrelated histories"
    ));
    // 真实输出里这句前面还跟着 fetch 日志，必须按「包含」判定
    assert!(is_unrelated_histories(
            "POST git-upload-pack (440 bytes)\nFrom https://github.com/x/y\n = [up to date] main -> origin/main\nfatal: refusing to merge unrelated histories"
        ));

    assert!(!is_unrelated_histories(
        "CONFLICT (content): Merge conflict in core/converter.py"
    ));
    assert!(!is_unrelated_histories(
        "error: Your local changes to the following files would be overwritten by merge"
    ));
    assert!(!is_unrelated_histories(
        "fatal: could not read Username for 'https://github.com'"
    ));
    assert!(!is_unrelated_histories(""));
}

#[test]
fn test_version_satisfies() {
    assert!(version_satisfies("20.11.0", ">=20"));
    assert!(version_satisfies("22.0.0", ">=20"));
    assert!(!version_satisfies("18.0.0", ">=20"));
    assert!(version_satisfies("18.19.0", ">=18"));
    assert!(!version_satisfies("17.0.0", ">=18"));
    assert!(version_satisfies("20.0.0", ""));
    assert!(version_satisfies("21.0.0", ">20"));
    assert!(!version_satisfies("20.0.0", ">20"));
    assert!(version_satisfies("18.0.0", "<=18"));
    assert!(version_satisfies("20.5.0", "20"));
}

/// 新加的定义文件必须在服务列表里看得到。
///
/// 这守的是「新加的服务不出现」：资源目录（`target/debug/_up_`）里那份
/// `node-projects/*.json` 是**打包时**拷的副本，后来新增的 json 不会同步过去。
/// 而搜索顺序曾把资源目录排在第一位、命中即 return，于是仓库里的新定义永远读不到。
///
/// 这里把资源目录设成 `target/debug`（与 `tauri dev` 运行时一致）来复现该场景。
#[test]
fn newly_added_definition_is_visible_in_dev() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let dev_target = repo.join("src-tauri").join("target").join("debug");
    crate::commands::utils::set_resource_dir(dev_target.clone());

    let picked = find_node_projects_dir().expect("应能找到 node-projects 目录");
    let picked = std::fs::canonicalize(&picked).unwrap();
    // 开发构建必须用仓库源目录，而不是 target/debug/_up_ 里的打包副本
    assert_eq!(
        picked,
        std::fs::canonicalize(repo.join("node-projects")).unwrap(),
        "开发构建应优先命中仓库里的 node-projects"
    );
    assert!(
        !picked.starts_with(&dev_target),
        "开发构建不该用 target/debug 下的打包副本（它不会随新增定义更新）"
    );
    assert!(
        NodeProjectRegistry::load().find("artex").is_some(),
        "artex 应出现在服务注册表里"
    );
}

/// 同一颗地雷也在另两个注册表上：`ai-tools` / `projects` 曾共用「资源目录优先」的
/// 搜索顺序，新增的 `ai-tools/*.json`、`projects/*/*.json` 同样读不到。
#[test]
fn dev_build_prefers_source_tree_for_all_definition_registries() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    crate::commands::utils::set_resource_dir(repo.join("src-tauri").join("target").join("debug"));
    let under_target = |p: &std::path::Path| p.starts_with(repo.join("src-tauri").join("target"));

    let ai_tools = crate::commands::ai_registry::AiToolRegistry::find_registry_dir();
    assert_eq!(
        std::fs::canonicalize(&ai_tools).unwrap(),
        std::fs::canonicalize(repo.join("ai-tools")).unwrap(),
        "ai-tools 注册表应命中仓库源目录"
    );

    let projects = crate::commands::project::registry::load_registry();
    assert!(
        !projects.is_empty(),
        "projects 注册表应从仓库源目录读到定义"
    );
}

/// ARTEX 的第二个对外端口**不许写死**。
///
/// 写死过一次（`127.0.0.1:8788:8788`），而 Kira 自带的 twoapi 代理默认就监听 8788
/// （`buddy/twoapi::DEFAULT_PORT`），于是 `docker compose up` 必然失败在
/// `ports are not available`，而且错误信息完全指不到「是你自己的应用占了端口」。
#[test]
fn artex_second_port_is_configurable_and_does_not_collide() {
    let raw = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../node-projects/artex.json"
    ));
    let def: NodeProjectDef = serde_json::from_str(raw).expect("artex.json 应能解析");
    assert_eq!(def.default_port2, 8789, "第二个端口要有自己的默认值");
    assert_ne!(
        def.default_port2, 8788,
        "8788 是 Kira twoapi 代理的默认端口，当默认值必然撞车"
    );
    let d = def.docker.as_ref().expect("artex 是 docker 服务");
    assert!(
        d.compose.contains("ARTEX_PORT2"),
        "第二个端口要走模板变量，不能写死"
    );
    assert!(
        !d.compose.contains("127.0.0.1:8788:8788"),
        "compose 里仍有写死的 8788"
    );
    assert!(d.env.contains("ARTEX_PORT2={{port2}}"), "env 要暴露该变量");
    // 两个默认端口还不能相同（compose 会自己报错，但那时已经创建了卷）
    assert_ne!(def.default_port, def.default_port2);
}

/// ARTEX 的定义文件必须能被解析成完整的 docker 服务。
///
/// 为什么需要这条：`serde` 默认**忽略未知字段**，所以模板里拼错一个字段名
/// （比如 `healthPort` 写错）会静默丢掉整份配置，界面表现成「声明了 docker
/// 类型却什么都不显示」，极难排查。这里把定义文件钉死。
#[test]
fn artex_definition_is_a_complete_docker_service() {
    let raw = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../node-projects/artex.json"
    ));
    let def: NodeProjectDef = serde_json::from_str(raw).expect("artex.json 应能解析");
    assert_eq!(def.id, "artex");
    assert!(def.is_docker(), "runtime 应为 docker 且 docker 配置非空");
    assert_eq!(def.effective_port(), 8787);

    let spec = def.docker_spec().expect("应能投影成 docker spec");
    assert_eq!(
        spec.project, "kira-artex",
        "compose 项目名要隔离，别和用户自己的撞"
    );
    assert!(
        spec.compose_tpl.contains("mianhuatang11/artex"),
        "镜像要用自维护源（原 autumn27/artex 已失效）"
    );
    assert!(
        spec.compose_tpl.contains("127.0.0.1:${ARTEX_PORT"),
        "端口必须只绑回环 —— 这类双用途安全工具不该暴露到局域网"
    );
    assert!(
        !spec.compose_tpl.contains("8787:8787"),
        "不应残留上游的全网卡端口映射"
    );
    assert!(
        spec.compose_tpl.contains("${POSTGRES_PASSWORD}"),
        "数据库密码要走 .env 变量，不能写死"
    );
    assert!(
        spec.env_tpl.contains("{{pg_password}}"),
        "env 模板要带密码占位符"
    );
    assert!(
        spec.volumes.iter().any(|v| v == "data"),
        "data 目录必须持久化（jwt.key / SQLite 在里面）"
    );
}

#[test]
fn test_web_path_render() {
    let def = NodeProjectDef {
        id: "harness".into(),
        display_name: "t".into(),
        repo: String::new(),
        website: String::new(),
        icon: String::new(),
        description: String::new(),
        default_port: 3080,
        default_port2: 0,
        web_path: "http://127.0.0.1:{port}".into(),
        node_requirement: String::new(),
        runtime: String::new(),
        package_manager: "pnpm".into(),
        build_script: String::new(),
        start_cmd: Vec::new(),
        managed: true,
        npx_package: String::new(),
        npx_bin: String::new(),
        console_url_pattern: String::new(),
        auto_open_console_url: false,
        pip_package: String::new(),
        pip_extra_packages: Vec::new(),
        pip_module: String::new(),
        env: HashMap::new(),
        init_cmd: Vec::new(),
        init_marker: String::new(),
        docker: None,
    };
    assert_eq!(def.resolved_web_path(), "http://127.0.0.1:3080");
}

#[test]
fn test_parse_version_parts() {
    assert_eq!(parse_version_parts("20.11.0"), vec![20, 11, 0]);
    assert_eq!(parse_version_parts("v22"), vec![22]);
    assert_eq!(parse_version_parts("0"), vec![0]);
}

fn npx_def(pkg: &str, bin: &str) -> NodeProjectDef {
    NodeProjectDef {
        id: "harness".into(),
        display_name: "t".into(),
        repo: String::new(),
        website: String::new(),
        icon: String::new(),
        description: String::new(),
        default_port: 3080,
        default_port2: 0,
        web_path: "http://127.0.0.1:{port}".into(),
        node_requirement: String::new(),
        runtime: String::new(),
        package_manager: String::new(),
        build_script: String::new(),
        start_cmd: Vec::new(),
        managed: true,
        npx_package: pkg.into(),
        npx_bin: bin.into(),
        console_url_pattern: String::new(),
        auto_open_console_url: false,
        pip_package: String::new(),
        pip_extra_packages: Vec::new(),
        pip_module: String::new(),
        env: HashMap::new(),
        init_cmd: Vec::new(),
        init_marker: String::new(),
        docker: None,
    }
}

#[test]
fn test_npx_bin_name() {
    assert_eq!(npx_def("cowsay", "").npx_bin_name(), "cowsay");
    assert_eq!(npx_def("@scope/name", "").npx_bin_name(), "name");
    assert_eq!(
        npx_def("@scope/name", "custom-bin").npx_bin_name(),
        "custom-bin"
    );
    assert_eq!(npx_def("deepseek-harness", "dsh").npx_bin_name(), "dsh");
}

#[test]
fn test_strip_pkg_version() {
    assert_eq!(strip_pkg_version("cowsay"), "cowsay");
    assert_eq!(strip_pkg_version("cowsay@1.5.0"), "cowsay");
    assert_eq!(strip_pkg_version("@scope/name"), "@scope/name");
    assert_eq!(strip_pkg_version("@scope/name@0.0.1-rc.1"), "@scope/name");
}

#[test]
fn test_npx_installed_path() {
    // 无实际安装时不视为已安装
    assert!(!npx_def("cowsay", "").installed());
    assert!(!npx_def("", "").is_npx());
    assert!(npx_def("@scope/name", "").is_npx());
}

#[test]
fn test_npx_runtime_dir_isolated_from_managed_root() {
    let def = npx_def("@deepseek-ai/dsh", "dsh");
    let runtime = def.npx_runtime_dir();
    assert_eq!(
        runtime.file_name().and_then(|x| x.to_str()),
        Some(".npx-runtime")
    );
    assert!(runtime.starts_with(def.managed_dir()));
    assert_ne!(runtime, def.managed_dir());
}

#[test]
fn test_npx_install_shows_verbose_output_flags() {
    let args = npx_install_args(
        vec!["/c".into(), "npm.cmd".into()],
        Path::new("runtime"),
        "@scope/pkg@latest".into(),
    );
    assert_eq!(args[2], "install");
    assert_eq!(args[3], "--prefix");
    assert!(args.contains(&"--loglevel=verbose".to_string()));
    assert!(args.contains(&"--progress=true".to_string()));
    assert!(args.contains(&"--foreground-scripts".to_string()));
    assert!(args.contains(&"--timing".to_string()));
    assert_eq!(args.last().map(String::as_str), Some("@scope/pkg@latest"));
}

#[test]
fn test_npx_pnpm_args_use_add_and_ignore_workspace() {
    let args = npx_pm_args(
        "pnpm",
        vec!["/c".into(), "pnpm.cmd".into()],
        Path::new("runtime"),
        "@scope/pkg@latest".into(),
    );
    assert_eq!(args[2], "add");
    assert_eq!(args[3], "--dir");
    assert_eq!(args[4], "runtime");
    assert!(args.contains(&"--ignore-workspace".to_string()));
    assert_eq!(args.last().map(String::as_str), Some("@scope/pkg@latest"));
}

#[test]
fn test_npx_npm_args_use_install_prefix() {
    let args = npx_pm_args(
        "npm",
        vec!["/c".into(), "npm.cmd".into()],
        Path::new("runtime"),
        "pkg".into(),
    );
    assert_eq!(args[2], "install");
    assert_eq!(args[3], "--prefix");
    assert_eq!(args[4], "runtime");
    assert_eq!(args.last().map(String::as_str), Some("pkg"));
}

#[test]
fn test_npx_yarn_args_use_cwd() {
    let args = npx_pm_args(
        "yarn",
        vec!["/c".into(), "yarn.cmd".into()],
        Path::new("runtime"),
        "pkg".into(),
    );
    assert_eq!(args[2], "--cwd");
    assert_eq!(args[3], "runtime");
    assert_eq!(args[4], "add");
    assert_eq!(args.last().map(String::as_str), Some("pkg"));
}

#[test]
fn test_devtools_browser_args_isolated_profile() {
    let args = devtools_browser_args(
        "http://127.0.0.1:3080",
        Path::new("C:\\Temp\\Kira DevTools"),
    );
    assert!(args.contains(&"--new-window".to_string()));
    assert!(args.contains(&"--auto-open-devtools-for-tabs".to_string()));
    assert!(args.contains(&"--no-first-run".to_string()));
    assert!(args.contains(&"--no-default-browser-check".to_string()));
    assert!(args
        .iter()
        .any(|x| x == "--user-data-dir=C:\\Temp\\Kira DevTools"));
    assert_eq!(
        args.last().map(String::as_str),
        Some("http://127.0.0.1:3080")
    );
}

#[test]
fn test_browser_candidates_keep_path_lookup_first() {
    // 不依赖本机是否安装浏览器：候选列表至少保持可由 PATH 命中的结果优先，
    // Windows 上再追加 Program Files / LOCALAPPDATA 的标准目录。
    let candidates = browser_candidates("a-browser-that-is-not-installed");
    assert!(candidates.is_empty());
    let candidates = browser_candidates("chrome");
    if !candidates.is_empty() {
        assert!(candidates
            .iter()
            .any(|p| p.to_string_lossy().to_lowercase().contains("chrome")));
    }
}

#[test]
fn test_format_command_quotes_paths() {
    let command = format_command(
        "npm.cmd",
        &["install", "--prefix", "C:\\Program Files\\Kira", "pkg"],
    );
    assert_eq!(
        command,
        "npm.cmd install --prefix \"C:\\Program Files\\Kira\" pkg"
    );
}

// ─── 控制台 URL 捕获 ───

#[test]
fn test_extract_console_url_dsh_line() {
    // dsh web 启动时打印：dsh web: http://127.0.0.1:3080/?token=xxx（可能带 LAN 后缀）
    let line = "dsh web: http://127.0.0.1:3080/?token=dV2LhQhkX5yF18tf9baCDDsdbIfeLslc2g8P8wiTMNY";
    assert_eq!(
        extract_console_url("dsh web: (http\\S+)", line).as_deref(),
        Some("http://127.0.0.1:3080/?token=dV2LhQhkX5yF18tf9baCDDsdbIfeLslc2g8P8wiTMNY")
    );
    // 行尾括号包裹的 URL 也要剥干净
    let line2 = "dsh web: http://127.0.0.1:3080/?token=abc123)";
    assert_eq!(
        extract_console_url("dsh web: (http\\S+)", line2).as_deref(),
        Some("http://127.0.0.1:3080/?token=abc123")
    );
    // 无匹配行 / 非 http 开头捕获组 → None
    assert_eq!(
        extract_console_url("dsh web: (http\\S+)", "some other log line"),
        None
    );
    assert_eq!(
        extract_console_url("found at (\\S+)", "found at ftp://x"),
        None
    );
    // 空 pattern → None
    assert_eq!(extract_console_url("", "dsh web: http://x"), None);
    // 非法 regex → None（不 panic）
    assert_eq!(extract_console_url("(unclosed", "anything"), None);
}

#[test]
fn test_console_url_lifecycle() {
    let id = "test-console-url-lifecycle";
    // 初始为空
    clear_console_url(id);
    assert_eq!(console_url_of(id), None);
    // 首次记录：changed = true，且可读回
    assert!(record_console_url_if_changed(
        id,
        "http://127.0.0.1:3080/?token=first"
    ));
    assert_eq!(
        console_url_of(id).as_deref(),
        Some("http://127.0.0.1:3080/?token=first")
    );
    // 重复命中同一 URL：changed = false（不重复自动打开浏览器）
    assert!(!record_console_url_if_changed(
        id,
        "http://127.0.0.1:3080/?token=first"
    ));
    // 覆盖（服务重启后新 token）：changed = true
    assert!(record_console_url_if_changed(
        id,
        "http://127.0.0.1:3080/?token=second"
    ));
    assert_eq!(
        console_url_of(id).as_deref(),
        Some("http://127.0.0.1:3080/?token=second")
    );
    // 清除后为空（停止服务）
    clear_console_url(id);
    assert_eq!(console_url_of(id), None);
}

#[test]
fn test_def_console_url_flags() {
    let mut def = npx_def("@deepseek-ai/dsh", "dsh");
    assert!(!def.has_console_url_pattern());
    def.console_url_pattern = "dsh web: (http\\S+)".into();
    assert!(def.has_console_url_pattern());
    // 配置文件 JSON 反序列化：camelCase 字段映射
    let json = r#"{
            "id": "x", "displayName": "X", "defaultPort": 1,
            "consoleUrlPattern": "dsh web: (http\\S+)", "autoOpenConsoleUrl": true
        }"#;
    let def: NodeProjectDef = serde_json::from_str(json).unwrap();
    assert_eq!(def.console_url_pattern, "dsh web: (http\\S+)");
    assert!(def.auto_open_console_url);
    assert!(def.has_console_url_pattern());
}

// ─── pip 包模式（PyPI 直装，无需 clone） ───

#[test]
fn test_pip_install_args_variants() {
    // 基础：装进当前环境（venv）
    assert_eq!(
        pip_install_args("headroom-ai[proxy]", &[], None, false),
        vec!["-m", "pip", "install", "headroom-ai[proxy]"]
    );
    // 升级 + 目标目录（嵌入式 Python 的 .deps 降级）
    assert_eq!(
        pip_install_args("headroom-ai[proxy]", &[], Some(".deps"), true),
        vec![
            "-m",
            "pip",
            "install",
            "--upgrade",
            "--target",
            ".deps",
            "headroom-ai[proxy]"
        ]
    );
}

#[test]
fn test_pip_launch_args_prepends_module() {
    let args = pip_launch_args(
        "headroom.cli",
        &["proxy", "--port", "8791"].map(String::from),
    );
    assert_eq!(args, vec!["-m", "headroom.cli", "proxy", "--port", "8791"]);
    // 无子命令时也能工作（仅 -m module）
    assert_eq!(pip_launch_args("pkg.mod", &[]), vec!["-m", "pkg.mod"]);
}

#[test]
fn test_headroom_service_def_is_pip_mode() {
    // 内置 headroom 服务项必须是 pip 包模式，且启动命令/端口自洽
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../node-projects/headroom.json");
    let raw = fs::read_to_string(&path).expect("headroom.json 读取失败");
    let def: NodeProjectDef = serde_json::from_str(&raw).expect("headroom.json 解析失败");
    assert!(def.is_pip_package(), "headroom 服务项应配置 pipPackage");
    assert_eq!(def.runtime, "python");
    assert_eq!(def.pip_module, "headroom.cli");
    assert!(
        def.start_cmd
            .iter()
            .any(|a| a == &def.default_port.to_string()),
        "startCmd 里的端口应与 defaultPort 一致"
    );
    assert!(def.start_cmd.first().map(|s| s.as_str()) == Some("proxy"));
    // 默认关闭匿名遥测（隐私）；文本压缩开关由 AI 设置页控制
    assert_eq!(
        def.env.get("HEADROOM_BEACON").map(String::as_str),
        Some("off")
    );
}

// ─── Python 运行时选择（venv 探测 / .deps 降级） ───

#[test]
fn test_select_python_runtime_prefers_venv_capable() {
    let probes = vec![
        ("py".to_string(), vec!["-3".to_string()], false, false),
        ("python".to_string(), vec![], true, true),
    ];
    assert_eq!(
        select_python_runtime(&probes),
        PythonRuntimePlan::Venv("python".to_string(), vec![])
    );
}

#[test]
fn test_select_python_runtime_falls_back_to_pip_target() {
    // 嵌入式发行版：无 venv（即便有 pip）→ 降级 --target 安装
    let probes = vec![
        ("py".to_string(), vec!["-3".to_string()], false, false),
        ("python".to_string(), vec![], false, true),
    ];
    assert_eq!(
        select_python_runtime(&probes),
        PythonRuntimePlan::TargetDeps("python".to_string(), vec![])
    );
}

#[test]
fn test_select_python_runtime_none_when_no_usable_python() {
    let probes = vec![("python".to_string(), vec![], false, false)];
    assert_eq!(select_python_runtime(&probes), PythonRuntimePlan::None);
}

#[test]
fn test_plan_python_launch_prefers_venv() {
    let root = std::env::temp_dir().join(format!("kira-pyplan-{}", std::process::id()));
    let venv_py = root.join(".venv").join("Scripts").join("python.exe");
    std::fs::create_dir_all(&venv_py.parent().unwrap()).unwrap();
    std::fs::write(&venv_py, b"").unwrap();

    assert_eq!(
        plan_python_launch(&root, &venv_py),
        PythonLaunchPlan::Venv(venv_py.clone())
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn test_plan_python_launch_deps_mode_uses_marker() {
    let root = std::env::temp_dir().join(format!("kira-pyplan-deps-{}", std::process::id()));
    let fake_py = root.join("base-python.exe");
    std::fs::create_dir_all(&root);
    std::fs::write(&fake_py, b"").unwrap();
    let deps = root.join(".deps");
    std::fs::create_dir_all(&deps);
    let marker = serde_json::json!({ "program": fake_py.to_string_lossy(), "prefix": [] });
    std::fs::write(deps.join(".python-runtime.json"), marker.to_string()).unwrap();

    let venv_py = root.join(".venv").join("Scripts").join("python.exe");
    match plan_python_launch(&root, &venv_py) {
        PythonLaunchPlan::Deps {
            program,
            prefix,
            deps_dir,
        } => {
            assert_eq!(program, fake_py.to_string_lossy());
            assert!(prefix.is_empty());
            assert_eq!(deps_dir, deps);
        }
        other => panic!("期望 Deps，实际 {:?}", other),
    }
    let _ = std::fs::remove_dir_all(&root);
}

// ─── pywin32 .deps 子目录探测 / pip extras 拼参 ───

#[test]
fn test_with_utf8_flag_prepends_x_utf8() {
    let args = with_utf8_flag(vec!["-m".into(), "core.converter".into()]);
    assert_eq!(args, vec!["-X", "utf8", "-m", "core.converter"]);
    let empty = with_utf8_flag(vec![]);
    assert_eq!(empty, vec!["-X", "utf8"]);
}

#[test]
fn test_deps_pythonpath_includes_project_and_deps() {
    let root = std::env::temp_dir().join(format!("kira-pypp-{}", std::process::id()));
    let proj = root.join("workbuddy2api");
    let deps = proj.join(".deps");
    std::fs::create_dir_all(&deps).unwrap();
    let pp = deps_pythonpath(&proj, &deps);
    let parts: Vec<&str> = if cfg!(windows) {
        pp.split(';').collect()
    } else {
        pp.split(':').collect()
    };
    assert_eq!(parts[0], deps.to_string_lossy());
    assert_eq!(
        parts[1],
        proj.to_string_lossy(),
        "项目源码目录必须在 PYTHONPATH（-m core 可导入）"
    );
    // 项目目录排在 .deps 之后（依赖优先于同名源码目录是安全序；这里仅验证包含）
    assert!(parts.len() >= 2);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn test_pywin32_deps_dirs_only_existing() {
    let root = std::env::temp_dir().join(format!("kira-pyw32-{}", std::process::id()));
    let deps = root.join(".deps");
    std::fs::create_dir_all(deps.join("win32").join("lib")).unwrap();
    std::fs::create_dir_all(deps.join("pywin32_system32")).unwrap();
    let dirs = pywin32_deps_dirs(&deps);
    assert_eq!(
        dirs.len(),
        3,
        "应命中 win32、win32/lib、pywin32_system32: {:?}",
        dirs
    );
    // 未安装 pywin32 的项目：.deps 存在但无子目录 → 空
    let bare = root.join(".deps-bare");
    std::fs::create_dir_all(&bare).unwrap();
    assert!(pywin32_deps_dirs(&bare).is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn test_pip_install_args_appends_extras() {
    let extras = vec!["pywin32; sys_platform == 'win32'".to_string()];
    let args = pip_install_args("headroom-ai[proxy]", &extras, Some(".deps"), false);
    assert_eq!(
        args,
        vec![
            "-m",
            "pip",
            "install",
            "--target",
            ".deps",
            "headroom-ai[proxy]",
            "pywin32; sys_platform == 'win32'",
        ]
    );
    // 空 extras 不产生多余参数
    let no_extra = pip_install_args("foo", &[], None, true);
    assert_eq!(no_extra, vec!["-m", "pip", "install", "--upgrade", "foo"]);
}

#[test]
fn test_installed_python_project_accepts_deps_runtime() {
    // Python git 项目没有 package.json：.deps 运行时标记存在即视为已安装
    let def = NodeProjectDef {
        id: "wb-test".into(),
        display_name: "wb".into(),
        repo: "https://x.git".into(),
        website: String::new(),
        icon: String::new(),
        description: String::new(),
        default_port: 8788,
        default_port2: 0,
        web_path: String::new(),
        node_requirement: String::new(),
        runtime: "python".into(),
        package_manager: "pip".into(),
        build_script: String::new(),
        start_cmd: vec!["-m".into(), "core.converter".into()],
        managed: true,
        npx_package: String::new(),
        npx_bin: String::new(),
        console_url_pattern: String::new(),
        auto_open_console_url: false,
        pip_package: String::new(),
        pip_extra_packages: Vec::new(),
        pip_module: String::new(),
        env: Default::default(),
        init_cmd: Vec::new(),
        init_marker: String::new(),
        docker: None,
    };
    let root = std::env::temp_dir().join(format!("kira-inst-{}", std::process::id()));
    let deps = root.join(".deps");
    std::fs::create_dir_all(&deps).unwrap();
    assert!(!def.installed_in(&root), "无标记时不应视为已安装");
    std::fs::write(
        deps.join(".python-runtime.json"),
        r#"{"program":"py","prefix":[]}"#,
    )
    .unwrap();
    assert!(def.installed_in(&root), ".deps 运行时标记存在应为已安装");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn test_plan_python_launch_missing_when_no_marker_or_bad_program() {
    let root = std::env::temp_dir().join(format!("kira-pyplan-miss-{}", std::process::id()));
    std::fs::create_dir_all(&root);
    let venv_py = root.join(".venv").join("Scripts").join("python.exe");

    // 无 .deps → Missing
    assert_eq!(
        plan_python_launch(&root, &venv_py),
        PythonLaunchPlan::Missing
    );

    // 有 .deps 但 marker 指向不存在的解释器 → 仍 Missing（启动必然失败）
    let deps = root.join(".deps");
    std::fs::create_dir_all(&deps);
    let marker =
        serde_json::json!({ "program": root.join("gone.exe").to_string_lossy(), "prefix": [] });
    std::fs::write(deps.join(".python-runtime.json"), marker.to_string()).unwrap();
    assert_eq!(
        plan_python_launch(&root, &venv_py),
        PythonLaunchPlan::Missing
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// 未配置 initCmd 的服务恒视为已初始化：既有服务的行为不能因这个新字段改变。
#[test]
fn test_initialized_defaults_true_without_init_config() {
    let def = npx_def("wigolo", "wigolo");
    assert!(!def.has_init());
    assert!(
        def.initialized(),
        "没配 initCmd 的服务不应出现「初始化」入口"
    );

    // 配了 initCmd 但没配 marker：无法判定，仍视为已初始化（不打扰用户）
    let mut no_marker = npx_def("wigolo", "wigolo");
    no_marker.init_cmd = vec!["init".into()];
    assert!(no_marker.has_init());
    assert!(no_marker.initialized());
}

/// marker 不存在 → 未初始化；marker 出现 → 已初始化。
#[test]
fn test_initialized_follows_marker_path() {
    let marker = std::env::temp_dir().join(format!("kira-init-marker-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&marker);

    let mut def = npx_def("wigolo", "wigolo");
    def.init_cmd = vec!["init".into()];
    def.init_marker = marker.to_string_lossy().to_string();
    assert!(!def.initialized(), "marker 不存在应视为未初始化");

    std::fs::create_dir_all(&marker).unwrap();
    assert!(def.initialized(), "marker 存在应视为已初始化");
    let _ = std::fs::remove_dir_all(&marker);
}

/// `~` 开头的 marker 要展开到用户主目录（wigolo 的 `~/.wigolo`）。
#[test]
fn test_init_marker_expands_home() {
    let mut def = npx_def("wigolo", "wigolo");
    def.init_marker = "~/.wigolo".into();
    let path = def.init_marker_path().expect("应能展开 ~");
    assert!(path.ends_with(".wigolo"));
    assert!(
        !path.to_string_lossy().starts_with('~'),
        "~ 应已展开: {}",
        path.display()
    );

    def.init_marker = String::new();
    assert!(def.init_marker_path().is_none());
}

/// npx 模式的初始化命令行要带上包自带的 bin（`wigolo init`）。
#[test]
fn test_init_command_line() {
    let mut def = npx_def("wigolo", "wigolo");
    def.init_cmd = vec!["init".into()];
    assert_eq!(def.init_command_line(), "wigolo init");

    // 非 npx 模式直接拼 initCmd，不加 bin 前缀
    let mut git_def = npx_def("", "");
    git_def.init_cmd = vec!["setup".into(), "--yes".into()];
    assert_eq!(git_def.init_command_line(), "setup --yes");
}
