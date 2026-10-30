use super::*;

// ─── 依赖检测命令 ───

#[tauri::command]
pub fn npm_deps(project_id: String) -> Result<DepCheckResult, String> {
    let def = find_project(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    // docker 服务不依赖 git / node / pnpm：把「依赖就绪」直接映射成 Docker 就绪，
    // 这样前端原有的「全部就绪」门禁无需为 docker 单独放行。
    // 细节展示由 docker_env_check 负责（前端对 docker 服务隐藏这三行 git/node/pm）。
    if def.is_docker() {
        let env = crate::commands::docker::check_env();
        let ready = env.cli && env.daemon_running && env.compose;
        let mk = |name: &str, version: &str| DepCheck {
            name: name.to_string(),
            exists: ready,
            path: None,
            version: Some(version.to_string()),
            satisfies: ready,
            requirement: None,
        };
        return Ok(DepCheckResult {
            git: mk("docker", &env.cli_version),
            node: mk("compose", &env.compose_version),
            package_manager: mk("daemon", if env.daemon_running { "running" } else { "" }),
            all_ready: ready,
        });
    }
    Ok(check_deps(&def))
}

pub(super) fn check_deps(def: &NodeProjectDef) -> DepCheckResult {
    let git = check_dep("git", &["--version"]);
    // Python 运行时：node 槽位复用为 python 检查（前端徽章按 runtime 显示 python），
    // pm 槽位显示 pip（随 venv 提供，系统只需 python 就绪）。
    // `python --version` 输出 "Python 3.12.4"，parse_version_parts 会忽略非数字前缀。
    if def.is_python() {
        let mut py = check_dep("python", &["--version"]);
        py.name = "python".to_string();
        if py.exists {
            let v = py.version.as_deref().unwrap_or("");
            py.satisfies = version_satisfies(v, &def.node_requirement);
        } else {
            py.satisfies = false;
        }
        py.requirement = Some(def.node_requirement.clone());
        let mut pm = py.clone();
        pm.name = "pip".to_string();
        pm.requirement = Some("随 python 提供".to_string());
        pm.satisfies = py.exists && py.satisfies;
        // pip 包模式无需 git（直接从 PyPI 安装）
        let all_ready = py.exists && py.satisfies && (def.is_pip_package() || git.exists);
        return DepCheckResult {
            git,
            node: py,
            package_manager: pm,
            all_ready,
        };
    }
    let mut node = check_dep("node", &["--version"]);
    // node 版本约束
    if node.exists {
        let v = node
            .version
            .as_deref()
            .unwrap_or("")
            .trim_start_matches('v')
            .trim()
            .to_string();
        node.satisfies = version_satisfies(&v, &def.node_requirement);
        node.requirement = Some(def.node_requirement.clone());
    } else {
        node.satisfies = false;
        node.requirement = Some(def.node_requirement.clone());
    }
    // npx 模式：npm/npx 随 Node 分发，node 就绪即视为包管理器就绪；无需 git。
    let pm = if def.is_npx() {
        let mut p = check_dep("npm", &["--version"]);
        p.name = "npm".to_string();
        p.requirement = Some("随 node 提供".to_string());
        p.satisfies = node.exists && node.satisfies;
        p
    } else {
        check_dep(&def.package_manager, &["--version"])
    };
    let all_ready = if def.is_npx() {
        node.exists && node.satisfies
    } else {
        git.exists && node.exists && node.satisfies && pm.exists
    };
    DepCheckResult {
        git,
        node,
        package_manager: pm,
        all_ready,
    }
}

// ─── 状态命令 ───

#[tauri::command]
pub fn npm_status(project_id: String) -> Result<NodeProjectStatus, String> {
    let def = find_project(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    Ok(status_for(&def))
}

/// 更新检查结果：git 模式比较 commit；npx 模式比较本地已装版本与 npm registry 远程版本。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeUpdateInfo {
    pub has_update: bool,
    pub current_commit: String,
    pub latest_commit: String,
    pub behind: i64,
    /// npx 模式：本地已装版本号。
    #[serde(default)]
    pub current_version: Option<String>,
    /// npx 模式：npm registry 远程最新版本号。
    #[serde(default)]
    pub latest_version: Option<String>,
    pub error: Option<String>,
}

/// npx 模式读取本地已装版本：`.npx-runtime/node_modules/<包名>/package.json` 的 version 字段。
/// scoped 包会按 `@scope/name` 拆成多级目录；pnpm 符号链接也能读到目标文件。
pub(super) fn npx_local_version(def: &NodeProjectDef) -> Option<String> {
    let pkg = def.npx_package.trim();
    if pkg.is_empty() {
        return None;
    }
    let mut rel = PathBuf::from("node_modules");
    for part in pkg.split('/') {
        rel.push(part);
    }
    let pkg_json = def.npx_runtime_dir().join(rel).join("package.json");
    let data = fs::read_to_string(pkg_json).ok()?;
    let v: serde_json::Value = serde_json::from_str(&data).ok()?;
    v.get("version")
        .and_then(|x| x.as_str())
        .map(|s| s.trim().to_string())
}

/// 查询 npm registry 上包的最新版本：`npm view <pkg> version`。
/// 带超时（npm view 的网络请求同样可能挂死）；网络不可用 / 超时 / npm 不在
/// PATH / 包不存在时返回 None。
pub(super) fn npm_remote_version(pkg: &str) -> Option<String> {
    npm_remote_version_with_timeout(pkg, Duration::from_secs(30))
}

pub(super) fn npm_remote_version_with_timeout(pkg: &str, timeout: Duration) -> Option<String> {
    let (prog, prefix) =
        match resolve_exe_invocation("npm", "请先安装 Node.js (https://nodejs.org)") {
            Ok(x) => x,
            Err(_) => return None,
        };
    let mut args = prefix;
    args.push("view".to_string());
    args.push(pkg.to_string());
    args.push("version".to_string());
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut cmd = hidden_cmd(&prog);
    cmd.args(&arg_refs);
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::null());
    let mut child = cmd.spawn().ok()?;
    let started_at = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                let mut out = String::new();
                if let Some(mut so) = child.stdout.take() {
                    use std::io::Read;
                    let _ = so.read_to_string(&mut out);
                }
                return out
                    .lines()
                    .map(|l| l.trim().to_string())
                    .find(|l| !l.is_empty());
            }
            Ok(None) => {
                if started_at.elapsed() >= timeout {
                    kill_process_tree(&mut child);
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => return None,
        }
    }
}

/// 简单 semver 比较：`version_gt(a, b)` 判断 a > b（数字段逐一比较，忽略前置 v）。
/// 无法解析的段按 0 处理；相等或 a < b 返回 false。
pub(super) fn version_gt(a: &str, b: &str) -> bool {
    fn digit_parts(v: &str) -> Vec<i64> {
        v.trim_start_matches('v')
            .split(|c: char| c != '.' && !c.is_ascii_digit())
            .filter(|s| !s.is_empty())
            .map(|s| s.parse::<i64>().unwrap_or(0))
            .collect()
    }
    let (pa, pb) = (digit_parts(a), digit_parts(b));
    for i in 0..pa.len().max(pb.len()) {
        let (x, y) = (
            pa.get(i).copied().unwrap_or(0),
            pb.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x > y;
        }
    }
    false
}

pub(super) fn git_cmd(dir: &Path, args: &[&str]) -> Result<String, String> {
    let mut cmd = std::process::Command::new("git");
    cmd.current_dir(dir);
    cmd.args(crate::commands::utils::with_git_long_paths(args));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let output = cmd.output().map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// 检查项目是否有新版本：git fetch 后比较本地 HEAD 与远端跟踪分支。
/// 返回是否落后、本地/远端 commit、落后提交数。
#[tauri::command]
pub fn npm_check_update(project_id: String) -> NodeUpdateInfo {
    let empty = |e: String| NodeUpdateInfo {
        has_update: false,
        current_commit: String::new(),
        latest_commit: String::new(),
        behind: 0,
        current_version: None,
        latest_version: None,
        error: Some(e),
    };
    let Some(def) = find_project(&project_id) else {
        return empty(format!("未找到项目: {}", project_id));
    };
    // npx 模式：比较本地已装版本与 npm registry 远程最新版本
    if def.is_npx() {
        let current = npx_local_version(&def);
        let latest = npm_remote_version(&def.npx_package.trim().to_string());
        let has_update = match (&current, &latest) {
            (Some(c), Some(l)) => c != l && version_gt(l, c),
            _ => false,
        };
        return NodeUpdateInfo {
            has_update,
            current_commit: String::new(),
            latest_commit: String::new(),
            behind: if has_update { 1 } else { 0 },
            current_version: current,
            latest_version: latest.clone(),
            error: latest
                .is_none()
                .then(|| "无法获取远程版本，请检查网络 / npm registry 可达性".to_string()),
        };
    }
    let dir = def.managed_dir();
    if !dir.exists() {
        return empty("项目尚未安装".to_string());
    }

    // 1. fetch 远端（失败不致命，可能离线，仍尝试用本地缓存跟踪分支判断）
    let fetch_result = git_cmd(&dir, &["fetch", "origin", "--prune"]);
    if let Err(e) = &fetch_result {
        eprintln!("[node_manager] check_update fetch 失败(忽略): {}", e);
    }

    // 2. 本地 HEAD
    let current = git_cmd(&dir, &["rev-parse", "--short", "HEAD"]).unwrap_or_default();

    // 3. 远端跟踪分支（@{u} 优先，回退 origin/master / origin/main）
    let upstream = git_cmd(&dir, &["rev-parse", "--short", "@{u}"])
        .or_else(|_| git_cmd(&dir, &["rev-parse", "--short", "origin/master"]))
        .or_else(|_| git_cmd(&dir, &["rev-parse", "--short", "origin/main"]));

    let latest = match upstream {
        Ok(u) => u,
        Err(e) => return empty(e),
    };

    if current.is_empty() || latest.is_empty() {
        return empty("无法获取 git 提交信息".to_string());
    }

    // 4. 落后提交数
    let behind = git_cmd(
        &dir,
        &["rev-list", "--count", &format!("{}..{}", current, latest)],
    )
    .ok()
    .and_then(|s| s.parse::<i64>().ok())
    .unwrap_or(0);

    NodeUpdateInfo {
        has_update: current != latest && behind > 0,
        current_commit: current,
        latest_commit: latest,
        behind,
        current_version: None,
        latest_version: None,
        error: None,
    }
}

pub(super) fn status_for(def: &NodeProjectDef) -> NodeProjectStatus {
    if def.is_docker() {
        return docker_status_for(def);
    }
    let installed = def.installed();
    if !installed {
        return NodeProjectStatus {
            id: def.id.clone(),
            display_name: def.display_name.clone(),
            installed: false,
            status: "not_installed".to_string(),
            port: Some(def.default_port),
            pid: None,
            git_version: None,
            local_version: None,
            error: None,
            conflict_process: None,
            containers: 0,
            init_available: def.has_init(),
            initialized: def.initialized(),
        };
    }

    let git_version = command_version(
        "git",
        &[
            "-C",
            &def.managed_dir().to_string_lossy(),
            "rev-parse",
            "--short",
            "HEAD",
        ],
    );
    let git_version = git_version.filter(|v| !v.is_empty());
    // npx 模式：从本地 node_modules 读取已装包版本号
    let local_version = if def.is_npx() {
        npx_local_version(def)
    } else {
        None
    };

    // 运行状态：端口被我们的进程占用 → running；被其他进程占用 → port_conflict
    let mut status = "stopped".to_string();
    let mut pid = None;
    let mut conflict_process = None;
    if let Some(p) = crate::commands::utils::port_owner_pid(def.default_port) {
        let recorded = recorded_pid(&def.id);
        if recorded == Some(p) || recorded.is_some() {
            pid = Some(p);
            status = "running".to_string();
        } else if recorded.is_some() {
            // 记录过 PID 但端口换了归属，仍视为 running（按记录 pid 判定）
            pid = recorded;
            status = "running".to_string();
        } else {
            status = "port_conflict".to_string();
            pid = Some(p);
            // 借用 port.rs 的 tasklist 查询占用进程名，供前端明确展示
            conflict_process =
                crate::commands::port::find_port_owner(&def.default_port.to_string())
                    .map(|o| o.process_name);
        }
    } else if let Some(p) = recorded_pid(&def.id) {
        // 进程被本应用 spawn 但端口未监听（启动慢/异常）
        pid = Some(p);
        status = "running".to_string();
    }

    NodeProjectStatus {
        id: def.id.clone(),
        display_name: def.display_name.clone(),
        installed: true,
        status,
        port: Some(def.default_port),
        pid,
        git_version,
        local_version,
        error: None,
        conflict_process,
        containers: 0,
        init_available: def.has_init(),
        initialized: def.initialized(),
    }
}

#[tauri::command]
pub fn npm_list_projects() -> Vec<NodeProjectDef> {
    registry()
}

/// 返回当前服务类项目存储目录（供前端「设置」展示）。
#[tauri::command]
pub fn get_node_projects_dir() -> String {
    crate::commands::config::get_node_projects_dir()
        .to_string_lossy()
        .to_string()
}

/// 更新服务类项目存储目录，并把旧目录下已安装的项目移动到新目录。
/// 返回迁移结果（移动失败的项目列表）。
#[tauri::command]
pub fn update_node_projects_dir(new_dir: String) -> Result<Vec<String>, String> {
    use crate::commands::config::{get_node_projects_dir, load_config, save_config};

    let new_dir = new_dir.trim().to_string();
    if new_dir.is_empty() {
        return Err("服务类项目存储路径不能为空".to_string());
    }
    let old_dir = get_node_projects_dir();
    let new_path = Path::new(&new_dir);

    // 迁移：把旧目录下每个已存在的子项目移到新目录
    let mut failures: Vec<String> = Vec::new();
    if old_dir.exists() && old_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&old_dir) {
            for entry in entries.flatten() {
                let src = entry.path();
                if !src.is_dir() {
                    continue;
                }
                let name = src
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let dst = new_path.join(&name);
                if dst.exists() {
                    failures.push(format!("{} 已存在于新目录，跳过", name));
                    continue;
                }
                if let Some(parent) = dst.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                if let Err(e) = fs::rename(&src, &dst) {
                    eprintln!("[node_manager] 迁移项目 {} 失败: {}", name, e);
                    failures.push(format!("{} 迁移失败: {}", name, e));
                } else {
                    eprintln!(
                        "[node_manager] 已迁移项目 {} -> {}",
                        src.display(),
                        dst.display()
                    );
                }
            }
        }
    }

    // 保存新配置
    let mut config = load_config();
    config.node_projects_dir = new_path.to_string_lossy().to_string();
    save_config(&config)?;

    Ok(failures)
}
