use super::*;

// ─── 依赖状态与安装 ───

/// 插件沙箱状态（Node 权限模型）。
#[derive(Debug, Clone, Serialize)]
pub struct SandboxStatus {
    /// 是否启用了 `--permission`
    pub enabled: bool,
    /// 未启用时的原因（如 Node 版本过低）
    pub reason: Option<String>,
}

/// 依赖现状（直接喂给前端展示）。
#[derive(Debug, Clone, Serialize)]
pub struct DepsReport {
    /// 依赖是否齐备（可以跑插件）。
    pub ready: bool,
    pub node_found: bool,
    /// node 可执行文件路径（找到了才有）。
    pub node_path: Option<String>,
    pub npm_path: Option<String>,
    /// 缺失的包名（空 = 齐备）。
    pub missing_packages: Vec<String>,
    /// 插件根目录（界面上「打开目录」用）。
    pub root: String,
    /// 环境问题（缺 node 等）；有值时 `ready` 必为 false。
    pub problem: Option<String>,
    /// 插件沙箱状态
    pub sandbox: SandboxStatus,
}

/// 读磁盘得出依赖现状（除沙箱探测外不跑命令）。
pub fn deps_report() -> DepsReport {
    let missing: Vec<String> = REQUIRED_PACKAGES
        .iter()
        .filter(|name| !node_modules_dir().join(name).join("package.json").is_file())
        .map(|name| name.to_string())
        .collect();

    match resolve_toolchain() {
        Ok(toolchain) => DepsReport {
            ready: missing.is_empty(),
            node_found: true,
            node_path: Some(toolchain.node.resolved.to_string_lossy().to_string()),
            npm_path: Some(toolchain.npm.resolved.to_string_lossy().to_string()),
            missing_packages: missing,
            root: plugins_root().to_string_lossy().to_string(),
            problem: None,
            sandbox: sandbox_status(&toolchain.node),
        },
        Err(problem) => DepsReport {
            ready: false,
            node_found: false,
            node_path: None,
            npm_path: None,
            missing_packages: missing,
            root: plugins_root().to_string_lossy().to_string(),
            problem: Some(problem),
            // 连 node 都没有，沙箱无从谈起；问题已经由 problem 说清楚了
            sandbox: SandboxStatus {
                enabled: false,
                reason: None,
            },
        },
    }
}

/// 跑一个子进程，把 stdout / stderr 的每一行交给 `log`（npm 的进度在 stderr 上）。
///
/// 用「两个转发线程 + 一个消费循环」而不是 `Command::output()`：
/// 安装要几十秒，必须能边跑边把进度报给前端，否则界面只能干等。
pub(super) fn run_streaming(cmd: &mut Command, log: &mut dyn FnMut(&str)) -> Result<bool, String> {
    hide_console(cmd);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("启动进程失败: {e}"))?;

    let (tx, rx) = mpsc::channel::<String>();
    let mut pumps = Vec::new();
    if let Some(out) = child.stdout.take() {
        let tx = tx.clone();
        pumps.push(std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                let _ = tx.send(line);
            }
        }));
    }
    if let Some(err) = child.stderr.take() {
        let tx = tx.clone();
        pumps.push(std::thread::spawn(move || {
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                let _ = tx.send(line);
            }
        }));
    }
    drop(tx); // 主线程不持有发送端，子进程结束后 rx 才会断开

    while let Ok(line) = rx.recv() {
        if !line.trim().is_empty() {
            log(&line);
        }
    }
    for pump in pumps {
        let _ = pump.join();
    }
    let status = child.wait().map_err(|e| format!("等待进程结束失败: {e}"))?;
    Ok(status.success())
}

/// 安装 / 补齐插件依赖（幂等：已就绪直接返回，不跑 npm）。
///
/// 依赖是**功能级前置**（一次装、所有插件共享），不像插件那样按个安装 ——
/// 插件依赖的都是 axios/cheerio 这类通用库。
pub fn install_deps(log: &mut dyn FnMut(&str)) -> Result<DepsReport, String> {
    ensure_layout()?;
    let before = deps_report();
    if let Some(problem) = before.problem.clone() {
        return Err(problem);
    }
    if before.ready {
        log("依赖已就绪，无需安装");
        return Ok(before);
    }

    let toolchain = resolve_toolchain()?;
    log(&format!(
        "使用 node: {} / npm: {}",
        toolchain.node.resolved.display(),
        toolchain.npm.resolved.display()
    ));
    log(&format!("正在安装插件依赖到 {}", plugins_root().display()));

    let mut cmd = Command::new(&toolchain.npm.program);
    cmd.args(&toolchain.npm.prefix_args)
        .arg("install")
        .arg("--no-audit")
        .arg("--no-fund")
        .arg("--loglevel=error")
        .current_dir(plugins_root());

    if !run_streaming(&mut cmd, log)? {
        // npm 失败时磁盘上可能留下半个 node_modules，下次会被「缺失包」检查抓出来，
        // 所以这里只报错、不做清理（清理反而可能删掉用户已有的其它内容）。
        return Err("npm install 失败，请检查网络或用终端进入插件目录手动安装".to_string());
    }

    let after = deps_report();
    if !after.ready {
        return Err(format!(
            "安装完成但仍缺少依赖: {}",
            after.missing_packages.join(", ")
        ));
    }
    log("依赖安装完成");
    Ok(after)
}
