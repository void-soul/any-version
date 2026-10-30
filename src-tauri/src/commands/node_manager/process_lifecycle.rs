use super::*;

// ─── 进程生命周期 ───

/// 记录由本应用 spawn 的 Node 服务根 PID。
pub(super) static RUNNING_PIDS: Mutex<Option<HashMap<String, u32>>> = Mutex::new(None);

pub(super) fn running_pids() -> std::sync::MutexGuard<'static, Option<HashMap<String, u32>>> {
    RUNNING_PIDS.lock().unwrap()
}

pub(super) fn record_pid(id: &str, pid: u32) {
    let mut g = running_pids();
    g.get_or_insert_with(HashMap::new)
        .insert(id.to_string(), pid);
}

pub(super) fn recorded_pid(id: &str) -> Option<u32> {
    running_pids().as_ref().and_then(|m| m.get(id).copied())
}

/// 停止项目进程树，并清除该项目的控制台 URL 捕获记录。
pub(crate) fn stop_project_process(def: &NodeProjectDef) -> Result<(), String> {
    // 控制台 URL 随进程失效（重启后由新输出重新捕获）
    clear_console_url(&def.id);
    // 1) 优先杀本应用记录的 PID 进程树
    if let Some(pid) = recorded_pid(&def.id) {
        let (_, stderr, ok) =
            run_capture("taskkill", &["/f", "/t", "/pid", &pid.to_string()], None);
        if !ok
            && !stderr.is_empty()
            && !stderr.contains("not found")
            && !stderr.contains("没有运行")
        {
            // 不直接报错，继续尝试端口杀
        }
        if let Some(m) = running_pids().as_mut() {
            m.remove(&def.id);
        }
    }
    // 2) 兜底：杀占用默认端口的进程（若是我们的）
    if let Some(pid) = crate::commands::utils::port_owner_pid(def.default_port) {
        let _ = run_capture("taskkill", &["/f", "/t", "/pid", &pid.to_string()], None);
    }
    Ok(())
}

#[tauri::command]
pub async fn npm_start(app: tauri::AppHandle, project_id: String) -> Result<(), String> {
    let def = find_project(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    if !def.installed() {
        return Err("项目尚未安装，请先安装".to_string());
    }
    // docker 服务没有 startCmd（由容器自带入口），启动 = compose up -d
    if def.is_docker() {
        return docker_start(&app, &def);
    }
    if def.start_cmd.is_empty() {
        return Err("该项目未配置启动命令".to_string());
    }

    // 端口冲突检测
    if let Some(pid) = crate::commands::utils::port_owner_pid(def.default_port) {
        let name = crate::commands::utils::process_name_by_pid(pid).unwrap_or_default();
        return Err(format!(
            "端口 {} 已被进程 {} (PID {}) 占用，请先停止或处理端口冲突",
            def.default_port, name, pid
        ));
    }

    let dir = def.managed_dir();

    // npx 模式启动策略：
    // - 未安装 → 先完整安装（npx_install 内部带 15 分钟超时兜底，网络挂死时
    //   不会永久卡住，超时会终止进程树并返回失败）。
    // - 已安装 → 先用 `npm view`（30s 超时）快速比对远程最新版，仅在有新版时
    //   才重新安装；比对失败/超时/离线则直接用本地已装版本启动，保证可启动性。
    //   之前「每次启动都全量 npm install pkg@latest」在网络慢或 npm fetch 挂死时
    //   会导致服务永远起不来，且正常情况下也要先花数分钟下载校验才能启动。
    if def.is_npx() {
        if !def.npx_installed() {
            npx_install(&app, &def, false)?;
        } else {
            let current = npx_local_version(&def);
            let latest = npm_remote_version(def.npx_package.trim());
            let outdated = match (&current, &latest) {
                (Some(c), Some(l)) => version_gt(l, c),
                _ => false,
            };
            if outdated {
                emit_progress(
                    &app,
                    &def.id,
                    "npx",
                    &format!(
                        "发现新版本 {}（本地 {}），正在更新…",
                        latest.as_deref().unwrap_or("?"),
                        current.as_deref().unwrap_or("?")
                    ),
                );
                if let Err(e) = npx_install(&app, &def, true) {
                    emit_log(
                        &app,
                        &def.id,
                        "stderr",
                        &format!("更新到最新版失败，使用本地已装版本启动: {}", e),
                    );
                }
            }
        }
    }

    // 组装启动命令：git 模式 `pnpm dsh web`；npx 模式 `npx -y --prefix <托管目录> <bin> <args...>`；
    // python 模式 `.venv 内 python <startCmd...>`（startCmd 即 python 参数，如 `-m core.converter`）；
    // .deps 降级模式：基准 python + PYTHONPATH=<托管目录>/.deps（嵌入式 Python 已在安装期补 ._pth）。
    let mut python_extra_env: Vec<(String, String)> = Vec::new();
    let (prog, args) = if def.is_python() {
        // pip 包模式：`python -m <pipModule> <startCmd...>`；普通 python 项目：startCmd 直接是 python 参数
        let pip_module = def.pip_module.trim().to_string();
        if def.is_pip_package() && pip_module.is_empty() {
            return Err("pip 包模式需要配置 pipModule（如 headroom.cli）".to_string());
        }
        let build_args = |cmd: &[String]| -> Vec<String> {
            let inner = if def.is_pip_package() {
                pip_launch_args(&pip_module, cmd)
            } else {
                cmd.to_vec()
            };
            with_utf8_flag(inner)
        };
        match plan_python_launch(&dir, &def.venv_python()) {
            PythonLaunchPlan::Venv(py) => {
                (py.to_string_lossy().to_string(), build_args(&def.start_cmd))
            }
            PythonLaunchPlan::Deps {
                program,
                prefix,
                deps_dir,
            } => {
                let mut a = prefix;
                a.extend(build_args(&def.start_cmd));
                // PYTHONPATH = .deps + 项目源码目录（`-m core.converter` 的 core 包在托管目录，
                // 嵌入式 Python 的 ._pth 锁死 sys.path 且不自动加 cwd，必须显式给出）
                // + pywin32 子目录（--target 安装的 pywin32 把模块放在 win32/ 与
                // pywin32_system32/ 下，不在 .deps 顶层）。
                python_extra_env.push(("PYTHONPATH".to_string(), deps_pythonpath(&dir, &deps_dir)));
                // pywintypes 的 DLL 依赖需要 pywin32_system32 进入 DLL 搜索路径（PATH）
                if let Some(sys32) = pywin32_deps_dirs(&deps_dir).into_iter().find(|p| {
                    p.file_name()
                        .map(|n| n == "pywin32_system32")
                        .unwrap_or(false)
                }) {
                    let mut path_val = sys32.to_string_lossy().to_string();
                    if let Some(existing) = std::env::var_os("PATH") {
                        path_val.push_str(";");
                        path_val.push_str(&existing.to_string_lossy());
                    }
                    python_extra_env.push(("PATH".to_string(), path_val));
                }
                (program, a)
            }
            PythonLaunchPlan::Missing => {
                return Err(
                    "Python 运行时未就绪（.venv 与 .deps 均缺失），请先「安装」或「安装依赖」"
                        .to_string(),
                )
            }
        }
    } else if def.is_npx() {
        let (p, prefix) = resolve_exe_invocation("npx", "请先安装 Node.js (https://nodejs.org)")?;
        let mut a = prefix;
        a.push("-y".to_string());
        a.push("--prefix".to_string());
        a.push(def.npx_runtime_dir().to_string_lossy().to_string());
        a.push(def.npx_bin_name());
        a.extend(def.start_cmd.iter().cloned());
        (p, a)
    } else {
        let (p, prefix) = resolve_pm_invocation(&def)?;
        let mut a = prefix;
        a.push(def.start_cmd[0].clone());
        a.extend_from_slice(&def.start_cmd[1..]);
        (p, a)
    };

    // npx 发布包运行在隔离 prefix 中，避免从旧 workspace checkout 读取配置。
    let runtime_cwd = if def.is_npx() {
        def.npx_runtime_dir()
    } else {
        dir.clone()
    };

    // 启动后台进程（CREATE_NO_WINDOW），记录 PID
    let mut cmd = hidden_cmd(&prog);
    cmd.args(&args);
    cmd.current_dir(&runtime_cwd);
    for (key, val) in &python_extra_env {
        cmd.env(key, val);
    }
    for (key, val) in &def.env {
        cmd.env(key, val);
    }
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    // 额外确保创建独立的进程组，便于后续 taskkill /T；
    // 通过 spawn_breakaway_fallback 尝试脱离 AnyVersion 生命周期，
    // 若所在 Job 不允许 breakaway 则自动降级（不会整体启动失败）。
    let mut child = crate::commands::hidden_cmd::spawn_breakaway_fallback(cmd)
        .map_err(|e| format!("启动失败: {}", e))?;
    record_pid(&def.id, child.id());

    // 实时回传启动日志（进程常驻，子线程持续读取 stdout/stderr 直到进程退出）。
    // 配置了 consoleUrlPattern 的项目，同时在输出行中提取带凭据主页地址：
    // 捕获到新地址（以最后一次输出为准）时记录、通知前端，并按配置自动打开。
    emit_progress(
        &app,
        &def.id,
        "start",
        &format!("启动命令: {}", def.start_cmd.join(" ")),
    );
    // 诊断：把实际命令行与关键 env 打进日志（排查「无输出退出」时能看到真实启动形态）
    {
        let pypp = python_extra_env
            .iter()
            .find(|(k, _)| k == "PYTHONPATH")
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        emit_log(
            &app,
            &def.id,
            "stdout",
            &format!("$ {} {}", prog, args.join(" ")),
        );
        emit_log(
            &app,
            &def.id,
            "stdout",
            &format!("  cwd: {}", runtime_cwd.display()),
        );
        if !pypp.is_empty() {
            emit_log(&app, &def.id, "stdout", &format!("  PYTHONPATH: {}", pypp));
        }
    }
    let mut readers: Vec<std::thread::JoinHandle<()>> = Vec::new();
    let app_out = app.clone();
    let pid_out = def.id.clone();
    let pattern_out = def.console_url_pattern.clone();
    let auto_open_out = def.auto_open_console_url;
    if let Some(mut so) = child.stdout.take() {
        readers.push(std::thread::spawn(move || {
            read_child_stream(
                &mut so,
                &app_out,
                &pid_out,
                "stdout",
                &pattern_out,
                auto_open_out,
            );
        }));
    }
    let app_err = app.clone();
    let pid_err = def.id.clone();
    let pattern_err = def.console_url_pattern.clone();
    let auto_open_err = def.auto_open_console_url;
    if let Some(mut se) = child.stderr.take() {
        readers.push(std::thread::spawn(move || {
            read_child_stream(
                &mut se,
                &app_err,
                &pid_err,
                "stderr",
                &pattern_err,
                auto_open_err,
            );
        }));
    }

    // 等待端口就绪（最多 120s，异步等待，不阻塞 UI）。
    // 期间持续回传进度，并检测进程是否提前退出（启动命令报错时避免干等）。
    let deadline = Instant::now() + Duration::from_secs(120);
    let started_at = Instant::now();
    let mut last_report = Instant::now();
    loop {
        if crate::commands::utils::port_owner_pid(def.default_port).is_some() {
            emit_progress(
                &app,
                &def.id,
                "running",
                &format!("已启动，端口 {}", def.default_port),
            );
            return Ok(());
        }
        // 进程已退出且端口未就绪 → 启动失败
        if let Ok(Some(status)) = child.try_wait() {
            let code_desc = status
                .code()
                .map(|c| format!("exit code {c}"))
                .unwrap_or_else(|| format!("{status:?}"));
            emit_log(
                &app,
                &def.id,
                "stderr",
                &format!(
                    "启动进程已退出（端口 {} 未就绪，{}）",
                    def.default_port, code_desc
                ),
            );
            // 等 reader 线程读到 EOF 并把尾部日志发到前端，避免「退出即失败、日志丢失」
            for h in readers.drain(..) {
                let _ = h.join();
            }
            return Err(format!(
                "启动进程已退出，端口 {} 未就绪（{}）。请查看上方日志确认报错",
                def.default_port, code_desc
            ));
        }
        if Instant::now() >= deadline {
            emit_progress(
                &app,
                &def.id,
                "running",
                "进程已启动（端口暂未就绪，请查看日志或稍候打开主页）",
            );
            return Ok(());
        }
        if last_report.elapsed() >= Duration::from_secs(5) {
            emit_progress(
                &app,
                &def.id,
                "starting",
                &format!(
                    "等待端口 {} 就绪…（已等待 {}s）",
                    def.default_port,
                    started_at.elapsed().as_secs()
                ),
            );
            last_report = Instant::now();
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[tauri::command]
pub async fn npm_stop(app: tauri::AppHandle, project_id: String) -> Result<(), String> {
    let def = find_project(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    // docker 服务的「停止」= compose down（容器由 daemon 持有，没有可 kill 的本地 PID）
    if def.is_docker() {
        return docker_stop(&app, &def);
    }
    stop_project_process(&def)
}

/// 卸载：停止进程并清空托管目录（{node_projects_dir}/{id}）。
/// 用于 npx 模式切换 / 重装前一键清理旧目录（含 node_modules 等残留）。
#[tauri::command]
pub async fn npm_uninstall(app: tauri::AppHandle, project_id: String) -> Result<(), String> {
    let def = find_project(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    // docker 服务走另一套「卸载」：停容器 + 删渲染产物，数据与密码都保留（见 docker_uninstall）
    if def.is_docker() {
        return docker_uninstall(&app, &def);
    }
    // 先停止运行中的进程，避免文件被占用导致删除失败
    let _ = stop_project_process(&def);

    let base = crate::commands::config::get_node_projects_dir();
    let dir = def.managed_dir();
    // 安全防护：托管目录必须是托管根目录的子目录（防配置异常/非法 id 误删其它目录）
    let base_c = base.canonicalize().unwrap_or_else(|_| base.clone());
    let dir_c = dir.canonicalize().unwrap_or_else(|_| dir.clone());
    if !dir_c.starts_with(&base_c) {
        return Err(format!(
            "拒绝卸载：托管目录 {} 不在托管根目录 {} 内",
            dir.to_string_lossy(),
            base.to_string_lossy()
        ));
    }
    if !dir.exists() {
        return Err(format!(
            "托管目录不存在（可能尚未安装）：{}",
            dir.to_string_lossy()
        ));
    }
    // 标准删除失败（只读/锁定）时用 `cmd /c rmdir /s /q` 兜底
    crate::commands::cache::remove_dir_all_forced(&dir)
        .map_err(|e| format!("清空托管目录失败: {}", e))?;
    Ok(())
}
