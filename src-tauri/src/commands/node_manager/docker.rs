use super::*;

// ─── Docker 运行方式（runtime = "docker"）───
//
// 与进程模式的差别只有「怎么起」：安装 = 生成 compose + 拉镜像，启动 = `compose up -d`，
// 停止 = `compose down`。其余契约全部复用（事件名、状态结构、自启 id 前缀），
// 所以前端不必维护两套逻辑，Docker 相关的通用能力都在 commands/docker.rs。

/// 跑一条 `docker compose` 子命令；输出逐行走既有 `npm-log` 事件，前端日志区无需改动。
pub(super) fn docker_compose_run(
    app: &tauri::AppHandle,
    def: &NodeProjectDef,
    phase: &str,
    sub: &[&str],
) -> Result<(), String> {
    let dir = crate::commands::docker::docker_dir(&def.id);
    let spec = def.docker_spec().ok_or("该服务缺少 docker 配置")?;
    let args = crate::commands::docker::compose_args(spec.project, &dir, sub);
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let (ok, last_err, _out) = run_capture_live(
        app,
        &def.id,
        phase,
        "docker",
        &arg_refs,
        Some(&dir),
        &[],
        None,
    );
    if ok {
        return Ok(());
    }
    let msg = last_err.trim();
    Err(format!(
        "docker {} 失败: {}",
        sub.first().copied().unwrap_or("compose"),
        if msg.is_empty() { "未知错误" } else { msg }
    ))
}

/// Docker 就绪检查：把「没装 / daemon 没起 / 缺 compose」分开说清楚。
///
/// Windows 上最常见的是第二种（装了 Docker Desktop 但没启动），笼统报「失败」
/// 用户无法自救，所以三种情况各给一句话。
pub(super) fn ensure_docker_ready() -> Result<(), String> {
    let env = crate::commands::docker::check_env();
    if env.cli && env.daemon_running && env.compose {
        return Ok(());
    }
    let detail = env.error.trim();
    Err(if detail.is_empty() {
        env.summary
    } else {
        format!("{}\n{}", env.summary, detail)
    })
}

pub(super) fn docker_install(app: &tauri::AppHandle, def: &NodeProjectDef) -> Result<(), String> {
    ensure_docker_ready()?;
    let spec = def.docker_spec().ok_or("该服务缺少 docker 配置")?;
    let dir = crate::commands::docker::docker_dir(&def.id);
    emit_progress(app, &def.id, "prepare", "正在生成 compose 配置…");
    let cfg = crate::commands::docker::prepare_in(&dir, &spec)?;
    emit_progress(
        app,
        &def.id,
        "pull",
        "正在拉取镜像（首次较慢，请耐心等待）…",
    );
    docker_compose_run(app, def, "pull", &["pull"])?;
    emit_progress(
        app,
        &def.id,
        "done",
        &format!("镜像就绪，端口 {}", cfg.port),
    );
    Ok(())
}

pub(super) fn docker_start(app: &tauri::AppHandle, def: &NodeProjectDef) -> Result<(), String> {
    ensure_docker_ready()?;
    let spec = def.docker_spec().ok_or("该服务缺少 docker 配置")?;
    // 启动前重渲一次：用户可能刚在配置弹窗里改过端口/密码
    let dir = crate::commands::docker::docker_dir(&def.id);
    let cfg = crate::commands::docker::prepare_in(&dir, &spec)?;
    let port = cfg.port;

    docker_compose_run(app, def, "up", &["up", "-d"])?;

    // 等端口监听再返回：否则前端会立刻显示「未运行」（与进程模式同一策略）。
    // 容器首次启动可能要初始化数据库，给 180s。
    let deadline = Instant::now() + Duration::from_secs(180);
    while Instant::now() < deadline {
        if crate::commands::utils::port_owner_pid(port).is_some() {
            emit_progress(
                app,
                &def.id,
                "ready",
                &format!(
                    "服务已就绪：{}",
                    def.web_path.replace("{port}", &port.to_string())
                ),
            );
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(format!(
        "容器已启动，但 180 秒内端口 {} 仍未监听：请展开日志查看容器输出",
        port
    ))
}

pub(super) fn docker_stop(app: &tauri::AppHandle, def: &NodeProjectDef) -> Result<(), String> {
    docker_compose_run(app, def, "down", &["down"])?;
    // PID 记录是进程模式的产物，容器由 daemon 持有；清掉避免状态判定残留
    if let Some(m) = running_pids().as_mut() {
        m.remove(&def.id);
    }
    Ok(())
}

pub(super) fn docker_upgrade(app: &tauri::AppHandle, def: &NodeProjectDef) -> Result<(), String> {
    ensure_docker_ready()?;
    let spec = def.docker_spec().ok_or("该服务缺少 docker 配置")?;
    let dir = crate::commands::docker::docker_dir(&def.id);
    crate::commands::docker::prepare_in(&dir, &spec)?;
    emit_progress(app, &def.id, "pull", "正在拉取最新镜像…");
    docker_compose_run(app, def, "pull", &["pull"])?;
    emit_progress(app, &def.id, "up", "正在用新镜像重建容器…");
    docker_compose_run(app, def, "up", &["up", "-d"])?;
    emit_progress(app, &def.id, "done", "已更新到最新镜像");
    Ok(())
}

/// 卸载：停容器 + 删**渲染产物**（compose / .env）。
///
/// 刻意不删 `kira.json`（里面有数据库密码）与数据目录：postgres 数据卷是**用当时那个
/// 密码**初始化的，密码一换，卷就再也连不上——那等于把用户数据变成砖。删除后
/// `installed()` 重新为 false，再装一次会沿用同一密码，数据照旧可用。
pub(super) fn docker_uninstall(app: &tauri::AppHandle, def: &NodeProjectDef) -> Result<(), String> {
    let dir = crate::commands::docker::docker_dir(&def.id);
    if dir.join("docker-compose.yml").is_file() {
        // down 需要 compose 文件在，先停再删
        let _ = docker_compose_run(app, def, "down", &["down"]);
    }
    for name in ["docker-compose.yml", ".env"] {
        let p = dir.join(name);
        if p.exists() {
            fs::remove_file(&p).map_err(|e| format!("删除 {} 失败: {}", p.display(), e))?;
        }
    }
    emit_progress(app, &def.id, "done", "已卸载（数据与数据库密码保留）");
    Ok(())
}

/// docker 服务的状态快照。
///
/// 状态判定用「在跑的容器数 + 端口探活」：容器数来自 `compose ps -q`（一行一个 id，
/// 比解析表格/JSON 稳定）；容器没跑但端口被占 = 有别人的进程占了那个端口，
/// 与「没启动」必须分开（前者用户得改端口或停掉占用者）。
pub(super) fn docker_status_for(def: &NodeProjectDef) -> NodeProjectStatus {
    let dir = crate::commands::docker::docker_dir(&def.id);
    let mut containers = 0usize;
    let mut error: Option<String> = None;
    let mut port = def.default_port;

    if let Some(spec) = def.docker_spec() {
        // 端口以**配置里的**为准（用户可能在弹窗里改过；定义里的 healthPort 只是默认值），
        // 否则改完端口后状态探测会去盯一个没人监听的旧端口，界面永远显示「未运行」。
        port = crate::commands::docker::read_config_in(&dir, &spec).port;
        if dir.join("docker-compose.yml").is_file() {
            let args = crate::commands::docker::compose_args(spec.project, &dir, &["ps", "-q"]);
            let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            let out = crate::commands::docker::probe("docker", &arg_refs);
            containers = crate::commands::docker::parse_ps_ids(&out.stdout);
            // 查不到容器但命令失败（daemon 没起等）：把原因带给界面，别只显示「未运行」。
            // 用带缓存的探测：状态每 4 秒查一次，而引擎没起时 docker 命令要等超时。
            if !out.ok && containers == 0 {
                let env = crate::commands::docker::check_env_cached();
                if !env.daemon_running {
                    error = Some(env.summary);
                }
            }
        }
    }

    let owner = crate::commands::utils::port_owner_pid(port);
    let status = crate::commands::docker::compose_state(containers, owner.is_some());
    let conflict_process = if status == "port_conflict" {
        crate::commands::port::find_port_owner(&port.to_string()).map(|o| o.process_name)
    } else {
        None
    };

    NodeProjectStatus {
        id: def.id.clone(),
        display_name: def.display_name.clone(),
        installed: def.installed(),
        status: if def.installed() {
            status.to_string()
        } else {
            "not_installed".to_string()
        },
        port: Some(port),
        pid: None,
        git_version: None,
        local_version: None,
        error,
        conflict_process,
        containers,
        init_available: def.has_init(),
        initialized: def.initialized(),
    }
}

/// 本机 Docker 环境（前端环境条用；与具体服务无关）。
#[tauri::command]
pub fn docker_env_check() -> crate::commands::docker::DockerEnv {
    crate::commands::docker::check_env()
}

/// 某个 docker 服务的可编辑配置（端口 / 数据库密码 / 配置目录）。
#[tauri::command]
pub fn docker_service_config(
    project_id: String,
) -> Result<crate::commands::docker::DockerServiceConfig, String> {
    let def = find_project(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    let spec = def
        .docker_spec()
        .ok_or_else(|| format!("{} 不是 Docker 服务", def.display_name))?;
    Ok(crate::commands::docker::read_config_in(
        &crate::commands::docker::docker_dir(&def.id),
        &spec,
    ))
}

/// 保存端口 / 数据库密码并重渲 compose。
///
/// 改密码会让**已有数据卷连不上**（postgres 只在初始化时用该密码建库），
/// 所以前端必须提示「需要删掉数据卷重建」——这个后果不该由后端静默兜底。
#[tauri::command]
pub fn docker_save_service_config(
    project_id: String,
    port: u16,
    // None / 0 = 沿用定义里的默认值（前端对「只发布一个端口」的服务不传这个）
    port2: Option<u16>,
    pg_password: String,
) -> Result<crate::commands::docker::DockerServiceConfig, String> {
    let def = find_project(&project_id).ok_or_else(|| format!("未找到项目: {}", project_id))?;
    let spec = def
        .docker_spec()
        .ok_or_else(|| format!("{} 不是 Docker 服务", def.display_name))?;
    crate::commands::docker::save_config_in(
        &crate::commands::docker::docker_dir(&def.id),
        &spec,
        port,
        port2.unwrap_or(0),
        &pg_password,
    )
}
