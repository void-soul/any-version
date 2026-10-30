use super::*;

#[tauri::command]
pub async fn npm_install(app: tauri::AppHandle, project_id: String) -> Result<(), String> {
    let def = find_project(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    // docker 服务：不依赖本机 Node/git，改走「生成 compose + 拉镜像」
    if def.is_docker() {
        if def.installed() {
            return Err("项目已安装，如需更新请使用「升级」".to_string());
        }
        return docker_install(&app, &def);
    }
    ensure_deps_ready(&def)?;

    let dir = def.managed_dir();
    // 已安装过则报错（避免覆盖；升级请走 npm_upgrade）
    if def.installed() {
        return Err("项目已安装，如需更新请使用「升级」".to_string());
    }

    // npx 模式：直接 npm install --prefix，一步到位，无需 git clone / 依赖安装 / 编译
    if def.is_npx() {
        return npx_install(&app, &def, false);
    }

    // pip 包模式：直接从 PyPI 安装，无需 git clone
    if def.is_pip_package() {
        return pip_package_install(&app, &def, &dir, false);
    }

    emit_progress(&app, &def.id, "clone", &format!("正在克隆 {} …", def.repo));
    let base = crate::commands::config::get_data_dir();
    let parent = dir.parent().unwrap_or(&base);
    let _ = fs::create_dir_all(parent);
    let _ = fs::remove_dir_all(&dir);
    // 直接指定克隆目标目录为托管目录 {id}，避免依赖仓库名。
    let dest = dir.to_string_lossy().to_string();
    let clone_args: Vec<String> = crate::commands::utils::with_git_long_paths(&[
        "clone",
        "--depth",
        "1",
        def.repo.as_str(),
        dest.as_str(),
    ]);
    let arg_refs: Vec<&str> = clone_args.iter().map(|s| s.as_str()).collect();
    let (ok, last_err, _out) = run_capture_live(
        &app,
        &def.id,
        "clone",
        "git",
        &arg_refs,
        Some(parent),
        &[],
        None,
    );
    if !ok {
        let msg = last_err.trim();
        return Err(format!(
            "git clone 失败: {}",
            if msg.is_empty() { "未知错误" } else { msg }
        ));
    }

    let manifest = if def.is_python() {
        "requirements.txt"
    } else {
        "package.json"
    };
    if !dir.join(manifest).exists() {
        let _ = fs::remove_dir_all(&dir);
        return Err(format!("克隆完成但未找到 {}，可能仓库结构异常", manifest));
    }

    pm_install(&app, &def, &dir)?;
    pm_build(&app, &def, &dir)?;
    emit_progress(&app, &def.id, "done", "安装完成");
    Ok(())
}

/// `git pull` 失败是否因为「本地与上游没有共同祖先」。
///
/// 托管项目一律 `clone --depth 1` 浅克隆，本地只带最近一次提交；上游一旦改写历史
/// （rebase / squash / force-push），那次提交就不再是新 `origin/main` 的祖先，
/// 此后每次 `git pull` 都必然撞上 `refusing to merge unrelated histories`
/// （真机：`workbuddy2api` 本地停在 PR #26、远端已是 PR #28，`merge-base` 为空）。
///
/// 判定必须**只认这一句**：其它 pull 失败（合并冲突、认证失败、网络问题）若被误判，
/// 就会走到「丢弃本地提交」那条路，白扔用户的数据。
pub(super) fn is_unrelated_histories(stderr: &str) -> bool {
    stderr.contains("refusing to merge unrelated histories")
}

#[tauri::command]
pub async fn npm_upgrade(app: tauri::AppHandle, project_id: String) -> Result<(), String> {
    let def = find_project(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    if !def.installed() {
        return Err("项目尚未安装，请先安装".to_string());
    }
    // docker 服务：升级 = 拉新镜像 + 用新镜像重建容器（数据卷不动）
    if def.is_docker() {
        return docker_upgrade(&app, &def);
    }
    ensure_deps_ready(&def)?;

    let dir = def.managed_dir();
    // 先停止运行中的进程，避免 pull / 文件占用冲突
    let _ = stop_project_process(&def);

    // npx 模式：重新 npm install 即更新到最新版
    if def.is_npx() {
        return npx_install(&app, &def, true);
    }

    // pip 包模式：pip install --upgrade 即更新到最新版
    if def.is_pip_package() {
        return pip_package_install(&app, &def, &dir, true);
    }

    emit_progress(&app, &def.id, "pull", "正在拉取最新代码 (git pull)…");
    let (mut ok, mut last_err, _out) = run_capture_live(
        &app,
        &def.id,
        "pull",
        "git",
        &["pull"],
        Some(&dir),
        &[],
        None,
    );
    if !ok && is_unrelated_histories(&last_err) {
        // 浅克隆（--depth 1）+ 上游改写历史 → 本地与 origin/main 无共同祖先，pull 永远拒绝合并。
        // 托管项目的代码以 upstream 为准：对齐到 origin/main，而不是让用户每次手动处理。
        // `reset --hard` 只动已跟踪文件 —— 未跟踪的东西（.deps/、本地 .env、qa.db）原样保留。
        emit_progress(
            &app,
            &def.id,
            "pull",
            "本地历史与上游无共同祖先（浅克隆 + 上游改写过历史），正在丢弃本地提交并对齐到 origin/main（未跟踪文件保留）…",
        );
        let (reset_ok, reset_err, _) = run_capture_live(
            &app,
            &def.id,
            "pull",
            "git",
            &["reset", "--hard", "origin/main"],
            Some(&dir),
            &[],
            None,
        );
        if !reset_ok {
            return Err(format!(
                "git pull 失败（本地与上游无共同祖先），且无法对齐到 origin/main: {}",
                reset_err.trim()
            ));
        }
        // 对齐后一般已是最新；再 pull 一次确认状态干净（例如期间有新的远端提交）
        let (retry_ok, retry_err, _) = run_capture_live(
            &app,
            &def.id,
            "pull",
            "git",
            &["pull"],
            Some(&dir),
            &[],
            None,
        );
        ok = retry_ok;
        last_err = retry_err;
    }
    if !ok {
        let msg = last_err.trim();
        return Err(format!(
            "git pull 失败: {}",
            if msg.is_empty() { "未知错误" } else { msg }
        ));
    }

    pm_install(&app, &def, &dir)?;
    pm_build(&app, &def, &dir)?;
    emit_progress(&app, &def.id, "done", "升级完成");
    Ok(())
}

/// 单独安装依赖（不重新克隆/拉取代码）：执行包管理器 install + 构建脚本。
/// 用于项目已安装但依赖缺失/损坏时，仅重新安装依赖并打印实时日志。
#[tauri::command]
pub async fn npm_install_deps(app: tauri::AppHandle, project_id: String) -> Result<(), String> {
    let def = find_project(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    if !def.installed() {
        return Err("项目尚未安装，请先「安装」".to_string());
    }
    ensure_deps_ready(&def)?;

    let dir = def.managed_dir();
    // npx 模式没有独立依赖步骤：重新安装即拉取最新版
    if def.is_npx() {
        return npx_install(&app, &def, true);
    }

    // pip 包模式：重装即修复依赖
    if def.is_pip_package() {
        pip_package_install(&app, &def, &dir, false)?;
        emit_progress(&app, &def.id, "done", "依赖安装完成");
        return Ok(());
    }
    pm_install(&app, &def, &dir)?;
    pm_build(&app, &def, &dir)?;
    emit_progress(&app, &def.id, "done", "依赖安装完成");
    Ok(())
}

/// 编译原生模块（npx 模式专用）。
///
/// pnpm 10+ 默认跳过依赖的构建脚本，安装时会以 ERR_PNPM_IGNORED_BUILDS 非零码
/// 结束（包已完整安装，仅原生模块未编译）。本命令执行
/// `pnpm approve-builds --all`：非交互批准全部待构建依赖**并立即执行其构建脚本**
/// （实测 node-pty/koffi 等会当场完成 postinstall/编译；需要本机 C++ 工具链）。
/// npm 本身默认执行构建脚本，重跑用 `npm rebuild`；yarn 1 无拦截，重跑 `yarn rebuild`。
#[tauri::command]
pub async fn npm_build_native(app: tauri::AppHandle, project_id: String) -> Result<(), String> {
    let def = find_project(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    if !def.installed() {
        return Err("项目尚未安装，请先「安装」".to_string());
    }
    if !def.is_npx() {
        return Err("仅 npx 模式的项目需要单独编译原生模块".to_string());
    }
    let runtime_dir = def.npx_runtime_dir();
    let (pm_name, prog, prefix) = npx_installer(&app, &def)?;
    emit_progress(
        &app,
        &def.id,
        "native",
        &format!("使用 {} 编译原生模块…", pm_name),
    );
    let args = if pm_name == "pnpm" {
        let mut a = prefix;
        a.push("approve-builds".to_string());
        a.push("--all".to_string());
        a.push("--dir".to_string());
        a.push(runtime_dir.to_string_lossy().to_string());
        a
    } else if pm_name == "yarn" {
        let mut a = prefix;
        a.push("--cwd".to_string());
        a.push(runtime_dir.to_string_lossy().to_string());
        a.push("rebuild".to_string());
        a
    } else {
        let mut a = prefix;
        a.push("rebuild".to_string());
        a.push("--prefix".to_string());
        a.push(runtime_dir.to_string_lossy().to_string());
        a
    };
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    // pnpm 用 --dir、npm 用 --prefix、yarn 用 --cwd 指定运行目录，均无需额外 cwd
    let (ok, last_err, _out) = run_capture_live(
        &app,
        &def.id,
        "native",
        &prog,
        &arg_refs,
        None,
        NPM_ISOLATION_ENV,
        Some(Duration::from_secs(900)),
    );
    if !ok {
        let msg = last_err.trim();
        return Err(format!(
            "{} 编译原生模块失败: {}",
            pm_name,
            if msg.is_empty() { "未知错误" } else { msg }
        ));
    }
    emit_progress(&app, &def.id, "done", "原生模块编译完成");
    Ok(())
}

/// 在服务的运行目录中执行任意命令行。
///
/// npx 模式的服务不做全局安装，`dsh plugin --profile web add` 这类命令无法在
/// 系统任意位置使用；本命令把运行目录下的 `node_modules/.bin` 前置到 PATH 后
/// 在运行目录（npx = .npx-runtime，git = 托管目录）内执行整条命令行，
/// 项目自带命令与任意命令均可直接运行。
///
/// Windows 上用 `raw_arg` 传递整条命令行：若走普通 arg，Rust 会给含空格的命令
/// 加引号，触发 cmd.exe「首字符为引号时剥离首尾引号」的规则，把命令拆坏。
#[tauri::command]
pub async fn npm_exec(
    app: tauri::AppHandle,
    project_id: String,
    command: String,
) -> Result<(), String> {
    let command = command.trim().to_string();
    if command.is_empty() {
        return Err("命令不能为空".to_string());
    }
    let def = find_project(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    if !def.installed() {
        return Err("项目尚未安装，请先「安装」".to_string());
    }
    // docker 服务：命令在容器里执行（需在定义里声明 execService；未声明则明确拒绝，
    // 而不是猜一个容器名 —— 猜错会在错误的容器上执行命令）
    if def.is_docker() {
        let svc = def
            .docker
            .as_ref()
            .map(|d| d.exec_service.trim().to_string())
            .unwrap_or_default();
        if svc.is_empty() {
            return Err("该服务未开放「执行命令」".to_string());
        }
        return docker_compose_run(
            &app,
            &def,
            "exec",
            &["exec", "-T", svc.as_str(), "sh", "-lc", command.as_str()],
        );
    }
    let cwd = if def.is_npx() {
        def.npx_runtime_dir()
    } else {
        def.managed_dir()
    };
    if !cwd.exists() {
        return Err(format!("运行目录不存在: {}", cwd.display()));
    }

    let cmd = project_shell_command(&cwd, &command);

    emit_log(&app, &def.id, "exec", &format!("$ {}", command));
    emit_log(
        &app,
        &def.id,
        "exec",
        &format!("工作目录: {}", cwd.display()),
    );
    let (ok, last_err, _out) = run_cmd_live(
        &app,
        &def.id,
        "exec",
        &command,
        cmd,
        Some(Duration::from_secs(600)),
    );
    if !ok {
        let msg = last_err.trim();
        return Err(format!(
            "命令执行失败（退出码非 0）: {}",
            if msg.is_empty() { "未知错误" } else { msg }
        ));
    }
    emit_progress(&app, &def.id, "done", "命令执行完成");
    Ok(())
}

/// 构造「在服务运行目录里执行整条命令行」的 Command（`npm_exec` / `npm_init` 共用）。
///
/// Windows 上用 `raw_arg` 传整条命令行：若走普通 arg，Rust 会给含空格的命令加引号，
/// 触发 cmd.exe「首字符为引号时剥离首尾引号」的规则，把命令拆坏。
/// 同时把运行目录下的 `node_modules/.bin` 前置进 PATH，服务自带的命令无需全局安装即可调用。
pub(super) fn project_shell_command(cwd: &Path, command: &str) -> std::process::Command {
    #[cfg(windows)]
    let mut cmd = {
        use std::os::windows::process::CommandExt;
        let mut c = hidden_cmd(&cmd_exe_path());
        c.arg("/c");
        c.raw_arg(command);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = std::process::Command::new("sh");
        c.arg("-c");
        c.arg(command);
        c
    };
    cmd.current_dir(cwd);
    let bin_dir = cwd.join("node_modules").join(".bin");
    if bin_dir.exists() {
        if let Some(path) = std::env::var_os("PATH") {
            if let Ok(joined) =
                std::env::join_paths(std::iter::once(bin_dir).chain(std::env::split_paths(&path)))
            {
                cmd.env("PATH", joined);
            }
        }
    }
    cmd
}

/// 首次初始化服务：把 `initCmd` 在服务运行目录里跑一遍（如 `wigolo init`）。
///
/// 有些服务装完还不能直接用，必须先拉取运行时资源：wigolo 的 `init` 会下载浏览器引擎
/// 与端上模型（约 1.5 GB）。这里复用 [`npm_exec`] 的执行环境与日志通道，
/// 初始化输出直接出现在服务日志区；超时放宽到 1 小时（首次下载可能很久）。
#[tauri::command]
pub async fn npm_init(app: tauri::AppHandle, project_id: String) -> Result<(), String> {
    let def = find_project(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    if !def.has_init() {
        return Err("该服务未配置初始化命令".to_string());
    }
    if !def.installed() {
        return Err("项目尚未安装，请先「安装」".to_string());
    }
    let cwd = if def.is_npx() {
        def.npx_runtime_dir()
    } else {
        def.managed_dir()
    };
    if !cwd.exists() {
        return Err(format!("运行目录不存在: {}", cwd.display()));
    }

    let command = def.init_command_line();
    emit_log(&app, &def.id, "init", &format!("$ {}", command));
    emit_log(
        &app,
        &def.id,
        "init",
        &format!("工作目录: {}", cwd.display()),
    );
    emit_progress(
        &app,
        &def.id,
        "init",
        "正在初始化：首次会下载运行时资源（wigolo 约 1.5 GB），进度见下方日志",
    );

    let (ok, last_err, _out) = run_cmd_live(
        &app,
        &def.id,
        "init",
        &command,
        project_shell_command(&cwd, &command),
        Some(Duration::from_secs(3600)),
    );
    if !ok {
        let msg = last_err.trim();
        return Err(format!(
            "初始化失败（退出码非 0）: {}",
            if msg.is_empty() { "未知错误" } else { msg }
        ));
    }
    emit_progress(&app, &def.id, "done", "初始化完成");
    Ok(())
}
