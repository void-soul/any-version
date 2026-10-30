use super::*;

/// 服务状态快照缓存。
///
/// 关键：`service_status_for_def` 内部会同步执行 `tasklist`/`wmic`/`netstat` 等
/// 系统命令，这些命令在长时运行后可能变慢甚至挂起。若在托盘菜单构建（主线程
/// 事件循环）里同步调用，会阻塞整个事件循环，表现为「服务消失 + 所有托盘命令
/// 无响应」。因此引入后台刷新的快照缓存：主线程只读缓存，shell 调用移入后台线程。
pub(super) const SERVICE_STATUS_TTL: Duration = Duration::from_secs(3);

pub(super) static SERVICE_STATUS_CACHE: Mutex<Option<(Instant, HashMap<String, ServiceStatus>)>> =
    Mutex::new(None);

/// 启动后首次同步回退是否已完成。
/// 同步执行 tasklist/netstat 会阻塞调用线程；收窄为仅启动后首次执行一次，
/// 之后缓存缺失一律走后台刷新，避免托盘主线程事件循环被反复阻塞。
pub(super) static BOOT_SYNC_DONE: AtomicBool = AtomicBool::new(false);

/// 失效服务状态快照缓存。在启动/停止/强制终止服务成功后调用，
/// 确保随后的前端轮询不会读到 TTL 内过期的旧状态（避免状态回跳）。
pub(crate) fn invalidate_service_status_cache() {
    if let Ok(mut guard) = SERVICE_STATUS_CACHE.lock() {
        *guard = None;
    }
}

/// 清理缓存并同步检测指定服务，供进入 SDK 详情页时获取最新状态。
pub(crate) fn refresh_service_status_for_id(id: &str) -> Option<ServiceStatus> {
    invalidate_service_status_cache();
    let registry = crate::commands::project::registry::registry();
    let def = registry.iter().find(|def| def.id == id)?;
    Some(service_status_for_def(def))
}

/// 刷新指定服务 id 的状态快照（在后台线程调用，内部会跑 tasklist/wmic/netstat）。
pub(crate) fn refresh_service_status_snapshot(ids: &[String]) -> HashMap<String, ServiceStatus> {
    let mut map = HashMap::new();
    let registry = crate::commands::project::registry::registry();
    for id in ids {
        if let Some(def) = registry.iter().find(|d| &d.id == id) {
            map.insert(id.clone(), service_status_for_def(def));
        }
    }
    if let Ok(mut guard) = SERVICE_STATUS_CACHE.lock() {
        // 单个 SDK 页面刷新时合并已有快照，避免启动阶段逐个扫描服务后，
        // 托盘请求全量状态只能看到最后一个服务。
        let merged = guard
            .as_ref()
            .map(|(_, previous)| {
                let mut merged = previous.clone();
                merged.extend(map.clone());
                merged
            })
            .unwrap_or_else(|| map.clone());
        *guard = Some((Instant::now(), merged.clone()));
        return merged;
    }
    map
}

/// 读取服务状态快照（不阻塞，不执行任何 shell 命令）。
/// 命中缓存则返回；缓存过期或缺失则触发一次后台刷新，本轮返回空快照。
pub(crate) fn service_status_snapshot(managed_ids: &[String]) -> HashMap<String, ServiceStatus> {
    let fresh = {
        let guard = SERVICE_STATUS_CACHE.lock().ok();
        guard
            .as_ref()
            .and_then(|g| g.as_ref())
            .map(|(at, map)| {
                let covers_all = managed_ids.iter().all(|id| map.contains_key(id));
                (at.elapsed() < SERVICE_STATUS_TTL && covers_all, map.clone())
            })
            .unwrap_or((false, HashMap::new()))
    };
    if fresh.0 {
        return fresh.1;
    }
    // 缓存过期/缺失：后台刷新，本轮返回空（托盘会先渲染无服务状态，稍后 rebuild 补齐）。
    // 关键修复：若缓存完全缺失（首次构建 / 重启后），本轮直接回退为同步检测真实状态，
    // 与 scanner.rs 的 build_service_status 保持一致——否则托盘会一直停在“待启动”直到某次
    // 异步重建被触发，而该重建在启动阶段时序下可能被节流丢弃（见 tray.rs 兜底定时器）。
    let ids = managed_ids.to_vec();
    let _ = std::thread::spawn(move || {
        let _ = refresh_service_status_snapshot(&ids);
        let _ = crate::tray::rebuild_tray_menu_global();
    });
    if fresh.1.is_empty() && !BOOT_SYNC_DONE.swap(true, Ordering::SeqCst) {
        // 完全无缓存且尚未做过同步回退（仅启动后首次）：
        // 同步检测一次真实状态，保证托盘首屏即显示真实状态（已在运行的服务显示“运行中”）。
        // 之后的缓存缺失一律返回空并等待后台刷新，避免阻塞事件循环。
        let registry = crate::commands::project::registry::registry();
        let mut fallback = HashMap::new();
        for id in managed_ids {
            if let Some(def) = registry.iter().find(|d| &d.id == id) {
                fallback.insert(id.clone(), service_status_for_def(def));
            }
        }
        return fallback;
    }
    fresh.1
}

pub(crate) fn service_status_for_def(def: &ProjectDef) -> ServiceStatus {
    let runtime = resolve_service_runtime(def, None).ok();
    let install_root = runtime.as_ref().and_then(|r| r.install_root.clone());
    let all_processes = service_processes(def);
    let running_system_service = find_running_system_service(def);
    let external_process = all_processes
        .iter()
        .find(|process| process_matches_def(&process.name, def))
        .cloned();
    // 诊断：打印判定输入，定位"服务在跑但判定 stopped"的根因
    crate::exit_log!(
        "[svc-status] id={} name={} install_root={:?} 进程数={}",
        def.id,
        def.display_name,
        install_root
            .as_ref()
            .map(|p| p.to_string_lossy().to_string()),
        all_processes.len()
    );
    for p in &all_processes {
        crate::exit_log!(
            "[svc-status]   进程 pid={} name={} exe_path={:?}",
            p.pid,
            p.name,
            p.exe_path
        );
    }

    // 只有路径属于本 Any Version 实例的 install_root，才是我们真正的服务进程
    let processes: Vec<ProcessInfo> = if let Some(ref root) = install_root {
        let canonical_root = fs::canonicalize(root).unwrap_or_else(|_| root.clone());
        all_processes
            .into_iter()
            .filter(|p| {
                if let Some(ref exe_path_str) = p.exe_path {
                    let exe_path = Path::new(exe_path_str);
                    if let Ok(canonical_exe) = fs::canonicalize(exe_path) {
                        canonical_exe.starts_with(&canonical_root)
                    } else {
                        exe_path.starts_with(root)
                    }
                } else {
                    false
                }
            })
            .collect()
    } else {
        // 如果没有已启用的安装目录，则不能把别人的进程当作我们的进程
        Vec::new()
    };

    let port = runtime.as_ref().and_then(|r| r.port).or(def.default_port);

    let mut running = false;
    let mut external = false;
    let mut status = if install_root.is_some() {
        "stopped"
    } else {
        "not_installed"
    }
    .to_string();
    let mut pid = None;
    let mut process_name = None;

    if let Some(process) = processes.first() {
        running = true;
        status = "running".to_string();
        pid = Some(process.pid);
        process_name = Some(process.name.clone());
    } else if let Some(process) = external_process {
        running = true;
        external = true;
        status = "external_running".to_string();
        pid = Some(process.pid);
        process_name = Some(process.name);
    } else if running_system_service.is_some() {
        running = true;
        external = true;
        status = "external_running".to_string();
    } else if let Some(port) = port {
        if let Some(owner) = find_port_owner_simple(&port.to_string()) {
            let mut owner_is_ours = false;
            if let Ok(owner_pid) = owner.pid.parse::<u32>() {
                pid = Some(owner_pid);
                if let Some(ref root) = install_root {
                    if let Some(exe_path_str) = exe_path_by_pid(owner_pid) {
                        let exe_path = Path::new(&exe_path_str);
                        let canonical_root =
                            fs::canonicalize(root).unwrap_or_else(|_| root.clone());
                        if let Ok(canonical_exe) = fs::canonicalize(exe_path) {
                            owner_is_ours = canonical_exe.starts_with(&canonical_root);
                        } else {
                            owner_is_ours = exe_path.starts_with(root);
                        }
                    }
                }
            }
            process_name = Some(owner.process_name.clone());
            if owner_is_ours {
                running = true;
                status = "running".to_string();
            } else if process_matches_def(&owner.process_name, def) {
                // 外部目录或系统服务启动的同名服务仍属于该 SDK 的运行实例，
                // 但不能交给 AnyVersion 的停止命令接管。
                running = true;
                external = true;
                status = "external_running".to_string();
            } else {
                status = "port_conflict".to_string();
            }
        }
    }

    crate::exit_log!(
        "[svc-status] => id={} running={} status={} pid={:?} 注册系统服务={:?} 运行中系统服务={:?}",
        def.id,
        running,
        status,
        pid,
        find_registered_system_service(def),
        running_system_service
    );
    ServiceStatus {
        running,
        port,
        pid,
        data_dir: runtime
            .as_ref()
            .map(|r| r.data_dir.clone())
            .unwrap_or_default(),
        log_dir: runtime
            .as_ref()
            .map(|r| r.log_dir.clone())
            .unwrap_or_default(),
        status: Some(status),
        external,
        process_name,
        install_root: install_root.map(|p| p.to_string_lossy().to_string()),
        config_file: runtime
            .as_ref()
            .and_then(|r| r.config_file.as_ref())
            .map(|p| p.to_string_lossy().to_string()),
        system_service_name: find_registered_system_service(def),
    }
}
