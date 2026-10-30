use super::*;

// ─── 安装 / 升级 ───

/// 解析包管理器为「可直接被 Command::new 执行」的调用方式 `(program, prefix_args)`。
/// 不能直接用裸命令名执行：pnpm/npm/yarn 在 Windows 上通常是 `pnpm.cmd` 脚本，
/// 既不在应用进程 PATH 的裸名解析范围内，也需经 `cmd.exe /c` 才能执行。
/// 返回的 program 为绝对路径，prefix_args 为执行前需插入的参数（脚本类为 `/c` 前缀）。
/// 返回 Windows cmd.exe 的绝对路径（避免裸名 `cmd` 受 PATH 影响）。
#[cfg(target_os = "windows")]
pub(super) fn cmd_exe_path() -> String {
    std::env::var("COMSPEC")
        .or_else(|_| std::env::var("SystemRoot").map(|r| format!("{}\\System32\\cmd.exe", r)))
        .unwrap_or_else(|_| "C:\\Windows\\System32\\cmd.exe".to_string())
}

/// 解析任意命令为「可直接被 Command::new 执行」的调用方式 `(program, prefix_args)`。
/// 不能直接用裸命令名执行：npm/npx/pnpm 在 Windows 上通常是 `.cmd` 脚本，
/// 既不在应用进程 PATH 的裸名解析范围内，也需经 `cmd.exe /c` 才能执行。
/// 返回的 program 为绝对路径，prefix_args 为执行前需插入的参数（脚本类为 `/c` 前缀）。
/// not_found_hint：命令不在 PATH 时的提示语。
pub(super) fn resolve_exe_invocation(
    exe: &str,
    not_found_hint: &str,
) -> Result<(String, Vec<String>), String> {
    let path = find_in_path(exe)
        .ok_or_else(|| format!("无法定位 {}（未在 PATH 中找到），{}", exe, not_found_hint))?;
    let p_str = path.to_string_lossy().to_string();
    let lower = p_str.to_lowercase();
    // Windows 上只有 .exe/.com 这类 PE 文件能被 CreateProcess 直接执行；
    // .cmd/.bat 及无扩展名脚本必须经 `cmd.exe /c` 解释，否则报 os error 193。
    let result = if !lower.ends_with(".exe") && !lower.ends_with(".com") {
        #[cfg(target_os = "windows")]
        {
            (cmd_exe_path(), vec!["/c".to_string(), p_str])
        }
        #[cfg(not(target_os = "windows"))]
        {
            (p_str, Vec::new())
        }
    } else {
        (p_str, Vec::new())
    };
    eprintln!(
        "[node_manager] resolve_exe_invocation({}) -> prog={:?}, prefix={:?}",
        exe, result.0, result.1
    );
    Ok(result)
}

pub(super) fn resolve_pm_invocation(def: &NodeProjectDef) -> Result<(String, Vec<String>), String> {
    resolve_exe_invocation(&def.package_manager, "请先安装或启用 corepack")
}

/// 在托管目录执行包管理器 install。
pub(super) fn pm_install(
    app: &tauri::AppHandle,
    def: &NodeProjectDef,
    dir: &Path,
) -> Result<(), String> {
    // Python 项目不走 pnpm/npm：requirements.txt 是唯一依赖清单，用 venv + pip 安装。
    if def.is_python() {
        return python_install(app, def, dir);
    }
    let pm = &def.package_manager;
    let (prog, prefix) = resolve_pm_invocation(def)?;
    emit_progress(
        app,
        &def.id,
        "install",
        &format!("正在运行 `{} install`（首次可能较慢）…", pm),
    );
    let mut args = prefix;
    args.push("install".to_string());
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let (ok, last_err, _out) = run_capture_live(
        app,
        &def.id,
        "install",
        &prog,
        &arg_refs,
        Some(dir),
        &[],
        None,
    );
    if !ok {
        let msg = last_err.trim();
        return Err(format!(
            "`{} install` 失败: {}",
            pm,
            if msg.is_empty() { "未知错误" } else { msg }
        ));
    }
    Ok(())
}

/// Python 解释器候选（按优先级）：`py -3`（官方启动器）→ `python3` → `python`。
/// 不再无脑用裸 `python`：PATH 上的可能是嵌入式发行版（无 venv 模块），
/// 如本应用 SDK 管理器安装的 embeddable Python。
pub(super) fn python_candidates() -> Vec<(String, Vec<String>)> {
    let mut candidates: Vec<(String, Vec<String>)> = Vec::new();
    if cfg!(windows) {
        candidates.push(("py".to_string(), vec!["-3".to_string()]));
    }
    candidates.push(("python3".to_string(), Vec::new()));
    candidates.push(("python".to_string(), Vec::new()));
    candidates
}

/// 探测解释器能否导入某模块（`<py> -c "import xxx"`）。
pub(super) fn probe_python_module(program: &str, prefix: &[String], code: &str) -> bool {
    let mut cmd = hidden_cmd(program);
    for a in prefix {
        cmd.arg(a);
    }
    cmd.args(["-c", code]);
    matches!(cmd.output(), Ok(out) if out.status.success())
}

/// 探测所有候选解释器，返回 (program, prefix, has_venv, has_pip) 列表。
pub(super) fn probe_python_candidates() -> Vec<(String, Vec<String>, bool, bool)> {
    python_candidates()
        .into_iter()
        .map(|(program, prefix)| {
            let has_venv = probe_python_module(&program, &prefix, "import venv, ensurepip");
            let has_pip = has_venv || probe_python_module(&program, &prefix, "import pip");
            (program, prefix, has_venv, has_pip)
        })
        .collect()
}

/// Python 运行时安装方案。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PythonRuntimePlan {
    /// 用该解释器创建 .venv（program + 前缀参数，如 py + ["-3"]）
    Venv(String, Vec<String>),
    /// 解释器无 venv 模块但可用 pip → 降级 `pip install --target .deps`
    TargetDeps(String, Vec<String>),
    /// 无可用解释器
    None,
}

/// 纯函数：venv 优先，其次 pip-only 降级，最后 None。
pub(crate) fn select_python_runtime(
    probes: &[(String, Vec<String>, bool, bool)],
) -> PythonRuntimePlan {
    for (program, prefix, has_venv, _has_pip) in probes {
        if *has_venv {
            return PythonRuntimePlan::Venv(program.clone(), prefix.clone());
        }
    }
    for (program, prefix, has_venv, has_pip) in probes {
        if !*has_venv && *has_pip {
            return PythonRuntimePlan::TargetDeps(program.clone(), prefix.clone());
        }
    }
    PythonRuntimePlan::None
}

/// Python 项目启动计划。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PythonLaunchPlan {
    /// 使用 .venv 内解释器
    Venv(PathBuf),
    /// 使用基准解释器 + .deps 依赖目录（PYTHONPATH / ._pth 注入）
    Deps {
        program: String,
        prefix: Vec<String>,
        deps_dir: PathBuf,
    },
    /// 运行时未就绪
    Missing,
}

/// 纯函数（仅读文件系统）：优先 .venv，其次读 `.deps/.python-runtime.json` 标记。
/// marker 里的解释器必须真实存在，否则按 Missing 处理（启动必然失败，应提示重装）。
pub(crate) fn plan_python_launch(managed_dir: &Path, venv_python: &Path) -> PythonLaunchPlan {
    if venv_python.exists() {
        return PythonLaunchPlan::Venv(venv_python.to_path_buf());
    }
    let deps_dir = managed_dir.join(".deps");
    let Ok(raw) = fs::read_to_string(deps_dir.join(".python-runtime.json")) else {
        return PythonLaunchPlan::Missing;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return PythonLaunchPlan::Missing;
    };
    let program = value
        .get("program")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if program.is_empty() || !Path::new(&program).exists() {
        return PythonLaunchPlan::Missing;
    }
    let prefix = value
        .get("prefix")
        .and_then(serde_json::Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    PythonLaunchPlan::Deps {
        program,
        prefix,
        deps_dir,
    }
}

/// `.deps` 模式下 `--target` 安装的 pywin32 需要显式加入 sys.path 的子目录：
/// 模块在 `win32/`（pywintypes 等）与 `win32/lib/`，DLL 在 `pywin32_system32/`，
/// 都不在 `.deps` 顶层，裸 PYTHONPATH=.deps 导入不了 pywintypes。
/// 只返回真实存在的子目录（未安装 pywin32 的项目返回空）。
pub(crate) fn pywin32_deps_dirs(deps_dir: &Path) -> Vec<PathBuf> {
    ["win32", "win32/lib", "pywin32_system32"]
        .iter()
        .map(|rel| deps_dir.join(rel))
        .filter(|p| p.exists())
        .collect()
}

/// `.deps` 模式启动的 PYTHONPATH（纯函数，便于测试）：
/// .deps（pip --target 依赖）+ 项目源码目录（`-m core.converter` 的包所在，
/// 嵌入式 Python 不会自动把 cwd 加进 sys.path）+ pywin32 子目录。
pub(crate) fn deps_pythonpath(project_dir: &Path, deps_dir: &Path) -> String {
    let mut paths = vec![deps_dir.to_path_buf(), project_dir.to_path_buf()];
    paths.extend(pywin32_deps_dirs(deps_dir));
    let sep = if cfg!(windows) { ";" } else { ":" };
    paths
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join(sep)
}

/// 嵌入式 Python（带 `*._pth`）会忽略 PYTHONPATH；
/// 把 .deps / 项目源码目录 / pywin32 子目录追加进 `._pth` 才能让它 import。
/// 只在解释器目录存在 `*._pth` 时生效（官方完整安装没有这个文件，走 PYTHONPATH 即可）。
pub(super) fn append_deps_to_embeddable_pth(
    interpreter: &str,
    deps_dir: &Path,
    project_dir: Option<&Path>,
) {
    let path = Path::new(interpreter);
    let Some(dir) = path.parent() else {
        return;
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if !name.ends_with("._pth") {
            continue;
        }
        let Ok(content) = fs::read_to_string(entry.path()) else {
            continue;
        };
        let mut lines: Vec<String> = Vec::new();
        lines.push(deps_dir.to_string_lossy().to_string());
        if let Some(pd) = project_dir {
            if pd != deps_dir {
                lines.push(pd.to_string_lossy().to_string());
            }
        }
        lines.extend(
            pywin32_deps_dirs(deps_dir)
                .into_iter()
                .map(|p| p.to_string_lossy().to_string()),
        );
        // 已全部追加过则不动
        if lines
            .iter()
            .all(|l| content.lines().any(|x| x.trim().eq_ignore_ascii_case(l)))
        {
            return;
        }
        lines.retain(|l| !content.lines().any(|x| x.trim().eq_ignore_ascii_case(l)));
        let mut out = content.clone();
        if !out.ends_with('\n') {
            out.push_str("\r\n");
        }
        for l in &lines {
            out.push_str(l);
            out.push_str("\r\n");
        }
        let _ = fs::write(entry.path(), out);
        eprintln!("[node_manager] 已把 .deps（含项目目录/pywin32 子目录）追加到 {}（嵌入式 Python 依赖可见性）", entry.path().display());
        return;
    }
}

/// 把降级方案写盘：marker（启动时读回）+ 嵌入式 ._pth 追加（.deps + 项目源码目录 + pywin32 子目录）。
pub(super) fn record_deps_runtime(
    dir: &Path,
    program: &str,
    prefix: &[String],
) -> Result<(), String> {
    let deps = dir.join(".deps");
    let resolved = find_in_path(program)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| program.to_string());
    let marker = serde_json::json!({ "program": resolved, "prefix": prefix });
    fs::write(deps.join(".python-runtime.json"), marker.to_string())
        .map_err(|e| format!("写入 .deps 运行时标记失败: {}", e))?;
    append_deps_to_embeddable_pth(&resolved, &deps, Some(dir));
    Ok(())
}

/// 纯函数：pip 安装参数（spec 可带 extras，如 `headroom-ai[proxy]`）。
/// `extras` 为项目声明的附加依赖（如 pywin32），与主包同一条命令安装，保证环境一致。
/// `target` 为 Some 时走 `--target`（嵌入式 Python 的 .deps 降级方案），否则装进当前环境（venv）。
pub(crate) fn pip_install_args(
    spec: &str,
    extras: &[String],
    target: Option<&str>,
    upgrade: bool,
) -> Vec<String> {
    let mut args = vec!["-m".to_string(), "pip".to_string(), "install".to_string()];
    if upgrade {
        args.push("--upgrade".to_string());
    }
    if let Some(dir) = target {
        args.push("--target".to_string());
        args.push(dir.to_string());
    }
    args.push(spec.to_string());
    args.extend(extras.iter().filter(|s| !s.trim().is_empty()).cloned());
    args
}

/// 纯函数：pip 包模式的启动参数 = `-m <module> <startCmd...>`。
pub(crate) fn pip_launch_args(module: &str, start_cmd: &[String]) -> Vec<String> {
    let mut args = vec!["-m".to_string(), module.to_string()];
    args.extend(start_cmd.iter().cloned());
    args
}
