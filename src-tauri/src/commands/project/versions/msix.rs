use super::*;

/// MSIX 依赖包里可能出现的架构写法（顺序有意义：`arm64` 必须排在 `arm` 之前）。
pub(super) const MSIX_ARCHS: [&str; 4] = ["x86", "x64", "arm64", "arm"];

/// 本机架构（取值与 MSIX 包里的写法一致）。
pub(super) fn system_arch_token() -> &'static str {
    match std::env::var("PROCESSOR_ARCHITECTURE")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "arm64" => "arm64",
        "x86" => "x86",
        _ => "x64", // AMD64 及其它默认按 x64
    }
}

/// 一条依赖包路径里携带的架构标记。
///
/// 实测 WinGet 的依赖归档（`DesktopAppInstaller_Dependencies.zip`）长这样：
/// ```text
/// robe\arm64\Microsoft.VCLibs.140.00.UWPDesktop_14.0.33728.0_arm64.appx
/// robe\x64\Microsoft.VCLibs.140.00.UWPDesktop_14.0.33728.0_x64.appx
/// robe\x86\Microsoft.VCLibs.140.00.UWPDesktop_14.0.33728.0_x86.appx
/// ```
/// 架构既出现在**目录名**也出现在**文件名后缀**（`_x64.appx`）里，而且不是
/// Store 下载那套 `_x64__8wekyb3d8bbwe`。所以两个位置都要认，且**每个路径段只认一个架构**
/// （`arm64` 先于 `arm`，否则 arm64 包会同时被标记成 arm + arm64 两种而被误排除）。
pub(super) fn dep_arch_markers(path: &str) -> Vec<&'static str> {
    let mut found: Vec<&'static str> = Vec::new();
    for comp in path.split(|c| c == '/' || c == '\\') {
        let lower = comp.to_ascii_lowercase();
        let hit = MSIX_ARCHS
            .iter()
            .find(|arch| lower == **arch || lower.contains(&format!("_{}", arch)));
        if let Some(arch) = hit {
            if !found.contains(arch) {
                found.push(*arch);
            }
        }
    }
    found
}

/// 只保留与本机架构一致的依赖包：带任何「非本机」架构标记的一律排除。
///
/// WinGet 的依赖归档同时提供 x86/x64/arm64 三套框架包，不加筛选地全塞进
/// `-DependencyPath`，部署引擎会挑到与本机不符的那份并失败 —— 实测 x64 机器上被
/// ARM64 的 VCLibs 卡住，报 HRESULT 0x80073D10。
///
/// 注意这里**没有**「一个都不匹配就原样返回」的兜底：那正是上一版把 arm64 漏过去的
/// 原因（按 `_x64__` 精确匹配 → 0 个命中 → 全量返回）。没有架构标记的包（架构无关）
/// 本来就会保留，认不出架构时宁可空列表，也不要把异架构的包交给部署引擎。
pub(super) fn filter_deps_for_arch(deps: Vec<String>, native: &str) -> Vec<String> {
    deps.into_iter()
        .filter(|p| dep_arch_markers(p).iter().all(|a| *a == native))
        .collect()
}

/// 递归收集目录下的 MSIX 系包文件（.appx / .msix / .appxbundle / .msixbundle）。
pub(super) fn collect_msix_files(dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_msix_files(&path, out);
        } else {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if matches!(ext.as_str(), "appx" | "msix" | "appxbundle" | "msixbundle") {
                out.push(path.to_string_lossy().to_string());
            }
        }
    }
}

/// 实际安装逻辑（在可取消的 Tokio 任务中运行）
pub(super) async fn do_install(
    app: AppHandle,
    id: String,
    version: String,
    dl_info: DownloadInfo,
) -> Result<(), String> {
    let def = crate::commands::project::registry::find_by_id(&id)
        .ok_or_else(|| format!("未找到项目: {}", id))?;
    let config = load_config();

    // 0. npm 包类型：通过 `npm install --prefix` 安装到 versions_dir，而非下载归档。
    if let Some(ref pkg) = def.npm_pkg_name {
        return do_npm_install(app, id, version, pkg.clone()).await;
    }

    // 0'. MSIX 包类型（如 WinGet）：下载后用 Add-AppxPackage 覆盖安装，
    //     不进 versions_dir / 不做 junction —— 系统级只存在一份。
    if def.install_mode.as_deref() == Some("msix") {
        return do_msix_install(app, id, version, dl_info, &def).await;
    }

    let file_ext = dl_info.file_ext.clone();
    let download_url = dl_info.url.clone();

    // 1. 创建临时目录，并用 RAII guard 保证取消/错误时自动清理
    let (temp_dir, _) = setup_temp_dir(&id)?;
    let _guard = TempDirGuard {
        path: temp_dir.clone(),
    };
    let archive_path = temp_dir.join(format!("archive.{}", file_ext));

    // 2. 下载（带进度和速度事件）
    let id_cap = id.clone();
    let app_handle = app.clone();
    let dl_result = download_with_progress(
        &download_url,
        &archive_path,
        move |downloaded, total, speed| {
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
            // 同步写入内存状态，供前端挂载时查询恢复进度条
            if let Some(entry) = get_active_downloads().lock().unwrap().get_mut(&id_cap) {
                entry.active.downloaded = downloaded;
                entry.active.total = total;
                entry.active.pct = pct;
                entry.active.speed_str = speed_str;
            }
        },
    )
    .await;

    if let Err(e) = dl_result {
        return Err(format!("下载失败: {}", e));
    }

    sync_install_step(&app, &id, "解压中");

    // 3. 解压
    let extract_dir = temp_dir.join("extracted");
    let ext_result = match file_ext.as_str() {
        "tar.gz" | "tgz" => extract_tar_gz(&archive_path, &extract_dir),
        "tar.xz" => extract_tar_xz(&archive_path, &extract_dir),
        "tar.bz2" => extract_tar_bz2(&archive_path, &extract_dir),
        "7z" => extract_7z(&archive_path, &extract_dir),
        "msi" => extract_msi(&archive_path, &extract_dir),
        "exe" => {
            fs::create_dir_all(&extract_dir).map_err(|e| e.to_string())?;
            fs::copy(&archive_path, extract_dir.join(format!("{}.exe", id)))
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
        _ => unzip_file(&archive_path, &extract_dir),
    };

    if let Err(e) = ext_result {
        return Err(format!("解压失败: {}", e));
    }

    // 4. 安装到 versions_dir
    let dest_dir = Path::new(&config.versions_dir).join(&id).join(&version);
    let extract_subdir = dl_info.extract_subdir.as_deref().unwrap_or("");
    let src_dir = if !extract_subdir.is_empty() {
        extract_dir.join(extract_subdir)
    } else {
        extract_dir
    };

    if let Err(e) = move_extract_to_dest(&src_dir, &dest_dir, def.merge_extracted_subdirs) {
        return Err(format!("安装失败: {}", e));
    }

    // _guard 在此之后 drop 时会清理 temp_dir（正常流程也清理）

    // 5. 后置配置（从 JSON post_install 读取）
    if let Some(ref post_install) = def.post_install {
        if post_install
            .get("generate_config")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            if let Some(tpl) = post_install.get("config_template").and_then(|v| v.as_str()) {
                // 配置文件名取 def.config_file（如 mongod.cfg），回退 my.ini 以保持旧行为
                let config_filename = def
                    .config_file
                    .clone()
                    .unwrap_or_else(|| "my.ini".to_string());
                let ini_path = dest_dir.join(&config_filename);
                // 数据/日志目录取值与运行时保持一致：统一由 def.data_dirs 解析
                // （同一套 custom → 配置文件 → 环境变量 → possible_paths → default_path 规则）。
                // 此前这里硬编码 data / log，与 data_dirs 的声明互相矛盾——
                // 例如 mongodb 声明 {install_root}\data\db，生成的 mongod.cfg 却写 dbPath=<root>/data，
                // 且日志目录统一为 logs 后硬编码的 log 会再次跑偏。
                let resolve_dir = |kind: &str, fallback: &str| -> String {
                    def.data_dirs
                        .iter()
                        .find(|d| d.kind.as_deref() == Some(kind))
                        .map(|d| {
                            crate::commands::service::resolve_data_dir(
                                &def,
                                d,
                                &config,
                                Some(&dest_dir),
                            )
                            .path
                        })
                        .unwrap_or_else(|| dest_dir.join(fallback).to_string_lossy().to_string())
                };
                let data_dir = resolve_dir("data", "data").replace("\\", "/");
                let log_dir = resolve_dir("log", "log").replace("\\", "/");
                let port = def.default_port.unwrap_or(0).to_string();
                let install_root_str = dest_dir.to_string_lossy().replace("\\", "/");
                let content = tpl
                    .replace("{install_root}", &install_root_str)
                    .replace("{basedir}", &install_root_str)
                    .replace("{datadir}", &data_dir)
                    .replace("{data_dir}", &data_dir)
                    .replace("{log_dir}", &log_dir)
                    .replace("{port}", &port);
                let _ = fs::write(&ini_path, content);
            }
        }
        if let Some(init_cmd) = post_install.get("init_command").and_then(|v| v.as_str()) {
            let parts: Vec<&str> = init_cmd.splitn(2, ' ').collect();
            let exe = dest_dir.join(parts[0]);
            let args: Vec<&str> = if parts.len() > 1 {
                parts[1].split(' ').collect()
            } else {
                vec![]
            };
            let _ = crate::commands::hidden_cmd::hidden_cmd(exe)
                .args(&args)
                .current_dir(&dest_dir)
                .output();
        }
    }

    // 7. 首次安装时自动创建 junction。环境变量在托管/修复时统一配置，版本安装不隐式修改注册表。
    sync_install_step(&app, &id, "创建链接中");
    let junction_path = Path::new(&config.links_dir).join(&id);
    if !junction_path.exists() {
        let _ = crate::commands::cache::create_junction(&junction_path, &dest_dir);
    }

    // 8. 安装根布局：首次安装后 links_dir/<id> 才存在，托管那一步会跳过（见 layout::apply）。
    if let Err(e) = crate::commands::project::layout::apply(&def, &junction_path) {
        eprintln!("[install] 补齐 {} 的安装根布局失败: {}", id, e);
    }

    sync_install_step(&app, &id, "完成");

    Ok(())
}
