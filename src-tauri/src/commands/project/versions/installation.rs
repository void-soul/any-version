use super::*;

pub(super) fn apply_transform(version: &str, transform: &str) -> String {
    let mut ver = version.to_string();
    for op in transform.split(';') {
        let op = op.trim_start(); // 只去掉前导空白，保留尾部空格（如 "trim_prefix:Python " 中的空格）
        if let Some(suffix) = op.strip_prefix("trim_suffix:") {
            ver = ver.strip_suffix(suffix).unwrap_or(&ver).to_string();
        } else if let Some(prefix) = op.strip_prefix("trim_prefix:") {
            ver = ver.strip_prefix(prefix).unwrap_or(&ver).to_string();
        } else if let Some(prefix) = op.strip_prefix("prefix:") {
            ver = format!("{}{}", prefix, ver);
        } else if let Some(repl) = op.strip_prefix("replace:") {
            // format: replace:old:new
            let parts: Vec<&str> = repl.split(':').collect();
            if parts.len() >= 2 {
                ver = ver.replace(parts[0], parts[1]);
            }
        }
    }
    ver
}

/// 安装指定版本（下载 -> 解压 -> 安装到 versions_dir -> 创建 junction -> 配置环境变量）
/// 安装在独立 Tokio 任务中运行，支持通过 project_cancel_install 取消。
#[tauri::command]
pub async fn project_install_version(
    app: AppHandle,
    id: String,
    version: String,
) -> Result<(), String> {
    crate::runtime_tasks::run_if_enabled("sdk", async move {
        // 预检：项目和下载信息
        let config = load_config();
        if !config.managed_items.contains(&id) {
            return Err("项目尚未开启托管，无法下载/安装版本".to_string());
        }
        let def = crate::commands::project::registry::find_by_id(&id)
            .ok_or_else(|| format!("未找到项目: {}", id))?;
        let delegation =
            crate::commands::project::scanner::get_project_delegation(&config, &id, &def);
        if !delegation.version_control {
            return Err("项目尚未开启“版本控制与下载”功能，无法下载/安装版本".to_string());
        }
        // 配了 github_asset 的托管物走 assets 解析（资产名里有推不出来的值，例如 Ghidra 的构建日期）；
        // 其余走 URL 模板。两条路径互斥，未配置时行为与从前完全一致。
        let dl_info = match async_resolve_github_asset(&id, &version).await? {
            Some(info) => info,
            None => get_download_url(&id, &version)?,
        };

        let cancellation = crate::commands::foundation::owned_process::Cancellation::default();
        let (start, ready) = tokio::sync::oneshot::channel::<()>();
        let (send, receive) = tokio::sync::oneshot::channel();
        let id_task = id.clone();
        let version_task = version.clone();
        let app_task = app.clone();
        let worker_token = cancellation.clone();
        {
            let mut active = get_active_downloads()
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if active.contains_key(&id) {
                return Err("该项目已有安装进行中，请先取消或等待完成".into());
            }
            active.insert(
                id.clone(),
                ActiveInstallEntry {
                    active: ActiveInstall {
                        sdk: id.clone(),
                        version,
                        step: "下载中".into(),
                        downloaded: 0,
                        total: 0,
                        pct: 0,
                        speed_str: String::new(),
                    },
                    abort_handle: None,
                    cancellation: cancellation.clone(),
                },
            );
        }

        let cleanup = InstallRegistrationGuard {
            id: id.clone(),
            cancellation: cancellation.clone(),
        };
        let abort_handle =
            crate::runtime_tasks::spawn_owned("sdk", &format!("install-{id}"), async move {
                let cleanup = cleanup;
                if ready.await.is_err() {
                    return;
                }
                let result = crate::commands::foundation::owned_process::run(
                    "sdk",
                    worker_token,
                    move || async move {
                        let _cleanup = cleanup;
                        do_install(app_task, id_task, version_task, dl_info).await
                    },
                )
                .await;
                let _ = send.send(result);
            })
            .ok_or("模块已停用或安装任务尚在安全收尾")?;
        if let Some(entry) = get_active_downloads()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(&id)
        {
            entry.abort_handle = Some(abort_handle);
        }
        let _ = start.send(());
        receive
            .await
            .map_err(|_| "安装已取消或模块已停用".to_string())??;
        let _ = crate::tray::rebuild_tray_menu(&app);
        Ok(())
    })
    .await
}

/// npm 包类型的安装：在 versions_dir/{id}/{version} 目录下执行
/// `npm install --prefix <dest_dir> <pkg>@{version}`（npm 包名来自 def.npm_pkg_name）。
/// 复用 junction 与 PATH 托管机制；bin 目录为 `<dest>/node_modules/.bin`。
pub(super) async fn do_npm_install(
    app: AppHandle,
    id: String,
    version: String,
    pkg: String,
) -> Result<(), String> {
    let config = load_config();
    let dest_dir = Path::new(&config.versions_dir).join(&id).join(&version);

    // 1. 先确认 npm 可用（npm_pkg_name 型项目依赖 Node.js 环境）
    let npm_probe = crate::commands::foundation::owned_process::capture(
        "sdk",
        crate::commands::hidden_cmd::hidden_cmd("npm").arg("--version"),
        std::time::Duration::from_secs(30 * 60),
    )
    .map_err(|e| format!("未找到 npm，请先安装 Node.js: {}", e))?;
    if !npm_probe.status.success() {
        return Err("npm 不可用，请先安装 Node.js".to_string());
    }

    // 2. 清理旧目录并创建
    if dest_dir.exists() {
        let _ = fs::remove_dir_all(&dest_dir);
    }
    fs::create_dir_all(&dest_dir).map_err(|e| e.to_string())?;

    // 3. 执行 npm install --prefix <dest_dir> <pkg>@{version}
    sync_install_step(&app, &id, "npm 安装中");
    let version_spec = if version.is_empty() || version.eq_ignore_ascii_case("latest") {
        pkg.clone()
    } else {
        format!("{}@{}", pkg, version)
    };
    crate::exit_log!("[npm 安装] {} -> {}", id, version_spec);

    let npm_result = crate::commands::foundation::owned_process::capture(
        "sdk",
        crate::commands::hidden_cmd::hidden_cmd("npm")
            .args(["install", "--prefix"])
            .arg(&dest_dir)
            .arg(&version_spec),
        std::time::Duration::from_secs(30 * 60),
    )
    .map_err(|e| format!("npm install 执行失败: {}", e))?;

    if !npm_result.status.success() {
        let stderr = String::from_utf8_lossy(&npm_result.stderr);
        let stdout = String::from_utf8_lossy(&npm_result.stdout);
        crate::exit_log!("[npm 安装失败] stdout: {} stderr: {}", stdout, stderr);
        return Err(format!(
            "npm install 失败: {}",
            if stderr.trim().is_empty() {
                stdout.trim()
            } else {
                stderr.trim()
            }
        ));
    }

    // 若 `--prefix` 实际把内容装到了 dest_dir/.，但在 Windows 上 node_modules 就在 dest_dir 根下
    crate::commands::foundation::owned_process::ensure_running("sdk")?;
    // 4. 首次安装时自动创建 junction
    sync_install_step(&app, &id, "创建链接中");
    let junction_path = Path::new(&config.links_dir).join(&id);
    if !junction_path.exists() {
        let _ = crate::commands::cache::create_junction(&junction_path, &dest_dir);
    }

    sync_install_step(&app, &id, "完成");

    Ok(())
}

/// MSIX 安装（WinGet 这类「系统级包」专用）：下载 MSIX 包 + 依赖包归档，
/// 再用 `Add-AppxPackage` 覆盖安装。
///
/// 与归档型安装的差别：**不落 versions_dir、不做 junction**。MSIX 由 Windows 统一管理，
/// 同名包只能有一份，所以「装新版本」就是覆盖旧的 —— 前端的版本列表用于选择装哪个版本，
/// 「切换版本」对这类 SDK 无意义（见 project_use_version_impl 里的显式报错）。
pub(super) async fn do_msix_install(
    app: AppHandle,
    id: String,
    version: String,
    dl_info: DownloadInfo,
    def: &crate::commands::project::types::ProjectDef,
) -> Result<(), String> {
    let (temp_dir, _) = setup_temp_dir(&id)?;
    let _guard = TempDirGuard {
        path: temp_dir.clone(),
    };

    // 1. 主包
    let main_path = temp_dir.join(format!("package.{}", dl_info.file_ext));
    let id_cap = id.clone();
    let app_handle = app.clone();
    download_with_progress(&dl_info.url, &main_path, move |downloaded, total, speed| {
        let pct = if total > 0 {
            (downloaded * 100 / total) as u8
        } else {
            0
        };
        let speed_str = format!("{}/s", crate::commands::cache::format_bytes(speed as u64));
        let _ = app_handle.emit(
            "download-progress",
            DownloadProgress {
                sdk: id_cap.clone(),
                downloaded,
                total,
                pct,
                speed_str: speed_str.clone(),
            },
        );
        if let Some(entry) = get_active_downloads().lock().unwrap().get_mut(&id_cap) {
            entry.active.downloaded = downloaded;
            entry.active.total = total;
            entry.active.pct = pct;
            entry.active.speed_str = speed_str;
        }
    })
    .await
    .map_err(|e| format!("下载失败: {}", e))?;

    // 2. 依赖包（可选）：解压后收集其中的 .msix / .appx / .msixbundle
    let mut dep_paths: Vec<String> = Vec::new();
    if let Some(tpl) = def.dependency_url_template.as_deref() {
        let dep_url = tpl
            .replace("{version}", &version)
            .replace("{ver}", version.trim_start_matches('v'));
        let dep_archive = temp_dir.join("dependencies.zip");
        sync_install_step(&app, &id, "下载依赖包");
        let id_cap = id.clone();
        let app_handle = app.clone();
        download_with_progress(&dep_url, &dep_archive, move |downloaded, total, speed| {
            let pct = if total > 0 {
                (downloaded * 100 / total) as u8
            } else {
                0
            };
            let speed_str = format!("{}/s", crate::commands::cache::format_bytes(speed as u64));
            let _ = app_handle.emit(
                "download-progress",
                DownloadProgress {
                    sdk: id_cap.clone(),
                    downloaded,
                    total,
                    pct,
                    speed_str: speed_str.clone(),
                },
            );
            if let Some(entry) = get_active_downloads().lock().unwrap().get_mut(&id_cap) {
                entry.active.downloaded = downloaded;
                entry.active.total = total;
                entry.active.pct = pct;
                entry.active.speed_str = speed_str;
            }
        })
        .await
        .map_err(|e| format!("依赖包下载失败: {}", e))?;

        let dep_dir = temp_dir.join("deps");
        unzip_file(&dep_archive, &dep_dir).map_err(|e| format!("依赖包解压失败: {}", e))?;
        collect_msix_files(&dep_dir, &mut dep_paths);
        // 依赖 zip 里同时含 x86 / x64 / arm64 三份，必须按本机架构筛选：
        // 全传会让部署引擎挑到不匹配的那份，报 HRESULT 0x80073D10。
        let before = dep_paths.len();
        dep_paths = filter_deps_for_arch(dep_paths, system_arch_token());
        // 写入实际保留的包名：下次再失败时不用重新下载归档就能看出筛掉了什么
        let kept_names: Vec<String> = dep_paths
            .iter()
            .map(|p| {
                std::path::Path::new(p)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?")
                    .to_string()
            })
            .collect();
        crate::exit_log!(
            "[MSIX 依赖] 共 {} 个，按架构 {} 筛选后 {} 个: {}",
            before,
            system_arch_token(),
            dep_paths.len(),
            kept_names.join(", ")
        );
    }

    // 3. Add-AppxPackage（覆盖安装：-ForceApplicationShutdown 让占用中的旧版也能被替换）
    sync_install_step(&app, &id, "安装中");
    let main_str = main_path.to_string_lossy().to_string();
    let mut script = format!(
        "Add-AppxPackage -Path '{}' -ForceApplicationShutdown",
        main_str.replace('\'', "''")
    );
    if !dep_paths.is_empty() {
        let list = dep_paths
            .iter()
            .map(|p| format!("'{}'", p.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(",");
        script.push_str(&format!(" -DependencyPath {}", list));
    }

    let out = crate::commands::foundation::owned_process::capture(
        "sdk",
        crate::commands::hidden_cmd::hidden_cmd("powershell").args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &script,
        ]),
        std::time::Duration::from_secs(30 * 60),
    )
    .map_err(|e| format!("执行 Add-AppxPackage 失败: {}", e))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        crate::exit_log!("[MSIX 安装失败] stdout: {} stderr: {}", stdout, stderr);
        return Err(format!(
            "MSIX 安装失败: {}",
            if stderr.trim().is_empty() {
                stdout.trim()
            } else {
                stderr.trim()
            }
        ));
    }

    crate::commands::foundation::owned_process::ensure_running("sdk")?;
    // 记录「已装版本」（即便它不由 versions_dir 承载），
    // 并让扫描器的 MSIX 版本缓存立即失效 —— 否则下一次扫描会读到缓存里旧的「未安装」，
    // 装完界面上还是空的。
    {
        use crate::commands::config::{load_config, save_config};
        let mut config = load_config();
        config.active_versions.insert(id.clone(), version);
        let _ = save_config(&config);
        if let Some(ref pkg) = def.msix_package_name {
            crate::commands::project::scanner::invalidate_msix_version_cache(pkg);
        }
    }

    sync_install_step(&app, &id, "完成");
    Ok(())
}
