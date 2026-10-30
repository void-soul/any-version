use super::*;

/// 执行构建（若有 build_script）。
pub(super) fn pm_build(
    app: &tauri::AppHandle,
    def: &NodeProjectDef,
    dir: &Path,
) -> Result<(), String> {
    if def.build_script.trim().is_empty() {
        return Ok(());
    }
    let pm = &def.package_manager;
    let (prog, prefix) = resolve_pm_invocation(def)?;
    emit_progress(
        app,
        &def.id,
        "build",
        &format!("正在运行 `{} run {}`…", pm, def.build_script),
    );
    let mut args = prefix;
    args.push("run".to_string());
    args.push(def.build_script.clone());
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let (ok, last_err, _out) = run_capture_live(
        app,
        &def.id,
        "build",
        &prog,
        &arg_refs,
        Some(dir),
        &[],
        None,
    );
    if !ok {
        let msg = last_err.trim();
        return Err(format!(
            "`{} run {}` 失败: {}",
            pm,
            def.build_script,
            if msg.is_empty() { "未知错误" } else { msg }
        ));
    }
    Ok(())
}

/// 从包描述中剥离版本段（`@scope/name@1.0.0` → `@scope/name`；`name@1.0.0` → `name`），
/// 用于升级时拼 `@latest`。无版本段时原样返回。
pub(super) fn strip_pkg_version(pkg: &str) -> &str {
    match pkg.rfind('@') {
        Some(idx) if idx > 0 => &pkg[..idx],
        _ => pkg,
    }
}

/// npx 模式安装/升级：`npm install --prefix <托管目录> <包名>`。
/// 一步到位（免 git clone / 依赖安装 / 编译）；`latest=true` 时强制取最新版（升级语义）。
///
/// 安装进程在 AnyVersion 的 CWD（通常是仓库根）里继承环境，npm 7+ 会向上查找
/// package.json / pnpm-workspace.yaml 把安装当 workspace 操作处理——仓库用的
/// pnpm `workspace:*` 协议依赖会触发 `EUNSUPPORTEDPROTOCOL Unsupported URL Type
/// "workspace:"`（npm 不认识 pnpm 的协议）。因此显式传 `--prefix` 指向托管目录
/// 并注入 `npm_config_workspaces=false` + `npm_config_workspaces_update=false`
/// 彻底禁用 workspace 模式；同时清理继承的 npm/pnpm 环境变量，防止宿主 shell
/// 的 `npm_config_*` 覆盖本次安装行为。
pub(super) const NPM_ISOLATION_ENV: &[(&str, &str)] = &[
    // 显式关闭 workspaces 解析（含向上冒泡查找），npm 7+ 生效
    ("npm_config_workspaces", "false"),
    ("npm_config_workspaces_update", "false"),
    ("npm_config_workspaces_install", "false"),
    // pnpm 环境变量若被继承会改变依赖解析行为，清掉
    ("npm_config_shared_workspace_lockfile", "false"),
    ("npm_config_shell_emulator", "false"),
    // 关闭 fund/audit，同时强制 npm 输出可被前端逐条显示的详细日志。
    ("npm_config_fund", "false"),
    ("npm_config_audit", "false"),
    ("npm_config_loglevel", "verbose"),
    ("npm_config_progress", "true"),
    ("npm_config_foreground_scripts", "true"),
    ("npm_config_timing", "true"),
    ("npm_config_update_notifier", "false"),
];

pub(super) const NPM_OUTPUT_ARGS: &[&str] = &[
    "--workspaces=false",
    "--no-fund",
    "--no-audit",
    "--loglevel=verbose",
    "--progress=true",
    "--foreground-scripts",
    "--timing",
    // 显式 fetch 超时/重试，缓解 npm(undici) 网络请求 CloseWait 挂死问题
    "--fetch-timeout=60000",
    "--fetch-retries=2",
    "--fetch-retry-mintimeout=10000",
    "--fetch-retry-maxtimeout=60000",
];

pub(super) fn npx_install_args(
    prefix: Vec<String>,
    runtime_dir: &Path,
    spec: String,
) -> Vec<String> {
    let mut args = prefix;
    args.push("install".to_string());
    args.push("--prefix".to_string());
    args.push(runtime_dir.to_string_lossy().to_string());
    args.extend(NPM_OUTPUT_ARGS.iter().map(|arg| (*arg).to_string()));
    args.push(spec);
    args
}

/// pnpm 安装参数：`pnpm add --dir <运行目录> --ignore-workspace <spec>`。
///
/// `--ignore-workspace` 必须加：运行目录位于托管目录内部，而托管目录可能残留旧版
/// Git checkout 的 `pnpm-workspace.yaml`，pnpm 会向上识别为 workspace 而对本次安装报错。
pub(super) const PNPM_OUTPUT_ARGS: &[&str] = &["--ignore-workspace"];

/// 按包管理器名组装 npx 模式的安装参数。
/// pnpm → `pnpm add --dir <dir> --ignore-workspace <spec>`
/// yarn → `yarn --cwd <dir> add <spec>`
/// npm  → `npm install --prefix <dir> <flags> <spec>`
pub(super) fn npx_pm_args(
    pm: &str,
    prefix: Vec<String>,
    runtime_dir: &Path,
    spec: String,
) -> Vec<String> {
    match pm {
        "pnpm" => {
            let mut a = prefix;
            a.push("add".to_string());
            a.push("--dir".to_string());
            a.push(runtime_dir.to_string_lossy().to_string());
            a.extend(PNPM_OUTPUT_ARGS.iter().map(|arg| (*arg).to_string()));
            a.push(spec);
            a
        }
        "yarn" => {
            let mut a = prefix;
            a.push("--cwd".to_string());
            a.push(runtime_dir.to_string_lossy().to_string());
            a.push("add".to_string());
            a.push(spec);
            a
        }
        _ => npx_install_args(prefix, runtime_dir, spec),
    }
}

/// 选择 npx 模式的安装器：优先项目配置的包管理器（harness 为 pnpm），未安装则回退 npm。
///
/// 必须优先 pnpm 的原因（实测 @deepseek-ai/dsh）：该包有 62 个直接依赖、505 个
/// 传递包，且**全部是预发布版本**（`0.1.1-rc.2` + `^0.1.1-rc.2` 范围）。npm 的依赖
/// 解析（idealTree/placeDep）在这种图上会组合爆炸：CPU 跑满、内存涨到 2.4GB 且不
/// 收敛，15 分钟仍停在同一个 placeDep 步骤（与网络无关，换镜像/代理都一样）。
/// pnpm 的解析器可在 1 分钟内完成同样的安装，且用硬链接复用全局 store，更省磁盘。
pub(super) fn npx_installer(
    app: &tauri::AppHandle,
    def: &NodeProjectDef,
) -> Result<(String, String, Vec<String>), String> {
    let configured = def.package_manager.trim().to_lowercase();
    if !configured.is_empty() && configured != "npm" {
        match resolve_exe_invocation(&configured, "") {
            Ok((prog, prefix)) => return Ok((configured, prog, prefix)),
            Err(_) => {
                emit_log(
                    app,
                    &def.id,
                    "npx",
                    &format!("未检测到 {}，回退使用 npm 安装", configured),
                );
            }
        }
    }
    let (prog, prefix) = resolve_exe_invocation("npm", "请先安装 Node.js (https://nodejs.org)")?;
    Ok(("npm".to_string(), prog, prefix))
}

pub(super) fn npx_install(
    app: &tauri::AppHandle,
    def: &NodeProjectDef,
    latest: bool,
) -> Result<(), String> {
    let pkg = def.npx_package.trim();
    let spec = if latest {
        format!("{}@latest", strip_pkg_version(pkg))
    } else {
        pkg.to_string()
    };
    emit_progress(app, &def.id, "npx", &format!("正在通过 npx 安装 {} …", pkg));
    // 不要把 Git workspace 根目录作为安装 prefix：旧版 harness checkout 中包含
    // `workspace:^`，npm 即使收到 --workspaces=false 也会先解析该根 package.json。
    let runtime_dir = def.npx_runtime_dir();
    fs::create_dir_all(&runtime_dir).map_err(|e| format!("创建 npx 运行目录失败: {}", e))?;
    // 预先创建最小 package.json，阻止包管理器向上冒泡到 managed_dir 的 workspace 元数据。
    let runtime_manifest = runtime_dir.join("package.json");
    if !runtime_manifest.exists() {
        fs::write(&runtime_manifest, "{\"private\":true}\n")
            .map_err(|e| format!("初始化 npx 运行目录失败: {}", e))?;
    }
    let (pm_name, prog, prefix) = npx_installer(app, def)?;
    emit_progress(
        app,
        &def.id,
        "npx",
        &format!("使用 {} 安装 {} …", pm_name, spec),
    );
    let args = npx_pm_args(&pm_name, prefix, &runtime_dir, spec);
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let (ok, last_err, full_output) = run_capture_live(
        app,
        &def.id,
        "npx",
        &prog,
        &arg_refs,
        None,
        NPM_ISOLATION_ENV,
        Some(Duration::from_secs(900)),
    );
    if !ok {
        let msg = last_err.trim();
        // pnpm 10+ 默认不执行依赖的构建脚本，并会以 ERR_PNPM_IGNORED_BUILDS
        // 非零码退出——但包本身已完整安装，只是原生模块（node-pty/koffi 等）未编译。
        // 这类失败不构成安装失败：批准构建需要本机 C++ 工具链（node-gyp + MSVC），
        // 在无构建环境的机器上强制开启会让安装彻底失败；而绝大多数服务
        // （实测 dsh web）并不依赖这些原生模块即可正常运行。
        // 仅当「确实是构建被跳过」且「包已安装」时才降级为警告。
        let ignored_builds = full_output.contains("ERR_PNPM_IGNORED_BUILDS")
            || msg.contains("ERR_PNPM_IGNORED_BUILDS")
            || full_output.contains("Ignored build scripts");
        if !(ignored_builds && def.npx_installed()) {
            return Err(format!(
                "{} 安装失败: {}",
                pm_name,
                if msg.is_empty() { "未知错误" } else { msg }
            ));
        }
        emit_log(
            app,
            &def.id,
            "npx",
            "已跳过部分原生模块构建（需本机 C++ 构建工具链），服务仍可启动；如确需原生模块，请安装构建工具后手动执行 pnpm approve-builds",
        );
    }
    if !def.npx_installed() {
        let _ = fs::remove_dir_all(&runtime_dir);
        return Err(format!(
            "安装完成但未找到包 {}，请检查 npxPackage 是否拼写正确",
            pkg
        ));
    }
    emit_progress(app, &def.id, "done", "npx 安装完成");
    Ok(())
}

/// 安装前检查依赖是否就绪，并返回 deps 结果用于报错。
pub(super) fn ensure_deps_ready(def: &NodeProjectDef) -> Result<DepCheckResult, String> {
    let deps = check_deps(def);
    // pip 包模式：直接从 PyPI 安装，无需 git（仅需 Python 可用）
    if def.is_pip_package() {
        if !deps.node.exists {
            return Err(
                "未检测到 python，请先安装 Python 3.8+ (https://www.python.org)".to_string(),
            );
        }
        if !deps.node.satisfies {
            return Err(format!(
                "Python 版本 {} 不满足要求 {}，请升级 Python",
                deps.node.version.as_deref().unwrap_or("未知"),
                deps.node.requirement.as_deref().unwrap_or("")
            ));
        }
        return Ok(deps);
    }
    // npx 模式只需 Node（npm/npx 随 Node 提供），无需 git / 独立包管理器
    if def.is_npx() {
        if !deps.node.exists {
            return Err("未检测到 node，请先安装 Node.js (https://nodejs.org)".to_string());
        }
        if !deps.node.satisfies {
            return Err(format!(
                "Node 版本 {} 不满足要求 {}，请升级 Node.js",
                deps.node.version.as_deref().unwrap_or("未知"),
                deps.node.requirement.as_deref().unwrap_or("")
            ));
        }
        return Ok(deps);
    }
    if def.is_python() {
        if !deps.git.exists {
            return Err("未检测到 git，请先安装 Git for Windows (https://git-scm.com)".to_string());
        }
        if !deps.node.exists {
            return Err(
                "未检测到 python，请先安装 Python 3.8+ (https://www.python.org)".to_string(),
            );
        }
        if !deps.node.satisfies {
            return Err(format!(
                "Python 版本 {} 不满足要求 {}，请升级 Python",
                deps.node.version.as_deref().unwrap_or("未知"),
                deps.node.requirement.as_deref().unwrap_or("")
            ));
        }
        return Ok(deps);
    }
    if !deps.git.exists {
        return Err("未检测到 git，请先安装 Git for Windows (https://git-scm.com)".to_string());
    }
    if !deps.node.exists {
        return Err("未检测到 node，请先安装 Node.js (https://nodejs.org)".to_string());
    }
    if !deps.node.satisfies {
        return Err(format!(
            "Node 版本 {} 不满足要求 {}，请升级 Node.js",
            deps.node.version.as_deref().unwrap_or("未知"),
            deps.node.requirement.as_deref().unwrap_or("")
        ));
    }
    if !deps.package_manager.exists {
        return Err(format!(
            "未检测到 {}，请先安装（npm i -g {} 或启用 corepack）",
            def.package_manager, def.package_manager
        ));
    }
    Ok(deps)
}
